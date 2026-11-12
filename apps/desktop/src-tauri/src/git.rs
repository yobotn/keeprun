//! Git state of an app folder, and pulling it. Runs `git` as the signed-in user, never in the
//! agent: the service account has none of the user's credentials, and Git refuses repositories
//! owned by another account.

use serde::Serialize;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct GitStatus {
    /// None on a detached HEAD.
    pub branch: Option<String>,
    /// Tracked remote branch, e.g. "origin/main".
    pub upstream: Option<String>,
    /// Commits not pushed and not pulled, as of the last fetch.
    pub ahead: u32,
    pub behind: u32,
    /// Changed, added, deleted and untracked files.
    pub changed: u32,
    pub commit: Option<Commit>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    /// Unix time in milliseconds.
    pub time: u64,
}

#[derive(Serialize, Debug)]
pub struct Pulled {
    /// New commits on this branch.
    pub commits: u32,
    /// A file that lists dependencies changed, so they need installing.
    pub dependencies: bool,
    pub status: GitStatus,
}

/// Run git in `cwd`. `interactive` lets Git ask for credentials, for a pull the user clicked.
fn git(cwd: &Path, args: &[&str], interactive: bool, timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !interactive {
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "no-git".to_string()
        } else {
            format!("Cannot run git: {e}")
        }
    })?;
    // Read both pipes on threads so a chatty git never blocks on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "git {} did not finish in {} seconds.",
                args[0],
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        let msg = err.trim();
        Err(if msg.is_empty() {
            out.trim().to_string()
        } else {
            msg.to_string()
        })
    }
}

const QUICK: Duration = Duration::from_secs(10);
const NETWORK: Duration = Duration::from_secs(120);

fn parse_status(text: &str) -> GitStatus {
    let mut s = GitStatus::default();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            s.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
            s.upstream = Some(up.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    s.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    s.behind = n.parse().unwrap_or(0);
                }
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            s.changed += 1;
        }
    }
    s
}

fn parse_commit(text: &str) -> Option<Commit> {
    let mut f = text.trim_end_matches(['\r', '\n']).split('\u{1f}');
    let hash = f.next()?.to_string();
    let subject = f.next()?.to_string();
    let author = f.next()?.to_string();
    let time = f.next()?.trim().parse::<u64>().ok()? * 1000;
    (!hash.is_empty()).then_some(Commit {
        hash,
        subject,
        author,
        time,
    })
}

fn status(cwd: &Path) -> Result<GitStatus, String> {
    let text = git(cwd, &["status", "--porcelain=v2", "--branch"], false, QUICK)?;
    let mut s = parse_status(&text);
    s.commit = git(
        cwd,
        &["log", "-1", "--format=%h%x1f%s%x1f%an%x1f%ct"],
        false,
        QUICK,
    )
    .ok()
    .and_then(|t| parse_commit(&t));
    Ok(s)
}

/// Files whose change means dependencies need installing.
fn lists_dependencies(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    matches!(
        name.as_str(),
        "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
            | "requirements.txt"
            | "pyproject.toml"
            | "poetry.lock"
            | "uv.lock"
            | "pipfile.lock"
            | "packages.lock.json"
            | "directory.packages.props"
    ) || name.ends_with(".csproj")
        || (name.starts_with("requirements") && name.ends_with(".txt"))
}

/// Git state of the folder, without network access. None when it is not in a Git repository
/// or Git is not installed.
#[tauri::command(async)]
pub fn git_status(cwd: String) -> Option<GitStatus> {
    status(Path::new(&cwd)).ok()
}

/// Fetch the upstream branch without asking for credentials, then return the state.
#[tauri::command(async)]
pub fn git_fetch(cwd: String) -> Result<GitStatus, String> {
    let cwd = Path::new(&cwd);
    git(cwd, &["fetch", "--quiet", "--no-tags"], false, NETWORK)?;
    status(cwd)
}

/// Fast-forward the branch to its upstream. Never merges or rebases: diverged branches and
/// local changes in the way are reported, and nothing changes.
#[tauri::command(async)]
pub fn git_pull(cwd: String) -> Result<Pulled, String> {
    let cwd = Path::new(&cwd);
    let before = git(cwd, &["rev-parse", "HEAD"], false, QUICK)?;
    git(
        cwd,
        &["pull", "--ff-only", "--no-rebase", "--quiet"],
        true,
        NETWORK,
    )?;
    let after = git(cwd, &["rev-parse", "HEAD"], false, QUICK)?;
    let (before, after) = (before.trim(), after.trim());
    let (commits, dependencies) = if before == after {
        (0, false)
    } else {
        let range = format!("{before}..{after}");
        let commits = git(cwd, &["rev-list", "--count", &range], false, QUICK)?
            .trim()
            .parse()
            .unwrap_or(0);
        // Paths relative to the app folder's repository root.
        let files = git(cwd, &["diff", "--name-only", before, after], false, QUICK)?;
        (commits, files.lines().any(lists_dependencies))
    };
    Ok(Pulled {
        commits,
        dependencies,
        status: status(cwd)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_branch_upstream_and_changes() {
        let s = parse_status(
            "# branch.oid 1234\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -3\n\
             1 .M N... 100644 100644 100644 a b src/app.ts\n? notes.txt\n",
        );
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind, s.changed), (2, 3, 2));
        let d = parse_status("# branch.oid 1234\n# branch.head (detached)\n");
        assert_eq!((d.branch, d.upstream, d.changed), (None, None, 0));
    }

    #[test]
    fn reads_the_last_commit() {
        let c = parse_commit("ab12cd3\u{1f}Fix login\u{1f}Dana\u{1f}1700000000\n").unwrap();
        assert_eq!(c.hash, "ab12cd3");
        assert_eq!(c.subject, "Fix login");
        assert_eq!(c.time, 1_700_000_000_000);
        assert!(parse_commit("").is_none());
    }

    #[test]
    fn spots_dependency_files() {
        assert!(lists_dependencies("package-lock.json"));
        assert!(lists_dependencies("apps/web/pnpm-lock.yaml"));
        assert!(lists_dependencies("api/Api.csproj"));
        assert!(lists_dependencies("requirements-dev.txt"));
        assert!(!lists_dependencies("src/package.ts"));
        assert!(!lists_dependencies("README.md"));
    }

    /// A real repository: a clone is behind until it pulls, and a lockfile change is spotted.
    #[test]
    fn pulls_a_fast_forward() {
        let root = std::env::temp_dir().join(format!("keeprun-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (origin, clone) = (root.join("origin"), root.join("clone"));
        std::fs::create_dir_all(&origin).unwrap();
        let run = |dir: &Path, args: &[&str]| git(dir, args, false, QUICK).unwrap();
        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["config", "user.email", "t@example.com"]);
        run(&origin, &["config", "user.name", "T"]);
        std::fs::write(origin.join("server.js"), "1").unwrap();
        run(&origin, &["add", "."]);
        run(&origin, &["commit", "-q", "-m", "First"]);
        run(&root, &["clone", "-q", origin.to_str().unwrap(), "clone"]);

        let s = git_status(clone.to_string_lossy().into()).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.commit.as_ref().unwrap().subject, "First");

        std::fs::write(origin.join("package-lock.json"), "{}").unwrap();
        run(&origin, &["add", "."]);
        run(&origin, &["commit", "-q", "-m", "Lock"]);
        let fetched = git_fetch(clone.to_string_lossy().into()).unwrap();
        assert_eq!(fetched.behind, 1);

        let p = git_pull(clone.to_string_lossy().into()).unwrap();
        assert_eq!((p.commits, p.dependencies, p.status.behind), (1, true, 0));
        let again = git_pull(clone.to_string_lossy().into()).unwrap();
        assert_eq!((again.commits, again.dependencies), (0, false));

        assert!(git_status(root.to_string_lossy().into()).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
