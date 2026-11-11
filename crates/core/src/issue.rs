//! Spots errors in app output while the process keeps running, such as a dev server whose
//! build broke. The process is alive, so its state says Running, but the app is broken.
//!
//! The agent feeds every output line to an `IssueTracker`. The first error starts an issue
//! and its block of output is kept once; repeats of the same error only raise the count, so
//! a browser retrying a broken page 60 times still gives one short excerpt. A successful
//! request or build after the errors stop clears it, and so does a new run.

use crate::model::{format_utc, AppIssue, AppState, AppView, Event};
use std::collections::VecDeque;

/// Lines kept before the error line, e.g. the file name Next.js prints above it.
const CONTEXT_BEFORE: usize = 2;
const EXCERPT_MAX: usize = 40;
const LINE_MAX: usize = 400;
const SUMMARY_MAX: usize = 200;
/// A success line clears the issue only once errors have been quiet this long, so one good
/// request in the middle of an error burst does not hide it.
const QUIET_MS: u64 = 5_000;
/// An issue with no new error for this long is over.
pub const STALE_MS: u64 = 15 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Error,
    Ok,
    Other,
}

/// Remove ANSI escape sequences (colors, cursor moves).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // CSI: parameters, then one final byte in @..~.
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    out
}

/// The line without colors, indentation and the prefixes dev servers add.
fn clean(line: &str) -> String {
    let s = strip_ansi(line);
    let mut s = s.trim();
    loop {
        let before = s;
        for p in ["[browser]", "⨯", "×", "\u{2716}", "✘"] {
            s = s.strip_prefix(p).unwrap_or(s).trim_start();
        }
        if s == before {
            return s.to_string();
        }
    }
}

const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// Status code of a request log line: `GET / 500 in 292ms` (Next.js),
/// `"GET / HTTP/1.1" 500 Internal Server Error` (uvicorn), `"GET / HTTP/1.1" 200 145` (Django).
fn http_status(s: &str) -> Option<u16> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let at = tokens
        .iter()
        .position(|t| HTTP_METHODS.contains(&t.trim_start_matches('"')))?;
    tokens[at + 1..].iter().take(4).find_map(|t| {
        let t = t.trim_end_matches(',');
        (t.len() == 3 && t.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse::<u16>().ok())
            .flatten()
            .filter(|c| (100..600).contains(c))
    })
}

/// `TypeError:`, `ValueError:`, `System.InvalidOperationException:` and similar.
fn starts_with_error_type(s: &str) -> bool {
    let Some(first) = s.split_whitespace().next() else {
        return false;
    };
    let Some(name) = first.strip_suffix(':') else {
        return false;
    };
    (name.ends_with("Error") || name.ends_with("Exception"))
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

fn is_error(raw: &str, s: &str) -> bool {
    // Next.js marks errors with ⨯, esbuild and Vite with ✘.
    let raw = strip_ansi(raw);
    let raw = raw.trim_start();
    if raw.starts_with('⨯') || raw.starts_with("✘ [ERROR]") {
        return true;
    }
    const STARTS: [&str; 19] = [
        "Uncaught ",
        "Unhandled",
        "Traceback (most recent call last)",
        "Failed to compile",
        "Fatal error",
        "FATAL",
        "fail: ",
        "crit: ",
        "ERROR:",
        "ERROR ",
        "[ERROR]",
        "ERROR in ",
        "Exception in ",
        "npm ERR!",
        "npm error",
        "panic:",
        "Error [",
        "error[E",
        "error TS",
    ];
    const CONTAINS: [&str; 7] = [
        "Module not found",
        "Cannot find module",
        ": error TS",
        ": error CS",
        "panicked at",
        "EADDRINUSE",
        "] ERROR",
    ];
    starts_with_error_type(s)
        || STARTS.iter().any(|p| s.starts_with(p))
        || CONTAINS.iter().any(|p| s.contains(p))
}

/// Whether a line of app output reports an error, a success, or neither.
pub fn classify(line: &str) -> Signal {
    let s = clean(line);
    if let Some(code) = http_status(&s) {
        return match code {
            500.. => Signal::Error,
            ..400 => Signal::Ok,
            _ => Signal::Other,
        };
    }
    if is_error(line, &s) {
        Signal::Error
    } else if s.starts_with("✓ Compiled") || s.contains("compiled successfully") {
        Signal::Ok
    } else {
        Signal::Other
    }
}

/// Same error, ignoring numbers such as times, line numbers and PIDs.
fn signature(line: &str) -> String {
    let mut out = String::new();
    for c in clean(line).chars().take(SUMMARY_MAX) {
        if c.is_ascii_digit() {
            if !out.ends_with('#') {
                out.push('#');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

#[derive(Debug, Default)]
pub struct IssueTracker {
    issue: Option<AppIssue>,
    /// Signature of the error that started the issue.
    sig: String,
    /// Lines still to add to the excerpt after the error.
    collecting: usize,
    /// Excerpt length up to and including the error line.
    head: usize,
    /// The last few lines, for context before an error.
    recent: VecDeque<String>,
}

impl IssueTracker {
    /// Feed one line of app output. `now` is Unix time in milliseconds.
    pub fn feed(&mut self, line: &str, now: u64) {
        let text = truncate(strip_ansi(line).trim_end(), LINE_MAX);
        match classify(line) {
            Signal::Error => self.error(line, text.clone(), now),
            Signal::Ok => {
                self.collecting = 0;
                if self
                    .issue
                    .as_ref()
                    .is_some_and(|i| now.saturating_sub(i.last_at) >= QUIET_MS)
                {
                    self.issue = None;
                }
            }
            Signal::Other => {
                if self.collecting > 0 {
                    if let Some(i) = self.issue.as_mut() {
                        i.excerpt.push(text.clone());
                    }
                    self.collecting -= 1;
                }
            }
        }
        if !text.trim().is_empty() {
            if self.recent.len() == CONTEXT_BEFORE {
                self.recent.pop_front();
            }
            self.recent.push_back(text);
        }
    }

    fn error(&mut self, line: &str, text: String, now: u64) {
        let sig = signature(line);
        if let Some(i) = self
            .issue
            .as_mut()
            .filter(|i| now.saturating_sub(i.last_at) < STALE_MS)
        {
            i.last_at = now;
            if sig == self.sig {
                i.count += 1;
                if self.collecting > 0 {
                    // The same error again: the first block is complete. Its end holds the
                    // lead-in to the repeat, the same lines that came before the first error.
                    self.collecting = 0;
                    let lead: Vec<String> = i.excerpt[..self.head - 1].to_vec();
                    while i.excerpt.len() > self.head
                        && i.excerpt
                            .last()
                            .is_some_and(|l| l.trim().is_empty() || lead.contains(l))
                    {
                        i.excerpt.pop();
                    }
                }
            } else if self.collecting > 0 {
                i.excerpt.push(text);
                self.collecting -= 1;
            }
            return;
        }
        let mut excerpt: Vec<String> = self.recent.drain(..).collect();
        excerpt.push(text);
        self.head = excerpt.len();
        self.collecting = EXCERPT_MAX.saturating_sub(excerpt.len());
        self.sig = sig;
        self.issue = Some(AppIssue {
            first_at: now,
            last_at: now,
            count: 1,
            summary: truncate(&clean(line), SUMMARY_MAX),
            excerpt,
        });
    }

    /// The current issue, unless it went stale.
    pub fn current(&self, now: u64) -> Option<AppIssue> {
        let mut i = self
            .issue
            .clone()
            .filter(|i| now.saturating_sub(i.last_at) < STALE_MS)?;
        while i.excerpt.last().is_some_and(|l| l.trim().is_empty()) {
            i.excerpt.pop();
        }
        Some(i)
    }

    pub fn clear(&mut self) {
        self.issue = None;
        self.collecting = 0;
        self.recent.clear();
    }
}

/// Text for a coding agent: what broke, where the app lives, and how to check a fix.
/// `events` are newest first, as the API returns them.
pub fn agent_prompt(app: &AppView, events: &[Event]) -> Option<String> {
    let issue = app.issue.as_ref()?;
    let spec = &app.spec;
    let id = &spec.id;
    let mut p = String::new();
    p.push_str(&format!(
        "My app \"{}\" is printing errors. Keeprun runs it in the background on Windows, so you \
         cannot see its console. Find the cause and fix it in the code.\n\n",
        spec.name
    ));
    p.push_str(&format!("Folder: {}\n", spec.cwd.display()));
    p.push_str(&format!("Command: {}\n", spec.command));
    let mut state = app.state.to_string();
    if let Some(pid) = app.pid.filter(|_| app.state == AppState::Running) {
        state.push_str(&format!(" (PID {pid})"));
    }
    if let Some(ports) = app.stats.as_ref().filter(|s| !s.ports.is_empty()) {
        let ports: Vec<String> = ports.ports.iter().map(|p| p.to_string()).collect();
        state.push_str(&format!(", listening on port {}", ports.join(", ")));
    }
    p.push_str(&format!("State: {state}\n"));
    let seen = if issue.count == 1 {
        format!("once at {}", format_utc(issue.first_at))
    } else {
        format!(
            "{} times between {} and {}",
            issue.count,
            format_utc(issue.first_at),
            format_utc(issue.last_at)
        )
    };
    p.push_str(&format!("Error seen: {seen}\n\n"));
    p.push_str("First occurrence, repeats removed:\n```\n");
    for l in &issue.excerpt {
        p.push_str(l);
        p.push('\n');
    }
    p.push_str("```\n");
    let recent: Vec<&Event> = events.iter().take(6).collect();
    if !recent.is_empty() {
        p.push_str("\nRecent events:\n");
        for e in recent.iter().rev() {
            p.push_str(&format!("- {}  {}\n", format_utc(e.ts), e.message));
        }
    }
    p.push_str(&format!(
        "\nTo check your fix:\n\
         - `keeprun logs {id} -n 100` shows the latest output\n\
         - `keeprun restart {id}` restarts the app (dev servers usually reload on their own)\n\
         - `keeprun issue {id}` shows whether the error is still there\n\
         Do not stop, remove or reconfigure the app in Keeprun.\n"
    ));
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_lines() {
        use Signal::*;
        for l in [
            "Error: Module not found: Can't resolve '@/lib/topics'",
            "[browser] Uncaught Error: ./lib/list-updates/repo.ts:8:1",
            " ⨯ ./app/page.tsx:3:1",
            "\u{1b}[31m⨯\u{1b}[39m unhandledRejection: boom",
            "TypeError: Cannot read properties of undefined (reading 'x')",
            "Traceback (most recent call last):",
            "ValueError: bad value",
            "ERROR:    Exception in ASGI application",
            "fail: Microsoft.AspNetCore.Server.Kestrel[13]",
            "Unhandled exception. System.InvalidOperationException: nope",
            "System.InvalidOperationException: nope",
            "src/a.ts(3,1): error TS2304: Cannot find name 'x'.",
            "Program.cs(5,1): error CS0103: The name 'x' does not exist",
            "Error: listen EADDRINUSE: address already in use :::3000",
            " GET / 500 in 292ms (next.js: 270ms)",
            "INFO:     127.0.0.1:5000 - \"GET / HTTP/1.1\" 500 Internal Server Error",
        ] {
            assert_eq!(classify(l), Error, "{l}");
        }
        for l in [
            " GET / 200 in 2.2s (next.js: 1690ms)",
            " GET /api/documents/held 200 in 509ms (next.js: 378ms)",
            "[11/Nov/2026 10:00:00] \"GET / HTTP/1.1\" 200 145",
            "INFO:     127.0.0.1:5000 - \"POST /x HTTP/1.1\" 302 Found",
            " ✓ Compiled /api in 1.2s",
        ] {
            assert_eq!(classify(l), Ok, "{l}");
        }
        for l in [
            "▲ Next.js 16.4.4 (Turbopack)",
            "✓ Ready in 516ms",
            " GET /missing 404 in 20ms",
            "Compiled with 0 errors",
            "warn  - something",
            "No errors found",
            "   8 | import { topicOf } from \"@/lib/topics\";",
            "info: Microsoft.Hosting.Lifetime[0]",
        ] {
            assert_eq!(classify(l), Other, "{l}");
        }
    }

    fn next_block(t: &mut IssueTracker, now: u64) {
        for l in [
            "[browser] ./lib/list-updates/repo.ts:8:1",
            "Error: Module not found: Can't resolve '@/lib/topics'",
            "   6 | import { savedMapping } from \"@/lib/import-mapping-repo\";",
            ">  8 | import { topicOf } from \"@/lib/topics\";",
            "",
            "https://nextjs.org/docs/messages/module-not-found ",
            "[browser] ./lib/list-updates/repo.ts:9:1",
            "Error: Module not found: Can't resolve '@/lib/topics-repo'",
            ">  9 | import { topicSenders } from \"@/lib/topics-repo\";",
            "",
        ] {
            t.feed(l, now);
        }
    }

    #[test]
    fn repeats_keep_one_excerpt() {
        let mut t = IssueTracker::default();
        t.feed("✓ Ready in 516ms", 0);
        for n in 0..60 {
            next_block(&mut t, 1000 + n);
        }
        let i = t.current(2000).unwrap();
        assert_eq!(i.count, 60);
        assert_eq!(i.first_at, 1000);
        assert_eq!(i.last_at, 1059);
        assert_eq!(
            i.summary,
            "Error: Module not found: Can't resolve '@/lib/topics'"
        );
        assert_eq!(i.excerpt[0], "✓ Ready in 516ms");
        assert_eq!(i.excerpt[1], "[browser] ./lib/list-updates/repo.ts:8:1");
        assert!(i.excerpt.iter().any(|l| l.contains("topics-repo")));
        // One copy of the block, not 60.
        let firsts = i
            .excerpt
            .iter()
            .filter(|l| l.contains("Can't resolve '@/lib/topics'"))
            .count();
        assert_eq!(firsts, 1);
        assert!(i.excerpt.len() <= EXCERPT_MAX);
        // Ends with the second error's block, not the lead-in to the repeat.
        assert_eq!(
            i.excerpt.last().unwrap(),
            ">  9 | import { topicSenders } from \"@/lib/topics-repo\";"
        );
    }

    #[test]
    fn success_clears_after_quiet() {
        let mut t = IssueTracker::default();
        next_block(&mut t, 1000);
        // A good request during the burst does not hide it.
        t.feed(" GET /other 200 in 5ms", 2000);
        assert!(t.current(2000).is_some());
        t.feed(" GET / 200 in 5ms", 1000 + QUIET_MS);
        assert!(t.current(1000 + QUIET_MS).is_none());
    }

    #[test]
    fn stale_and_clear() {
        let mut t = IssueTracker::default();
        t.feed("TypeError: x is undefined", 0);
        assert!(t.current(STALE_MS - 1).is_some());
        assert!(t.current(STALE_MS).is_none());
        // A new error after the stale one starts a fresh issue.
        t.feed("TypeError: y is undefined", STALE_MS + 1);
        let i = t.current(STALE_MS + 1).unwrap();
        assert_eq!(i.count, 1);
        assert_eq!(i.summary, "TypeError: y is undefined");
        t.clear();
        assert!(t.current(STALE_MS + 1).is_none());
    }

    #[test]
    fn signature_ignores_numbers() {
        assert_eq!(
            signature("⨯ Error at 12:01:02 pid 4411"),
            signature("Error at 9:59:59 pid 7")
        );
    }

    #[test]
    fn ansi() {
        assert_eq!(strip_ansi("\u{1b}[1;31mred\u{1b}[0m ok"), "red ok");
    }

    #[test]
    fn prompt() {
        let mut t = IssueTracker::default();
        next_block(&mut t, 1_762_862_299_000);
        next_block(&mut t, 1_762_862_301_000);
        let mut app: AppView = serde_json::from_value(serde_json::json!({
            "id": "invoice-extractor", "name": "invoice-extractor", "cwd": "C:\\apps\\inv",
            "command": "npm run dev", "state": "running", "pid": 5612, "exit_code": null,
            "started_at": null, "next_restart_at": null,
            "stats": {"cpu_percent": 1.0, "memory_bytes": 1, "processes": 1, "ports": [3000]}
        }))
        .unwrap();
        assert!(agent_prompt(&app, &[]).is_none());
        app.issue = t.current(1_762_862_301_000);
        let events = vec![Event {
            id: 1,
            app_id: "invoice-extractor".into(),
            ts: 1_762_862_000_000,
            kind: crate::model::EventKind::Started,
            message: "Started (PID 5612)".into(),
        }];
        let p = agent_prompt(&app, &events).unwrap();
        assert!(p.contains("Folder: C:\\apps\\inv"));
        assert!(p.contains("State: Running (PID 5612), listening on port 3000"));
        assert!(p.contains("2 times between 2025-11-11 11:58:19Z and 2025-11-11 11:58:21Z"));
        assert!(p.contains("Can't resolve '@/lib/topics'"));
        assert!(p.contains("Started (PID 5612)"));
        assert!(p.contains("keeprun logs invoice-extractor -n 100"));
    }
}
