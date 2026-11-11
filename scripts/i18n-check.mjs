// Checks the desktop UI translations against the English catalog: same keys, same
// {placeholders} in every string and plural form. Missing keys are also a type error, but a
// dropped or renamed placeholder is not, and it shows the raw {name} to users.
// Usage: node scripts/i18n-check.mjs
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const root = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const dir = join(root, "apps/desktop/src/lib/i18n");
const ts = createRequire(join(root, "apps/desktop/package.json"))("typescript");

/** Evaluate a catalog module: strip types, then read its single object literal. */
function load(file) {
  const js = ts.transpileModule(readFileSync(join(dir, file), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const module = { exports: {} };
  new Function("module", "exports", "require", js)(module, module.exports, () => ({}));
  return module.exports.default ?? module.exports.en;
}

const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
const forms = (v) => (typeof v === "string" ? [["", v]] : Object.entries(v));

const en = load("en.ts");
const problems = [];
const files = readdirSync(dir).filter((f) => f.endsWith(".ts") && f !== "en.ts" && !f.startsWith("index"));

for (const file of files) {
  const cat = load(file);
  for (const key of Object.keys(en)) {
    if (!(key in cat)) {
      problems.push(`${file}: missing ${key}`);
      continue;
    }
    // Plural forms may drop {count} (Arabic "zero", for example), but no form may add or
    // misspell a placeholder, and the set across all forms must match English.
    const want = new Set(forms(en[key]).flatMap(([, v]) => placeholders(v).split(",").filter(Boolean)));
    const seen = new Set();
    for (const [form, text] of forms(cat[key])) {
      if (typeof text !== "string" || !text.trim()) problems.push(`${file}: ${key}${form && "." + form} is empty`);
      for (const p of placeholders(text ?? "").split(",").filter(Boolean)) {
        seen.add(p);
        if (!want.has(p)) problems.push(`${file}: ${key}${form && "." + form} has unknown {${p}}`);
      }
    }
    for (const p of want) {
      if (!seen.has(p) && !(p === "count" && typeof en[key] !== "string")) problems.push(`${file}: ${key} lost {${p}}`);
    }
    if (typeof en[key] !== "string" && typeof cat[key] === "string") problems.push(`${file}: ${key} must be plural forms`);
  }
  for (const key of Object.keys(cat)) if (!(key in en)) problems.push(`${file}: unknown key ${key}`);
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} translation problem(s).`);
  process.exit(1);
}
console.log(`Translations OK: ${files.length} languages, ${Object.keys(en).length} strings.`);
