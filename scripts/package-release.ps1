# Builds one native Windows Windie release archive.
#
# Usage: scripts/package-release.ps1 <rust-target> <asset-label> <dist-dir>
# The Windows runner already matches the target, so Rust and CGO builds remain
# native and the resulting archive contains the unified Windie executable,
# Bifrost, and Inspector.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$RustTarget,

    [Parameter(Mandatory = $true, Position = 1)]
    [string]$AssetLabel,

    [Parameter(Mandatory = $true, Position = 2)]
    [string]$DistDir
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$InspectorDir = Join-Path $RepoRoot "dev\windie-inspector"
$BifrostDir = Join-Path $RepoRoot "vendor\bifrost"
$BifrostHttpDir = Join-Path $BifrostDir "transports\bifrost-http"
$BifrostVersion = "stable"
$Version = if ($env:GITHUB_REF_NAME) { $env:GITHUB_REF_NAME } else { "dev" }
$StagingDir = Join-Path ([System.IO.Path]::GetTempPath()) ("windie-release-" + [guid]::NewGuid())
$BifrostBinary = Join-Path $BifrostDir "tmp\bifrost-http.exe"

function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

try {
    Write-Host "==> windie release: target=$RustTarget label=$AssetLabel version=$Version"
    New-Item -ItemType Directory -Path $StagingDir -Force | Out-Null

    $WindieBinary = Join-Path $RepoRoot "target\$RustTarget\release\windie.exe"
    $InspectorBinary = Join-Path $RepoRoot "target\$RustTarget\release\windie-inspector.exe"

    if ($env:WINDIE_REUSE_INSPECTOR -eq "1" -and (Test-Path -LiteralPath $InspectorBinary -PathType Leaf)) {
        Write-Host "==> reusing cached windie inspector"
    }
    else {
        Write-Host "==> building inspector UI"
        Invoke-Native "npm" @("ci", "--prefix", $InspectorDir, "--legacy-peer-deps")
        Invoke-Native "npm" @("run", "build", "--prefix", $InspectorDir)
        Write-Host "==> building windie inspector ($RustTarget)"
        Invoke-Native "cargo" @("build", "--release", "--target", $RustTarget, "--manifest-path", (Join-Path $RepoRoot "Cargo.toml"), "--bin", "windie-inspector")
    }
    if (-not (Test-Path -LiteralPath $InspectorBinary -PathType Leaf)) {
        throw "windie inspector binary not found at $InspectorBinary"
    }

    Write-Host "==> building windie ($RustTarget)"
    Invoke-Native "cargo" @("build", "--release", "--target", $RustTarget, "--manifest-path", (Join-Path $RepoRoot "Cargo.toml"), "--bin", "windie")
    if (-not (Test-Path -LiteralPath $WindieBinary -PathType Leaf)) {
        throw "windie binary not found at $WindieBinary"
    }

    if ($env:WINDIE_REUSE_BIFROST -eq "1" -and (Test-Path -LiteralPath $BifrostBinary -PathType Leaf)) {
        Write-Host "==> reusing cached bifrost ($BifrostVersion)"
    }
    else {
        Write-Host "==> building bifrost UI"
        Invoke-Native "npm" @("ci", "--prefix", (Join-Path $BifrostDir "ui"))
        Invoke-Native "npm" @("run", "build", "--prefix", (Join-Path $BifrostDir "ui"))
        $BifrostIndex = Join-Path $BifrostHttpDir "ui\index.html"
        if (-not (Test-Path -LiteralPath $BifrostIndex -PathType Leaf)) {
            throw "bifrost UI build did not produce $BifrostIndex"
        }

        Write-Host "==> setting up bifrost go workspace (use local modules)"
        Push-Location $BifrostDir
        try {
            Remove-Item -LiteralPath @((Join-Path $BifrostDir "go.work"), (Join-Path $BifrostDir "go.work.sum")) -Force -ErrorAction SilentlyContinue
            Invoke-Native "go" @("work", "init", "./cli", "./core", "./framework", "./transports")
            Get-ChildItem (Join-Path $BifrostDir "plugins") -Directory | ForEach-Object {
                if (Test-Path -LiteralPath (Join-Path $_.FullName "go.mod")) {
                    Invoke-Native "go" @("work", "use", $_.FullName)
                }
            }
            Invoke-Native "go" @("work", "sync")
        }
        finally {
            Pop-Location
        }

        Write-Host "==> building bifrost (native Windows CGO sqlite)"
        New-Item -ItemType Directory -Path (Join-Path $BifrostDir "tmp") -Force | Out-Null
        Push-Location $BifrostHttpDir
        try {
            $env:CGO_ENABLED = "1"
            Invoke-Native "go" @(
                "build",
                "-ldflags=-w -s -X main.Version=$BifrostVersion",
                "-trimpath",
                "-tags=sqlite_static",
                "-o=$BifrostBinary",
                "."
            )
        }
        finally {
            Pop-Location
        }
    }
    if (-not (Test-Path -LiteralPath $BifrostBinary -PathType Leaf)) {
        throw "bifrost binary not found at $BifrostBinary"
    }

    Copy-Item -LiteralPath $WindieBinary -Destination (Join-Path $StagingDir "windie.exe")
    Copy-Item -LiteralPath $BifrostBinary -Destination (Join-Path $StagingDir "bifrost.exe")
    Copy-Item -LiteralPath $InspectorBinary -Destination (Join-Path $StagingDir "windie-inspector.exe")
    @(
        "windie_version=$Version"
        "bifrost_version=$BifrostVersion"
        "asset_label=$AssetLabel"
        "rust_target=$RustTarget"
        "os=windows"
        "cpu=$(if ($RustTarget.StartsWith('aarch64')) { 'aarch64' } else { 'x86_64' })"
        "contents=windie.exe,bifrost.exe,windie-inspector.exe"
    ) | Set-Content -LiteralPath (Join-Path $StagingDir "release-manifest.txt") -Encoding utf8
    Invoke-Native $WindieBinary @("--version")
    & $BifrostBinary --help *> $null
    New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
    $Archive = Join-Path $DistDir "windie-$AssetLabel.zip"
    Compress-Archive -Path (Join-Path $StagingDir "windie.exe"), (Join-Path $StagingDir "bifrost.exe"), (Join-Path $StagingDir "windie-inspector.exe"), (Join-Path $StagingDir "release-manifest.txt") -DestinationPath $Archive -Force
    (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash.ToLowerInvariant() + "  " + (Split-Path $Archive -Leaf) | Set-Content -LiteralPath "$Archive.sha256" -Encoding ascii
    Write-Host "==> wrote $Archive"
    Get-Item -LiteralPath $Archive | Select-Object FullName, Length
}
finally {
    if (Test-Path -LiteralPath $StagingDir) {
        Remove-Item -LiteralPath $StagingDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
