# Builds a native local release and installs it through the local copy of the
# public Windows installer. The test uses isolated paths so it does not replace
# a normal Windie installation.
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts/test-local-installer.ps1
#
# Optional endpoint settings are inherited by the installed processes:
#   $env:WINDIE_GATEWAY_PORT = "8081"
#   $env:WINDIE_API_PORT = "8788"

[CmdletBinding()]
param(
    [string]$RustTarget,
    [string]$AssetLabel
)

$ErrorActionPreference = "Stop"

function Import-WindieWindowsBuildEnvironment {
    $gcc = Get-Command gcc.exe -ErrorAction SilentlyContinue
    if (-not $env:VCToolsInstallDir) {
        $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
        if (-not (Test-Path -LiteralPath $vswhere)) {
            throw "Visual Studio Build Tools are required. Install the C++ workload before running the local Windows release test."
        }
        $vsPath = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath).Trim()
        if (-not $vsPath) {
            throw "Visual Studio C++ Build Tools were not found. Install the C++ workload before running the local Windows release test."
        }
        $vsDevCmd = Join-Path $vsPath "Common7\Tools\VsDevCmd.bat"
        $environmentLines = & cmd.exe /d /s /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set"
        if ($LASTEXITCODE -ne 0) {
            throw "failed to load the Visual Studio build environment"
        }
        foreach ($line in $environmentLines) {
            if ($line -match "^(?<name>[A-Za-z_][A-Za-z0-9_]*)=(?<value>.*)$") {
                Set-Item -Path ("Env:{0}" -f $matches.name) -Value $matches.value
            }
        }
    }

    $env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\Go\bin;$env:Path"
    if ($gcc) {
        $env:WINDIE_GO_CC = $gcc.Source
    }
    $linker = Get-Command link.exe -ErrorAction SilentlyContinue
    if ($linker -and $linker.Source -match "Microsoft Visual Studio") {
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker.Source
    }
}

Import-WindieWindowsBuildEnvironment

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$LandingRoot = if ($env:WINDIE_LANDING_DIR) {
    $env:WINDIE_LANDING_DIR
} else {
    Join-Path $RepoRoot "vendor\windie-landing-2nd"
}
$Installer = Join-Path $LandingRoot "frontend\public\install.ps1"

if (-not $env:WINDIE_LANDING_DIR -and -not (Test-Path -LiteralPath $Installer -PathType Leaf)) {
    $SharedLandingRoot = Join-Path $RepoRoot "..\windie\vendor\windie-landing-2nd"
    $SharedInstaller = Join-Path $SharedLandingRoot "frontend\public\install.ps1"
    if (Test-Path -LiteralPath $SharedInstaller -PathType Leaf) {
        $LandingRoot = $SharedLandingRoot
        $Installer = $SharedInstaller
    }
}

if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) {
    throw "local Windie installer was not found at $Installer"
}

$Architecture = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}
if (-not $RustTarget -or -not $AssetLabel) {
    switch ($Architecture) {
        "AMD64" {
            $RustTarget = "x86_64-pc-windows-msvc"
            $AssetLabel = "windows-x86_64"
        }
        "ARM64" {
            throw "Windows ARM64 assets are not published yet; use x64 Windows or WSL."
        }
        default { throw "unsupported Windows architecture: $Architecture" }
    }
}

$TestRoot = if ($env:WINDIE_LOCAL_TEST_ROOT) {
    $env:WINDIE_LOCAL_TEST_ROOT
} else {
    Join-Path $RepoRoot "target\local-installer\$AssetLabel"
}
$DistDir = Join-Path $TestRoot "dist"
$InstallDir = Join-Path $TestRoot "bin"
$WindieHome = Join-Path $TestRoot ".windie"
$Archive = Join-Path $DistDir "windie-$AssetLabel.zip"
$PackageScript = Join-Path $RepoRoot "scripts\package-release.ps1"

New-Item -ItemType Directory -Path $TestRoot -Force | Out-Null

$PreviousWindie = Join-Path $InstallDir "windie.exe"
if (Test-Path -LiteralPath $PreviousWindie -PathType Leaf) {
    Write-Host "==> stopping previous local Windie installation"
    $env:WINDIE_HOME = $WindieHome
    $env:WINDIE_INSTALL_DIR = $InstallDir
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $PreviousWindie uninstall --yes 2>$null | Out-Null
    $previousUninstallExitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousErrorActionPreference
    if ($previousUninstallExitCode -ne 0) {
        throw "previous local Windie uninstall failed with exit code $previousUninstallExitCode"
    }

    # Older local packages could schedule a broken self-cleanup command. The
    # test root is disposable, so clear its remaining state before rebuilding.
    foreach ($path in @($InstallDir, $WindieHome)) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}

Write-Host "==> packaging local release"
$env:GITHUB_REF_NAME = "local-dev"
$env:WINDIE_REUSE_BIFROST = "1"
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $PackageScript $RustTarget $AssetLabel $DistDir
if ($LASTEXITCODE -ne 0) {
    throw "local release packaging failed with exit code $LASTEXITCODE"
}

if (-not (Test-Path -LiteralPath $Archive -PathType Leaf) -or
    -not (Test-Path -LiteralPath "$Archive.sha256" -PathType Leaf)) {
    throw "local release package is incomplete: $Archive"
}

$env:WINDIE_ASSET_URL = ([Uri]$Archive).AbsoluteUri
$env:WINDIE_CHECKSUM_URL = ([Uri]([string]$Archive + ".sha256")).AbsoluteUri
$env:WINDIE_INSTALL_DIR = $InstallDir
$env:WINDIE_HOME = $WindieHome
$env:WINDIE_SKIP_PATH_UPDATE = "1"

Write-Host "==> running local installer"
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Installer
if ($LASTEXITCODE -ne 0) {
    throw "local installer failed with exit code $LASTEXITCODE"
}

Write-Host "==> local installer test completed"
Write-Host "install directory: $InstallDir"
Write-Host "Windie home: $WindieHome"
