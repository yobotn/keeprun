// Checks the built website in _site/: pages, Markdown copies, JSON-LD, llms.txt, sitemap,
// robots.txt and the OpenAPI spec. Run after the build:
//   node scripts/build-site.mjs && node --test scripts/site.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const out = fileURLToPath(new URL("../_site", import.meta.url));
const read = (p) => readFileSync(join(out, p), "utf8");
const site = "https://keeprun.app";
const sitemapUrls = [...read("sitemap.xml").matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]);
// URL on the site -> file in _site.
const fileFor = (url) => {
  const path = url.slice(site.length);
  if (path === "/") return "index.html";
  if (path.endsWith("/")) return `${path.slice(1)}index.html`;
  return path.endsWith(".html") ? path.slice(1) : `${path.slice(1)}.html`;
};
const jsonld = (html) =>
  [...html.matchAll(/<script type="application\/ld\+json">([\s\S]*?)<\/script>/g)].flatMap((m) => JSON.parse(m[1])["@graph"]);

test("sitemap lists the home page, docs and standalone pages, and each exists", () => {
  for (const u of [`${site}/`, `${site}/docs/`, `${site}/developers`, `${site}/about`, `${site}/privacy`, `${site}/contact`, `${site}/docs/api.html`]) {
    assert.ok(sitemapUrls.includes(u), u);
  }
  for (const u of sitemapUrls) assert.ok(existsSync(join(out, fileFor(u))), u);
});

test("every page has a title, description, canonical URL, Markdown copy and valid JSON-LD", () => {
  for (const u of sitemapUrls) {
    const html = read(fileFor(u));
    assert.match(html, /<title>[^<]{10,}<\/title>/, u);
    assert.match(html, /<meta name="description" content="[^"]{50,}">/, u);
    assert.ok(html.includes(`<link rel="canonical" href="${u}">`), u);
    const md = html.match(/<link rel="alternate" type="text\/markdown" href="([^"]+)"/)?.[1];
    assert.ok(md?.startsWith(site), `${u} has no Markdown alternate`);
    assert.ok(read(md.slice(site.length + 1)).trim().startsWith("#"), `${md} is not Markdown`);
    assert.ok(jsonld(html).length >= 2, u);
    assert.doesNotMatch(html, /\{\{\w+\}\}|%(TITLE|DESCRIPTION|URL|MD_URL|NAME|JSONLD|MENU|BODY)%/, `${u} has a placeholder left`);
  }
});

test("home page JSON-LD describes the app, with keywords that appear on the page", () => {
  const html = read("index.html");
  const graph = jsonld(html);
  const app = graph.find((n) => n["@type"] === "SoftwareApplication");
  assert.equal(app.name, "Keeprun");
  assert.equal(app.offers.price, "0");
  const keywords = app.keywords.split(", ");
  assert.equal(keywords[0], "Windows process manager");
  const text = html.replace(/<script[\s\S]*?<\/script>/g, "").replace(/<[^>]+>/g, " ");
  for (const k of keywords) assert.ok(text.includes(k), `keyword "${k}" is not on the page`);
  assert.match(html.match(/<title>(.*)<\/title>/)[1], /Keeprun .*Windows process manager/);
  const org = graph.find((n) => n["@type"] === "Organization");
  assert.equal(org.contactPoint.url, `${site}/contact`);
  const faq = graph.find((n) => n["@type"] === "FAQPage");
  assert.ok(faq.mainEntity.length >= 5);
});

test("API and CLI docs have the product name in the title and heading", () => {
  for (const [file, name] of [["docs/api.html", "Keeprun Agent API"], ["docs/cli.html", "Keeprun CLI"]]) {
    const html = read(file);
    assert.ok(html.includes(`<title>${name} | Keeprun docs</title>`), file);
    assert.match(html, new RegExp(`<h1[^>]*>${name}</h1>`), file);
  }
});

test("llms.txt follows the llms.txt format and links to Markdown docs", () => {
  const llms = read("llms.txt");
  const lines = llms.split("\n");
  assert.equal(lines[0], "# Keeprun");
  assert.ok(lines.some((l) => l.startsWith("> ")), "summary blockquote");
  for (const h of ["## When to use Keeprun", "## How an agent uses Keeprun", "## Docs", "## Optional"]) {
    assert.ok(lines.includes(h), h);
  }
  const links = [...llms.matchAll(/\]\((https:\/\/keeprun\.app\/[^)]+)\)/g)].map((m) => m[1]);
  assert.ok(links.length >= 10);
  for (const l of links) assert.ok(existsSync(join(out, l.slice(site.length + 1))), l);
  assert.ok(read("llms-full.txt").length > read("llms.txt").length);
});

test("robots.txt allows crawling and points to the sitemap", () => {
  const robots = read("robots.txt");
  assert.match(robots, /^User-agent: \*$/m);
  assert.match(robots, /^Allow: \/$/m);
  assert.match(robots, new RegExp(`^Sitemap: ${site}/sitemap.xml$`, "m"));
});

test("404 page exists and is not indexed", () => {
  const html = read("404.html");
  assert.match(html, /<meta name="robots" content="noindex">/);
  assert.match(html, /href="\/llms.txt"/);
});

test("OpenAPI spec is 3.1 with a unique operationId, summary and responses on every operation", () => {
  const spec = JSON.parse(read("openapi.json"));
  assert.match(spec.openapi, /^3\.1\./);
  const ids = new Set();
  for (const [path, item] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(item)) {
      if (method === "parameters") continue;
      assert.ok(op.operationId, `${method} ${path}`);
      assert.ok(!ids.has(op.operationId), `duplicate ${op.operationId}`);
      ids.add(op.operationId);
      assert.ok(op.summary, `${op.operationId} summary`);
      assert.ok(op.responses?.["200"] || op.responses?.["201"] || op.responses?.["204"], `${op.operationId} success response`);
    }
  }
  // Every $ref resolves.
  const refs = [...JSON.stringify(spec).matchAll(/"\$ref":"#\/([^"]+)"/g)].map((m) => m[1]);
  for (const r of refs) assert.ok(r.split("/").reduce((o, k) => o?.[k], spec), r);
});
