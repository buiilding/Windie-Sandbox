//! Account-scoped enrollment and fenced presence transactions, independent of sessions.

use super::*;
use crate::device::*;
use crate::runtime::progression::{ToolAction, next_tool_action};
use crate::session::SessionId;
use crate::tool::control::{
    AttachmentFacts, ControlRequest, plan_provider_attachment, validate_mcp_membership,
};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolApprovalMode, ToolProviderId, ToolProviderKind,
    ToolProviderRef, ToolProviderRegistry, ToolSchema, ToolSchemaName,
};
use std::collections::HashSet;

#[cfg(test)]
pub(crate) mod tests;

type Result<T> = std::result::Result<T, DeviceError>;

/// Enrollment access is deliberately different from account and active-device authority.
pub(crate) struct EnrollmentPrincipal {
    pub id: EnrollmentId,
    pub digest: String,
}
pub(crate) struct DevicePrincipal {
    pub digest: String,
}

/// One durable browser approval for an immutable device-bound model call.
/// It is a hosted transport record, while the policy that requires it remains
/// shared in `tool::policy`.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct HostedToolApproval {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) assistant_message_id: String,
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) arguments_json: String,
    pub(crate) device_id: String,
    pub(crate) reason: String,
}

/// What an approved hosted tool call will do.  Only `Device` is sent over the
/// agent transport. `AttachMcp` changes hosted context after user approval and
/// deliberately never reaches the Mac as a command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PendingToolWork {
    Device {
        work: DeviceWork,
    },
    AttachMcp {
        plugin_id: String,
        component_id: String,
    },
}

const ENROLL_SELECT: &str = "SELECT *, floor(extract(epoch FROM expires_at))::bigint AS expiry, expires_at <= now() AS expired FROM device_enrollments";

fn view(row: &sqlx::postgres::PgRow) -> Result<EnrollmentView> {
    let state = if row.get::<bool, _>("expired") {
        EnrollmentState::Expired
    } else {
        match row.get::<String, _>("state").as_str() {
            "pending" => EnrollmentState::Pending,
            "approved" => EnrollmentState::Approved,
            "consumed" => EnrollmentState::Consumed,
            "denied" => EnrollmentState::Denied,
            "cancelled" => EnrollmentState::Cancelled,
            _ => return Err(DeviceError::Unavailable),
        }
    };
    Ok(EnrollmentView {
        id: EnrollmentId(
            Uuid::parse_str(&row.get::<String, _>("id")).map_err(|_| DeviceError::Unavailable)?,
        ),
        state,
        metadata: serde_json::from_value(row.get("metadata"))
            .map_err(|_| DeviceError::Unavailable)?,
        expires_at: row.get("expiry"),
        account_id: row.get("account_id"),
        account_label: row.get("account_label"),
        device_id: row
            .get::<Option<String>, _>("device_id")
            .map(|id| Uuid::parse_str(&id).map(DeviceId))
            .transpose()
            .map_err(|_| DeviceError::Unavailable)?,
    })
}

impl HostedStore {
    /// Attaches the current report's exact MCP schemas to a bound hosted
    /// session. This changes only hosted model visibility; it never starts the
    /// MCP provider on the device.
    pub(crate) async fn attach_device_mcp(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        plugin_id: &str,
        component_id: &str,
    ) -> Result<Vec<AttachedTool>> {
        let mut tx = self.pool.begin().await?;
        let session = sqlx::query(
            "SELECT bound_device_id,status FROM sessions WHERE id=$1 AND account_id=$2 FOR UPDATE",
        )
        .bind(session_id.as_str())
        .bind(&account.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::NotFound)?;
        if matches!(
            session.get::<String, _>("status").as_str(),
            "running" | "waiting_for_tool"
        ) {
            return Err(DeviceError::Conflict);
        }
        let device_id: String = session
            .get::<Option<String>, _>("bound_device_id")
            .ok_or(DeviceError::Conflict)?;
        let report = sqlx::query(
            "SELECT revision, capabilities_json FROM device_capability_reports r \
             JOIN device_presence p ON p.device_id=r.device_id AND p.lease_id=r.lease_id \
             WHERE r.device_id=$1 AND p.expires_at > now() ORDER BY r.created_at DESC LIMIT 1 FOR UPDATE OF r",
        )
        .bind(&device_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::Conflict)?;
        let snapshot: crate::plugin::PluginCapabilitySnapshot = serde_json::from_value(
            report
                .get::<Json<serde_json::Value>, _>("capabilities_json")
                .0,
        )
        .map_err(|_| DeviceError::Unavailable)?;
        let revision: String = report.get("revision");
        let attached = attach_mcp_from_snapshot(
            &mut tx,
            session_id,
            plugin_id,
            component_id,
            &snapshot,
            &revision,
        )
        .await
        .map_err(|_| DeviceError::Conflict)?;
        tx.commit().await?;
        Ok(attached)
    }

    /// Binds one account-owned session to an online device with a current
    /// capability report. The binding is intentionally immutable in v1:
    /// choosing a different Mac is a future explicit device-switch operation.
    pub(crate) async fn bind_session_device(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        device_id: DeviceId,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT bound_device_id FROM sessions WHERE id=$1 AND account_id=$2 FOR UPDATE",
        )
        .bind(session_id.as_str())
        .bind(&account.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::NotFound)?;
        if let Some(existing) = existing {
            if existing == device_id.to_string() {
                tx.commit().await?;
                return Ok(());
            }
            return Err(DeviceError::Conflict);
        }
        let executable: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM devices d
                JOIN device_presence p ON p.device_id=d.id AND p.expires_at > now()
                JOIN device_capability_reports r ON r.device_id=d.id AND r.lease_id=p.lease_id
                WHERE d.id=$1 AND d.account_id=$2 AND d.revoked_at IS NULL
            )",
        )
        .bind(device_id.to_string())
        .bind(&account.id)
        .fetch_one(&mut *tx)
        .await?;
        if !executable {
            return Err(DeviceError::Conflict);
        }
        sqlx::query("UPDATE sessions SET bound_device_id=$1, updated_at=now() WHERE id=$2")
            .bind(device_id.to_string())
            .bind(session_id.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Returns only the immutable browser-selected binding for an account-owned
    /// session. Device credentials, catalog details, and presence leases stay
    /// behind their respective endpoints.
    pub(crate) async fn session_bound_device_id(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> std::result::Result<Option<String>, HostedStoreError> {
        sqlx::query_scalar("SELECT bound_device_id FROM sessions WHERE id=$1 AND account_id=$2")
            .bind(session_id.as_str())
            .bind(&account.id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(HostedStoreError::SessionNotFound)
    }

    /// Saves a model's next attached tool call and waits for an explicit
    /// browser approval. The worker claim is yielded in the same transaction;
    /// no assignment exists until a user approves this exact call.
    pub(crate) async fn park_claimed_session_for_device_approval(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &crate::session::SessionExecutionClaim,
        content: &str,
        metadata: &crate::conversation::MessageMetadata,
        call: &crate::conversation::ToolCall,
        reason: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = claimed_session_for_update(&mut tx, &account.id, session_id, claim).await?;
        let assistant_message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, parent_message_id, role, content, metadata) \
             VALUES ($1,$2,$3,'assistant',$4,$5)",
        )
        .bind(&assistant_message_id)
        .bind(session.conversation_id.as_str())
        .bind(session.current_head_message_id.as_ref().map(|id| id.as_str()))
        .bind(content)
        .bind(Json(metadata))
        .execute(&mut *tx)
        .await?;
        if !content.is_empty() {
            insert_parts(
                &mut tx,
                &assistant_message_id,
                &[PreparedPart::Text(content.to_owned())],
            )
            .await?;
        }
        let (device_id, capability_revision, work, attachment) =
            device_work_for_call(&mut tx, &account.id, session_id, call).await?;
        match next_tool_action(call, Some(&attachment), true, ToolApprovalMode::Manual) {
            ToolAction::RequestApproval { .. } => {}
            // The hosted first version uses manual approval exclusively. Keep
            // unexpected future policy changes from becoming implicit work.
            ToolAction::Execute | ToolAction::PersistDenied(_) => {
                return Err(HostedStoreError::SessionConflict);
            }
        }
        let approval_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO device_tool_approvals \
             (id,account_id,session_id,assistant_message_id,result_parent_message_id,tool_call_id,tool_name,arguments_json,device_id,capability_revision,work_json,reason,status) \
             VALUES($1,$2,$3,$4,$4,$5,$6,$7,$8,$9,$10,$11,'pending')",
        )
        .bind(&approval_id)
        .bind(&account.id)
        .bind(session_id.as_str())
        .bind(&assistant_message_id)
        .bind(call.id.as_str())
        .bind(call.name())
        .bind(call.arguments())
        .bind(&device_id)
        .bind(&capability_revision)
        .bind(Json(work))
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        yield_claim_for_wait(
            &mut tx,
            session_id,
            claim,
            &assistant_message_id,
            "waiting_for_approval",
            None,
        )
        .await?;
        let revision = bump_revision(&mut tx, session.conversation_id.as_str()).await?;
        let saved = append_hosted_session_event(
            &mut tx,
            session_id,
            SessionEvent::AssistantMessageSaved {
                message_id: assistant_message_id.clone(),
            },
        )
        .await?;
        let waiting =
            append_hosted_session_event(&mut tx, session_id, SessionEvent::WaitingForApproval)
                .await?;
        append_event(
            &mut tx,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            serde_json::json!({"conversation_id": session.conversation_id.as_str(), "message_id": assistant_message_id, "revision": revision}),
        )
        .await?;
        tx.commit().await?;
        Ok(vec![saved, waiting])
    }

    /// Records a non-executable model call as the same linked tool failure the
    /// local policy produces. This keeps unknown, stale, or unavailable
    /// capability calls visible to the model instead of failing the entire
    /// session or silently dropping the assistant metadata.
    pub(crate) async fn park_claimed_session_with_denied_tool(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &crate::session::SessionExecutionClaim,
        content: &str,
        metadata: &crate::conversation::MessageMetadata,
        call: &crate::conversation::ToolCall,
        reason: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = claimed_session_for_update(&mut tx, &account.id, session_id, claim).await?;
        let assistant_message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
             VALUES($1,$2,$3,'assistant',$4,$5)",
        )
        .bind(&assistant_message_id)
        .bind(session.conversation_id.as_str())
        .bind(
            session
                .current_head_message_id
                .as_ref()
                .map(|id| id.as_str()),
        )
        .bind(content)
        .bind(Json(metadata))
        .execute(&mut *tx)
        .await?;
        if !content.is_empty() {
            insert_parts(
                &mut tx,
                &assistant_message_id,
                &[PreparedPart::Text(content.to_owned())],
            )
            .await?;
        }
        let result =
            crate::tool::ToolExecutionResult::failure(call.id.clone(), call.name(), reason);
        let tool_message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
             VALUES($1,$2,$3,'tool',$4,$5)",
        )
        .bind(&tool_message_id)
        .bind(session.conversation_id.as_str())
        .bind(&assistant_message_id)
        .bind(&result.content)
        .bind(Json(crate::conversation::MessageMetadata {
            tool_call_id: Some(result.tool_call_id),
            ..Default::default()
        }))
        .execute(&mut *tx)
        .await?;
        insert_parts(
            &mut tx,
            &tool_message_id,
            &[PreparedPart::Text(result.content.clone())],
        )
        .await?;
        yield_claim_for_wait(
            &mut tx,
            session_id,
            claim,
            &tool_message_id,
            "waiting_for_tool",
            None,
        )
        .await?;
        let revision = bump_revision(&mut tx, session.conversation_id.as_str()).await?;
        let assistant_saved = append_hosted_session_event(
            &mut tx,
            session_id,
            SessionEvent::AssistantMessageSaved {
                message_id: assistant_message_id,
            },
        )
        .await?;
        let tool_saved = append_hosted_session_event(
            &mut tx,
            session_id,
            SessionEvent::ToolResultSaved {
                message_id: tool_message_id.clone(),
            },
        )
        .await?;
        append_event(
            &mut tx,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            serde_json::json!({"conversation_id": session.conversation_id.as_str(), "message_id": tool_message_id, "revision": revision}),
        )
        .await?;
        let wakeup_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO wakeups(id,session_id,trigger_type,due_at,payload) VALUES($1,$2,'tool_result',now(),$3)",
        )
        .bind(wakeup_id)
        .bind(session_id.as_str())
        .bind(Json(serde_json::json!({"reason":"unavailable_tool", "revision":revision})))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(vec![assistant_saved, tool_saved])
    }

    /// Parks the next unresolved call from an already-saved multi-call
    /// assistant response. This preserves the local runtime's sequential
    /// tool-call ordering instead of asking the model to rediscover the next
    /// call after every result.
    pub(crate) async fn park_existing_device_tool_approval(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        claim: &crate::session::SessionExecutionClaim,
        assistant_message_id: &str,
        result_parent_message_id: &str,
        call: &crate::conversation::ToolCall,
        reason: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = claimed_session_for_update(&mut tx, &account.id, session_id, claim).await?;
        if session
            .current_head_message_id
            .as_ref()
            .map(|id| id.as_str())
            != Some(result_parent_message_id)
        {
            return Err(HostedStoreError::SessionConflict);
        }
        let (device_id, capability_revision, work, attachment) =
            device_work_for_call(&mut tx, &account.id, session_id, call).await?;
        match next_tool_action(call, Some(&attachment), true, ToolApprovalMode::Manual) {
            ToolAction::RequestApproval { .. } => {}
            ToolAction::Execute | ToolAction::PersistDenied(_) => {
                return Err(HostedStoreError::SessionConflict);
            }
        }
        let approval_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO device_tool_approvals \
             (id,account_id,session_id,assistant_message_id,result_parent_message_id,tool_call_id,tool_name,arguments_json,device_id,capability_revision,work_json,reason,status) \
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'pending')",
        )
        .bind(approval_id)
        .bind(&account.id)
        .bind(session_id.as_str())
        .bind(assistant_message_id)
        .bind(result_parent_message_id)
        .bind(call.id.as_str())
        .bind(call.name())
        .bind(call.arguments())
        .bind(device_id)
        .bind(capability_revision)
        .bind(Json(work))
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        yield_claim_for_wait(
            &mut tx,
            session_id,
            claim,
            result_parent_message_id,
            "waiting_for_approval",
            None,
        )
        .await?;
        let waiting =
            append_hosted_session_event(&mut tx, session_id, SessionEvent::WaitingForApproval)
                .await?;
        tx.commit().await?;
        Ok(vec![waiting])
    }

    /// Lists only still-pending, account-owned approvals. This is the hosted
    /// equivalent of the local session-approval read model; it contains no
    /// executable credential or local provider details.
    pub(crate) async fn pending_device_tool_approvals(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
    ) -> std::result::Result<Vec<HostedToolApproval>, HostedStoreError> {
        self.session(account, session_id).await?;
        let rows = sqlx::query(
            "SELECT id,session_id,assistant_message_id,tool_call_id,tool_name,arguments_json,device_id,reason \
             FROM device_tool_approvals WHERE account_id=$1 AND session_id=$2 AND status='pending' ORDER BY created_at,id",
        )
        .bind(&account.id)
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| HostedToolApproval {
                id: row.get("id"),
                session_id: row.get("session_id"),
                assistant_message_id: row.get("assistant_message_id"),
                tool_call_id: row.get("tool_call_id"),
                tool_name: row.get("tool_name"),
                arguments_json: row.get("arguments_json"),
                device_id: row.get("device_id"),
                reason: row.get("reason"),
            })
            .collect())
    }

    /// Converts one pending browser approval into a deliverable assignment.
    /// Current device presence and capability revision are checked again so a
    /// stale approval cannot broaden a later package/catalog update.
    pub(crate) async fn approve_device_tool_approval(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        approval_id: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = load_session_for_update(&mut tx, &account.id, session_id).await?;
        if session.status != crate::session::SessionStatus::WaitingForApproval {
            return Err(HostedStoreError::SessionConflict);
        }
        let approval = sqlx::query(
            "SELECT assistant_message_id,result_parent_message_id,tool_call_id,device_id,capability_revision,work_json,status \
             FROM device_tool_approvals WHERE id=$1 AND account_id=$2 AND session_id=$3 FOR UPDATE",
        )
        .bind(approval_id)
        .bind(&account.id)
        .bind(session_id.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(HostedStoreError::NotFound)?;
        if approval.get::<String, _>("status") != "pending" {
            return Err(HostedStoreError::SessionConflict);
        }
        let device_id: String = approval.get("device_id");
        let capability_revision: String = approval.get("capability_revision");
        let current = sqlx::query(
            "SELECT r.revision,r.capabilities_json FROM device_presence p JOIN device_capability_reports r \
             ON r.device_id=p.device_id AND r.lease_id=p.lease_id \
             JOIN devices d ON d.id=p.device_id \
             WHERE p.device_id=$1 AND d.account_id=$2 AND d.revoked_at IS NULL AND p.expires_at > now() \
             ORDER BY r.created_at DESC LIMIT 1 FOR UPDATE OF p",
        )
        .bind(&device_id)
        .bind(&account.id)
        .fetch_optional(&mut *tx)
        .await?;
        if current
            .as_ref()
            .map(|row| row.get::<String, _>("revision"))
            .as_deref()
            != Some(capability_revision.as_str())
        {
            return Err(HostedStoreError::SessionConflict);
        }
        let work: PendingToolWork =
            serde_json::from_value(approval.get::<Json<serde_json::Value>, _>("work_json").0)
                .map_err(|_| HostedStoreError::InvalidSession)?;
        if let PendingToolWork::AttachMcp {
            plugin_id,
            component_id,
        } = work
        {
            let snapshot: crate::plugin::PluginCapabilitySnapshot = serde_json::from_value(
                current
                    .as_ref()
                    .expect("validated current capability row")
                    .get::<Json<serde_json::Value>, _>("capabilities_json")
                    .0,
            )
            .map_err(|_| HostedStoreError::InvalidSession)?;
            let attached = attach_mcp_from_snapshot(
                &mut tx,
                session_id,
                &plugin_id,
                &component_id,
                &snapshot,
                &capability_revision,
            )
            .await?;
            let message_id = Uuid::new_v4().to_string();
            let content = if attached.is_empty() {
                "MCP is already attached; its current tool schemas remain available.".to_owned()
            } else {
                format!("Attached MCP with {} tool schema(s).", attached.len())
            };
            sqlx::query(
                "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
                 VALUES($1,$2,$3,'tool',$4,$5)",
            )
            .bind(&message_id)
            .bind(session.conversation_id.as_str())
            .bind(approval.get::<String, _>("result_parent_message_id"))
            .bind(&content)
            .bind(Json(crate::conversation::MessageMetadata {
                tool_call_id: Some(crate::conversation::ToolCallId::new(
                    approval.get::<String, _>("tool_call_id"),
                )),
                ..Default::default()
            }))
            .execute(&mut *tx)
            .await?;
            insert_parts(&mut tx, &message_id, &[PreparedPart::Text(content)]).await?;
            sqlx::query(
                "UPDATE device_tool_approvals SET status='approved',decided_at=now() WHERE id=$1",
            )
            .bind(approval_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE sessions SET current_head_message_id=$1,status='waiting_for_tool',updated_at=now() \
                 WHERE id=$2 AND status='waiting_for_approval'",
            )
            .bind(&message_id)
            .bind(session_id.as_str())
            .execute(&mut *tx)
            .await?;
            let revision = bump_revision(&mut tx, session.conversation_id.as_str()).await?;
            let saved = append_hosted_session_event(
                &mut tx,
                session_id,
                SessionEvent::ToolResultSaved {
                    message_id: message_id.clone(),
                },
            )
            .await?;
            append_event(
                &mut tx,
                account,
                "conversation.changed",
                Some(session.conversation_id.as_str()),
                Some(revision),
                serde_json::json!({"conversation_id": session.conversation_id.as_str(), "message_id": message_id, "revision": revision}),
            )
            .await?;
            sqlx::query(
                "INSERT INTO wakeups(id,session_id,trigger_type,due_at,payload) VALUES($1,$2,'tool_result',now(),$3)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(session_id.as_str())
            .bind(Json(serde_json::json!({"approval_id": approval_id, "revision": revision})))
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(vec![saved]);
        }
        let PendingToolWork::Device { work } = work else {
            unreachable!("attach work returned above")
        };
        let assignment_id = DeviceWorkId::new().to_string();
        let token = new_secret("work").map_err(|_| HostedStoreError::InvalidSession)?;
        sqlx::query(
            "INSERT INTO device_work_assignments \
             (id,account_id,device_id,session_id,assistant_message_id,result_parent_message_id,tool_call_id,capability_revision,work_json,execution_token,status,expires_at) \
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'pending',now() + interval '10 minutes')",
        )
        .bind(&assignment_id)
        .bind(&account.id)
        .bind(&device_id)
        .bind(session_id.as_str())
        .bind(approval.get::<String, _>("assistant_message_id"))
        .bind(approval.get::<String, _>("result_parent_message_id"))
        .bind(approval.get::<String, _>("tool_call_id"))
        .bind(&capability_revision)
        .bind(Json(work))
        .bind(token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE device_tool_approvals SET status='approved',decided_at=now() WHERE id=$1",
        )
        .bind(approval_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE sessions SET status='waiting_for_tool',active_assignment_id=$1,updated_at=now() \
             WHERE id=$2 AND status='waiting_for_approval'",
        )
        .bind(&assignment_id)
        .bind(session_id.as_str())
        .execute(&mut *tx)
        .await?;
        let waiting = append_hosted_session_event(
            &mut tx,
            session_id,
            SessionEvent::WaitingForTool {
                assignment_id,
                device_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(vec![waiting])
    }

    /// Persists a linked denied tool result and schedules the same fresh-claim
    /// continuation used after a real device result. A denial is never a fake
    /// user message and never contacts the device.
    pub(crate) async fn deny_device_tool_approval(
        &self,
        account: &HostedAccount,
        session_id: &SessionId,
        approval_id: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = load_session_for_update(&mut tx, &account.id, session_id).await?;
        if session.status != crate::session::SessionStatus::WaitingForApproval {
            return Err(HostedStoreError::SessionConflict);
        }
        let approval = sqlx::query(
            "SELECT assistant_message_id,result_parent_message_id,tool_call_id,tool_name,status \
             FROM device_tool_approvals WHERE id=$1 AND account_id=$2 AND session_id=$3 FOR UPDATE",
        )
        .bind(approval_id)
        .bind(&account.id)
        .bind(session_id.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(HostedStoreError::NotFound)?;
        if approval.get::<String, _>("status") != "pending" {
            return Err(HostedStoreError::SessionConflict);
        }
        let result = crate::tool::ToolExecutionResult::failure(
            crate::conversation::ToolCallId::new(approval.get::<String, _>("tool_call_id")),
            approval.get::<String, _>("tool_name"),
            "tool request denied by user",
        );
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
             VALUES($1,$2,$3,'tool',$4,$5)",
        )
        .bind(&message_id)
        .bind(session.conversation_id.as_str())
        .bind(approval.get::<String, _>("result_parent_message_id"))
        .bind(&result.content)
        .bind(Json(crate::conversation::MessageMetadata {
            tool_call_id: Some(result.tool_call_id),
            ..Default::default()
        }))
        .execute(&mut *tx)
        .await?;
        insert_parts(
            &mut tx,
            &message_id,
            &[PreparedPart::Text(result.content.clone())],
        )
        .await?;
        sqlx::query(
            "UPDATE device_tool_approvals SET status='denied',decided_at=now() WHERE id=$1",
        )
        .bind(approval_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE sessions SET current_head_message_id=$1,status='waiting_for_tool',updated_at=now() \
             WHERE id=$2 AND status='waiting_for_approval'",
        )
        .bind(&message_id)
        .bind(session_id.as_str())
        .execute(&mut *tx)
        .await?;
        let revision = bump_revision(&mut tx, session.conversation_id.as_str()).await?;
        let saved = append_hosted_session_event(
            &mut tx,
            session_id,
            SessionEvent::ToolResultSaved {
                message_id: message_id.clone(),
            },
        )
        .await?;
        append_event(
            &mut tx,
            account,
            "conversation.changed",
            Some(session.conversation_id.as_str()),
            Some(revision),
            serde_json::json!({"conversation_id": session.conversation_id.as_str(), "message_id": message_id, "revision": revision}),
        )
        .await?;
        let wakeup_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO wakeups(id,session_id,trigger_type,due_at,payload) VALUES($1,$2,'tool_result',now(),$3)",
        )
        .bind(wakeup_id)
        .bind(session_id.as_str())
        .bind(Json(serde_json::json!({"approval_id": approval_id, "revision": revision})))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(vec![saved])
    }

    /// Saves a device's current capability snapshot only while its current
    /// presence lease is valid. The report is descriptive, not permission.
    pub(crate) async fn publish_capabilities(
        &self,
        principal: &DevicePrincipal,
        report: &CapabilityReport,
    ) -> Result<CapabilityAccepted> {
        report.validate()?;
        let encoded =
            serde_json::to_value(&report.capabilities).map_err(|_| DeviceError::InvalidRequest)?;
        if serde_json::to_vec(&encoded)
            .map_err(|_| DeviceError::InvalidRequest)?
            .len()
            > 256 * 1024
        {
            return Err(DeviceError::InvalidRequest);
        }
        let mut tx = self.pool.begin().await?;
        let device_id = device_locked(&mut tx, principal).await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM device_presence WHERE device_id=$1 AND lease_id=$2 AND expires_at > now())",
        ).bind(device_id.to_string()).bind(report.lease_id.to_string()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(DeviceError::StaleLease);
        }
        let existing = sqlx::query(
            "SELECT revision, capabilities_json FROM device_capability_reports \
             WHERE device_id=$1 AND lease_id=$2 ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
        )
        .bind(device_id.to_string())
        .bind(report.lease_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let revision = match existing {
            Some(existing)
                if existing
                    .get::<Json<serde_json::Value>, _>("capabilities_json")
                    .0
                    == encoded =>
            {
                existing.get("revision")
            }
            _ => {
                let revision = CapabilityRevision::new().to_string();
                sqlx::query("INSERT INTO device_capability_reports(revision,device_id,lease_id,capabilities_json) VALUES($1,$2,$3,$4)")
                    .bind(&revision).bind(device_id.to_string()).bind(report.lease_id.to_string()).bind(Json(encoded))
                    .execute(&mut *tx).await?;
                revision
            }
        };
        tx.commit().await?;
        Ok(CapabilityAccepted {
            revision: CapabilityRevision(
                Uuid::parse_str(&revision).map_err(|_| DeviceError::Unavailable)?,
            ),
        })
    }

    /// Delivers one pending assignment for the current agent lease. Absence is
    /// a normal long-poll result; it never exposes another device's work.
    pub(crate) async fn next_device_work(
        &self,
        principal: &DevicePrincipal,
        lease: LeaseId,
    ) -> Result<Option<DeviceWorkAssignment>> {
        let mut tx = self.pool.begin().await?;
        let device_id = device_locked(&mut tx, principal).await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM device_presence WHERE device_id=$1 AND lease_id=$2 AND expires_at > now())")
            .bind(device_id.to_string()).bind(lease.to_string()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(DeviceError::StaleLease);
        }
        let row = sqlx::query("SELECT id,capability_revision,execution_token,work_json, floor(extract(epoch FROM expires_at))::bigint AS expires_at FROM device_work_assignments WHERE device_id=$1 AND status='pending' AND expires_at > now() ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1")
            .bind(device_id.to_string()).fetch_optional(&mut *tx).await?;
        let assignment = row
            .map(|row| -> Result<DeviceWorkAssignment> {
                Ok(DeviceWorkAssignment {
                    id: DeviceWorkId(
                        Uuid::parse_str(&row.get::<String, _>("id"))
                            .map_err(|_| DeviceError::Unavailable)?,
                    ),
                    lease_id: lease,
                    capability_revision: CapabilityRevision(
                        Uuid::parse_str(&row.get::<String, _>("capability_revision"))
                            .map_err(|_| DeviceError::Unavailable)?,
                    ),
                    execution_token: row.get("execution_token"),
                    work: serde_json::from_value(
                        row.get::<Json<serde_json::Value>, _>("work_json").0,
                    )
                    .map_err(|_| DeviceError::Unavailable)?,
                    expires_at: row.get("expires_at"),
                })
            })
            .transpose()?;
        tx.commit().await?;
        Ok(assignment)
    }

    /// Fences assignment start by credential, current lease, and its distinct
    /// delivery token. Retries report the already-started state rather than
    /// producing a second execution authorization.
    pub(crate) async fn start_device_work(
        &self,
        principal: &DevicePrincipal,
        id: DeviceWorkId,
        lease: LeaseId,
        token: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let device_id = device_locked(&mut tx, principal).await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM device_presence WHERE device_id=$1 AND lease_id=$2 AND expires_at > now())")
            .bind(device_id.to_string()).bind(lease.to_string()).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(DeviceError::StaleLease);
        }
        let status: Option<String> = sqlx::query_scalar("SELECT w.status FROM device_work_assignments w WHERE w.id=$1 AND w.device_id=$2 AND w.execution_token=$3 AND w.expires_at > now() AND w.capability_revision=(SELECT r.revision FROM device_capability_reports r WHERE r.device_id=$2 AND r.lease_id=$4 ORDER BY r.created_at DESC LIMIT 1) FOR UPDATE")
            .bind(id.to_string()).bind(device_id.to_string()).bind(token).bind(lease.to_string()).fetch_optional(&mut *tx).await?;
        match status.as_deref() {
            Some("pending") => {
                sqlx::query("UPDATE device_work_assignments SET status='executing',started_at=now() WHERE id=$1").bind(id.to_string()).execute(&mut *tx).await?;
            }
            Some("executing") => {}
            Some(_) => return Err(DeviceError::Conflict),
            None => return Err(DeviceError::NotFound),
        }
        tx.commit().await?;
        Ok(())
    }

    /// Accepts one exact journaled result after a fenced local execution. A
    /// second identical delivery is harmless; a different result for the same
    /// assignment is rejected rather than replacing the first tool outcome.
    pub(crate) async fn finish_device_work(
        &self,
        principal: &DevicePrincipal,
        id: DeviceWorkId,
        authorization: &DeviceWorkAuthorization,
        result: &DeviceWorkResult,
    ) -> Result<DeviceWorkResultAccepted> {
        if serde_json::to_vec(result)
            .map_err(|_| DeviceError::InvalidRequest)?
            .len()
            > 512 * 1024
        {
            return Err(DeviceError::InvalidRequest);
        }
        let result_parts = if result.parts.is_empty() {
            vec![crate::conversation::UnsavedMessagePart::Text(
                result.content.clone(),
            )]
        } else {
            result.parts.clone()
        };
        let parts =
            prepare_unsaved_parts(&result_parts).map_err(|_| DeviceError::InvalidRequest)?;
        let mut tx = self.pool.begin().await?;
        let device_id = device_locked(&mut tx, principal).await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM device_presence WHERE device_id=$1 AND lease_id=$2 AND expires_at > now())",
        )
        .bind(device_id.to_string())
        .bind(authorization.lease_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if !valid {
            return Err(DeviceError::StaleLease);
        }
        let assignment = sqlx::query(
            "SELECT w.account_id,w.session_id,w.assistant_message_id,w.result_parent_message_id,w.tool_call_id,w.status,w.result_json,a.auth_subject \
             FROM device_work_assignments w JOIN accounts a ON a.id=w.account_id \
             WHERE w.id=$1 AND w.device_id=$2 AND w.execution_token=$3 FOR UPDATE",
        )
        .bind(id.to_string())
        .bind(device_id.to_string())
        .bind(&authorization.execution_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::NotFound)?;
        let encoded = serde_json::to_value(result).map_err(|_| DeviceError::InvalidRequest)?;
        if assignment.get::<String, _>("status") == "result_saved" {
            let existing: serde_json::Value = assignment
                .get::<Option<Json<serde_json::Value>>, _>("result_json")
                .map(|value| value.0)
                .ok_or(DeviceError::Unavailable)?;
            if existing != encoded {
                return Err(DeviceError::Conflict);
            }
            tx.commit().await?;
            return Ok(DeviceWorkResultAccepted { accepted: true });
        }
        if assignment.get::<String, _>("status") != "executing" {
            return Err(DeviceError::Conflict);
        }
        let account_id: String = assignment.get("account_id");
        let session_id = SessionId::new(assignment.get::<String, _>("session_id"));
        let active: Option<String> = sqlx::query_scalar(
            "SELECT id FROM sessions WHERE id=$1 AND account_id=$2 AND status='waiting_for_tool' AND active_assignment_id=$3 FOR UPDATE",
        )
        .bind(session_id.as_str())
        .bind(&account_id)
        .bind(id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if active.is_none() {
            return Err(DeviceError::Conflict);
        }
        let conversation_id: String =
            sqlx::query_scalar("SELECT conversation_id FROM sessions WHERE id=$1")
                .bind(session_id.as_str())
                .fetch_one(&mut *tx)
                .await?;
        let message_id = Uuid::new_v4().to_string();
        let content = if result.content.is_empty() {
            prepared_content(&parts)
        } else {
            result.content.clone()
        };
        let metadata = crate::conversation::MessageMetadata {
            tool_call_id: Some(crate::conversation::ToolCallId::new(
                assignment.get::<String, _>("tool_call_id"),
            )),
            ..Default::default()
        };
        sqlx::query(
            "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
             VALUES($1,$2,$3,'tool',$4,$5)",
        )
        .bind(&message_id)
        .bind(&conversation_id)
        .bind(assignment.get::<String, _>("result_parent_message_id"))
        .bind(content)
        .bind(Json(metadata))
        .execute(&mut *tx)
        .await?;
        insert_parts(&mut tx, &message_id, &parts)
            .await
            .map_err(|_| DeviceError::Unavailable)?;
        sqlx::query(
            "UPDATE device_work_assignments SET status='result_saved',result_json=$1,completed_at=now() WHERE id=$2",
        )
        .bind(Json(encoded))
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE sessions SET current_head_message_id=$1,active_assignment_id=NULL,updated_at=now() WHERE id=$2",
        )
        .bind(&message_id)
        .bind(session_id.as_str())
        .execute(&mut *tx)
        .await?;
        let revision = bump_revision(&mut tx, &conversation_id)
            .await
            .map_err(|_| DeviceError::Unavailable)?;
        append_hosted_session_event(
            &mut tx,
            &session_id,
            SessionEvent::ToolResultSaved {
                message_id: message_id.clone(),
            },
        )
        .await
        .map_err(|_| DeviceError::Unavailable)?;
        append_event(
            &mut tx,
            &HostedAccount {
                id: account_id.clone(),
                auth_subject: assignment.get("auth_subject"),
            },
            "conversation.changed",
            Some(&conversation_id),
            Some(revision),
            serde_json::json!({"conversation_id": conversation_id, "message_id": message_id, "revision": revision}),
        )
        .await
        .map_err(|_| DeviceError::Unavailable)?;
        let wakeup_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO wakeups(id,session_id,trigger_type,due_at,payload) \
             VALUES($1,$2,'device_tool_result',now(),$3) \
             ON CONFLICT ((payload->>'assignment_id')) WHERE trigger_type='device_tool_result' DO NOTHING",
        )
        .bind(wakeup_id)
        .bind(session_id.as_str())
        .bind(Json(serde_json::json!({"assignment_id": id.to_string(), "revision": revision})))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(DeviceWorkResultAccepted { accepted: true })
    }

    /// Resolves one assignment that expired before the device fenced its
    /// execution start.  An expired pending delivery is not silently dropped:
    /// it becomes the same linked failure result the model would receive from
    /// any other unavailable tool, then resumes through a fresh wakeup claim.
    /// Executing assignments are deliberately left alone because their local
    /// side effects may already be in progress and must be recovered by the
    /// device journal instead of guessed by the server.
    pub(crate) async fn expire_one_pending_device_work(
        &self,
    ) -> std::result::Result<Vec<SessionEventRecord>, HostedStoreError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT w.id,w.account_id,w.session_id,w.result_parent_message_id,w.tool_call_id, \
                    s.conversation_id,s.active_assignment_id,a.auth_subject \
             FROM device_work_assignments w \
             JOIN sessions s ON s.id=w.session_id \
             JOIN accounts a ON a.id=w.account_id \
             WHERE w.status='pending' AND w.expires_at <= now() \
             ORDER BY w.expires_at,w.id FOR UPDATE OF w SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(Vec::new());
        };
        let assignment_id: String = row.get("id");
        sqlx::query(
            "UPDATE device_work_assignments SET status='expired' WHERE id=$1 AND status='pending'",
        )
        .bind(&assignment_id)
        .execute(&mut *tx)
        .await?;
        let session_id = SessionId::new(row.get::<String, _>("session_id"));
        let active_assignment: Option<String> = row.get("active_assignment_id");
        if active_assignment.as_deref() != Some(assignment_id.as_str()) {
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let account = HostedAccount {
            id: row.get("account_id"),
            auth_subject: row.get("auth_subject"),
        };
        let conversation_id: String = row.get("conversation_id");
        let tool_call_id: String = row.get("tool_call_id");
        let content = crate::tool::ToolExecutionResult::failure(
            crate::conversation::ToolCallId::new(tool_call_id.clone()),
            "device tool",
            "tool assignment expired before the device started it",
        )
        .content;
        let message_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO messages (id,conversation_id,parent_message_id,role,content,metadata) \
             VALUES($1,$2,$3,'tool',$4,$5)",
        )
        .bind(&message_id)
        .bind(&conversation_id)
        .bind(row.get::<String, _>("result_parent_message_id"))
        .bind(&content)
        .bind(Json(crate::conversation::MessageMetadata {
            tool_call_id: Some(crate::conversation::ToolCallId::new(tool_call_id)),
            ..Default::default()
        }))
        .execute(&mut *tx)
        .await?;
        insert_parts(&mut tx, &message_id, &[PreparedPart::Text(content)]).await?;
        let changed = sqlx::query(
            "UPDATE sessions SET current_head_message_id=$1,active_assignment_id=NULL,status='waiting_for_tool',updated_at=now() \
             WHERE id=$2 AND status='waiting_for_tool' AND active_assignment_id=$3",
        )
        .bind(&message_id)
        .bind(session_id.as_str())
        .bind(&assignment_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            // A concurrent stop or result may have cleared this assignment
            // after we selected it. The delivery is still safely expired, but
            // it is no longer the session's active work and therefore must
            // not manufacture a second tool result or retry forever.
            tx.commit().await?;
            return Ok(Vec::new());
        }
        let revision = bump_revision(&mut tx, &conversation_id).await?;
        let saved = append_hosted_session_event(
            &mut tx,
            &session_id,
            SessionEvent::ToolResultSaved {
                message_id: message_id.clone(),
            },
        )
        .await?;
        append_event(
            &mut tx,
            &account,
            "conversation.changed",
            Some(&conversation_id),
            Some(revision),
            serde_json::json!({"conversation_id": conversation_id, "message_id": message_id, "revision": revision}),
        )
        .await?;
        sqlx::query(
            "INSERT INTO wakeups(id,session_id,trigger_type,due_at,payload) VALUES($1,$2,'tool_result',now(),$3)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(session_id.as_str())
        .bind(Json(serde_json::json!({"assignment_id": assignment_id, "revision": revision})))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(vec![saved])
    }
    /// Atomic fixed-window limiter, shared by every API process. Buckets contain hashes only.
    pub(crate) async fn device_rate_limit(
        &self,
        bucket: &str,
        maximum: i32,
        seconds: i32,
    ) -> Result<()> {
        let count: i32 = sqlx::query_scalar("INSERT INTO device_rate_limits (bucket, window_start, count) VALUES ($1, now(), 1) ON CONFLICT(bucket) DO UPDATE SET window_start = CASE WHEN device_rate_limits.window_start + make_interval(secs => $2) <= now() THEN now() ELSE device_rate_limits.window_start END, count = CASE WHEN device_rate_limits.window_start + make_interval(secs => $2) <= now() THEN 1 ELSE LEAST(device_rate_limits.count + 1, $3 + 1) END RETURNING count")
            .bind(bucket).bind(seconds as f64).bind(maximum).fetch_one(&self.pool).await?;
        if count > maximum {
            Err(DeviceError::RateLimited)
        } else {
            Ok(())
        }
    }

    /// Expiry removes pending secrets after a one-day recovery/debug window; audit lasts 90 days.
    pub(crate) async fn cleanup_devices(&self) -> Result<()> {
        sqlx::query("DELETE FROM device_enrollments WHERE expires_at < now() - interval '1 day'")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM device_rate_limits WHERE window_start < now() - interval '1 hour'",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM device_audit WHERE created_at < now() - interval '90 days'")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn initiate_device(
        &self,
        request: &EnrollmentRequest,
        key: &[u8],
    ) -> Result<EnrollmentStarted> {
        request.validate()?;
        let fingerprint =
            hash(&serde_json::to_string(request).map_err(|_| DeviceError::InvalidRequest)?);
        let key_version = keyed(key, "key-version", "v1");
        let mut tx = self.pool.begin().await?;
        // Serialize identical initiation retries, including the insert-not-yet-visible race.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(request.request_id.to_string())
            .execute(&mut *tx)
            .await?;
        let existing = sqlx::query(&format!("{ENROLL_SELECT} WHERE request_id = $1 FOR UPDATE"))
            .bind(request.request_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let (id, expires_at) = if let Some(row) = existing {
            if row.get::<String, _>("fingerprint") != fingerprint {
                return Err(DeviceError::Conflict);
            }
            if row.get::<bool, _>("expired") || row.get::<String, _>("key_version") != key_version {
                return Err(DeviceError::Expired);
            }
            let v = view(&row)?;
            (v.id, v.expires_at)
        } else {
            let mut inserted = None;
            for _ in 0..4 {
                let id = EnrollmentId::new();
                let code_digest = keyed(
                    key,
                    "code-lookup",
                    &normalize_code(&enrollment_code(key, id))?,
                );
                let expiry: Option<i64> = sqlx::query_scalar("INSERT INTO device_enrollments (id, request_id, fingerprint, enrollment_digest, device_digest, code_digest, key_version, metadata, state, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'pending',now() + make_interval(secs => $9)) ON CONFLICT DO NOTHING RETURNING floor(extract(epoch FROM expires_at))::bigint")
                    .bind(id.to_string()).bind(request.request_id.to_string()).bind(&fingerprint)
                    .bind(&request.enrollment_digest).bind(&request.device_digest).bind(code_digest).bind(&key_version)
                    .bind(Json(&request.metadata)).bind(ENROLLMENT_SECONDS as f64).fetch_optional(&mut *tx).await?;
                if let Some(expiry) = expiry {
                    inserted = Some((id, expiry));
                    break;
                }
            }
            inserted.ok_or(DeviceError::Conflict)?
        };
        tx.commit().await?;
        Ok(EnrollmentStarted {
            id,
            code: enrollment_code(key, id),
            verification_url: PAIRING_URL.into(),
            expires_at,
            poll_interval_seconds: POLL_SECONDS,
        })
    }

    /// Preview/approve/deny use a verified account and never return enrollment credentials.
    pub(crate) async fn device_code_action(
        &self,
        account: &HostedAccount,
        label: &str,
        code: &str,
        action: CodeAction,
        key: &[u8],
    ) -> Result<EnrollmentView> {
        let digest = keyed(key, "code-lookup", &normalize_code(code)?);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(&format!(
            "{ENROLL_SELECT} WHERE code_digest=$1 AND key_version=$2 FOR UPDATE"
        ))
        .bind(digest)
        .bind(keyed(key, "key-version", "v1"))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::NotFound)?;
        let mut v = view(&row)?;
        if v.state == EnrollmentState::Expired {
            return Err(DeviceError::NotFound);
        }
        if v.account_id.as_ref().is_some_and(|id| id != &account.id) {
            return Err(DeviceError::NotFound);
        }
        match action {
            CodeAction::Lookup => {
                if !matches!(
                    v.state,
                    EnrollmentState::Pending | EnrollmentState::Approved
                ) {
                    return Err(DeviceError::NotFound);
                }
            }
            CodeAction::Approve => {
                if v.state == EnrollmentState::Pending {
                    sqlx::query("UPDATE device_enrollments SET state='approved',account_id=$2,account_label=$3 WHERE id=$1")
                        .bind(v.id.to_string()).bind(&account.id).bind(label).execute(&mut *tx).await?;
                    v.state = EnrollmentState::Approved;
                    v.account_id = Some(account.id.clone());
                    v.account_label = Some(label.into());
                } else if !matches!(
                    v.state,
                    EnrollmentState::Approved | EnrollmentState::Consumed
                ) {
                    return Err(DeviceError::Conflict);
                }
            }
            CodeAction::Deny => {
                if v.state != EnrollmentState::Pending {
                    return Err(DeviceError::Conflict);
                }
                sqlx::query("UPDATE device_enrollments SET state='denied' WHERE id=$1")
                    .bind(v.id.to_string())
                    .execute(&mut *tx)
                    .await?;
                v.state = EnrollmentState::Denied;
            }
        }
        tx.commit().await?;
        Ok(v)
    }

    pub(crate) async fn poll_device_enrollment(
        &self,
        principal: &EnrollmentPrincipal,
        key: &[u8],
    ) -> Result<EnrollmentView> {
        let mut tx = self.pool.begin().await?;
        let row = enrollment_locked(&mut tx, principal, key).await?;
        let allowed: bool = sqlx::query_scalar("UPDATE device_enrollments SET last_poll_at=now() WHERE id=$1 AND (last_poll_at IS NULL OR last_poll_at <= now() - make_interval(secs => $2)) RETURNING true")
            .bind(principal.id.to_string()).bind(POLL_SECONDS as f64).fetch_optional(&mut *tx).await?.unwrap_or(false);
        if !allowed {
            return Err(DeviceError::RateLimited);
        }
        let v = view(&row)?;
        tx.commit().await?;
        Ok(v)
    }

    pub(crate) async fn cancel_device_enrollment(
        &self,
        principal: &EnrollmentPrincipal,
        key: &[u8],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let row = enrollment_locked(&mut tx, principal, key).await?;
        if row.get::<String, _>("state") == "consumed" {
            return Err(DeviceError::Conflict);
        }
        sqlx::query("UPDATE device_enrollments SET state='cancelled',account_id=NULL,account_label=NULL WHERE id=$1")
            .bind(principal.id.to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn finalize_device(
        &self,
        principal: &EnrollmentPrincipal,
        account_id: &str,
        key: &[u8],
    ) -> Result<DeviceId> {
        let mut tx = self.pool.begin().await?;
        let row = enrollment_locked(&mut tx, principal, key).await?;
        let v = view(&row)?;
        if v.state == EnrollmentState::Expired {
            return Err(DeviceError::Expired);
        }
        if v.account_id.as_deref() != Some(account_id) {
            return Err(DeviceError::Conflict);
        }
        if let Some(id) = v.device_id {
            return Ok(id);
        }
        if v.state != EnrollmentState::Approved {
            return Err(DeviceError::Conflict);
        }
        let id = DeviceId::new();
        sqlx::query("INSERT INTO devices (id,account_id,metadata) VALUES ($1,$2,$3)")
            .bind(id.to_string())
            .bind(account_id)
            .bind(Json(v.metadata))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO device_credentials (digest,device_id) VALUES ($1,$2)")
            .bind(row.get::<String, _>("device_digest"))
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE device_enrollments SET state='consumed',device_id=$2 WHERE id=$1")
            .bind(principal.id.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        audit(&mut tx, id, "registered").await?;
        tx.commit().await?;
        Ok(id)
    }

    pub(crate) async fn list_devices(&self, account: &HostedAccount) -> Result<Vec<DeviceView>> {
        let rows = sqlx::query(&format!(
            "{DEVICE_SELECT} WHERE d.account_id=$1 ORDER BY d.created_at,d.id"
        ))
        .bind(&account.id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(device_view).collect()
    }
    pub(crate) async fn revoke_device(&self, account: &HostedAccount, id: DeviceId) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let was_revoked: bool = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM devices WHERE id=$1 AND account_id=$2 FOR UPDATE",
        )
        .bind(id.to_string())
        .bind(&account.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DeviceError::NotFound)?;
        sqlx::query("UPDATE devices SET revoked_at=COALESCE(revoked_at,now()) WHERE id=$1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE device_credentials SET revoked_at=COALESCE(revoked_at,now()) WHERE device_id=$1").bind(id.to_string()).execute(&mut *tx).await?;
        sqlx::query("UPDATE device_presence SET expires_at=now() WHERE device_id=$1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        if !was_revoked {
            audit(&mut tx, id, "revoked").await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn device_self(&self, principal: &DevicePrincipal) -> Result<DeviceView> {
        let mut tx = self.pool.begin().await?;
        let id = device_locked(&mut tx, principal).await?;
        let row = sqlx::query(&format!("{DEVICE_SELECT} WHERE d.id=$1"))
            .bind(id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let v = device_view(&row)?;
        tx.commit().await?;
        Ok(v)
    }
    pub(crate) async fn connect_device(
        &self,
        principal: &DevicePrincipal,
        request: &ConnectRequest,
    ) -> Result<Lease> {
        if request.protocol_version != PROTOCOL_VERSION {
            return Err(DeviceError::UnsupportedProtocol);
        }
        let mut tx = self.pool.begin().await?;
        let id = device_locked(&mut tx, principal).await?;
        let row=sqlx::query("SELECT instance_id, lease_id, expires_at > now() AS active FROM device_presence WHERE device_id=$1")
            .bind(id.to_string()).fetch_optional(&mut *tx).await?;
        let lease_id = if let Some(row) = row {
            if row.get::<bool, _>("active") {
                if row.get::<String, _>("instance_id") != request.instance_id.to_string() {
                    return Err(DeviceError::AlreadyRunning);
                }
                LeaseId(
                    Uuid::parse_str(&row.get::<String, _>("lease_id"))
                        .map_err(|_| DeviceError::Unavailable)?,
                )
            } else {
                LeaseId::new()
            }
        } else {
            LeaseId::new()
        };
        let expires_at: i64=sqlx::query_scalar("INSERT INTO device_presence(device_id,instance_id,lease_id,last_seen,expires_at) VALUES($1,$2,$3,now(),now()+make_interval(secs => $4)) ON CONFLICT(device_id) DO UPDATE SET instance_id=$2,lease_id=$3,last_seen=now(),expires_at=now()+make_interval(secs => $4) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(id.to_string()).bind(request.instance_id.to_string()).bind(lease_id.to_string()).bind(LEASE_SECONDS as f64).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Lease {
            lease_id,
            expires_at,
        })
    }
    pub(crate) async fn heartbeat_device(
        &self,
        principal: &DevicePrincipal,
        lease: LeaseId,
        release: bool,
    ) -> Result<Lease> {
        let mut tx = self.pool.begin().await?;
        let id = device_locked(&mut tx, principal).await?;
        let expiry: Option<i64>=sqlx::query_scalar("UPDATE device_presence SET expires_at=CASE WHEN $3 THEN now() ELSE now()+make_interval(secs => $4) END, last_seen=CASE WHEN $3 THEN last_seen ELSE now() END WHERE device_id=$1 AND lease_id=$2 AND (expires_at > now() OR $3) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(id.to_string()).bind(lease.to_string()).bind(release).bind(LEASE_SECONDS as f64).fetch_optional(&mut *tx).await?;
        let expires_at = expiry.ok_or(DeviceError::StaleLease)?;
        tx.commit().await?;
        Ok(Lease {
            lease_id: lease,
            expires_at,
        })
    }
}

/// Resolves an ordinary model-facing schema through the immutable session
/// attachment and the *current* report for the bound device. The hosted server
/// only builds a typed delivery; the Mac performs the registry lookup again
/// immediately before execution.
async fn device_work_for_call(
    transaction: &mut Transaction<'_, Postgres>,
    account_id: &str,
    session_id: &SessionId,
    call: &crate::conversation::ToolCall,
) -> std::result::Result<(String, String, PendingToolWork, AttachedTool), HostedStoreError> {
    if call.arguments().len() > 64 * 1024
        || serde_json::from_str::<serde_json::Value>(call.arguments()).is_err()
    {
        return Err(HostedStoreError::SessionConflict);
    }
    let device_id: String = sqlx::query_scalar(
        "SELECT bound_device_id FROM sessions WHERE id=$1 AND account_id=$2 AND bound_device_id IS NOT NULL FOR UPDATE",
    )
    .bind(session_id.as_str())
    .bind(account_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::SessionConflict)?;
    let report = sqlx::query(
        "SELECT r.revision,r.capabilities_json FROM device_presence p JOIN device_capability_reports r \
         ON r.device_id=p.device_id AND r.lease_id=p.lease_id JOIN devices d ON d.id=p.device_id \
         WHERE p.device_id=$1 AND d.account_id=$2 AND d.revoked_at IS NULL AND p.expires_at > now() \
         ORDER BY r.created_at DESC LIMIT 1 FOR UPDATE OF p",
    )
    .bind(&device_id)
    .bind(account_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::SessionConflict)?;
    let revision: String = report.get("revision");
    let snapshot: crate::plugin::PluginCapabilitySnapshot = serde_json::from_value(
        report
            .get::<Json<serde_json::Value>, _>("capabilities_json")
            .0,
    )
    .map_err(|_| HostedStoreError::InvalidSession)?;

    if matches!(call.name(), "windie__read_skill" | "windie__attach_mcp") {
        let builtin = ToolProviderRegistry::new()
            .builtin_tool(&ToolSchemaName::new(call.name()))
            .ok_or(HostedStoreError::SessionConflict)?
            .attached_tool();
        let request = ControlRequest::parse(builtin.provider.tool_name.as_str(), call)
            .map_err(|_| HostedStoreError::SessionConflict)?;
        let work = match request {
            ControlRequest::ReadSkill {
                plugin_id,
                skill_id,
            } => {
                let skill_exists = snapshot.index.installed.iter().any(|plugin| {
                    plugin.id == plugin_id && plugin.skills.iter().any(|skill| skill.id == skill_id)
                });
                if !skill_exists {
                    return Err(HostedStoreError::SessionConflict);
                }
                PendingToolWork::Device {
                    work: DeviceWork::ReadSkill {
                        tool_call_id: call.id.as_str().to_owned(),
                        plugin_id,
                        skill_id,
                    },
                }
            }
            ControlRequest::AttachMcp { plugin_id, mcp_id } => {
                validate_mcp_membership(
                    &plugin_id,
                    &mcp_id,
                    snapshot
                        .providers
                        .iter()
                        .filter(|provider| provider.plugin_id == plugin_id)
                        .map(|provider| provider.component_id.as_str()),
                )
                .map_err(|_| HostedStoreError::SessionConflict)?;
                PendingToolWork::AttachMcp {
                    plugin_id,
                    component_id: mcp_id,
                }
            }
        };
        return Ok((device_id, revision, work, builtin));
    }
    let attachment = sqlx::query(
        "SELECT plugin_id,component_id,provider_id,provider_tool_name,provider_kind,schema_json,permissions_json,annotations_json \
         FROM hosted_tool_attachments WHERE session_id=$1 AND schema_name=$2 AND capability_revision=$3 FOR UPDATE",
    )
    .bind(session_id.as_str())
    .bind(call.name())
    .bind(&revision)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(HostedStoreError::SessionConflict)?;
    if attachment.get::<String, _>("provider_kind") != ToolProviderKind::Mcp.as_storage() {
        return Err(HostedStoreError::SessionConflict);
    }
    let schema: ToolSchema = serde_json::from_value(
        attachment
            .get::<Json<serde_json::Value>, _>("schema_json")
            .0,
    )
    .map_err(|_| HostedStoreError::InvalidSession)?;
    let attached = AttachedTool {
        schema_name: schema.name,
        description: schema.description,
        parameters: schema.parameters,
        provider: ToolProviderRef::new(
            ToolProviderId::new(attachment.get::<String, _>("provider_id")),
            ProviderToolName::new(attachment.get::<String, _>("provider_tool_name")),
            ToolProviderKind::Mcp,
        ),
        permissions: serde_json::from_value(
            attachment
                .get::<Json<serde_json::Value>, _>("permissions_json")
                .0,
        )
        .map_err(|_| HostedStoreError::InvalidSession)?,
        annotations: serde_json::from_value(
            attachment
                .get::<Json<serde_json::Value>, _>("annotations_json")
                .0,
        )
        .map_err(|_| HostedStoreError::InvalidSession)?,
    };
    Ok((
        device_id,
        revision,
        PendingToolWork::Device {
            work: DeviceWork::McpCall {
                tool_call_id: call.id.as_str().to_owned(),
                plugin_id: attachment.get("plugin_id"),
                component_id: attachment.get("component_id"),
                provider_id: attachment.get("provider_id"),
                schema_name: call.name().to_owned(),
                tool_name: attachment.get("provider_tool_name"),
                schema: attached.schema(),
                arguments: call.arguments().to_owned(),
            },
        },
        attached,
    ))
}

/// Persists the exact schemas from one current, already-validated device
/// report. Both the explicit browser route and approved `attach_mcp` control
/// call use this adapter around the shared attachment planner.
async fn attach_mcp_from_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: &SessionId,
    plugin_id: &str,
    component_id: &str,
    snapshot: &crate::plugin::PluginCapabilitySnapshot,
    revision: &str,
) -> std::result::Result<Vec<AttachedTool>, HostedStoreError> {
    validate_mcp_membership(
        plugin_id,
        component_id,
        snapshot
            .providers
            .iter()
            .filter(|provider| provider.plugin_id == plugin_id)
            .map(|provider| provider.component_id.as_str()),
    )
    .map_err(|_| HostedStoreError::SessionConflict)?;
    let provider = snapshot
        .providers
        .iter()
        .find(|provider| provider.plugin_id == plugin_id && provider.component_id == component_id)
        .ok_or(HostedStoreError::SessionConflict)?;
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT schema_name FROM hosted_tool_attachments WHERE session_id=$1 FOR UPDATE",
    )
    .bind(session_id.as_str())
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(ToolSchemaName::new)
    .collect::<HashSet<_>>();
    let attached = plan_provider_attachment(
        &provider.provider_id,
        AttachmentFacts {
            registered: true,
            enabled: provider.state == crate::plugin::PluginState::Enabled,
            unavailable: false,
            tools: Some(&provider.tools),
        },
        &existing,
    )
    .map_err(|_| HostedStoreError::SessionConflict)?;
    for tool in &attached {
        sqlx::query(
            "INSERT INTO hosted_tool_attachments \
             (session_id,schema_name,plugin_id,component_id,provider_id,provider_tool_name,provider_kind,capability_revision,schema_json,permissions_json,annotations_json) \
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(session_id.as_str())
        .bind(tool.schema_name.as_str())
        .bind(plugin_id)
        .bind(component_id)
        .bind(tool.provider.provider_id.as_str())
        .bind(tool.provider.tool_name.as_str())
        .bind(tool.provider.kind.as_storage())
        .bind(revision)
        .bind(Json(tool.schema()))
        .bind(Json(&tool.permissions))
        .bind(Json(&tool.annotations))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(attached)
}

/// Releases a hosted claim without pretending the model turn completed.
/// Approval and device waits resume only through later, fresh claims.
async fn yield_claim_for_wait(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: &SessionId,
    claim: &crate::session::SessionExecutionClaim,
    current_head_message_id: &str,
    status: &str,
    assignment_id: Option<&str>,
) -> std::result::Result<(), HostedStoreError> {
    sqlx::query(
        "UPDATE sessions SET current_head_message_id=$1,status=$2,active_assignment_id=$3, \
         execution_owner=NULL,current_claim_id=NULL,updated_at=now() \
         WHERE id=$4 AND current_claim_id=$5",
    )
    .bind(current_head_message_id)
    .bind(status)
    .bind(assignment_id)
    .bind(session_id.as_str())
    .bind(claim.id.as_str())
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "UPDATE session_execution_claims SET status='yielded',released_at=now() WHERE id=$1 AND status='active'",
    )
    .bind(claim.id.as_str())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum CodeAction {
    Lookup,
    Approve,
    Deny,
}

async fn enrollment_locked(
    tx: &mut Transaction<'_, Postgres>,
    p: &EnrollmentPrincipal,
    key: &[u8],
) -> Result<sqlx::postgres::PgRow> {
    let row = sqlx::query(&format!(
        "{ENROLL_SELECT} WHERE id=$1 AND enrollment_digest=$2 FOR UPDATE"
    ))
    .bind(p.id.to_string())
    .bind(&p.digest)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(DeviceError::Unauthorized)?;
    if row.get::<String, _>("key_version") != keyed(key, "key-version", "v1") {
        return Err(DeviceError::Expired);
    }
    Ok(row)
}
/// Lock device before credential validation, matching revoke's lock ordering.
async fn device_locked(
    tx: &mut Transaction<'_, Postgres>,
    p: &DevicePrincipal,
) -> Result<DeviceId> {
    let id: String = sqlx::query_scalar("SELECT device_id FROM device_credentials WHERE digest=$1")
        .bind(&p.digest)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(DeviceError::Unauthorized)?;
    let active: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM devices WHERE id=$1 FOR UPDATE")
            .bind(&id)
            .fetch_one(&mut **tx)
            .await?;
    let credential_active: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM device_credentials WHERE digest=$1")
            .bind(&p.digest)
            .fetch_one(&mut **tx)
            .await?;
    if !active || !credential_active {
        return Err(DeviceError::Unauthorized);
    }
    Ok(DeviceId(
        Uuid::parse_str(&id).map_err(|_| DeviceError::Unavailable)?,
    ))
}
const DEVICE_SELECT: &str = "SELECT d.id,d.metadata,d.revoked_at IS NOT NULL AS revoked,(d.revoked_at IS NULL AND COALESCE(p.expires_at > now(),false)) AS online,floor(extract(epoch FROM p.last_seen))::bigint AS last_seen FROM devices d LEFT JOIN device_presence p ON p.device_id=d.id";
fn device_view(row: &sqlx::postgres::PgRow) -> Result<DeviceView> {
    Ok(DeviceView {
        id: DeviceId(
            Uuid::parse_str(&row.get::<String, _>("id")).map_err(|_| DeviceError::Unavailable)?,
        ),
        metadata: serde_json::from_value(row.get("metadata"))
            .map_err(|_| DeviceError::Unavailable)?,
        revoked: row.get("revoked"),
        online: row.get("online"),
        last_seen: row.get("last_seen"),
    })
}
async fn audit(tx: &mut Transaction<'_, Postgres>, id: DeviceId, event: &str) -> Result<()> {
    sqlx::query("INSERT INTO device_audit(device_id,event) VALUES($1,$2)")
        .bind(id.to_string())
        .bind(event)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
