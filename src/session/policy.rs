//! Storage-independent durable-session policy.
//!
//! SQLite and PostgreSQL each make persistence atomic in their own way. This
//! module owns the small set of decisions that must remain identical: resolving
//! a branch head to zero/one/many sessions and deciding whether a loaded
//! unclaimed session may begin a requested execution.

use super::{Session, SessionExecutionStart, SessionResolution, SessionStatus};

/// Resolves a set of sessions that currently end at one canonical tree head.
///
/// Backends deliberately do not choose an arbitrary session when storage
/// contains more than one matching branch; the caller must surface the
/// ambiguity instead.
pub(crate) fn resolve_sessions_at_head(sessions: Vec<Session>) -> SessionResolution {
    match sessions.len() {
        0 => SessionResolution::NoSessionAtHead,
        1 => {
            SessionResolution::Existing(Box::new(sessions.into_iter().next().expect("one session")))
        }
        _ => SessionResolution::Ambiguous(sessions),
    }
}

/// Returns whether an unclaimed session can transition to `Running` now.
///
/// The persistence adapter must still perform its row lock and claim update in
/// one transaction. This function intentionally has no database knowledge.
pub(crate) fn can_start(session: &Session, start: &SessionExecutionStart, now_millis: i64) -> bool {
    match start {
        SessionExecutionStart::Runnable => !matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        ),
        SessionExecutionStart::RunnableAtHead(head) => {
            session.current_head_message_id.as_ref() == head.as_ref()
                && !matches!(
                    session.status,
                    SessionStatus::Running | SessionStatus::WaitingForApproval
                )
        }
        SessionExecutionStart::WaitingForApproval => {
            session.status == SessionStatus::WaitingForApproval
        }
        SessionExecutionStart::IdleWakeup {
            eligible_before,
            interval,
        } => {
            session.keep_awake
                && session.idle_wakeup_interval == *interval
                && session.last_user_activity_at <= *eligible_before
                && session
                    .last_idle_wakeup_completed_at
                    .is_none_or(|completed| completed <= *eligible_before)
                && now_millis >= *eligible_before
                && !matches!(
                    session.status,
                    SessionStatus::Running | SessionStatus::WaitingForApproval
                )
        }
        SessionExecutionStart::ManualWakeup => !matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationId;
    use crate::session::{IdleWakeupInterval, SessionId};

    fn session(status: SessionStatus) -> Session {
        Session {
            id: SessionId::new("session"),
            conversation_id: ConversationId::new("conversation"),
            start_head_message_id: None,
            current_head_message_id: None,
            status,
            model: "windie/test".to_string(),
            reasoning: None,
            error: None,
            keep_awake: false,
            idle_wakeup_interval: IdleWakeupInterval::ThirtyMinutes,
            last_user_activity_at: 0,
            last_idle_wakeup_completed_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn runnable_never_claims_running_or_approval_sessions() {
        assert!(!can_start(
            &session(SessionStatus::Running),
            &SessionExecutionStart::Runnable,
            1,
        ));
        assert!(!can_start(
            &session(SessionStatus::WaitingForApproval),
            &SessionExecutionStart::Runnable,
            1,
        ));
    }
}
