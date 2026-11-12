//! Opens a source file from app output at a line, in the user's code editor.
//! Editors are started through their command line launcher (`code --goto file:line:col`),
//! found on PATH or in the default install folder. Without one, the file opens in the
//! default app for its type, without the line.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Editor launchers by id: the file name on PATH, and install folders under
/// %LOCALAPPDATA% and %ProgramFiles%.
const EDITORS: &[(&str, &str, &[&str])] = &[
    (
        "vscode",
        "code.cmd",
        &[r"Programs\Microsoft VS Code\bin", r"Microsoft VS Code\bin"],
    ),
    (
        "cursor",
        "cursor.cmd",
        &[
            r"Programs\cursor\resources\app\bin",
            r"cursor\resources\app\bin",
        ],
    ),
    (
        "windsurf",
        "windsurf.cmd",
        &[r"Programs\Windsurf\bin", r"Windsurf\bin"],
    ),
];

/// Turn a path as printed by the app into a file on disk. Relative paths, and the
/// root-relative ones Vite prints (`/src/App.tsx`), are relative to the app folder.
fn resolve(cwd: &Path, printed: &str) -> PathBuf {
    let mut p = printed;
    if let Some(rest) = p.strip_prefix("file://") {
        p = rest.strip_prefix('/').unwrap_or(rest);
    }
    let p = percent_decode(p).replace('/', "\\");
    let is_absolute = p.starts_with(r"\\") || (p.len() > 2 && p.as_bytes()[1] == b':');
    if is_absolute {
        PathBuf::from(p)
    } else {
        let p = p.strip_prefix(".\\").unwrap_or(&p);
        cwd.join(p.trim_start_matches('\\'))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find_launcher(file: &str, folders: &[&str]) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join(file));
    let installed = ["LOCALAPPDATA", "ProgramFiles"]
        .iter()
        .filter_map(std::env::var_os)
        .flat_map(|root| {
            folders
                .iter()
                .map(move |f| PathBuf::from(&root).join(f).join(file))
        });
    on_path.chain(installed).find(|f| f.is_file())
}

/// Errors the UI translates: `not-found` and `no-editor`. Anything else is shown as is.
#[tauri::command]
pub fn open_in_editor(
    app: tauri::AppHandle,
    cwd: PathBuf,
    path: String,
    line: u32,
    col: Option<u32>,
    editor: String,
) -> Result<(), String> {
    let file = resolve(&cwd, &path);
    if !file.is_file() {
        return Err("not-found".into());
    }
    if editor != "default" {
        let wanted: Vec<_> = EDITORS
            .iter()
            .filter(|(id, ..)| editor == "auto" || editor == *id)
            .collect();
        if let Some(launcher) = wanted
            .iter()
            .find_map(|(_, f, dirs)| find_launcher(f, dirs))
        {
            let at = format!(
                "{}:{}:{}",
                file.display(),
                line.max(1),
                col.unwrap_or(1).max(1)
            );
            let mut cmd = Command::new(launcher);
            cmd.arg("--goto").arg(at);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            return cmd.spawn().map(|_| ()).map_err(|e| e.to_string());
        }
        if editor != "auto" {
            return Err("no-editor".into());
        }
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_printed_paths() {
        let cwd = Path::new(r"C:\apps\web");
        assert_eq!(
            resolve(cwd, "src/a.ts"),
            PathBuf::from(r"C:\apps\web\src\a.ts")
        );
        assert_eq!(
            resolve(cwd, "./src/a.ts"),
            PathBuf::from(r"C:\apps\web\src\a.ts")
        );
        assert_eq!(
            resolve(cwd, "/src/App.tsx"),
            PathBuf::from(r"C:\apps\web\src\App.tsx")
        );
        assert_eq!(resolve(cwd, r"D:\x\y.js"), PathBuf::from(r"D:\x\y.js"));
        assert_eq!(
            resolve(cwd, "file:///D:/My%20App/y.js"),
            PathBuf::from(r"D:\My App\y.js")
        );
        assert_eq!(
            resolve(cwd, r"\\srv\share\a.py"),
            PathBuf::from(r"\\srv\share\a.py")
        );
    }
}
