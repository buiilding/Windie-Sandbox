# Hosted Server Operation Refactor and Inspector Context

## Purpose of this checkpoint

This records the current hosted Windie conversation so a later session can resume without confusing the existing local runtime, the temporary hosted proof UI, and the future official Windie UI.

Read these before changing the architecture or hosted implementation:

- `AGENTS.md`
- `docs/index/Backend.md`
- `docs/index/Frontend.md`
- `docs/guides/hosted-windie-server.md`
- `docs/decisions/0003-multi-device-sync-and-local-execution.md`
- `docs/official-design/README.md`
- `docs/plans/main-hosted-windie-server.md`
- `memory/hosted-windie-server-context.md`
- `memory/hosted-windie-deployment-checkpoint.md`

## Product direction agreed with Peter

Windie is moving toward an authenticated hosted main server. A browser signs in through Google via Supabase, receives an access token, and sends that bearer token to the hosted `windie-server`. The hosted server validates the token, maps it to an account, and reads or writes that account's canonical PostgreSQL conversation state.

The hosted server is not a replacement for the current local `windie api`. The local runtime currently owns local SQLite state, local model streaming, and local tool execution. The hosted server is being built first for account-scoped conversation durability, synchronization, durable events, and eventually hosted turn orchestration. Remote device agents, graphical VMs, and remote tool execution are intentionally later work.

The core domain model must remain canonical: conversations are message trees; the selected message head determines the flattened model-context path; sessions are durable branch-execution records. Do not invent a separate cloud-chat model merely because PostgreSQL is used.

The eventual responsibility split is:

```text
Browser -> hosted Windie server -> account-owned PostgreSQL state
                                 -> later: orchestration and model gateway
                                 -> later: dispatch to registered local devices

Registered local device -> executes local tools only after an explicit device protocol exists
```

The hosted server must not directly execute a user's local filesystem, browser, MCP tools, or other local-machine actions.

## Current hosted server state

The initial hosted proof was implemented and deployed. It has:

- Supabase bearer-token authentication and account mapping.
- Account-scoped PostgreSQL conversations, message trees, parts, revisions, idempotency records, and durable account events.
- Hosted routes for health, conversation list/create/load, append/update/delete message, truncate, fork, and replayable SSE events.
- Revision and idempotency behavior required for cross-browser convergence.
- A private PostgreSQL deployment behind a public Cloudflare hostname.
- CORS restricted to the hosted app origin.

The live proof still needs its full human verification: sign in, create a conversation in one browser, observe it in a second browser for the same account, confirm another account cannot access it, then restart `windie-server` and verify the same state remains after reconnecting.

Do not record credentials, database URLs, private keys, server IP addresses, Supabase secrets, or Cloudflare tunnel credentials in this repository or in future checkpoints.

## Operation-layer refactor completed in the latest turn

The prior hosted implementation had its HTTP handlers in `src/hosted/api.rs` call `HostedStore` mutations directly. That made the API route layer own persistence orchestration.

The refactor added `src/operation/hosted_conversation.rs` and changed hosted handlers into thin adapters:

```text
HTTP handler
  -> authenticate account
  -> validate request and build a typed hosted command
  -> HostedConversationOperations
  -> HostedStore / PostgreSQL transaction
  -> HTTP or SSE response
```

`HostedConversationOperations` owns hosted conversation workflows for list, load, create, append, update, delete, truncate, fork, and durable-event reads. Its typed commands make hosted mutation inputs explicit. It also owns the compatibility rule that turns a legacy plain `text` message body into a text part when explicit parts were not supplied.

`src/hosted/events.rs` now reads events through `HostedConversationOperations`; `src/hosted/api.rs` should only reach into `state.store` for account resolution in authentication middleware. New hosted behavior should preserve that boundary.

`docs/index/Backend.md` was updated to point engineers to the hosted operation module.

### Important limitation of this refactor

`hosted_conversation.rs` is not yet a shared SQLite/PostgreSQL abstraction. It is a hosted operation layer that still calls `HostedStore`. The existing local operations in `src/operation/` mostly take the SQLite `Store` directly. Therefore, local and hosted implementations currently share domain meaning, but not the same storage-independent command implementation.

This was intentional as a small first extraction. Do not create one enormous generic database trait spanning all Windie features. When there is a concrete next feature that both local and hosted paths must share, extract only that narrow operation policy or repository capability, then provide a SQLite and PostgreSQL implementation. Before doing so, inspect the relevant current local route, operation, store, and tests.

The local route files in `src/api/` are not safe to reuse unchanged: they depend on local `ApiState`, local pairing, SQLite `Store`, `SessionManager`, and local runtime assumptions. Reuse their validation, domain semantics, response contracts, and operation rules where appropriate, not their entire handler wiring.

## Inspector decision

Peter does not want to build the full official chat UI yet. The existing Inspector should become the temporary proof UI for hosted conversations so the hosted server can be verified before official UI implementation.

This does not mean pointing the untouched existing Inspector at the hosted API. Its normal `WindieProvider` starts local-only requests for models, Bifrost/gateway configuration, tools, sessions, plugins, providers, approvals, pairing, and local runtime routes. Those routes and assumptions do not exist on the hosted server yet.

The correct temporary implementation is:

```text
Existing Inspector layout and reusable presentation components
  + hosted Inspector provider/API adapter
  + hosted conversation list, message-tree/transcript, composer, and SSE updates
  - local-only controls and data fetches until their hosted backend exists
```

Build a hosted provider or adapter that presents only the supported hosted capabilities to existing Inspector components. Keep local Inspector behavior unchanged. Do not replace the official UI with Inspector permanently; Inspector is a technical verification surface, while `docs/official-design/README.md` remains the visual product direction.

There is currently a minimal hosted conversation client/page deployed at `app.windieos.com`. It proved Google login and hosted PostgreSQL conversation access, but it is only a temporary bridge. The next frontend direction is to fold the hosted conversation proof into the existing Inspector rather than grow that separate page.

## Authentication and branding state

Google login works through the Supabase project, and the browser has successfully reached the hosted conversation UI after sign-in.

The Google account chooser previously displayed the Supabase project hostname instead of `Windie`. This is not caused by the hosted Rust server. Google Cloud branding is configured with the Windie name, logo, homepage, privacy-policy URL, terms URL, and authorized domains, but Google reported that ownership of `windieos.com` had not been verified.

The next manual branding work is:

1. Complete domain ownership verification in Google Search Console through Cloudflare.
2. Return to Google Auth Platform branding, select the option that the issue was fixed, and request re-verification.
3. Wait for Google to publish the verified branding.

Keep the Supabase OAuth callback domain and redirect configuration intact. Supabase is the identity broker; the desired user-facing brand is Windie once Google accepts the verified branding.

## Deployment and verification notes

The hosted server is running with PostgreSQL and a Cloudflare tunnel on the resized production Droplet. The deployment uses a separate server account/service and a protected environment file. The public health endpoint returned success, while an unauthenticated hosted conversation request returned `401`, as intended.

The latest refactor was built and deployed after correcting the remote noninteractive Cargo path. The service was confirmed active and health responded after startup. PostgreSQL data was not reset during this deploy/restart.

Local verification after the refactor passed:

```text
cargo fmt --check
cargo test hosted --lib
cargo test operation --lib
git diff --check
```

The PostgreSQL live acceptance test remains ignored because it needs an explicitly supplied isolated PostgreSQL test database. That is separate from the manual live proof above.

## Worktree cautions

The root worktree and Inspector submodule have unrelated or in-progress changes. Preserve them. Do not reset, clean, broadly stage, or overwrite existing work. The Inspector is a Git submodule, so inspect both root and submodule status before a future commit.

The newly added hosted operation file, related API/event/module changes, backend index note, hosted docs/plans/memory, and Inspector hosted-client changes may all still be uncommitted. Verify exact status before making any commit. Do not push unless Peter explicitly asks.

## Practical next step

Before implementing another change, inspect the exact existing Inspector provider/component path and the local API/operation/store routes it uses. Then add the smallest hosted Inspector adapter that lets the existing Inspector render and mutate only the hosted conversation capabilities already supplied by `windie-server`. Use the proof to test two-browser SSE convergence and account isolation, not to prematurely add hosted tools, model execution, devices, or the final official chat UI.
