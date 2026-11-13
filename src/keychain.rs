//! The database key in the OS credential store. One backend per OS, same two calls.
//! macOS goes through `/usr/bin/security`, not a keychain crate: "Always Allow" is bound to the
//! calling binary, so a crate would re-prompt after every rebuild; `security` never changes.
use anyhow::Result;

const SERVICE: &str = "atclaude";
const ACCOUNT: &str = "db-key";

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use anyhow::{Context, bail};
    use std::process::{Command, Stdio};

    pub fn get() -> Result<Option<String>> {
        let out = Command::new("security")
            .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
            .stderr(Stdio::null())
            .output()
            .context("cannot run `security`")?;
        match (out.status.success(), out.status.code()) {
            (true, _) => Ok(
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                    .filter(|k| !k.is_empty()),
            ),
            (false, Some(44)) => Ok(None), // item not found
            _ => bail!("keychain lookup failed ({})", out.status),
        }
    }

    // NOTE: the key is briefly visible in `ps` while stored (same-user only).
    pub fn set(key: &str) -> Result<()> {
        let status = Command::new("security")
            .args([
                "add-generic-password",
                "-s",
                SERVICE,
                "-a",
                ACCOUNT,
                "-w",
                key,
                "-U",
            ])
            .status()?;
        if !status.success() {
            bail!("could not store the key in the keychain ({status})");
        }
        Ok(())
    }
}

/// Linux and other Unix: Secret Service through libsecret's `secret-tool` (package
/// `libsecret-tools`; needs a running keyring such as GNOME Keyring or KWallet).
#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;
    use anyhow::{Context, bail};
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub fn get() -> Result<Option<String>> {
        let out = Command::new("secret-tool")
            .args(["lookup", "service", SERVICE, "account", ACCOUNT])
            .stderr(Stdio::piped())
            .output()
            .context(
                "cannot run `secret-tool` (install libsecret-tools), or set `headless = true`",
            )?;
        let err = String::from_utf8_lossy(&out.stderr);
        match (out.status.success(), out.status.code()) {
            (true, _) => Ok(
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                    .filter(|k| !k.is_empty()),
            ),
            // not found: exit 1 and nothing on stderr; anything printed means the keyring itself failed
            (false, Some(1)) if err.trim().is_empty() => Ok(None),
            _ => bail!(
                "keyring lookup failed: {}; on a machine without a running keyring set `headless = true`",
                err.trim()
            ),
        }
    }

    pub fn set(key: &str) -> Result<()> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=atclaude",
                "service",
                SERVICE,
                "account",
                ACCOUNT,
            ])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(
                "cannot run `secret-tool` (install libsecret-tools), or set `headless = true`",
            )?;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(key.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "could not store the key in the keyring: {}; on a machine without a running keyring set `headless = true`",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

/// Windows Credential Manager (per user, not per binary).
#[cfg(windows)]
mod imp {
    use super::*;

    pub fn get() -> Result<Option<String>> {
        match keyring::Entry::new(SERVICE, ACCOUNT)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(key: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, ACCOUNT)?.set_password(key)?;
        Ok(())
    }
}

pub fn get() -> Result<Option<String>> {
    imp::get()
}

pub fn set(key: &str) -> Result<()> {
    imp::set(key)
}
