//! Git state of an app folder, and pulling it. The desktop app and the CLI run this as the
//! signed-in user, never the agent: the service account has none of the user's credentials,
//! and Git refuses repositories owned by another account.

use serde::Serialize;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How the branch compares with its remote, from the remote-tracking refs of the last fetch.
#[derive(Serialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sync {
    /// Same commit as the upstream branch.
    #[default]
    InSync,
    /// The upstream has commits to pull.
    Behind,
    /// Local commits not pushed.
    Ahead,
    /// Both: a pull cannot fast-forward.
    Diverged,
    /// Nothing to pull or push, and every commit is already in the remote's default branch.
    Merged,
    /// The upstream branch was deleted on the remote, often after its pull request merged.
    Gone,
    /// The branch tracks no remote branch.
    NoUpstream,
    /// The repository has no remote.
    NoRemote,
    Detached,
}

#[derive(Serialize, Debug, Default, Clone, PartialEq)]
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
    pub sync: Sync,
    /// The remote's default branch, e.g. "origin/main". None when unknown.
    pub default_branch: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
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
    /// A migration was added or changed, so the database may need migrating.
    pub migrations: bool,
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
/// A background fetch: short, since nobody is waiting on it and it must never pile up.
const FETCH: Duration = Duration::from_secs(45);

fn parse_status(text: &str) -> GitStatus {
    let mut s = GitStatus::default();
    let mut counted = false;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            s.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
            s.upstream = Some(up.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            counted = true;
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
    s.sync = match (&s.branch, &s.upstream) {
        (None, _) => Sync::Detached,
        (Some(_), None) => Sync::NoUpstream,
        // Git leaves out the counts when the upstream ref no longer exists.
        (Some(_), Some(_)) if !counted => Sync::Gone,
        _ => match (s.ahead, s.behind) {
            (0, 0) => Sync::InSync,
            (_, 0) => Sync::Ahead,
            (0, _) => Sync::Behind,
            _ => Sync::Diverged,
        },
    };
    s
}

/// The remote's default branch as a remote-tracking ref, e.g. "origin/main": what
/// `<remote>/HEAD` points to, else `<remote>/main` or `<remote>/master` when it exists.
fn default_branch(cwd: &Path, remote: &str) -> Option<String> {
    let head = format!("refs/remotes/{remote}/HEAD");
    if let Ok(r) = git(
        cwd,
        &["symbolic-ref", "--quiet", "--short", &head],
        false,
        QUICK,
    ) {
        let r = r.trim();
        if !r.is_empty() {
            return Some(r.to_string());
        }
    }
    ["main", "master"].iter().find_map(|b| {
        let r = format!("refs/remotes/{remote}/{b}");
        git(cwd, &["rev-parse", "--verify", "--quiet", &r], false, QUICK)
            .ok()
            .map(|_| format!("{remote}/{b}"))
    })
}

/// Fill in the remote, the default branch and whether the branch is merged into it.
fn compare_with_remote(cwd: &Path, s: &mut GitStatus) {
    let remotes = git(cwd, &["remote"], false, QUICK).unwrap_or_default();
    let remotes: Vec<&str> = remotes
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    // The upstream's remote, else origin, else the only one.
    let remote = s
        .upstream
        .as_deref()
        .and_then(|u| remotes.iter().find(|r| u.starts_with(&format!("{r}/"))))
        .or_else(|| remotes.iter().find(|r| **r == "origin"))
        .or(remotes.first())
        .copied();
    let Some(remote) = remote else {
        if s.sync == Sync::NoUpstream {
            s.sync = Sync::NoRemote;
        }
        return;
    };
    s.default_branch = default_branch(cwd, remote);
    let (Some(branch), Some(default)) = (&s.branch, &s.default_branch) else {
        return;
    };
    // The default branch itself is never "merged into" itself.
    let tracks_default = s.upstream.as_ref() == Some(default)
        || default.strip_prefix(&format!("{remote}/")) == Some(branch.as_str());
    let settled = matches!(s.sync, Sync::InSync | Sync::Gone | Sync::NoUpstream);
    if !tracks_default
        && settled
        && git(
            cwd,
            &["merge-base", "--is-ancestor", "HEAD", default],
            false,
            QUICK,
        )
        .is_ok()
    {
        s.sync = Sync::Merged;
    }
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

/// Git state of the folder, without network access. Fails when it is not in a Git repository
/// or Git is not installed ("no-git").
pub fn status(cwd: &Path) -> Result<GitStatus, String> {
    let text = git(cwd, &["status", "--porcelain=v2", "--branch"], false, QUICK)?;
    let mut s = parse_status(&text);
    compare_with_remote(cwd, &mut s);
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

/// Fetch without asking for credentials, then return the state. Only updates remote-tracking
/// refs, never the working tree. `--prune` drops refs of branches deleted on the remote, so a
/// merged and deleted branch shows as gone, not as in sync with a stale ref.
pub fn fetch(cwd: &Path) -> Result<GitStatus, String> {
    git(
        cwd,
        &["fetch", "--quiet", "--no-tags", "--prune"],
        false,
        FETCH,
    )?;
    status(cwd)
}

/// Fast-forward the branch to its upstream. Never merges or rebases: diverged branches and
/// local changes in the way are reported, and nothing changes. `interactive` lets Git ask for
/// credentials.
pub fn pull(cwd: &Path, interactive: bool) -> Result<Pulled, String> {
    let before = git(cwd, &["rev-parse", "HEAD"], false, QUICK)?;
    git(
        cwd,
        &["pull", "--ff-only", "--no-rebase", "--quiet", "--prune"],
        interactive,
        NETWORK,
    )?;
    let after = git(cwd, &["rev-parse", "HEAD"], false, QUICK)?;
    let (before, after) = (before.trim(), after.trim());
    let (commits, dependencies, migrations) = if before == after {
        (0, false, false)
    } else {
        let range = format!("{before}..{after}");
        let commits = git(cwd, &["rev-list", "--count", &range], false, QUICK)?
            .trim()
            .parse()
            .unwrap_or(0);
        // Paths relative to the app folder's repository root.
        let files = git(cwd, &["diff", "--name-only", before, after], false, QUICK)?;
        (
            commits,
            files.lines().any(lists_dependencies),
            files.lines().any(crate::migrate::is_migration_path),
        )
    };
    Ok(Pulled {
        commits,
        dependencies,
        migrations,
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
        assert_eq!(d.sync, Sync::Detached);
        let gone = parse_status("# branch.head feature\n# branch.upstream origin/feature\n");
        assert_eq!(gone.sync, Sync::Gone);
    }

    /// A remote with a default branch, and a clone of it.
    struct Repos {
        root: std::path::PathBuf,
        origin: std::path::PathBuf,
        clone: std::path::PathBuf,
    }

    impl Repos {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("keeprun-sync-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let (origin, clone) = (root.join("origin"), root.join("clone"));
            std::fs::create_dir_all(&origin).unwrap();
            let r = Repos {
                root,
                origin,
                clone,
            };
            r.run(&r.origin, &["init", "-q", "-b", "main"]);
            r.commit(&r.origin, "first");
            r.run(
                &r.root,
                &["clone", "-q", r.origin.to_str().unwrap(), "clone"],
            );
            r
        }

        fn run(&self, dir: &Path, args: &[&str]) -> String {
            git(dir, args, false, QUICK).unwrap()
        }

        fn commit(&self, dir: &Path, name: &str) {
            std::fs::write(dir.join(name), name).unwrap();
            self.run(dir, &["add", "."]);
            self.run(
                dir,
                &[
                    "-c",
                    "user.email=t@example.com",
                    "-c",
                    "user.name=T",
                    "commit",
                    "-q",
                    "-m",
                    name,
                ],
            );
        }

        fn sync(&self) -> Sync {
            fetch(&self.clone).unwrap().sync
        }
    }

    impl Drop for Repos {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn sync_states_come_from_real_comparisons() {
        let r = Repos::new("states");
        let s = fetch(&r.clone).unwrap();
        assert_eq!(s.sync, Sync::InSync);
        assert_eq!(s.default_branch.as_deref(), Some("origin/main"));

        r.commit(&r.origin, "remote");
        assert_eq!(r.sync(), Sync::Behind);
        r.run(&r.clone, &["pull", "-q", "--ff-only"]);
        assert_eq!(r.sync(), Sync::InSync);

        r.commit(&r.clone, "local");
        assert_eq!(r.sync(), Sync::Ahead);
        r.commit(&r.origin, "remote2");
        let s = fetch(&r.clone).unwrap();
        assert_eq!((s.sync, s.ahead, s.behind), (Sync::Diverged, 1, 1));

        r.run(&r.clone, &["checkout", "-q", "--detach"]);
        assert_eq!(r.sync(), Sync::Detached);
    }

    /// The real case: a feature branch equal to its remote branch, already merged into main.
    #[test]
    fn a_merged_branch_says_merged_and_a_deleted_one_gone() {
        let r = Repos::new("merged");
        r.run(&r.clone, &["checkout", "-q", "-b", "feature"]);
        r.commit(&r.clone, "feature");
        r.run(&r.clone, &["push", "-q", "-u", "origin", "feature"]);
        assert_eq!(r.sync(), Sync::InSync);

        r.run(
            &r.origin,
            &["merge", "-q", "--no-ff", "-m", "Merge", "feature"],
        );
        assert_eq!(r.sync(), Sync::Merged);

        // Deleted on the remote after the merge: the prune drops the stale ref.
        r.run(&r.origin, &["branch", "-q", "-D", "feature"]);
        let s = fetch(&r.clone).unwrap();
        assert_eq!(s.upstream.as_deref(), Some("origin/feature"));
        assert_eq!(s.sync, Sync::Merged);

        // Deleted without being merged, e.g. after a squash merge or a closed pull request.
        r.run(&r.clone, &["checkout", "-q", "-b", "other"]);
        r.commit(&r.clone, "other");
        r.run(&r.clone, &["push", "-q", "-u", "origin", "other"]);
        assert_eq!(r.sync(), Sync::InSync);
        r.run(&r.origin, &["branch", "-q", "-D", "other"]);
        assert_eq!(r.sync(), Sync::Gone);
    }

    #[test]
    fn no_upstream_and_no_remote_are_states() {
        let r = Repos::new("upstream");
        r.run(&r.clone, &["checkout", "-q", "-b", "local-only"]);
        r.commit(&r.clone, "x");
        assert_eq!(status(&r.clone).unwrap().sync, Sync::NoUpstream);

        r.run(&r.clone, &["remote", "remove", "origin"]);
        let s = status(&r.clone).unwrap();
        assert_eq!((s.sync, s.default_branch), (Sync::NoRemote, None));
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

        let s = status(&clone).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.commit.as_ref().unwrap().subject, "First");

        std::fs::write(origin.join("package-lock.json"), "{}").unwrap();
        std::fs::create_dir_all(origin.join("prisma/migrations/2_add")).unwrap();
        std::fs::write(origin.join("prisma/migrations/2_add/migration.sql"), "").unwrap();
        run(&origin, &["add", "."]);
        run(&origin, &["commit", "-q", "-m", "Lock"]);
        let fetched = fetch(&clone).unwrap();
        assert_eq!(fetched.behind, 1);

        let p = pull(&clone, false).unwrap();
        assert_eq!((p.commits, p.dependencies, p.status.behind), (1, true, 0));
        assert!(p.migrations);
        let again = pull(&clone, false).unwrap();
        assert_eq!((again.commits, again.dependencies), (0, false));

        assert!(status(&root).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
