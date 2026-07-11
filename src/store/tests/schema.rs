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
