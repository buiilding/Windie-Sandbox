# Inspector API Contract

This document covers the API routes the current Windie inspector actually
requests. It does not list backend routes that have no active inspector caller.

## Connection and Authentication

The inspector uses `VITE_WINDIE_API_URL` when set, otherwise
`http://127.0.0.1:8787`. `VITE_WINDIE_API_TOKEN` supplies the development
token when the URL does not contain one.

The API prints a per-process token in the operator UI URL. The inspector reads
`windie_token` from the URL, saves it in local storage, and sends it as:

```text
X-Windie-Api-Token: <token>
```

JSON requests send `Content-Type: application/json`. Image and SSE requests
send only the token header. Missing or invalid tokens produce `401`.

The API accepts frontend development origins `localhost:3000` and
`127.0.0.1:3000`. JSON request bodies are limited by the backend body-size
limit.

## Startup and Catalog Routes

| Method and route | Inspector use | Request | Important response |
| --- | --- | --- | --- |
| `GET /api/status` | Refresh gateway state | None | `gateway_running` |
| `GET /api/models` | Populate model selector | None | Model IDs and token limits |
| `GET /api/model-parameters?model=...` | Populate reasoning controls | Model query parameter | Capability and parameter metadata |
| `GET /api/tools` | Populate attachable tool catalog | None | Provider-neutral tool definitions |

The backend also exposes direct `POST /api/gateway/start` and
`POST /api/gateway/stop` operations. The current inspector does not call them;
gateway lifecycle remains available to the CLI and future clients.

The UI refreshes status on startup, then attempts model and tool catalog loads.
Starting or stopping the gateway triggers another status and model refresh.

`GET /api/tools` may start approved MCP providers to discover their catalogs.
Successful catalogs are cached by the API's process-owned registry.

## Conversation Collection

### `GET /api/conversations`

Loads lightweight summaries for the sidebar. Each summary contains ID,
persisted model, and message count. It does not load message trees.

### `POST /api/conversations`

Creates an empty conversation using Windie's default model. The response
contains `conversation_id`. The inspector then loads the full inspection route.

## One Conversation

### `GET /api/conversations/{conversation_id}`

Returns the read-only inspection snapshot used to reconstruct the inspector's
state. It includes:

- conversation and active message IDs;
- selected model and reasoning;
- system prompt and tool approval mode;
- attached tool schemas;
- complete message tree;
- selected active path;
- exact model-facing context;
- latest compaction when present.

Image parts contain asset identity, MIME type, and byte count rather than raw
bytes. The UI fetches image bytes separately.

Loading a conversation is paired with `GET .../approvals` and normally followed
by a token-count request.

### `DELETE /api/conversations/{conversation_id}`

Deletes the conversation's messages, compactions, attachments, and orphaned
images, then deletes the conversation row. The inspector refreshes the list and
selects another conversation when available.

Runtime runs, run events, and tool-execution claims are conversation-owned and
delete through foreign-key cascades with the conversation.

## Conversation Settings

| Method and route | Body | Result |
| --- | --- | --- |
| `PATCH /api/conversations/{id}/system-prompt` | `{"text":"..."}` | Current system prompt |
| `PATCH /api/conversations/{id}/model` | `{"model":"provider/model"}` | Persisted model |
| `PATCH /api/conversations/{id}/reasoning` | `{"effort":"high"}` or `null` | Persisted normalized reasoning |
| `PATCH /api/conversations/{id}/tool-approval-mode` | `{"mode":"manual"}` or `auto_approve_attached` | Persisted mode |

Changing the model clears the persisted reasoning effort in the store. The UI
then reloads model parameters for the new model. Reasoning updates are the one
setting mutation the inspector applies locally without reloading the complete
conversation because reasoning does not change message context by itself.

The backend also has a DELETE system-prompt route, but the inspector does not
currently call it; it clears the prompt by patching the chosen text behavior.

## Messages and Paths

### `POST /api/conversations/{id}/messages`

The inspector sends:

```json
{
  "role": "user",
  "parts": [
    {"type": "text", "text": "hello"},
    {"type": "image", "path": "/local/path.png"},
    {"type": "image_data", "mime_type": "image/png", "data": "base64..."}
  ]
}
```

Only the parts present are included. The backend validates and inserts the
message below the current active node, then returns its message ID. The normal
send workflow immediately starts a query run.

### `PATCH /api/conversations/{id}/messages/{message_id}`

Body: `{"text":"replacement"}`. Replaces visible text in place and preserves
metadata, role, parent, and descendants.

### `DELETE /api/conversations/{id}/messages/{message_id}`

Splice-deletes a normal message. Tool-call groups are deleted atomically. The
inspector reloads the conversation after completion.

### `POST /api/conversations/{id}/activate`

Body: `{"message_id":"..."}`. Sets the selected node as active. Although the
UI calls its function `setActivePath`, only the leaf ID is sent; the backend
derives the path.

### `POST /api/conversations/{id}/truncate`

Body: `{"message_id":"..."}`. Keeps that message and removes all descendants.

### `POST /api/conversations/{id}/fork`

Body: `{"message_id":"..."}`. Creates a new conversation from the path through
that message and returns the new `conversation_id`. The inspector selects and
loads the fork.

## Image Route

### `GET /api/conversations/{id}/images/{asset_id}`

Returns raw image bytes with the stored MIME type. Access succeeds only when a
message in the requested conversation references the asset. The inspector uses
the response as an image blob.

## Attached Tools

### `POST /api/conversations/{id}/tools`

Body:

```json
{"provider_id":"cua-driver","tool_name":"click"}
```

Finds the provider tool in the registry, converts it to an attachment, persists
it, and returns its model-facing schema name.

### `POST /api/conversations/{id}/tools/batch`

Body:

```json
{
  "tools": [
    {"provider_id":"cua-driver","tool_name":"click"},
    {"provider_id":"cua-driver","tool_name":"type"}
  ]
}
```

Loads each provider catalog at most once and inserts all attachments atomically.

### `DELETE /api/conversations/{id}/tools/{schema_name}`

Detaches one provider-backed schema by its model-facing name. The inspector's
multi-remove action currently sends this request sequentially for each selected
schema; there is no batch-detach route.

The backend also exposes raw `tool-schemas` routes, but the inspector does not
request them. The UI attaches catalog-backed provider tools instead.

## Approval Inspection

### `GET /api/conversations/{id}/approvals`

Returns zero or one approval request. Runtime exposes only the next unresolved
manual call in assistant metadata order.

The inspector fetches approvals whenever it loads conversation state.

## Input Tokens

### `POST /api/conversations/{id}/input-tokens`

Body: `{"model":null}` or an explicit model override. The inspector currently
passes its helper's default `null`, so the backend resolves the persisted model.

The response includes optional input/total counts, measured model, source, and
raw provider data. See [token_count.md](../features/token_count.md).

## Direct Runtime Operations

These routes remain public API primitives even though the current inspector
uses durable runs. They return one JSON response without a replay event
journal, but they acquire and finalize the same durable per-conversation run
ownership record as the run routes. They therefore cannot race durable runs or
CLI operations.

### `POST /api/conversations/{id}/query`

Body: `{"model":null,"reasoning":null}`. Queries Bifrost against the active
path captured at operation admission and persists the assistant message. The
runtime resolves automatically allowed or denied tool calls, but stops when a
call requires manual approval.

### `POST /api/conversations/{id}/approvals/{assistant_id}/{call_id}/approve`

Executes the exactly identified pending tool call and persists its tool output.
The assistant ID keeps the operation independent from the currently selected
path. It stops after that operation and does not automatically query the model
again.

### `POST /api/conversations/{id}/approvals/{assistant_id}/{call_id}/deny`

Persists a rejected tool output for the exactly identified pending call. It
also stops without automatically querying the model again.

## Backend-Owned Runs

The active inspector uses durable runs instead of the older response-owned
stream helpers.

### `POST /api/conversations/{id}/runs`

Body:

```json
{"model":null,"reasoning":null}
```

Creates a query run and returns its snapshot immediately. The API spawns the
runtime task after durable run state exists.

### `POST /api/conversations/{id}/approvals/{assistant_id}/{call_id}/approve-run`

Starts a run that approves, executes, and persists the identified pending call.
It may continue on that branch into another model turn when no later manual
approval remains.

### `POST /api/conversations/{id}/approvals/{assistant_id}/{call_id}/deny-run`

Starts a run that persists a rejected tool output and may continue when no
later approval remains.

### `GET /api/conversations/{id}/active-run`

Returns `{"run":null}` or the newest running run for that conversation. The
inspector calls it after selecting a conversation so a browser reload can
resume following existing work.

### `GET /api/runs/{run_id}/events?after={sequence}`

Streams SSE. The API replays persisted events after the cursor and then follows
live events. The inspector tracks the highest received sequence.

### `POST /api/runs/{run_id}/cancel`

Signals cooperative cancellation, waits for active model or tool work to
acknowledge cleanup, persists cancellation, and returns the updated run
snapshot. Disconnecting the SSE request alone does not cancel the run.

See [streaming.md](../features/streaming.md) for events and reconnection flow.

## UI Request Pattern

Most inspector mutations use the same sequence:

```text
send mutation
  -> clear/display errors
  -> optionally refresh conversation list
  -> reload inspection + approvals
  -> request a fresh token count
```

This reload behavior is a client consistency strategy. Backend correctness does
not depend on it.

## Relevant Code

- `src/api.rs`
- `src/operation.rs`
- `dev/windie-inspector/src/lib/windieApi.js`
- `dev/windie-inspector/src/context/WindieContext.jsx`
