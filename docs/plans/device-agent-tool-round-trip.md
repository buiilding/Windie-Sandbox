# First device-tool round trip through the shared Windie foundation

## Status, outcome, and scope

Implementation checkpoint — 2026-09-20. The source work described in Phases
1–5 is now present in this working tree: shared capability/context/progression
helpers are used by the local path and hosted adapter; PostgreSQL persistence,
agent work/report/journal transport, hosted approval/continuation, and the
minimal transcript controls have been added. A live Desktop Commander round
trip now proves the normal hosted path. This is **not** a completion claim:
the focused isolated PostgreSQL acceptance suite passes, while a nonce-bearing
local development fixture, recovery/rollback exercise, and the remaining
adversarial proofs stay open. The compatible hosted-server and official-UI rollout is deployed;
enrollment/presence's remaining independent live checks remain recorded in the
[enrollment plan](device-agent-enrollment-and-connectivity.md).

Current verification evidence:

- `cargo check --all-targets --quiet` passes.
- The focused journal lost-acknowledgement recovery test passes.
- The official UI test suite and production build pass.
- The five focused isolated PostgreSQL acceptance tests pass against the
  dedicated `windie_test` database: account sync, session execution, device
  HTTP, capability-report lease fencing, and device lifecycle.
- A prior full Rust run could not finish because the development machine ran
  out of disk space while plugin tests created temporary package files; do not
  treat that as a successful full-suite result.

### Deployment and live-agent checkpoint — 2026-09-20

- Root commit `095dab05` is deployed to the Droplet with additive migration
  `0004_device_tool_work`. The server is healthy locally and through the public
  API; an unauthenticated agent-self request correctly returns `401`.
- Peter ran the current release build with `windie agent run --tools`. The Mac
  reported itself online with explicitly enabled local capabilities. This is a
  real agent transport/opt-in proof, but it is not a real package/MCP execution.
- `packages/parallel-search` is a useful discovery and assignment smoke test:
  it is a streamable HTTP MCP package backed by Parallel's remote service. It
  cannot prove that a tool action happened on the Mac. The required full proof
  remains a small local development MCP package returning a caller nonce and
  actual local OS/architecture.
- There is currently no CLI package-install command. `windie install` installs
  Windie dependencies, not marketplace packages. Existing local installation
  is exposed through the local Inspector/API; do not document or use a
  nonexistent agent-specific installer.

### Live Desktop Commander round trip — 2026-09-20

The first normal product path is now verified without a mock browser response:

- Peter selected his execution-enabled Mac in a deployed hosted conversation.
- The model received the reported plugin index plus `windie__attach_mcp`, then
  attached 26 Desktop Commander schemas for that immutable session/device
  binding.
- Peter approved the attachment and the user-visible
  `desktop_commander__create_directory` request. This is a local side effect,
  not a read-only fixture.
- The authenticated device completed one assignment. PostgreSQL records its
  `result_saved` status; the same session recorded `waiting_for_tool`,
  `tool_result_saved`, and a final `completed` event in order.

This proves agent transport, reported capabilities, schema attachment, explicit
approval, device-side MCP execution, durable result persistence, and model
continuation. It does not meet the full definition of done below: no
nonce-bearing fixture, crash/retry proof, recovery proof, or adversarial
account/revocation proof was run in this exercise.

Deliver one complete hosted conversation turn that executes a tool on Peter's
registered Mac, persists its result, and continues the model response in the
same session. The model stays behind the hosted Bifrost gateway. The Mac runs
the existing plugin/MCP execution machinery.

**The first proof uses one tool; the implementation is not a hardcoded
one-tool system.** Plugin discovery and the plugin index must work because
the local runtime and agent/hosted path consume the same extracted foundation.
An empty plugin installation must produce a valid empty index, not synthetic
capabilities. Use a small development MCP plugin for the real execution proof.

This is a focused refactor plus the network/persistence infrastructure that
does not exist yet. Merely reading the local implementation and reproducing
its behavior in hosted-only code does not meet this plan.

Included:

- Shared capability/index construction, context assembly, tool-call progression,
  attachment validation, and policy decisions, with existing local callers
  rewired to use them.
- Report installed plugin capabilities from the registered Mac; explicitly bind
  a hosted execution session to that Mac.
- Existing `windie__read_skill` and `windie__attach_mcp` behavior adapted through
  shared rules and the appropriate local/hosted adapters.
- Durable assignments, agent execution journal, result acceptance, and resumed
  model execution; minimal per-call approval and browser status.
- Regression, conformance, failure-recovery, and end-to-end tests.

Excluded:

- `switch_device`, multi-device selection/routing, VM provisioning, remote
  desktop, public local API ports, and a second model loop on the Mac.
- Marketplace installation UI, remote installation/update commands, public
  publication of the proof plugin, background agent installation, or autostart.
- A new plugin format, MCP implementation, conversation model, general-purpose
  broker, universal database trait, or parallel `operation/hosted_*.rs` layer.
- Automatically enabling installed plugins or granting execution permission
  from the enrollment approval or a plugin's self-reported read-only annotation.

The future product flow remains Install → choose computer → install locally →
report readiness. This milestone uses explicit local setup of a test package
through existing installation code.

## Source baseline and mandatory reuse

Read the backend/frontend indexes, [ADR 0003](../decisions/0003-multi-device-sync-and-local-execution.md),
[hosted guide](../guides/hosted-windie-server.md),
[shared-operation plan](shared-operation-persistence-refactor.md), and relevant
local APIs, operations, persistence, and tests before implementing. Older guide
deployment statements predate the September 19 enrollment deployment; use the
enrollment plan for that checkpoint, not as proof of remote execution.

The following findings were checked against source while writing this plan:

| Existing foundation | Required use / present limitation |
| --- | --- |
| `plugin/store.rs`, `plugin/installer.rs`, `mcp/loader.rs` | Reuse package validation/loading and explicit installation. Do not introduce an agent plugin directory format. |
| `tool/registry.rs` | Reuse plugin registration, provider discovery, persistent MCP sessions, and `call_tool`. Registry presence alone does not prove the provider is enabled/ready. |
| `operation/component.rs`, `store/component.rs`, `store/tool_catalog.rs` | Reuse installation/readiness/catalog facts. Discovery may start MCP; heartbeat must not trigger it. |
| `plugin/catalog.rs` | Extract typed index construction/rendering from filesystem and SQLite loading. Current `build_index` reads both; preserve installed/available distinction and lazy skill loading. |
| `runtime/context.rs` | Extract complete context assembly from `&Store` loading. Preserve selected path, system prompt, compaction, attached schemas, and generated plugin index. |
| `runtime/turn.rs`, `runtime/tool_execution.rs` | Share pending-call selection, call ordering, attachment validation, and next-action decisions. Current loop executes immediately against SQLite. |
| `tool/policy/`, `operation/session_approval.rs` | Reuse allow/ask/deny decisions and approval semantics. Hosted approval persistence is new; local workflows still take `Store`. |
| `runtime/mod.rs::RuntimeMessagePersistence` | Existing port still accepts `&mut Store`; do not claim it is already backend-neutral or create a universal replacement trait. |
| `llm/`, `runtime/retry.rs`, `session/live_events.rs` | Reuse model serialization/parsing, model-attempt reset/retry behavior, event types, and publish-after-commit delivery. |
| `hosted/runtime.rs` | Currently passes no tool schemas, rejects tool calls, and combines assistant saving with session completion. Split those transitions. |
| `hosted/store.rs` | Model-path hydration currently sets metadata to `None`. Preserve `MessageMetadata` and tool-result linkage before enabling tools. |
| `tool/result.rs`, `mcp/result.rs` | Reuse normalized results. `ToolExecutionResult.parts` has `serde(skip)`; direct serialization is not a remote result protocol. |
| `agent/`, `device/`, `hosted/device_api.rs` | Extend existing credentials, principal separation, protected local storage, leases, and outbound transport. Current agent is presence-only. |
| Official UI `app/hosted/`, `lib/hosted-types.ts` | Extend existing ordered reconciliation. Current message contract lacks tool metadata; waiting/tool calls must not appear as final completion. |

## Architecture and reuse gates

```text
Mac package files + existing SQLite component/catalog state
  → shared capability projection → authenticated capability report
                                      ↓
Hosted PostgreSQL snapshot → shared context assembly → Bifrost/model
                                      ↓
                     shared next-action + tool policy
                         ↓                    ↓
                  local adapter         hosted adapter
                  registry call         durable assignment
                                              ↓
                             Mac journal → existing registry/MCP
                                              ↓
                         hosted tool-result transaction + continuation
                                              ↓
                            shared context → model → browser SSE
```

The local path remains fully usable without hosted services or device identity.
The agent may read the existing local component/catalog SQLite state; it must
not create local conversations/sessions for hosted work or start a local API
or Bifrost to gain access to library functions. A separate protected agent
journal records delivery/execution recovery only.

Required extractions, with names finalized during implementation:

1. **Capability projection:** load package metadata and component/catalog facts
   in the local adapter; construct a typed snapshot and render `PluginIndex`
   through shared functions. Refactor `PluginCatalog` to call those same
   functions. Hosted code consumes the reported snapshot; it never scans the
   Droplet's installed packages as the user's capabilities.
2. **Context assembly:** add explicit loaded inputs for path, system prompt,
   compaction, attachments, plugin index, and supported control schemas. Both
   the existing local builder and hosted worker call one assembler. Preserve
   byte-equivalent model payloads for unchanged local fixtures.
3. **Tool progression:** extract pure decisions over canonical messages and
   loaded capability/approval facts: call model, resolve the next pending call,
   request approval, persist a denied result, or finish. Both runners invoke
   these decisions. Preserve original sequential tool-call ordering.
4. **Control-tool planning:** share parsing and validation for `attach_mcp`
   and skill references. Attachment planning returns validated tools for the
   respective transaction adapter to save. Local skill reads and remote skill
   reads use the same `PluginStore` reader on the machine that owns the package.

Keep adapters responsible for loading, atomic writes, and immediate execution
versus durable suspension. Shared decisions must not contain SQL, HTTP,
account authentication, or MCP transport details. Do not convert all SQLite
APIs to async merely to share these rules. A narrow port is justified only
where these two callers actually need it.

**Refactor gate:** an extraction is incomplete until the original local
caller uses it and its old duplicate rule is removed. Tests must demonstrate
both call paths consume the extracted code; similar filenames or matching
copied tests are insufficient.

## First-version behavior

### Explicit local execution and one Mac binding

- Preserve `windie agent run` as presence-only. Add an explicit execution
  opt-in, proposed `windie agent run --tools`, whose CLI description states
  that enabled local plugin capabilities can receive approved hosted work.
  Enrollment consent must not silently become execution consent on upgrade.
- Reuse the protected process lock and device credential. Keep heartbeat and
  work handling independent so a long MCP call does not expire presence.
- Execute at most one device assignment at a time for this first version.
- The browser offers an explicit “Use connected Mac” action when exactly one
  owned, online, execution-enabled device is eligible. It submits that device
  ID to the existing query/resolve workflow; the server revalidates it and
  persists the binding for that session. It never trusts a browser-supplied
  account ID. Zero or multiple candidates produce an actionable error.
- Bind before the session's first tool-capable turn. Repeated submissions with
  the same device are idempotent; changing the binding is rejected in v1.
  Existing unbound sessions continue text-only. No silent binding by latest
  heartbeat, no switching, and no device ID added to every MCP tool schema.
- Default to existing ask-for-approval policy. Add only the approval/list and
  approve/deny controls needed for this workflow. Do not import a local
  conversation's auto-approval preference into a cloud conversation.

### Capability reporting and the plugin index

- Build the snapshot from existing installed packages and enabled/readiness/
  discovered-tool facts. Include plugin/component IDs and versions, package
  identity, skill summaries, provider state, schemas, and tool/provider mapping.
  Do not include environment secrets, process command lines, local paths,
  complete transcripts, or arbitrary filesystem contents.
- Use a versioned typed report and server-assigned revision. Report once after
  explicit discovery and again when local package/catalog state changes. A
  restart republishes before accepting new work. Repeated identical reports
  are idempotent; old-run updates are fenced by the current presence lease.
- Normal reporting reads cached discovery results; explicit refresh/startup
  discovery uses existing provider operations. Do not auto-install, enable,
  repair, or launch all known plugins just because the agent connected.
- Persist the report under the authenticated device/account. Reject duplicate
  IDs, invalid schema mappings, reserved control names, unsupported versions,
  and oversized catalogs. A report describes capabilities; it grants no rights.
- Version-bound attachments map an ordinary model-facing schema name to the
  selected device, plugin/component, provider-native name, and report revision.
  A change to that mapping/schema invalidates the attachment until revalidated;
  it must never redirect an existing call to a different provider/version.
- Reuse the compact index renderer even with no installed plugins. The main
  server supplies marketplace availability separately from reported installation.
  An available marketplace listing never becomes executable through reporting.
- `windie__attach_mcp` validates plugin/component membership, enabled/readiness
  state, selected device, and catalog revision using shared rules; the hosted
  adapter persists attachments in PostgreSQL. This control operation executes
  on the server because it changes hosted conversation context, not Mac state.
- `windie__read_skill` requests the precise installed plugin/version/skill from
  the Mac through the assignment channel and existing bounded package reader.
  Skill content is loaded on demand, not copied wholesale into every prompt.
- The next model call uses the shared assembler with newly attached schemas.
  Never advertise a control tool whose complete hosted handler is missing.

### Remote assignments and results

Use bounded outbound HTTPS long polling for v1, extending the existing agent
client. No WebSocket framework or heartbeat command payloads are needed.
Proposed routes are contracts to finalize with tests before implementing:

| Contract | Authority | Behavior |
| --- | --- | --- |
| `POST /v1/agent/capabilities` | Device + current lease | Validate/publish an idempotent capability snapshot. |
| `POST /v1/agent/work/next` | Device + current lease | Wait up to 20 seconds; return one assignment or an explicit no-work response. |
| `POST /v1/agent/work/{id}/start` | Device + assignment token | Atomically authorize the execution start and return its current status on retry. |
| `POST /v1/agent/work/{id}/result` | Device + assignment token | Accept one correlated outcome idempotently. |
| Existing query/session responses | Browser account | Accept/return the immutable session device binding. |
| Session approval read/approve/deny routes | Browser account | Follow the local API's contract and shared approval rules; never execute in HTTP handlers. |

Retain exact-origin credentials, disabled redirects, no-store, bounded retries,
and terminal authentication failure handling. Scope larger body limits and the
30-second client timeout to work/report routes; enrollment retains its existing
limits and 10-second requests. Limit one outstanding work poll per agent.
Database rows remain authoritative; notifications only wake a waiter. Periodic
bounded checks recover lost notifications and reconnects.

An assignment contains server-issued identity, target device, session/assistant
message/tool-call IDs, origin claim, capability revision, full arguments, expiry,
and a distinct execution authorization token. Typed work variants cover an MCP
call and an installed-skill read. Neither variant accepts an arbitrary shell
command, executable path, or arbitrary file-read instruction.

The agent validates the mapping against its current installed package/registry,
checks local enablement and execution opt-in, and journals it before requesting
start authorization. Immediately before executor entry it durably records
`executing`; after execution it durably stores the normalized result before
posting it. Use existing owner-only storage/atomic-write conventions. Resolve
MCP providers through the registry and reuse `mcp/executor.rs` and
`mcp/result.rs`; do not put an MCP client in `agent/` or `hosted/`.

Return an explicit wire DTO that converts to/from existing
`ToolExecutionResult`, preserving call ID, success/failure, text, and supported
ordered text/image parts. Reuse existing image validation/storage. Do not
serialize `ToolExecutionResult` directly and silently lose `parts`. Set tested
limits for report size/tool count, arguments, skill content, text, decoded
images, and total result payload in the protocol phase. Unsupported or excessive
results produce a bounded explicit failure, not silent truncation or a fake
success. Full artifact/file transfer is deferred.

### Durable hosted lifecycle and claim ownership

Saving an assistant message must be separate from completing its session. Save
the entire `MessageMetadata`, including tool calls and provider reasoning lanes;
preserve it in inspection, fork/path loading, and subsequent model serialization.
Backfill old text messages as metadata-free without rewriting their content.

For the next pending call, shared policy decides deny, ask, or allow. Persist
denial as a linked tool result; persist an approval wait when needed; only an
allowed/approved call can become executable work. Approval is bound to the
assistant message, exact call/arguments, device, and capability revision. A
changed package/capability invalidates approval rather than broadening it.

Add an explicit shared `WaitingForTool` status. All relevant lifecycle matches,
SQLite decoding/constraints where needed, hosted constraints, scheduler rules,
and UI types must recognize it; local execution need not emit it. Preserve
`WaitingForApproval` separately.

In one fenced transaction, persist the assignment, set the session's active
assignment reference and `WaitingForTool`, and yield/release the worker claim.
Add a distinct yielded claim outcome rather than pretending the session has
completed. No database transaction or live model request stays open while the
Mac works.

Result acceptance authenticates the device and locks the session/assignment. It
checks the active assignment, original assistant/call, expected tree head,
execution token, and cancellation/revocation state. The originating claim is
audit provenance, not a reusable write token after it has yielded. An exact
repeat returns the same acknowledgement; a different outcome for the same
assignment conflicts.

For a valid finished result, one transaction appends the linked tool message
and parts, advances the head/revision, marks the assignment result saved, emits
durable events, and schedules a unique continuation through the existing wakeup
infrastructure. Add a typed result-continuation trigger and unique assignment
deduplication; do not insert a fake user message for a tool result.

The scheduler acquires a fresh claim only if that exact result-continuation
still owns the waiting session/head. It transitions back to running and uses
shared progression to handle another pending tool or call the model. This
continues the same session. FIFO inputs drain after the whole turn completes;
they must not jump ahead of pending tool results. Define query behavior while
waiting as durable queueing; explicit continue and scheduled user wakeups must
not bypass the unresolved call. Keep existing approval-wait rejection behavior
unless separately and deliberately changed.

Result acceptance and model continuation are separate recoverable steps. A
crash after saving a result but before waking a worker must resume from the
stored continuation without executing the tool again. Startup recovery must
leave parked tool/approval waits intact. Current startup recovery marks all
running hosted sessions failed; v1 retains a single active hosted worker
deployment and does not claim general multi-instance worker failover. Concurrent
result/claim transactions still require database fencing and race tests.

### Failure behavior required in v1

| Situation | Required outcome |
| --- | --- |
| No plugins | Valid empty index; no pretend execution or hidden OS tool. |
| Plugin disabled/removed or schema changed | Refuse stale work before execution; report a structured unavailable result or invalidate unstarted work. |
| Device offline before execution starts | Keep work pending within a documented deadline; expiry records a not-executed failure and lets the model explain it. |
| Delivery or start acknowledgement lost | Retry the same assignment identity; consult journal/server state, never create a fresh execution automatically. |
| Result acknowledgement lost | Resend the persisted identical result; one canonical tool message and continuation. |
| Crash with journal marked executing and no durable result | Record an uncertain outcome; no automatic re-execution, even for the proof tool. Fail/block the session explicitly for operator recovery. |
| Stop races with result | Lock/serialize the transitions; a cancelled session cannot be revived or continued by a late result. |
| Revocation | Deny new polling/start/results; cancel or fail associated waiting work through a bounded server cleanup path. |
| Tool already started when stop/revoke/disconnect happens | Attempt cancellation if supported; do not claim the action was undone or guaranteed never to finish. Preserve audit/uncertain outcome. |
| Server restarts during wait | Recovered assignment/journal handshake continues; presence lease reacquisition alone never reauthorizes a second execution. |

A device that reconnects with a new presence lease may reconcile its existing
assignment using the current device credential and unchanged assignment token.
An old process cannot start new work with an obsolete lease. New execution
attempts must not invalidate a still-uncertain action and silently repeat it.
Claim fencing protects database writes; it cannot guarantee exactly-once
external side effects. Stop/revoke after result acceptance must also fence the
pending continuation before it can run.

While tool work is unresolved, hosted edit/delete/truncate/fork operations must
preserve call/result integrity. Extract applicable local tool-group rules or
reject mutations that would invalidate active work. After completion, reuse
the local semantic rules for tool groups and metadata copying; do not leave
new hosted tool messages vulnerable to ordinary node-splicing assumptions.

## Persistence and module changes

Add versioned hosted migrations after `0003_devices`; never edit applied SQL.
Persist capabilities, immutable session bindings, device-bound attachments,
approval decisions, assignments/results, message metadata, and deduplicated
continuation references. Use typed domain objects at boundaries, relational
ownership/identity constraints, and JSON only for structured schemas/metadata.
Scope tool-call uniqueness to session plus assistant message plus call ID;
provider IDs are not globally unique execution IDs.

Proposed placement (combine files where a separate module is unjustified):

| Location | Responsibility |
| --- | --- |
| `plugin/catalog.rs`, focused plugin snapshot types | Shared index projection/rendering plus existing local loaders. |
| `runtime/context.rs`, `runtime/turn.rs`, `runtime/tool_execution.rs` | Shared loaded-input assembly and tool progression; local adapters remain callers. |
| `tool/`, `session/` | Existing contracts/policy plus narrowly required wait/control types. |
| `device/` | Versioned capability/work/result DTOs and typed IDs; no MCP details. |
| `agent/capabilities.rs`, `agent/execution.rs`, `agent/journal.rs` | Local loading/reporting, existing executor wiring, protected delivery recovery. |
| `hosted/runtime.rs` | Adapter from shared decisions to model calls, PostgreSQL, and remote suspension. |
| `hosted/device_api.rs` or a focused work HTTP module | Device auth/validation/HTTP adaptation only. |
| `hosted/store/` child modules | Account-scoped capability, assignment, approval, and result transactions. |
| `migrations/hosted/` | Additive schema and constraints. |
| Official UI existing hosted coordinator/types/transcript | Mac binding, approval, tool/wait visibility, ordered saved-message reconciliation. |

Do not create `operation/hosted_tool.rs`, an agent-specific plugin registry, or
a second context builder. Review every new helper against its local counterpart
and record why it is shared or why it is specifically transport/persistence.

## Ordered implementation phases

### Phase 0 — Capture the behavioral baseline

- [ ] Inspect the listed code, local routes, relevant tests, and dirty worktrees.
- [ ] Record existing test/build results; preserve unrelated root and nested
  repository work. The official UI is an independent repository.
- [ ] Define conformance fixtures for empty/installed plugin indexes, lazy
  attachment, skill reads, model context, approval, ordered multiple calls,
  denied results, and continuation. Reuse current fixtures where possible.
- [ ] Select or create a development-only package using the existing package
  format, with one read-only MCP tool and a small skill. Its result includes a
  caller-supplied nonce and actual local OS/architecture so the proof cannot be
  satisfied with enrollment metadata. No real user files or arbitrary commands.

### Phase 1 — Extract and rewire shared foundations

- [ ] Implement the four extractions above; convert original local callers in
  the same changes. Keep existing local API/CLI contracts and policy behavior.
- [ ] Share model request/event/retry behavior where needed; preserve resetting
  failed stream previews rather than concatenating retried assistant attempts.
- [ ] Prove identical local model payloads and tool progression before wiring
  the network path. Identify any intentional hosted differences explicitly.

### Phase 2 — Define protocol and add durable tool persistence

- [ ] Finalize typed report, assignment, start, result, approval, continuation,
  error, timeout, body-size, and compatibility contracts with tests.
- [ ] Add metadata hydration/persistence, attachments, binding, and new records
  through additive migrations and existing account-scoped store infrastructure.
- [ ] Split assistant save from completion; test save/reload through the
  existing LLM serializer, including tool-call IDs and rich results.
- [ ] Implement result idempotency, approval races, waiting transitions,
  yielded/fresh claims, continuation deduplication, and mutation guards.

### Phase 3 — Agent capabilities and existing execution path

- [ ] Add explicit CLI execution opt-in; initialize existing package/registry/
  catalog facilities without local API, Bifrost, or local hosted-chat copies.
- [ ] Publish bounded snapshots and report readiness accurately; empty plugin
  store works. Respect disabled/broken/unavailable components.
- [ ] Implement independent heartbeat/work loops, serial executor, protected
  journal, start authorization, result retry, and uncertain-outcome handling.
- [ ] Execute the proof plugin via real MCP transport and normalize its result
  through existing code. Read its skill through the existing package reader.

### Phase 4 — Hosted orchestration and approvals

- [ ] Bind the session to the registered Mac explicitly; build context using
  the shared index/assembler and device-bound attachments.
- [ ] Implement existing control-tool semantics through shared planning and
  adapters. Persist every assistant call/result, including control calls.
- [ ] Drive policy, assignment, wait, result continuation, and final completion
  through the shared progression functions; no copied hosted tool loop rules.
- [ ] Integrate stop, revocation, deadlines, queues, scheduler, and startup
  recovery. Event publication always follows successful commit.

### Phase 5 — Minimal existing browser integration

- [ ] Add explicit Mac use and compact pending approval with approve/deny.
  Display tool name, target Mac, arguments, waiting status, and result/error.
- [ ] Extend typed messages/events for metadata and `waiting_for_tool` while
  reusing the existing coordinator and stable transcript rows.
- [ ] Hydrate tool and assistant saves sequentially; keep listening across
  approval/tool waits and show the final continued response without reload.
- [ ] Preserve `/` draft and `/c/:id` navigation, text-only sessions, replay,
  token refresh, and account changes. No separate proof page or fake stream.

### Phase 6 — Verify, document, and prepare deployment

- [ ] Run required conformance/regression/protocol tests below and the real
  local-process round trip with an isolated hosted database and fake model.
- [ ] Update indexes, CLI help, agent guide, this plan's evidence, and related
  current-status documentation. Separate source completion from live proof.
- [ ] Prepare a staged compatible rollout with remote execution gated off
  until server, agent, and UI contracts match. Old presence-only agents remain
  usable; a text-only client must not unknowingly enable remote execution.
- [ ] Before any requested production rollout, build/test, back up, validate
  migrations, and inspect unresolved work. Do not downgrade a binary that
  cannot read new wait states while assignments remain active; disable new
  work and drain/reconcile first. Preserve durable data on rollback.
- [ ] Give Peter exact manual acceptance commands/actions using the actual
  implemented CLI and package installation entrypoint. Do not invent a command
  in advance. Use terminal verification; browser automation only if requested.

Creating this plan does not authorize production mutation, installing a real
plugin on Peter's Mac, commit, or push. Implementation can run isolated fixture
installations/tests; live installation and rollout follow explicit requests.

## Verification required for completion

**Shared-code conformance:** both paths use the same fixtures for index output,
selected-path context, attachment validation, pending-call order, approval
decisions, denied-result progression, and model continuation. Test empty plugin
state and at least two different fixture catalogs so the implementation cannot
pass with a proof-plugin special case. Local regression must cover API, CLI,
Inspector contracts, compaction/system prompt, and existing plugin operations.

**PostgreSQL/protocol:** run only against the isolated test database and
disposable schemas. Cover foreign accounts/devices, credential separation,
revocation, stale capability/lease/assignment identities, concurrent claims,
duplicate/conflicting reports/results, expiry, body limits, and no secret
leakage. Verify exact result-message linkage, metadata, image/text parts,
revision/event commits, and one continuation per accepted assignment.

**Crash/recovery:** inject failure before/after journal start, after tool
execution, before/after result commit, and before continuation claim. Prove
resending a completed result never invokes MCP again. An uncertain execution
must remain uncertain rather than masquerading as a known failed action.
Exercise stop/revoke races, network interruption, new presence leases, hosted
restart during wait, and queued input during a tool turn.

**End-to-end automated:** scripted model requests the indexed skill, attaches
the fixture MCP, requests the tool with a unique nonce, receives its linked
result, and emits a final answer. Use the real agent HTTP client, real package
loader/registry/MCP transport, PostgreSQL, and SSE; fake only model output and
external credentials. Assert that tool execution occurs in the device process,
not in the hosted worker. Repeat with no plugins and with a denied approval.

**Browser coordinator:** terminal-run tests for a tool-only assistant message,
waiting/approval states, tool-result hydration, multiple model turns, duplicate
events, reconnect, and final response persistence. Tests must ensure the UI
does not stop subscribing after the first saved assistant/tool call.

Run the repository's Rust library/all-target checks and documentation checker,
plus official UI tests, build, and focused lint. Record actual commands/results;
do not relabel old enrollment or text-chat proofs as tool-execution proof.

Manual/live acceptance (remaining checks):

- [x] Bind the paired Mac in hosted chat, attach Desktop Commander, approve a
  real tool request, persist its result, and complete the same hosted session.
  This was the `create_directory` exercise above.

- [ ] Explicitly set up the development plugin on the paired Mac using existing
  installation code; start the execution-enabled agent with no local API or LLM.
- [ ] Empty installation first reports no plugins/tools; installing/enabling the
  fixture makes its index/tools appear without agent-specific plugin code.
- [ ] In hosted chat, explicitly use that Mac and ask for the fixture's live
  nonce-bearing result. Approve the requested calls through the normal UI.
- [ ] Observe execution on the Mac and a saved, correlated tool result followed
  by a streamed assistant response in the same session, with no page reload.
- [ ] Refresh the conversation; tool history and final response remain present.
- [ ] Disconnect/reconnect after execution or lose an acknowledgement; no repeat
  action and no duplicate tool message occur.
- [ ] Restart the hosted service during a tool wait; the round trip recovers.
- [ ] Denial, another account/device, revocation, and stale capabilities cannot
  authorize execution; stop cannot revive the session through a late result.
- [ ] Local runtime/Inspector execution still works through the shared code.

## Definition of done

The first version is complete when the Mac tool round trip and its recovery
tests pass, the normal official transcript shows the continued model response,
and the local runtime uses the same extracted index/context/tool rules without
regression. Completion requires both reuse evidence and execution evidence.

A new hosted-only tool loop, a special OS-info shortcut, a mocked browser
response, or an online device without an actual MCP call does not satisfy it.
Broader device switching and marketplace UX remain separate work.
