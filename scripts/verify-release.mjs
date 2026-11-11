// Check the release files before they are published:
//   - the tag matches the workspace version in Cargo.toml
//   - latest.json has that version and points at the installers of this tag
//   - each signature in latest.json is the matching .sig file
//   - each .sig verifies against the updater public key in tauri.conf.json
// A release that fails here would install, but installed copies could not update to it.
//
// Env: GITHUB_REPOSITORY (owner/repo), KEEPRUN_TAG (default v<version>).
// Usage: node scripts/verify-release.mjs [bundle folder]
import { createHash, createPublicKey, verify } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const bundle = process.argv[2] || join(root, "target", "release", "bundle");
const cargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const version = cargo.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s)?.[1];
const repo = process.env.GITHUB_REPOSITORY || "keeprun/keeprun";
const tag = process.env.KEEPRUN_TAG || `v${version}`;

let failed = 0;
function check(name, ok, detail = "") {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${ok || !detail ? "" : `: ${detail}`}`);
  if (!ok) failed++;
}

// Minisign as used by Tauri. Keys and signatures are base64 of the minisign text files.
const b64 = (s) => Buffer.from(s.trim(), "base64");
const lines = (s) => b64(s).toString("utf8").split("\n").filter(Boolean);

function publicKey() {
  const conf = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
  const raw = b64(lines(conf.plugins.updater.pubkey)[1]); // "Ed" + key id (8) + key (32)
  const key = createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: raw.subarray(10).toString("base64url") }, format: "jwk" });
  return { id: raw.subarray(2, 10), key };
}

function verifySignature(file, sigText, pub) {
  const [, sigLine, trusted, globalLine] = lines(sigText);
  const sig = b64(sigLine); // "ED" or "Ed" + key id (8) + signature (64)
  const alg = sig.subarray(0, 2).toString("latin1");
  if (!sig.subarray(2, 10).equals(pub.id)) return "signed with a different key";
  const data = readFileSync(file);
  const msg = alg === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, msg, pub.key, sig.subarray(10))) return "signature does not match the file";
  const comment = Buffer.from(trusted.replace(/^trusted comment: /, ""), "utf8");
  if (!verify(null, Buffer.concat([sig.subarray(10), comment]), pub.key, b64(globalLine))) return "trusted comment signature is invalid";
  return "";
}

check(`Tag ${tag} matches Cargo.toml version ${version}`, tag === `v${version}`);

const manifestPath = join(bundle, "latest.json");
if (!existsSync(manifestPath)) {
  check("latest.json exists", false, manifestPath);
} else {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  check(`latest.json version is ${version}`, manifest.version === version, manifest.version);
  const pub = publicKey();
  const prefix = `https://github.com/${repo}/releases/download/${tag}/`;
  for (const [platform, entry] of Object.entries(manifest.platforms || {})) {
    const name = decodeURIComponent(entry.url.slice(prefix.length));
    const okUrl = entry.url.startsWith(prefix) && basename(name) === name;
    check(`${platform}: URL points at this release`, okUrl, entry.url);
    if (!okUrl) continue;
    const file = join(bundle, name.endsWith(".msi") ? "msi" : "nsis", name);
    if (!existsSync(file) || !existsSync(`${file}.sig`)) {
      check(`${platform}: ${name} and its .sig exist`, false);
      continue;
    }
    const sigText = readFileSync(`${file}.sig`, "utf8");
    check(`${platform}: signature equals ${name}.sig`, entry.signature === sigText.trim());
    const problem = verifySignature(file, sigText, pub);
    check(`${platform}: ${name} verifies with the updater key`, !problem, problem);
  }
  check("latest.json has MSI and NSIS entries", ["windows-x86_64-msi", "windows-x86_64-nsis", "windows-x86_64"].every((p) => manifest.platforms?.[p]));
}

if (failed) {
  console.error(`${failed} check(s) failed.`);
  process.exit(1);
}
console.log("Release files verified.");
