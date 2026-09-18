# Multi-device synchronization and local execution

## Status

Proposed.

## Context

Windie is intended to work across multiple user devices while retaining access
to local computing environments. A user should be able to open the same
conversation on another device and see its transcript, graph, session state,
and wakeup activity.

At the same time, Windie must be able to operate on local resources such as
virtual machines, files, browsers, and computers. Those resources belong to a
specific device and should not be treated as though they are automatically
available everywhere.

The current disposable demo uses a hosted web application connected through a
tunneled API to a Windie API and LLM gateway running on one virtual machine.
That arrangement is suitable for demonstrating one persistent environment, but
it is not the long-term architecture for account-based multi-device use.

## Decision

Windie should separate globally synchronized conversation state from
device-local execution state.

The canonical server should be a Windie cloud control plane, not the LLM
gateway itself. The control plane and its account database own:

- account ownership and authentication;
- conversations, messages, and conversation graphs;
- selected graph heads and branch metadata;
- durable sessions, session status, and session events;
- wakeups and queued runtime inputs;
- registered devices and computers;
- synchronized talent/extension metadata; and
- synchronization cursors and mutation history.

The LLM gateway is a separate inference service. It receives authorized model
requests from Windie runtime orchestration and communicates with configured
model providers. It is not the source of truth for conversations.

Each user device that Windie can operate should run a registered local device
agent or Windie runtime. The device agent owns or accesses:

- the device's virtual machines;
- local files and applications;
- local browsers;
- computer-control sessions; and
- locally installed tools, talents, or extensions.

Device agents should establish authenticated outbound connections to the
control plane. The production system should not require every user's local API
or virtual machine to be publicly exposed.

The official UI should communicate primarily with the authenticated control
plane. It should display the canonical conversation everywhere, while routing
computer and local-resource actions to the selected registered device.

## Ownership boundaries

| Concept | Canonical owner |
| --- | --- |
| Conversation and graph | Cloud control plane/database |
| Transcript messages and tool history | Cloud control plane/database |
| Session record and durable events | Cloud control plane/database |
| Wakeup definition and queued input | Cloud control plane/database |
| LLM provider request | LLM gateway |
| Virtual machine and local files | Registered device agent |
| Computer controls | Device agent, authorized through the control plane |
| Talent/extension catalog | Cloud control plane |
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

1. A device authenticates to the Windie control plane.
2. The UI downloads the account's conversations and synchronization cursor.
3. Creating a chat creates or reserves a canonical conversation ID, then the
   UI navigates directly to `/c/:conversationId`.
4. Sending a message stores the user input in the canonical conversation.
5. The control plane creates or resumes a session at the selected graph head.
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
Windie cloud control plane ───> account database
              │
              ├──> LLM gateway ───> model providers
              │
              └──> registered device agents
                         ├── local VM
                         ├── local files and browser
                         └── computer controls
```

## Consequences

- A conversation can be viewed and continued from any authenticated device.
- Graph navigation, branching, sessions, and wakeups have one authoritative
  cross-device state.
- A device can go offline without making its local VM publicly reachable.
- A session can be observed from another device while execution remains owned
  by a selected device agent.
- Concurrent execution requires explicit session claims or leases so two
  devices do not unknowingly run the same session at once.
- Cloud storage of synchronized conversations becomes a deliberate product and
  privacy responsibility.
- Local files, VM state, browser state, and installed capabilities are not
  automatically portable across devices.
- The LLM gateway should eventually support authentication, quotas, provider
  secret isolation, queueing, and failure recovery; it should be treated as a
  logical service rather than a permanently single physical server.

## Open questions

- Whether the hosted control plane will use the existing hosted-account and
  Supabase boundary or another account database.
- Whether model requests use Windie's centrally operated provider credentials,
  user-provided credentials, or a per-device/local gateway option.
- How a session is selected when a user sends input from a second device while
  another device is actively executing.
- Which tool results are stored as full content versus references to local
  files or artifacts.
- Whether and how users may explicitly synchronize files between devices.
- The device-agent transport, enrollment, revocation, and capability policy.

## Related code

- `src/store/conversation.rs`: conversation-level persistence boundary.
- `src/store/message.rs`: canonical conversation-tree persistence.
- `src/store/session.rs`: durable sessions, heads, queues, and events.
- `src/session/manager.rs`: background execution, claims, and wakeups.
- `src/api/runtime_access.rs`: hosted account authorization and local runtime
  access boundary.
- `src/api/session.rs`: session lifecycle and event API.
- `src/llm/gateway.rs`: local LLM gateway lifecycle boundary.
- `vendor/windie-inspector/frontend/`: current browser client and presentation
  implementation.
