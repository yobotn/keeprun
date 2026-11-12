//! SQLite store for apps, desired state and events.

use crate::dpapi;
use anyhow::{Context, Result};
use keeprun_core::model::{AppSpec, Event, EventKind};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

const EVENTS_KEPT_PER_APP: i64 = 1000;

pub struct Db {
    conn: Mutex<Connection>,
}

pub struct StoredApp {
    pub spec: AppSpec,
    pub desired_running: bool,
}

const MIGRATIONS: &[&str] = &[
    // 1
    "CREATE TABLE apps (
        id TEXT PRIMARY KEY,
        spec TEXT NOT NULL,
        desired_running INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
     );
     CREATE TABLE events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        app_id TEXT NOT NULL,
        ts INTEGER NOT NULL,
        kind TEXT NOT NULL,
        message TEXT NOT NULL
     );
     CREATE INDEX events_app ON events (app_id, id DESC);",
];

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)
            .with_context(|| format!("Cannot open database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            // Schema change and version bump commit together, so a crash cannot half-apply one.
            let tx = conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("Database migration {} failed", i + 1))?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        for sql in MIGRATIONS {
            conn.execute_batch(sql).unwrap();
        }
        Db {
            conn: Mutex::new(conn),
        }
    }

    fn encode(spec: &AppSpec) -> Result<String> {
        let mut s = spec.clone();
        for (k, v) in s.env.iter_mut() {
            if spec.secrets.contains(k) {
                *v = dpapi::encrypt(v)?;
            }
        }
        Ok(serde_json::to_string(&s)?)
    }

    fn decode(json: &str) -> Result<AppSpec> {
        let mut s: AppSpec = serde_json::from_str(json)?;
        let secrets = s.secrets.clone();
        for (k, v) in s.env.iter_mut() {
            if secrets.contains(k) {
                *v = dpapi::decrypt(v).with_context(|| format!("Env {k} of app {}", s.id))?;
            }
        }
        Ok(s)
    }

    pub fn load_apps(&self) -> Result<Vec<StoredApp>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT spec, desired_running FROM apps ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
        let mut out = vec![];
        for row in rows {
            let (json, desired_running) = row?;
            match Self::decode(&json) {
                Ok(spec) => out.push(StoredApp {
                    spec,
                    desired_running,
                }),
                Err(e) => tracing::error!("skipping app: {e:#}"),
            }
        }
        Ok(out)
    }

    pub fn insert_app(&self, spec: &AppSpec) -> Result<()> {
        let now = crate::supervisor::now_ms() as i64;
        self.conn.lock().unwrap().execute(
            "INSERT INTO apps (id, spec, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![spec.id, Self::encode(spec)?, now],
        )?;
        Ok(())
    }

    pub fn update_app(&self, spec: &AppSpec) -> Result<()> {
        let now = crate::supervisor::now_ms() as i64;
        self.conn.lock().unwrap().execute(
            "UPDATE apps SET spec = ?2, updated_at = ?3 WHERE id = ?1",
            params![spec.id, Self::encode(spec)?, now],
        )?;
        Ok(())
    }

    pub fn delete_app(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM apps WHERE id = ?1", [id])?;
        conn.execute("DELETE FROM events WHERE app_id = ?1", [id])?;
        Ok(())
    }

    pub fn set_desired_running(&self, id: &str, running: bool) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE apps SET desired_running = ?2 WHERE id = ?1",
            params![id, running],
        )?;
        Ok(())
    }

    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM apps", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    pub fn add_event(&self, app_id: &str, kind: EventKind, message: &str) -> Result<Event> {
        let ts = crate::supervisor::now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (app_id, ts, kind, message) VALUES (?1, ?2, ?3, ?4)",
            params![app_id, ts as i64, kind.as_str(), message],
        )?;
        let id = conn.last_insert_rowid();
        // Keep the newest events per app. Uses the (app_id, id) index, so it is cheap.
        conn.execute(
            "DELETE FROM events WHERE app_id = ?1 AND id <= (SELECT id FROM events WHERE app_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
            params![app_id, EVENTS_KEPT_PER_APP],
        )?;
        Ok(Event {
            id,
            app_id: app_id.into(),
            ts,
            kind,
            message: message.into(),
        })
    }

    pub fn events(&self, app_id: Option<&str>, limit: u32) -> Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, app_id, ts, kind, message FROM events WHERE (?1 IS NULL OR app_id = ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![app_id, limit], |r| {
            Ok(Event {
                id: r.get(0)?,
                app_id: r.get(1)?,
                ts: r.get::<_, i64>(2)? as u64,
                kind: EventKind::parse(&r.get::<_, String>(3)?).unwrap_or(EventKind::ConfigChanged),
                message: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    #[allow(dead_code)]
    pub fn app_exists(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT 1 FROM apps WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn apps_and_secrets_round_trip() {
        let db = Db::memory();
        let spec = AppSpec {
            id: "a".into(),
            name: "A".into(),
            cwd: "C:\\".into(),
            command: "x".into(),
            env: BTreeMap::from([
                ("PORT".into(), "1".into()),
                ("TOKEN".into(), "hunter2".into()),
            ]),
            secrets: BTreeSet::from(["TOKEN".into()]),
            tags: vec![],
            autostart: true,
            restart: Default::default(),
            max_restarts: 5,
            tasks: vec![],
            disabled: false,
            watch_errors: true,
            env_file: None,
            health_check: None,
            health_restart: false,
        };
        db.insert_app(&spec).unwrap();
        let raw: String = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT spec FROM apps", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains("hunter2"), "secret stored in clear text");
        db.set_desired_running("a", true).unwrap();
        let apps = db.load_apps().unwrap();
        assert_eq!(apps[0].spec, spec);
        assert!(apps[0].desired_running);
    }

    #[test]
    fn events_newest_first() {
        let db = Db::memory();
        db.add_event("a", EventKind::Started, "one").unwrap();
        db.add_event("a", EventKind::Stopped, "two").unwrap();
        db.add_event("b", EventKind::Started, "other").unwrap();
        let ev = db.events(Some("a"), 10).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].message, "two");
        assert_eq!(db.events(None, 10).unwrap().len(), 3);
    }
}
