// Write target/release/bundle/latest.json, the manifest the desktop updater reads.
//
// The installers and their .sig files are uploaded to the GitHub release for the tag, and the
// manifest points at them. The website serves the manifest of the latest published release,
// so publishing a release is what rolls an update out.
//
// Env: GITHUB_REPOSITORY (owner/repo), KEEPRUN_TAG (default v<version>),
//      KEEPRUN_NOTES (release notes, optional).
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const version = cargo.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s)?.[1];
if (!version) throw new Error("Cannot read the workspace version from Cargo.toml.");

const repo = process.env.GITHUB_REPOSITORY || "keeprun/keeprun";
const tag = process.env.KEEPRUN_TAG || `v${version}`;
const bundle = join(root, "target", "release", "bundle");

function artifact(dir, test) {
  const folder = join(bundle, dir);
  const file = existsSync(folder) && readdirSync(folder).find((f) => test(f) && f.includes(`_${version}_`));
  if (!file) throw new Error(`No ${dir} installer for ${version} in ${folder}. Run the build first.`);
  const sig = join(folder, `${file}.sig`);
  if (!existsSync(sig)) throw new Error(`Missing ${sig}. Set TAURI_SIGNING_PRIVATE_KEY when building.`);
  return {
    signature: readFileSync(sig, "utf8").trim(),
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(file)}`,
  };
}

const msi = artifact("msi", (f) => f.endsWith(".msi"));
const nsis = artifact("nsis", (f) => f.endsWith("-setup.exe"));

const manifest = {
  version,
  notes: process.env.KEEPRUN_NOTES || `Keeprun ${version}. See https://github.com/${repo}/releases/tag/${tag}`,
  pub_date: new Date().toISOString(),
  platforms: {
    // The updater picks the entry that matches how Keeprun was installed, so an MSI install
    // stays an MSI install. The plain key is the fallback.
    "windows-x86_64-msi": msi,
    "windows-x86_64-nsis": nsis,
    "windows-x86_64": msi,
  },
};

const out = join(bundle, "latest.json");
writeFileSync(out, JSON.stringify(manifest, null, 2) + "\n");
console.log(`Wrote ${out} for ${version} (${tag}).`);
