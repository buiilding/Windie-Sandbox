-- Canonical, account-owned Windie conversation storage for windie-server.
-- This database is private to the hosted server. The browser never receives
-- direct database access or credentials.

CREATE TABLE IF NOT EXISTS hosted_schema_migrations (
    version TEXT PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    auth_subject TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    title TEXT NULL,
    model TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    parent_message_id TEXT NULL,
    role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool')),
    content TEXT NOT NULL,
    position BIGSERIAL NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT messages_parent_not_self CHECK (parent_message_id IS NULL OR parent_message_id <> id)
);

CREATE TABLE IF NOT EXISTS message_parts (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    part_type TEXT NOT NULL CHECK (part_type IN ('text', 'image')),
    text_content TEXT NULL,
    image_mime_type TEXT NULL,
    image_bytes BYTEA NULL,
    CHECK (
        (part_type = 'text' AND text_content IS NOT NULL AND image_mime_type IS NULL AND image_bytes IS NULL)
        OR
        (part_type = 'image' AND text_content IS NULL AND image_mime_type IS NOT NULL AND image_bytes IS NOT NULL)
    ),
    UNIQUE (message_id, position)
);

CREATE TABLE IF NOT EXISTS account_change_events (
    id BIGSERIAL PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    conversation_id TEXT NULL,
    conversation_revision BIGINT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS idempotency_records (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    response_status SMALLINT NOT NULL,
    response_body JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, idempotency_key),
    CHECK (char_length(idempotency_key) BETWEEN 1 AND 255)
);

CREATE INDEX IF NOT EXISTS conversations_account_updated_idx
    ON conversations(account_id, updated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS messages_conversation_parent_idx
    ON messages(conversation_id, parent_message_id, position);
CREATE INDEX IF NOT EXISTS account_change_events_account_id_idx
    ON account_change_events(account_id, id);
