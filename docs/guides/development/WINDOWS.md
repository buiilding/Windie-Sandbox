# Windows development

This guide prepares a clean 64-bit Windows machine for Windie source development. Once the prerequisites and Windows bootstrap steps are complete, follow the shared [development workflow](README.md) to run the gateway, API, Inspector, tray, and notifier.

Use a new PowerShell window after installing a tool so that its `PATH` changes are available.

## Prerequisites

Install these tools before building Windie:

| Tool | Required version | Why it is needed |
| --- | --- | --- |
| Git | Current | Clone the repository and its submodules. |
| Visual Studio Build Tools 2022 | Desktop development with C++ workload and Windows SDK | Rust's MSVC toolchain and native dependencies. |
| Rust | 1.98.0, including `rustfmt` and `clippy` | Windie's application and CLI. The repository records this in `rust-toolchain.toml`. |
| Go | 1.26.5 or newer | Bifrost gateway. CI uses Go 1.26.5. |
| Node.js | 22.23.2 | Inspector. This exact version is recorded in `vendor/windie-inspector/frontend/.nvmrc`. Bifrost's UI records 22.12.0 in its own `.nvmrc`. |
| MinGW-w64 GCC | POSIX/UCRT x64 | Enables Go CGO support for Bifrost's SQLite driver. |

Install Visual Studio Build Tools from [Visual Studio downloads](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022). In the installer, select **Desktop development with C++**, including the Windows SDK.

The following commands install the remaining system tools with WinGet:

```powershell
winget install --id Git.Git --exact --source winget
winget install --id Rustlang.Rustup --exact --source winget
winget install --id GoLang.Go --exact --source winget
winget install --id BrechtSanders.WinLibs.POSIX.UCRT --exact --source winget
```

Install Node.js 22.23.2 using the [Windows x64 installer](https://nodejs.org/dist/v22.23.2/node-v22.23.2-x64.msi). If you use a Windows Node version manager, select the version recorded in the relevant submodule's `.nvmrc` before installing or building its frontend dependencies.

Open a fresh PowerShell window, then initialize Rust and verify the toolchain:

```powershell
rustup toolchain install 1.98.0 --component rustfmt --component clippy

git --version
rustc +1.98.0 --version
cargo +1.98.0 --version
go version
node --version
npm --version
gcc --version
```

The Rust commands should report Rust 1.98.0, and `node --version` should report `v22.23.2`. `gcc --version` must work before starting the gateway; installing Go alone is not sufficient because Bifrost uses CGO-backed SQLite.

## Clone and bootstrap

Clone the repository with all submodules. If it was cloned without `--recurse-submodules`, initialize them before continuing.

```powershell
git clone --recurse-submodules https://github.com/buiilding/Windie-Sandbox.git
Set-Location Windie-Sandbox

# Only needed for an existing non-recursive clone.
git submodule update --init --recursive
```

Install the Inspector dependencies from the repository root:

```powershell
npm ci --legacy-peer-deps --prefix vendor\windie-inspector\frontend
```

Windie's gateway builds Bifrost with Go, but a fresh checkout does not yet include Bifrost's generated embedded UI. Build and copy those files once before the first gateway run, and repeat this step whenever Bifrost UI dependencies change:

```powershell
Push-Location vendor\bifrost\ui
# If using a Node version manager, select the version in vendor\bifrost\.nvmrc.
npm ci
npm exec vite build
npx tsc --noEmit

# Replace only Bifrost's generated Go-embedded UI directory.
Remove-Item -LiteralPath ..\transports\bifrost-http\ui -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item -LiteralPath out -Destination ..\transports\bifrost-http\ui -Recurse -Force
Pop-Location
```

Run the shared [verification commands](README.md#3-verify-the-checkout-before-starting-services) before starting services. The basic Windows build check is:

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Running Windie

Use three PowerShell windows from the repository root, following the shared [core development loop](README.md#4-run-the-core-development-components):

```powershell
# Terminal 1
cargo run --bin windie -- dev run gateway

# Terminal 2
$env:WINDIE_MARKETPLACE_INDEX_URL = "http://127.0.0.1:8788/index.json"
cargo run --bin windie -- dev run api

# Terminal 3
cargo run --bin windie -- dev run inspector
```

When testing local packages, build and serve the marketplace in a fourth PowerShell window:

```powershell
cargo run --bin windie -- marketplace build
cargo run --bin windie -- marketplace serve
```

The local service ports are gateway `8080`, API `8787`, Inspector `3000`, and marketplace `8788`. Check which process owns a port with:

```powershell
Get-NetTCPConnection -State Listen -LocalPort 8080,8787,3000,8788 -ErrorAction SilentlyContinue |
  Select-Object LocalAddress, LocalPort, OwningProcess
```

For an occupied port, inspect the displayed process ID with `Get-Process -Id <pid>`. Do not terminate a process unless you know it is a stale local Windie process.

Set port overrides in every PowerShell window that starts a Windie component:

```powershell
$env:WINDIE_GATEWAY_PORT = "18080"
$env:WINDIE_API_PORT = "18787"
$env:REACT_APP_WINDIE_API_URL = "http://127.0.0.1:18787"
```

## Troubleshooting

### `failed to run Go workspace command: program not found`

Go is not installed or the PowerShell window predates its installation. Install Go, close PowerShell completely, open a new window, and confirm `go version` succeeds.

### `pattern all:ui: no matching files found`

Bifrost's embedded UI has not been generated. Run the commands in [Clone and bootstrap](#clone-and-bootstrap) beginning with `Push-Location vendor\bifrost\ui`, then start the gateway again.

### `go-sqlite3 requires cgo to work` or `CGO_ENABLED=0`

Install the WinLibs POSIX/UCRT package, open a new PowerShell window, and confirm `gcc --version` works. Then check `go env CGO_ENABLED`; it should report `1`. Setting `CGO_ENABLED=1` without a working GCC compiler does not fix this error.

### `craco` is not recognized, or npm reports a peer-dependency resolution error

Install the Inspector dependencies exactly as shown above:

```powershell
npm ci --legacy-peer-deps --prefix vendor\windie-inspector\frontend
```

### The gateway starts but models fail to load

The gateway is running, but the configured provider credential or selected model is invalid. Configure a valid provider in Windie and verify its API key; this is separate from the local Rust, Go, Node, and marketplace setup.
