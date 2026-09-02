# macOS prerequisites

This guide installs the native prerequisites required for Windie development
on macOS. Complete these steps before continuing with the shared development
workflow.

## 1. Install command-line tools

The command-line tools provide the compiler, linker, SDK, and Git needed by
the source builds. See [Apple's command-line tools documentation][apple-cli]
for more information.

Open Terminal and run:

```bash
xcode-select --install
```

Verify Installation:

```bash
xcode-select -p
clang --version
git --version
```

## 2. Install Rust

Install Rust version 1.98.0 through [Rustup][rustup]:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Restart the terminal then run:

```bash
rustup toolchain install 1.98.0 --component rustfmt --component clippy
```

Verify Installation:

```bash
rustc --version
cargo --version
```

The Rust compiler should report version `1.98.0`.

## 3. Install Go

Install Go `1.26.5` from the [official Go downloads page][go-downloads].
Choose the macOS installer that matches your Mac, then follow the installer
steps.

### Optional: install Go with one command

```bash
go_arch="$(uname -m)"; [ "$go_arch" = "arm64" ] || [ "$go_arch" = "x86_64" ] || { echo "unsupported Mac architecture: $go_arch" >&2; exit 1; }; [ "$go_arch" = "x86_64" ] && go_arch="amd64"; go_temp_dir="$(mktemp -d)" && trap 'rm -rf "$go_temp_dir"' EXIT && curl -fsSL "https://go.dev/dl/go1.26.5.darwin-${go_arch}.pkg" -o "$go_temp_dir/go.pkg" && sudo installer -pkg "$go_temp_dir/go.pkg" -target / && export PATH="/usr/local/go/bin:$PATH"
```

The command detects Apple Silicon or Intel, downloads the matching official
package, and asks for the macOS administrator password.

Verify Installation:

```bash
go version
```

The output should report Go `1.26.5`.

## 4. Install Node.js

Install Node.js `22.23.2` from the [Node.js 22 downloads page][node-downloads].
Download and run the installer.

### Optional: install Node.js with one command

```bash
node_temp_dir="$(mktemp -d)" && trap 'rm -rf "$node_temp_dir"' EXIT && curl -fsSL "https://nodejs.org/dist/v22.23.2/node-v22.23.2.pkg" -o "$node_temp_dir/node.pkg" && sudo installer -pkg "$node_temp_dir/node.pkg" -target / && export PATH="/usr/local/bin:$PATH"
```

The command downloads the official macOS package and asks for the macOS
administrator password.

Verify Installation:

```bash
node --version
npm --version
```

Node.js should report version `22.23.2`.

## 5. Final check

Run the complete check from a new Terminal:

```bash
xcode-select -p
clang --version
git --version
rustc --version
cargo --version
go version
node --version
npm --version
```

Confirm that the required commands are available and that Rust, Go, and
Node.js report the versions listed above. Fix any failed check before
continuing.

## 6. Continue setup

After the final check succeeds, continue with the [shared development
workflow](README.md#2-clone-windie).

## 7. Common troubleshooting

### `xcrun: error: invalid active developer path`

Run `xcode-select --install`, complete the installer, and repeat the final
check. If the tools are already installed, run `xcode-select -p` to inspect
the selected developer directory.

### `cargo`, `go`, or `node` is not found

Open a new Terminal after installing the tool and run its verification command
again. A new shell loads the installation's PATH changes.

### A tool reports the wrong version

Install the required version from the source linked in the relevant section.
Do not continue until the final check reports Rust `1.98.0`, Go `1.26.5`, and
Node.js `22.23.2`.

## 8. Related code and documentation

- [Shared development workflow](README.md)
- [`BACKEND.md`](../../index/BACKEND.md), the Rust runtime source map
- [`FRONTEND.md`](../../index/FRONTEND.md), the Inspector source map
- [Architecture overview](../../architecture/overview.md)

[apple-cli]: https://developer.apple.com/documentation/xcode/installing-the-command-line-tools
[go-downloads]: https://go.dev/dl/
[node-downloads]: https://nodejs.org/en/download/archive/v22
[rustup]: https://rustup.rs/
