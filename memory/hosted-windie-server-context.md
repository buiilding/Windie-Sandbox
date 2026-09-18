```md
# Hosted Windie server: situational context

Read `docs/plans/main-hosted-windie-server.md` for the implementation plan.
This document is context and implementation guidance, not a duplicate plan.

## What the user wants

Windie should become a real hosted product where a user can sign in, use the
same Windie account from multiple browsers, and have durable cloud-backed
conversations and later hosted model execution.

The user does not want a separate or reinvented conversation system.

Windie already has the correct foundational model:

- conversations are canonical parent-linked message trees;
- a selected root-to-head path is the model-visible transcript;
- sessions point at tree heads and do not copy conversation history;
- backend-owned head/session resolution prevents stale browser state from
  deciding ownership;
- durable events support reconnecting clients;
- execution claims fence stale session runners.

The hosted server must preserve these rules.

## Naming and boundaries

Call it the **hosted server**, not the “control plane.”

Use clear names such as:

```text
hosted server
HostedApi
HostedStore
HostedAuth
HostedEvents
```

The existing local runtime and new hosted server are different deployment
surfaces:

```text
local `windie api` → loopback API + local SQLite + local runtime
`windie-server`    → hosted API + PostgreSQL + account-owned cloud state
```

Do not replace or casually alter the local API while building the hosted
server. The local runtime remains useful and its established behavior is the
reference implementation for conversation and session semantics.

## Most important implementation rule

Before implementing any hosted-server feature, inspect the equivalent existing
code first.

For every hosted route or persistence behavior:

1. Find the current local API route in `src/api/`.
2. Read its handler and request/response contract.
3. Read the corresponding workflow in `src/operation/`.
4. Read the SQLite behavior in `src/store/`.
5. Read existing tests for tree, session, event, and conflict behavior.
6. Preserve the same domain rule in PostgreSQL unless there is an explicit,
   documented reason to change it.
7. Add hosted-server tests proving the preserved behavior.

Do not create “simpler” hosted semantics that weaken tree ownership, session
claims, durable events, stale-head handling, or approval boundaries merely
because PostgreSQL is new.

The goal is:

```text
same Windie domain model
+ account ownership
+ PostgreSQL durability
+ cross-browser synchronization
```

Not:

```text
new cloud chat app that happens to be named Windie
```

## What is genuinely new

The hosted server must add concepts that the local runtime does not need:

- many authenticated accounts instead of one locally paired account;
- account-scoped authorization on every request and event subscription;
- PostgreSQL persistence for shared account state;
- durable revisions and idempotency keys for browser retries;
- synchronization between separate browser sessions;
- hosted deployment, backups, observability, and server-only secrets.

The browser must never receive a database credential or talk to PostgreSQL
directly. It talks only to the hosted Windie API.

## Supabase context

Supabase is the identity provider, not the user-facing product and not the
authority for Windie conversations.

Expected flow:

```text
user signs in with Google
→ Supabase issues an access token
→ hosted Windie server validates the token
→ server maps the stable Supabase user ID to a Windie account
→ server reads/writes only that account's data
```

The connected Supabase project is named `windie-auth` and is active. Its
actual Google return flow remains a required Phase 6 live-proof check.

The Google sign-in experience must be branded as **Windie**, not Supabase or a
project reference. Configure the Google OAuth consent screen, app name, logo,
authorized domain, privacy policy, terms, support contact, scopes, callback,
and redirect URLs as described in the hosted-server plan.

Use minimal initial scopes: `openid`, `email`, and `profile`. Do not request
Gmail, Drive, Calendar, or file access during basic Windie sign-in.

Windie-managed provider access is the chosen initial model policy. Provider
credentials are server-only secrets; users do not configure or expose their own
provider keys in the first hosted build.

## Synchronization nuance

SSE replay is not fake streaming and must never rerun a model request.

When a browser reconnects, it gives its last accepted durable event ID. The
server reads later events already saved in PostgreSQL, sends them as catch-up,
then stays connected for new events.

The UI should treat historical events as recovery data and render authoritative
saved conversation/session state. It should not animate old model-token deltas
as if the model is generating again.

## Scope discipline

The initial hosted-server work is about:

- accounts;
- durable cloud conversation state;
- cross-browser synchronization;
- then hosted sessions, Bifrost execution, approvals, queues, and wakeups.

Do not pull these later concerns into the initial build:

- device agents;
- registered computers;
- VM provisioning;
- remote control;
- local filesystem/browser execution;
- local MCP execution;
- official UI redesign.

They depend on a correct hosted account, conversation, session, and permission
foundation first.

## Communication preferences

Explain architecture using concrete ownership and request flows before jargon.

Be direct about what is verified versus proposed. A successful local build,
unit test, or health endpoint is not proof that a hosted deployment, Google
sign-in flow, database backup, SSE reconnect, or model response works in
production.

Preserve unrelated working-tree changes and untracked documentation. Do not
overwrite the user’s plan or redesign it while implementing individual tasks.
```
