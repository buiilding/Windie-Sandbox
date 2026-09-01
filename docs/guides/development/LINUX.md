# Linux development

This guide prepares a Linux machine for Windie source development. It installs
the native prerequisites; the shared [development workflow](README.md) then
clones, verifies, and runs Windie.

## Supported Linux environments

Windie releases native GNU/Linux archives for x86_64 and ARM64 (`aarch64`).
Bifrost is a native Go build using CGO, so the development machine needs a
working compiler; this is not a cross-compilation workflow.

```bash
uname -m
```

Use a graphical desktop session for the local Inspector and desktop
notifications. Headless Linux can run the gateway and API, but cannot normally
open the Inspector through `xdg-open`.

## 1. Install system build prerequisites

Install Git, Curl, certificate roots, a C/C++ compiler, `pkg-config`, OpenSSL
headers, and `xdg-open`. Rust's HTTP/TLS dependency uses the OpenSSL development
package; Bifrost's SQLite dependency is built through CGO.

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

Install Rust through Rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new shell, then install the compiler and components in
`rust-toolchain.toml`:

```bash
rustup toolchain install 1.98.0 --component rustfmt --component clippy
```

Cargo selects the repository version after cloning. Verify it from the
checkout:

```bash
rustc --version
cargo --version
```

See [the Rust installation guide][rust-install].

## 3. Install Go 1.26.5

Install Go 1.26.5, the version in Windie's
[`.go-version`](../../../.go-version), from the [official Go downloads
page][go-downloads], using the archive for the machine architecture. Ensure its
`bin` directory is on `PATH`, open a new shell, and verify it:

```bash
go version
```

The result should report Go 1.26.5 and `linux/amd64` or `linux/arm64`.

## 4. Install Node.js 22.23.2

After cloning Windie, use Node.js 22.23.2, the Inspector version recorded in
`vendor/windie-inspector/frontend/.nvmrc`. A version manager such as [nvm][nvm]
makes this repeatable:

```bash
(
  cd vendor/windie-inspector/frontend
  nvm install
  nvm use
  node --version
)
```

The shared guide installs the Inspector's dependencies. Windie's development
gateway supplies Bifrost's ignored Go-embed placeholder itself, so normal
Windie development does not need Bifrost's separate dashboard Node version.

## Continue with the shared workflow

The machine is ready when the toolchain checks above succeed. Continue with
the shared [development workflow](README.md#2-clone-windie-and-its-submodules).

## Linux desktop behavior

The shared guide starts the three core components in separate terminals. The
Inspector needs an active desktop session because Windie asks `xdg-open` to
open it with a one-time local access code.

The Linux notifier is optional and uses the active Freedesktop notification
service over the user's D-Bus session:

```bash
cargo run --bin windie -- dev run notifier
```

Run it only from a graphical login with a notification daemon. The notifier
observes the API; it does not own or start the API or gateway.

Do not run `cargo run --bin windie -- dev run tray` on Linux. Windie's tray is
currently supported on macOS and Windows only; the Linux command reports that
limitation and exits.

## Linux port diagnostics

Inspect listeners before starting another checkout:

```bash
ss -ltnp '( sport = :8080 or sport = :8787 or sport = :3000 )'
```

Use distinct runtime ports in every terminal for that checkout when needed:

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

Install the distribution's OpenSSL development and `pkg-config` packages, then
confirm `pkg-config --modversion openssl` succeeds.

### `go-sqlite3 requires cgo to work` or `CGO_ENABLED=0`

Install the distribution's compiler toolchain, confirm `cc --version` works,
then run `go env CGO_ENABLED`. It should report `1`. Setting `CGO_ENABLED=1`
does not help without a working compiler.

### Inspector does not open a browser

Install `xdg-utils`, start the API before the Inspector, and run the command
from a graphical desktop session. An SSH or headless session generally has no
browser handler for `xdg-open`.

### `Failed to connect to D-Bus` or notification delivery fails

The notifier needs the user's graphical-session D-Bus and a running desktop
notification service. Do not use it for a headless server; the gateway and API
remain independent of notifications.

## Related material

- [`rust-toolchain.toml`](../../../rust-toolchain.toml), [`.go-version`](../../../.go-version), and the Inspector `.nvmrc` record toolchain versions.
- [`src/dev.rs`](../../../src/dev.rs) builds the local Bifrost workspace and starts one foreground component at a time.
- [`src/local/tray.rs`](../../../src/local/tray.rs) defines the Linux tray limitation.
- [`src/local/tray_notification.rs`](../../../src/local/tray_notification.rs) delivers Linux notifications through the desktop service.
- [`scripts/package-release.sh`](../../../scripts/package-release.sh) owns the full Bifrost-dashboard build for release packaging.

[go-downloads]: https://go.dev/dl/
[nvm]: https://github.com/nvm-sh/nvm
[rust-install]: https://rust-lang.org/tools/install/
