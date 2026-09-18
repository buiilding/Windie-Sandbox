# Hosted Windie server

This guide records the current hosted-server context and the intended first
deployment shape for Windie. It is a deployment handoff and orientation
document, not a replacement for the architectural decision in
[`0003-multi-device-sync-and-local-execution.md`](../decisions/0003-multi-device-sync-and-local-execution.md).

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
- Status: Active

The Phase 1–5 hosted service is deployed on the Droplet: `windie-server`,
private PostgreSQL, a loopback-only listener, and a named Cloudflare Tunnel.
Bifrost and hosted model execution are intentionally not deployed. Phase 6
live verification has confirmed Google sign-in, same-account two-browser
convergence, and cross-account isolation. Server-restart/replay proof and the
isolated PostgreSQL acceptance test remain.

Do not store the Droplet's public IP address, passwords, private SSH keys,
provider credentials, Cloudflare tokens, or other secrets in this document.

## Intended first server layout

For the initial hosted prototype, one persistent Linux server may run these
logical components together:

```text
DigitalOcean Droplet
├── Windie backend/API
├── Account and conversation database
├── Conversation graph and execution persistence
├── Wakeup scheduler and device-work dispatcher
├── User authentication boundary
└── LLM gateway and Bifrost
```

The LLM gateway and Bifrost may share the server with the Windie API at first,
but their responsibilities remain separate:

- Windie owns users, permissions, conversations, graph heads, sessions,
  wakeups, computers, tool history, and durable runtime state. It persists
  input, coordinates execution, and dispatches authorized computer work.
- The LLM gateway accepts authorized inference requests and streams model
  responses back to Windie.
- Bifrost communicates with configured model providers and handles provider
  integration and model-facing gateway behavior.

Bifrost and the Windie API should remain private services on the server. Only
the intended authenticated application/API surface should be reachable by
users.

## Networking

Cloudflare is a networking and DNS layer, not the server host. A Cloudflare
Tunnel may run on the Droplet and connect an application hostname to the local
Windie API without exposing the API's listening port directly.

For Phase 6, `app.windieos.com` serves a minimal authenticated conversation
client and the hosted API is routed through a named Cloudflare Tunnel. This is
not the full Inspector or the official UI. The older anonymous demo remains a
separate disposable arrangement and must not be treated as the production
account architecture.

The eventual production arrangement should be:

```text
app.windieos.com
        │
        ▼
Cloudflare DNS/proxy or Tunnel
        │
        ▼
Authenticated Windie API on the DigitalOcean Droplet
        │
        ├── account/conversation database
        └── LLM gateway/Bifrost
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

Each registered computer runs a local Windie API/runtime as its execution
engine. It has access to that computer's files, applications, browser, VM, and
remote-control surface. It should establish an authenticated outbound
connection to the main server rather than expose a public API port directly.

The agent may choose among the user's authorized registered computers for an
operation, but the main server must validate that selection against the
account's permissions and the computer's capabilities before dispatching any
work. An agent must never gain arbitrary network access to other users'
machines merely by naming a computer in a tool call.

## Data, execution, and wakeup boundary

The central server should synchronize:

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
Windie main server persists the input in the canonical conversation
        │
        ▼
Windie resolves the graph head and active execution, then calls the LLM gateway
        │
        ▼
The model response streams back through the main server to every signed-in UI
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

## Current implementation relationship

The current Windie runtime is organized around a local API, local SQLite
storage, the Bifrost gateway, durable sessions, wakeups, and SSE event delivery.
The browser Inspector is a client of that API. The hosted multi-device model
described here is a proposed extension: the cloud server becomes the canonical
account and conversation store, while local device agents handle machine-local
execution.

See:

- [Backend mental model](../index/Backend.md)
- [Frontend mental model](../index/Frontend.md)
- [Multi-device synchronization and local execution decision](../decisions/0003-multi-device-sync-and-local-execution.md)
- [Official UI design](../official-design/README.md)

## Deployment state and next steps

The current next step is to access and inspect the Droplet. Do not assume the
server is deployed merely because DigitalOcean reports it as active.

1. Access the Droplet through DigitalOcean Web Console or SSH.
2. Verify the Ubuntu version, architecture, resources, disk, and network.
3. Configure SSH and a host firewall before exposing application services.
4. Decide and configure the account/conversation database.
5. Deploy the Windie API and LLM gateway/Bifrost as separately inspectable
   services.
6. Configure authenticated account access; do not use the disposable anonymous
   demo policy for production users.
7. Configure the production hostname and Cloudflare networking.
8. Verify health, authentication, conversation persistence, model responses,
   event streaming, and failure behavior separately.
9. Only then connect the official UI to the hosted API.

Successful source builds or a local health response are not proof of a working
hosted deployment. Verify the complete path from the browser to the API,
database, gateway, provider, and streamed response.
