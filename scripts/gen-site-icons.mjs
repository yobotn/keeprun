// Extract the Lucide icons the website uses into site/icons.json, from the copy the desktop
// app already depends on. Run after changing the list or upgrading @lucide/svelte:
//
//   node scripts/gen-site-icons.mjs
//
// The site build inlines them as SVG, so pages need no icon font or extra requests.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ICONS = [
  "arrow-right", "activity", "badge-check", "boxes", "building", "check", "circle-check",
  "cloud-off", "copy", "cpu", "download", "eye-off", "file-key", "folder-search", "gauge",
  "git-branch", "key-round", "list-checks", "list-tree", "lock-keyhole", "monitor",
  "network", "package", "plug-zap", "power", "refresh-cw", "rotate-ccw", "scroll-text",
  "server", "server-cog", "shield-check", "square-terminal", "terminal", "workflow", "zap",
];

const root = fileURLToPath(new URL("..", import.meta.url));
const dir = join(root, "apps", "desktop", "node_modules", "@lucide", "svelte", "dist", "icons");
const out = {};
for (const name of ICONS) {
  const src = readFileSync(join(dir, `${name}.svelte`), "utf8");
  const json = src.match(/__iconData = (\{.*?\});/s)?.[1];
  if (!json) throw new Error(`Cannot read icon data for ${name}.`);
  const nodes = JSON.parse(json).node;
  out[name] = nodes
    .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(" ")}/>`)
    .join("");
}
// Lucide has no brand marks. GitHub's own mark (Octicons, MIT), 16x16 and filled.
out["github"] = {
  viewBox: "0 0 16 16",
  fill: true,
  body: '<path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z"/>',
};
writeFileSync(join(root, "site", "icons.json"), JSON.stringify(out, null, 1) + "\n");
// The README cannot inline SVG, so it uses files. Green reads well on GitHub light and dark.
const svgDir = join(root, "docs", "assets", "icons");
mkdirSync(svgDir, { recursive: true });
for (const name of ICONS) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${out[name]}</svg>\n`;
  writeFileSync(join(svgDir, `${name}.svg`), svg);
}
console.log(`Wrote ${ICONS.length + 1} icons to site/icons.json (Lucide, ISC license).`);
