$ErrorActionPreference = "Stop"

$repo = if ($env:WINDIE_REPO) { $env:WINDIE_REPO } else { "buiilding/Windie-Sandbox" }
$installDir = if ($env:WINDIE_INSTALL_DIR) {
    [Environment]::ExpandEnvironmentVariables($env:WINDIE_INSTALL_DIR)
} else {
    Join-Path $env:USERPROFILE ".local\bin"
}
$windieHome = if ($env:WINDIE_HOME) {
    [Environment]::ExpandEnvironmentVariables($env:WINDIE_HOME)
} else {
    Join-Path $env:USERPROFILE ".windie"
}
$gatewayAddress = "127.0.0.1:8080"
$apiAddress = "127.0.0.1:8787"
$inspectorAddress = "127.0.0.1:3000"

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "Windie requires a 64-bit Windows installation."
}

$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64" } else { "x86_64" }
$asset = "windie-windows-$arch.zip"
$assetUrl = if ($env:WINDIE_ASSET_URL) {
    $env:WINDIE_ASSET_URL
} else {
    "https://github.com/$repo/releases/latest/download/$asset"
}

New-Item -ItemType Directory -Path $installDir, $windieHome, (Join-Path $windieHome "bifrost"), (Join-Path $windieHome "benchmarks") -Force | Out-Null
$envFile = Join-Path $windieHome ".env"
if (-not (Test-Path -LiteralPath $envFile -PathType Leaf)) {
    New-Item -ItemType File -Path $envFile -Force | Out-Null
}

$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("windie-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
try {
    $archive = Join-Path $tempDir $asset
    Invoke-WebRequest -Uri $assetUrl -OutFile $archive
    Expand-Archive -LiteralPath $archive -DestinationPath $tempDir -Force

    foreach ($name in @("windie.exe", "bifrost.exe", "windie-inspector.exe")) {
        $source = Join-Path $tempDir $name
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Release asset did not contain $name."
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $installDir $name) -Force
    }
}
finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @()
if ($userPath) {
    $pathEntries = @($userPath -split ";" | Where-Object { $_ })
}
$normalizedInstallDir = [IO.Path]::GetFullPath($installDir).TrimEnd("\")
$hasInstallDir = $pathEntries | Where-Object {
    try {
        ([IO.Path]::GetFullPath($_).TrimEnd("\") -ieq $normalizedInstallDir)
    }
    catch {
        $false
    }
}
if (-not $hasInstallDir) {
    $pathEntries += $installDir
    [Environment]::SetEnvironmentVariable("Path", ($pathEntries -join ";"), "User")
    $env:Path = "$installDir;$env:Path"
}

$windie = Join-Path $installDir "windie.exe"
$gatewayHealthUrl = "http://$gatewayAddress/health"
$apiHealthUrl = "http://$apiAddress/api/health"
$inspectorHealthUrl = "http://$inspectorAddress/"

function Test-WindieHealth {
    param([string]$Uri)

    try {
        Invoke-WebRequest -UseBasicParsing -Uri $Uri -TimeoutSec 2 | Out-Null
        return $true
    }
    catch {
        return $false
    }
}

function Write-WindieProgressBar {
    param([int]$Percent)

    $width = 20
    $filled = [Math]::Floor($width * $Percent / 100)
    $empty = $width - $filled
    $bar = ("#" * $filled) + ("-" * $empty)
    Write-Host ("`r[{0}] {1,3}%" -f $bar, $Percent) -NoNewline
}

function Wait-WindieHealth {
    param(
        [string]$Uri,
        [int]$Attempts,
        [string]$Component
    )

    Write-WindieProgressBar 80
    for ($attempt = 0; $attempt -lt $Attempts; $attempt++) {
        if (Test-WindieHealth $Uri) {
            Write-WindieProgressBar 100
            Write-Host ""
            return
        }
        Start-Sleep -Seconds 1
    }
    Write-Host ""
    throw "Windie installed, but $Component did not start."
}

$env:WINDIE_BIFROST_BIN = Join-Path $installDir "bifrost.exe"

Write-Host "Installing LLM gateway"
if (-not (Test-WindieHealth $gatewayHealthUrl)) {
    Write-WindieProgressBar 80
    & $windie "gateway" "start" *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        throw "failed to start the LLM gateway"
    }
}
Wait-WindieHealth $gatewayHealthUrl 30 "the LLM gateway"
Write-Host "Started the gateway at http://$gatewayAddress"

Write-Host "Installing Windie runtime"
if (-not (Test-WindieHealth $apiHealthUrl)) {
    Write-WindieProgressBar 80
    & $windie "api" "start" *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        throw "failed to start the Windie runtime. Output: $windieHome\windie-api.log"
    }
}
try {
    Wait-WindieHealth $apiHealthUrl 75 "the Windie runtime"
}
catch {
    throw "Windie installed, but the local API did not start. Output: $windieHome\windie-api.log"
}
Write-Host "Started the runtime at http://$apiAddress"

Write-Host "Installing Windie Inspector UI"
if (-not (Test-WindieHealth $inspectorHealthUrl)) {
    Write-WindieProgressBar 80
    & $windie "inspector" "start" *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        throw "failed to start the Windie Inspector UI. Output: $windieHome\windie-inspector.log"
    }
}
Wait-WindieHealth $inspectorHealthUrl 30 "the Windie Inspector UI"

$uiUrl = "http://$inspectorAddress"
Start-Process $uiUrl
Start-Process -FilePath $windie -ArgumentList @("tray")
Write-Host "Started the UI at $uiUrl"
Write-Host "Click on the tray on your desktop to manage these processes."

Write-Output "windie installed at $(Join-Path $installDir 'windie.exe')"
Write-Output "Windie tray available as: $windie tray"
Write-Output "bundled Bifrost installed at $(Join-Path $installDir 'bifrost.exe')"
Write-Output "Inspector installed at $(Join-Path $installDir 'windie-inspector.exe')"
Write-Output "Windie home ready at $windieHome"
Write-Output "provider keys file: $envFile"
Write-Output "Bifrost: http://$gatewayAddress"
Write-Output "Windie API: http://$apiAddress"
Write-Output "Inspector: $uiUrl"
