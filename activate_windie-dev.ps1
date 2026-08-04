# Dot-source this file in PowerShell: . .\activate_windie-dev.ps1
$windieDevRoot = (Resolve-Path (Join-Path $PSScriptRoot ".")).Path
cargo build --release --manifest-path (Join-Path $windieDevRoot "Cargo.toml") --bin windie-dev
$env:WINDIE_DEV_ROOT = $windieDevRoot
$env:Path = "$(Join-Path $windieDevRoot 'target\release');$env:Path"
Write-Host "windie-dev active: $(Join-Path $windieDevRoot 'target\release\windie-dev.exe')"
