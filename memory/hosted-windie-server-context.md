# Hosted Windie server: situational context

Read this after a context reset before resuming hosted-server work. It records
decisions, current implementation/deployment facts, and cautions; the phase
plan remains the authority for work scope and completion status.

Last updated: 2026-09-19, after deploying registered-device enrollment and
presence and confirming Peter's Mac pairing/presence proof.
Deployment facts below are checkpoint evidence; recheck live state before
changing production.

## Read first

Before changing hosted behavior, read:

- `AGENTS.md`
- `docs/index/Backend.md`
- `docs/index/Frontend.md`
- `docs/guides/hosted-windie-server.md`
- `docs/decisions/0003-multi-device-sync-and-local-execution.md`
- `docs/official-design/README.md`
- `docs/plans/main-hosted-windie-server.md`
- `docs/plans/shared-operation-persistence-refactor.md`
- `docs/plans/official-ui-hosted-integration.md`
- `docs/plans/device-agent-enrollment-and-connectivity.md`

For every hosted feature, inspect its local counterpart first: the route in
`src/api/`, workflow in `src/operation/`, SQLite behavior in `src/store/`, and
its tests. Reuse domain rules, types, validation, response contracts, and test
intent. Do not reuse local HTTP handlers unchanged: they carry local pairing,
SQLite, `SessionManager`, and local-tool assumptions.

## Product direction and boundaries

Call the service the **hosted server**, not the control plane.

```text
Local today
browser → local `windie api` → SQLite → Bifrost → local MCP/tool execution

Hosted today
browser → `windie-server` → PostgreSQL → private Bifrost → hosted model stream

Hosted later
browser → `windie-server` → PostgreSQL → model
                                        → authorized device agent → tool result
```

The hosted server is not a replacement for the local runtime. Its canonical
domain model must remain Windie's existing one:

- conversations are parent-linked message trees;
- the selected root-to-head path is model context;
- sessions are durable branch-execution records, not copied conversations;
- backend-owned session/head resolution prevents stale browser ownership;
- revisions, idempotency keys, durable events, and execution claims prevent
  duplicate or conflicting work.

SQLite and PostgreSQL are both relational persistence implementations. Do not
create a parallel cloud-chat model because PostgreSQL requires different SQL.
Extract only narrow shared policies when a concrete feature needs both storage
backends; do not build one huge generic database trait.

The hosted server never executes a user's local filesystem, browser, or MCP
tool. Later it owns the workflow: persist tool request, approval, authorized
device assignment, result, and continuation. Device agents, VMs, remote
control, and full official UI capabilities remain later work. The official
transcript client is now deployed; do not mistake its computer-control branding
for implemented hosted device execution.

## Registered-device checkpoint

Device enrollment and foreground presence were deployed on September 19:

- `windie-server` runs the additive `0003_devices` migration behind the
  existing private Cloudflare Tunnel;
- the official UI exposes `/devices/connect` and `/computers` at
  `app.windieos.com`;
- production smoke checks passed for health, device-route authentication,
  public enrollment creation/poll/cancellation, credential separation,
  `no-store`, and allowed-origin CORS;
- Peter confirmed his Mac paired through Google, activated `windie agent run`,
  and appeared online in Computers.

This is **presence only**. It does not activate a local API, SQLite, Bifrost,
MCP process, plugin, tool, remote desktop, or model-directed computer action.
Do not describe an online computer as tool-ready.

Do not mark all enrollment acceptance complete from the pairing report alone:
second-account isolation, offline/reconnect, hosted-service restart recovery,
and revocation while running are still independent checks.

## Authentication and account ownership

```text
Google sign-in → Supabase access token → hosted server validation
→ stable Supabase user ID → Windie account → account-scoped PostgreSQL data
```

Supabase is the identity broker, not the product or conversation authority.
The browser sends the bearer token only to the hosted API; it never receives a
database credential or provider secret. The active Supabase project is
`windie-auth`. Basic sign-in needs only `openid`, `email`, and `profile`.

Google branding is configured for Windie, but Google must accept verified
ownership of `windieos.com` before its account chooser stops displaying the
Supabase project hostname. Preserve the Supabase callback and redirect setup;
complete domain verification in Google Search Console, request branding
re-verification in Google Auth Platform, then wait for publication.

Windie-managed provider access is the initial hosted inference policy. Provider
keys are server-only secrets.

## Implemented hosted server

Phases 1–6 are implemented and live-verified: Google sign-in, same-account
two-browser convergence, account isolation, durable PostgreSQL state, and
restart recovery passed. The hosted server provides account-scoped
conversation/message-tree operations, revision/idempotency handling, durable
account events, and replayable SSE.

Phase 7 is deployed and partially live-verified. PostgreSQL now contains
sessions, FIFO session inputs, session events, execution claims, and wakeups.
The hosted worker resolves selected heads, atomically claims execution,
compiles the selected tree path, streams through private Bifrost, persists the
assistant result/events, drains queued input, and supports durable wakeups.
Tool calls intentionally stop the current hosted worker: device dispatch and
approval continuation have not been implemented yet.

The deployed gateway uses a server-only Kimi Code credential and supports
`kimi-code/kimi-for-coding`. Direct gateway and hosted browser streaming have
been observed. Queue-under-load and interrupted-run/restart recovery still
need explicit Phase 7 live proof before that phase is marked fully verified.

Phase 8's limited Inspector bridge previously ran at `app.windieos.com`. It kept
Google/Supabase authentication, uses the hosted API, creates new conversations
with the deployment default model, resolves/query sessions, renders live
assistant text, and reloads the durable final message. It is a temporary proof
surface, not the final official UI and not a reason to add unsupported local
controls to the hosted server. On September 18 the official React/Vite UI
replaced this proof client on the public hostname; see the client and deployment
checkpoint below. This does not by itself complete any remaining phase proofs.

## Current streaming behavior and implemented improvement

The earlier hosted session stream was durable but visually chunkier than the
local runtime because it used this path:

```text
Bifrost delta → PostgreSQL session_events → 250 ms hosted DB poll → browser
```

The local API persists events then uses an in-process `SessionManager`
subscription for immediate delivery. That was the reference for replacing
the hosted 250 ms session polling path, not a reason to replace persistence.

The shared live-event delivery revision was deployed to the Droplet on
2026-09-18. It preserves durability and reuses the local pattern:

```text
commit PostgreSQL event → publish that returned record to a local event hub
                         → immediately send to connected browsers
reconnect → replay later PostgreSQL rows using the durable cursor
```

`src/session/live_events.rs` now owns the shared process-local hub. The local
`SessionManager` and hosted runtime publish the exact record returned after a
durable commit. Hosted session SSE subscribes before replay, uses the durable
cursor to suppress duplicates/repair lag, receives same-process events
immediately, and uses a slow database fallback only for recovery.

PostgreSQL `LISTEN`/`NOTIFY` sends only the committed event ID to other
`windie-server` instances, which reload the record from PostgreSQL and publish
it locally. Never broadcast before the database transaction commits. The
implementation passed local Rust suites and the deployed service and private
Bifrost health checks. Peter subsequently reported that it streams well.
Later disappearing responses were a separate official-client reconciliation
problem, not evidence that the new event hub should be removed.

## Official client: requirements and reconciliation

Peter explicitly rejected repeatedly patching the temporary hosted Inspector
or duplicating its weak reload lifecycle. Read the **local Inspector interacting
with the local API** first: `useSessionRuntime.js`, `useConversationStore.js`,
`useSessionPreview.js`, `useSessionTransport.js`, and `lib/sessionEvent.js`.
Reuse the responsibilities and lifecycle, adapted to the deployed `/v1` payloads.

The official UI is in the independent Git repository
`vendor/windie-UI-official/` (remote `buiilding/windie-UI-official`). At this
checkpoint it is NOT a registered root Git submodule; root status shows the
directory as untracked. Commit inside it, not as an accidental embedded gitlink.

Required navigation:

- `/` is New Chat, with no database creation until the first send.
- First send creates a conversation and adopts `/c/<server-issued-id>` without
  reopening or clearing the turn being submitted.
- `/c/<id>` loads only that conversation. Its transcript/composer stay blank
  until loaded. Missing/unauthorized IDs show an error on that route, never the
  New Chat landing. The shell may remain visible while loading.
- Backend owns session resolution; a unique tree leaf is only a display choice.
  Multiple branches require choosing a head, not guessing session ownership.

Client structure after refactoring:

- `app/hosted/use-hosted-windie.ts`: thin React external-store/lifecycle binding.
- `app/hosted/conversation-client.ts`: navigation generations, account list,
  query bootstrap, backend session resolution, per-session replay cursors.
- `app/hosted/transcript-state.ts`: canonical message upserts, preview projection,
  route visibility, and stable keys for streamed/saved assistant rows.
- `lib/hosted-api.ts`, `lib/hosted-types.ts`, `lib/sse.ts`: typed authenticated
  requests and ordered asynchronous SSE delivery.

Concrete bugs found and addressed:

- Ordinary immediate hosted queries do NOT emit `input_started`; queued inputs
  do. Like local Inspector, load the saved user node after query and before
  subscribing/replaying. Do not wait indefinitely for a nonexistent event.
- Local SSE hydrates saved messages; current hosted SSE envelopes contain IDs.
  Hydrate missing saved nodes in the ordered event adapter, then upsert them
  before clearing previews. Do not launch detached full-view reloads.
- Terminal events previously overtook pending saved-message fetches; account
  refreshes could restore the old user head and hide a finished assistant.
  Await reconciliation and advance the cursor only afterward. Failed hydration
  retains the preview and replays from before the unprocessed saved event.
- Account invalidations must not navigate or reset an active preview/head;
  late responses from a previous route are fenced by the view generation.
- Streamed and saved assistant rows now use the same component and key.
- Token refresh reconnects without clearing current state. Account changes
  remount the client. Running-session replay reconstructs previews offscreen.

The client refactor did not change/deploy Rust or PostgreSQL. Tests cover route
races, missing routes, duplicate sends, two successive turns, account/save
interleaving, hydration failure/retry, token refresh, replay, second-browser
activation, and fragmented/sequential SSE. These are terminal-run regressions,
not authenticated visual/browser proof.

## Sign-in design and exact approved copy

`app/hosted/auth-screen.tsx` is presentation-only; OAuth stays in
`lib/hosted-auth.ts`. The gate now sits OUTSIDE `SidebarProvider`: that flex
wrapper previously shrink-wrapped the page into the narrow left column shown
in Peter's screenshot. Only authenticated chat is wrapped by the sidebar.

The design uses the existing warm dark palette, a small top-left Windie wordmark,
centered unboxed content, subtle amber accents, and a light Google button with
the provider mark. Loading/configuration errors use the same standalone layout.

Approved copy:

- Eyebrow: `AI that controls computers` (replaced `YOUR SPACE TO THINK`).
- Heading: `Welcome to Windie.`
- Description: `Tell Windie what you need.` / `Let it take care of the clicks.`
- Action: `Continue with Google`.

Do not reintroduce the earlier thought/idea description or change authentication
behavior while making styling adjustments.

## Deployment and operations

The production DigitalOcean Droplet is the hosted account/conversation server,
not a future user remote-control VM. It has a private PostgreSQL database,
loopback-only `windie-server`, private loopback Bifrost, and Cloudflare Tunnel
public routing for `https://hosted-api.windieos.com`. The official browser UI is
hosted separately by Vercel at `https://app.windieos.com`.

The Droplet has 2 GiB RAM. PostgreSQL is conservatively tuned. `windie-server`
and Bifrost run as separate unprivileged service accounts; provider data is not
readable by the Windie application service. A dedicated local SSH deployment
key exists. Never restore or use the older key exposed in a screenshot, and
never record keys, credentials, database URLs, tunnel credentials, or server
addresses in repository files or chat.

Production checks already established include public health, unauthenticated
conversation rejection, restrictive CORS, active Cloudflare routing, private
Bifrost health, a successful Kimi completion, and a compatible hosted
Responses-stream request. Before every deployment, build/test locally as
appropriate, create a release binary on the Droplet, restart only the affected
service, then verify health and logs without printing environment files.

An isolated PostgreSQL test database exists on the Droplet for hosted
acceptance tests. Keep it separate from production; do not run test migrations
or test commands against production data.

### Official UI deployment checkpoint — September 18

Peter explicitly authorized commit, push, and publication to `app.windieos.com`.

- Official UI `7bf438c`: hosted integration, route and transcript refactor.
- Official UI `47b8d49`: redesigned sign-in, approved copy, tests, `vercel.json`.
  Pushed to `origin/main`; this also pushed the preceding local UI commits.
- Root `fe55d1a7`: integration plan/frontend index documentation, committed in
  the prior step. Do not infer that the root branch was pushed with the UI.
- Vercel existing project: `frontend`, scope `peterbuics-8590s-projects`.
- Current release:
  `https://frontend-5485uixh3-peterbuics-8590s-projects.vercel.app`
  (`dpl_13shrJXCRdCwtLCzPRn93Neq7jqc`).
- Previous Inspector rollback release:
  `https://frontend-cwakw419b-peterbuics-8590s-projects.vercel.app`
  (`dpl_6xzRR9beKZfZeahbWGptfLxxvNHt`).

Build used `VITE_WINDIE_API_URL=https://hosted-api.windieos.com npm run build`.
Public Supabase settings came from ignored local environment configuration;
never print/commit secret files. Development still uses `/hosted-api` through
the Vite proxy at `http://localhost:3000`; that proxy is not available in a
production static bundle.

Deployment used a prebuilt static output with a filesystem-first route then
`/index.html` fallback. The Vercel project had legacy CRA/build-directory settings;
do not blindly run its old remote build against the Vite source. The UI's new
`vercel.json` declares Vite, `dist`, and deep-link rewrites. The exact tested
bundle was uploaded with `vercel deploy --prebuilt --prod --skip-domain`, then
`vercel alias set <release-url> app.windieos.com`. No DNS or Droplet change.

Verified after publication:

- 28 tests, TypeScript/Vite build, targeted lint, and whitespace checks passed.
  Full-project lint still has unrelated generated-component findings.
- `/` and `/c/deployment-route-check` on the public hostname returned HTTP 200
  with HTML byte-identical to the release (the latter is only a shell test).
- Served JS/CSS matched local release bytes; the JS contains the production API,
  public auth configuration, and approved new copy.
- Production CORS allows `https://app.windieos.com`; unauthenticated conversation
  requests return 401.

Still unverified for this official UI release: actual Google login, visual
layout, send/stream/final-response continuity, deep-link loading of a real
conversation, two-browser behavior, and browser recovery. Peter was asked to
perform these manually. Do not relabel old Inspector acceptance as proof of
the new official UI. No browser automation was used.

## Implementation structure and cautions

Hosted HTTP handlers should authenticate, validate input, call an operation or
runtime method, and serialize the result. They must not accumulate direct
PostgreSQL mutation logic. `HostedStore` owns account-scoped PostgreSQL
transactions; local `Store` owns SQLite. The initial hosted conversation
operation is not yet a full shared persistence abstraction, so extend it only
when a concrete shared rule justifies a narrow extraction.

The Inspector's normal local runtime path requests local-only models, provider
configuration, tools, plugins, approvals, pairing, and device behavior. The
hosted bridge must continue exposing only hosted capabilities. Do not automate
browser UI tests unless Peter asks; use terminal checks where possible and ask
him to perform authenticated UI proof.

The root checkout, Inspector submodule, and independent official UI repository
may have in-progress work. Preserve unrelated changes. For an explicit commit,
inspect all relevant statuses and stage only intended files. If Inspector
changed, commit it before its root pointer. Do not add the official UI as a
gitlink without an intentional submodule decision. Never push without explicit
authorization. `.playwright-cli/` and `output/` in the official UI were left
untracked, as were unrelated Inspector edits; do not include them in commits.

## Resume sequence

1. Check `docs/plans/main-hosted-windie-server.md` for the authoritative next
   phase and evidence gaps.
2. Read the local equivalent before making any hosted change.
3. Keep durable PostgreSQL events authoritative; never bypass account checks or
   execution claims for convenience.
4. For current UI follow-up, first collect Peter's live official-client results:
   Google login, direct conversation URL, two successive sends whose assistant
   replies remain visible, New Chat, and refresh/reconnect. Diagnose the actual
   deployed release before adding another state/reload workaround.
5. Keep Phase 7 queue and interrupted-run/restart proof gaps explicit. Continue
   only with the next requested/planned capability; do not jump ahead to
   device execution or remote control.
