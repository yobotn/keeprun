// Build the website into _site/: the landing page, docs rendered from docs/*.md, and the
// update manifest of the latest published release.
//
//   npm install --no-save marked@15
//   node scripts/build-site.mjs
//
// Env (set by .github/workflows/pages.yml):
//   GITHUB_REPOSITORY  owner/repo, used in links
//   KEEPRUN_TAG        latest published release tag, e.g. v0.2.0. Empty before the first release.
//   KEEPRUN_MANIFEST   path to that release's latest.json, if it has one
import { copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const root = fileURLToPath(new URL("..", import.meta.url));
const out = join(root, "_site");
const repo = process.env.GITHUB_REPOSITORY || "keeprun/keeprun";
const tag = process.env.KEEPRUN_TAG || "";
const version = tag.replace(/^v/, "");

rmSync(out, { recursive: true, force: true });
mkdirSync(join(out, "docs"), { recursive: true });

const releases = `https://github.com/${repo}/releases`;
const asset = (name) => (tag ? `${releases}/download/${tag}/${name}` : releases);
const vars = {
  REPO: repo,
  VERSION: version || "x.y.z",
  VERSION_LINE: version ? `Version ${version}.` : "First release coming soon.",
  VERSION_PILL: version ? `Keeprun ${version} is out` : "Open source on GitHub",
  MSI_URL: asset(`Keeprun_${version}_x64_en-US.msi`),
  EXE_URL: asset(`Keeprun_${version}_x64-setup.exe`),
};
// Lucide icons (and the GitHub mark), inlined as SVG: {{icon:name}}.
const icons = JSON.parse(readFileSync(join(root, "site", "icons.json"), "utf8"));
function icon(name) {
  const i = icons[name];
  if (!i) throw new Error(`Unknown icon "${name}". Add it to scripts/gen-site-icons.mjs.`);
  if (typeof i === "string") {
    return `<svg class="i" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${i}</svg>`;
  }
  return `<svg class="i" viewBox="${i.viewBox}" fill="currentColor" aria-hidden="true">${i.body}</svg>`;
}
const fill = (text) =>
  text
    .replace(/\{\{icon:([\w-]+)\}\}/g, (_, name) => icon(name))
    .replace(/\{\{(\w+)\}\}/g, (m, k) => (k in vars ? vars[k] : m));

// Landing page and static files.
writeFileSync(join(out, "index.html"), fill(readFileSync(join(root, "site", "index.html"), "utf8")));
copyFileSync(join(root, "site", "style.css"), join(out, "style.css"));
copyFileSync(join(root, "apps", "desktop", "app-icon.svg"), join(out, "logo.svg"));
writeFileSync(join(out, ".nojekyll"), "");
// Screenshots live in docs/assets so the README on GitHub can show them too.
cpSync(join(root, "docs", "assets"), join(out, "assets"), { recursive: true });

// Docs. Order of the side menu; other files follow alphabetically.
const order = ["getting-started", "deployment", "configuration", "cli", "api", "security", "troubleshooting", "roadmap", "releasing"];
const files = readdirSync(join(root, "docs"))
  .filter((f) => f.endsWith(".md"))
  .map((f) => f.slice(0, -3))
  .sort((a, b) => (order.indexOf(a) + 1 || 99) - (order.indexOf(b) + 1 || 99) || a.localeCompare(b));
const pages = files.map((name) => {
  const md = readFileSync(join(root, "docs", `${name}.md`), "utf8");
  const title = md.match(/^#\s+(.+)$/m)?.[1] ?? name;
  return { name, title, md };
});
const template = readFileSync(join(root, "site", "docs.html"), "utf8");
for (const p of pages) {
  const menu = pages
    .map((q) => `<a href="${q.name}.html"${q.name === p.name ? ' aria-current="page"' : ""}>${q.title}</a>`)
    .join("\n        ");
  // Links between docs point at .md files in the repo. On the site they are .html pages.
  const body = marked
    .parse(p.md)
    .replace(/href="(?!https?:)([\w-]+)\.md(#[\w-]*)?"/g, 'href="$1.html$2"')
    .replace(/href="(?!https?:|#)([^"]+\.md)"/g, `href="https://github.com/${repo}/blob/main/$1"`);
  const html = fill(template)
    .replaceAll("%TITLE%", p.title)
    .replace("%MENU%", () => menu)
    .replace("%BODY%", () => body);
  writeFileSync(join(out, "docs", `${p.name}.html`), html);
}

// Update manifest for the desktop updater (https://<domain>/latest.json).
const manifest = process.env.KEEPRUN_MANIFEST;
if (manifest && existsSync(manifest)) {
  copyFileSync(manifest, join(out, "latest.json"));
  console.log(`Included latest.json for ${tag}.`);
} else {
  console.log("No release manifest. The updater sees no update until a release is published.");
}
console.log(`Built ${pages.length} docs pages into ${out}.`);
