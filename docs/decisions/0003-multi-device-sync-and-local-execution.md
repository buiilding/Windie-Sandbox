# Multi-device synchronization and local execution

## Status

Partially implemented. The authenticated hosted conversation/session foundation
and the first registered-device execution protocol are deployed. The official
UI can bind a device and approve an exact tool call; a real MCP tool round trip,
recovery proof, and authenticated browser acceptance remain pending.

Implementation checkpoint: 2026-09-18. Detailed phase completion and verification
remain in the [hosted server plan](../plans/main-hosted-windie-server.md) and
[official UI integration plan](../plans/official-ui-hosted-integration.md).

## Context

Windie is intended to work across multiple user devices while retaining access
to local computing environments. A user should be able to open the same
conversation on another device and see its transcript, graph, session state,
and wakeup activity.

At the same time, Windie must be able to operate on local resources such as
virtual machines, files, browsers, and computers. Those resources belong to a
specific device and should not be treated as though they are automatically
available everywhere.

The earlier disposable demo used a hosted web application connected through a
tunneled API to a Windie API and LLM gateway running on one virtual machine.
That arrangement is suitable for demonstrating one persistent environment, but
it is not the long-term architecture for account-based multi-device use.
The current official browser client at `app.windieos.com` instead connects to
the authenticated hosted `windie-server`, with PostgreSQL and a private Bifrost
gateway. The local API/SQLite runtime remains a separate supported path.

## Decision

Windie should separate globally synchronized conversation state from
device-local execution state.

The canonical service is the **hosted Windie server**, not the LLM gateway
itself. The hosted server and its account database own:

- account ownership and authentication;
- conversations, messages, and conversation graphs;
- canonical parent links, execution heads, and durable branch metadata;
- durable sessions, session status, and session events;
- wakeups and queued runtime inputs;
- registered devices and computers;
- synchronized talent/extension metadata; and
- synchronization cursors and mutation history.

The LLM gateway is a separate inference service. It receives authorized model
requests from Windie runtime orchestration and communicates with configured
model providers. It is not the source of truth for conversations.

Supabase provides identity; the hosted server validates the access token and
maps its stable subject to an account. PostgreSQL owns account-scoped canonical
conversation/session data. Initial inference uses Windie-managed provider
credentials behind the server-only gateway boundary.

The browser's selected graph head is a view choice, not proof of session
ownership. The backend resolves the conversation/head to an existing session,
no match, or ambiguity; query/continue workflows resolve or create execution
branches and enforce stale-head and claim rules.

Each user device that Windie can operate should run a registered local device
agent or Windie runtime. The device agent owns or accesses:

- the device's virtual machines;
- local files and applications;
- local browsers;
- computer-control sessions; and
- locally installed tools, talents, or extensions.

Device agents should establish authenticated outbound connections to the
hosted server. The production system should not require every user's local API
or virtual machine to be publicly exposed.

The official UI should communicate primarily with the authenticated hosted
server. It should display the canonical conversation everywhere, while routing
computer and local-resource actions to the selected registered device.

The server owns the tool **workflow**, even though the device executes the
tool: record the model request, apply approval policy, authorize/assign a
machine, receive and persist the result, then continue the model turn. The
hosted server must not execute a user's local filesystem/browser/MCP action
itself. That dispatch/approval continuation is not implemented yet.

## Ownership boundaries

| Concept | Canonical owner |
| --- | --- |
| Conversation and graph | Hosted server/PostgreSQL |
| Transcript messages and tool history | Hosted server/PostgreSQL |
| Session record, execution claim, and durable events | Hosted server/PostgreSQL |
| Wakeup definition and queued runtime input | Hosted server/PostgreSQL |
| Selected transcript view and transient stream preview | Browser, projected from backend state |
| LLM provider request | LLM gateway |
| Virtual machine and local files | Registered device agent |
| Computer controls | Device agent, authorized through the hosted server |
| Talent/extension catalog | Hosted server |
| Talent/extension installation and execution | Device agent |

Conversation content should synchronize globally. Machine state should remain
local unless the user explicitly enables a separate file or artifact-sync
feature. For example, a conversation opened on Device B should show that a
file was created on Device A, but the file should not silently be assumed to
exist on Device B.

## Synchronization model

The cloud database should be authoritative. Each client may keep a local cache
and an outbound mutation queue, but it must not infer durable ownership from
stale cached state.

A client's possible outbound/offline mutation queue is distinct from the
server's durable session-input queue. The latter already exists in PostgreSQL
and serializes inputs received while a session is executing.

Synchronization should use:

- globally unique typed identifiers;
- an append-oriented durable event or mutation history;
- per-account revision or cursor values;
- idempotency keys for client mutations;
- replay followed by live event delivery; and
- backend-owned resolution of conversation heads and session ownership.

The conversation tree must remain canonical. If two devices submit changes
against the same stale head, the backend should explicitly resolve the
conflict, usually by creating or preserving separate branches or returning a
stale-head result. Clients should not silently merge or choose ownership.

Full offline conflict-free editing is not required for the first version.

## Runtime flow

1. A browser authenticates through Supabase and sends its access token to the
   hosted server.
2. The UI downloads the account's conversations and synchronization cursor.
3. `/` is a new-chat draft. First send creates a canonical conversation ID and
   navigates to `/c/:conversationId`. Opening that URL loads only that
   conversation; the transcript/composer stay blank while loading. Errors must
   not silently fall back to New Chat.
4. The browser submits the conversation ID, explicit head, and input through
   the query API. The backend resolves/creates the branch and appends the input
   immediately or persists it in that running session's FIFO input queue.
5. The hosted worker claims execution. When a turn finishes, it takes the next
   queued input through the same execution path. Every owned write is fenced
   by the current execution claim.
6. Runtime orchestration requests a model response from the LLM gateway.
7. If the model requests a local action, the authorized task is sent to the
   selected device agent.
8. The device agent performs the action within its permission boundary and
   returns the result.
9. Messages, tool results, session events, and completion state are persisted
   centrally and streamed to all connected devices.

This produces the following relationship:

```text
Browsers on multiple devices
              │
              ▼
Hosted Windie server ─────────> account database
              │
              ├──> LLM gateway ───> model providers
              │
              └──> registered device agents
                         ├── local VM
                         ├── local files and browser
                         └── computer controls
```

Steps involving device agents describe the target architecture, not today's
available hosted tool behavior.

## Current API coverage versus browser coverage

An implemented endpoint does not mean the official UI exposes its operation.
As of September 18, the hosted router defines 17 explicit method/path endpoints
on 15 URL paths, including health and excluding automatic HEAD/CORS OPTIONS.

| Capability | Hosted server | Official browser UI |
| --- | --- | --- |
| List/create/load conversations | Implemented | Connected; first send creates a draft's conversation |
| Select a tree head and resolve a session | Implemented | Connected; ownership remains backend-resolved |
| Query, session load/events, account events, stop | Implemented | Connected, with ordered saved-message reconciliation |
| Direct append/edit/delete message | Implemented | No dedicated controls yet |
| Truncate or fork conversation | Implemented | Not exposed; Branch action remains disabled |
| Continue without new user input | Implemented | Not exposed |
| Schedule wakeups | Implemented | Not exposed |
| Durable session-input queue | Implemented through the existing query route | Not fully exposed: composer is disabled while running; no dedicated queue status UI |
| Device tool execution and approval/result continuation | Deployed protocol; real MCP proof pending | Device binding and exact-call approval controls are connected |

The client can handle a query response marked queued, but that alone is not a
complete queue UI. To expose queueing, allow another send while a session runs,
display server-returned queued input/status, and use `input_queued` /
`input_started` events to distinguish waiting input from a saved tree message.
Do not optimistically insert a queued message into the canonical transcript.
This uses the existing query endpoint; it does not require inventing a separate
enqueue endpoint. Queue listing/editing/removal would require separately
specified contracts and is not implied by the existing query/stop routes.

For streaming, follow the local Inspector's lifecycle: durable user bootstrap,
transient deltas, saved-message upsert, then preview cleanup. Hosted SSE carries
saved-message IDs rather than local SSE's hydrated snapshots; hydrate those
in order before acknowledging their cursor. The browser must not clear a preview
and race a detached reload against completion or restore an older selected head.

The official UI release is deployed and terminal checks passed. Its new-release
Google login, visual streaming continuity, same-account convergence, isolation,
and browser recovery still need explicit manual acceptance. Prior Inspector
proofs do not automatically cover the replacement UI.

## Consequences

- A conversation can be viewed and continued from any authenticated device.
- Graph navigation, branching, sessions, and wakeups have one authoritative
  cross-device state.
- A device can go offline without making its local VM publicly reachable.
- A session can be observed from another device while the hosted worker owns
  orchestration and an authorized device agent executes assigned local tools.
- Concurrent orchestration requires explicit fenced session claims so two
  workers do not unknowingly run/write the same session at once.
- Cloud storage of synchronized conversations becomes a deliberate product and
  privacy responsibility.
- Local files, VM state, browser state, and installed capabilities are not
  automatically portable across devices.
- The LLM gateway should eventually support authentication, quotas, provider
  secret isolation, queueing, and failure recovery; it should be treated as a
  logical service rather than a permanently single physical server.

## Open questions

- Whether later inference options include user-provided credentials or a
  per-device/local gateway; the initial hosted policy is Windie-managed access.
- Queue presentation and any future queue-management contracts beyond the
  existing server-owned FIFO/query behavior.
- Which tool results are stored as full content versus references to local
  files or artifacts.
- Whether and how users may explicitly synchronize files between devices.
- The device-agent transport, enrollment, revocation, and capability policy.

## Related code

- `src/hosted/api.rs`: authenticated hosted route set.
- `src/hosted/store.rs`: account-scoped PostgreSQL persistence and fenced writes.
- `src/hosted/runtime.rs`: hosted session execution, queues, and wakeups.
- `src/hosted/events.rs`: durable replay/live SSE and PostgreSQL notifications.
- `src/conversation/tree.rs`, `src/session/policy.rs`: shared canonical policies
  reused by local and hosted persistence, not duplicate hosted operations.
- `src/session/live_events.rs`: shared committed-event live delivery.
- `vendor/windie-UI-official/app/hosted/`: current official client coordination,
  transcript projection, and sign-in presentation.

- `src/store/conversation.rs`: conversation-level persistence boundary.
- `src/store/message.rs`: canonical conversation-tree persistence.
- `src/store/session.rs`: durable sessions, heads, queues, and events.
- `src/session/manager.rs`: background execution, claims, and wakeups.
- `src/api/runtime_access.rs`: hosted account authorization and local runtime
  access boundary.
- `src/api/session.rs`: session lifecycle and event API.
- `src/llm/gateway.rs`: local LLM gateway lifecycle boundary.
- `vendor/windie-inspector/frontend/`: local runtime Inspector, the behavioral
  reference for session/stream reconciliation; its hosted proof client is no
  longer the primary public UI.
