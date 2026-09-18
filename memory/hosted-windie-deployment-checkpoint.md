# Hosted Windie deployment checkpoint

Read this after a context reset before continuing hosted-server work. It is a
checkpoint, not a replacement for:

- `docs/plans/main-hosted-windie-server.md` — the phase plan and status;
- `docs/guides/hosted-windie-server.md` — deployment shape;
- `docs/decisions/0003-multi-device-sync-and-local-execution.md` — the
  architecture decision; and
- `memory/hosted-windie-server-context.md` — the broader implementation
  context.

Never store passwords, API keys, database URLs, private keys, or the
Droplet's public IP address in this file.

## Goal and scope

The current goal is Windie's first hosted service: authenticated accounts,
canonical PostgreSQL-backed conversations, and cross-browser synchronization.
It does not include devices, VMs, remote control, local filesystem/browser
execution, local MCP, the official UI redesign, Bifrost, or model execution.

The local runtime remains unchanged:

```text
local `windie api` → loopback API + SQLite + local runtime
`windie-server`    → hosted API + PostgreSQL + account-owned cloud state
```

Do not reinvent conversations or message trees. Reuse the local semantics:
canonical parent-linked trees, selected root-to-head context paths, durable
sessions that point at heads, backend-owned resolution, idempotency, revisions,
and durable event cursors.

## Current source state

Phases 1–5 are deployed; Phase 6 is ready for its manual live proof.
Preserve these uncommitted hosted-server changes:

```text
src/hosted/
src/bin/windie-server.rs
migrations/hosted/0001_account_conversations.sql
Cargo.toml
Cargo.lock
src/lib.rs
```

`windie-server` owns PostgreSQL migrations, Supabase-token account resolution,
account-scoped conversation/message/fork/truncate APIs, revisions,
idempotency, and replayable `/v1/events` SSE. It intentionally does not run
Bifrost or models yet.

Hosted HTTP handlers are adapters only: they authenticate, parse HTTP input,
and call `src/operation/hosted_conversation.rs`. That operation owns the
account-conversation workflow over `HostedStore`; do not add PostgreSQL
mutations directly to `src/hosted/api.rs`.

Required server environment variables:

```text
WINDIE_HOSTED_DATABASE_URL
WINDIE_HOSTED_SUPABASE_URL
WINDIE_HOSTED_SUPABASE_PUBLISHABLE_KEY
WINDIE_HOSTED_ADDRESS            # defaults to 127.0.0.1:8788
WINDIE_HOSTED_ALLOWED_ORIGIN     # defaults to https://app.windieos.com
```

Focused hosted tests pass locally. The PostgreSQL Phase 6 acceptance test is
present but ignored until an isolated `WINDIE_HOSTED_TEST_DATABASE_URL` is
provided. A prior full `cargo test --all-targets` failed during linking because
the development Mac ran out of disk space, not because of a known test failure.

## Deployed service

The DigitalOcean Droplet now runs:

- private PostgreSQL with the hosted migration applied;
- `windie-server` as the unprivileged `windie` system service, listening only
  on loopback;
- a named Cloudflare Tunnel serving `https://hosted-api.windieos.com`; and
- a host firewall that allows SSH only. PostgreSQL and the server port are not
  public.

The hosted health endpoint is public and responds. An unauthenticated
`/v1/conversations` request returns `401`, while the exact browser origin is
allowed by CORS. Do not print the server environment file: it contains the
database connection credential.

The server has deliberately conservative PostgreSQL settings for the resized
2 GiB Droplet. Bifrost is not installed or running.

## Phase 6 proof and Inspector bridge

The required live proof is:

1. Google sign-in;
2. create a conversation in one browser;
3. observe it in a second browser for the same account;
4. prove a different account cannot access it; and
5. restart `windie-server` and recover durable state plus the replay cursor.

For Phase 6, a deliberately limited hosted client was brought forward:

```text
Inspector → Google/Supabase sign-in → Supabase Bearer token
          → hosted `/v1` conversation APIs + hosted SSE
```

`app.windieos.com` is now assigned to that production build. It serves the
hosted client, which has Google/Supabase sign-in, account-owned conversation
list/create/read/message operations, and durable event refresh. It has no
models, tools, sessions, wakeups, local pairing, device access, or official-UI
work. Do not build those before the proof. After it passes, resume Phase 7 for
hosted sessions and Bifrost execution.

## Supabase and Google checkpoint

- Supabase project: `windie-auth`; it was restored and is active.
- Google branding and an existing Web OAuth client have been configured.
- The Supabase Google provider has the Google client ID/secret and is enabled.
- The redirect is the exact Supabase callback shown in its Google provider
  settings, not a Windie API route.
- Before proof, verify the hosted client Site URL/allowed redirect and add both
  proof accounts as Google Audience test users if they are not already present.
- Test the branded Windie → Google → Windie return flow as the first proof
  step; it is the remaining validation of the current OAuth configuration.
- Request only `openid`, `email`, and `profile`; never Gmail, Drive, Calendar,
  or file scopes for basic sign-in.

## DigitalOcean checkpoint

The main server is the existing DigitalOcean Droplet
`windie-main-server-droplet-do`, Ubuntu 24.04 in NYC1. It is the hosted
account/conversation server, not a later remote-control VM.

Verified state:

- resized to a 2 GiB plan with sufficient headroom for this server;
- PostgreSQL, `windie-server`, and `cloudflared` are installed and running;
- the server has a loopback-only application listener and a private database;
- UFW is enabled and permits SSH only; and
- automated backups remain a future operational decision.

Reconsider the Droplet size before Phase 7 once Bifrost and hosted model
execution are introduced.

## SSH checkpoint

Use terminal SSH for normal Droplet work. A verified dedicated deployment key
exists locally at:

```text
/Users/peterbui/.ssh/windie_main_server_ed25519
```

Its public key is authorized for the existing `root` account, and terminal
login using it has been verified. The local `windie-prod` alias is stale; do
not use it until it is intentionally repaired with the Droplet's current
address.

An older workspace SSH private key was exposed in a screenshot. Its matching
public key was revoked from the Droplet. Never use, copy, or re-add it. Do not
put private keys in this repository, documentation, screenshots, or chat.
The temporary DigitalOcean Web Console key is separate and expires normally.

## User hardware decision

The user has a 32 GB RAM PC with an RTX 5070 Ti. It can later support
development, local inference, or device execution. It should not replace the
always-on hosted account/conversation server unless the user explicitly accepts
the home-uptime and electricity tradeoff. Keep the inexpensive Droplet for the
Phase 6 proof and use the PC on demand.

## Resume order

```text
1. Complete the five Phase 6 live checks in two browser profiles and a second
   Google account.
2. Record the outcome in `docs/plans/main-hosted-windie-server.md`.
3. Provision an isolated PostgreSQL database and run the ignored PostgreSQL
   acceptance test.
4. Only then begin Phase 7.
```
