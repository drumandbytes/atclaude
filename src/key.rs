//! The master key: random, 256-bit, kept in a file next to the config (or the OS credential
//! store with `keychain = true`). It wraps the data key that encrypts history content, so it can
//! be rotated without rewriting the history, and it can be shown as words for backup.
use crate::config;
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::{fs, io::Write};

pub type Key = [u8; 32];

pub fn random() -> Result<Key> {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).map_err(|e| anyhow!("no system randomness: {e}"))?;
    Ok(k)
}

pub fn path() -> Result<PathBuf> {
    Ok(config::global_path()
        .ok_or_else(|| anyhow!("cannot determine config directory"))?
        .with_file_name("key"))
}

pub fn load(keychain: bool) -> Result<Option<Key>> {
    let hex = if keychain {
        crate::keychain::get()?
    } else {
        read_file(&path()?)?
    };
    hex.map(|h| from_hex(h.trim())).transpose()
}

pub fn save(key: &Key, keychain: bool) -> Result<()> {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    if keychain {
        crate::keychain::set(&hex)
    } else {
        write_file(&path()?, &hex)
    }
}

fn from_hex(s: &str) -> Result<Key> {
    let bad = || anyhow!("the stored key is not 64 hex characters");
    if s.len() != 64 || !s.is_ascii() {
        return Err(bad());
    }
    let mut k = [0u8; 32];
    for (i, b) in k.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
    }
    Ok(k)
}

/// Refuses a key file other users can read, like ssh does for private keys.
fn read_file(path: &Path) -> Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o077 != 0 {
        bail!(
            "{} is readable by others; run: chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    let _ = meta;
    Ok(Some(fs::read_to_string(path)?).filter(|k| !k.trim().is_empty()))
}

/// Written to a temporary file and renamed, so an existing key is replaced atomically.
fn write_file(path: &Path, hex: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600); // elsewhere the file inherits the user profile's private ACL
    opts.open(&tmp)?.write_all(hex.as_bytes())?;
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))
}

pub fn to_words(key: &Key) -> Result<String> {
    Ok(bip39::Mnemonic::from_entropy(key)
        .map_err(|e| anyhow!("{e}"))?
        .to_string())
}

pub fn from_words(words: &str) -> Result<Key> {
    let normalized = words
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let entropy = bip39::Mnemonic::parse(&normalized)
        .map_err(|e| anyhow!("not a valid key phrase: {e}"))?
        .to_entropy();
    entropy
        .try_into()
        .map_err(|_| anyhow!("the key phrase must be 24 words"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_roundtrip() {
        let k = random().unwrap();
        let words = to_words(&k).unwrap();
        assert_eq!(words.split(' ').count(), 24);
        assert_eq!(
            from_words(&format!("  {}  ", words.to_uppercase())).unwrap(),
            k
        );
        assert!(from_words("not a real phrase").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_perms() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir()
            .join(format!("atclaude-key-{}", std::process::id()))
            .join("key");
        assert_eq!(read_file(&p).unwrap(), None);
        write_file(&p, "aa").unwrap();
        write_file(&p, "bb").unwrap();
        assert_eq!(read_file(&p).unwrap().as_deref(), Some("bb"));
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            read_file(&p).is_err(),
            "group/other-readable key must be refused"
        );
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
