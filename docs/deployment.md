# Deployment

## Installers

`npm run build` produces both:

| File | Use |
|---|---|
| `target/release/bundle/msi/Keeprun_<v>_x64_en-US.msi` | Intune, SCCM, Group Policy |
| `target/release/bundle/nsis/Keeprun_<v>_x64-setup.exe` | Manual installs |

Both install per machine (Administrator rights needed), register the `KeeprunAgent` service,
add the install folder to the machine PATH and start the service.

## Silent install

```powershell
# MSI. KEEPRUN_USER grants a user access to the desktop app (default: the installing user).
# KEEPRUN_PORT sets the agent port (default 7171). Leave it out to keep the current port.
msiexec /i Keeprun_0.1.0_x64_en-US.msi /qn KEEPRUN_USER=CONTOSO\jdoe KEEPRUN_PORT=7272

# NSIS
Keeprun_0.1.0_x64-setup.exe /S /PORT=7272
```

Repair reinstalls the files and registers the service again: run the MSI again and choose
**Repair**, use **Repair** in Apps and features, or run `msiexec /fa <file or product code> /qn`.

Uninstall with `msiexec /x <file or product code> /qn` or `"%ProgramFiles%\Keeprun\uninstall.exe" /S`.
Uninstall removes the service and the PATH entry. Apps, events, logs and the token in
`C:\ProgramData\Keeprun` are kept, so a reinstall picks up where it left off.

### Intune (Win32 app)

- Install command: `msiexec /i Keeprun_0.1.0_x64_en-US.msi /qn`
- Uninstall command: `msiexec /x {product-code} /qn`
- Detection rule: MSI product code, or registry `HKLM\Software\Keeprun` value `InstallDir`.
- Install behavior: System.

## Test an installer

On a test machine or VM, from an Administrator terminal:

```powershell
npm run smoke:install
```

The script installs the MSI silently, checks the service, token permissions, PATH, the API on port 7171 and the CLI, runs a sample app, then uninstalls and checks cleanup. Pass `-KeepInstalled` to skip the uninstall.

## Upgrades

Install the new version over the old one. The installer stops the service (apps stop with it),
replaces the files and starts the service again. Apps that were running start again. The
database is migrated automatically on first start.

## Who can use the desktop app and CLI

The agent API needs the token in `C:\ProgramData\Keeprun\agent.token`. Readable by:

- SYSTEM and Administrators,
- the local group **Keeprun Users** (created by the installer),
- the user who ran the installer.

Give someone access:

```powershell
net localgroup "Keeprun Users" CONTOSO\jdoe /add
```

Group membership applies at their next sign-in. Remove them from the group to revoke access.

## Configuration

`C:\ProgramData\Keeprun\config.toml` (written on first install, kept on upgrade):

```toml
host = "127.0.0.1"
port = 7171
log_level = "keeprun=info,warn"
stop_timeout_ms = 10000
```

Restart the **Keeprun Agent** service after editing. See [configuration.md](configuration.md).

### Change the port

From an Administrator terminal:

```powershell
keeprun-agent set-port 7272
```

It checks that the port is free, saves it to `config.toml` and restarts the service. Apps that
were running start again. The desktop app and CLI read the same file, so they follow without
changes. In the desktop app, the same is available from the server button in the header
(Windows asks for administrator approval).

## Code signing

Unsigned agents running as SYSTEM are often flagged by antivirus. Sign every release.
`scripts/build.ps1` passes `KEEPRUN_SIGN_COMMAND` to Tauri's `signCommand`, which signs the
desktop app, the sidecar binaries and the installers. `%1` is the file to sign.

Azure Trusted Signing:

```powershell
cargo install trusted-signing-cli
$env:AZURE_CLIENT_ID = "..."; $env:AZURE_CLIENT_SECRET = "..."; $env:AZURE_TENANT_ID = "..."
$env:KEEPRUN_SIGN_COMMAND = "trusted-signing-cli -e https://weu.codesigning.azure.net -a <account> -c <profile> %1"
npm run build
```

A local certificate works the same way with `signtool sign /fd SHA256 /tr <timestamp-url> /td SHA256 /n "<subject>" %1`.

## Updates

The desktop app checks for a new version 10 seconds after it starts and every 6 hours. When
one exists, an **Update** button appears in the header. Updating downloads the installer,
checks its signature, and runs it after administrator approval. The installer updates the
desktop app, the agent service and the CLI, then restarts the service. Apps that were running
start again. An MSI install is updated with the MSI, an NSIS install with the setup.exe.

Nothing installs without someone clicking **Install and restart**. The agent service never
updates itself.

### Turn update checks off, or use a mirror

Fleets that roll out versions through Intune or SCCM usually turn the check off. Two policy
values under `HKLM\SOFTWARE\Policies\Keeprun` control it. Set them with Group Policy,
Intune, or at install time:

| Value | Type | Effect | Install option |
|---|---|---|---|
| `DisableUpdates` | DWORD `1` | No checks, no Update button | MSI `KEEPRUN_DISABLE_UPDATES=1`, NSIS `/NOUPDATES` |
| `UpdateUrl` | String | Read the manifest from this URL instead of `https://keeprun.dev/latest.json` | MSI `KEEPRUN_UPDATE_URL=https://...` |

```powershell
msiexec /i Keeprun_0.2.0_x64_en-US.msi /qn KEEPRUN_DISABLE_UPDATES=1
```

A mirror serves a copy of `latest.json` with the `url` fields changed to point at the mirrored
installers. Keep the `signature` fields as they are: the desktop app rejects installers whose
signature does not match the key built into it.
