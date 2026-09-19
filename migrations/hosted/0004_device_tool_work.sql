-- Device execution is deliberately additive to enrollment. An online device
-- gains no work until it explicitly reports capabilities and a session binds it.

ALTER TABLE messages ADD COLUMN IF NOT EXISTS metadata JSONB NULL;

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS bound_device_id TEXT NULL
    REFERENCES devices(id) ON DELETE RESTRICT;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS active_assignment_id TEXT NULL;
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_status_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_status_check CHECK (status IN
    ('ready', 'running', 'waiting_for_approval', 'waiting_for_tool',
     'completed', 'failed', 'cancelled'));

ALTER TABLE session_execution_claims DROP CONSTRAINT IF EXISTS session_execution_claims_status_check;
ALTER TABLE session_execution_claims ADD CONSTRAINT session_execution_claims_status_check
    CHECK (status IN ('active', 'yielded', 'completed', 'failed', 'cancelled'));

CREATE TABLE device_capability_reports (
    revision TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    lease_id TEXT NOT NULL,
    capabilities_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX device_capability_reports_device_created_idx
    ON device_capability_reports(device_id, created_at DESC);

-- One explicit session-level exposure maps a model schema to the immutable
-- capability report that supplied it. A later device report cannot silently
-- redirect an existing model call.
CREATE TABLE hosted_tool_attachments (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    schema_name TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    component_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    provider_tool_name TEXT NOT NULL,
    provider_kind TEXT NOT NULL,
    capability_revision TEXT NOT NULL REFERENCES device_capability_reports(revision) ON DELETE RESTRICT,
    schema_json JSONB NOT NULL,
    permissions_json JSONB NOT NULL,
    annotations_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, schema_name)
);

-- Browser approval is a durable, narrow authorization for one immutable
-- assistant call. It is intentionally separate from the assignment so a
-- capable connected device never turns a model request into immediate work.
CREATE TABLE device_tool_approvals (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    assistant_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE RESTRICT,
    result_parent_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE RESTRICT,
    tool_call_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE RESTRICT,
    capability_revision TEXT NOT NULL REFERENCES device_capability_reports(revision) ON DELETE RESTRICT,
    work_json JSONB NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied', 'cancelled')),
    decided_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, assistant_message_id, tool_call_id)
);
CREATE INDEX device_tool_approvals_pending_idx
    ON device_tool_approvals(session_id, created_at) WHERE status = 'pending';

CREATE TABLE device_work_assignments (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE RESTRICT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    assistant_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE RESTRICT,
    result_parent_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE RESTRICT,
    tool_call_id TEXT NOT NULL,
    capability_revision TEXT NOT NULL REFERENCES device_capability_reports(revision) ON DELETE RESTRICT,
    work_json JSONB NOT NULL,
    execution_token TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'executing', 'result_saved', 'cancelled', 'expired', 'uncertain')),
    result_json JSONB NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ NULL,
    completed_at TIMESTAMPTZ NULL,
    UNIQUE (session_id, assistant_message_id, tool_call_id)
);
ALTER TABLE sessions ADD CONSTRAINT sessions_active_assignment_fk
    FOREIGN KEY (active_assignment_id) REFERENCES device_work_assignments(id) ON DELETE SET NULL;
CREATE INDEX device_work_assignments_pending_idx
    ON device_work_assignments(device_id, created_at) WHERE status = 'pending';
CREATE INDEX device_work_assignments_session_idx ON device_work_assignments(session_id, created_at);

CREATE UNIQUE INDEX wakeups_device_tool_result_assignment_idx
    ON wakeups ((payload->>'assignment_id'))
    WHERE trigger_type = 'device_tool_result';
