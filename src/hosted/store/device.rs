//! Account-scoped enrollment and fenced presence transactions, independent of sessions.

use super::*;
use crate::device::*;

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
