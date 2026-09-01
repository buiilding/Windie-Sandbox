# macOS development

This guide installs Windie's native prerequisites on a new Mac. After the
toolchains are ready, continue with the [shared development workflow](README.md)
to clone, build, run, and verify Windie.

## Supported Mac hardware

The repository builds native releases for both Apple Silicon (`arm64`) and
Intel (`x86_64`) Macs. Run the following command if you do not know which Mac
you have:

```bash
uname -m
```

Official Go and Node.js downloads must match that architecture. Rustup selects
the native Rust target automatically.

## 1. Install Apple's command-line tools

Open Terminal and run:

```bash
xcode-select --install
```

Accept the macOS installation dialog, wait for it to finish, and verify the
active developer directory:

```bash
xcode-select -p
clang --version
git --version
```

`xcode-select -p` should print `/Library/Developer/CommandLineTools` unless the
full Xcode application is selected. The full Xcode application is not required
for the normal Windie development flow. The command-line tools provide Git,
Clang, the macOS SDK, and the native linker used by Rust and Go builds.

See [Apple's command-line tools documentation][apple-command-line-tools].

## 2. Install stable Rust

Install Rust through Rustup, the installer recommended by the Rust project:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Choose the default installation when prompted. Close and reopen Terminal so
the updated command path is loaded, then install the same components used by
Windie's continuous-integration checks:

```bash
rustup toolchain install stable
rustup default stable
rustup component add rustfmt clippy
rustc --version
cargo --version
```

Windie uses Rust edition 2024 and does not yet pin a repository-specific
compiler, so the current stable toolchain is the source of truth.

See [the official Rust installation guide][rust-install].

## 3. Install Go 1.26.5

Windie's release workflow currently builds Bifrost with Go 1.26.5. Install the
matching macOS package from [the official Go downloads page][go-downloads],
then open a new Terminal and verify it:

```bash
go version
```

The output should report Go 1.26.5 and either `darwin/arm64` or
`darwin/amd64`. The Bifrost modules require Go 1.26.4 or newer; using the exact
continuous-integration version avoids local-versus-release differences.

## 4. Install Node.js 22

Windie's frontend checks run on Node.js 22. Install a Node.js 22 macOS package
from [the official Node.js 22 downloads page][node-22-downloads]. Open a new
Terminal and verify both Node.js and npm:

```bash
node --version
npm --version
```

The Node.js version should begin with `v22.`.

## 5. Continue with shared setup

The Mac is ready when all toolchain checks above succeed. Continue with
[cloning Windie and running the shared development workflow](README.md#1-clone-windie-and-its-submodules).

## macOS desktop behavior

The shared guide explains how to start the optional tray and notifier. On
macOS, the system may request notification permission the first time the
notifier presents a notification. The unbundled development notifier can
display notifications, but notification click handling requires the packaged
`Windie Notifier.app`.

## macOS port diagnostics

Check for an existing listener when a component cannot bind its address:

```bash
lsof -nP -iTCP:8080 -sTCP:LISTEN
lsof -nP -iTCP:8787 -sTCP:LISTEN
lsof -nP -iTCP:3000 -sTCP:LISTEN
```

Do not start a development component over an installed copy already using the
same port. Separate Windie checkouts can use different runtime ports:

```bash
export WINDIE_GATEWAY_PORT=18080
export WINDIE_API_PORT=18787
```

Every Terminal running that checkout must receive the same values. When the API
port changes, start the Inspector with its API URL override in the same
terminal:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

The locally served Inspector origin remains `http://localhost:3000` unless the
frontend development server selects another port.

## macOS troubleshooting

### `xcrun: error: invalid active developer path`

Run `xcode-select --install`, finish the macOS installer, and open a new
Terminal. If the tools are installed but macOS selected the wrong developer
directory, inspect it with `xcode-select -p` before changing it.

### `cargo`, `go`, or `node` is not found

Open a new Terminal after installing the tool. Then repeat the version checks
from the corresponding installation section. Do not continue until the command
is available in a normal new shell.

Repository, submodule, Inspector, API, and Bifrost failures belong in the
[shared troubleshooting section](README.md#common-troubleshooting).

[apple-command-line-tools]: https://developer.apple.com/documentation/xcode/installing-the-command-line-tools
[go-downloads]: https://go.dev/dl/
[node-22-downloads]: https://nodejs.org/en/download/archive/v22
[rust-install]: https://rust-lang.org/tools/install/
