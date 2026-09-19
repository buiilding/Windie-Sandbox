# Hosted Windie server

This guide records the hosted deployment checkpoint and the next device-agent
execution milestone for Windie. It is a deployment handoff and orientation
document, not a replacement for the architectural decision in
[`0003-multi-device-sync-and-local-execution.md`](../decisions/0003-multi-device-sync-and-local-execution.md).

Implementation checkpoint: **2026-09-18**. Deployment and verification statements
below summarize the recorded evidence in the plans; they are not a fresh live
infrastructure audit. A September 19 working-tree addition implements device
enrollment/presence (not deployed); remote tool dispatch remains proposed.

## Purpose

Windie needs a persistent server for account-based, multi-device use. The
server should keep each user's conversations synchronized while local device
agents retain access to the user's virtual machines, files, browsers, and
computer controls.

The central server is responsible for the shared Windie state. It should not
be treated as the place where every user's local machine state lives.

## Current hosting choice

The current hosting provider is DigitalOcean. The first server is a Droplet
with the following observed configuration:

- Name: `windie-main-server-droplet-do`
- Region: `NYC1`
- Operating system: Ubuntu 24.04 LTS x64
- Memory: 2 GiB (resized from the original 512 MiB Droplet)

The Droplet runs `windie-server`, private PostgreSQL, private Bifrost, and a
named Cloudflare Tunnel. Hosted inference uses Windie-managed provider access;
Kimi Code is configured through Bifrost. The server owns conversations,
sessions, execution claims, FIFO inputs, scheduled wakeups, and durable events.

Phase 6 passed Google sign-in, same-account two-browser convergence,
cross-account isolation, isolated PostgreSQL acceptance, and production
restart/reconnect recovery with the earlier hosted Inspector client. Hosted
model streaming and shared live-event delivery are deployed. Phase 7
queue-under-load and interrupted-run/restart live proofs remain pending.

The official UI replaced the temporary Inspector client at `app.windieos.com`
on September 18. Its release/build and public asset checks passed; the new
release's authenticated browser acceptance remains separate from the earlier
Inspector proof. See the [official UI integration plan](../plans/official-ui-hosted-integration.md).

Do not store the Droplet's public IP address, passwords, private SSH keys,
provider credentials, Cloudflare tokens, or other secrets in this document.

## Current server layout

One persistent Linux server currently hosts these logical components:

```text
DigitalOcean Droplet
├── windie-server: authenticated API, session worker, wakeup scheduler
├── PostgreSQL: account-scoped conversation/session/event persistence
├── Bifrost: private LLM gateway
└── cloudflared: hosted API tunnel connector
```

Supabase supplies identity; the server validates its access tokens and maps
their subjects to Windie accounts. Bifrost is the LLM gateway, not a second
conversation server. Responsibilities remain separate:

- Windie owns account-scoped conversations, graph heads, sessions, wakeups,
  queues, and durable runtime state. Registered computers, tool dispatch, and
  approval/result continuation are the next extension, not deployed behavior.
- Bifrost receives server-side inference requests, communicates with configured
  providers, and streams responses back to Windie. Windie persists execution
  events and streams them to authenticated browser subscribers.

Bifrost and the Windie API should remain private services on the server. Only
the intended authenticated application/API surface should be reachable by
users.

## Networking

Cloudflare is a networking and DNS layer, not the server host. A Cloudflare
Tunnel may run on the Droplet and connect an application hostname to the local
Windie API without exposing the API's listening port directly.

`app.windieos.com` serves the official UI from Vercel. Its API requests go to
`hosted-api.windieos.com`, routed through a named Cloudflare Tunnel to the
Droplet. The older anonymous demo remains a separate disposable arrangement
and must not be treated as the production account architecture.

The deployed request path is:

```text
Browser loads app.windieos.com from Vercel
        │
        │ authenticated API requests / SSE
        ▼
hosted-api.windieos.com → Cloudflare Tunnel
        │
        ▼
Authenticated Windie API on the DigitalOcean Droplet
        │
        ├── account/conversation database
        └── private Bifrost → model provider
```

User devices that expose local computers or VMs should eventually connect as
authenticated outbound device agents. A tunnel to the central server is not a
substitute for device identity, authorization, or a device-agent protocol.

## Computers and local execution

A user-owned computer and a Windie-managed virtual machine are both
**registered computers**. They differ only in who provides the underlying
machine:

- A user-owned computer runs its own Windie device agent.
- A Windie-managed VM is supplied by a third-party VM provider, then has a
  Windie device agent and remote-control capability installed on it.

The proposed device agent is a Windie process on the computer being operated.
It should reuse the existing local runtime's tool discovery and execution
components, not start another cloud-conversation owner or an independent model
loop for the same hosted session. It executes only enabled capabilities within
the computer's OS and local permission boundaries. It should establish an
authenticated outbound connection to the main server rather than expose a
public local API port directly.

The agent may choose among the user's authorized registered computers for an
operation, but the main server must validate that selection against the
account's permissions and the computer's capabilities before dispatching any
work. An agent must never gain arbitrary network access to other users'
machines merely by naming a computer in a tool call.

The server still owns the tool **workflow**: persist the model's tool request,
apply approval policy, authorize the target computer, assign work, receive and
persist its result, and continue the model turn. The device owns the actual
filesystem/browser/MCP execution and may reject work its local policy denies.
The main Droplet must not execute a user's machine-local tools itself.

Human remote desktop control (a screen plus mouse/keyboard input in the browser)
is a separate capability. It is not required to prove AI tool execution on a
registered computer, and VM provisioning is not part of the first agent slice.

## Data, execution, and wakeup boundary

The target central-server ownership includes the following. Registered-device
and extension synchronization are not yet implemented:

- accounts and access permissions;
- conversation messages and graph branches;
- selected graph heads;
- account-level Windie configuration;
- execution records, queues, statuses, and durable events;
- wakeup definitions, schedule state, and generated inputs;
- registered devices and computer metadata; and
- talent/extension metadata.

The selected device agent should retain responsibility for:

- local virtual machines;
- local files and applications;
- browsers;
- computer-control input; and
- locally installed or enabled capabilities.

Conversation history should be globally synchronized. Local machine state should
not be silently copied to every device. A separate explicit artifact or file
sync feature is required if users need a file created on one device to appear
on another.

One canonical conversation can therefore span work on multiple computers. The
central database records the user/assistant transcript, graph, tool requests,
tool results, and the computer that performed each operation. The physical
files and application state affected by an operation remain on the selected
computer unless a separate sync feature copies them elsewhere.

### Request and execution flow

An ordinary user message and a scheduled wakeup follow the same basic path:

```text
User at app.windieos.com sends input, or a schedule becomes due
        │
        ▼
Windie resolves the branch and persists input immediately or in its FIFO queue
        │
        ▼
Windie resolves the graph head and active execution, then calls the LLM gateway
        │
        ▼
The model response streams through the main server to authorized subscribers
        │
        ▼
If the model needs computer work, Windie authorizes and dispatches it to one
registered computer
        │
        ▼
That computer's local Windie API/runtime performs the work and returns results
        │
        ▼
Windie persists the result and streams the updated conversation state
```

The device-dispatch/result steps describe the next milestone. They are not
enabled by today's hosted text-inference worker. After saving a tool result,
the server must resume the same session's model loop rather than treat the
result as a separate browser-submitted chat.

The main server owns scheduled wakeups because it is persistent. When a
schedule fires, it creates a runtime input and dispatches it like a user
request. If the selected computer is unavailable, the main server must record
and apply an explicit policy such as queue, retry, fail, or ask the user; it
must not silently lose the wakeup.

Device-local sources such as file, browser, and system events are detected by
the relevant device agent. The agent reports that event to the main server,
which creates the canonical wakeup/input record and coordinates the resulting
execution. The server is the authority that prevents duplicate delivery across
devices.

## Reuse the existing runtime

The local API/SQLite runtime remains supported. The hosted server already owns
cloud conversations and inference; device execution is the missing connection:

```text
Local today:
API → shared operations/policies → SQLite → Bifrost → local MCP execution

Hosted target:
API → shared operations/policies → PostgreSQL → Bifrost → device-agent execution
```

Before implementing the agent, read the existing code and API routes. Reuse
conversation/session types, tree and session policies, tool schemas, approval
rules, MCP execution, and result normalization wherever applicable. Different
SQL adapters do not justify duplicating the operation layer into parallel
local and hosted workflows.

Relevant implementation boundaries:

- `src/hosted/runtime.rs`: hosted session orchestration and fenced writes.
- `src/operation/session.rs` and `src/operation/session_approval.rs`: existing
  lifecycle and approval workflows to inspect before extending hosted behavior.
- `src/tool/registry.rs` and `src/tool/policy/`: capability discovery/dispatch
  and shared allow/ask/deny rules.
- `src/mcp/executor.rs` and `src/mcp/result.rs`: execution of approved local
  MCP calls and normalization of their results.
- `src/session/live_events.rs`: shared committed-event publication. Preserve
  durable replay and publish only after database commit.

The agent's local state may include device credentials, capability metadata,
and a durable work/result journal. It is not the canonical hosted transcript.

See:

- [Backend mental model](../index/Backend.md)
- [Frontend mental model](../index/Frontend.md)
- [Multi-device synchronization and local execution decision](../decisions/0003-multi-device-sync-and-local-execution.md)
- [Official UI design](../official-design/README.md)

## Next milestone: one registered computer, one safe tool

The enrollment/connectivity prerequisite is now implemented in source. See
[device-agent setup and recovery](device-agent.md) and the
[enrollment plan](../plans/device-agent-enrollment-and-connectivity.md).
It adds `windie agent connect/run/status`, account-scoped registration/revocation,
fenced presence leases, and browser pairing/Computers views. PostgreSQL proofs
use only the isolated test database. Production deployment and manual proofs
remain pending. **Online does not mean tools can execute yet.** The steps below
remain the subsequent tool-execution target, not functionality delivered by
enrollment alone.

Build an end-to-end device execution slice before expanding to general computer
control. Both the hosted dispatch side and the device agent are required.

1. **Enrollment and identity.** Explicitly link a computer to the signed-in
   account, issue a device-scoped revocable credential, and track online/offline
   status. A device must not impersonate another account or select its own
   account authority from a request field.
2. **Capability reporting.** Report tools actually installed and enabled on
   that computer. Start with one safe read-only capability; expose only the
   authorized selected computer's capabilities to the hosted model.
3. **Durable dispatch.** Persist the tool request and approval decision before
   sending work. Bind each assignment to its account, device, session, model
   tool-call ID, and execution attempt. Select the precise transport separately;
   an authenticated outbound connection is the requirement, not a settled
   WebSocket-versus-HTTP implementation choice.
4. **Local execution and result return.** Validate the assignment locally,
   reuse the existing executor, and return a correlated result. The server
   validates ownership/current assignment, saves the tool result and events,
   then resumes the same session through Bifrost.
5. **Disconnect and retry safety.** Journal accepted work/results, deduplicate
   delivery, reject stale assignments/results, and define timeout/cancellation
   behavior. A lost acknowledgement does not prove the action failed: never
   blindly repeat side effects after reconnect. Persist an uncertain outcome
   for reconciliation when execution cannot be confirmed.

First proof: register Peter's Mac, request the read-only tool through hosted
chat, observe execution on that Mac, and see the saved result and continued
assistant response in the same conversation. Verify that another account
cannot dispatch to it, revocation blocks new work, and reconnect does not
duplicate accepted work. Broader tools and user-facing approvals follow this
slice; authorization and local permission checks are required from the start.

## Remaining verification and operations work

Moving to device agents does not mark every earlier phase complete:

- Finish the official UI release's manual authenticated checks: Google login,
  actual conversation deep links, send/stream/final-response continuity,
  two-browser behavior, and refresh/reconnect.
- Keep queue-under-load and interrupted-run/restart proof pending until tested.
- Complete production hardening, including automated backups, tested restore,
  rate limits, and the operations checklist.
- Keep the isolated PostgreSQL acceptance database separate from production.

Use terminal checks where they can establish behavior. Ask the user to perform
authenticated UI checks unless browser use is explicitly requested. Track
evidence in the [main hosted server plan](../plans/main-hosted-windie-server.md)
and [official UI integration plan](../plans/official-ui-hosted-integration.md).

Successful source builds or a local health response are not proof of a working
hosted deployment. Verify the complete path from the browser to the API,
database, gateway, provider, and streamed response.
