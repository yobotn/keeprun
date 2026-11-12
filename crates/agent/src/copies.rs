//! Copies of an app started outside Keeprun, e.g. `npm run dev` in a terminal or by a coding
//! agent. Two copies fight over the port and files, and the output of the outside copy never
//! reaches Keeprun. Keeprun ends outside copies when it starts the app and while it runs.
//!
//! A copy is a process outside every Keeprun job that listens on a TCP port and runs from the
//! app folder: its command line names a file there, like
//! `node C:\app\node_modules\next\...\start-server.js`, or it is a runtime such as node.exe or
//! python.exe started in the folder (`node server.js`). Editors and language servers do not
//! listen, and a shell or coding agent working in the folder is not a runtime. The copy ends
//! from its topmost parent that also runs from the folder, with every process under it.

pub use crate::job::ProcInfo;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A process from a system snapshot.
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

/// Command line, start time and current folder of a process, None when unreadable.
pub type Probe<'a> = &'a dyn Fn(u32) -> Option<ProcInfo>;

/// Programs that run an app's code. One of these started in the app folder runs the app,
/// while a shell or coding agent there only works on it.
const RUNTIMES: [&str; 11] = [
    "node.exe",
    "bun.exe",
    "deno.exe",
    "python.exe",
    "pythonw.exe",
    "dotnet.exe",
    "java.exe",
    "javaw.exe",
    "ruby.exe",
    "php.exe",
    "php-cgi.exe",
];

/// One outside copy: the process it ends from and every process in it, children first.
#[derive(Debug, PartialEq)]
pub struct Copy {
    pub root: u32,
    pub name: String,
    pub pids: Vec<u32>,
}

/// Lowercase, forward slashes as backslashes, repeated backslashes collapsed.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = if c == '/' { '\\' } else { c };
        if !(c == '\\' && out.ends_with('\\')) {
            out.push(c);
        }
    }
    out
}

/// `C:\app` as `c:\app\`, the prefix every file inside it starts with.
pub fn folder_key(folder: &Path) -> String {
    let raw = folder.to_string_lossy();
    let mut k = normalize(raw.strip_prefix(r"\\?\").unwrap_or(&raw));
    if !k.ends_with('\\') {
        k.push('\\');
    }
    k
}

/// Does the command line name a file in `folder`, and not in a managed app folder inside it?
fn names_folder(cmd: &str, folder: &str, others: &[String]) -> bool {
    let cmd = normalize(cmd);
    cmd.contains(folder)
        && !others
            .iter()
            .any(|o| o.len() > folder.len() && o.starts_with(folder) && cmd.contains(o.as_str()))
}

/// Is `dir` the app folder or inside it, and not in a managed app folder inside it?
fn in_folder(dir: &str, folder: &str, others: &[String]) -> bool {
    let dir = folder_key(Path::new(dir));
    dir.starts_with(folder)
        && !others
            .iter()
            .any(|o| o.len() > folder.len() && o.starts_with(folder) && dir.starts_with(o.as_str()))
}

/// Does this process run from the app folder?
fn runs_from(name: &str, info: &ProcInfo, folder: &str, others: &[String]) -> bool {
    // A script from a managed app folder inside this one belongs to that app.
    let cmd = normalize(&info.cmd);
    if others
        .iter()
        .any(|o| o.len() > folder.len() && o.starts_with(folder) && cmd.contains(o.as_str()))
    {
        return false;
    }
    names_folder(&info.cmd, folder, others)
        || (RUNTIMES.iter().any(|r| r.eq_ignore_ascii_case(name))
            && info
                .cwd
                .as_deref()
                .is_some_and(|d| in_folder(d, folder, others)))
}

/// Outside copies of the app in `folder` (a [`folder_key`]). `others` are the other managed
/// app folders, `own` the processes in Keeprun jobs, `listening` the pids with a TCP listener.
pub fn find(
    procs: &[Proc],
    folder: &str,
    others: &[String],
    own: &HashSet<u32>,
    listening: &HashSet<u32>,
    probe: Probe,
) -> Vec<Copy> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let me = std::process::id();
    let outside = |pid: u32| pid > 4 && pid != me && !own.contains(&pid);
    let matches = |pid: u32| {
        let name = &by_pid.get(&pid)?.name;
        probe(pid)
            .filter(|i| runs_from(name, i, folder, others))
            .map(|i| i.started)
    };

    let mut roots: Vec<u32> = vec![];
    for &pid in listening {
        if !outside(pid) || !by_pid.contains_key(&pid) {
            continue;
        }
        let Some(mut started) = matches(pid) else {
            continue;
        };
        // Climb to the topmost parent that is part of the same copy. A parent started after
        // its child is a reused pid, not the real parent.
        let mut root = pid;
        for _ in 0..64 {
            let parent = by_pid[&root].parent;
            if !outside(parent) || !by_pid.contains_key(&parent) {
                break;
            }
            match matches(parent) {
                Some(t) if t <= started => {
                    root = parent;
                    started = t;
                }
                _ => break,
            }
        }
        if !roots.contains(&root) {
            roots.push(root);
        }
    }

    // A root inside another copy is already covered by that copy.
    let mut copies: Vec<Copy> = roots
        .iter()
        .map(|&root| Copy {
            root,
            name: by_pid[&root].name.clone(),
            pids: tree(procs, root, &outside, probe),
        })
        .collect();
    let all: Vec<(u32, HashSet<u32>)> = copies
        .iter()
        .map(|c| (c.root, c.pids.iter().copied().collect()))
        .collect();
    copies.retain(|c| {
        !all.iter()
            .any(|(r, set)| *r != c.root && set.contains(&c.root))
    });
    copies.sort_by_key(|c| c.root);
    copies
}

/// `root` and every process under it outside Keeprun, deepest first.
fn tree(procs: &[Proc], root: u32, outside: &dyn Fn(u32) -> bool, probe: Probe) -> Vec<u32> {
    let started = |pid| probe(pid).map(|i| i.started);
    let mut order = vec![root];
    let mut i = 0;
    while i < order.len() {
        let parent = order[i];
        let parent_started = started(parent);
        for p in procs {
            if p.parent == parent && outside(p.pid) && !order.contains(&p.pid) {
                // Skip a process whose parent pid was reused after it started.
                let ok = match (parent_started, started(p.pid)) {
                    (Some(a), Some(b)) => b >= a,
                    _ => true,
                };
                if ok {
                    order.push(p.pid);
                }
            }
        }
        i += 1;
    }
    order.reverse();
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOP: &str = r"C:\Users\me\Desktop\shop";

    fn snapshot() -> (Vec<Proc>, HashMap<u32, ProcInfo>) {
        // The tree a coding agent left behind with `npm run dev`, as seen on a real machine.
        let rows: &[(u32, u32, &str, &str, u64)] = &[
            (25944, 1, "bash.exe", "bash", 1),
            (
                21328,
                25944,
                "rtk.exe",
                r"C:\Users\me\.local\bin\rtk.exe npm run dev",
                2,
            ),
            (
                23500,
                21328,
                "cmd.exe",
                r#"cmd.exe /d /c ""C:\Program Files\nodejs\npm.cmd" run dev""#,
                3,
            ),
            (
                4108,
                23500,
                "node.exe",
                r#""C:\Program Files\nodejs\node.exe" "C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js" run dev"#,
                4,
            ),
            (
                12260,
                4108,
                "cmd.exe",
                r"C:\Windows\system32\cmd.exe /d /s /c next dev",
                5,
            ),
            (
                21224,
                12260,
                "node.exe",
                r#""node"   "C:\Users\me\Desktop\shop\node_modules\.bin\\..\next\dist\bin\next" dev"#,
                6,
            ),
            (
                16376,
                21224,
                "node.exe",
                r#""C:\Program Files\nodejs\node.exe" C:\Users\me\Desktop\shop\node_modules\next\dist\server\lib\start-server.js"#,
                7,
            ),
            (
                19860,
                16376,
                "node.exe",
                r#""node" C:\Users\me\Desktop\shop\.next\dev\build\chunks\pool.js 58462"#,
                8,
            ),
            // An editor language server for the same folder: it does not listen.
            (
                9000,
                1,
                "node.exe",
                r"node C:\Users\me\Desktop\shop\node_modules\typescript\lib\tsserver.js",
                9,
            ),
            // Another project's dev server.
            (
                9100,
                1,
                "node.exe",
                r"node C:\Users\me\Desktop\other\node_modules\next\dist\server\lib\start-server.js",
                9,
            ),
        ];
        let procs = rows
            .iter()
            .map(|r| Proc {
                pid: r.0,
                parent: r.1,
                name: r.2.into(),
            })
            .collect();
        // The dev server runs in its app folder, and so does everything around it.
        let info = rows
            .iter()
            .map(|r| {
                let cwd = (r.0 != 9100).then(|| SHOP.to_string());
                let cmd = r.3.to_string();
                (
                    r.0,
                    ProcInfo {
                        cmd,
                        started: r.4,
                        cwd,
                    },
                )
            })
            .collect();
        (procs, info)
    }

    fn run(own: &[u32], others: &[&str]) -> Vec<Copy> {
        let (procs, info) = snapshot();
        let probe = move |pid: u32| info.get(&pid).cloned();
        let others: Vec<String> = others.iter().map(|o| folder_key(Path::new(o))).collect();
        find(
            &procs,
            &folder_key(Path::new(SHOP)),
            &others,
            &own.iter().copied().collect(),
            &[16376, 9000u32, 9100]
                .into_iter()
                .filter(|p| *p != 9000)
                .collect(),
            &probe,
        )
    }

    #[test]
    fn ends_the_dev_server_from_its_topmost_process_in_the_folder() {
        assert_eq!(
            run(&[], &[]),
            vec![Copy {
                root: 21224,
                name: "node.exe".into(),
                pids: vec![19860, 16376, 21224]
            }]
        );
    }

    #[test]
    fn leaves_processes_in_keeprun_jobs_alone() {
        assert!(run(&[16376], &[]).is_empty());
    }

    #[test]
    fn leaves_a_managed_app_in_a_subfolder_alone() {
        assert!(run(&[], &[r"C:\Users\me\Desktop\shop\node_modules"]).is_empty());
    }

    #[test]
    fn finds_a_runtime_started_in_the_folder_with_a_relative_script() {
        // `npm start` runs `node server.js`: only the current folder places it.
        let procs: Vec<Proc> = [
            (10, 1, "claude.exe"),
            (11, 10, "bash.exe"),
            (12, 11, "cmd.exe"),
            (13, 12, "node.exe"),
            // A tool that listens and was started in the folder, but is not a runtime.
            (14, 11, "caddy.exe"),
        ]
        .into_iter()
        .map(|(pid, parent, name)| Proc {
            pid,
            parent,
            name: name.into(),
        })
        .collect();
        let probe = |pid: u32| {
            let cmd = match pid {
                12 => "cmd /d /s /c node server.js",
                13 => "node server.js",
                _ => "x",
            };
            Some(ProcInfo {
                cmd: cmd.into(),
                started: pid as u64,
                cwd: Some(SHOP.into()),
            })
        };
        let found = find(
            &procs,
            &folder_key(Path::new(SHOP)),
            &[],
            &HashSet::new(),
            &[10, 13, 14].into_iter().collect(),
            &probe,
        );
        assert_eq!(
            found,
            vec![Copy {
                root: 13,
                name: "node.exe".into(),
                pids: vec![13]
            }]
        );
    }

    #[test]
    fn folder_matching_ignores_case_and_slashes() {
        let k = folder_key(Path::new(r"\\?\C:\Apps\Shop\"));
        assert_eq!(k, r"c:\apps\shop\");
        assert!(names_folder("node c:/APPS//shop/server.js", &k, &[]));
        assert!(!names_folder(r"node C:\Apps\Shopping\server.js", &k, &[]));
    }
}
