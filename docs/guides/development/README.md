# Build Windie from source

Building Windie from source lets you make direct changes to Windie and test
those changes locally. This guide explains the shared workflow after the
platform-specific prerequisites are installed.

## 1. Install platform prerequisites

Based on your operating system, complete the appropriate guide before
continuing:

- [macOS](MACOS.md)
- [Windows](WINDOWS.md)
- [Linux](LINUX.md)

Each platform guide explains how to install Git, the native build tools, Rust,
Go, and Node.js required by the source components.

Windie records the toolchain versions used by local development and CI:

| Tool | Required version | Source of truth |
| --- | --- | --- |
| Rust | `1.98.0` | [`rust-toolchain.toml`](../../../rust-toolchain.toml) |
| Go | `1.26.5` | [`.go-version`](../../../.go-version) |
| Node.js | `22.23.2` | [`vendor/windie-inspector/frontend/.nvmrc`](../../../vendor/windie-inspector/frontend/.nvmrc) |

Use the versions documented by the platform guide rather than an unpinned
latest release.

## 2. Clone Windie

This repository contains Windie's Rust source code. The UI, LLM gateway, and
other source components are checked out as Git submodules under `vendor/`.
Clone recursively so all of the source needed for development is available:

```bash
git clone --recurse-submodules https://github.com/buiilding/Windie-Sandbox.git
cd Windie-Sandbox
```

Confirm that every submodule is initialized:

```bash
git submodule status --recursive
```

A leading `-` means a submodule is not initialized. A leading `+` means its
checkout does not match the commit recorded by the Windie repository. Repair a
non-recursive clone with:

```bash
git submodule update --init --recursive
```

Run the remaining commands from the repository root unless a step says
otherwise.

## 3. Terminal 1: The LLM gateway

The gateway is Bifrost, the local service Windie uses to communicate with LLM
providers. Windie sends it the selected model context, model, reasoning mode,
and tool schemas. Bifrost routes the request to the configured provider and
streams typed output back, such as reasoning, normal response text, and tool
calls. It also provides model and token-counting operations used by Windie.

Start it in the first terminal:

```bash
cargo run --bin windie -- dev run gateway
```

Leave this terminal running. Press `Control-C` to stop the gateway.

## 4. Terminal 2: The Windie API server

The API server is Windie's main local runtime process. It receives requests
from the Inspector, owns conversations and durable sessions, resolves the
selected conversation head, builds model context, applies tool-approval rules,
executes approved tools, and stores runtime state in SQLite. It sends model
requests to the gateway and streams durable session events back to clients.

Start it in a second terminal:

```bash
cargo run --bin windie -- dev run api
```

By default, the API listens on `http://127.0.0.1:8787`. Leave this terminal
running. Press `Control-C` to stop the API.

## 5. Terminal 3: The Windie Inspector

The Inspector is Windie's browser-based user interface. It displays runtime
state and sends user actions to the local API. It does not read SQLite, call
the gateway directly, execute tools, or decide what the model sees. In
development, it runs as a React development server with hot reload.

Start it in a third terminal:

```bash
cargo run --bin windie -- dev run inspector
```

The command prepares the Inspector development environment, waits for the
frontend to become ready, and opens a local browser session. Leave this
terminal running while you work. Press `Control-C` to stop the Inspector.

## 6. Open the local Inspector

The development Inspector uses port `3000` by default:

[Open the local Inspector](http://localhost:3000)

The Inspector browser client talks to the API on port `8787`; the API talks to
the Bifrost gateway on port `8080`.

## 7. Verify the running system

From a fourth terminal, check the gateway, API, and Windie component status:

```bash
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8787/api/health
cargo run --bin windie -- status
```

The health requests should succeed, and `windie status` should report the
gateway and API as running. If a check fails, read the corresponding process
terminal before continuing.

## 8. Configure a model provider

In the Inspector, open the model-provider section and configure a provider with
its API key. The key is stored in Windie's local configuration; never commit
provider credentials to the repository.

After configuring a provider, select one of its models in the Inspector and
send a test message. A successful response confirms that the API can reach the
gateway and that the gateway can reach the selected provider.

You can also configure providers through the command-line onboarding flow:

```bash
cargo run --bin windie -- onboard
```

## 9. Configure plugins

Plugins are Windie's extension and distribution unit. A plugin can provide MCP
components and other presentation or capability metadata. An enabled MCP
component can expose tools that a model may use on the operating system, subject
to Windie's approval and permission rules.

In the Inspector, open the **Plugins** section, choose a plugin, and follow its
installation and setup instructions. Configure only the credentials the plugin
declares, and grant tool approval deliberately.

For package authors, see the [plugin package guide](../plugin-packages.md).

## 10. Optional operating-system components

The tray and notifier are independent optional processes. They are not required
to run the gateway, API, or Inspector, and each should run in its own terminal:

```bash
cargo run --bin windie -- dev run tray
cargo run --bin windie -- dev run notifier
```

The tray presents local component status and start/stop controls. It is
currently supported on macOS and Windows. The notifier observes completed
session events and presents native desktop notifications; it does not run
models, execute tools, or modify sessions. Linux requires an active graphical
notification service for the notifier.

See the [desktop notifications guide](../desktop-notifications.md) for the
notifier flow and development probe.

## 11. Default process ports

| Process | Default address |
| --- | --- |
| Bifrost LLM gateway | `http://127.0.0.1:8080` |
| Windie API server | `http://127.0.0.1:8787` |
| Windie Inspector | `http://localhost:3000` |

If you run more than one Windie checkout at the same time, assign different
gateway and API ports with `WINDIE_GATEWAY_PORT` and `WINDIE_API_PORT`. Apply
the same values in every terminal for that checkout, and start the Inspector
with the matching API URL:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

Use the shell syntax shown in your platform guide when setting these variables.

## 12. Verify the checkout

Before opening a pull request, run the repository checks from the root:

```bash
scripts/check-release-notes.sh
scripts/check-docs.sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
npm run build --prefix vendor/windie-inspector/frontend
```

These checks validate release-note and documentation structure, Rust
formatting, Rust tests, lint rules, and the Inspector production build. The
first Rust and frontend builds can take several minutes. They do not require a
model-provider credential or existing runtime state.

## 13. Common troubleshooting

### A submodule is missing

Run:

```bash
git submodule update --init --recursive
```

Then retry the component command.

### A port is already in use

Use the port-diagnostic command in your platform guide to identify the
process. Stop only a stale Windie process, or assign distinct ports as
described above.

### The Inspector cannot reach the API

Confirm that the API health request succeeds and that the Inspector's API URL
matches the API port. Start the API before starting the Inspector, then restart
the Inspector so it can create a new local browser session.

### The gateway starts but model requests fail

The gateway may be healthy while the provider credential, selected model, or
provider configuration is invalid. Recheck the provider setup in the Inspector
and verify the model name.

### A component exits immediately

Read the error in that component's terminal. The usual causes are an
uninitialized submodule, a missing platform toolchain, an occupied port, or an
environment variable that does not match the other terminals.

## 14. Related code and documentation

- [macOS development](MACOS.md), [Windows development](WINDOWS.md), and
  [Linux development](LINUX.md) contain platform-specific setup and diagnostics.
- [`Backend.md`](../../index/Backend.md) maps the Rust runtime source.
- [`Frontend.md`](../../index/Frontend.md) maps the Inspector source.
- [Local components](../../architecture/local-components/README.md) explains
  the API, gateway, tray, and notifier boundaries.
- [Architecture overview](../../architecture/overview.md) explains Windie's
  runtime design.
- [`src/dev.rs`](../../../src/dev.rs) implements the foreground development
  commands.
- [`CONTRIBUTING.md`](../../../CONTRIBUTING.md) explains the contribution and
  pull-request workflow.
