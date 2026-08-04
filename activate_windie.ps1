# Dot-source this file in PowerShell: . .\activate_windie.ps1
$windieRoot = (Resolve-Path (Join-Path $PSScriptRoot ".")).Path
if ($env:WINDIE_INSTALL_DIR) {
    $windieInstallDir = $env:WINDIE_INSTALL_DIR
} else {
    $windieInstallDir = Join-Path $windieRoot "target\local-installer\windows-x86_64\bin"
}
$windieExecutable = Join-Path $windieInstallDir "windie.exe"
if (-not (Test-Path -LiteralPath $windieExecutable -PathType Leaf)) {
    throw "local Windie binary not found at $windieExecutable; run windie-dev release install"
}
$env:Path = "$windieInstallDir;$env:Path"
Write-Host "windie active: $windieExecutable"
