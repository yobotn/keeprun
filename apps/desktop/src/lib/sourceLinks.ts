// Finds source locations in app output (stack traces, compiler errors) so the UI can open
// them in an editor at the right line. Covers Node, TypeScript, Vite, .NET and Python.

export interface SourceLink {
  start: number;
  end: number;
  /** As printed: absolute, relative to the app folder, or a file:/// URL. */
  path: string;
  line: number;
  col?: number;
}

/** Extensions that count as source in `path:line` form, so `db.example.com:5432` does not. */
const EXT =
  "(?:[cm]?[jt]sx?|vue|svelte|astro|py|pyi|cs|fs|vb|cshtml|razor|json|ya?ml|toml|css|scss|less|html?|go|rs|java|kt|rb|php|sql|xml|config|csproj|ps1|bat|cmd)";
/** Characters allowed in a path without spaces. */
const P = `[^\\s:"'()<>|,;\\[\\]]`;
/** Must not start in the middle of a word or path. */
const LEAD = `(?<![\\w/\\\\.:@-])`;
const FILE_URL = `(?:file:\\/\\/\\/?)?`;

// Ordered most specific first: earlier patterns win when matches overlap.
/** `pathOnly`: the link covers just the path. `trim`: characters to leave out at the end. */
const PATTERNS: { re: RegExp; path: number; line: number; col?: number; pathOnly?: boolean; trim?: number }[] = [
  // Python: File "C:\app\main.py", line 88
  { re: /File "([^"]+)", line (\d+)/g, path: 1, line: 2, pathOnly: true },
  // .NET stack trace: in C:\app\Program.cs:line 42
  { re: /\bin ((?:[A-Za-z]:[\\/]|\\\\)[^:]*?):line (\d+)/g, path: 1, line: 2 },
  // Node stack frame in parentheses, path may hold spaces: (C:\My App\server.js:12:5)
  { re: new RegExp(`\\((${FILE_URL}(?:[A-Za-z]:[\\\\/]|\\\\\\\\)[^()]*?\\.${EXT}):(\\d+):(\\d+)\\)`, "g"), path: 1, line: 2, col: 3, trim: 1 },
  // MSBuild and tsc: Program.cs(42,13): error / src/a.ts(12,5): error TS2322
  { re: new RegExp(`${LEAD}((?:[A-Za-z]:[\\\\/])?${P}*\\.${EXT})\\((\\d+),(\\d+)\\)`, "g"), path: 1, line: 2, col: 3 },
  // Anything else: src/a.ts:12:5, C:\app\a.js:12, file:///C:/app/a.js:3:1
  { re: new RegExp(`${LEAD}(${FILE_URL}(?:[A-Za-z]:[\\\\/])?${P}*\\.${EXT}):(\\d+)(?::(\\d+))?(?!\\d)`, "g"), path: 1, line: 2, col: 3 },
];

/** Cheap test before running the patterns on every line. */
const MAYBE = /\.\w+["']?(?::\d|\(\d|, line \d)|:line \d/;
const URL_RE = /\b(?!file:)[a-z][a-z0-9+.-]*:\/\/\S+/gi;

export function findSourceLinks(text: string): SourceLink[] {
  if (!MAYBE.test(text)) return [];
  // Paths inside web URLs (http://localhost:5173/src/main.ts:4:1) are not files.
  const urls = [...text.matchAll(URL_RE)].map((m) => [m.index, m.index + m[0].length]);
  const found: SourceLink[] = [];
  for (const { re, path, line, col, pathOnly, trim = 0 } of PATTERNS) {
    for (const m of text.matchAll(re)) {
      const p = m[path];
      const start = m.index + m[0].indexOf(p);
      const end = pathOnly ? start + p.length : m.index + m[0].length - trim;
      if (/^(node:|internal[\\/])/.test(p)) continue;
      if (urls.some(([a, b]) => start < b && end > a)) continue;
      if (found.some((f) => start < f.end && end > f.start)) continue;
      found.push({ start, end, path: p, line: Number(m[line]), col: col && m[col] ? Number(m[col]) : undefined });
    }
  }
  return found.sort((a, b) => a.start - b.start);
}

export type Segment = string | (SourceLink & { text: string });

/** Split a line into plain text and links, in order. */
export function splitSourceLinks(text: string): Segment[] {
  const links = findSourceLinks(text);
  if (!links.length) return [text];
  const out: Segment[] = [];
  let at = 0;
  for (const l of links) {
    if (l.start > at) out.push(text.slice(at, l.start));
    out.push({ ...l, text: text.slice(l.start, l.end) });
    at = l.end;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}
