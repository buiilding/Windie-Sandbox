# Builds a native local release and installs it through the local copy of the
# public Windows installer. The test uses isolated paths so it does not replace
# a normal Windie installation.
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts/test-local-installer.ps1

[CmdletBinding()]
param(
    [string]$RustTarget,
    [string]$AssetLabel
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$LandingRoot = if ($env:WINDIE_LANDING_DIR) {
    $env:WINDIE_LANDING_DIR
} else {
    Join-Path $RepoRoot "..\windie-landing-2nd"
}
$Installer = Join-Path $LandingRoot "frontend\public\install.ps1"
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
    & $PreviousWindie uninstall --yes *> $null
}

Write-Host "==> packaging local release"
$env:GITHUB_REF_NAME = "local-dev"
$env:WINDIE_REUSE_BIFROST = "1"
$env:WINDIE_REUSE_INSPECTOR = "1"
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
