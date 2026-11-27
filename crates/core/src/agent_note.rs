//! A note in an app's CLAUDE.md or AGENTS.md that tells coding agents the app runs in Keeprun.
//! Without it, an agent starts its own `npm run dev`: a second copy that fights over the port
//! and prints errors Keeprun never sees. The desktop app and `keeprun agent-setup` write it.

use std::path::Path;

/// Marks the note, so it is added once and found again.
const MARK: &str = "<!-- keeprun:app -->";
/// Ends the note, so a newer note replaces it in place.
const END: &str = "<!-- /keeprun:app -->";
/// Last line of the first version of the note, which had no end mark.
const OLD_LAST: &str =
    "- The same as MCP tools: `claude mcp add --scope user keeprun -- keeprun mcp`";
pub const FILES: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];

pub fn note(id: &str) -> String {
    format!(
        "{MARK}\n\
         ## Running this app\n\
         \n\
         Keeprun runs this app as `{id}`. Do not start it yourself (`npm run dev` or similar): a\n\
         second copy fights over the port, its errors never reach Keeprun, and Keeprun ends it.\n\
         `kr` is short for `keeprun`. In this folder it needs no app name; elsewhere add `{id}`.\n\
         \n\
         - Restart, e.g. after changing code, config or dependencies: `kr restart`. It waits\n\
         \x20 until the app runs and prints its URL, or the errors of the new run.\n\
         - Database migrations: `kr migrate` runs the app's migrate task, then restarts it.\n\
         \x20 `kr status` says when a migration is needed.\n\
         - Errors as a fix prompt: `kr issue`\n\
         - Logs: `kr logs` (add `-f` to follow)\n\
         - Installed as a service, Keeprun runs the app as LocalSystem, not as you. Programs\n\
         \x20 that refuse administrator rights (postgres.exe; pg_ctl works) and per-user installs\n\
         \x20 or logins fail there even when they work in your terminal.\n\
         - The same as MCP tools: `claude mcp add --scope user keeprun -- keeprun mcp`\n\
         {END}\n"
    )
}

/// The file in `cwd` that holds the note.
pub fn noted(cwd: &Path) -> Option<&'static str> {
    FILES.into_iter().find(|f| {
        std::fs::read_to_string(cwd.join(f))
            .map(|t| t.contains(MARK))
            .unwrap_or(false)
    })
}

/// `text` with its note replaced by `new`. None when the note cannot be told apart from the
/// rest of the file.
fn replace(text: &str, new: &str) -> Option<String> {
    let start = text.find(MARK)?;
    let end = match text[start..].find(END) {
        Some(i) => start + i + END.len(),
        None => start + text[start..].find(OLD_LAST)? + OLD_LAST.len(),
    };
    let end = if text[end..].starts_with("\r\n") {
        end + 2
    } else if text[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    Some(format!("{}{new}{}", &text[..start], &text[end..]))
}

/// What `write` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Written {
    Added(&'static str),
    Updated(&'static str),
    Unchanged(&'static str),
}

impl Written {
    pub fn file(&self) -> &'static str {
        match self {
            Written::Added(f) | Written::Updated(f) | Written::Unchanged(f) => f,
        }
    }
}

/// Add the note to CLAUDE.md, or to AGENTS.md when only that exists, or bring an older note
/// up to date. Line endings of the file are kept.
pub fn write(cwd: &Path, id: &str) -> Result<Written, String> {
    let (name, existing) = match noted(cwd) {
        Some(f) => (f, true),
        None if !cwd.join(FILES[0]).exists() && cwd.join(FILES[1]).exists() => (FILES[1], false),
        None => (FILES[0], false),
    };
    let path = cwd.join(name);
    let old = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    let crlf = old.contains("\r\n");
    let mut new = note(id);
    if crlf {
        new = new.replace('\n', "\r\n");
    }
    let text = if existing {
        match replace(&old, &new) {
            Some(t) => t,
            // Edited by hand past recognition: leave it to the user.
            None => return Ok(Written::Unchanged(name)),
        }
    } else {
        let mut t = old.clone();
        if !t.is_empty() {
            let nl = if crlf { "\r\n" } else { "\n" };
            if !t.ends_with('\n') {
                t.push_str(nl);
            }
            t.push_str(nl);
        }
        t.push_str(&new);
        t
    };
    if text == old {
        return Ok(Written::Unchanged(name));
    }
    std::fs::write(&path, &text).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(if existing {
        Written::Updated(name)
    } else {
        Written::Added(name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("keeprun-note-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_claude_md_once() {
        let d = folder("new");
        assert_eq!(write(&d, "shop").unwrap(), Written::Added("CLAUDE.md"));
        assert_eq!(write(&d, "shop").unwrap(), Written::Unchanged("CLAUDE.md"));
        let text = std::fs::read_to_string(d.join("CLAUDE.md")).unwrap();
        assert_eq!(text.matches(MARK).count(), 1);
        assert!(text.contains("`kr restart`"));
        assert!(text.contains("`kr migrate`"));
        assert_eq!(noted(&d), Some("CLAUDE.md"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn appends_to_agents_md_and_keeps_crlf() {
        let d = folder("agents");
        std::fs::write(d.join("AGENTS.md"), "# Rules\r\nBe brief.\r\n").unwrap();
        assert_eq!(write(&d, "shop").unwrap(), Written::Added("AGENTS.md"));
        let text = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(text.starts_with("# Rules\r\nBe brief.\r\n\r\n<!-- keeprun:app -->\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        assert!(!d.join("CLAUDE.md").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn updates_the_first_version_in_place() {
        let d = folder("old");
        let old = "<!-- keeprun:app -->\n## Running this app\n\nKeeprun runs this app as `shop`.\n\
                   - Restart: `keeprun restart shop`\n\
                   - The same as MCP tools: `claude mcp add --scope user keeprun -- keeprun mcp`\n";
        std::fs::write(
            d.join("CLAUDE.md"),
            format!("# Shop\n\n{old}\n## Tests\nnpm test\n"),
        )
        .unwrap();
        assert_eq!(write(&d, "shop").unwrap(), Written::Updated("CLAUDE.md"));
        let text = std::fs::read_to_string(d.join("CLAUDE.md")).unwrap();
        assert!(text.starts_with("# Shop\n\n<!-- keeprun:app -->\n"));
        assert!(text.contains("`kr migrate`"));
        assert!(!text.contains("keeprun restart shop"));
        assert!(text.ends_with("<!-- /keeprun:app -->\n\n## Tests\nnpm test\n"));
        assert_eq!(write(&d, "shop").unwrap(), Written::Unchanged("CLAUDE.md"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
