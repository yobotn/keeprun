//! Detect what kind of app a folder holds and suggest commands to run it.
//!
//! Runs in the user's context (desktop UI, CLI), not in the agent: the agent runs as
//! LocalSystem and cannot see per-user tool installs on its PATH. Commands use absolute
//! executable paths so they work under any account.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Node,
    Dotnet,
    Python,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    /// Short label for a picker, e.g. "npm run start".
    pub label: String,
    /// Full command line with absolute paths where they were found.
    pub command: String,
    /// Extra detail, e.g. the script body.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    pub kind: Kind,
    /// Suggested app name.
    pub name: String,
    /// Best first. May be empty for Custom.
    pub suggestions: Vec<Suggestion>,
    /// Port the app probably listens on, when it can be read from config.
    pub port: Option<u16>,
    /// Values from a `.env` file in the folder, offered for import.
    pub dotenv: BTreeMap<String, String>,
    /// Problems the user should know about, e.g. a tool that is not installed.
    pub warnings: Vec<String>,
}

/// Find an executable on PATH, trying PATHEXT extensions. Returns the absolute path.
pub fn which(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_ascii_lowercase())
        .collect();
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Quote a path for cmd.exe when it contains spaces.
pub fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s
    }
}

fn tool(name: &str, warnings: &mut Vec<String>) -> String {
    match which(name) {
        Some(p) => quote(&p),
        None => {
            warnings.push(format!(
                "{name} was not found on PATH. Install it or edit the command to use a full path."
            ));
            name.to_string()
        }
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".into())
}

fn read_dotenv(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(dir.join(".env"))
        .ok()
        .and_then(|t| crate::model::parse_dotenv(&t).ok())
        .unwrap_or_default()
}

/// Find `--port N`, `-p N`, `PORT=N` or `:N` style port hints in a string.
fn port_in(text: &str) -> Option<u16> {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '=')
        .collect();
    for w in words.windows(2) {
        if matches!(w[0], "--port" | "-p" | "PORT" | "--urls") {
            if let Some(p) = w[1]
                .rsplit(':')
                .next()
                .and_then(|s| s.trim_matches('"').parse().ok())
            {
                return Some(p);
            }
        }
    }
    None
}

pub fn detect(dir: &Path) -> Result<Detection, String> {
    if !dir.is_dir() {
        return Err(format!(
            "Folder {} does not exist. Pick an existing folder.",
            dir.display()
        ));
    }
    let dotenv = read_dotenv(dir);
    let env_port = dotenv.get("PORT").and_then(|p| p.parse().ok());
    let mut d = if dir.join("package.json").is_file() {
        detect_node(dir)
    } else if let Some(proj) = find_ext(dir, "csproj").or_else(|| find_ext(dir, "fsproj")) {
        detect_dotnet(dir, &proj)
    } else if is_python(dir) {
        detect_python(dir)
    } else {
        detect_custom(dir)
    };
    d.port = d.port.or(env_port);
    d.dotenv = dotenv;
    Ok(d)
}

fn find_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
        .collect();
    found.sort();
    found.into_iter().next()
}

fn detect_node(dir: &Path) -> Detection {
    let mut warnings = vec![];
    let pkg: serde_json::Value = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    let name = pkg["name"]
        .as_str()
        .map(|n| n.rsplit('/').next().unwrap_or(n).to_string())
        .unwrap_or_else(|| folder_name(dir));

    let pm = if dir.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if dir.join("yarn.lock").is_file() {
        "yarn"
    } else if dir.join("bun.lockb").is_file() || dir.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    };
    let pm_path = tool(pm, &mut warnings);
    let has_deps = ["dependencies", "devDependencies"]
        .iter()
        .any(|k| pkg[k].as_object().is_some_and(|m| !m.is_empty()));
    if has_deps && !dir.join("node_modules").is_dir() {
        warnings.push(format!(
            "node_modules is missing. Run \"{pm} install\" in the folder before starting."
        ));
    }

    // Production style scripts first: an internal app host should prefer "start".
    let order = ["start", "serve", "preview", "prod", "dev"];
    let mut scripts: Vec<(String, String)> = pkg["scripts"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    scripts.retain(|(k, _)| !k.starts_with("pre") && !k.starts_with("post"));
    scripts.sort_by_key(|(k, _)| order.iter().position(|o| o == k).unwrap_or(order.len()));

    let mut port = None;
    let mut suggestions: Vec<Suggestion> = scripts
        .iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "test" | "lint" | "build" | "format" | "typecheck" | "check"
            )
        })
        .map(|(k, body)| {
            if port.is_none() {
                port = port_in(body);
            }
            let run = if pm == "npm" || pm == "bun" {
                format!("{pm} run {k}")
            } else {
                format!("{pm} {k}")
            };
            let command = if pm == "npm" || pm == "bun" {
                format!("{pm_path} run {k}")
            } else {
                format!("{pm_path} {k}")
            };
            Suggestion {
                label: run,
                command,
                detail: Some(body.clone()),
            }
        })
        .collect();

    let main = pkg["main"].as_str().map(str::to_string).or_else(|| {
        ["server.js", "index.js", "app.js", "main.js"]
            .iter()
            .find(|f| dir.join(f).is_file())
            .map(|s| s.to_string())
    });
    if let Some(main) = main.filter(|m| dir.join(m).is_file()) {
        let node = tool("node", &mut warnings);
        suggestions.push(Suggestion {
            label: format!("node {main}"),
            command: format!("{node} {main}"),
            detail: None,
        });
    }
    warnings.dedup();
    Detection {
        kind: Kind::Node,
        name,
        suggestions,
        port,
        dotenv: BTreeMap::new(),
        warnings,
    }
}

fn detect_dotnet(dir: &Path, proj: &Path) -> Detection {
    let mut warnings = vec![];
    let stem = proj
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dotnet = tool("dotnet", &mut warnings);
    let mut suggestions = vec![];

    // A published build is faster to start and does not need the SDK.
    for rel in ["publish", "bin/Release/publish"] {
        let base = dir.join(rel);
        let exe = base.join(format!("{stem}.exe"));
        let dll = base.join(format!("{stem}.dll"));
        if exe.is_file() {
            suggestions.push(Suggestion {
                label: format!("{rel}/{stem}.exe"),
                command: quote(&exe),
                detail: None,
            });
        } else if dll.is_file() {
            suggestions.push(Suggestion {
                label: format!("dotnet {rel}/{stem}.dll"),
                command: format!("{dotnet} {}", quote(&dll)),
                detail: None,
            });
        }
    }
    let file = proj.file_name().unwrap().to_string_lossy().to_string();
    suggestions.push(Suggestion {
        label: "dotnet run -c Release".into(),
        command: format!("{dotnet} run -c Release --project {file}"),
        detail: None,
    });

    let port = std::fs::read_to_string(dir.join("Properties").join("launchSettings.json"))
        .ok()
        .and_then(|t| {
            let i = t.find("applicationUrl")?;
            let rest = &t[i..];
            let url = rest.split('"').nth(2)?;
            url.split(';')
                .find(|u| u.starts_with("http://"))
                .or(url.split(';').next())?
                .rsplit(':')
                .next()?
                .trim_end_matches('/')
                .parse()
                .ok()
        });
    Detection {
        kind: Kind::Dotnet,
        name: stem,
        suggestions,
        port,
        dotenv: BTreeMap::new(),
        warnings,
    }
}

fn is_python(dir: &Path) -> bool {
    [
        "requirements.txt",
        "pyproject.toml",
        "manage.py",
        "main.py",
        "app.py",
        "Pipfile",
    ]
    .iter()
    .any(|f| dir.join(f).is_file())
}

fn detect_python(dir: &Path) -> Detection {
    let mut warnings = vec![];
    let venv_python = [".venv", "venv", "env"]
        .iter()
        .map(|v| dir.join(v).join("Scripts").join("python.exe"))
        .find(|p| p.is_file());
    let python = match &venv_python {
        Some(p) => quote(p),
        None => {
            warnings.push(
                "No virtual environment found (.venv, venv). Using the system Python.".into(),
            );
            tool("python", &mut warnings)
        }
    };
    let deps = ["requirements.txt", "pyproject.toml", "Pipfile"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .collect::<String>()
        .to_ascii_lowercase();

    let mut suggestions = vec![];
    let mut port = None;
    if dir.join("manage.py").is_file() {
        suggestions.push(Suggestion {
            label: "Django (manage.py runserver)".into(),
            command: format!("{python} manage.py runserver 127.0.0.1:8000 --noreload"),
            detail: None,
        });
        port = Some(8000);
    }
    for entry in ["main.py", "app.py", "server.py"] {
        if !dir.join(entry).is_file() {
            continue;
        }
        let module = entry.trim_end_matches(".py");
        if deps.contains("uvicorn") || deps.contains("fastapi") {
            suggestions.push(Suggestion {
                label: format!("uvicorn {module}:app"),
                command: format!("{python} -m uvicorn {module}:app --host 127.0.0.1 --port 8000"),
                detail: None,
            });
            port = port.or(Some(8000));
        }
        if deps.contains("flask") {
            suggestions.push(Suggestion {
                label: format!("flask --app {module} run"),
                command: format!("{python} -m flask --app {module} run --port 5000"),
                detail: None,
            });
            port = port.or(Some(5000));
        }
        suggestions.push(Suggestion {
            label: format!("python {entry}"),
            command: format!("{python} {entry}"),
            detail: None,
        });
    }
    Detection {
        kind: Kind::Python,
        name: folder_name(dir),
        suggestions,
        port,
        dotenv: BTreeMap::new(),
        warnings,
    }
}

fn detect_custom(dir: &Path) -> Detection {
    let mut suggestions = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| {
                    ["exe", "bat", "cmd", "ps1"]
                        .iter()
                        .any(|x| e.eq_ignore_ascii_case(x))
                })
            })
            .collect();
        files.sort();
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().to_string();
            let command = if name.to_ascii_lowercase().ends_with(".ps1") {
                format!(
                    "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
                    quote(&f)
                )
            } else {
                quote(&f)
            };
            suggestions.push(Suggestion {
                label: name,
                command,
                detail: None,
            });
        }
    }
    Detection {
        kind: Kind::Custom,
        name: folder_name(dir),
        suggestions,
        port: None,
        dotenv: BTreeMap::new(),
        warnings: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("keeprun-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn node_prefers_start_and_finds_port() {
        let d = tmp("node");
        std::fs::write(
            d.join("package.json"),
            r#"{"name":"@acme/hr-portal","scripts":{"dev":"vite --port 5173","start":"node server.js --port 3001","test":"x"}}"#,
        )
        .unwrap();
        std::fs::write(d.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(d.join(".env"), "PORT=4000\nDB_PASSWORD=x\n").unwrap();
        let r = detect(&d).unwrap();
        assert_eq!(r.kind, Kind::Node);
        assert_eq!(r.name, "hr-portal");
        assert_eq!(r.suggestions[0].label, "pnpm start");
        assert_eq!(r.suggestions[1].label, "pnpm dev");
        assert!(!r.suggestions.iter().any(|s| s.label.contains("test")));
        assert_eq!(r.port, Some(3001));
        assert_eq!(r.dotenv["DB_PASSWORD"], "x");
    }

    #[test]
    fn python_with_fastapi() {
        let d = tmp("py");
        std::fs::write(d.join("requirements.txt"), "fastapi\nuvicorn\n").unwrap();
        std::fs::write(d.join("main.py"), "").unwrap();
        let r = detect(&d).unwrap();
        assert_eq!(r.kind, Kind::Python);
        assert!(r.suggestions[0].command.contains("-m uvicorn main:app"));
        assert_eq!(r.port, Some(8000));
    }

    #[test]
    fn dotnet_project() {
        let d = tmp("dotnet");
        std::fs::write(d.join("Api.csproj"), "<Project/>").unwrap();
        let r = detect(&d).unwrap();
        assert_eq!(r.kind, Kind::Dotnet);
        assert_eq!(r.name, "Api");
        assert!(r
            .suggestions
            .last()
            .unwrap()
            .command
            .contains("run -c Release --project Api.csproj"));
    }

    #[test]
    fn ports() {
        assert_eq!(port_in("vite --port 5173"), Some(5173));
        assert_eq!(port_in("next start -p 3000"), Some(3000));
        assert_eq!(port_in("node x.js"), None);
    }
}
