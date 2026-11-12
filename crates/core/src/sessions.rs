//! Whether a coding agent is still working in an app's folder.
//!
//! While Claude Code edits an app, the dev server often prints errors from half-written
//! files. Those are not worth a notification: the session is still busy and will likely fix
//! them. Keeprun waits until every session in the folder has finished its turn, and only
//! then notifies if the app still has errors.
//!
//! Claude Code keeps one transcript per session in `~/.claude/projects/<folder>/<id>.jsonl`,
//! where `<folder>` is the session's working folder with every character other than a
//! letter or digit replaced by `-`. A session is working when its transcript changed
//! recently and its last turn has not ended.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A transcript untouched this long belongs to a session that is idle, waiting for
/// permission, or gone. Past this, errors are reported anyway.
pub const ACTIVE_MAX: Duration = Duration::from_secs(10 * 60);
/// Bytes read from the end of a transcript to find its last entry.
const TAIL: u64 = 256 * 1024;

/// The Claude Code config folder: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn claude_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("USERPROFILE")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|h| !h.is_empty()))?;
    Some(PathBuf::from(home).join(".claude"))
}

/// A folder as Claude Code names its transcript folder, lowercased because the drive letter
/// is spelled either way.
fn encode(dir: &Path) -> String {
    dir.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The repository folder above `dir`, if `dir` is inside one.
fn git_root(dir: &Path) -> Option<&Path> {
    dir.ancestors().skip(1).find(|d| d.join(".git").exists())
}

/// Whether a transcript folder belongs to sessions in `dir`: the folder itself, a folder
/// inside it, or the root of the repository it is in.
fn matches(name: &str, app: &str, root: Option<&str>) -> bool {
    let name = name.to_ascii_lowercase();
    name == app
        || name
            .strip_prefix(app)
            .is_some_and(|rest| rest.starts_with('-'))
        || root.is_some_and(|r| name == r)
}

/// What the last entries of a transcript say about its session.
pub fn tail_is_busy(tail: &str) -> bool {
    for line in tail.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["isSidechain"] == true || v["isMeta"] == true {
            continue;
        }
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "turn_duration" => return false,
            Some("assistant") => {
                return !matches!(
                    v["message"]["stop_reason"].as_str(),
                    Some("end_turn" | "stop_sequence" | "max_tokens" | "refusal")
                );
            }
            Some("user") => {
                let content = &v["message"]["content"];
                let text = match content {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(parts) => parts
                        .iter()
                        .filter_map(|p| p["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => String::new(),
                };
                return !text.contains("[Request interrupted by user");
            }
            _ => continue,
        }
    }
    // Nothing decisive in the tail, e.g. one huge tool result: it changed recently.
    true
}

fn read_tail(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Whether a Claude Code session in `dir` (or in a folder inside it, or at the root of its
/// repository) is in the middle of a turn.
pub fn agent_working(dir: &Path) -> bool {
    let Some(projects) = claude_dir().map(|d| d.join("projects")) else {
        return false;
    };
    agent_working_in(&projects, dir, SystemTime::now())
}

fn agent_working_in(projects: &Path, dir: &Path, now: SystemTime) -> bool {
    let app = encode(dir);
    let root = git_root(dir).map(encode);
    let Ok(entries) = std::fs::read_dir(projects) else {
        return false;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !matches(&name, &app, root.as_deref()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(e.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let recent = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok().or(Some(Duration::ZERO)))
                .is_some_and(|age| age < ACTIVE_MAX);
            if recent && read_tail(&path).is_some_and(|t| tail_is_busy(&t)) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_claude_code() {
        assert_eq!(
            encode(Path::new(r"C:\Users\YB\Desktop\keeprun")),
            "c--users-yb-desktop-keeprun"
        );
        assert_eq!(encode(Path::new("/home/me/my_app/")), "-home-me-my-app");
    }

    #[test]
    fn matches_the_folder_its_subfolders_and_repo_root() {
        let app = "c--x-repo-apps-web";
        assert!(matches("C--x-repo-apps-web", app, None));
        assert!(matches("c--x-repo-apps-web-src", app, None));
        assert!(!matches("c--x-repo-apps-webby", app, None));
        assert!(!matches("c--x-repo", app, None));
        assert!(matches("c--x-repo", app, Some("c--x-repo")));
        assert!(!matches("c--x-repo-apps-api", app, Some("c--x-repo")));
    }

    fn line(v: serde_json::Value) -> String {
        v.to_string() + "\n"
    }

    #[test]
    fn reads_the_turn_state_from_the_tail() {
        let tool_use = line(serde_json::json!({
            "type": "assistant", "message": {"stop_reason": "tool_use", "content": []}
        }));
        let result = line(serde_json::json!({
            "type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]}
        }));
        let done = line(serde_json::json!({
            "type": "assistant", "message": {"stop_reason": "end_turn", "content": []}
        }));
        let duration = line(serde_json::json!({"type": "system", "subtype": "turn_duration"}));
        let noise = line(serde_json::json!({"type": "cost-state"}));
        let interrupted = line(serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}
        }));

        assert!(tail_is_busy(&tool_use));
        assert!(tail_is_busy(&format!("{tool_use}{result}{noise}")));
        assert!(!tail_is_busy(&format!("{result}{done}{noise}")));
        assert!(!tail_is_busy(&format!("{tool_use}{duration}{noise}")));
        assert!(!tail_is_busy(&format!("{tool_use}{interrupted}")));
        // A partial first line from reading only the tail is skipped.
        assert!(!tail_is_busy(&format!("ol_result\"}}]}}\n{done}")));
    }

    #[test]
    fn only_recent_transcripts_in_the_folder_count() {
        let tmp = std::env::temp_dir().join(format!("keeprun-sessions-{}", std::process::id()));
        let app = tmp.join("work").join("site");
        std::fs::create_dir_all(&app).unwrap();
        let projects = tmp.join("projects");
        let mine = projects.join(encode(&app));
        let other = projects.join(encode(&tmp.join("work").join("other")));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let busy = r#"{"type":"assistant","message":{"stop_reason":"tool_use"}}"#;
        let idle = r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#;
        let now = SystemTime::now();

        std::fs::write(other.join("a.jsonl"), busy).unwrap();
        std::fs::write(mine.join("b.jsonl"), idle).unwrap();
        assert!(!agent_working_in(&projects, &app, now));

        std::fs::write(mine.join("c.jsonl"), busy).unwrap();
        assert!(agent_working_in(&projects, &app, now));
        // Untouched for longer than ACTIVE_MAX: no longer working.
        let later = SystemTime::now() + ACTIVE_MAX + Duration::from_secs(1);
        assert!(!agent_working_in(&projects, &app, later));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
