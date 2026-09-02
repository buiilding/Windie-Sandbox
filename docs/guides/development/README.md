# Develop Windie from source

This is the one workflow for a fresh Windie checkout: install your platform's
native prerequisites, clone the recursive repository, verify it, then run the
gateway, API, and Inspector as separate foreground processes.

## 1. Install platform prerequisites

Choose the guide for your development machine, complete its native-toolchain
steps, then return here:

- [macOS](MACOS.md)
- [Windows](WINDOWS.md)
- [Linux](LINUX.md)

Every platform needs Git, a native C toolchain, Rust, Go, and Node.js with npm.
The repository records the exact versions that local development and CI use:

| Tool | Current version | Source of truth | Responsibility |
| --- | --- | --- | --- |
| Rust | 1.98.0 | [`rust-toolchain.toml`](../../../rust-toolchain.toml) | Builds Windie's CLI and local runtime. |
| Go | 1.26.5 | [`.go-version`](../../../.go-version) | Builds the checked-out Bifrost gateway. |
| Node.js | 22.23.2 | [`vendor/windie-inspector/frontend/.nvmrc`](../../../vendor/windie-inspector/frontend/.nvmrc) | Builds and serves the Windie Inspector. |

Do not replace these with a floating “latest” version. Update the declarations
only after Windie and its CI have been verified with the new toolchain.

## 2. Clone Windie and its submodules

Choose a source-checkout directory, then clone recursively:

```bash
git clone --recurse-submodules https://github.com/buiilding/Windie-Sandbox.git
cd Windie-Sandbox
```

Windie develops against checked-out source under `vendor/`, especially Bifrost
and the Inspector. Confirm every submodule is initialized:

```bash
git submodule status --recursive
```

A leading `-` means a submodule is uninitialized. A leading `+` means its
checkout differs from the commit recorded by Windie. Repair a non-recursive
clone with:

```bash
git submodule update --init --recursive
```

Run the remaining commands from the repository root unless a step says
otherwise.

## 3. Install Inspector dependencies

Use the Node.js version recorded by the Inspector, then install its locked
dependency graph:

```bash
npm ci --legacy-peer-deps --prefix vendor/windie-inspector/frontend
```

Ensure `node --version` matches the Inspector's `.nvmrc` before running this
command. The `--legacy-peer-deps` flag matches CI.

Windie's development gateway does not require Bifrost's dashboard or its
separate frontend toolchain. Before compiling Bifrost, `windie dev run gateway`
creates an ignored embed placeholder when necessary. Release packaging builds
the full Bifrost dashboard separately.

The local Inspector does not need a hosted account or `frontend/.env.local`.
Windie opens it through a short-lived local browser session; the hosted
Inspector at `app.windieos.com` is a separate deployment.

## 4. Verify the checkout

Run the checks required before a pull request:

```bash
scripts/check-release-notes.sh
scripts/check-docs.sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
npm run build --prefix vendor/windie-inspector/frontend
```

The first Rust and frontend build can take several minutes. These checks do not
need model-provider credentials or existing local runtime state.

## 5. Run the core development components

Open three terminals at the repository root. Each command owns one foreground
process; Windie deliberately has no aggregate development runner.

### Terminal 1: Bifrost gateway

```bash
cargo run --bin windie -- dev run gateway
```

This command prepares Bifrost's local Go workspace, creates its development
embed placeholder if needed, builds the checked-out gateway source, starts it
on loopback port `8080`, and waits for health.

### Terminal 2: Windie API

```bash
cargo run --bin windie -- dev run api
```

The API owns runtime execution and durable local state at
`http://127.0.0.1:8787` by default. Wait for:

```text
windie api listening on http://127.0.0.1:8787
```

### Terminal 3: Inspector

```bash
cargo run --bin windie -- dev run inspector
```

The Inspector is a React development server with hot reload. Once it is ready,
Windie opens [http://localhost:3000][local-inspector] with a one-time local
launch code. The browser talks to the API on port `8787`; the API talks to
Bifrost on port `8080`.

## 6. Verify the running system

From a fourth terminal, check both health endpoints and the typed component
status:

```bash
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8787/api/health
cargo run --bin windie -- status
```

The status command reports the gateway and API. The Inspector is a development
web client, not a managed runtime process.

## 7. Configure a model provider

Keep the gateway running, then launch onboarding:

```bash
cargo run --bin windie -- onboard
```

Choose an LLM provider and enter its API key at the hidden prompt. Press Enter
at the extension prompt to skip optional MCP extensions. Do not commit provider
keys; Windie stores only approved local configuration.

Confirm that Bifrost exposes a model:

```bash
cargo run --bin windie -- models
```

Supported providers can also be configured through the Inspector.

## Optional desktop components

The tray and notifier are independent optional processes on supported
platforms:

```bash
cargo run --bin windie -- dev run tray
cargo run --bin windie -- dev run notifier
```

Run each selected component in its own terminal. Platform-specific desktop
behavior belongs in the platform guide; notification behavior belongs in the
[desktop notifications guide](../desktop-notifications.md).

## Stopping development

Press `Control-C` in each component terminal. Stopping one component does not
stop the others.

Development uses Windie's normal user-local data directory. It contains SQLite
state, Bifrost data, component credentials, logs, and Windie's private
environment file. Do not delete it merely to stop development.

| Platform | Default data directory |
| --- | --- |
| macOS and Linux | `~/.windie` |
| Windows | `%USERPROFILE%\.windie` |

## Ports and parallel checkouts

| Component | Address |
| --- | --- |
| Bifrost gateway | `http://127.0.0.1:8080` |
| Windie API | `http://127.0.0.1:8787` |
| Inspector | `http://localhost:3000` |

Do not start a development component over another local Windie copy using the
same port. Separate checkouts can set `WINDIE_GATEWAY_PORT` and
`WINDIE_API_PORT` to distinct values. Every terminal for that checkout must
receive the same values.

When the API port changes, start the Inspector with its matching API URL:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

The platform guides provide native shell syntax and port-diagnostic commands.

## Common troubleshooting

### `Bifrost source is missing`

Initialize the Bifrost submodule:

```bash
git submodule update --init vendor/bifrost
```

### Bifrost reports an unsupported Go version

Run `go version`. Install the exact version in `.go-version`.

### Inspector reports missing packages

Recreate its locked installation:

```bash
npm ci --legacy-peer-deps --prefix vendor/windie-inspector/frontend
```

### Inspector loads but cannot reach Windie

Confirm the API health request succeeds and the Inspector API URL matches the
API port. Restart `dev run inspector` so Windie can mint a new one-time launch
code. The local Inspector still requires an API-issued browser token.

### A component exits immediately

Read that component's foreground-terminal error first. Common causes are an
uninitialized submodule, missing toolchain, port conflict, or API URL mismatch.

## Related material

- [Documentation index](../../README.md)
- [`Backend.md`](../../index/Backend.md) and
  [`Frontend.md`](../../index/Frontend.md) map the runtime and Inspector
  source.
- [`CONTRIBUTING.md`](../../../CONTRIBUTING.md) defines the contribution and
  pull-request workflow.
- [`src/dev.rs`](../../../src/dev.rs) owns foreground development commands.

[local-inspector]: http://localhost:3000
