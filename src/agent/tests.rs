//! Loopback HTTP and protected-file proofs; no production credentials or LLM calls.

use super::*;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn serve(router: Router) -> (Client, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (client, task)
}

#[tokio::test]
async fn redirects_and_errors_never_leak_or_forward_credentials() {
    let redirected = Arc::new(AtomicUsize::new(0));
    let hits = redirected.clone();
    let (other, other_task) = serve(Router::new().route(
        "/target",
        get(move || {
            hits.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({})) }
        }),
    ))
    .await;
    let target = format!("{}/target", other.server);
    let (client, task) = serve(
        Router::new()
            .route(
                "/redirect",
                get(
                    move || async move { (StatusCode::TEMPORARY_REDIRECT, [("location", target)]) },
                ),
            )
            .route(
                "/busy",
                get(|| async {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "60")],
                        Json(json!({"error":"rate_limited"})),
                    )
                }),
            )
            .route(
                "/unavailable",
                get(|| async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "DO NOT EXPOSE THIS SERVER BODY",
                    )
                }),
            ),
    )
    .await;
    let secret = new_secret("device").unwrap();
    let e = client
        .request::<Value>(
            reqwest::Method::GET,
            "/redirect",
            Some(&secret),
            None::<&()>,
        )
        .await
        .unwrap_err();
    assert!(!e.retryable());
    assert_eq!(redirected.load(Ordering::SeqCst), 0);
    assert!(!format!("{e:?}").contains(&secret));
    let e = client
        .request::<Value>(reqwest::Method::GET, "/busy", Some(&secret), None::<&()>)
        .await
        .unwrap_err();
    assert!(e.retryable());
    assert_eq!(e.retry_after, 60);
    let e = client
        .request::<Value>(
            reqwest::Method::GET,
            "/unavailable",
            Some(&secret),
            None::<&()>,
        )
        .await
        .unwrap_err();
    assert!(e.retryable());
    assert!(!format!("{e:?}").contains("EXPOSE"));
    task.abort();
    other_task.abort();
}

#[tokio::test]
async fn presence_stops_after_one_rejected_credential_without_local_runtime() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let (client, task) = serve(Router::new().route(
        "/v1/agent/connect",
        post(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::UNAUTHORIZED }
        }),
    ))
    .await;
    let credentials = fresh_credentials(client.server.clone()).unwrap();
    let mut lease = None;
    let error = presence(&client, &credentials, &mut lease, |_| {})
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<AgentError>().unwrap().code,
        DeviceError::Unauthorized
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(lease.is_none());
    task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn completed_journal_retries_only_the_saved_result_after_lost_acknowledgement() {
    use crate::agent::journal::{WorkJournal, WorkJournalState};

    let root = std::env::temp_dir().join(format!("windie-agent-recovery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let storage = storage::Storage::at(root.join("agent")).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let result_hits = hits.clone();
    let assignment_id = DeviceWorkId::new();
    let (client, task) = serve(Router::new().route(
        "/v1/agent/work/{id}/result",
        post(move || {
            let result_hits = result_hits.clone();
            async move {
                result_hits.fetch_add(1, Ordering::SeqCst);
                Json(DeviceWorkResultAccepted { accepted: true })
            }
        }),
    ))
    .await;
    let credentials = fresh_credentials(client.server.clone()).unwrap();
    let assignment = DeviceWorkAssignment {
        id: assignment_id,
        lease_id: LeaseId::new(),
        capability_revision: CapabilityRevision::new(),
        execution_token: "test-token".into(),
        work: DeviceWork::ReadSkill {
            tool_call_id: "call-1".into(),
            plugin_id: "plugin".into(),
            skill_id: "skill".into(),
        },
        expires_at: i64::MAX,
    };
    let journal = WorkJournal::new(&storage);
    let mut entry = journal.accept(assignment).unwrap();
    journal.mark_executing(&mut entry).unwrap();
    journal
        .complete(
            &mut entry,
            DeviceWorkResult {
                success: true,
                content: "already executed".into(),
                parts: Vec::new(),
            },
        )
        .unwrap();

    recover_journaled_work(&client, &credentials, &storage)
        .await
        .unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        journal.load().unwrap().unwrap().state,
        WorkJournalState::Completed
    );
    task.abort();
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn enrollment_local_decline_and_lost_finalize_response_recover_safely() {
    for confirm in [false, true] {
        let base =
            std::env::temp_dir().join(format!("windie-enroll-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let storage = storage::Storage::at(base.join("agent")).unwrap();
        let id = EnrollmentId::new();
        let device_id = DeviceId::new();
        let finish_count = Arc::new(AtomicUsize::new(0));
        let finishes = finish_count.clone();
        let cancels = Arc::new(AtomicUsize::new(0));
        let cancel_count = cancels.clone();
        let metadata = fresh_credentials(PRODUCTION_API.into())
            .unwrap()
            .request
            .metadata;
        let preview = EnrollmentView {
            id,
            state: EnrollmentState::Approved,
            metadata: metadata.clone(),
            expires_at: i64::MAX,
            account_id: Some("a".into()),
            account_label: Some("verified-account".into()),
            device_id: None,
        };
        let self_view = DeviceView {
            id: device_id,
            metadata,
            revoked: false,
            online: false,
            last_seen: None,
        };
        let (client, task) = serve(
            Router::new()
                .route(
                    "/v1/device-enrollments",
                    post(move |h: HeaderMap| async move {
                        assert!(!h.contains_key("authorization"));
                        Json(EnrollmentStarted {
                            id,
                            code: "1234567-ABCDEF0".into(),
                            verification_url: PAIRING_URL.into(),
                            expires_at: i64::MAX,
                            poll_interval_seconds: POLL_SECONDS,
                        })
                    }),
                )
                .route(
                    &format!("/v1/device-enrollments/{id}"),
                    get(move || {
                        let view = preview.clone();
                        async { Json(view) }
                    }),
                )
                .route(
                    &format!("/v1/device-enrollments/{id}/finalize"),
                    post(move || {
                        let attempt = finishes.fetch_add(1, Ordering::SeqCst);
                        async move {
                            if attempt == 0 {
                                (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
                            } else {
                                (StatusCode::OK, Json(json!({"device_id":device_id})))
                            }
                        }
                    }),
                )
                .route(
                    &format!("/v1/device-enrollments/{id}/cancel"),
                    post(move || {
                        cancel_count.fetch_add(1, Ordering::SeqCst);
                        async { StatusCode::NO_CONTENT }
                    }),
                )
                .route(
                    "/v1/agent/self",
                    get(move || {
                        let view = self_view.clone();
                        async { Json(view) }
                    }),
                ),
        )
        .await;
        let mut credentials = fresh_credentials(client.server.clone()).unwrap();
        storage.save(&credentials).unwrap();
        enrollment::enroll(
            &client,
            &storage,
            &mut credentials,
            move |view| async move {
                assert_eq!(view.account_label.as_deref(), Some("verified-account"));
                Ok(confirm)
            },
            |_| {},
        )
        .await
        .unwrap();
        if confirm {
            assert_eq!(finish_count.load(Ordering::SeqCst), 2);
            assert_eq!(cancels.load(Ordering::SeqCst), 0);
            let saved = storage.load().unwrap().unwrap();
            assert_eq!(saved.device_id, Some(device_id));
            assert_eq!(client.own_device(&saved).await.unwrap().id, device_id);
        } else {
            assert_eq!(finish_count.load(Ordering::SeqCst), 0);
            assert_eq!(cancels.load(Ordering::SeqCst), 1);
            assert!(storage.load().unwrap().is_none());
        }
        task.abort();
        std::fs::remove_dir_all(base).unwrap();
    }
}
