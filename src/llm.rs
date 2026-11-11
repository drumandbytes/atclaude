use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static BINARY: OnceLock<String> = OnceLock::new();

/// The `claude_path` setting (custom install location, or a `.cmd` shim on Windows).
pub fn set_binary(path: Option<String>) {
    if let Some(p) = path {
        let _ = BINARY.set(p);
    }
}

pub const PIPE: &str = "You are a POSIX command-line tool in a UNIX pipeline. Output ONLY the raw text or data requested. No markdown, no code fences, no greetings, no commentary.";
pub const INTERACTIVE: &str = "You are an inline terminal companion. Answer directly, no pleasantries. Keep it under 15 lines unless asked for more. Prefer commands, snippets and paths over background.";
pub const AGENT: &str = "You are a workspace agent. After edits or test runs, output ONLY a concise ledger: `[ACTION] <path> | [STATUS] <ok/fail> | [SUMMARY] <one line>`. No diff dumps unless asked.";

/// Flags for helper calls (pruner, search): no tools, no CLAUDE.md/hooks/MCP.
/// `--tools` is variadic, so it goes last.
pub const HELPER_ARGS: &[&str] = &["--safe-mode", "--tools", ""];

/// Read-only main calls: deny everything except these rules (dontAsk). User settings stay on, so
/// hooks that rewrite commands (RTK: `git status` -> `rtk git status`) run before rule matching;
/// `atclaude setup` probes the hooks and caches the rewritten forms, which are allowed too.
/// The `readonly_bash` setting adds rules by hand. Variadic flags go last.
pub fn read_only_args(commands: &[String], extra: &[String]) -> Vec<String> {
    let mut rules = commands
        .iter()
        .map(|c| format!("Bash({c}:*)"))
        .collect::<Vec<_>>()
        .join(" ");
    match crate::setup::cached_rules() {
        Some(extra) => {
            rules.push(' ');
            rules.push_str(&extra);
        }
        None => eprintln!("[atclaude] no hook rules cached; if you have Bash hooks that rewrite commands, run `atclaude setup`"),
    }
    for r in extra {
        rules.push(' ');
        rules.push_str(r);
    }
    [
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        &rules,
        "--tools",
        "Read,Glob,Grep,WebFetch,WebSearch,Bash",
    ]
    .map(String::from)
    .into()
}

pub fn complete(
    model: Option<&str>,
    system: Option<&str>,
    args: &[&str],
    prompt: &str,
) -> Result<String> {
    let bin = BINARY.get().map(String::as_str).unwrap_or("claude");
    let mut cmd = Command::new(bin);
    cmd.args(["-p", "--output-format", "text"]);
    if let Some(m) = model {
        cmd.args(["--model", m]);
    }
    if let Some(s) = system {
        cmd.args(["--append-system-prompt", s]);
    }
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().with_context(|| format!("failed to run `{bin}` (is Claude Code installed and on PATH? see the `claude_path` setting)"))?;
    // prompt goes via stdin: no argv size limit for piped logs
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(prompt.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("claude exited with {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
