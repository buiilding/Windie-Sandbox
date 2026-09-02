# API

The API is the server that hosts Windie's runtime primitives. It exposes HTTP
API routes for clients to call. Clients connect to this API server, and the
API authenticates requests before allowing access to protected runtime data or
operations.

The API is a running local process that listens for requests and transmits
results and live session events back to clients. By default, it listens on
`http://127.0.0.1:8787`.

## Clients and authentication

The API is bound to the local machine. Being able to connect to its port is not
the same as being authorized to use Windie: protected routes reject requests
without valid credentials.

Currently, the API accepts two browser clients:

- the hosted Inspector at `https://app.windieos.com`, which sends a validated
  hosted account token after the account has been paired with this local
  runtime;
- the local development Inspector started with
  `cargo run --bin windie -- dev run inspector`, which receives a short-lived
  launch code and exchanges it for a local Inspector token.

The API also has narrowly scoped public lifecycle routes for health, status,
shutdown, and exchanging a local launch code. Internal event routes require a
private local component credential. The API does not treat an arbitrary request
from another program on the same machine as an authorized client.

## Owns

The API process owns the runtime boundary between clients and Windie's core
systems. It:

- maps HTTP requests to shared conversation, session, tool, provider, and
  component operations;
- authenticates hosted accounts and local Inspector sessions;
- stores and loads conversations, sessions, messages, approvals, and events
  through SQLite;
- resolves conversation branches and supervises durable session execution;
- builds model context and sends model requests to the Bifrost gateway;
- executes approved tools and persists their results;
- streams replayable and live session events through Server-Sent Events (SSE);
- starts wakeup and session-supervision work and handles graceful shutdown.

## Does not own

The API does not perform provider inference itself. Bifrost is the separate
gateway responsible for communicating with configured LLM providers. The API
also does not own browser presentation: the Inspector displays API responses
and sends user actions, while the tray and notifier are independent local
components.

## Main flow

1. The API process initializes its SQLite-backed state, tool registry, plugin
   catalog, and session manager.
2. A client sends an HTTP request to an API route.
3. Authentication middleware determines whether the route is public, belongs
   to a trusted local component, or requires a local Inspector token or paired
   hosted account.
4. The route handler converts the request into a shared operation. The API
   resolves the selected conversation or session, builds context, and starts
   or advances runtime work when requested.
5. The API sends model requests to Bifrost, executes tools only after the
   approval policy allows them, and persists messages, session state, and
   durable events.
6. The API returns a response or streams events to the client through SSE.

## Important invariants

- The API is the authority for session ownership and conversation-head
  resolution. The Inspector does not infer either from cached browser state.
- Protected runtime routes require authorization. Loopback network access alone
  is not sufficient.
- The API owns durable runtime state; the Inspector owns presentation state.
- Bifrost remains a separate provider boundary and does not own Windie
  conversations, sessions, tools, or SQLite state.
- Session work continues in the API process even if an Inspector tab closes or
  loses its SSE connection.
- API restart recovery does not blindly replay an interrupted external model or
  tool request.

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

## Related code

- `src/api/mod.rs`: starts the API process and initializes shared runtime state.
- `src/api/router.rs`: defines the HTTP route table and CORS policy.
- `src/api/runtime_access.rs`: authenticates hosted accounts and local
  Inspector sessions.
- `src/api/state.rs`: defines the state shared by route handlers.
- `src/api/sse.rs` and `src/api/event.rs`: serialize session and aggregate event
  streams.
- `src/session/manager.rs`: supervises durable session execution inside the API
  process.
- `src/operation/`: shared workflows called by both API and CLI adapters.
