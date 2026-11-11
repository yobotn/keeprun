# Releasing

How a version gets from a git tag to users, and the one-time setup it needs.

## How it works

1. Push a tag `vX.Y.Z`. The **Release** workflow runs the CI checks on the tagged commit and
   builds the MSI and the setup.exe, signed for the updater, with `latest.json`.
2. It verifies the release before anything is public: the tag matches the version, `latest.json`
   points at this release, and every update signature verifies against the public key the
   desktop app ships with (`scripts/verify-release.mjs`).
3. On a clean Windows runner it installs the MSI and the setup.exe silently, checks the service,
   API, CLI and version, and uninstalls (`scripts/smoke-install.ps1`, `scripts/smoke-setup.ps1`).
4. Only when all of that passes does it publish the GitHub release and run the **Website**
   workflow, which rebuilds the site with the new download links and copies the release's
   `latest.json` to `https://keeprun.app/latest.json`. If a check fails, nothing is published.
5. Desktop apps find the update on their next check (within 6 hours, or at the next start).

To build and test installers without releasing, run the **Release** workflow by hand.

To stop a rollout, delete the release or mark it as a pre-release, then run the Website workflow
by hand. The site falls back to the previous published release.

## Version number

The version lives in one place: `version` under `[workspace.package]` in the root `Cargo.toml`.
The agent, the CLI and the desktop app all read it. Bump it, commit, then tag:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

The tag must match the version, or the manifest points at files that do not exist.

## One-time setup

### Update signing key

The desktop app only installs updates signed with the key whose public half is in
`apps/desktop/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). This is separate from
Windows code signing and costs nothing.

The private key is at `%USERPROFILE%\.tauri\keeprun-updater.key` on the machine where it was
generated. Add it to the repository secrets:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Empty, unless you generated a key with a password |

Keep a backup in your password manager. If the key is lost, installed copies cannot verify any
new version, and every user has to install the next version by hand. To rotate the key, ship
one release signed with the old key that contains the new public key.

Local builds sign too when the variable is set:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.tauri\keeprun-updater.key"
npm run build
```

### Code signing (optional, recommended)

Unsigned installers trigger SmartScreen warnings. Set `KEEPRUN_SIGN_COMMAND` (and the Azure
secrets for Trusted Signing) as described in [deployment.md](deployment.md#code-signing).

### Website and domain

1. In the repository, open **Settings > Pages** and set **Source** to **GitHub Actions**.
2. Under **Custom domain**, enter `keeprun.app` and save. Turn on **Enforce HTTPS** once the
   certificate is issued.
3. At the DNS provider, point the apex domain at GitHub Pages:
   - `A` records: `185.199.108.153`, `185.199.109.153`, `185.199.110.153`, `185.199.111.153`
   - `AAAA` records: `2606:50c0:8000::153`, `2606:50c0:8001::153`, `2606:50c0:8002::153`, `2606:50c0:8003::153`
   - Optional `CNAME` for `www`: `<owner>.github.io`
4. Verify the domain for your account or organization under **Settings > Pages > Verified
   domains**, so no other repository can claim it.
5. Run the **Website** workflow once by hand.

The site is built from `site/` (landing page) and `docs/` (rendered to `/docs/*.html`). Build it
locally with:

```powershell
npm install --no-save marked@15
node scripts/build-site.mjs   # output in _site/
```

### Using another domain

The domain appears in three places. Change all of them:

- `apps/desktop/src-tauri/tauri.conf.json`: `plugins.updater.endpoints`
- `docs/deployment.md` and this file
- The custom domain in **Settings > Pages**

Installed copies keep checking the old URL until they update, so keep the old domain serving
`latest.json` for a while after a change.
