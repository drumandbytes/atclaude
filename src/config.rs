use std::path::{Path, PathBuf};
use std::fs;
use toml::{Table, Value};

#[derive(Clone, Copy)]
pub enum Ty {
    Bool,
    Str,
    Num,
    List,
}

/// Every setting. Each one also reads the env var `ATCLAUDE_<KEY>` (lists: comma-separated).
/// Precedence: flag > env > nearest `.atclauderc` > parent `.atclauderc` files > global config > default.
pub const KEYS: &[(&str, Ty)] = &[
    ("headless", Ty::Bool),
    ("claude_path", Ty::Str),
    ("record", Ty::Bool),
    ("session", Ty::Str),
    ("model", Ty::Str),
    ("helper_model", Ty::Str),
    ("context_turns", Ty::Num),
    ("context_budget", Ty::Num),
    ("search_budget", Ty::Num),
    ("readonly_bash", Ty::List),
    ("prompt_pipe", Ty::Str),
    ("prompt_interactive", Ty::Str),
    ("prompt_agent", Ty::Str),
    ("readonly_commands", Ty::List),
    ("search_prompt_chars", Ty::Num),
    ("search_response_chars", Ty::Num),
    ("fast_model", Ty::Str),
    ("deep_model", Ty::Str),
    ("retention_days", Ty::Num),
    ("retention_turns", Ty::Num),
    ("payload_max_chars", Ty::Num),
];

pub const DEFAULT_READONLY: &[&str] = &["git status", "git diff", "git log", "git show", "ls", "pwd"];

pub struct Config {
    table: Table,
    pub files: Vec<PathBuf>,
}

pub fn global_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ATCLAUDE_CONFIG") {
        return Some(p.into());
    }
    #[cfg(windows)]
    return Some(directories::ProjectDirs::from("", "", "atclaude")?.config_dir().join("config.toml"));
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".config")))?;
        Some(base.join("atclaude/config.toml"))
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(format!("ATCLAUDE_{}", key.to_uppercase())).ok().filter(|v| !v.is_empty())
}

fn read(path: &Path) -> Option<Table> {
    let text = fs::read_to_string(path).ok()?;
    match toml::from_str::<Table>(&text) {
        Ok(t) => Some(t),
        Err(e) => {
            // legacy `.atclauderc`: a bare session name
            let name = text.trim();
            if !name.is_empty() && !name.contains(['=', '\n']) {
                let mut t = Table::new();
                t.insert("session".into(), Value::String(name.into()));
                return Some(t);
            }
            eprintln!("[atclaude] ignoring {}: {e}", path.display());
            None
        }
    }
}

fn fits(ty: Ty, v: &Value) -> bool {
    match ty {
        Ty::Bool => v.is_bool(),
        Ty::Str => v.is_str(),
        Ty::Num => v.as_integer().is_some_and(|n| n >= 0),
        Ty::List => v.as_array().is_some_and(|a| a.iter().all(Value::is_str)),
    }
}

impl Config {
    pub fn load(cwd: &Path) -> Config {
        Self::load_from(global_path(), cwd)
    }

    pub fn load_from(global: Option<PathBuf>, cwd: &Path) -> Config {
        let mut paths: Vec<PathBuf> = global.into_iter().collect();
        let mut dirs: Vec<PathBuf> = cwd.ancestors().map(|d| d.join(".atclauderc")).collect();
        dirs.reverse(); // farthest first, so the nearest file wins
        paths.extend(dirs);

        let mut cfg = Config { table: Table::new(), files: Vec::new() };
        for p in paths {
            if let Some(t) = read(&p) {
                for (k, v) in t {
                    // a typo like `recrod = false` must not silently keep recording, and a bad
                    // value must not shadow a good one from a lower layer
                    match KEYS.iter().find(|(n, _)| *n == k) {
                        None => eprintln!("[atclaude] {}: unknown key `{k}` (see `atclaude config`)", p.display()),
                        Some((_, ty)) if !fits(*ty, &v) => eprintln!("[atclaude] {}: key `{k}` has the wrong type; ignored", p.display()),
                        Some(_) => {
                            cfg.table.insert(k, v);
                        }
                    }
                }
                cfg.files.push(p);
            }
        }
        cfg
    }

    fn get(&self, key: &str, ty: Ty) -> Option<&Value> {
        self.table.get(key).filter(|v| fits(ty, v))
    }

    pub fn str(&self, key: &str) -> Option<String> {
        env(key).or_else(|| self.get(key, Ty::Str).and_then(|v| v.as_str()).map(str::to_owned))
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match env(key) {
            Some(e) => match e.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            },
            None => self.get(key, Ty::Bool).and_then(|v| v.as_bool()),
        }
    }

    pub fn num(&self, key: &str) -> Option<usize> {
        env(key).and_then(|e| e.parse().ok()).or_else(|| self.get(key, Ty::Num).and_then(|v| v.as_integer()).map(|n| n as usize))
    }

    pub fn list(&self, key: &str) -> Vec<String> {
        match env(key) {
            Some(e) => e.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect(),
            None => self
                .get(key, Ty::List)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
        }
    }

    pub fn readonly_commands(&self) -> Vec<String> {
        let l = self.list("readonly_commands");
        if l.is_empty() { DEFAULT_READONLY.iter().map(|s| s.to_string()).collect() } else { l }
    }

    /// Effective value and where it came from, for `atclaude config`.
    pub fn describe(&self, key: &str) -> (String, &'static str) {
        if let Some(e) = env(key) {
            return (e, "env");
        }
        match self.table.get(key) {
            Some(v) => (v.to_string(), "config"),
            None => ("-".into(), "default"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layering_and_legacy() {
        let root = std::env::temp_dir().join(format!("atclaude-cfg-{}", std::process::id()));
        let sub = root.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        let global = root.join("global.toml");
        fs::write(&global, "record = false\nsession = \"g\"\ncontext_turns = 3\n").unwrap();
        fs::write(root.join(".atclauderc"), "proj\n").unwrap(); // legacy bare name
        fs::write(sub.join(".atclauderc"), "record = true\n").unwrap();

        let c = Config::load_from(Some(global.clone()), &sub);
        assert_eq!(c.bool("record"), Some(true), "nearest file wins");
        assert_eq!(c.str("session").as_deref(), Some("proj"), "parent file beats global");
        assert_eq!(c.num("context_turns"), Some(3), "global applies when nothing nearer sets it");
        let c = Config::load_from(None, &root.join("nowhere"));
        assert_eq!(c.str("model"), None);
        assert!(Config::load_from(Some(global), &root.join("nowhere")).bool("record") == Some(false));
        fs::remove_dir_all(&root).unwrap();
    }
}
