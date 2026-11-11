# End to end installer check. Run from an Administrator terminal on a test machine or VM.
# Installs the MSI silently, checks the service, API and CLI, then uninstalls and checks cleanup.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\smoke-install.ps1 [-Msi path] [-Version x.y.z] [-KeepInstalled]
param(
    [string]$Msi = (Get-ChildItem "$PSScriptRoot\..\target\release\bundle\msi\*.msi" | Select-Object -First 1).FullName,
    # When set, the running agent must report this version.
    [string]$Version,
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
# From here on, report failures as FAIL lines instead of stopping. Windows PowerShell 5.1 turns
# stderr from native commands into errors, which would end the script under 'Stop'.
$ErrorActionPreference = 'Continue'

# A previous build with the same version blocks the install (error 1638). Remove it first.
$old = Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall' |
    ForEach-Object { Get-ItemProperty $_.PSPath } |
    Where-Object { $_.DisplayName -eq 'Keeprun' -and $_.PSChildName -like '{*}' }
foreach ($o in $old) {
    Write-Host "Removing installed Keeprun $($o.DisplayVersion) ($($o.PSChildName))"
    Start-Process msiexec -ArgumentList "/x $($o.PSChildName) /qn" -Wait | Out-Null
}

$log = Join-Path $env:TEMP 'keeprun-install.log'
Write-Host "Installing $Msi"
# KEEPRUN_DISABLE_UPDATES checks the policy component. Uninstall removes it again.
$p = Start-Process msiexec -ArgumentList "/i `"$Msi`" /qn KEEPRUN_DISABLE_UPDATES=1 /l*v `"$log`"" -Wait -PassThru
Check "msiexec install exit code 0 (log: $log)" { $p.ExitCode -eq 0 }

$dir = (Get-ItemProperty 'HKLM:\Software\Keeprun' -ErrorAction SilentlyContinue).InstallDir
if (-not $dir) { $dir = Join-Path $env:ProgramFiles 'Keeprun' }
Check 'Files installed' { (Test-Path "$dir\keeprun-agent.exe") -and (Test-Path "$dir\keeprun.exe") -and (Test-Path "$dir\keeprun-desktop.exe") }
Check 'Service KeeprunAgent running' { (Get-Service KeeprunAgent).Status -eq 'Running' }
Check 'Service start type is Automatic' { (Get-Service KeeprunAgent).StartType -eq 'Automatic' }
$token = "$env:ProgramData\Keeprun\agent.token"
Check 'Token file exists' { Test-Path $token }
Check 'Token not readable by Everyone or Users' { (Test-Path $token) -and -not ((icacls $token) -match 'Everyone|BUILTIN\\Users') }
Check 'Keeprun Users group exists' { net localgroup 'Keeprun Users' *> $null; $LASTEXITCODE -eq 0 }
Check 'Update policy written' { (Get-ItemProperty 'HKLM:\SOFTWARE\Policies\Keeprun' -ErrorAction Stop).DisableUpdates -eq 1 }
Check 'Install folder on machine PATH' { ([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $dir.TrimEnd('\') }

$ok = $false
for ($i = 0; $i -lt 20 -and -not $ok; $i++) {
    try { $h = Invoke-RestMethod http://127.0.0.1:7171/health; $ok = $h.ok } catch { Start-Sleep -Milliseconds 500 }
}
Check 'API /health on 7171 reports prod' { $ok -and $h.env -eq 'prod' }
if ($Version) { Check "Agent reports version $Version" { $ok -and $h.version -eq $Version } }
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
Check 'Task runs and reports exit code' { & "$dir\keeprun.exe" run smoke-app -- exit /b 3 *> $null; $LASTEXITCODE -eq 3 }
Check 'CLI stop app' { & "$dir\keeprun.exe" stop smoke-app *> $null; $LASTEXITCODE -eq 0 }

# Port change: the service restarts on the new port and the CLI follows the config file.
Check 'set-port 7272' { & "$dir\keeprun-agent.exe" set-port 7272 *> $null; $LASTEXITCODE -eq 0 }
$ok = $false
for ($i = 0; $i -lt 20 -and -not $ok; $i++) {
    try { $ok = (Invoke-RestMethod http://127.0.0.1:7272/health).ok } catch { Start-Sleep -Milliseconds 500 }
}
Check 'API answers on 7272' { $ok }
Check 'CLI follows the new port' { & "$dir\keeprun.exe" list *> $null; $LASTEXITCODE -eq 0 }
& "$dir\keeprun-agent.exe" set-port 7171 *> $null
& "$dir\keeprun.exe" rm smoke-app *> $null

$r = Start-Process msiexec -ArgumentList "/fa `"$Msi`" /qn /l*v `"$log.repair`"" -Wait -PassThru
Check 'MSI repair exit code 0' { $r.ExitCode -eq 0 }
Start-Sleep -Seconds 2
Check 'Service running after repair' { (Get-Service KeeprunAgent).Status -eq 'Running' }

if (-not $KeepInstalled) {
    Write-Host 'Uninstalling'
    $u = Start-Process msiexec -ArgumentList "/x `"$Msi`" /qn /l*v `"$log.uninstall`"" -Wait -PassThru
    Check 'msiexec uninstall exit code 0' { $u.ExitCode -eq 0 }
    Check 'Service removed' { -not (Get-Service KeeprunAgent -ErrorAction SilentlyContinue) }
    Check 'PATH entry removed' { -not (([Environment]::GetEnvironmentVariable('Path', 'Machine') -split ';') -contains $dir.TrimEnd('\')) }
    Check 'Update policy removed' { -not (Get-ItemProperty 'HKLM:\SOFTWARE\Policies\Keeprun' -ErrorAction SilentlyContinue).DisableUpdates }
    Check 'Data folder kept' { Test-Path "$env:ProgramData\Keeprun" }
}

Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
if ($failed) { Write-Host "$failed check(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed.'
