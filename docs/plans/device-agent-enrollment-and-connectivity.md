# Device-agent enrollment and connectivity

## Status and scope

Implemented in the working tree — 2026-09-19. **Not deployed or live-verified.**
Commands and API paths below now exist; the original design requirements are
retained below. Deployment and the independent manual acceptance checklist
remain pending. No tool execution or plugin installation was added.

### Implementation evidence

- Phases 1–4: shared typed protocol and hashing, additive PostgreSQL migration,
  separate browser/enrollment/device authority, rate limits/retention, protected
  local state, existing CLI integration, and foreground fenced presence.
- Phase 5: official UI `/devices/connect` and `/computers`, explicit consent and
  revoke confirmation, exact allowlisted OAuth return paths, visibility-aware
  bounded polling, and stop-to-sign-in on 401. `/` and `/c/:id` stay chat routes.
- Phase 6 automated proof: Rust library suite, two opt-in PostgreSQL acceptance
  tests in disposable schemas of the Droplet's **isolated `windie_test` database**,
  official UI tests/build, and focused lint. Production service/data were not
  restarted or migrated. Full-UI lint has pre-existing unrelated failures.
  Final results: `cargo test --lib` **476 passed, 5 ignored**; the two new
  `postgres_device` tests **2 passed** when explicitly enabled; official UI
  **32 tests passed**, production build and changed-file lint passed.
  `cargo check --all-targets` and `scripts/check-docs.sh` passed. The temporary
  test schemas were removed; no device test schemas remained afterward.
- HTTP/DB tests cover concurrent ownership/finalization, account isolation,
  lost-response retries, denial/cancellation/expiry, stale leases, revocation
  races, principal separation, body limits, no-store, shared rate counters and
  feature-disable behavior. Agent tests cover protected files/locks, failed
  atomic writes, local decline, finalize-response failure, redirect refusal,
  transport errors and terminal 401. Browser polling tests cover visibility,
  no overlap, 429 backoff, disposal and stopping after 401.
- Full Google/browser/OS-process/network/server-restart live proof remains
  unchecked below. No automatic browser verification, commit, push or deployment.

Implementation placement follows the existing CLI parser; adjacent hosted
workflow/auth adapters are kept in `hosted/device_api.rs`, and atomic database
transitions in `hosted/store/device.rs`. Reusable enrollment orchestration is
in `agent/enrollment.rs`, with prompts/output in `cli/adapter/agent.rs`.
This avoids empty wrapper modules and does not duplicate session operations.

See [operator setup, commands and recovery](../guides/device-agent.md).

First outcome: Peter can register his Mac to his Windie account, run a small
agent, inspect its online/offline state, and revoke access. Another account
cannot inspect or control that registration.

This is the first slice of [registered-device execution](../guides/hosted-windie-server.md),
consistent with [ADR 0003](../decisions/0003-multi-device-sync-and-local-execution.md).
It extends, rather than replaces, the [hosted server foundation](main-hosted-windie-server.md).

### Included

- A mode of the existing `windie` executable: connect, foreground run, status.
- Explicit browser approval and local confirmation of account pairing.
- Account-owned device records and separate revocable device credentials.
- Outbound HTTPS presence reporting with bounded retries and lease expiry.
- Minimal browser pairing and registered-computer list/revoke surfaces.
- Terminal-run protocol, persistence, security, and regression tests.

### Excluded

- Plugin installation, discovery/reporting of executable tool capabilities,
  tool dispatch, approvals for tool execution, and result continuation.
- VM provisioning, screen streaming, remote desktop, file synchronization.
- Local LLM inference or local conversation/session creation for the agent.
- Background service installation, login startup, tray integration, or a new
  auto-updater. The first executable proof runs in a terminal on macOS.
- A general message broker, WebSocket framework, or broad runtime refactor.

The future marketplace flow remains **Install → choose registered computer →
agent installs locally → report readiness**. Do not implement it here. An
online device in this milestone has no remotely executable capabilities.

## Source-grounded baseline and reuse

Read these before implementation, including current changes in the worktree:

| Existing boundary | Reuse / limitation |
| --- | --- |
| `src/main.rs`, `src/cli/command.rs`, `src/cli/parser.rs`, `src/cli/adapter/` | Existing typed CLI and thin entrypoint; add agent commands here. |
| `src/hosted/auth.rs`, `account.rs`, `api.rs` | Reuse verified Supabase subject-to-account resolution for browser requests. Add a separate device authentication boundary, not a Supabase refresh session on the Mac. |
| `src/hosted/store.rs`, `migrations/hosted/` | Extend existing PostgreSQL migration/transaction infrastructure. Do not open SQLite on the hosted server. |
| `src/store/runtime_access.rs`, `src/api/runtime_access.rs` | Existing single-owner local API pairing is not device enrollment. Preserve it unchanged; never silently adopt it as consent for hosted agent access. |
| `src/tool/registry.rs`, `src/mcp/executor.rs`, `src/mcp/result.rs` | Existing discovery/execution/result foundation for a later slice. Do not initialize MCP processes merely to report presence. |
| `src/api/plugin.rs`, `src/plugin/installer.rs`, `src/plugin/store.rs` | Later targeted installation should extract/reuse existing install orchestration, not copy the HTTP handler into the agent. |
| `src/local/process.rs` | Reference existing lifecycle conventions; detached startup is deferred. |
| `vendor/windie-UI-official/lib/hosted-auth.ts`, `app/page.tsx` | Reuse Google sign-in and typed hosted transport; add explicit pairing/computers routing without altering `/` and `/c/:id` semantics. |

Read `docs/index/Backend.md` and `docs/index/Frontend.md` first. The official UI
is an independent nested Git repository, not a registered root submodule at
this checkpoint. Preserve unrelated root/Inspector/UI edits and do not create
an accidental gitlink.

Important later gaps: the hosted worker currently sends no tool schemas and
rejects tool calls; hosted model-path loading sets message metadata to `None`;
local approval/persistence workflows remain `Store`-bound. Also,
`ToolExecutionResult.parts` is skipped by serialization. Enrollment does not
resolve those gaps or justify claiming remote execution is ready.

## Ownership and trust

```text
Mac: windie agent connect/run ── outbound HTTPS ──> hosted Windie server
                                                      │
Browser: Google/Supabase sign-in ── account token ───────┤
                                                      ▼
                                              private PostgreSQL
```

- Browser authority: verified account identity; may approve, list, and revoke
  only that account's devices. Client-supplied account IDs confer no authority.
- Device authority: one enrollment, then one registered device; no conversation,
  model, account-administration, plugin-install, or tool-execution authority.
- Server: canonical registration, revocation, and presence lease. Device name,
  OS, and architecture are self-reported metadata, not hardware attestation.
- Mac: private credential storage and process lifetime. No public listener,
  browser-to-localhost pairing dependency, port-forwarding, or API tunnel.

Use separate middleware/principal types for account, pending enrollment, and
device routes. Never accept a device token on account routes or treat a
browser access token as a device token. Authenticate and authorize on every
heartbeat; a successful initial connection is not perpetual permission.

## CLI and local state

Proposed interface:

```text
windie agent connect
windie agent run
windie agent status
```

- `connect`: show the official pairing URL and short code; opening the browser
  is optional convenience. Wait with bounded polling, display the approved
  account identity, and require explicit local confirmation before activation.
- `run`: foreground only, fails clearly if unpaired/revoked; Ctrl-C stops it
  and makes a best-effort release request. No Bifrost, local API, or MCP startup.
- `status`: display device ID, configured server, registration state, server
  presence, and last contact if available. Network failure means “unknown / API
  unreachable,” not “revoked.” It must not acquire or renew a presence lease.

Default to the production HTTPS API and fixed trusted browser origin. Permit
an explicit development override for loopback HTTP only; bind credentials to
the exact server origin and never forward them across HTTP redirects.

For the macOS-first slice, use an agent-specific directory under `~/.windie`
with mode 0700 and credential files created atomically with mode 0600. Reject
symlinks/unsafe ownership or permissions; serialize connect/run state updates
with an OS-released exclusive lock. Do not print secrets in argv, logs, status,
URLs, crash diagnostics, or checked-in environment files. A protected file is
not protection from another process running as the same OS user; OS credential
vault integration can follow. Do not claim Windows credential safety without a
separate ACL/vault implementation and tests.

Keep pending enrollment state separate from an active credential so a crash
does not destroy an existing registration. Refuse implicit replacement of an
active pairing; changing accounts requires revoking the old registration and
explicitly starting a new enrollment. Local credential deletion alone is not
server-side revocation.

## Enrollment protocol

Use Windie-owned pairing, not a claim that Supabase supplies a device-code
grant. Account login stays in the browser. A short code alone must never obtain
a device credential.

1. The Mac generates two independent cryptographically random secrets: a
   pending-enrollment secret and a future device bearer secret (at least 256
   bits each). Save them securely before making requests. Send only their
   domain-separated digests when initiating enrollment, plus a stable random
   enrollment request ID and bounded device metadata.
2. Server initiation is public but rate-limited. Persist the digests, request
   fingerprint, random enrollment ID, short user code, and a ten-minute expiry.
   Return the enrollment ID, code, fixed verification URL, expiry, and minimum
   poll interval. Retrying the same request ID and fingerprint returns the same
   live enrollment; changed parameters conflict. Never log the request payload.
3. The user opens `/devices/connect` and signs in. They enter the code and see
   device details and the account being used. Clearly instruct them to approve
   only a pairing they initiated. GET/preview must not approve anything.
4. An explicit approval POST atomically binds the enrollment to the verified
   account. Concurrent approvals cannot change its owner. Repeated approval by
   the same account is idempotent; another account cannot take over.
5. The Mac polls with the separate enrollment secret in an Authorization
   header. It receives pending/approved/denied/expired, never a browser token.
   Once approved, show server-verified account identity locally and require a
   yes/no confirmation. Do not rely on the self-reported device name as proof
   of who is approving. Extend the verified auth identity projection narrowly
   if a verified email/display label is needed; ownership still uses `sub`.
6. On confirmation, finalize with the enrollment secret and expected approved
   account binding. In one transaction, create the account-owned device and
   activate the previously registered device-secret digest. No secret needs to
   be returned by the server. Mark the enrollment consumed and return device ID.
7. Atomically promote local pending state to the active credential. Finalize
   retries within the enrollment window return the same device, not duplicates.
   If the response was lost and enrollment has expired, recover via device
   `GET /self` using the already-saved device secret before starting over.
8. Denial, expiry, or local cancellation cannot activate the credential. A
   completed/cancelled enrollment cannot be rebound. No automatic re-enrollment
   after revocation or invalid credentials.

Use a human-readable code with at least 50 bits of pseudorandom output,
normalized for entry, unique among unexpired enrollments. Derive it from the
random enrollment ID using a standard HMAC implementation and a server-only
enrollment key; use a distinct domain label for its persisted lookup digest.
This lets identical initiation retries recover the same code without storing
plaintext codes or adding encrypted payload storage. Resolve active-code
collisions by generating a new enrollment ID inside initiation. Retain the key
across process restarts; key replacement explicitly expires pending enrollments
but must not invalidate active device credentials. Use a maintained HMAC/CSPRNG
dependency rather than inventing cryptography. Do not persist plaintext
device/enrollment bearer secrets. Domain-separated hashes of high-entropy
bearers are sufficient for lookup; do not confuse these with human passwords.

Public enrollment and authenticated code lookup require bounded request sizes,
expiry cleanup, per-source/per-account throttling and bounded failed attempts.
Use PostgreSQL-backed bounded counters for security limits across restarts and
multiple server instances, plus optional edge limits. Specify concrete limits
in tests (initial targets: 5 initiations/source/minute, 10 code attempts/account/
minute, polling no faster than every 3 seconds). Trust forwarded client IP only
from the configured trusted ingress; never trust arbitrary forwarded headers.
Responses use `Cache-Control: no-store`; avoid exposing whether a code belongs
to another account. Handle rate limiting with `429` and `Retry-After`.

## Proposed API contracts

Names below should be finalized in contract tests before handlers are added.
Use typed request/response structs and stable error codes, not string matching.

| Method and path | Authority | Purpose |
| --- | --- | --- |
| `POST /v1/device-enrollments` | Public, rate-limited | Initiate or idempotently retry enrollment. |
| `POST /v1/device-enrollments/lookup` | Browser account | Preview an entered code without consuming it. Code stays in body, not URL. |
| `POST /v1/device-enrollments/approve` | Browser account | Approve the code explicitly and bind the account. |
| `POST /v1/device-enrollments/deny` | Browser account | Explicitly deny a still-pending code. |
| `GET /v1/device-enrollments/{id}` | Enrollment secret | Poll state; secrets/identity are not public. |
| `POST /v1/device-enrollments/{id}/finalize` | Enrollment secret | Confirm account binding and activate once. |
| `POST /v1/device-enrollments/{id}/cancel` | Enrollment secret | Cancel before activation. |
| `GET /v1/devices` | Browser account | List only owned registrations and derived presence. |
| `POST /v1/devices/{id}/revoke` | Browser account | Idempotently revoke registration and invalidate its lease. |
| `GET /v1/agent/self` | Device credential | Read own identity/registration without touching presence. |
| `POST /v1/agent/connect` | Device credential | Acquire a presence lease for a run instance. |
| `POST /v1/agent/heartbeat` | Device credential + lease | Renew current lease. |
| `POST /v1/agent/disconnect` | Device credential + lease | Release only the matching current lease. |

Route groups must not inherit the wrong auth middleware. Device ID and account
on agent requests come from the credential record, not request-supplied claims.
Foreign device list/revoke targets return non-disclosing not-found responses.
Unsupported protocol versions get an explicit non-retryable compatibility error.

## Persistence and presence semantics

Add a new versioned migration; never edit already-applied migrations.

- `devices`: typed ID, immutable owning account, bounded display name, OS,
  architecture, agent/protocol versions, creation/revocation timestamps.
- `device_credentials`: unique bearer digest, device reference, created/revoked
  timestamps. One active credential per device in this slice. No browser tokens.
- `device_enrollments`: request ID/fingerprint, secret digests, code lookup data,
  bounded metadata, expiry, lifecycle state, approved account, resulting device.
- `device_presence`: one current run instance/lease per device, server-assigned
  lease ID, last seen and expiry using database time.
- Bounded enrollment-abuse counters and minimal registration/revocation audit
  records; no secret-bearing bodies. Retention/cleanup must be explicit.

Use foreign keys, uniqueness, account-scoped operations, and row locks for
approval/finalization/revocation/lease transitions. Do not reuse the conversation
idempotency table for anonymous enrollment or store devices as conversations.
Registration/revocation may publish account-change events transactionally; do
not emit conversation changes or durable session events for heartbeats.

Presence is **recent authenticated contact**, not proof that tools are ready:

- Initial constants: heartbeat every 20 seconds with jitter; lease lasts 90
  seconds; each HTTP request times out after 10 seconds. Centralize constants.
- Online means active registration and an unexpired lease according to server
  time. Expiry derives offline state on read; no scheduler is required to flip
  a boolean. Revoked always overrides online.
- `run` generates one instance ID. Repeated connect with that instance is
  idempotent. Another instance cannot steal a still-live lease; return conflict
  and exit with an actionable message. Reacquire after expiry, not by force.
- Renew/release require the current lease ID. A late heartbeat or disconnect
  from an old process cannot revive/release a newer lease. Serialize against
  revocation so no racing request can undo it.
- Retry network/5xx failures with jittered exponential backoff (1–30 seconds);
  honor `Retry-After` for 429. Never accumulate concurrent requests or tasks.
- Device 401/revoked response is terminal: stop automatic requests and require
  explicit operator action. A stale lease uses the bounded reacquisition path;
  protocol errors and active-instance conflict are not infinite retries.
- Sleep, process kill, network loss, and server restart preserve registration;
  old presence expires naturally. Never claim immediate detection of unplugging.

This slice uses periodic outbound HTTPS requests, not a persistent command
channel. Later dispatch can introduce long polling or WebSockets without
changing device/account ownership. Do not send install/tool commands in a
heartbeat response as an undocumented shortcut.

## Proposed code placement

Keep these responsibilities small; combine adjacent files if implementation
does not justify separate modules. These are not parallel hosted session rules.

```text
src/device/                    shared device IDs, protocol DTOs, lifecycle rules
src/agent/                     local enrollment, credential storage, HTTPS loop
src/cli/agent.rs                parsing for agent commands
src/cli/adapter/agent.rs        terminal prompts/output and agent wiring
src/hosted/device.rs            enrollment/registration workflow
src/hosted/device_api.rs        HTTP adaptation and principal-specific routes
src/hosted/device_auth.rs       device/enrollment credential verification
src/hosted/store/device.rs      PostgreSQL device persistence (new child module)
migrations/hosted/0003_devices.sql
```

Wire through the existing `lib.rs`, CLI command/adapter modules, hosted router,
and migration runner. `src/hosted/store.rs` remains the parent store module;
adding a child must not require unrelated store refactoring. Keep prompts in
the CLI/output boundary and reusable state machines outside HTTP/CLI code.

No giant database/executor trait. Do not change local session orchestration or
start the local API merely to reuse its library functions.

## Ordered implementation phases

### 1 — Lock the contract and pure rules

- Define IDs, credential principal separation, versioned messages, enrollment
  transitions, presence rules, and the keyed short-code derivation contract.
- Write deterministic tests with injected clock/randomness/HTTP dependencies.
- Document account binding and safe recovery before introducing remote writes.

### 2 — Hosted enrollment, registration, and revocation

- Add migration, account-scoped persistence, credential hashing, rate limits,
  expiry cleanup, and thin route adapters.
- Prove exactly one registration under concurrent approval/finalize retries.
- Prove credential types cannot cross route boundaries and revocation wins races.

### 3 — CLI enrollment and protected local state

- Add connect/status and crash-safe pending/active credential storage.
- Test lost initiation/finalize responses, local decline, disk failure, unsafe
  permissions, cancellation, and resuming an interrupted enrollment.
- Do not overwrite an existing pairing or expose secrets to the browser.

### 4 — Foreground presence loop

- Add run, single-process guard, lease acquisition/renewal/release, bounded
  retries, Ctrl-C handling, and redacted diagnostic output.
- Prove it works without local API, Bifrost, plugins, or conversation writes.

### 5 — Minimal browser pairing and computer management

- Add explicit `/devices/connect` and `/computers` routes. Preserve `/` New
  Chat and `/c/:id` conversation loading behavior.
- Reuse Supabase sign-in. Preserve only allowlisted same-origin return paths
  across OAuth; current sign-in redirects to the origin, so do not assume the
  pairing page automatically survives login. Never accept arbitrary return URLs.
- Show code entry, device details, signed-in account, consent/deny, expiry,
  and confirmation. No auto-approval on page load or after login.
  Consent must accurately describe registration/presence only; future tool or
  installation permissions are not silently granted by this approval.
- Show owned devices, last-seen/presence and explicit revoke confirmation.
  Poll presence only while the view is visible; refresh on focus, bound retries.
- Handle API 401 by recovering auth once through the shared auth lifecycle or
  returning to sign-in, never repeating rejected approval/list requests in a
  tight loop. The existing chat SSE auth-recovery bug remains separately tracked;
  this feature must not replicate it or claim it fixed without tests.
- No Install, Run tool, or remote-desktop controls enabled by this milestone.

### 6 — Verification and deployment handoff

- Run Rust unit/integration suites and focused frontend tests/build/lint.
- Run PostgreSQL acceptance only against the existing isolated test database.
- Update Backend/Frontend indexes, CLI help, guide, and this status with actual
  results. Preserve remaining hosted/chat verification gaps.
- Deploy only when requested; use an additive migration and staged compatible
  server/client release. Roll back binaries/UI without dropping device data.
  Gate new route availability so rollback can disable registration safely.
- Provide exact manual UI checks to Peter. Do not automate browser testing
  unless explicitly requested. No commit/push is implied by implementation.

## Required tests and live acceptance

Terminal tests must cover:

- Pending → approved → activated, denial/cancellation/expiry, duplicate requests,
  approval ownership races, finalize response loss, and invalid fingerprints.
- Account A cannot list/revoke B's device; the short code cannot poll/finalize;
  an enrollment secret cannot use an agent route; a device token cannot read
  conversations, enroll another device, or use account administration.
- Revocation blocks self/connect/heartbeat and wins concurrent renewal; missing,
  malformed, guessed, expired enrollment credentials fail without disclosure.
- Rate limits survive API restart and work across two instances; bounded bodies,
  no-store responses, safe error messages, and secret-redacted logs.
- Correct online/offline lease boundaries with fake time; stale heartbeats,
  duplicate run, process restart, server restart, network loss, 429, and 5xx.
- No auth-failure retry storm; no credentials sent to redirected/untrusted hosts.
- Local storage permissions/atomicity/locking and recovery after interrupted
  writes; revocation does not silently erase recovery information.
- Existing local API/pairing, conversation routes, session streaming, and query
  behavior remain unchanged. HTTP protocol tests do not require an LLM call.

Manual/live proof (record each independently; all start unchecked):

- [ ] Pair the Mac through Google sign-in and explicit browser + local approval.
- [ ] Run the agent and see the same device online in the account's Computers view.
- [ ] A second account cannot see or revoke that device.
- [ ] Stop/kill the agent; graceful release or lease expiry yields offline state.
- [ ] Restart it and recover the same device ID without a new registration.
- [ ] Interrupt networking and restore it; presence recovers without duplicate devices.
- [ ] Restart only the hosted service; registration survives and heartbeats recover.
- [ ] Revoke while running; further heartbeats fail and the agent stops retrying.
- [ ] Confirm local API/Bifrost were not required and no tool/plugin action ran.

## Definition of done and next boundary

This milestone is complete when the registration, presence, isolation, and
revocation proofs pass with documented evidence. It is not complete merely
because a CLI process is running or a database row exists.

Next: design targeted plugin installation using the registered device identity,
then capability reporting and durable remote tool assignments. Reuse the local
installer, registry, MCP executor, and shared policy rules. Before any remote
side effects, specify assignment identity, approval, deduplication, uncertain
outcomes, result persistence, and hosted-session continuation separately.
