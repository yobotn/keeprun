//! Spots a database that needs migrating, and finds the task that migrates it.
//!
//! Two signals. The strong one is an error in the app output that says the schema is behind:
//! "You have 3 unapplied migration(s)", "relation \"users\" does not exist" and similar. The
//! soft one is migration files that appeared since the last successful migrate run, e.g. after
//! a pull. Files alone can be wrong (a migration applied from a terminal), so the user can
//! dismiss them.
//!
//! Migrations never run on their own: a crash loop would run them again on every restart.
//! They run when the user or a coding agent asks, or before every start when the app is set
//! up that way.

use crate::issue::strip_ansi;
use crate::model::TaskSpec;
use std::path::Path;

/// Words in a task name or command that change the schema in other ways than applying
/// pending migrations: creating, resetting or rolling back. Such a task is never picked.
const RISKY: &[&str] = &[
    "dev",
    "reset",
    "rollback",
    "down",
    "undo",
    "revert",
    "make",
    "makemigrations",
    "create",
    "generate",
    "gen",
    "new",
    "status",
    "fresh",
    "refresh",
    "drop",
    "redo",
    "diff",
    "squash",
    "seed",
];

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
}

/// The command without paths, e.g. "D:\dev\shop\.venv\Scripts\python.exe manage.py migrate"
/// gives "manage.py migrate": a folder named "dev" says nothing about the task.
fn arguments(command: &str) -> String {
    command
        .split_whitespace()
        .filter(|t| !t.contains('\\') && !t.contains('/'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a task applies pending migrations, judged by its name and command.
fn applies_migrations(t: &TaskSpec) -> bool {
    let args = arguments(&t.command);
    let name_says = words(&t.name).any(|w| w.starts_with("migrat"));
    let command_says = {
        let c = t.command.to_ascii_lowercase();
        c.contains("migrate deploy")
            || c.contains("manage.py migrate")
            || c.contains("alembic upgrade")
            || c.contains("ef database update")
            || c.contains("db:migrate")
            || c.contains("migrate:latest")
            || c.contains("migration:run")
            || c.contains("migrations:run")
    };
    let risky = words(&t.name)
        .chain(words(&args))
        .any(|w| RISKY.contains(&w.as_str()));
    (name_says || command_says) && !risky
}

/// The saved task that applies pending migrations: one named "migrate" first, then the first
/// task whose name or command says it migrates. Tasks that create, reset or roll back
/// migrations are never picked.
pub fn task(tasks: &[TaskSpec]) -> Option<&TaskSpec> {
    tasks
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case("migrate") && applies_migrations(t))
        .or_else(|| tasks.iter().find(|t| applies_migrations(t)))
}

/// Messages that mean the database schema is behind the code. Matched without case.
const PENDING: &[&str] = &[
    // Django
    "unapplied migration",
    // Rails
    "activerecord::pendingmigrationerror",
    "migrations are pending",
    // Ecto (Phoenix), Laravel, Knex and others
    "there are pending migrations",
    "pending migration",
    // Alembic
    "target database is not up to date",
    // Prisma, at run time
    "does not exist in the current database",
    // EF Core
    "pendingmodelchangeswarning",
    "has pending changes",
    // SQLite
    "no such table:",
    "no such column:",
    // SQL Server
    "invalid object name '",
    "invalid column name '",
    // MySQL, MariaDB, Laravel
    "base table or view not found",
    "unknown column '",
];

/// Whether a line of app output says the database needs migrating.
pub fn is_pending(line: &str) -> bool {
    let s = strip_ansi(line).to_ascii_lowercase();
    if PENDING.iter().any(|p| s.contains(p)) {
        return true;
    }
    // PostgreSQL: relation "users" does not exist, column "x" does not exist.
    // MySQL: Table 'shop.users' doesn't exist.
    (s.contains("does not exist") && (s.contains("relation \"") || s.contains("column \"")))
        || (s.contains("doesn't exist") && s.contains("table '"))
}

/// Folders, relative to the app folder, that hold migrations in common frameworks.
const FOLDERS: &[&str] = &[
    "prisma/migrations",
    "migrations",
    "db/migrate",
    "db/migrations",
    "database/migrations",
    "alembic/versions",
    "drizzle",
    "supabase/migrations",
    "src/migrations",
    "src/db/migrations",
    "src/database/migrations",
    "Data/Migrations",
];

/// Folders never searched for Django app migrations.
const SKIP: &[&str] = &[
    "node_modules",
    ".git",
    ".venv",
    "venv",
    "env",
    "dist",
    "build",
    "target",
    "bin",
    "obj",
    "__pycache__",
    ".next",
];

const MAX_FILES: usize = 5000;

/// Names that are part of a migrations folder but are not migrations.
fn is_migration(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !(n.starts_with('.')
        || n.starts_with("__")
        || n == "meta"
        || n == "migration_lock.toml"
        || n.ends_with(".pyc")
        || n.ends_with(".md")
        || n.ends_with("modelsnapshot.cs"))
}

fn list(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if is_migration(&name) && out.len() < MAX_FILES {
            out.push(format!("{rel}/{name}"));
        }
    }
}

/// Migrations in the app folder, as paths relative to it with forward slashes, sorted. Only
/// names are read, never contents.
pub fn files(cwd: &Path) -> Vec<String> {
    let mut out = vec![];
    for f in FOLDERS {
        list(&cwd.join(f), f, &mut out);
    }
    // Django keeps migrations per app: <app>/migrations.
    if let Ok(rd) = std::fs::read_dir(cwd) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let skip = SKIP.iter().any(|s| s.eq_ignore_ascii_case(&name))
                || FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(&name))
                || name.eq_ignore_ascii_case("src")
                || name.eq_ignore_ascii_case("db")
                || name.eq_ignore_ascii_case("database");
            if !skip && e.file_type().is_ok_and(|t| t.is_dir()) {
                list(
                    &e.path().join("migrations"),
                    &format!("{name}/migrations"),
                    &mut out,
                );
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Files in `now` that are not in `known`, i.e. added since the last migrate run.
pub fn added(known: &[String], now: &[String]) -> Vec<String> {
    let known: std::collections::HashSet<String> =
        known.iter().map(|k| k.to_ascii_lowercase()).collect();
    now.iter()
        .filter(|f| !known.contains(&f.to_ascii_lowercase()))
        .cloned()
        .collect()
}

/// Whether a changed path from `git diff --name-only` is a migration. Paths are relative to
/// the repository root, which may be above the app folder.
pub fn is_migration_path(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    if !is_migration(name) {
        return false;
    }
    let dir = format!("/{}", p.rsplit_once('/').map_or("", |(d, _)| d));
    FOLDERS
        .iter()
        .map(|f| f.to_ascii_lowercase())
        .chain(["migrations".to_string()])
        .any(|f| {
            // The migration is in the folder itself, or one level down (Prisma keeps one
            // folder per migration).
            let f = format!("/{f}");
            dir.ends_with(&f)
                || dir
                    .rsplit_once('/')
                    .is_some_and(|(parent, _)| parent.ends_with(&f))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, command: &str) -> TaskSpec {
        TaskSpec {
            name: name.into(),
            command: command.into(),
        }
    }

    #[test]
    fn picks_the_task_that_applies_migrations() {
        let tasks = [
            t("install", "npm install"),
            t("db:migrate:dev", "prisma migrate dev"),
            t("db:deploy", "prisma migrate deploy"),
        ];
        assert_eq!(task(&tasks).unwrap().name, "db:deploy");
        let tasks = [t("db:migrate", "knex migrate:latest"), t("migrate", "x")];
        assert_eq!(task(&tasks).unwrap().name, "migrate");
        assert_eq!(
            task(&[t("upgrade", "python -m alembic upgrade head")])
                .unwrap()
                .name,
            "upgrade"
        );
        assert_eq!(
            task(&[t("ef-update", "dotnet ef database update")])
                .unwrap()
                .name,
            "ef-update"
        );
        for risky in [
            t("migrate", "prisma migrate dev"),
            t("migrate:reset", "prisma migrate reset"),
            t("makemigrations", "python manage.py makemigrations"),
            t("migrate:rollback", "knex migrate:rollback"),
            t("migration:generate", "typeorm migration:generate"),
        ] {
            assert!(task(std::slice::from_ref(&risky)).is_none(), "{risky:?}");
        }
        assert!(task(&[t("build", "tsc")]).is_none());
        assert!(task(&[t("upgrade-deps", "npm update")]).is_none());
        // A folder named dev in a path does not make the task risky.
        assert_eq!(
            task(&[t(
                "migrate",
                r"D:\dev\shop\.venv\Scripts\python.exe manage.py migrate --noinput"
            )])
            .unwrap()
            .name,
            "migrate"
        );
    }

    #[test]
    fn spots_pending_migration_errors() {
        for l in [
            "You have 3 unapplied migration(s). Your project may not work properly until you apply the migrations for app(s): auth.",
            "ActiveRecord::PendingMigrationError: Migrations are pending.",
            "error: relation \"users\" does not exist",
            "PrismaClientKnownRequestError: The table `public.User` does not exist in the current database.",
            "The column `User.role` does not exist in the current database.",
            "sqlite3.OperationalError: no such table: orders",
            "SQLSTATE[42S02]: Base table or view not found: 1146 Table 'shop.users' doesn't exist",
            "Microsoft.Data.SqlClient.SqlException (0x80131904): Invalid object name 'Orders'.",
            "ERROR: column \"role\" does not exist at character 8",
            "\u{1b}[31mFAILED: Target database is not up to date.\u{1b}[0m",
        ] {
            assert!(is_pending(l), "{l}");
        }
        for l in [
            "Error: Cannot find module 'x'",
            "File does not exist",
            "GET / 200 in 5ms",
            "No migrations to apply.",
        ] {
            assert!(!is_pending(l), "{l}");
        }
    }

    #[test]
    fn lists_migration_files() {
        let d = std::env::temp_dir().join(format!("keeprun-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for dir in [
            "prisma/migrations/20260101_init",
            "shop/migrations/__pycache__",
            "node_modules/x/migrations",
        ] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        for f in [
            "prisma/migrations/migration_lock.toml",
            "shop/migrations/__init__.py",
            "shop/migrations/0001_initial.py",
            "node_modules/x/migrations/1.js",
        ] {
            std::fs::write(d.join(f), "").unwrap();
        }
        let found = files(&d);
        assert_eq!(
            found,
            [
                "prisma/migrations/20260101_init",
                "shop/migrations/0001_initial.py"
            ]
        );
        assert_eq!(
            added(&found[..1], &found),
            ["shop/migrations/0001_initial.py"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn spots_migrations_in_a_diff() {
        assert!(is_migration_path(
            "prisma/migrations/20260101_init/migration.sql"
        ));
        assert!(is_migration_path("apps/api/db/migrate/2026_add_users.rb"));
        assert!(is_migration_path("shop/migrations/0002_order.py"));
        assert!(is_migration_path("alembic/versions/ab12_add.py"));
        assert!(!is_migration_path("prisma/migrations/migration_lock.toml"));
        assert!(!is_migration_path("shop/migrations/__init__.py"));
        assert!(!is_migration_path("src/migrate.ts"));
        assert!(!is_migration_path("README.md"));
    }
}
