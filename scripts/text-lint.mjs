// Fails when UI strings, docs or source contain emojis or em dashes (see PLAN.md, section 2).
// Usage: node scripts/text-lint.mjs [--commit-msg <file>]
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, extname } from "node:path";

const root = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const exts = new Set([".rs", ".ts", ".js", ".mjs", ".svelte", ".html", ".css", ".md", ".toml", ".json", ".ps1", ".yml"]);
const skip = new Set(["node_modules", "target", "dist", "gen", ".git", ".dev-data", "icons"]);
const banned = [
  { re: new RegExp(String.fromCharCode(0x2014), "g"), why: "em dash" },
  { re: /\p{Extended_Pictographic}/gu, why: "emoji" },
];

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    if (skip.has(name) || name === "package-lock.json") continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* walk(p);
    else if (exts.has(extname(name))) yield p;
  }
}

// The root README is the marketing page: emojis are allowed there, em dashes are not.
const emojiAllowed = new Set(["README.md"]);

function check(label, text) {
  const out = [];
  const allowEmoji = emojiAllowed.has(label);
  text.split(/\r?\n/).forEach((line, i) => {
    for (const { re, why } of banned) {
      if (allowEmoji && why === "emoji") continue;
      re.lastIndex = 0;
      if (re.test(line)) out.push(`${label}:${i + 1}: ${why}: ${line.trim().slice(0, 100)}`);
    }
  });
  return out;
}

const args = process.argv.slice(2);
const problems = [];
const msgIdx = args.indexOf("--commit-msg");
if (msgIdx >= 0) {
  problems.push(...check("commit message", readFileSync(args[msgIdx + 1], "utf8")));
} else {
  for (const f of walk(root)) problems.push(...check(relative(root, f), readFileSync(f, "utf8")));
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} problem(s). Replace em dashes with periods, commas or hyphens. Remove emojis.`);
  process.exit(1);
}
console.log("Text lint passed.");
