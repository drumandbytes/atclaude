use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, Row, params};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Turn {
    pub id: i64,
    pub ts: String,
    pub session: Option<String>,
    /// "turn", or "summary" for a compacted digest of older turns
    pub kind: String,
    pub cwd: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// interactive | pipe | agent | yolo | compact
    pub mode: String,
    pub model: String,
    pub prompt: String,
    pub payload: String,
    pub response: String,
}

pub struct NewTurn<'a> {
    pub session: Option<&'a str>,
    pub kind: &'a str,
    pub cwd: &'a str,
    pub repo: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub mode: &'a str,
    pub tty: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub payload: &'a str,
    pub response: &'a str,
    pub duration_ms: i64,
    /// Override the timestamp (compaction keeps the time of the newest turn it replaces).
    pub at: Option<&'a str>,
}

/// SQLite datetime modifiers ("-14 days") for `since`/`older`.
#[derive(Default)]
pub struct Filter {
    pub since: Option<String>,
    pub older: Option<String>,
    pub session: Option<String>,
    pub repo: Option<String>,
}

pub struct Database {
    conn: Connection,
}

const COLS: &str =
    "id, created_at, session, kind, cwd, repo, branch, mode, model, prompt, payload, response";

fn turn(r: &Row) -> rusqlite::Result<Turn> {
    Ok(Turn {
        id: r.get(0)?,
        ts: r.get(1)?,
        session: r.get(2)?,
        kind: r.get(3)?,
        cwd: r.get(4)?,
        repo: r.get(5)?,
        branch: r.get(6)?,
        mode: r.get(7)?,
        model: r.get(8)?,
        prompt: r.get(9)?,
        payload: r.get(10)?,
        response: r.get(11)?,
    })
}

impl Database {
    pub fn open_default(headless: bool) -> Result<Self> {
        let path = db_path()?;
        let key = key(!path.exists(), headless)?;
        Self::open(&path, &key)
    }

    // NOTE: schema v1 with no migrations (no database existed before it); add
    // `PRAGMA user_version` migrations with the first release.
    pub fn open(path: &Path, key: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // 64 hex chars = raw SQLCipher key (skips the KDF); anything else is a passphrase.
        let pragma = if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            format!("PRAGMA key = \"x'{key}'\";")
        } else {
            format!("PRAGMA key = '{}';", key.replace('\'', "''"))
        };
        conn.execute_batch(&pragma)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .context("cannot decrypt history db (wrong key?)")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                session TEXT,
                kind TEXT NOT NULL DEFAULT 'turn',
                cwd TEXT NOT NULL,
                repo TEXT,
                branch TEXT,
                mode TEXT NOT NULL,
                tty TEXT,
                model TEXT NOT NULL,
                prompt TEXT NOT NULL,
                payload TEXT NOT NULL DEFAULT '',
                response TEXT NOT NULL,
                duration_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS turns_session ON turns(session, created_at);
            CREATE INDEX IF NOT EXISTS turns_created ON turns(created_at);
            CREATE INDEX IF NOT EXISTS turns_repo ON turns(repo);",
        )?;
        Ok(Database { conn })
    }

    pub fn record(&self, t: &NewTurn) -> Result<()> {
        insert(&self.conn, t)
    }

    /// Last `n` turns of a session (summaries included), oldest first.
    pub fn recent(&self, session: &str, n: usize) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM turns WHERE session = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let mut v = stmt
            .query_map(params![session, n as i64], turn)?
            .collect::<Result<Vec<_>, _>>()?;
        v.reverse();
        Ok(v)
    }

    /// Newest first.
    pub fn query(&self, f: &Filter, limit: usize) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM turns
             WHERE (?1 IS NULL OR created_at >= datetime('now','localtime',?1))
               AND (?2 IS NULL OR created_at <  datetime('now','localtime',?2))
               AND (?3 IS NULL OR session = ?3)
               AND (?4 IS NULL OR repo = ?4)
             ORDER BY created_at DESC, id DESC LIMIT ?5"
        ))?;
        let rows = stmt.query_map(
            params![f.since, f.older, f.session, f.repo, limit as i64],
            turn,
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Turn>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM turns WHERE id = ?1"))?;
        let mut rows = stmt.query_map(params![id], turn)?;
        let t = rows.next().transpose()?;
        Ok(t)
    }

    pub fn sessions(&self) -> Result<Vec<(String, i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT session, count(*), max(created_at) FROM turns
             WHERE session IS NOT NULL GROUP BY session ORDER BY max(created_at) DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn now(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))?)
    }

    pub fn delete_ids(&self, ids: &[i64]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let n = delete(&tx, ids)?;
        tx.commit()?;
        Ok(n)
    }

    /// Insert a summary and delete the turns it replaces, atomically.
    pub fn replace(&self, summary: &NewTurn, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        insert(&tx, summary)?;
        delete(&tx, ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Retention: drop turns older than `days` and everything beyond the newest `max_turns`.
    /// 0 disables a limit. Summaries are exempt: they are what's left after compaction.
    pub fn prune(&self, days: usize, max_turns: usize) -> Result<usize> {
        let mut n = 0;
        if days > 0 {
            n += self.conn.execute(
                "DELETE FROM turns WHERE kind = 'turn' AND created_at < datetime('now','localtime', ?1)",
                params![format!("-{days} days")],
            )?;
        }
        if max_turns > 0 {
            n += self.conn.execute(
                "DELETE FROM turns WHERE kind = 'turn' AND id <= (
                   SELECT id FROM turns WHERE kind = 'turn' ORDER BY id DESC LIMIT 1 OFFSET ?1)",
                params![max_turns as i64],
            )?;
        }
        Ok(n)
    }
}

fn insert(conn: &Connection, t: &NewTurn) -> Result<()> {
    conn.execute(
        "INSERT INTO turns (created_at, session, kind, cwd, repo, branch, mode, tty, model, prompt, payload, response, duration_ms)
         VALUES (COALESCE(?1, datetime('now','localtime')), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![t.at, t.session, t.kind, t.cwd, t.repo, t.branch, t.mode, t.tty, t.model, t.prompt, t.payload, t.response, t.duration_ms],
    )?;
    Ok(())
}

fn delete(conn: &Connection, ids: &[i64]) -> Result<usize> {
    let mut n = 0;
    for id in ids {
        n += conn.execute("DELETE FROM turns WHERE id = ?1", params![id])?;
    }
    Ok(n)
}

/// The global history database: one per user, shared by every directory.
pub fn global_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "atclaude")
        .ok_or_else(|| anyhow!("cannot determine data directory"))?;
    fs::create_dir_all(dirs.data_dir())?;
    Ok(dirs.data_dir().to_path_buf())
}

/// One database per user, in the global data directory. Not configurable; debug builds read
/// ATCLAUDE_DB / ATCLAUDE_DB_KEY so development and tests never touch the real history.
fn db_path() -> Result<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ATCLAUDE_DB") {
        return Ok(PathBuf::from(p));
    }
    Ok(global_dir()?.join("history.db"))
}

/// The key lives in the OS keychain, or with `headless = true` in a 0600 key file in the config
/// directory (away from the database, so backing up the data directory doesn't carry the key).
/// A new key is only generated for a new database, so a lost key can't silently orphan history.
fn key(fresh_db: bool, headless: bool) -> Result<String> {
    #[cfg(debug_assertions)]
    if let Ok(k) = std::env::var("ATCLAUDE_DB_KEY") {
        return Ok(k);
    }
    let path = if headless { Some(key_path()?) } else { None };
    let existing = match &path {
        Some(p) => file_get(p)?,
        None => crate::keychain::get()?,
    };
    if let Some(k) = existing {
        return Ok(k);
    }
    if !fresh_db {
        bail!(
            "history db exists but its key is missing from the {}",
            if headless { "key file" } else { "keychain" }
        );
    }
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("no system randomness: {e}"))?;
    let k: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    match &path {
        Some(p) => file_set(p, &k)?,
        None => crate::keychain::set(&k)?,
    }
    Ok(k)
}

fn key_path() -> Result<PathBuf> {
    Ok(crate::config::global_path()
        .ok_or_else(|| anyhow!("cannot determine config directory"))?
        .with_file_name("db.key"))
}

/// Refuses a key file other users can read, like ssh does for private keys.
fn file_get(path: &Path) -> Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    if meta.permissions().mode() & 0o077 != 0 {
        bail!(
            "{} is readable by others; run: chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    Ok(Some(fs::read_to_string(path)?.trim().to_string()).filter(|k| !k.is_empty()))
}

fn file_set(path: &Path, key: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(0o600); // elsewhere the file inherits the user profile's private ACL
    opts.open(path)?.write_all(key.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new<'a>(session: Option<&'a str>, prompt: &'a str) -> NewTurn<'a> {
        NewTurn {
            session,
            kind: "turn",
            cwd: "/",
            repo: Some("r"),
            branch: None,
            mode: "pipe",
            tty: None,
            model: "m",
            prompt,
            payload: "",
            response: "r",
            duration_ms: 1,
            at: None,
        }
    }

    #[test]
    fn roundtrip_encrypted() {
        let path = std::env::temp_dir().join(format!("atclaude-test-{}.db", std::process::id()));
        let _ = fs::remove_file(&path);
        let key = "ab".repeat(32);
        let db = Database::open(&path, &key).unwrap();
        db.record(&new(Some("s"), "p")).unwrap();
        assert_eq!(db.recent("s", 5).unwrap().len(), 1);
        drop(db);
        assert!(
            Database::open(&path, &"cd".repeat(32)).is_err(),
            "wrong key must fail"
        );
        assert!(
            !String::from_utf8_lossy(&fs::read(&path).unwrap()).contains("CREATE TABLE"),
            "file must not be plaintext"
        );
        fs::remove_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn key_file_perms() {
        let dir = std::env::temp_dir().join(format!("atclaude-key-{}", std::process::id()));
        let p = dir.join("db.key");
        assert_eq!(file_get(&p).unwrap(), None);
        file_set(&p, "k1").unwrap();
        assert_eq!(file_get(&p).unwrap().as_deref(), Some("k1"));
        assert!(
            file_set(&p, "k2").is_err(),
            "never overwrite an existing key"
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            file_get(&p).is_err(),
            "group/other-readable key must be refused"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filter_prune_replace() {
        let path = std::env::temp_dir().join(format!("atclaude-test2-{}.db", std::process::id()));
        let _ = fs::remove_file(&path);
        let db = Database::open(&path, &"ab".repeat(32)).unwrap();
        for i in 0..5 {
            db.record(&new(Some(if i < 3 { "a" } else { "b" }), &format!("p{i}")))
                .unwrap();
        }
        let f = Filter {
            session: Some("a".into()),
            ..Default::default()
        };
        assert_eq!(db.query(&f, 100).unwrap().len(), 3);
        assert_eq!(
            db.query(
                &Filter {
                    repo: Some("nope".into()),
                    ..Default::default()
                },
                100
            )
            .unwrap()
            .len(),
            0
        );
        // retention by count keeps the newest 2
        assert_eq!(db.prune(0, 2).unwrap(), 3);
        let left = db.query(&Filter::default(), 100).unwrap();
        assert_eq!(
            left.iter().map(|t| t.prompt.as_str()).collect::<Vec<_>>(),
            ["p4", "p3"]
        );
        // replace swaps turns for a summary that retention leaves alone
        let ids: Vec<i64> = left.iter().map(|t| t.id).collect();
        let mut sum = new(Some("b"), "(summary)");
        sum.kind = "summary";
        db.replace(&sum, &ids).unwrap();
        assert_eq!(db.prune(0, 1).unwrap(), 0);
        let all = db.query(&Filter::default(), 100).unwrap();
        assert_eq!((all.len(), all[0].kind.as_str()), (1, "summary"));
        drop(db);
        fs::remove_file(&path).unwrap();
    }
}
