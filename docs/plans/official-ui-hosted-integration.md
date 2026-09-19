## Official UI hosted integration plan

Replace the temporary hosted Inspector proof client at `app.windieos.com` with
the transcript-first official UI, connected to the existing authenticated
hosted Windie server.

This is a browser-client integration plan. It does not redesign the hosted
server, change PostgreSQL ownership, add device agents, add VM control, or
turn the official UI into a second runtime.

## Implementation status — 2026-09-18

The official UI replaced the temporary Inspector at `app.windieos.com` on
September 18. Code is deployed, but authenticated acceptance of this release
is still pending; earlier Inspector proofs do not verify the new client.

| Phase | Status | Evidence / remaining work |
| --- | --- | --- |
| 1 — Hosted contract | Implemented and deployed | Typed `/v1` HTTP requests, account/session SSE, bearer authentication, and production API configuration. |
| 2 — Authentication/bootstrap | Partially implemented; deployed | Supabase Google login, token rotation, sign-out, and account-keyed client remount are wired. Explicit API authorization-failure handling that clears cached transcript state and returns to sign-in is still missing. New-release Google/account-isolation checks remain pending. |
| 3 — Canonical conversations/routing | Implemented and deployed; live acceptance pending | `/` is New Chat; first send creates `/c/<id>`; direct links wait for their own tree; missing IDs stay errors; navigation races are tested. |
| 4 — Query/stream reconciliation | Implemented and deployed; live acceptance pending | Local-Inspector-style query bootstrap, ordered saved-message hydration/upserts, stable assistant rows, replay cursors, token refresh, and stop route. Terminal regression tests pass; browser continuity/recovery/stop checks remain pending. |
| 5 — Feature boundaries | Implemented and deployed | Unsupported device, voice, upload, tool, and message-action controls remain disabled. Computer-control sign-in copy is product direction, not an implemented hosted execution capability. |
| 6 — Test/deploy | Deployed; acceptance incomplete | 28 tests, TypeScript/Vite build, targeted lint, production asset equality, deep-link shell, CORS, and unauthenticated API rejection passed. Full-project lint has existing component findings; authenticated browser checks remain open. |

Published commits in `buiilding/windie-UI-official`:

- `7bf438c`: hosted client integration and transcript reconciliation.
- `47b8d49`: standalone sign-in redesign, approved computer-control copy,
  sign-in tests, and Vercel configuration. Pushed to `origin/main`.

Release: `https://frontend-5485uixh3-peterbuics-8590s-projects.vercel.app`.
Inspector rollback: `https://frontend-cwakw419b-peterbuics-8590s-projects.vercel.app`.
Both belong to the existing Vercel `frontend` project. The public hostname was
explicitly aliased to the new release after deployment became Ready.

## Goal

```text
Browser at app.windieos.com
        ↓ Supabase access token
Official React + TypeScript UI
        ↓ authenticated HTTPS + replayable SSE
Hosted windie-server
        ↓
PostgreSQL + Bifrost
```

The hosted server remains authoritative for accounts, conversations, selected
heads, session resolution, execution state, durable events, and final
messages. The official UI owns only rendering and short-lived interaction
state.

## Current implementation

The official UI at `vendor/windie-UI-official/` now has Supabase authentication,
typed hosted requests, addressable conversations, and durable session SSE.
Its transcript-first visual shell remains independent of runtime execution.

The September 18 client refactor uses the **local Inspector talking to the
local API** as the behavioral reference: `useSessionRuntime`,
`useConversationStore`, `useSessionPreview`, and `projectSessionEvent`.
It does not use the temporary hosted proof client's reload lifecycle as the
reference. Canonical saved messages and transient deltas are separate state.

The hosted Inspector proof client at
`vendor/windie-inspector/frontend/src/components/hosted/HostedConversationClient.jsx`
already proves the required hosted flow:

- Supabase-authenticated browser requests;
- account-scoped conversation list/create/load;
- backend-owned session resolution and query;
- durable session SSE with cursors and reconnect;
- durable account-change SSE.

Do not copy that component wholesale. Its layout is intentionally temporary
and its transcript reload behavior is not suitable for the official UI.
Reuse the deployed hosted transport contracts, but follow local Inspector's
ordered saved-message reconciliation.

## Non-negotiable boundaries

- Do not expose the Bifrost endpoint, database credentials, service-role key,
  or model-provider key to the browser.
- Do not have the browser infer session ownership, choose a session from
  cached state, or write directly to PostgreSQL.
- Do not retain mock streaming timers once real session SSE is connected.
- Do not add local MCP, tool execution, devices, remote control, wakeup
  management, voice, or file upload behavior in this integration.
- Do not create a second hosted API contract if the existing `/v1` routes
  already cover the need.
- Keep the temporary Inspector deploy available until the official client
  passes the live acceptance checks below.

## Implemented client structure

The visual components use a typed hosted-client layer:

```text
vendor/windie-UI-official/
├── app/
│   ├── page.tsx                    transcript and dock presentation
│   └── hosted/
│       ├── auth-screen.tsx         sign-in/loading/error presentation
│       ├── use-hosted-windie.ts    React lifecycle and external-store binding
│       ├── conversation-client.ts route, account, and session coordination
│       └── transcript-state.ts    canonical message/preview projection
├── lib/
│   ├── hosted-auth.ts              browser Supabase client and account session
│   ├── hosted-api.ts               typed authenticated `/v1` requests
│   ├── hosted-types.ts             API payload and UI mapping types
│   ├── sse.ts                     SSE parsing and ordered async delivery
│   ├── conversation-tree.ts        selected path and leaf/head helpers
│   └── conversation-route.ts       /c/<id> URL mapping
└── vercel.json                     Vite build and SPA deep-link fallback
```

The exact file split may stay smaller if clarity is better. The important
boundary is presentation → typed browser client → hosted API.

## Phase 1 — Preserve and document the hosted contract

Before changing the official UI, read the current hosted API routes and the
temporary hosted Inspector implementation. Treat these as the integration
contract:

- `GET /v1/conversations` for list plus account event cursor;
- `POST /v1/conversations` with an idempotency key;
- `GET /v1/conversations/{id}?head={message_id}` for canonical tree/path;
- `POST /v1/conversations/{id}/sessions/resolve` only when the UI needs the
  backend’s resolution result;
- `POST /v1/conversations/{id}/query` to send a message at an explicit head;
- `GET /v1/sessions/{id}/events?after={cursor}` for durable session replay and
  live activity;
- `GET /v1/events?after={cursor}` for account-level changes from another tab.

Document the response shapes in TypeScript at the client boundary. Do not
invent browser-only session rules.

Acceptance:

- every official-client request uses the actual deployed route and payload;
- no private server setting appears in the Vite bundle;
- invalid/stale/ambiguous backend responses remain visible typed errors, not
  silently guessed UI state.

## Phase 2 — Authentication and application bootstrap

Port only the browser Supabase session behavior from Inspector into a typed
official-UI auth boundary:

1. Configure public build-time values for the Supabase URL, publishable key,
   and hosted API origin.
2. Render a Windie-branded sign-in state when no access token exists.
3. Send `Authorization: Bearer <access token>` on every hosted request and
   SSE connection.
4. On token refresh, reconnect account and session streams using their durable
   cursors.
5. On sign-out or an authorization failure, clear browser-only state and
   return to sign-in; do not erase hosted data.

Acceptance:

- signing in returns only that account’s conversations;
- a second browser for the same account can independently sign in;
- expired/revoked access never leaves prior account transcript visible.

## Phase 3 — Replace mock conversation state

Replace `recents`, `MOCK_CONVERSATION_ID`, `MOCK_RESPONSE`, and timer-driven
turn state with server-derived state.

1. Load the conversation list and select a conversation only from the server
   snapshot.
2. Render the server-selected root-to-head path in the official transcript.
3. Make **New chat** navigate to `/` and clear the current view. Do not create
   a durable conversation until the first send. Then create with an idempotency
   key and adopt `/c/<server-issued-id>` without reopening the submitted turn.
4. Populate recent conversations from account-owned list data, using a safe
   server-provided title or a concise derived display label.
5. Load a selected conversation/head before rendering its transcript; preserve
   the selected head in browser navigation state only as a view choice.
6. `/c/<id>` addresses only that conversation. Keep its transcript/composer
   blank while loading. A missing/unauthorized ID displays an error on that
   route, never the New Chat page. Only `/` displays the New Chat landing.

The graph dock should initially show the canonical selected path and allow
opening a message. Its **Branch from here** action must remain disabled until
the matching real branch/message operation is explicitly integrated.

Acceptance:

- refresh restores the selected account-owned conversation from PostgreSQL;
- a different account cannot load its ID;
- another same-account tab’s newly created conversation appears through
  durable account events or a safe refresh.

## Phase 4 — Real query and transcript streaming

Connect the official composer to hosted session execution.

1. On send, submit the current conversation ID, explicit selected head, text
   parts, and selected reasoning setting to the existing hosted query route.
2. Store the returned server session and subscribe before losing the durable
   event cursor.
3. Reduce `assistant_delta`, reasoning, state, failure, cancellation, and
   completion events into a transient transcript row.
4. Preserve one assistant row for a turn. On the saved assistant message,
   merge the durable message into that row in the same state transition; do
   not clear the streaming row and then fetch a replacement.
5. On reconnect or receiver lag, replay after the last durable cursor and
   deduplicate by event ID.
6. Implement **Stop** through the hosted stop route and render the server’s
   terminal state.

The browser does not claim a turn completed because it stopped receiving
chunks. Only a durable terminal server event decides that.

Acceptance:

- a stream remains one visually continuous assistant response from first
  delta through final saved message;
- refresh/reconnect during a response reconstructs the same durable state;
- a second same-account browser observes the completed transcript;
- stale or ambiguous selected heads receive the hosted API’s explicit result.

## Phase 5 — Truthful feature boundaries

Keep non-integrated controls honest:

- Voice: disabled or hidden until a real voice contract exists.
- Attachments: disabled or hidden until the hosted upload/message-part flow is
  implemented.
- Wakeups, Computers, Talents, Review, Browser, Files, and Side chat: retain
  design-preview navigation only if visibly unavailable; do not simulate
  successful actions.
- Assistant action buttons: only enable actions with an existing hosted API
  operation and clear account/branch semantics.

This protects users from believing a mock action changed durable state.

## Phase 6 — Test and deploy the replacement

Automated checks:

```text
npm run build
npm run lint
client unit tests for API mapping, SSE parsing/cursors, and transcript reducer
```

Manual hosted checks, performed by Peter in browsers:

1. Sign in through Google at `app.windieos.com`.
2. Create a conversation, send a message, and watch a continuous assistant
   row become the final response in place.
3. Open a second browser signed into the same account and confirm durable
   conversation/result convergence.
4. Sign in as the other account and confirm the conversation is absent.
5. Refresh during or after a session and confirm replay/recovery.
6. Confirm mock-only controls neither claim success nor alter hosted state.

The original rollout gate was to complete those checks before switching the
hostname. Peter subsequently explicitly authorized deployment; the following
deployment steps are complete, but that authorization does not mark the manual
checks above as passed:

1. build the official UI with production public variables;
2. deploy its static bundle to `app.windieos.com`;
3. retain a versioned rollback artifact for the temporary hosted Inspector;
4. verify public HTTPS, authentication, API origin, SSE, and the full
   browser-to-server-to-Bifrost-to-browser flow.

Release checklist:

- [x] Build with public Supabase settings and the absolute hosted API URL, not
  the development-only `/hosted-api` proxy.
- [x] Publish and alias the official UI to `app.windieos.com`; retain the prior
  Inspector deployment as a rollback target.
- [x] Verify public `/` and `/c/deployment-route-check` return the release HTML,
  and served JS/CSS match the built artifact byte-for-byte.
- [x] Verify production-origin CORS and HTTP 401 for unauthenticated
  conversation requests. These do not prove authenticated user isolation.
- [ ] Verify Google login and sign-out in the deployed official UI.
- [ ] Verify real conversation deep links, New Chat, Back/Forward, and two
  successive streamed responses remaining visible after completion.
- [ ] Verify same-account two-browser convergence and different-account isolation.
- [ ] Verify stream reconnect, refresh recovery, stop, and unsupported controls.
- [ ] Implement/test API authorization-failure cache clearing and sign-in return.
- [ ] Resolve existing full-project lint failures before claiming a clean
  full lint gate; targeted changed-file lint already passes.

## Explicitly deferred

- full message-tree editing/fork/truncate UI;
- tool approval and tool-result presentation;
- uploads and image message parts;
- wakeup management;
- providers and account settings;
- registered computers, device agents, remote desktop, and VM control;
- marketplace/talent management.

Those are separate capabilities. The first official deployment needs a real,
durable, account-authenticated hosted chat transcript—not a false impression
that all later Windie surfaces already work.

## September 18 client reconciliation refactor

Implemented and deployed in the release above:

- `app/hosted/use-hosted-windie.ts`: React lifecycle/external-store binding.
- `app/hosted/conversation-client.ts`: fenced route loading, account summaries,
  backend session resolution, query bootstrap, and per-session replay cursors.
- `app/hosted/transcript-state.ts`: canonical message upserts, transient event
  projection, explicit route visibility, and stable assistant-row keys.
- `lib/sse.ts`: await each event handler before delivering the next record;
  cancel/release readers on exit. Advance client cursors after reconciliation.
- Ordinary hosted queries load the saved user node before subscribing, as local
  Inspector does. Do not wait for `input_started`: the current hosted worker
  emits that event for queued inputs, not ordinary immediate queries.
- Hosted SSE currently carries message IDs instead of local SSE's hydrated
  messages. The adapter fetches missing saved nodes in event order and upserts
  them before clearing previews. Failure retains the preview and retries from
  the preceding durable cursor. No backend deployment is required.
- Account invalidations cannot navigate, clear a preview, or restore an older
  selected head. A second browser reads the server-owned session head when a
  subscribed idle session starts streaming.
- A streamed assistant and its saved response use the same component and key.

Verification: 28 terminal-run tests (25 client/transport/tree/route tests plus
3 sign-in presentation tests), the TypeScript/Vite build, and targeted
lint for the changed client files pass. Full-project lint still reports
unrelated existing component findings. Local `/` and `/c/<id>` HTTP shell
requests both return 200 (not proof of authenticated rendering). Tests
include navigation races, missing routes, duplicate sends, saved-message/account
event interleaving, failed hydration/replay, token rotation, second-browser
session activation, two successive completed turns, and sequential/fragmented
SSE. Production deployment and HTTP/asset checks passed; authenticated browser
and visual acceptance remain unverified. Peter performs those checks unless
he explicitly requests browser automation.
