# Register a computer with Windie

Implementation checkpoint: September 19, 2026. Enrollment and presence are
implemented in the working tree, not deployed. This does **not** enable tools,
plugin installation, remote desktop, or model execution on your computer.

## Operator setup (deployment requires explicit authorization)

1. Build/test the updated `windie-server` and existing `windie` executable.
   The server applies additive migration `0003_devices` at startup. Back up the
   production database before the authorized rollout; do not run acceptance
   tests against production.
2. Configure `WINDIE_DEVICE_ENROLLMENT_KEY` in the server's protected environment
   file: 32 random bytes encoded as 64 lowercase hex characters. Generate it
   with a secure OS random source, keep it private, and retain it across server
   restarts. Do not put it in the browser build or agent. Missing key means all
   new device routes return 503; existing hosted chat routes are unchanged.
   Replacing the key expires pending pairings, not registered device credentials.
3. For the existing loopback Cloudflare Tunnel connector, enable
   `WINDIE_DEVICE_TRUST_LOOPBACK_PROXY=1` only after verifying the listener is
   private and the trusted ingress supplies/overwrites `CF-Connecting-IP`.
   Otherwise the server uses the actual TCP peer, never arbitrary forwarded
   headers. Without trusted ingress configuration, tunnel users share one
   source-rate bucket. Never expose the loopback listener through an untrusted
   proxy that passes client-supplied identity headers unchanged.
4. Deploy the server first, then the official UI. Existing Supabase settings,
   allowed browser origin and Google login are reused. Device pages preserve
   only `/devices/connect` and `/computers` across OAuth using same-tab session
   storage; no arbitrary return URL is accepted. If storage is disabled, sign
   in and reopen the pairing page manually.
5. Rollback can remove the enrollment key to disable device routes and restore
   the previous UI/binary. Keep the additive tables and registered credentials;
   do not drop device data as a rollback step. While disabled, agents see 503
   and back off; presence expires naturally.

The CLI release remains the existing `windie` binary; no separate agent package,
background daemon, or autostart installer is introduced. Credential safety for
this release is Unix/macOS-first; Windows enrollment is refused until an
ACL/vault adapter is implemented and tested.

## Pair and run (after deployment)

```sh
windie agent connect
```

Open the printed `https://app.windieos.com/devices/connect`, sign in, and enter
the code. Preview the computer and explicitly approve **registration only**.
Only approve a pairing you just initiated yourself. Compare the verified
account ID on the page with the one printed in the terminal; type `yes` locally
to activate the registration. The device name/OS are self-reported, not hardware
attestation. The account ID is Supabase's verified subject, not a label supplied
by the computer.

```sh
windie agent run
windie agent status
```

`run` stays in the foreground. Open `/computers` to see its online state and last
contact. `status` is read-only and does not renew presence. No local API,
SQLite conversation store, Bifrost, MCP process, or tool registry is started.

Heartbeats run every 20–23 seconds, one request at a time, with a 10-second
request timeout. The server considers a lease offline after 90 seconds without
renewal. Ctrl-C makes a best-effort release; kill/sleep/network loss instead
wait for lease expiry. Network/5xx retries use jittered exponential backoff up
to 25 seconds, with server `Retry-After` taking precedence. Invalid credentials
stop the loop. A second live agent instance is rejected, not allowed to steal
the lease; after a crash, wait for the old lease to expire and rerun.

In `/computers`, choose **Revoke access**, then **Confirm revoke**. The next
heartbeat is rejected and the agent stops. Existing transcript state is not
affected. To change accounts, revoke first and explicitly rerun `connect`.

## Local state and recovery

Credentials are saved under `~/.windie/agent/` (0700), in atomically replaced
0600 files. The OS lock prevents concurrent `connect`/`run` processes. Symlinks,
unsafe ownership/permissions, and multiply-linked files are refused. Same-OS-user
malware can still read these files; this is not a credential vault.

Both secrets are persisted before initiation. Pending state and active state
are distinguished by a device ID in the atomic record; an active pairing is
never implicitly replaced. If initialization/finalization loses its response,
rerun `connect`: it checks the saved device credential first, then resumes the
same request. Expired/declined/cancelled pending records are archived; a revoked
active record is archived only after explicit confirmation. Archives remain
private credential files and must not be shared or committed. Deleting a local
file does not revoke the registration on the server.

For loopback development only, set `WINDIE_AGENT_SERVER=http://127.0.0.1:<port>`.
Credentials are bound to that exact origin; changing the origin does not send
an existing secret to a different server. Redirects are not followed. The CLI
always prints the trusted production pairing URL; when developing, manually
open `/devices/connect` on the local official UI wired to the same test server.
Do not switch an already paired production profile to a development origin.

## Tests and manual proof

```sh
cargo test --lib
# Set WINDIE_HOSTED_TEST_DATABASE_URL privately to the isolated windie_test DB.
cargo test --lib postgres_device -- --ignored --nocapture
```

PostgreSQL tests refuse any database name other than `windie_test`, use fresh
`device_test_<uuid>` schemas, and remove only those test-owned schemas on success.
They do not restart production or change its schema. If a test crashes, inspect
its unique schema before explicit cleanup; never reset the whole test database.

In `vendor/windie-UI-official`, run `npm test`, `npm run build`, and focused lint
for the changed files. Existing repository-wide lint failures remain separate.

After an authorized rollout, manually verify each item in the
[implementation plan](../plans/device-agent-enrollment-and-connectivity.md#required-tests-and-live-acceptance):
pair through Google, confirm terminal identity, observe online/offline, test a
second account, kill/restart and network recovery, restart the server, and revoke
while running. Automated protocol tests are not a substitute for these live
proofs. No browser automation was used for this implementation.
