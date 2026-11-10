# End to end installer check. Run from an Administrator terminal on a test machine or VM.
# Installs the MSI silently, checks the service, API and CLI, then uninstalls and checks cleanup.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\smoke-install.ps1 [-Msi path] [-KeepInstalled]
param(
    [string]$Msi = (Get-ChildItem "$PSScriptRoot\..\target\release\bundle\msi\*.msi" | Select-Object -First 1).FullName,
    [switch]$KeepInstalled
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
if (-not $Msi -or -not (Test-Path $Msi)) { throw 'MSI not found. Run npm run build first.' }

$log = Join-Path $env:TEMP 'keeprun-install.log'
Write-Host "Installing $Msi"
$p = Start-Process msiexec -ArgumentList "/i `"$Msi`" /qn /l*v `"$log`"" -Wait -PassThru
Check "msiexec install exit code 0 (log: $log)" { $p.ExitCode -eq 0 }

$dir = (Get-ItemProperty 'HKLM:\Software\Keeprun' -ErrorAction SilentlyContinue).InstallDir
if (-not $dir) { $dir = Join-Path $env:ProgramFiles 'Keeprun' }
Check 'Files installed' { (Test-Path "$dir\keeprun-agent.exe") -and (Test-Path "$dir\keeprun.exe") -and (Test-Path "$dir\keeprun-desktop.exe") }
Check 'Service KeeprunAgent running' { (Get-Service KeeprunAgent).Status -eq 'Running' }
Check 'Service start type is Automatic' { (Get-Service KeeprunAgent).StartType -eq 'Automatic' }
Check 'Token file exists' { Test-Path "$env:ProgramData\Keeprun\token" }
Check 'Token not readable by Everyone or Users' { -not ((icacls "$env:ProgramData\Keeprun\token") -match 'Everyone|BUILTIN\\Users') }
Check 'Keeprun Users group exists' { net localgroup 'Keeprun Users' *> $null; $LASTEXITCODE -eq 0 }
Check 'Install folder on machine PATH' { ([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $dir.TrimEnd('\') }

$ok = $false
for ($i = 0; $i -lt 20 -and -not $ok; $i++) {
    try { $h = Invoke-RestMethod http://127.0.0.1:7171/health; $ok = $h.ok } catch { Start-Sleep -Milliseconds 500 }
}
Check 'API /health on 7171 reports prod' { $ok -and $h.env -eq 'prod' }
Check 'CLI status' { & "$dir\keeprun.exe" status *> $null; $LASTEXITCODE -eq 0 }

# Add, start and stop a real app through the installed CLI.
$app = Join-Path $env:TEMP 'keeprun-smoke-app'
New-Item -ItemType Directory -Force $app | Out-Null
Set-Content "$app\run.cmd" "@echo smoke ok`r`n@ping -n 600 127.0.0.1 > nul"
& "$dir\keeprun.exe" rm smoke-app *> $null
Check 'CLI add and start app' { & "$dir\keeprun.exe" add smoke-app --cwd $app --start -- run.cmd *> $null; $LASTEXITCODE -eq 0 }
Start-Sleep -Seconds 2
Check 'App is running' { (& "$dir\keeprun.exe" --json list | Out-String | ConvertFrom-Json | ForEach-Object { $_ } | Where-Object id -eq 'smoke-app').state -eq 'running' }
Check 'App logs captured' { (& "$dir\keeprun.exe" logs smoke-app -n 20) -match 'smoke ok' }
Check 'CLI stop app' { & "$dir\keeprun.exe" stop smoke-app *> $null; $LASTEXITCODE -eq 0 }
& "$dir\keeprun.exe" rm smoke-app *> $null

if (-not $KeepInstalled) {
    Write-Host 'Uninstalling'
    $u = Start-Process msiexec -ArgumentList "/x `"$Msi`" /qn /l*v `"$log.uninstall`"" -Wait -PassThru
    Check 'msiexec uninstall exit code 0' { $u.ExitCode -eq 0 }
    Check 'Service removed' { -not (Get-Service KeeprunAgent -ErrorAction SilentlyContinue) }
    Check 'PATH entry removed' { -not (([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $dir.TrimEnd('\')) }
    Check 'Data folder kept' { Test-Path "$env:ProgramData\Keeprun" }
}

Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
if ($failed) { Write-Host "$failed check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed.'
