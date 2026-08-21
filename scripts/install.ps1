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
$gatewayUrl = if ($env:WINDIE_GATEWAY_URL) {
    $env:WINDIE_GATEWAY_URL.TrimEnd('/')
} elseif ($env:WINDIE_GATEWAY_PORT) {
    "http://127.0.0.1:$($env:WINDIE_GATEWAY_PORT)"
} else {
    "http://127.0.0.1:8080"
}
$apiAddress = if ($env:WINDIE_API_ADDRESS) {
    $env:WINDIE_API_ADDRESS
} elseif ($env:WINDIE_API_PORT) {
    "127.0.0.1:$($env:WINDIE_API_PORT)"
} else {
    "127.0.0.1:8787"
}
$inspectorAddress = if ($env:WINDIE_INSPECTOR_ADDRESS) {
    $env:WINDIE_INSPECTOR_ADDRESS
} elseif ($env:WINDIE_INSPECTOR_PORT) {
    "127.0.0.1:$($env:WINDIE_INSPECTOR_PORT)"
} else {
    "127.0.0.1:3000"
}
$env:WINDIE_GATEWAY_URL = $gatewayUrl
$env:WINDIE_API_ADDRESS = $apiAddress
$env:WINDIE_INSPECTOR_ADDRESS = $inspectorAddress

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "Windie requires a 64-bit Windows installation."
}

$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64" } else { "x86_64" }
$runtimeNames = @("node", "npx", "uv", "uvx")
Write-Output "Checking runtimes"
foreach ($runtime in $runtimeNames) {
    if (Get-Command $runtime -ErrorAction SilentlyContinue) {
        Write-Output "$runtime`: detected"
    } else {
        Write-Output "$runtime`: missing"
    }
}
Write-Output "missing runtimes will be installed and managed by Windie if the selected providers require them"

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
$gatewayHealthUrl = "$gatewayUrl/health"
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

    Write-WindieProgressBar 0
    for ($attempt = 0; $attempt -lt $Attempts; $attempt++) {
        if (Test-WindieHealth $Uri) {
            Write-WindieProgressBar 100
            Write-Host ""
            return
        }
        $progress = [Math]::Min(95, 5 + [Math]::Floor((($attempt + 1) * 90) / $Attempts))
        Write-WindieProgressBar $progress
        Start-Sleep -Seconds 1
    }
    Write-Host ""
    throw "Windie installed, but $Component did not start."
}

function Invoke-WindieLifecycle {
    param(
        [string[]]$Arguments,
        [string]$Component,
        [int]$TimeoutSeconds
    )

    $stdoutPath = Join-Path $windieHome "windie-installer-$Component.stdout.log"
    $stderrPath = Join-Path $windieHome "windie-installer-$Component.stderr.log"
    Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue

    $process = Start-Process -FilePath $windie -ArgumentList $Arguments -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru
    try {
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            throw "Windie $Component command timed out after $TimeoutSeconds seconds. Logs: $stdoutPath and $stderrPath"
        }

        $process.Refresh()
        $exitCode = $process.ExitCode
        if ($null -ne $exitCode -and "$exitCode" -ne "" -and [int]$exitCode -ne 0) {
            $details = @(
                if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw }
                if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw }
            ) -join "`n"
            $details = $details.Trim()
            if ($details) {
                throw "Windie $Component command failed with exit code $exitCode`: $details"
            }
            throw "Windie $Component command failed with exit code $exitCode. Logs: $stdoutPath and $stderrPath"
        }
    }
    finally {
        $process.Dispose()
    }
}

$env:WINDIE_BIFROST_BIN = Join-Path $installDir "bifrost.exe"

Write-Host "Installing LLM gateway"
if (-not (Test-WindieHealth $gatewayHealthUrl)) {
    Write-WindieProgressBar 5
    Invoke-WindieLifecycle @("gateway", "start") "gateway" 75
}
Wait-WindieHealth $gatewayHealthUrl 30 "the LLM gateway"
Write-Host "Started the gateway at $gatewayUrl"

Write-Host "Installing Windie runtime"
if (-not (Test-WindieHealth $apiHealthUrl)) {
    Write-WindieProgressBar 5
    Invoke-WindieLifecycle @("api", "start") "api" 30
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
    Write-WindieProgressBar 5
    Invoke-WindieLifecycle @("inspector", "start") "inspector" 30
}
Wait-WindieHealth $inspectorHealthUrl 30 "the Windie Inspector UI"

$uiUrl = "http://$inspectorAddress"
Start-Process $uiUrl
Start-Process -FilePath $windie -ArgumentList @("tray", "start") -WindowStyle Hidden
Start-Process -FilePath $windie -ArgumentList @("notifier", "start") -WindowStyle Hidden
Write-Host "Started the UI at $uiUrl"
Write-Host "Click on the tray on your desktop to manage these processes."

Write-Output "windie installed at $(Join-Path $installDir 'windie.exe')"
Write-Output "Windie tray available as: $windie tray start|stop|output"
Write-Output "Windie notifications available as: $windie notifier start|stop|output"
Write-Output "bundled Bifrost installed at $(Join-Path $installDir 'bifrost.exe')"
Write-Output "Inspector installed at $(Join-Path $installDir 'windie-inspector.exe')"
Write-Output "Windie home ready at $windieHome"
Write-Output "provider keys file: $envFile"
Write-Output "Bifrost: $gatewayUrl"
Write-Output "Windie API: http://$apiAddress"
Write-Output "Inspector: $uiUrl"
