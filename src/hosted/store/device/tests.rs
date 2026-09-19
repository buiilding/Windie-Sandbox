//! Opt-in PostgreSQL proofs in a unique disposable schema of the isolated test DB.

use super::*;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;

/// Never apply tests to the hosted database or delete a shared schema.
pub(crate) struct Fixture {
    pub store: HostedStore,
    admin: PgPool,
    schema: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let url =
            std::env::var("WINDIE_HOSTED_TEST_DATABASE_URL").expect("isolated test URL required");
        let options = PgConnectOptions::from_str(&url).expect("valid isolated test URL");
        assert_eq!(
            options.get_database(),
            Some("windie_test"),
            "Only the isolated windie_test database is allowed"
        );
        let admin = PgPool::connect_with(options.clone()).await.unwrap();
        let schema = format!("device_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .unwrap();
        let store = HostedStore { pool };
        store.migrate().await.unwrap();
        Self {
            store,
            admin,
            schema,
        }
    }
    pub async fn finish(self) {
        self.store.pool.close().await;
        assert!(self.schema.starts_with("device_test_"));
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
pub(crate) fn request() -> EnrollmentRequest {
    EnrollmentRequest {
        request_id: RequestId::new(),
        enrollment_digest: secret_digest("enroll", &new_secret("enroll").unwrap()).unwrap(),
        device_digest: secret_digest("device", &new_secret("device").unwrap()).unwrap(),
        metadata: DeviceMetadata {
            name: "Test Mac".into(),
            os: "macos".into(),
            architecture: "aarch64".into(),
            agent_version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
        },
    }
}

#[tokio::test]
#[ignore = "requires isolated WINDIE_HOSTED_TEST_DATABASE_URL (windie_test)"]
async fn postgres_device_lifecycle_acceptance() {
    let f = Fixture::new().await;
    let s = &f.store;
    let key = b"test-enrollment-key";
    let a = s.resolve_account("device-test-a").await.unwrap();
    let b = s.resolve_account("device-test-b").await.unwrap();
    let r = request();
    // Lost initiation response / simultaneous retry returns the same code and ID.
    let (one, two) = tokio::join!(s.initiate_device(&r, key), s.initiate_device(&r, key));
    let start = one.unwrap();
    assert_eq!(start.id, two.unwrap().id);
    assert_eq!(start.code, s.initiate_device(&r, key).await.unwrap().code);
    let mut changed = r.clone();
    changed.metadata.name = "Other".into();
    assert_eq!(
        s.initiate_device(&changed, key).await.unwrap_err(),
        DeviceError::Conflict
    );
    let p = EnrollmentPrincipal {
        id: start.id,
        digest: r.enrollment_digest.clone(),
    };
    assert_eq!(
        s.poll_device_enrollment(&p, key).await.unwrap().state,
        EnrollmentState::Pending
    );
    assert_eq!(
        s.poll_device_enrollment(&p, key).await.unwrap_err(),
        DeviceError::RateLimited
    );
    assert_eq!(
        s.finalize_device(&p, &a.id, key).await.unwrap_err(),
        DeviceError::Conflict
    );
    let wrong = EnrollmentPrincipal {
        id: start.id,
        digest: hash("guessed"),
    };
    assert_eq!(
        s.poll_device_enrollment(&wrong, key).await.unwrap_err(),
        DeviceError::Unauthorized
    );
    let (approve_a, approve_b) = tokio::join!(
        s.device_code_action(&a, &a.auth_subject, &start.code, CodeAction::Approve, key),
        s.device_code_action(&b, &b.auth_subject, &start.code, CodeAction::Approve, key)
    );
    assert_ne!(approve_a.is_ok(), approve_b.is_ok());
    let (owner, other) = if approve_a.is_ok() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    assert_eq!(
        s.device_code_action(other, "ignored", &start.code, CodeAction::Lookup, key)
            .await
            .unwrap_err(),
        DeviceError::NotFound
    );
    assert!(
        s.device_code_action(
            owner,
            &owner.auth_subject,
            &start.code,
            CodeAction::Approve,
            key
        )
        .await
        .is_ok()
    );
    assert_eq!(
        s.finalize_device(&p, &other.id, key).await.unwrap_err(),
        DeviceError::Conflict
    );
    let (first, second) = tokio::join!(
        s.finalize_device(&p, &owner.id, key),
        s.finalize_device(&p, &owner.id, key)
    );
    let id = first.unwrap();
    assert_eq!(id, second.unwrap());
    assert_eq!(s.list_devices(owner).await.unwrap().len(), 1);
    assert!(s.list_devices(other).await.unwrap().is_empty());
    assert_eq!(
        s.revoke_device(other, id).await.unwrap_err(),
        DeviceError::NotFound
    );
    assert_eq!(
        s.cancel_device_enrollment(&p, key).await.unwrap_err(),
        DeviceError::Conflict
    );
    let d = DevicePrincipal {
        digest: r.device_digest.clone(),
    };
    let idle = s.device_self(&d).await.unwrap();
    assert!(!idle.online);
    assert_eq!(idle.last_seen, None);
    let run = ConnectRequest {
        instance_id: InstanceId::new(),
        protocol_version: PROTOCOL_VERSION,
    };
    let lease = s.connect_device(&d, &run).await.unwrap();
    assert_eq!(
        lease.lease_id,
        s.connect_device(&d, &run).await.unwrap().lease_id
    );
    assert!(s.device_self(&d).await.unwrap().online);
    let other_run = ConnectRequest {
        instance_id: InstanceId::new(),
        protocol_version: PROTOCOL_VERSION,
    };
    assert_eq!(
        s.connect_device(&d, &other_run).await.unwrap_err(),
        DeviceError::AlreadyRunning
    );
    let seen = s.device_self(&d).await.unwrap().last_seen;
    assert_eq!(s.device_self(&d).await.unwrap().last_seen, seen); // read does not renew
    // Move database-owned timestamps instead of sleeping or relying on local wall time.
    sqlx::query(
        "UPDATE device_presence SET expires_at=now()-interval '1 second' WHERE device_id=$1",
    )
    .bind(id.to_string())
    .execute(&s.pool)
    .await
    .unwrap();
    assert!(!s.device_self(&d).await.unwrap().online);
    assert_eq!(
        s.heartbeat_device(&d, lease.lease_id, false)
            .await
            .unwrap_err(),
        DeviceError::StaleLease
    );
    let fresh = s.connect_device(&d, &other_run).await.unwrap();
    assert_ne!(lease.lease_id, fresh.lease_id);
    assert_eq!(
        s.heartbeat_device(&d, lease.lease_id, true)
            .await
            .unwrap_err(),
        DeviceError::StaleLease
    );
    s.heartbeat_device(&d, fresh.lease_id, true).await.unwrap();
    assert!(!s.device_self(&d).await.unwrap().online);
    let active = s.connect_device(&d, &run).await.unwrap();
    // Enrollment expiry/key rotation must not break lost-finalize-response recovery.
    sqlx::query("UPDATE device_enrollments SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(start.id.to_string())
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(
        s.finalize_device(&p, &owner.id, key).await.unwrap_err(),
        DeviceError::Expired
    );
    assert_eq!(s.device_self(&d).await.unwrap().id, id);
    assert_eq!(
        s.poll_device_enrollment(&p, b"rotated").await.unwrap_err(),
        DeviceError::Expired
    );
    let (_, revocation) = tokio::join!(
        s.heartbeat_device(&d, active.lease_id, false),
        s.revoke_device(owner, id)
    );
    revocation.unwrap();
    s.revoke_device(owner, id).await.unwrap();
    assert_eq!(
        s.device_self(&d).await.unwrap_err(),
        DeviceError::Unauthorized
    );
    assert_eq!(
        s.connect_device(&d, &run).await.unwrap_err(),
        DeviceError::Unauthorized
    );
    assert_eq!(
        s.heartbeat_device(&d, active.lease_id, false)
            .await
            .unwrap_err(),
        DeviceError::Unauthorized
    );
    assert!(s.list_devices(owner).await.unwrap()[0].revoked);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM device_audit WHERE event='revoked'")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // Denied/cancelled/expired state cannot activate or be rebound.
    for action in ["deny", "cancel", "expire"] {
        let r = request();
        let start = s.initiate_device(&r, key).await.unwrap();
        let p = EnrollmentPrincipal {
            id: start.id,
            digest: r.enrollment_digest,
        };
        match action {
            "deny" => {
                s.device_code_action(&a, "a", &start.code, CodeAction::Deny, key)
                    .await
                    .unwrap();
            }
            "cancel" => s.cancel_device_enrollment(&p, key).await.unwrap(),
            _ => {
                sqlx::query("UPDATE device_enrollments SET expires_at=now()-interval '1 second' WHERE id=$1").bind(start.id.to_string()).execute(&s.pool).await.unwrap();
            }
        }
        assert!(s.finalize_device(&p, &a.id, key).await.is_err());
        assert!(
            s.device_code_action(&a, "a", &start.code, CodeAction::Approve, key)
                .await
                .is_err()
        );
        assert!(
            s.device_self(&DevicePrincipal {
                digest: r.device_digest
            })
            .await
            .is_err()
        );
    }
    // Rate counters survive a new service/store handle; shared across instances.
    let second = HostedStore {
        pool: s.pool.clone(),
    };
    for _ in 0..5 {
        s.device_rate_limit("source", 5, 60).await.unwrap();
    }
    assert_eq!(
        second.device_rate_limit("source", 5, 60).await.unwrap_err(),
        DeviceError::RateLimited
    );
    sqlx::query("UPDATE device_rate_limits SET window_start=now()-interval '61 seconds'")
        .execute(&s.pool)
        .await
        .unwrap();
    second.device_rate_limit("source", 5, 60).await.unwrap();
    // No chat/session data was created by any device operation.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM conversations")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.finish().await;
}
