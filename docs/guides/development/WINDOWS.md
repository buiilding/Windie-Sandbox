# Windows prerequisites

This guide installs the native prerequisites required for Windie development
on 64-bit Windows. Complete these steps before continuing with the shared
development workflow.

Open a new PowerShell window after installing a tool so that its `PATH`
changes are loaded.

## 1. Install Windows build tools

Install [Visual Studio Build Tools 2022][visual-studio]. In the installer,
select **Desktop development with C++**, including the Windows SDK.

Windie's Bifrost gateway also requires a POSIX/UCRT GCC toolchain for its
CGO-backed SQLite dependency. Install Git and WinLibs with WinGet:

```powershell
winget install --id Git.Git --exact --source winget
winget install --id BrechtSanders.WinLibs.POSIX.UCRT --exact --source winget
```

Verify the native tools:

```powershell
git --version
gcc --version
```

## 2. Install Rust 1.98.0

Install Rustup with WinGet:

```powershell
winget install --id Rustlang.Rustup --exact --source winget
```

Open a new PowerShell window, then install the version and components required
by Windie:

```powershell
rustup toolchain install 1.98.0 --component rustfmt --component clippy
```

Verify Rust:

```powershell
rustc +1.98.0 --version
cargo +1.98.0 --version
```

The Rust compiler should report version `1.98.0`.

## 3. Install Go 1.26.5

Install Go `1.26.5` with WinGet:

```powershell
winget install --id GoLang.Go --exact --source winget
```

Alternatively, install the matching Windows package from the [official Go
downloads page][go-downloads].

Verify Go:

```powershell
go version
```

The result should report Go `1.26.5`.

## 4. Install Node.js 22.23.2

Install Node.js `22.23.2` from the [Node.js 22 downloads page][node-downloads].
Choose the Windows x64 installer and follow its installation steps.

Verify Node.js and npm:

```powershell
node --version
npm --version
```

Node.js should report version `22.23.2`.

## 5. Final check

Run the complete check from a new PowerShell window:

```powershell
git --version
gcc --version
rustc +1.98.0 --version
cargo +1.98.0 --version
go version
go env CGO_ENABLED
node --version
npm --version
```

Confirm that the required commands are available, Rust, Go, and Node.js report
the required versions, and `go env CGO_ENABLED` reports `1`. Fix any failed
check before continuing.

## 6. Continue setup

After the final check succeeds, continue with the [shared development
workflow](README.md#2-clone-windie).

## 7. Common troubleshooting

### `failed to run Go workspace command: program not found`

Open a new PowerShell window after installing Go and run `go version` again.
The earlier window may not contain the installation's updated `PATH`.

### `go-sqlite3 requires cgo to work` or `CGO_ENABLED=0`

Install the WinLibs POSIX/UCRT package, then confirm that `gcc --version`
works and `go env CGO_ENABLED` reports `1`. Setting `CGO_ENABLED=1` without a
working GCC compiler does not fix this error.

### The Inspector cannot start or reach the API

Confirm that the API is running and that the Inspector command is being run
from the repository root. If the frontend dependencies are unavailable,
restart the Inspector command from a new PowerShell window.

### The gateway starts but model requests fail

The gateway may be healthy while the provider credential, selected model, or
provider configuration is invalid. Recheck the provider setup in the
Inspector and verify the model name.

## 8. Related code and documentation

- [Shared development workflow](README.md)
- [`BACKEND.md`](../../index/BACKEND.md), the Rust runtime source map
- [`FRONTEND.md`](../../index/FRONTEND.md), the Inspector source map
- [Architecture overview](../../architecture/overview.md)

[go-downloads]: https://go.dev/dl/
[node-downloads]: https://nodejs.org/en/download/archive/v22
[visual-studio]: https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022
