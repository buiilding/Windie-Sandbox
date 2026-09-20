## Hosted server plan

This builds the main hosted Windie server only. It excludes devices, VMs, remote control, local MCP execution, and the official UI redesign.

The result is:

```text
Browser
  → Supabase Auth
  → hosted Windie server
  → PostgreSQL + private Bifrost
  → streamed hosted chat
```

The existing local API and SQLite runtime remain unchanged.

### Implementation status — 2026-09-20

| Phase | Status | Meaning |
| --- | --- | --- |
| 0 — Decisions and prerequisites | Live verified | Supabase and Google are configured, and the production Google sign-in/return flow has passed live verification. |
| 1 — Hosted-server boundary | Deployed | `windie-server` runs as a restricted system service on the Droplet. |
| 2 — Cloud database | Deployed | Private PostgreSQL has the hosted migrations and account-owned conversation schema. |
| 3 — Account authentication | Deployed | The server requires a Supabase Bearer token and resolves the account from its auth subject. |
| 4 — Conversation APIs | Deployed | Account-scoped `/v1` conversation, message, truncate, and fork routes are live. |
| 5 — Cross-browser synchronization | Deployed | Durable account-change events and replayable `/v1/events` SSE are live. |
| 6 — Cloud-synchronization milestone | Live verified | Google sign-in, same-account two-browser convergence, cross-account isolation, the isolated PostgreSQL acceptance test, and production restart/reconnect recovery all passed on 2026-09-17. |
| 7 — Hosted sessions, Bifrost execution, and wakeups | Partially live verified | PostgreSQL sessions/claims/queues/events/wakeups, private Bifrost/Kimi, and shared replay-plus-live delivery are deployed. Peter reported smooth streaming after the event-hub deployment. Queue-under-load and interrupted-run/restart recovery remain required live proof. |
| 8 — Current Inspector bridge | Implemented; superseded in production | The temporary hosted Inspector provided signed-in hosted streaming. The official UI replaced it at `app.windieos.com`; its September 20 production configuration repair now uses the absolute hosted API URL rather than the development proxy. Authenticated official-client acceptance remains separate and incomplete. |
| 9 — Production operations | Partial baseline; hardening incomplete | Restricted services, private database/gateway, HTTPS routing, and health checks exist. Automated backups/tested restore, rate limits, and the full operations acceptance checklist are not established as complete. |

“Deployed” means the Phase 1–5 service is running behind its restricted public
HTTPS boundary. The isolated PostgreSQL acceptance test proves the server-side
Phase 6 rules, including durable recovery after a new database pool, and the
signed-in browser reconnect proof confirms the deployed client recovers state.
Those Phase 6 proofs were obtained with the earlier hosted client. They are
not new-release acceptance for the official UI; track that separately in
`docs/plans/official-ui-hosted-integration.md`.

### September 20 device-tool protocol and client-configuration checkpoint

- The additive `0004_device_tool_work` migration and the compatible
  `windie-server` binary are deployed. Loopback and public health checks passed,
  unauthenticated device-self access correctly returns `401`, and CORS accepts
  `https://app.windieos.com`.
- The official UI production bundle was repaired after it initially inherited
  the development-only `/hosted-api` proxy. Production now uses public Vite
  variables, an absolute `https://hosted-api.windieos.com` API URL, and ignores
  local dotenv files during its Vercel build. Terminal checks confirmed the
  served bundle contains the absolute API URL and not the development proxy.
- Peter started the current Mac agent with `windie agent run --tools`; it
  connected and reported its explicitly enabled local capabilities. This proves
  agent transport and the opt-in mode. Peter then bound that Mac to a hosted
  session, attached Desktop Commander, approved a `create_directory` call, and
  received a durable tool result followed by same-session completion. Recovery,
  account-isolation, and no-duplicate-execution proofs remain separate.

### Phase 0 — Decisions and prerequisites

1. Keep the `windie-auth` Supabase project active. It is currently active and
   healthy.
2. Keep Supabase for Google sign-in and access tokens only.
3. Run PostgreSQL for Windie-owned data; the browser must never access it directly.
4. Use Windie-managed model-provider access. Provider keys live only in the hosted server’s protected environment.
5. Configure the production hosted-client URL as an allowed Supabase Google
   OAuth redirect.

Recommended deployment shape for the first release:

```text
DigitalOcean Droplet: the main hosted server
├── windie-server
├── PostgreSQL
└── Bifrost (Phase 7 onward)

Cloudflare: DNS, HTTPS proxy, and optional Tunnel
└── public hosted API hostname → the Droplet
```

PostgreSQL and Bifrost remain private. Cloudflare is not where the application
server runs: it routes the public HTTPS hostname to `windie-server` on the
DigitalOcean Droplet. When a Cloudflare Tunnel is used, its connector runs on
that Droplet.

### Identity and consent setup

Before production sign-in is enabled:

1. In [Google Auth Platform branding](https://console.cloud.google.com/auth/branding), configure Google OAuth branding as **Windie**:
   - app name and Windie logo;
   - support email and developer contact;
   - `windieos.com` as an authorized domain;
   - Windie privacy-policy and terms URLs once those pages are live.
2. In [Google OAuth clients](https://console.cloud.google.com/auth/clients), create a Web application client:
   - add `https://app.windieos.com` as an authorized JavaScript origin;
   - copy the exact Supabase callback displayed by the
     [Supabase Google provider settings](https://supabase.com/dashboard/project/dosrpwiiterwggicjpwn/auth/providers). It is expected to be
     `https://dosrpwiiterwggicjpwn.supabase.co/auth/v1/callback`;
   - keep the Google client secret in Supabase only, never in browser code or
     the Windie repository.
3. In the [Supabase Google provider settings](https://supabase.com/dashboard/project/dosrpwiiterwggicjpwn/auth/providers), enable Google and enter that
   client ID and secret. In [Supabase URL Configuration](https://supabase.com/dashboard/project/dosrpwiiterwggicjpwn/auth/url-configuration), set the
   hosted client as the Site URL and allow it as a redirect URL.
4. In Google OAuth Audience, begin in testing and add the two email accounts
   used for the Phase 6 proof as test users.
5. Show a Windie-owned pre-sign-in screen explaining that Google login gives
   Windie only the user’s identity, email address, and basic profile. It does
   not grant Gmail, Google Drive, or file access.
6. Request only the minimal identity scopes: `openid`, `email`, and `profile`.
7. Verify the full production flow on the real hosted domain:
   Windie pre-sign-in screen → Google consent screen branded as Windie →
   return to Windie as an authenticated user.

`app.windieos.com` now serves the deliberately restricted authenticated Phase 6
client. It is not the eventual full Inspector or official UI; it only exposes
the hosted conversation proof surface.

### Phase 1 — Create the hosted-server boundary

Add a new hosted-server binary in this repository:

```text
src/hosted/
  auth.rs
  account.rs
  store.rs
  events.rs
  api.rs
  config.rs

src/bin/windie-server.rs
```

Keep these separate:

```text
src/api/             local loopback API
src/store/           local SQLite store
src/hosted/          cloud server and PostgreSQL store
src/conversation/    shared conversation/message domain types
src/session/         shared session domain types
```

Do not force the current SQLite `Store` into a huge generic database trait immediately. Reuse the existing domain types and invariants; add a focused PostgreSQL `HostedStore`.

### Phase 2 — Add the cloud database

Create versioned PostgreSQL migrations for:

```text
accounts
conversations
messages
message_parts
account_change_events
idempotency_records
```

Later, when hosted model execution begins:

```text
sessions
session_inputs
session_events
pending_approvals
wakeup_definitions
```

Important rules:

- `conversations.account_id` is mandatory.
- `messages.parent_message_id` preserves the existing canonical message-tree structure.
- Every conversation has a monotonically increasing `revision`.
- `account_change_events.id` is a database-assigned replay cursor.
- All account ownership is enforced by server queries, never by a browser-supplied account ID.
- An event is not a duplicate conversation model; it is an activity/change record that points clients toward authoritative state.

### Phase 3 — Authenticate and resolve Windie accounts

For each request:

```text
1. Browser sends Supabase Bearer token.
2. Hosted server validates it with Supabase Auth.
3. Server receives the stable Supabase user ID.
4. Server finds or creates:

   accounts.auth_subject = Supabase user ID

5. All reads/writes are filtered to that account.
```

Initial account creation should use an idempotent insert, so the user’s first two browser tabs cannot create two accounts.

The browser only has the Supabase publishable key. It never receives a service-role key, database URL, or database credential.

### Phase 4 — Implement account-owned conversation APIs

Port the semantics of the existing local conversation API, rather than inventing new ones:

```text
GET/POST  /v1/conversations
GET       /v1/conversations/:conversationId
POST      /v1/conversations/:conversationId/messages
PATCH     /v1/conversations/:conversationId/messages/:messageId
DELETE    /v1/conversations/:conversationId/messages/:messageId
POST      /v1/conversations/:conversationId/truncate
POST      /v1/conversations/:conversationId/fork
```

Each mutation includes:

```text
Idempotency-Key: <client-generated UUID>
If-Match: <last-seen conversation revision>
```

The server uses one PostgreSQL transaction to:

```text
validate account ownership
→ validate tree/session invariants
→ write the change
→ increment the conversation revision
→ append an account change event
→ save the idempotent response
```

Conflict behavior:

- Two new messages under the same parent create valid separate branches.
- A stale update/delete/truncate returns an explicit conflict response.
- Retrying a timed-out request with the same idempotency key returns the original result, never a duplicate message.

### Phase 5 — Implement cross-browser synchronization

Add:

```text
GET /v1/events?after=<eventId>
```

A browser’s initial load returns:

```json
{
  "conversations": [],
  "event_cursor": 418
}
```

It then opens SSE using `after=418`.

If an event occurs while the browser is disconnected, the server reads the saved rows after `418`, sends them immediately as catch-up, then continues with live events.

The browser treats change events as signals to update its authoritative API state. It does not pretend old assistant deltas are newly generated text.

### Phase 6 — Verify the cloud-synchronization milestone

Required tests:

- User A cannot list, inspect, mutate, or subscribe to User B’s data.
- First authenticated request creates exactly one Windie account.
- Two browser sessions converge after a conversation/message mutation.
- Lost connection followed by replay produces no duplicate event application.
- Retried mutation produces one message.
- Stale destructive mutation returns a conflict.
- Tree branching, forks, truncation, and selected-head paths match existing local behavior.
- Server restart preserves all committed data and replay cursors.

Live proof status — 2026-09-17:

- [x] Sign in through Google.
- [x] Create a conversation in one browser and observe it in a second browser
  signed into the same account.
- [x] Confirm the conversation is absent for a different account.
- [x] Refresh/reconnect a signed-in browser after the production
  `windie-server` restart and confirm it recovers the same state.

The Phase 1–5 server is deployed to the DigitalOcean Droplet with private
PostgreSQL and migrations. A Cloudflare Tunnel routes the hosted API hostname,
and `app.windieos.com` serves the minimal authenticated proof client. On
2026-09-17, an isolated PostgreSQL acceptance test passed, the production
service restart passed both loopback and public health checks, and a signed-in
browser reconnected with its existing conversation intact. This bridge is not
the official UI redesign.

At this point the server is a complete hosted account and conversation service, but it does not yet run models.

### Phase 7 — Add hosted sessions, Bifrost execution, and wakeups

Deployment status — 2026-09-18: `0002_sessions.sql` and the Phase 7
`windie-server` binary are deployed to the Droplet. The isolated PostgreSQL
acceptance test passed against the separate `windie_test` database, which is
explicitly denied access to `windie_hosted`. `0002_sessions.sql` adds
account-scoped sessions, durable FIFO inputs, replayable session events, fenced
execution claims, and scheduled wakeups. The hosted worker streams private
Bifrost responses into durable session SSE events and stores the final
assistant message. A private, loopback-only Bifrost gateway now runs the
Windie-pinned Kimi Code provider under its own system account. Its credential
is installed, and a direct private completion with `kimi-code/kimi-for-coding`
passed. A signed-in browser has now received a hosted streamed response. Queue
delivery while a model is still running and restart recovery of an interrupted
run remain pending. On 2026-09-18 the shared live-event delivery implementation
in `docs/plans/shared-live-session-event-delivery.md` was also deployed: a
committed session event is delivered immediately to local SSE subscribers,
PostgreSQL notifications wake other server instances, and durable replay
remains the recovery path.
Peter subsequently reported that streaming works well. The official client's
later disappearing-final-response bug was a separate browser reconciliation
issue, addressed in its own plan; do not confuse it with the old 250 ms hosted
session polling path.

Only after Phase 6 passes, add the hosted runtime:

```text
Browser message
  → hosted server resolves/creates session
  → server claims the session
  → server compiles selected tree path
  → private Bifrost calls provider
  → server persists assistant message and session events
  → browser receives real SSE updates
```

Reuse existing Windie semantics:

- sessions point to conversation heads; they do not copy transcripts;
- every execution has a fresh claim;
- queued input is durable FIFO;
- interrupted server-owned execution becomes `Failed`, never silently replayed;
- the conversation tree is canonical;
- the selected root-to-head path is model context.

Initial hosted chat should expose no machine-local MCP tools. It is text/image conversation plus model responses only.

When hosted tool support is introduced later, the hosted server owns the
durable tool workflow but does not execute a user's local tools itself:

```text
Browser -> hosted server -> model
                         -> persisted tool request and approval policy
                         -> authorized registered machine executes the tool
                         -> persisted tool result and event stream
                         -> hosted server continues the model turn
```

The registered machine, not the hosted server, performs filesystem, browser,
MCP, or other local-machine execution.

The Phase 7 hosted endpoints are:

```text
POST /v1/conversations/:id/sessions/resolve
POST /v1/conversations/:id/query
POST /v1/conversations/:id/continue
GET  /v1/sessions/:id
GET  /v1/sessions/:id/events?after=<eventId>
POST /v1/sessions/:id/stop
POST /v1/sessions/:id/wakeup
```

Then move scheduled wakeups to the hosted server, because it stays alive when browsers close.

The scheduler claims due wakeups with PostgreSQL row locks, creates a fresh
hosted-server execution claim, and sends the session through that same worker.
User input already follows the same session/claim path. Approval results and
device-tool results will become durable wakeup inputs only when their later
workflows exist.

Required verification before this phase is marked live:

- run `postgres_phase_seven_session_execution_acceptance` against an isolated
  PostgreSQL database;
- deploy the migration and hosted binary to the Droplet;
- configure a private Bifrost instance and a real hosted model;
- verify a signed-in browser receives persisted deltas and a final assistant
  message, queues a second input while running, and recovers an interrupted
  run as `failed` after restart.

### Phase 8 — Connect the current Inspector

Historical implementation — 2026-09-18: the temporary hosted client creates
conversations with the deployment-selected model, resolves hosted sessions,
sends through `/query`, and consumes per-session durable SSE. Its production
bundle was deployed and a signed-in hosted response streamed. The official
client replaced it on the public hostname in release `47b8d49`; its new
route/reconciliation/sign-in code has terminal checks but still needs Peter's
authenticated browser acceptance. The earlier Inspector deployment is retained
for rollback. See `docs/plans/official-ui-hosted-integration.md`.

Original bridge scope (now implemented as a temporary client):

- retain Supabase Google sign-in;
- change its production endpoint from the anonymous demo API to the hosted server;
- replace local-runtime pairing UI with account-scoped hosted API state;
- retain its tree, transcript, sessions, and SSE presentation logic;
- keep official UI work separate; its subsequent integration is tracked by the
  official UI plan, not silently added to this bridge's completion criteria.

### Phase 9 — Production operations

Before calling production hardening complete (deployment alone is insufficient):

- database migrations are reviewed and reversible where practical;
- PostgreSQL has automated backups and a tested restore;
- server/Bifrost/database logs and health checks are available;
- provider credentials are server-only secrets;
- no database port or Bifrost port is public;
- rate limits exist on authentication, mutation, and SSE connection paths;
- Cloudflare routes only the hosted API;
- production tests prove browser → Auth → server → database → Bifrost → streamed response.

This plan gives you a real hosted Windie first: accounts, durable shared conversations, cross-browser sync, hosted model execution, sessions, and wakeups—without contaminating it with the later computer/VM system.
