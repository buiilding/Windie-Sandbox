# HTTP API

The HTTP API is the client-facing interface exposed by the local Windie API
process. It gives the Inspector and other authorized clients access to
Windie's runtime primitives over loopback HTTP.

The API process, its runtime ownership, and its startup and shutdown behavior
are documented in the [API component reference](../components/api.md).

## Owns

The HTTP interface owns:

- the route table and HTTP methods;
- request validation and JSON response shapes;
- CORS and runtime-access authorization;
- mapping requests to shared Windie operations; and
- mapping operation results and typed errors back to JSON or Server-Sent Events
  (SSE).

The default base URL is `http://127.0.0.1:8787`. Browser development clients
may run on ports `3000` or `5173`, and the hosted Inspector uses
`https://app.windieos.com`.

## Does not own

The HTTP adapter does not own conversation storage, session state, model
context, tool policy, tool execution, or provider inference. Route handlers
delegate those responsibilities to the shared store, session, operation,
runtime, and Bifrost boundaries.

The HTTP API also does not own browser presentation. The Inspector renders the
responses and sends user actions; it does not read SQLite or call Bifrost
directly.

## Main flow

1. A client sends an HTTP request to a route below.
2. CORS and runtime-access middleware classify the request as public, trusted
   local-component traffic, a local Inspector session, or a paired hosted
   account.
3. The route handler validates the request and adapts it to the relevant
   shared operation.
4. The shared operation performs the authoritative runtime or store work.
5. The API returns a JSON response, a JSON error, or an SSE stream. JSON errors
   use the `error` and `causes` fields so clients can display the root failure
   while retaining the full error chain.

## Authorization

The API is loopback-bound, but being able to reach its port is not sufficient
for protected runtime access.

- `GET /api/health`, `GET /api/status`, `POST /api/shutdown`, and
  `POST /api/runtime/local-access/exchange` are public lifecycle routes.
- Internal event streams and local Inspector launch-code issuance require the
  private local component credential.
- Other runtime routes require either a local Inspector token issued by this
  API process or a validated hosted account token whose account is paired with
  this local runtime.
- A local Inspector session cannot manage hosted-account pairing.

## API routes

The following route inventory is defined by `src/api/router.rs`. A path segment
in braces is a value supplied by the client, such as a conversation ID or
session ID.

### Runtime access and lifecycle

| Method | Route | Responsibility |
| --- | --- | --- |
| GET | `/api/health` | Confirm that the API process is reachable. |
| GET | `/api/status` | Report local runtime status, including gateway readiness. |
| GET, POST, DELETE | `/api/runtime/access` | Read, create, or remove hosted-account pairing. |
| POST | `/api/runtime/local-access/launch` | Issue a one-time local Inspector launch code to a trusted local component. |
| POST | `/api/runtime/local-access/exchange` | Exchange a launch code for a local Inspector token. |
| POST | `/api/shutdown` | Request graceful shutdown of the API process. |

### Events and development notifications

| Method | Route | Responsibility |
| --- | --- | --- |
| GET | `/api/events` | Stream aggregate durable session events. |
| GET | `/api/events/cursor` | Read the database-wide durable event cursor. |
| GET | `/api/dev/notifications` | Subscribe to the development notification probe. |
| POST | `/api/dev/notifications/assistant-completed` | Send a development assistant-completed notification probe. |
| GET | `/api/dev/tray-notifications` | Compatibility alias for the development notification stream. |
| POST | `/api/dev/tray-notifications/assistant-completed` | Compatibility alias for the notification probe. |

### Models, providers, and environment

| Method | Route | Responsibility |
| --- | --- | --- |
| GET | `/api/models` | List models available through the gateway. |
| GET | `/api/llm/providers` | List configured LLM provider definitions. |
| GET, POST | `/api/llm/providers/{provider}/keys` | List or create keys for an LLM provider. |
| POST | `/api/llm/providers/{provider}/ensure` | Ensure an LLM provider configuration exists. |
| DELETE | `/api/llm/providers/{provider}/keys/{key_id}` | Delete one LLM provider key. |
| PUT | `/api/env` | Store approved manifest-declared provider environment values. |
| GET | `/api/model-parameters` | Return model parameters for a selected provider/model. |

### Tools, plugins, and installed components

| Method | Route | Responsibility |
| --- | --- | --- |
| GET | `/api/tools` | List the available tool providers. |
| GET | `/api/tools/{provider_id}` | List tools discovered from one provider. |
| GET | `/api/plugins` | List marketplace and installed plugins. |
| GET, DELETE | `/api/plugins/{plugin_id}` | Read or uninstall one plugin. |
| POST | `/api/plugins/{plugin_id}/install` | Install one plugin from the marketplace. |
| GET | `/api/providers` | List installed provider components. |
| GET | `/api/providers/chrome-devtools/remote-debugging` | Read Chrome DevTools remote-debugging state. |
| POST | `/api/providers/chrome-devtools/open-remote-debugging` | Open Chrome DevTools remote debugging. |
| GET, DELETE | `/api/providers/{provider_id}` | Read or uninstall one provider component. |
| POST | `/api/providers/{provider_id}/install` | Install one provider component. |
| POST | `/api/providers/{provider_id}/setup` | Run provider setup. |
| POST | `/api/providers/{provider_id}/configuration` | Configure provider credentials or settings. |
| POST | `/api/providers/{provider_id}/enable` | Enable one provider component. |
| POST | `/api/providers/{provider_id}/disable` | Disable one provider component. |
| POST | `/api/providers/{provider_id}/repair` | Repair one provider component. |
| POST | `/api/providers/{provider_id}/health-check` | Check provider component health. |

### Conversations and messages

| Method | Route | Responsibility |
| --- | --- | --- |
| GET, POST | `/api/conversations` | List or create conversations. |
| GET, DELETE | `/api/conversations/{conversation_id}` | Inspect or delete a conversation. |
| POST | `/api/conversations/{conversation_id}/messages` | Insert a message. |
| PATCH, DELETE | `/api/conversations/{conversation_id}/messages/{message_id}` | Update or delete a message. |
| GET | `/api/conversations/{conversation_id}/images/{asset_id}` | Read a stored conversation image. |
| PATCH, DELETE | `/api/conversations/{conversation_id}/system-prompt` | Set or remove the conversation system prompt. |
| PATCH | `/api/conversations/{conversation_id}/model` | Set the conversation model. |
| PATCH | `/api/conversations/{conversation_id}/reasoning` | Set the conversation reasoning mode. |
| PATCH | `/api/conversations/{conversation_id}/tool-approval-mode` | Set the conversation tool-approval mode. |
| POST | `/api/conversations/{conversation_id}/tool-schemas` | Attach a tool schema to the conversation. |
| PATCH, DELETE | `/api/conversations/{conversation_id}/tool-schemas/{name}` | Update or remove a conversation tool schema. |
| GET, POST | `/api/conversations/{conversation_id}/tools` | List or attach tools to a conversation. |
| POST | `/api/conversations/{conversation_id}/tools/batch` | Attach multiple tools to a conversation. |
| DELETE | `/api/conversations/{conversation_id}/tools/{schema_name}` | Detach one tool schema. |
| POST | `/api/conversations/{conversation_id}/truncate` | Truncate the conversation tree at a message boundary. |
| POST | `/api/conversations/{conversation_id}/fork` | Fork the conversation at its current head. |
| GET | `/api/conversations/{conversation_id}/run-approvals` | List approvals for the conversation's sessions. |
| POST | `/api/conversations/{conversation_id}/input-tokens` | Count input tokens for a prospective request. |

### Conversation sessions

| Method | Route | Responsibility |
| --- | --- | --- |
| GET, POST | `/api/conversations/{conversation_id}/sessions` | List sessions or create a branch session. |
| POST | `/api/conversations/{conversation_id}/sessions/resolve` | Resolve the session branch at a selected message head. |
| POST | `/api/conversations/{conversation_id}/query` | Create or resolve a session and queue a query. |
| POST | `/api/conversations/{conversation_id}/continue` | Continue work from a conversation branch. |

### Sessions, events, and approvals

| Method | Route | Responsibility |
| --- | --- | --- |
| GET | `/api/sessions` | List durable sessions. |
| GET, DELETE | `/api/sessions/{session_id}` | Read or delete one session. |
| POST | `/api/sessions/{session_id}/query` | Queue a query for one session. |
| POST | `/api/sessions/{session_id}/continue` | Continue one session. |
| GET | `/api/sessions/{session_id}/approvals` | List pending approvals for one session. |
| GET | `/api/sessions/{session_id}/events` | Stream one session's durable events. |
| POST | `/api/sessions/{session_id}/stop` | Stop one session. |
| POST | `/api/sessions/{session_id}/wakeup` | Trigger an immediate session wakeup. |
| PATCH | `/api/sessions/{session_id}/keep-awake` | Enable or disable the session wakeup interval. |
| POST | `/api/sessions/{session_id}/approvals/{tool_call_id}/approve` | Approve a pending tool call. |
| POST | `/api/sessions/{session_id}/approvals/{tool_call_id}/deny` | Deny a pending tool call. |

## Important invariants

- The API route table is the client contract; runtime authority remains in the
  shared store and operation layers.
- Protected runtime routes require authorization. Loopback network access alone
  is not sufficient.
- Session resolution is backend-owned. Clients send the conversation and
  selected head, and the API returns the authoritative branch result.
- SSE event IDs are durable cursors. Clients can replay events after a
  disconnect instead of relying only on an in-memory browser stream.

## Related code

- [`src/api/router.rs`](../../../src/api/router.rs) — defines the route table,
  methods, middleware, and CORS policy.
- [`src/api/runtime_access.rs`](../../../src/api/runtime_access.rs) —
  authenticates hosted accounts and local Inspector sessions.
- [`src/api/state.rs`](../../../src/api/state.rs) — state shared by route
  handlers.
- [`src/api/error.rs`](../../../src/api/error.rs) — maps typed failures to JSON
  HTTP errors.
- [`src/api/sse.rs`](../../../src/api/sse.rs) and
  [`src/api/event.rs`](../../../src/api/event.rs) — serialize session and
  aggregate event streams.
- [`src/operation/`](../../../src/operation/) — shared workflows called by
  both API and CLI adapters.
