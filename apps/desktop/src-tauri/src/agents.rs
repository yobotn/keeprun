//! A note in an app's CLAUDE.md or AGENTS.md that tells coding agents the app runs in Keeprun.
//! Without it, an agent starts its own `npm run dev`: a second copy that fights over the port
//! and prints errors Keeprun never sees.

use std::path::{Path, PathBuf};

/// Marks the note, so it is added once and found again.
const MARK: &str = "<!-- keeprun:app -->";
const FILES: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];

fn note(id: &str) -> String {
    format!(
        "{MARK}\n\
         ## Running this app\n\
         \n\
         Keeprun runs this app as `{id}`. Do not start it yourself (`npm run dev` or similar): a\n\
         second copy fights over the port, its errors never reach Keeprun, and Keeprun ends it.\n\
         \n\
         - Logs: `keeprun logs {id}` (add `-f` to follow)\n\
         - Errors as a fix prompt: `keeprun issue {id}`\n\
         - Restart, e.g. after changing config or dependencies: `keeprun restart {id}`\n\
         - Status and port: `keeprun list`\n"
    )
}

/// The file that already holds the note.
fn noted(cwd: &Path) -> Option<&'static str> {
    FILES.into_iter().find(|f| {
        std::fs::read_to_string(cwd.join(f))
            .map(|t| t.contains(MARK))
            .unwrap_or(false)
    })
}

/// Name of the file in the app folder that tells coding agents to use Keeprun, if any.
#[tauri::command(async)]
pub fn agent_note_file(cwd: PathBuf) -> Option<String> {
    noted(&cwd).map(str::to_string)
}

/// Add the note to CLAUDE.md, or to AGENTS.md when only that exists. Returns the file name.
#[tauri::command(async)]
pub fn add_agent_note(cwd: PathBuf, app_id: String) -> Result<String, String> {
    add(&cwd, &app_id)
}

fn add(cwd: &Path, id: &str) -> Result<String, String> {
    if let Some(f) = noted(cwd) {
        return Ok(f.into());
    }
    let name = if !cwd.join(FILES[0]).exists() && cwd.join(FILES[1]).exists() {
        FILES[1]
    } else {
        FILES[0]
    };
    let path = cwd.join(name);
    let old = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
    };
    let mut text = old.clone();
    if !text.is_empty() {
        text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
    }
    text.push_str(&note(id));
    // Keep the file's line endings.
    if old.contains("\r\n") {
        text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    std::fs::write(&path, text).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("keeprun-note-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_claude_md_once() {
        let d = folder("new");
        assert_eq!(add(&d, "shop").unwrap(), "CLAUDE.md");
        assert_eq!(add(&d, "shop").unwrap(), "CLAUDE.md");
        let text = std::fs::read_to_string(d.join("CLAUDE.md")).unwrap();
        assert_eq!(text.matches(MARK).count(), 1);
        assert!(text.contains("keeprun restart shop"));
        assert_eq!(agent_note_file(d.clone()).as_deref(), Some("CLAUDE.md"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn appends_to_agents_md_and_keeps_crlf() {
        let d = folder("agents");
        std::fs::write(d.join("AGENTS.md"), "# Rules\r\nBe brief.\r\n").unwrap();
        assert_eq!(add(&d, "shop").unwrap(), "AGENTS.md");
        let text = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert!(text.starts_with("# Rules\r\nBe brief.\r\n\r\n<!-- keeprun:app -->\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        assert!(!d.join("CLAUDE.md").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
