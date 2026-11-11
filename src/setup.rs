use anyhow::Result;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, thread};

fn rules_path() -> Result<PathBuf> {
    Ok(crate::db::global_dir()?.join("readonly-rules.txt"))
}

fn settings_path() -> Option<PathBuf> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".claude")))?;
    Some(dir.join("settings.json"))
}

pub fn cached_rules() -> Option<String> {
    fs::read_to_string(rules_path().ok()?)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Hook commands that run before Bash calls (PreToolUse, matcher empty/`*`/containing `Bash`).
fn bash_hooks(settings: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for entry in settings["hooks"]["PreToolUse"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let m = entry["matcher"].as_str().unwrap_or("");
        if !(m.is_empty() || m == "*" || m.split('|').any(|p| p == "Bash")) {
            continue;
        }
        for h in entry["hooks"].as_array().into_iter().flatten() {
            if h["type"].as_str() == Some("command") {
                out.extend(h["command"].as_str().map(str::to_owned));
            }
        }
    }
    out
}

/// The rewritten command from a hook's stdout, if it changed `base` into something plain.
fn rewrite(stdout: &str, base: &str) -> Option<String> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let c = v["hookSpecificOutput"]["updatedInput"]["command"].as_str()?;
    // only simple words: a rewrite with operators or substitutions must not become an allow rule
    let plain = c
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || " -_./=:".contains(ch));
    (c != base && plain).then(|| c.to_owned())
}

fn hook_shell(hook: &str) -> Command {
    #[cfg(windows)]
    let (sh, flag) = ("cmd", "/C");
    #[cfg(not(windows))]
    let (sh, flag) = ("sh", "-c");
    let mut c = Command::new(sh);
    c.args([flag, hook]);
    c
}

/// Re-run `atclaude setup` in the background and return at once, so a shell's startup script
/// needs no shell-specific backgrounding syntax.
fn spawn_detached() -> Result<()> {
    let mut c = Command::new(std::env::current_exe()?);
    c.arg("setup")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0); // not part of the shell's job
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0008); // no window, detached
    c.spawn()?;
    Ok(())
}

fn probe(hook: &str, cmd: &str) -> Option<String> {
    let payload = json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": cmd}});
    let mut child = hook_shell(hook)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()?
        .write_all(payload.to_string().as_bytes())
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    rewrite(&out, cmd)
}

// NOTE: user settings only, hooks probed independently (no chaining), fixed command list;
// add project settings / chained hooks if someone needs them.
pub fn run(
    if_stale: bool,
    detach: bool,
    commands: &[String],
    config_files: &[PathBuf],
) -> Result<()> {
    let settings = settings_path();
    let rules_file = rules_path()?;
    if if_stale {
        let mtime = |p: &PathBuf| fs::metadata(p).and_then(|m| m.modified()).ok();
        let newest = settings.iter().chain(config_files).filter_map(mtime).max();
        if mtime(&rules_file).is_some_and(|r| newest.is_none_or(|n| r >= n)) {
            return Ok(());
        }
    }
    if detach {
        return spawn_detached();
    }
    let hooks = settings
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|v| bash_hooks(&v))
        .unwrap_or_default();

    let mut rules: Vec<String> = Vec::new();
    for hook in &hooks {
        for base in commands {
            if let Some(rw) = probe(hook, base.as_str()) {
                let rule = format!("Bash({rw}:*)");
                if !rules.contains(&rule) {
                    rules.push(rule);
                }
            }
        }
    }
    let tmp = rules_file.with_extension("tmp");
    fs::write(&tmp, rules.join(" "))?;
    fs::rename(&tmp, &rules_file)?;
    println!(
        "{} Bash hook(s) probed, {} extra read-only rule(s): {}",
        hooks.len(),
        rules.len(),
        rules.join(" ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let out = r#"{"hookSpecificOutput":{"updatedInput":{"command":"rtk git status"}}}"#;
        assert_eq!(
            rewrite(out, "git status").as_deref(),
            Some("rtk git status")
        );
        assert_eq!(rewrite(out, "rtk git status"), None);
        assert_eq!(
            rewrite(
                r#"{"hookSpecificOutput":{"updatedInput":{"command":"git status; rm x"}}}"#,
                "git status"
            ),
            None
        );
        assert_eq!(rewrite("", "ls"), None);
        let s: Value = serde_json::from_str(r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"a"}]},{"matcher":"Read","hooks":[{"type":"command","command":"b"}]}]}}"#).unwrap();
        assert_eq!(bash_hooks(&s), vec!["a"]);
    }
}
