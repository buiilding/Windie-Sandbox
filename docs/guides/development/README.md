# Windie development

This guide owns the operating-system-neutral workflow for developing Windie
from source. Start with the platform guide for installing native prerequisites,
then return here to clone, build, run, and verify the repository.

## Platform setup

- [macOS](MACOS.md)
- [Windows](WINDOWS.md)
- [Linux](LINUX.md)

Every platform needs:

- Git and a native C toolchain.
- Rust 1.98.0 with `rustfmt` and Clippy.
- Go 1.26.5.
- Node.js 22.23.2 with npm.

The repository records these versions in files that tools and CI can read:

| Tool | Source of truth | What uses it |
| --- | --- | --- |
| Rust | [`rust-toolchain.toml`](../../../rust-toolchain.toml) | Rustup and Cargo select it automatically inside the checkout. |
| Go | [`.go-version`](../../../.go-version) | Release CI installs this version. Install the same version locally. |
| Node.js | [`vendor/windie-inspector/frontend/.nvmrc`](../../../vendor/windie-inspector/frontend/.nvmrc) | `nvm use` and frontend/release CI. |

The Inspector's `package.json` also declares Node.js 22.23.2 as its supported
runtime. Do not substitute a floating “latest,” “stable,” or merely-major
version; update these declarations deliberately when Windie has been verified
with new toolchains.

Windie development uses those toolchains across three source boundaries:

- Rust builds the `windie` CLI and local API runtime.
- Go builds the Bifrost model gateway from `vendor/bifrost`.
- Node.js builds and serves the Inspector from
  `vendor/windie-inspector/frontend`.

The gateway, API, Inspector, tray, and notifier remain independent processes.
There is intentionally no command that starts every component together.

## 1. Clone Windie and its submodules

Choose a directory for source checkouts, then clone the repository recursively:

```bash
git clone --recurse-submodules https://github.com/buiilding/Windie-Sandbox.git
cd Windie-Sandbox
```

Windie depends on checked-out source beneath `vendor/`, especially Bifrost and
the Inspector. Verify that every submodule is initialized:

```bash
git submodule status --recursive
```

Each healthy line contains a leading space followed by a commit hash. A line
beginning with `-` identifies an uninitialized submodule; a line beginning with
`+` means the checkout differs from the commit recorded by Windie. Repair an
existing non-recursive clone with:

```bash
git submodule update --init --recursive
```

Run all remaining commands from the repository root unless a step says
otherwise.

## 2. Install Inspector dependencies

Install the frontend dependency versions recorded in `package-lock.json`:

```bash
cd vendor/windie-inspector/frontend
npm ci --legacy-peer-deps
cd ../../..
```

Use `npm ci`, not `npm install`, for initial setup. The `--legacy-peer-deps`
flag matches the repository's frontend continuous-integration job.

If you use `nvm`, select the pinned frontend Node version before installing:

```bash
(
  cd vendor/windie-inspector/frontend
  nvm use
)
```

The locally served Inspector does not need a hosted account or a
`frontend/.env.local` file. Windie creates a short-lived local browser session
when it opens the Inspector. The hosted deployment at `app.windieos.com` is a
separate path and retains its Supabase configuration in its deployment
environment.

## 3. Verify the checkout before starting services

Run the core pull-request checks. Cargo automatically selects Rust 1.98.0 from
the repository's `rust-toolchain.toml`:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Then verify the Inspector production build:

```bash
cd vendor/windie-inspector/frontend
npm run build
cd ../../..
```

The first build downloads and compiles dependencies and can take several
minutes. These checks prove the checkout builds before provider credentials or
local runtime state are involved.

## 4. Run the core development components

Open three terminals at the repository root. Each `dev run` command owns
exactly one foreground process and keeps its output visible.

### Terminal 1: Bifrost gateway

```bash
cargo run --bin windie -- dev run gateway
```

This command creates Bifrost's local Go workspace, builds the checked-out
Bifrost source, starts it on loopback port `8080`, and waits for its health
endpoint. Leave the process running.

### Terminal 2: Windie API

```bash
cargo run --bin windie -- dev run api
```

The API owns runtime execution and durable local state. Its default address is
`http://127.0.0.1:8787`. Wait for:

```text
windie api listening on http://127.0.0.1:8787
```

### Terminal 3: Inspector

```bash
cargo run --bin windie -- dev run inspector
```

The Inspector is a React development server with hot reload. Once it is ready,
Windie opens [http://localhost:3000][local-inspector] with a one-time local
launch code. No Google sign-in or hosted pairing is required. The browser is
only a client: it talks directly to the API on port `8787`, while the API talks
to Bifrost on port `8080`.

## 5. Verify the running system

From another terminal at the repository root, check both health endpoints and
the typed component status:

```bash
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8787/api/health
cargo run --bin windie -- status
```

The status command should report the gateway and API as running. The Inspector
does not appear in `windie status` because it is a development web client, not
a managed runtime process.

No model-provider API key is required to build Windie or verify these health
checks. A provider is required before Windie can complete a model request.

## 6. Configure a model provider

Keep the development gateway running, then launch terminal onboarding:

```bash
cargo run --bin windie -- onboard
```

Choose an LLM provider and enter its API key at the hidden prompt. Press Enter
at the extension selection prompt to skip optional MCP extensions. The
onboarding workflow submits model-provider keys to Bifrost and stores only
manifest-approved MCP environment values under Windie's local data directory.
Do not commit provider keys to the repository.

Confirm that Bifrost exposes at least one model:

```bash
cargo run --bin windie -- models
```

Supported providers can also be configured through the Inspector.

## Optional desktop components

The core development flow needs the gateway, API, and Inspector. The tray and
notifier are independent and optional on platforms that support them.

```bash
cargo run --bin windie -- dev run tray
cargo run --bin windie -- dev run notifier
```

Run each selected component in its own terminal. The notifier requires the API
as its event source but is not owned by the API or tray. Read the platform guide
for native desktop behavior and permission requirements.

## Stopping development

Press `Control-C` in each component terminal. Stopping one component does not
stop the others.

Development uses Windie's normal user-local `.windie` directory. It contains
SQLite state, Bifrost data, component credentials, logs, and Windie's private
environment file. Do not delete that directory as a routine way to stop
development.

| Platform | Default data directory |
| --- | --- |
| macOS and Linux | `~/.windie` |
| Windows | `%USERPROFILE%\.windie` |

## Ports and parallel checkouts

The default local ports are:

| Component | Address |
| --- | --- |
| Bifrost gateway | `http://127.0.0.1:8080` |
| Windie API | `http://127.0.0.1:8787` |
| Inspector | `http://localhost:3000` |

Do not start a development component over an installed copy already using the
same port. Separate checkouts can set `WINDIE_GATEWAY_PORT` and
`WINDIE_API_PORT` to different values. Every terminal running that checkout
must receive the same values.

When the API port changes, start the Inspector with an API URL override in the
same terminal:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

The platform guides provide the native shell syntax and port-diagnostic
commands.

## Common troubleshooting

### `Bifrost source is missing`

Initialize the Bifrost submodule:

```bash
git submodule update --init vendor/bifrost
```

### Bifrost reports an unsupported Go version

Run `go version`. Windie expects the exact version in `.go-version` (currently
Go 1.26.5).

### Inspector reports missing packages

Recreate its locked dependency installation:

```bash
cd vendor/windie-inspector/frontend
npm ci --legacy-peer-deps
cd ../../..
```

### Inspector loads but cannot reach Windie

Confirm that the API health request succeeds and that the Inspector API URL
matches the API port. Restart `dev run inspector` so Windie can mint a new
one-time local launch code. The local Inspector still requires an API-issued
browser token; it is not an unrestricted loopback page.

### A component exits immediately

Read the error in that component's foreground terminal first. Common causes
are an uninitialized submodule, missing toolchain, port conflict, or an API URL
that does not match the selected development ports.

## Related repository files

- [`Backend.md`](../../../Backend.md) and [`Frontend.md`](../../../Frontend.md)
  map the runtime and Inspector source.
- [`src/dev.rs`](../../../src/dev.rs) builds and supervises foreground
  development components.
- [`src/config.rs`](../../../src/config.rs) owns default gateway and API
  addresses.
- [`.github/workflows/check.yml`](../../../.github/workflows/check.yml) defines
  Rust and Inspector pull-request checks.
- [`.github/workflows/release.yml`](../../../.github/workflows/release.yml)
  defines the release Go version and native targets.
- [`docs/guides/desktop-notifications.md`](../desktop-notifications.md) explains
  notifier behavior.

[local-inspector]: http://localhost:3000
