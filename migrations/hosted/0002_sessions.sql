-- Durable hosted execution state. Conversation trees remain canonical in the
-- existing messages table; sessions only point at selected/current heads.

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    start_head_message_id TEXT NULL REFERENCES messages(id) ON DELETE SET NULL,
    current_head_message_id TEXT NULL REFERENCES messages(id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('ready', 'running', 'waiting_for_approval', 'completed', 'failed', 'cancelled')),
    model TEXT NOT NULL,
    reasoning JSONB NULL,
    error TEXT NULL,
    keep_awake BOOLEAN NOT NULL DEFAULT FALSE,
    idle_wakeup_interval TEXT NOT NULL DEFAULT 'thirty_minutes'
        CHECK (idle_wakeup_interval IN ('fifteen_minutes', 'thirty_minutes', 'one_hour', 'two_hours')),
    last_user_activity_at BIGINT NOT NULL,
    last_idle_wakeup_completed_at BIGINT NULL,
    execution_owner TEXT NULL CHECK (execution_owner IN ('api', 'cli', 'hosted_server')),
    current_claim_id TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (execution_owner IS NULL AND current_claim_id IS NULL)
        OR (execution_owner IS NOT NULL AND current_claim_id IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS session_execution_claims (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner TEXT NOT NULL CHECK (owner IN ('api', 'cli', 'hosted_server')),
    status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'failed', 'cancelled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at TIMESTAMPTZ NULL
);

CREATE TABLE IF NOT EXISTS session_inputs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position BIGINT NOT NULL CHECK (position >= 0),
    content TEXT NOT NULL,
    parts JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, position)
);

CREATE TABLE IF NOT EXISTS session_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS wakeups (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    trigger_type TEXT NOT NULL,
    due_at TIMESTAMPTZ NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'claimed', 'completed', 'cancelled')),
    claim_id TEXT NULL REFERENCES session_execution_claims(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ NULL,
    completed_at TIMESTAMPTZ NULL
);

CREATE INDEX IF NOT EXISTS sessions_account_updated_idx
    ON sessions(account_id, updated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS sessions_conversation_head_idx
    ON sessions(conversation_id, current_head_message_id, created_at, id);
CREATE INDEX IF NOT EXISTS session_inputs_fifo_idx
    ON session_inputs(session_id, position);
CREATE INDEX IF NOT EXISTS session_events_session_id_idx
    ON session_events(session_id, id);
CREATE INDEX IF NOT EXISTS wakeups_due_pending_idx
    ON wakeups(due_at, id) WHERE status = 'pending';
