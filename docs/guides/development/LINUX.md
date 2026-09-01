# Linux development

This guide prepares a Linux machine for Windie source development. It installs
the native prerequisites and bootstraps a source checkout; then the shared
[development workflow](README.md) verifies the checkout and runs the gateway,
API, and Inspector.

## Supported Linux environments

Windie's release workflow produces native GNU/Linux archives for x86_64 and
ARM64 (`aarch64`) hosts. The Bifrost gateway is a native Go build that uses
CGO, so it needs a working compiler on the Linux machine; it is not a
cross-compilation workflow.

Check the machine architecture before choosing Go and Node downloads:

```bash
uname -m
```

Common output is `x86_64` for 64-bit Intel/AMD systems or `aarch64` for
64-bit ARM systems. Use a graphical desktop session for the local Inspector
and desktop notifications. Headless Linux can run the gateway and API, but
the normal Inspector command asks `xdg-open` to launch a browser.

## 1. Install system build prerequisites

Install Git, Curl, certificate roots, a C/C++ compiler, `pkg-config`, OpenSSL
headers, and `xdg-open`. Rust's default HTTP/TLS dependency requires the
OpenSSL development package, and Bifrost's SQLite dependency is built through
CGO.

Choose the commands for the Linux distribution:

### Debian or Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git curl ca-certificates xdg-utils
```

### Fedora

```bash
sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config openssl-devel git curl ca-certificates xdg-utils
```

### Arch Linux

```bash
sudo pacman -Syu --needed base-devel pkgconf openssl git curl ca-certificates xdg-utils
```

Confirm the compiler and browser launcher are available:

```bash
cc --version
pkg-config --version
xdg-open --help
```

## 2. Install Rust 1.98.0

Install Rust through Rustup, then install the exact compiler and components
recorded in Windie's `rust-toolchain.toml`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Close and reopen the shell before running this command.
rustup toolchain install 1.98.0 --component rustfmt --component clippy
```

Do not change the global Rust default for Windie. After cloning the repository,
Cargo automatically selects the version in `rust-toolchain.toml`. Verify it
from the repository root:

```bash
rustc --version
cargo --version
```

Both commands should report Rust 1.98.0.

See the [Rust installation guide][rust-install].

## 3. Install Go 1.26.5

Install Go 1.26.5 from the [official Go downloads page][go-downloads], using
the archive for the machine architecture. Ensure its `bin` directory is on
`PATH`, open a new shell, and verify it:

```bash
go version
```

The output should report Go 1.26.5 and `linux/amd64` or `linux/arm64`.
Bifrost requires Go 1.26.4 or newer; using the release workflow's exact
version avoids local-versus-release differences.

## 4. Clone Windie and its submodules

Clone the repository with all submodules. If it was cloned without
`--recurse-submodules`, initialize the submodules before continuing.

```bash
git clone --recurse-submodules https://github.com/buiilding/Windie-Sandbox.git
cd Windie-Sandbox

# Only needed for an existing non-recursive clone.
git submodule update --init --recursive
```

## 5. Install the Inspector and Bifrost Node versions

The Inspector records Node.js 22.23.2 in
`vendor/windie-inspector/frontend/.nvmrc`. Bifrost's UI records Node.js
22.12.0 in `vendor/bifrost/.nvmrc`. Use a version manager such as
[nvm][nvm] so both version files remain the source of truth rather than
choosing a floating system-wide Node release.

After cloning the repository, install and select each version in its own
submodule before installing that submodule's dependencies:

```bash
(
  cd vendor/windie-inspector/frontend
  nvm install
  nvm use
  node --version
  npm ci --legacy-peer-deps
)

(
  cd vendor/bifrost
  nvm install
  nvm use
  node --version
)
```

The first `node --version` should be `v22.23.2`; the second should be
`v22.12.0`. If you do not use a version manager, install those exact Node
releases from the [Node.js 22 archive][node-22-downloads] and select the
appropriate one before working in each submodule.

## 6. Generate Bifrost's embedded UI

The Bifrost HTTP transport embeds generated UI files. A clean source checkout
does not include them, so build the UI once before the first
`windie dev run gateway`; repeat this after Bifrost UI dependencies change.

```bash
(
  cd vendor/bifrost/ui
  nvm use
  npm ci
  npm run build
)
```

The build copies its output to
`vendor/bifrost/transports/bifrost-http/ui`, which the Go gateway embeds. Do
not hand-copy generated files or commit them merely to make a local gateway
build succeed.

## Continue with the shared workflow

The machine is ready when the toolchain checks and Bifrost UI build succeed.
Continue with the shared [verification and development workflow](README.md#3-verify-the-checkout-before-starting-services).

## Linux desktop behavior

Run the three core components from the shared guide in separate terminals:

```bash
cargo run --bin windie -- dev run gateway
cargo run --bin windie -- dev run api
cargo run --bin windie -- dev run inspector
```

The Inspector is a local React development server. After it is healthy,
Windie asks `xdg-open` to open `http://localhost:3000` with a one-time local
access code. It needs the API to be running first and a browser handler in the
active desktop session.

The Linux notifier is optional and uses the active Freedesktop notification
service over the user's D-Bus session:

```bash
cargo run --bin windie -- dev run notifier
```

Run it only from a graphical login that has a notification daemon. The
notifier observes the API; it does not own or start the API or gateway.

Do not run `cargo run --bin windie -- dev run tray` on Linux. Windie's tray is
currently supported on macOS and Windows only; the Linux command reports that
limitation and exits.

## Linux port diagnostics

The default local ports are gateway `8080`, API `8787`, and Inspector `3000`.
Inspect listeners before starting another checkout:

```bash
ss -ltnp '( sport = :8080 or sport = :8787 or sport = :3000 )'
```

If necessary, use distinct runtime ports in every terminal for that checkout:

```bash
export WINDIE_GATEWAY_PORT=18080
export WINDIE_API_PORT=18787
```

Start the Inspector with its matching API address:

```bash
REACT_APP_WINDIE_API_URL=http://127.0.0.1:18787 \
  cargo run --bin windie -- dev run inspector
```

## Linux troubleshooting

### OpenSSL or `pkg-config` build failure

Install the distribution's OpenSSL development and `pkg-config` packages from
[Install system build prerequisites](#1-install-system-build-prerequisites),
then open a new shell and confirm `pkg-config --modversion openssl` succeeds.

### `go-sqlite3 requires cgo to work` or `CGO_ENABLED=0`

Install the distribution's compiler toolchain, confirm `cc --version` works,
then run `go env CGO_ENABLED`. It should report `1`. Setting `CGO_ENABLED=1`
does not help without a functioning compiler.

### `pattern all:ui: no matching files found`

Build Bifrost's UI as described in [Generate Bifrost's embedded UI](#6-generate-bifrosts-embedded-ui), then start the gateway again.

### Inspector does not open a browser

Install `xdg-utils`, start the API before the Inspector, and run the command
from a graphical desktop session. An SSH or headless session generally has no
browser handler for `xdg-open`.

### `Failed to connect to D-Bus` or notification delivery fails

The notifier needs the user's graphical-session D-Bus and a running desktop
notification service. Do not use it for a headless server; the gateway and API
remain independent of notifications.

Repository, API, Inspector, and Bifrost failures that are not Linux-specific
belong in the [shared troubleshooting section](README.md#common-troubleshooting).

## Related repository files

- [`rust-toolchain.toml`](../../../rust-toolchain.toml), [`.go-version`](../../../.go-version), and the two `.nvmrc` files record the toolchain versions.
- [`src/dev.rs`](../../../src/dev.rs) builds the local Bifrost workspace and starts one foreground component at a time.
- [`src/local/tray.rs`](../../../src/local/tray.rs) defines the Linux tray limitation.
- [`src/local/tray_notification.rs`](../../../src/local/tray_notification.rs) delivers Linux notifications through the desktop service.
- [`scripts/package-release.sh`](../../../scripts/package-release.sh) shows the native Linux release targets and Bifrost UI build order.

[go-downloads]: https://go.dev/dl/
[node-22-downloads]: https://nodejs.org/en/download/archive/v22
[nvm]: https://github.com/nvm-sh/nvm
[rust-install]: https://rust-lang.org/tools/install/
