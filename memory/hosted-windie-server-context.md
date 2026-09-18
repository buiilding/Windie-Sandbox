# Hosted Windie server: situational context

Read this after a context reset before resuming hosted-server work. It records
decisions, current implementation/deployment facts, and cautions; the phase
plan remains the authority for work scope and completion status.

## Read first

Before changing hosted behavior, read:

- `AGENTS.md`
- `docs/index/Backend.md`
- `docs/index/Frontend.md`
- `docs/guides/hosted-windie-server.md`
- `docs/decisions/0003-multi-device-sync-and-local-execution.md`
- `docs/official-design/README.md`
- `docs/plans/main-hosted-windie-server.md`
- `docs/plans/shared-operation-persistence-refactor.md`

For every hosted feature, inspect its local counterpart first: the route in
`src/api/`, workflow in `src/operation/`, SQLite behavior in `src/store/`, and
its tests. Reuse domain rules, types, validation, response contracts, and test
intent. Do not reuse local HTTP handlers unchanged: they carry local pairing,
SQLite, `SessionManager`, and local-tool assumptions.

## Product direction and boundaries

Call the service the **hosted server**, not the control plane.

```text
Local today
browser → local `windie api` → SQLite → Bifrost → local MCP/tool execution

Hosted today
browser → `windie-server` → PostgreSQL → private Bifrost → hosted model stream

Hosted later
browser → `windie-server` → PostgreSQL → model
                                        → authorized device agent → tool result
```

The hosted server is not a replacement for the local runtime. Its canonical
domain model must remain Windie's existing one:

- conversations are parent-linked message trees;
- the selected root-to-head path is model context;
- sessions are durable branch-execution records, not copied conversations;
- backend-owned session/head resolution prevents stale browser ownership;
- revisions, idempotency keys, durable events, and execution claims prevent
  duplicate or conflicting work.

SQLite and PostgreSQL are both relational persistence implementations. Do not
create a parallel cloud-chat model because PostgreSQL requires different SQL.
Extract only narrow shared policies when a concrete feature needs both storage
backends; do not build one huge generic database trait.

The hosted server never executes a user's local filesystem, browser, or MCP
tool. Later it owns the workflow: persist tool request, approval, authorized
device assignment, result, and continuation. Device agents, VMs, remote
control, and the final official chat UI remain later phases.

## Authentication and account ownership

```text
Google sign-in → Supabase access token → hosted server validation
→ stable Supabase user ID → Windie account → account-scoped PostgreSQL data
```

Supabase is the identity broker, not the product or conversation authority.
The browser sends the bearer token only to the hosted API; it never receives a
database credential or provider secret. The active Supabase project is
`windie-auth`. Basic sign-in needs only `openid`, `email`, and `profile`.

Google branding is configured for Windie, but Google must accept verified
ownership of `windieos.com` before its account chooser stops displaying the
Supabase project hostname. Preserve the Supabase callback and redirect setup;
complete domain verification in Google Search Console, request branding
re-verification in Google Auth Platform, then wait for publication.

Windie-managed provider access is the initial hosted inference policy. Provider
keys are server-only secrets.

## Implemented hosted server

Phases 1–6 are implemented and live-verified: Google sign-in, same-account
two-browser convergence, account isolation, durable PostgreSQL state, and
restart recovery passed. The hosted server provides account-scoped
conversation/message-tree operations, revision/idempotency handling, durable
account events, and replayable SSE.

Phase 7 is deployed and partially live-verified. PostgreSQL now contains
sessions, FIFO session inputs, session events, execution claims, and wakeups.
The hosted worker resolves selected heads, atomically claims execution,
compiles the selected tree path, streams through private Bifrost, persists the
assistant result/events, drains queued input, and supports durable wakeups.
Tool calls intentionally stop the current hosted worker: device dispatch and
approval continuation have not been implemented yet.

The deployed gateway uses a server-only Kimi Code credential and supports
`kimi-code/kimi-for-coding`. Direct gateway and hosted browser streaming have
been observed. Queue-under-load and interrupted-run/restart recovery still
need explicit Phase 7 live proof before that phase is marked fully verified.

Phase 8's limited Inspector bridge is deployed at `app.windieos.com`. It keeps
Google/Supabase authentication, uses the hosted API, creates new conversations
with the deployment default model, resolves/query sessions, renders live
assistant text, and reloads the durable final message. It is a temporary proof
surface, not the final official UI and not a reason to add unsupported local
controls to the hosted server.

## Current streaming behavior and implemented improvement

The current hosted session stream is durable but visually chunkier than the
local runtime:

```text
Bifrost delta → PostgreSQL session_events → 250 ms hosted DB poll → browser
```

The local API persists events then uses an in-process `SessionManager`
subscription for immediate delivery. The hosted Inspector correctly appends
received `assistant_delta` events; the main delay is the hosted event path,
not merely rendering.

The shared live-event delivery revision was deployed to the Droplet on
2026-09-18. It preserves durability and reuses the local pattern:

```text
commit PostgreSQL event → publish that returned record to a local event hub
                         → immediately send to connected browsers
reconnect → replay later PostgreSQL rows using the durable cursor
```

`src/session/live_events.rs` now owns the shared process-local hub. The local
`SessionManager` and hosted runtime publish the exact record returned after a
durable commit. Hosted session SSE subscribes before replay, uses the durable
cursor to suppress duplicates/repair lag, receives same-process events
immediately, and uses a slow database fallback only for recovery.

PostgreSQL `LISTEN`/`NOTIFY` sends only the committed event ID to other
`windie-server` instances, which reload the record from PostgreSQL and publish
it locally. Never broadcast before the database transaction commits. The
implementation passed local Rust suites and the deployed service and private
Bifrost health checks. A signed-in browser stream remains the required
post-deployment user-visible proof.

## Deployment and operations

The production DigitalOcean Droplet is the hosted account/conversation server,
not a future user remote-control VM. It has a private PostgreSQL database,
loopback-only `windie-server`, private loopback Bifrost, and Cloudflare Tunnel
public routing for `https://hosted-api.windieos.com`. The Inspector is hosted
separately at `https://app.windieos.com`.

The Droplet has 2 GiB RAM. PostgreSQL is conservatively tuned. `windie-server`
and Bifrost run as separate unprivileged service accounts; provider data is not
readable by the Windie application service. A dedicated local SSH deployment
key exists. Never restore or use the older key exposed in a screenshot, and
never record keys, credentials, database URLs, tunnel credentials, or server
addresses in repository files or chat.

Production checks already established include public health, unauthenticated
conversation rejection, restrictive CORS, active Cloudflare routing, private
Bifrost health, a successful Kimi completion, and a compatible hosted
Responses-stream request. Before every deployment, build/test locally as
appropriate, create a release binary on the Droplet, restart only the affected
service, then verify health and logs without printing environment files.

An isolated PostgreSQL test database exists on the Droplet for hosted
acceptance tests. Keep it separate from production; do not run test migrations
or test commands against production data.

## Implementation structure and cautions

Hosted HTTP handlers should authenticate, validate input, call an operation or
runtime method, and serialize the result. They must not accumulate direct
PostgreSQL mutation logic. `HostedStore` owns account-scoped PostgreSQL
transactions; local `Store` owns SQLite. The initial hosted conversation
operation is not yet a full shared persistence abstraction, so extend it only
when a concrete shared rule justifies a narrow extraction.

The Inspector's normal local runtime path requests local-only models, provider
configuration, tools, plugins, approvals, pairing, and device behavior. The
hosted bridge must continue exposing only hosted capabilities. Do not automate
browser UI tests unless Peter asks; use terminal checks where possible and ask
him to perform authenticated UI proof.

The root checkout and Inspector Git submodule may have in-progress work.
Preserve unrelated changes. For an explicit commit, inspect both statuses,
stage only the intended files, commit the Inspector first when it changed, then
commit the root submodule pointer; never push without explicit authorization.

## Resume sequence

1. Check `docs/plans/main-hosted-windie-server.md` for the authoritative next
   phase and evidence gaps.
2. Read the local equivalent before making any hosted change.
3. Keep durable PostgreSQL events authoritative; never bypass account checks or
   execution claims for convenience.
4. Finish Phase 7 queue and restart-recovery proof, then manually confirm the
   deployed replay-plus-live event path makes a signed-in stream feel smooth.
5. Continue only with the next planned hosted capability; do not jump ahead to
   device execution or remote control.
