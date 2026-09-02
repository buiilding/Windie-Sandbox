# macOS development

This guide installs Windie's native prerequisites on a new Mac. After the
toolchains are ready, continue with the shared [development workflow](README.md).

## Supported Mac hardware

Windie builds native releases for Apple Silicon (`arm64`) and Intel (`x86_64`)
Macs. Check the current architecture when selecting Go and Node.js downloads:

```bash
uname -m
```

Rustup selects the native Rust target automatically.

## 1. Install Apple's command-line tools

Open Terminal and run:

```bash
xcode-select --install
```

Accept the installation dialog, then verify Git, Clang, and the active developer
directory:

```bash
xcode-select -p
clang --version
git --version
```

The full Xcode application is not required. The command-line tools provide the
macOS SDK and linker required by Rust and Go builds.

See [Apple's command-line tools documentation][apple-command-line-tools].

## 2. Install Rust 1.98.0

Install Rust through Rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new Terminal, then install the compiler and components recorded in
Windie's `rust-toolchain.toml`:

```bash
rustup toolchain install 1.98.0 --component rustfmt --component clippy
```

Do not change your global Rust default for Windie. After cloning, Cargo selects
the repository version automatically. Verify it from the checkout:

```bash
rustc --version
cargo --version
```

See [the Rust installation guide][rust-install].

## 3. Install Go 1.26.5

Install Go 1.26.5, the version in Windie's
[`.go-version`](../../../.go-version), from the [official Go downloads
page][go-downloads]. Download and run the package that matches the Mac's
architecture.

### Optional: install Go with one command

If you prefer a terminal-only setup, this command detects Apple Silicon or
Intel, downloads the same official package to a temporary directory, asks for
the macOS administrator password, installs it, and exposes Go in the current
Terminal:

```bash
go_arch="$(uname -m)"; [ "$go_arch" = "arm64" ] || [ "$go_arch" = "x86_64" ] || { echo "unsupported Mac architecture: $go_arch" >&2; exit 1; }; [ "$go_arch" = "x86_64" ] && go_arch="amd64"; go_temp_dir="$(mktemp -d)" && trap 'rm -rf "$go_temp_dir"' EXIT && curl -fsSL "https://go.dev/dl/go1.26.5.darwin-${go_arch}.pkg" -o "$go_temp_dir/go.pkg" && sudo installer -pkg "$go_temp_dir/go.pkg" -target / && export PATH="/usr/local/go/bin:$PATH"
```

Open a new Terminal and verify it:

```bash
go version
```

The result should report Go 1.26.5 and either `darwin/arm64` or `darwin/amd64`.

## 4. Install Node.js 22.23.2

Install Node.js 22.23.2, the version recorded in the Inspector's `.nvmrc`,
using the [Node.js 22 downloads page][node-22-downloads]. Download and run
the macOS package.

### Optional: install Node.js with one command

If you prefer a terminal-only setup, this command downloads Node's official
macOS package (for both Apple Silicon and Intel) to a temporary directory,
asks for the macOS administrator password, installs it, and exposes Node's
normal installation directory in the current Terminal:

```bash
node_temp_dir="$(mktemp -d)" && trap 'rm -rf "$node_temp_dir"' EXIT && curl -fsSL "https://nodejs.org/dist/v22.23.2/node-v22.23.2.pkg" -o "$node_temp_dir/node.pkg" && sudo installer -pkg "$node_temp_dir/node.pkg" -target / && export PATH="/usr/local/bin:$PATH"
```

Open a new Terminal and verify Node.js and npm:

```bash
node --version
npm --version
```

If you use `nvm`, select that version after cloning. The shared guide installs
the Inspector's dependencies.

## 5. Continue with the shared workflow

The Mac is ready when the toolchain checks above succeed. Continue with the
[shared development workflow](README.md#2-clone-windie-and-its-submodules).

## macOS desktop behavior

The shared guide explains how to start the optional tray and notifier. macOS
may request notification permission when the notifier first presents a
notification. The unbundled development notifier can display notifications,
but notification click handling requires the packaged `Windie Notifier.app`.

## macOS port diagnostics

Check for an existing listener when a component cannot bind its address:

```bash
lsof -nP -iTCP:8080 -sTCP:LISTEN
lsof -nP -iTCP:8787 -sTCP:LISTEN
lsof -nP -iTCP:3000 -sTCP:LISTEN
```

Separate checkouts can use distinct runtime ports:

```bash
export WINDIE_GATEWAY_PORT=18080
export WINDIE_API_PORT=18787
```

Every Terminal running that checkout must receive the same values. Start the
Inspector with its matching API address:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

## macOS troubleshooting

### `xcrun: error: invalid active developer path`

Run `xcode-select --install`, finish the installer, and open a new Terminal.
If the tools are already installed, inspect `xcode-select -p` before changing
the selected developer directory.

### `cargo`, `go`, or `node` is not found

Open a new Terminal after installing the tool, then repeat the corresponding
version check. Do not continue until the command is available in a normal new
shell.

Repository, submodule, Inspector, API, and Bifrost failures belong in the
[shared troubleshooting section](README.md#common-troubleshooting).

[apple-command-line-tools]: https://developer.apple.com/documentation/xcode/installing-the-command-line-tools
[go-downloads]: https://go.dev/dl/
[node-22-downloads]: https://nodejs.org/en/download/archive/v22
[rust-install]: https://rust-lang.org/tools/install/
