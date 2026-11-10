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
msiexec /i Keeprun_0.1.0_x64_en-US.msi /qn KEEPRUN_USER=CONTOSO\jdoe

# NSIS
Keeprun_0.1.0_x64-setup.exe /S
```

Uninstall with `msiexec /x <file or product code> /qn` or `"%ProgramFiles%\Keeprun\uninstall.exe" /S`.
Uninstall removes the service and the PATH entry. Apps, events, logs and the token in
`C:\ProgramData\Keeprun` are kept, so a reinstall picks up where it left off.

### Intune (Win32 app)

- Install command: `msiexec /i Keeprun_0.1.0_x64_en-US.msi /qn`
- Uninstall command: `msiexec /x {product-code} /qn`
- Detection rule: MSI product code, or registry `HKLM\Software\Keeprun` value `InstallDir`.
- Install behavior: System.

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

There is no built-in auto-updater on purpose: managed Windows fleets roll out software through
Intune, SCCM or WSUS-style tools, and a self-updating SYSTEM service is a change that IT teams
must approve. Ship each version as an MSI through your normal channel.
