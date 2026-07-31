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

$env:WINDIE_BIFROST_BIN = Join-Path $installDir "bifrost.exe"
try {
    Invoke-WebRequest -Uri $gatewayHealthUrl -UseBasicParsing -TimeoutSec 2 | Out-Null
}
catch {
    & $windie "gateway" "start"
}

try {
    Invoke-WebRequest -Uri $apiHealthUrl -UseBasicParsing -TimeoutSec 2 | Out-Null
}
catch {
    & $windie "api" "start"
}

$apiReady = $false
for ($i = 0; $i -lt 75; $i++) {
    try {
        Invoke-WebRequest -Uri $apiHealthUrl -UseBasicParsing -TimeoutSec 2 | Out-Null
        $apiReady = $true
        break
    }
    catch {
        Start-Sleep -Seconds 1
    }
}
if (-not $apiReady) {
    throw "Windie installed, but the local API did not start. Output: $windieHome\windie-api.log"
}

try {
    Invoke-WebRequest -Uri $inspectorHealthUrl -UseBasicParsing -TimeoutSec 2 | Out-Null
}
catch {
    & $windie "inspector" "start"
}

$inspectorReady = $false
for ($i = 0; $i -lt 30; $i++) {
    try {
        Invoke-WebRequest -Uri $inspectorHealthUrl -UseBasicParsing -TimeoutSec 2 | Out-Null
        $inspectorReady = $true
        break
    }
    catch {
        Start-Sleep -Seconds 1
    }
}
if (-not $inspectorReady) {
    throw "Windie installed, but the Inspector did not start. Output: $windieHome\windie-inspector.log"
}

$uiUrl = "http://$inspectorAddress"
Start-Process $uiUrl
Start-Process -FilePath $windie -ArgumentList @("tray")

Write-Output "windie installed at $(Join-Path $installDir 'windie.exe')"
Write-Output "Windie tray available as: $windie tray"
Write-Output "bundled Bifrost installed at $(Join-Path $installDir 'bifrost.exe')"
Write-Output "Inspector installed at $(Join-Path $installDir 'windie-inspector.exe')"
Write-Output "Windie home ready at $windieHome"
Write-Output "provider keys file: $envFile"
Write-Output "Bifrost: http://$gatewayAddress"
Write-Output "Windie API: http://$apiAddress"
Write-Output "Inspector: $uiUrl"
