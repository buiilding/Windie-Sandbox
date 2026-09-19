-- Enrollment grants presence only. No tool or conversation privileges.
CREATE TABLE devices (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id),
 metadata JSONB NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), revoked_at TIMESTAMPTZ
);
CREATE INDEX devices_account_idx ON devices(account_id, created_at);
CREATE TABLE device_credentials (
 digest TEXT PRIMARY KEY, device_id TEXT NOT NULL UNIQUE REFERENCES devices(id),
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(), revoked_at TIMESTAMPTZ
);
CREATE TABLE device_enrollments (
 id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, fingerprint TEXT NOT NULL,
 enrollment_digest TEXT NOT NULL UNIQUE, device_digest TEXT NOT NULL UNIQUE,
 code_digest TEXT NOT NULL UNIQUE, key_version TEXT NOT NULL, metadata JSONB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','approved','consumed','denied','cancelled')),
 expires_at TIMESTAMPTZ NOT NULL, account_id TEXT REFERENCES accounts(id), account_label TEXT,
 device_id TEXT REFERENCES devices(id), last_poll_at TIMESTAMPTZ,
 CHECK ((state IN ('approved','consumed')) = (account_id IS NOT NULL)),
 CHECK ((state = 'consumed') = (device_id IS NOT NULL))
);
CREATE INDEX device_enrollments_expiry_idx ON device_enrollments(expires_at);
CREATE TABLE device_presence (
 device_id TEXT PRIMARY KEY REFERENCES devices(id), instance_id TEXT NOT NULL,
 lease_id TEXT NOT NULL, last_seen TIMESTAMPTZ NOT NULL, expires_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE device_rate_limits (
 bucket TEXT PRIMARY KEY, window_start TIMESTAMPTZ NOT NULL, count INTEGER NOT NULL CHECK(count > 0)
);
CREATE TABLE device_audit (
 id BIGSERIAL PRIMARY KEY, device_id TEXT NOT NULL REFERENCES devices(id),
 event TEXT NOT NULL CHECK(event IN ('registered','revoked')), created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
