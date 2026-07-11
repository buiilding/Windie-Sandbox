//! Schema persistence tests.

use super::*;

#[test]
fn sets_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let version: i32 = store
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();

    assert_eq!(version, DATABASE_SCHEMA_VERSION);
}

#[test]
fn creates_performance_indexes() {
    let store = Store::open_memory().unwrap();

    assert!(index_exists(&store, "messages_parent_idx"));
    assert!(index_exists(&store, "messages_id_conversation_idx"));
    assert!(index_exists(&store, "conversations_updated_idx"));
    assert!(index_exists(
        &store,
        "runtime_runs_one_running_per_conversation_idx"
    ));
}

#[test]
fn active_message_pointer_rejects_dangling_message_ids() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let error = store
        .connection
        .execute(
            "UPDATE conversations SET active_message_id = 'missing' WHERE id = ?1",
            [conversation_id.as_str()],
        )
        .unwrap_err();

    assert!(error.to_string().contains("FOREIGN KEY constraint failed"));
}

#[test]
fn stale_mutation_guard_detects_external_commits() {
    let path = std::env::temp_dir().join(format!(
        "windie-data-version-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut first = Store::open_at(&path).unwrap();
    let data_version = first.data_version().unwrap();
    let second = Store::open_at(&path).unwrap();
    second.create_conversation("openai/test").unwrap();

    let transaction = first
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let error = ensure_data_version(&transaction, data_version).unwrap_err();

    assert_eq!(
        error::kind_from_error(&error),
        Some(crate::error::WindieErrorKind::Conflict)
    );
    drop(transaction);
    let _ = std::fs::remove_file(path);
}

#[test]
fn rejects_newer_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let newer_version = DATABASE_SCHEMA_VERSION + 1;
    store
        .connection
        .pragma_update(None, "user_version", newer_version)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "database schema version {newer_version} is newer than supported version {DATABASE_SCHEMA_VERSION}"
        )
    );
}

#[test]
fn rejects_older_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let older_version = DATABASE_SCHEMA_VERSION - 1;
    store
        .connection
        .pragma_update(None, "user_version", older_version)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "database schema version {older_version} is older than supported version {DATABASE_SCHEMA_VERSION}; remove the old Windie database or recreate it"
        )
    );
}

#[test]
fn rejects_existing_unversioned_database_schema() {
    let store = Store::open_memory().unwrap();
    store
        .connection
        .pragma_update(None, "user_version", 0)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        "existing unversioned Windie database is not supported; remove the old Windie database or recreate it"
    );
}
