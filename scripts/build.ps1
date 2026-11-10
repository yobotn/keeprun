# Build release (prod) binaries and the Windows installers.
#
#   npm run build                  agent, CLI, desktop app, MSI and NSIS installers
#   npm run build -- -SkipBundle   release binaries only
#
# Output:
#   target/release/keeprun-agent.exe, target/release/keeprun.exe
#   target/release/bundle/msi/Keeprun_<version>_x64_en-US.msi
#   target/release/bundle/nsis/Keeprun_<version>_x64-setup.exe
#
# Code signing: set KEEPRUN_SIGN_COMMAND to a command that signs the file passed as %1,
# e.g. trusted-signing-cli for Azure Trusted Signing. See docs/deployment.md.

param([switch]$SkipBundle)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Release builds default to prod. Make it explicit so a stray KEEPRUN_ENV cannot leak in.
$env:KEEPRUN_ENV = "prod"

cargo build --release -p keeprun-agent -p keeprun-cli
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if ($SkipBundle) { exit 0 }

# Tauri ships extra binaries ("sidecars") named with the target triple.
$triple = (rustc -vV | Select-String "^host: (.+)$").Matches[0].Groups[1].Value
$bin = "apps/desktop/src-tauri/binaries"
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item target/release/keeprun-agent.exe "$bin/keeprun-agent-$triple.exe" -Force
Copy-Item target/release/keeprun.exe "$bin/keeprun-$triple.exe" -Force

if (-not (Test-Path "apps/desktop/node_modules")) { npm --prefix apps/desktop install }

$configs = @("--config", "src-tauri/tauri.bundle.conf.json")
if ($env:KEEPRUN_SIGN_COMMAND) {
    $sign = @{ bundle = @{ windows = @{ signCommand = $env:KEEPRUN_SIGN_COMMAND } } } | ConvertTo-Json -Depth 5 -Compress
    $signFile = Join-Path $env:TEMP "keeprun-sign.conf.json"
    Set-Content -Path $signFile -Value $sign -Encoding utf8
    $configs += @("--config", $signFile)
}

Push-Location apps/desktop
try {
    npx tauri build @configs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Installers:"
Get-ChildItem target/release/bundle -Recurse -Include *.msi, *-setup.exe | ForEach-Object { Write-Host "  $($_.FullName)" }
