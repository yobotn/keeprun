# Installer check for the NSIS setup.exe. Run from an Administrator terminal on a test machine or VM.
# Installs silently, checks the service, API and CLI, then uninstalls and checks the service is gone.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\smoke-setup.ps1 [-Setup path] [-Version x.y.z]
param(
    [string]$Setup = (Get-ChildItem "$PSScriptRoot\..\target\release\bundle\nsis\*-setup.exe" | Select-Object -First 1).FullName,
    # When set, the running agent must report this version.
    [string]$Version
)
$ErrorActionPreference = 'Stop'
$failed = 0
function Check($name, [scriptblock]$test) {
    try { if (& $test) { Write-Host "PASS  $name"; return } } catch { }
    Write-Host "FAIL  $name" -ForegroundColor Red
    $script:failed++
}

$admin = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'Run this script from an Administrator terminal.' }
if (-not $Setup -or -not (Test-Path $Setup)) { throw 'setup.exe not found. Run npm run build first.' }
$ErrorActionPreference = 'Continue'

Write-Host "Installing $Setup"
$p = Start-Process $Setup -ArgumentList '/S' -Wait -PassThru
Check 'setup.exe /S exit code 0' { $p.ExitCode -eq 0 }

$dir = Join-Path $env:ProgramFiles 'Keeprun'
Check 'Files installed' { (Test-Path "$dir\keeprun-agent.exe") -and (Test-Path "$dir\keeprun.exe") -and (Test-Path "$dir\keeprun-desktop.exe") }
Check 'Service KeeprunAgent running' { (Get-Service KeeprunAgent).Status -eq 'Running' }

$ok = $false
for ($i = 0; $i -lt 20 -and -not $ok; $i++) {
    try { $h = Invoke-RestMethod http://127.0.0.1:7171/health; $ok = $h.ok } catch { Start-Sleep -Milliseconds 500 }
}
Check 'API /health on 7171 reports prod' { $ok -and $h.env -eq 'prod' }
if ($Version) { Check "Agent reports version $Version" { $ok -and $h.version -eq $Version } }
Check 'CLI status' { & "$dir\keeprun.exe" status *> $null; $LASTEXITCODE -eq 0 }

Write-Host 'Uninstalling'
# _?= runs the uninstaller in place, so Start-Process waits for it to finish.
$u = Start-Process "$dir\uninstall.exe" -ArgumentList '/S', "_?=$dir" -Wait -PassThru
Check 'Uninstaller exit code 0' { $u.ExitCode -eq 0 }
Check 'Service removed' { -not (Get-Service KeeprunAgent -ErrorAction SilentlyContinue) }

if ($failed) { Write-Host "$failed check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed.'
