use std::fs;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

#[derive(Clone, Copy)]
pub enum Ty {
    Bool,
    Str,
    Num,
}

/// Every flat setting. Each one also reads the env var `ATCLAUDE_<KEY>`.
/// Precedence: flag > env > nearest `.atclauderc` > parent `.atclauderc` files > global config > default.
/// `[providers.<name>]` tables are the one nested setting.
pub const KEYS: &[(&str, Ty)] = &[
    ("provider", Ty::Str),
    ("model", Ty::Str),
    ("fast_model", Ty::Str),
    ("deep_model", Ty::Str),
    ("effort", Ty::Str),
    ("fast_effort", Ty::Str),
    ("deep_effort", Ty::Str),
    ("max_tokens", Ty::Num),
    ("record", Ty::Bool),
    ("cache", Ty::Str),
    ("cache_ttl", Ty::Str),
    ("keychain", Ty::Bool),
    ("prompt_pipe", Ty::Str),
    ("prompt_interactive", Ty::Str),
];

pub struct Config {
    table: Table,
    providers: Table,
    pub files: Vec<PathBuf>,
}

pub fn global_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ATCLAUDE_CONFIG") {
        return Some(p.into());
    }
    #[cfg(windows)]
    return Some(
        directories::ProjectDirs::from("", "", "atclaude")?
            .config_dir()
            .join("config.toml"),
    );
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".config")))?;
        Some(base.join("atclaude/config.toml"))
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(format!("ATCLAUDE_{}", key.to_uppercase()))
        .ok()
        .filter(|v| !v.is_empty())
}

fn fits(ty: Ty, v: &Value) -> bool {
    match ty {
        Ty::Bool => v.is_bool(),
        Ty::Str => v.is_str(),
        Ty::Num => v.as_integer().is_some_and(|n| n >= 0),
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

        let mut cfg = Config {
            table: Table::new(),
            providers: Table::new(),
            files: Vec::new(),
        };
        for p in paths {
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let t = match toml::from_str::<Table>(&text) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[atclaude] ignoring {}: {e}", p.display());
                    continue;
                }
            };
            for (k, v) in t {
                // a typo must not silently change behaviour, and a bad value must not shadow a
                // good one from a lower layer
                if k == "providers" {
                    match v {
                        Value::Table(ps) => cfg
                            .providers
                            .extend(ps.into_iter().filter(|(_, p)| p.is_table())),
                        _ => eprintln!(
                            "[atclaude] {}: `providers` must be a table; ignored",
                            p.display()
                        ),
                    }
                    continue;
                }
                match KEYS.iter().find(|(n, _)| *n == k) {
                    None => eprintln!(
                        "[atclaude] {}: unknown key `{k}` (see `atclaude config`)",
                        p.display()
                    ),
                    Some((_, ty)) if !fits(*ty, &v) => eprintln!(
                        "[atclaude] {}: key `{k}` has the wrong type; ignored",
                        p.display()
                    ),
                    Some(_) => {
                        cfg.table.insert(k, v);
                    }
                }
            }
            cfg.files.push(p);
        }
        cfg
    }

    pub fn str(&self, key: &str) -> Option<String> {
        env(key).or_else(|| {
            self.table
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match env(key) {
            Some(e) => match e.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            },
            None => self.table.get(key).and_then(|v| v.as_bool()),
        }
    }

    pub fn num(&self, key: &str) -> Option<u64> {
        env(key).and_then(|e| e.parse().ok()).or_else(|| {
            self.table
                .get(key)
                .and_then(|v| v.as_integer())
                .map(|n| n as u64)
        })
    }

    /// A field of `[providers.<name>]`.
    pub fn provider_str(&self, name: &str, field: &str) -> Option<String> {
        self.providers
            .get(name)?
            .get(field)?
            .as_str()
            .map(str::to_owned)
    }

    pub fn provider_names(&self) -> Vec<&String> {
        self.providers.keys().collect()
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
    fn layering() {
        let root = std::env::temp_dir().join(format!("atclaude-cfg-{}", std::process::id()));
        let sub = root.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        let global = root.join("global.toml");
        fs::write(&global, "record = false\nmodel = \"g\"\nmax_tokens = 100\n[providers.local]\ntype = \"openai\"\nbase_url = \"http://x/v1\"\n").unwrap();
        fs::write(
            root.join(".atclauderc"),
            "model = \"proj\"\nrecrod = true\n",
        )
        .unwrap();
        fs::write(sub.join(".atclauderc"), "record = true\nmodel = 5\n").unwrap();

        let c = Config::load_from(Some(global), &sub);
        assert_eq!(c.bool("record"), Some(true), "nearest file wins");
        assert_eq!(
            c.str("model").as_deref(),
            Some("proj"),
            "a wrongly typed value does not shadow the parent's"
        );
        assert_eq!(
            c.num("max_tokens"),
            Some(100),
            "global applies when nothing nearer sets it"
        );
        assert_eq!(
            c.provider_str("local", "base_url").as_deref(),
            Some("http://x/v1")
        );
        assert_eq!(c.provider_str("nope", "type"), None);
        fs::remove_dir_all(&root).unwrap();
    }
}
