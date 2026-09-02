# Windows development

This guide prepares a clean 64-bit Windows machine for Windie source
development. Once the prerequisites are ready, follow the shared
[development workflow](README.md).

Open a new PowerShell window after installing a tool so that its `PATH` changes
are available.

## Prerequisites

| Tool | Required version | Why it is needed |
| --- | --- | --- |
| Git | Current | Clone Windie and its submodules. |
| Visual Studio Build Tools 2022 | Desktop development with C++ workload and Windows SDK | Rust's MSVC toolchain and native dependencies. |
| Rust | 1.98.0, plus `rustfmt` and `clippy` | Windie's CLI and local runtime. |
| Go | 1.26.5 | Bifrost gateway. |
| Node.js | 22.23.2 | Windie Inspector. |
| MinGW-w64 GCC | POSIX/UCRT x64 | Bifrost's CGO-backed SQLite driver. |

Install Visual Studio Build Tools from [Visual Studio downloads][visual-studio].
In the installer, select **Desktop development with C++**, including the
Windows SDK.

Install the remaining system tools with WinGet:

```powershell
winget install --id Git.Git --exact --source winget
winget install --id Rustlang.Rustup --exact --source winget
winget install --id GoLang.Go --exact --source winget
winget install --id BrechtSanders.WinLibs.POSIX.UCRT --exact --source winget
```

Install Node.js 22.23.2, the Inspector version in its `.nvmrc`, using the
matching [Windows x64 installer][node-22-downloads]. If you use a Windows Node
version manager, select that version before running the Inspector.

Initialize Rust and verify the toolchain:

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

The Rust commands should report the version in `rust-toolchain.toml`, and
`node --version` should match the Inspector's `.nvmrc`. `gcc --version` must
work before starting the gateway because Bifrost uses CGO-backed SQLite.

## Continue with the shared workflow

The machine is ready when the toolchain checks succeed. Continue with the
[shared development workflow](README.md#2-clone-windie-and-its-submodules).

## Windows port diagnostics

The shared workflow uses gateway port `8080`, API port `8787`, and Inspector
port `3000`. Check which process owns a port with:

```powershell
Get-NetTCPConnection -State Listen -LocalPort 8080,8787,3000 -ErrorAction SilentlyContinue |
  Select-Object LocalAddress, LocalPort, OwningProcess
```

For an occupied port, inspect the displayed process ID with
`Get-Process -Id <pid>`. Do not terminate a process unless you know it is a
stale local Windie process.

Set port overrides in every PowerShell window that starts a Windie component:

```powershell
$env:WINDIE_GATEWAY_PORT = "18080"
$env:WINDIE_API_PORT = "18787"
$env:REACT_APP_WINDIE_API_URL = "http://127.0.0.1:18787"
```

## Windows troubleshooting

### `failed to run Go workspace command: program not found`

Go is not installed or the PowerShell window predates its installation. Install
Go, close PowerShell completely, open a new window, and confirm `go version`
succeeds.

### `go-sqlite3 requires cgo to work` or `CGO_ENABLED=0`

Install the WinLibs POSIX/UCRT package, open a new PowerShell window, and
confirm `gcc --version` works. Then check `go env CGO_ENABLED`; it should
report `1`. Setting `CGO_ENABLED=1` without a working GCC compiler does not fix
this error.

### `craco` is not recognized, or npm reports a peer-dependency resolution error

Stop the Inspector and run `windie dev run inspector` again. The development
command reinstalls its locked dependencies when the recorded dependency
fingerprint is missing or stale. To force a clean reinstall manually, run:

```powershell
npm ci --legacy-peer-deps --prefix vendor\windie-inspector\frontend
```

### The gateway starts but models fail to load

The gateway is running, but the configured provider credential or selected
model is invalid. Configure a valid provider in Windie and verify its API key;
this is separate from the local Rust, Go, and Node setup.

[node-22-downloads]: https://nodejs.org/en/download/archive/v22
[visual-studio]: https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022
