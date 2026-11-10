# Start the dev agent and the desktop UI together.
# The agent runs in this terminal's background job; closing the UI stops both.
#
#   npm run dev            agent (port 7181, data in .dev-data) + Tauri UI with hot reload
#   npm run dev -- -NoUi   agent only

param([switch]$NoUi)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:KEEPRUN_ENV = "dev"

if (-not (Test-Path "apps/desktop/node_modules")) {
    Write-Host "Installing UI dependencies..."
    npm --prefix apps/desktop install
}

Write-Host "Building agent..."
cargo build -p keeprun-agent -p keeprun-cli
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($NoUi) {
    & "$root/target/debug/keeprun-agent.exe" run --env dev
    exit $LASTEXITCODE
}

$agent = Start-Process -FilePath "$root/target/debug/keeprun-agent.exe" -ArgumentList "run", "--env", "dev" -NoNewWindow -PassThru
try {
    npm --prefix apps/desktop run tauri:dev
}
finally {
    if (-not $agent.HasExited) {
        Write-Host "Stopping dev agent..."
        # taskkill /T ends the agent; its Job Objects end every app it started.
        taskkill /PID $agent.Id /T /F | Out-Null
    }
}
