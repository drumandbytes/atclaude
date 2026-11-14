//! The history: plain SQLite, with each turn's content compressed and then encrypted.
//! Metadata (time, model, tokens, a hash of the directory) stays readable without the key, so
//! stats and prompt integrations never need it; content never is.
use crate::key::{self, Key};
use anyhow::{Context, Result, anyhow, bail};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Meta<'a> {
    pub dir: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub mode: &'a str,
    pub tier: &'a str,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
    pub cache_key: &'a str,
}

/// The encrypted part of a turn.
#[derive(Default)]
pub struct Content {
    pub instruction: String,
    pub payload: String,
    pub response: String,
    pub cwd: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

pub struct Turn {
    pub id: i64,
    pub when: String,
    pub provider: String,
    pub model: String,
    pub mode: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub content: Content,
}

/// Seconds relative to now for `since`/`older`; `dir` is a directory hash.
#[derive(Default)]
pub struct Filter {
    pub since: Option<i64>,
    pub older: Option<i64>,
    pub dir: Option<String>,
}

pub struct Store {
    conn: Connection,
    data_key: Key,
}

const SCHEMA_VERSION: i64 = 1;

/// One database per user, in the global data directory. Debug builds read ATCLAUDE_DB so
/// development and tests never touch the real history.
pub fn path() -> Result<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("ATCLAUDE_DB") {
        return Ok(PathBuf::from(p));
    }
    let dirs = directories::ProjectDirs::from("", "", "atclaude")
        .ok_or_else(|| anyhow!("cannot determine data directory"))?;
    std::fs::create_dir_all(dirs.data_dir())?;
    Ok(dirs.data_dir().join("history.db"))
}

/// Not secret-keyed on purpose: a prompt integration must be able to compute it without the key.
pub fn dir_hash(dir: &Path) -> String {
    blake3::hash(dir.to_string_lossy().as_bytes()).to_hex()[..16].to_string()
}

fn seal(key: &Key, plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("no system randomness: {e}"))?;
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    let ct = cipher
        .encrypt(&XNonce::from(nonce), plain)
        .map_err(|_| anyhow!("encryption failed"))?;
    Ok([nonce.as_slice(), &ct].concat())
}

fn unseal(key: &Key, blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 24 {
        bail!("corrupt record");
    }
    let (nonce, ct) = blob.split_at(24);
    let nonce: [u8; 24] = nonce.try_into().expect("24 bytes");
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    cipher
        .decrypt(&XNonce::from(nonce), ct)
        .map_err(|_| anyhow!("cannot decrypt (wrong key?)"))
}

fn pack(key: &Key, c: &Content) -> Result<Vec<u8>> {
    let v = json!({"i": c.instruction, "p": c.payload, "r": c.response, "cwd": c.cwd, "repo": c.repo, "branch": c.branch});
    seal(key, &zstd::encode_all(v.to_string().as_bytes(), 3)?)
}

fn unpack(key: &Key, blob: &[u8]) -> Result<Content> {
    let v: Value = serde_json::from_slice(&zstd::decode_all(unseal(key, blob)?.as_slice())?)?;
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let o = |k: &str| v[k].as_str().map(str::to_owned);
    Ok(Content {
        instruction: s("i"),
        payload: s("p"),
        response: s("r"),
        cwd: s("cwd"),
        repo: o("repo"),
        branch: o("branch"),
    })
}

impl Store {
    /// Opens the history, creating the database, the data key and (only for a new database) the
    /// master key as needed. A missing master key never silently orphans existing history.
    pub fn open(keychain: bool) -> Result<Self> {
        Self::open_at(&path()?, keychain)
    }

    pub fn open_at(path: &Path, keychain: bool) -> Result<Self> {
        let conn =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            bail!("the history was written by a newer version of atclaude (schema {version})");
        }
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                dir TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                mode TEXT NOT NULL,
                tier TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                cache_key TEXT NOT NULL,
                content BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS turns_dir ON turns(dir, ts);
             CREATE INDEX IF NOT EXISTS turns_cache ON turns(cache_key, ts);
             CREATE TABLE IF NOT EXISTS stats (
                day TEXT NOT NULL,
                dir TEXT NOT NULL,
                calls INTEGER NOT NULL DEFAULT 0,
                cache_hits INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (day, dir)
             );
             PRAGMA user_version = {SCHEMA_VERSION};"
        ))?;

        let wrapped: Option<Vec<u8>> = conn
            .query_row("SELECT value FROM meta WHERE key = 'data_key'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let data_key = match (wrapped, key::load(keychain)?) {
            (Some(w), Some(master)) => unseal(&master, &w)
                .context("the key does not match this history")?
                .try_into()
                .map_err(|_| anyhow!("corrupt data key"))?,
            (Some(_), None) => bail!(
                "the history exists but its key is missing; restore it with `atclaude key import`"
            ),
            (None, master) => {
                let master = match master {
                    Some(m) => m,
                    None => {
                        let m = key::random()?;
                        key::save(&m, keychain)?;
                        m
                    }
                };
                let dk = key::random()?;
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('data_key', ?1)",
                    params![seal(&master, &dk)?],
                )?;
                dk
            }
        };
        Ok(Store { conn, data_key })
    }

    /// Re-wrap the data key under a new master key. The history itself is not rewritten.
    // NOTE: the database is updated before the key is saved; if saving fails the old wrapping is restored.
    pub fn rewrap(&self, new_master: &Key, keychain: bool) -> Result<()> {
        let old: Vec<u8> =
            self.conn
                .query_row("SELECT value FROM meta WHERE key = 'data_key'", [], |r| {
                    r.get(0)
                })?;
        self.conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'data_key'",
            params![seal(new_master, &self.data_key)?],
        )?;
        if let Err(e) = key::save(new_master, keychain) {
            self.conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'data_key'",
                params![old],
            )?;
            return Err(e);
        }
        Ok(())
    }

    /// Whether `master` unlocks the history at `path` (used before importing a key).
    pub fn key_fits(path: &Path, master: &Key) -> Result<bool> {
        if !path.exists() {
            return Ok(true);
        }
        let conn = Connection::open(path)?;
        let wrapped: Option<Vec<u8>> = conn
            .query_row("SELECT value FROM meta WHERE key = 'data_key'", [], |r| {
                r.get(0)
            })
            .optional()
            .unwrap_or(None);
        Ok(wrapped.is_none_or(|w| unseal(master, &w).is_ok()))
    }

    /// A keyed hash, so the stored value reveals nothing about the prompt it stands for.
    pub fn cache_key(&self, parts: &[&str]) -> String {
        blake3::keyed_hash(&self.data_key, json!(parts).to_string().as_bytes())
            .to_hex()
            .to_string()
    }

    pub fn record(&self, m: &Meta, c: &Content) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turns (ts, dir, provider, model, mode, tier, input_tokens, output_tokens, duration_ms, cache_key, content)
             VALUES (unixepoch(), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![m.dir, m.provider, m.model, m.mode, m.tier, m.input_tokens as i64, m.output_tokens as i64, m.duration_ms as i64, m.cache_key, pack(&self.data_key, c)?],
        )?;
        self.bump(m.dir, 0, m.input_tokens, m.output_tokens)
    }

    /// The newest response recorded for this exact call, if it is younger than `ttl` seconds.
    pub fn cached(&self, cache_key: &str, ttl: i64, dir: &str) -> Result<Option<String>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT content FROM turns WHERE cache_key = ?1 AND ts >= unixepoch() - ?2 ORDER BY ts DESC, id DESC LIMIT 1",
                params![cache_key, ttl],
                |r| r.get(0),
            )
            .optional()?;
        let Some(blob) = blob else { return Ok(None) };
        self.bump(dir, 1, 0, 0)?;
        Ok(Some(unpack(&self.data_key, &blob)?.response))
    }

    fn bump(&self, dir: &str, hit: u64, input: u64, output: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO stats (day, dir, calls, cache_hits, input_tokens, output_tokens)
             VALUES (date('now','localtime'), ?1, 1, ?2, ?3, ?4)
             ON CONFLICT (day, dir) DO UPDATE SET calls = calls + 1, cache_hits = cache_hits + ?2,
                input_tokens = input_tokens + ?3, output_tokens = output_tokens + ?4",
            params![dir, hit as i64, input as i64, output as i64],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn list(&self, f: &Filter, limit: usize) -> Result<Vec<Turn>> {
        self.select(
            "WHERE (?1 IS NULL OR ts >= unixepoch() - ?1) AND (?2 IS NULL OR ts < unixepoch() - ?2) AND (?3 IS NULL OR dir = ?3)
             ORDER BY ts DESC, id DESC LIMIT ?4",
            params![f.since, f.older, f.dir, limit as i64],
        )
    }

    pub fn get(&self, id: i64) -> Result<Option<Turn>> {
        Ok(self
            .select("WHERE id = ?1", params![id])?
            .into_iter()
            .next())
    }

    fn select(&self, tail: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id, datetime(ts,'unixepoch','localtime'), provider, model, mode, input_tokens, output_tokens, content FROM turns {tail}"
        ))?;
        let rows = stmt.query_map(p, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Vec<u8>>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, when, provider, model, mode, i, o, blob) = row?;
            out.push(Turn {
                id,
                when,
                provider,
                model,
                mode,
                input_tokens: i as u64,
                output_tokens: o as u64,
                content: unpack(&self.data_key, &blob)?,
            });
        }
        Ok(out)
    }

    pub fn delete(&self, ids: &[i64]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0;
        for id in ids {
            n += tx.execute("DELETE FROM turns WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(n)
    }
}
