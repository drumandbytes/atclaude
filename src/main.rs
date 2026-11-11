use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use db::{Database, NewTurn};
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::process::{exit, Command, Stdio};
use std::time::Instant;

mod config;
mod db;
mod history;
mod keychain;
mod llm;
mod session;
mod setup;

#[derive(Parser)]
#[command(
    name = "atclaude",
    version,
    about = "Inline shell companion for Claude Code"
)]
struct Cli {
    /// Prompt; with piped stdin it is the instruction and stdin the data
    prompt: Option<String>,

    /// Session name ("none" = no context). Default: .atclauderc, then terminal session
    #[arg(short, long, env = "ATCLAUDE_SESSION")]
    session: Option<String>,

    /// Fast model (haiku)
    #[arg(short, long, conflicts_with_all = ["deep", "model"])]
    fast: bool,

    /// Deep model (opus)
    #[arg(short, long, conflicts_with = "model")]
    deep: bool,

    /// Model name or alias passed to `claude --model`
    #[arg(short, long)]
    model: Option<String>,

    /// Save this turn to history (overrides `record = false`)
    #[arg(long, conflicts_with = "no_record")]
    record: bool,

    /// Do not save this turn to history
    #[arg(long)]
    no_record: bool,

    /// Allow file edits (needs an interactive terminal)
    #[arg(short, long)]
    agent: bool,

    /// Skip all permission checks: edits and any shell command (needs an interactive terminal)
    #[arg(short, long)]
    yolo: bool,

    /// Answer a question from past history (the helper model reads the history)
    #[arg(long, value_name = "QUESTION", conflicts_with = "prompt")]
    search: Option<String>,

    /// With --search: narrow the history (--since, --in SESSION, --repo NAME)
    #[command(flatten)]
    filter: history::FilterArgs,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print shell integration (zsh, bash, fish, powershell, nushell; see the README)
    Init {
        shell: Shell,
        /// Name of the command that runs atclaude (default `@claude`; `ask` for powershell/nushell)
        #[arg(long)]
        alias: Option<String>,
    },
    /// Print a fresh random terminal id (used by the shell integration)
    #[command(hide = true)]
    Id,
    /// Probe your Bash hooks and cache the rewritten commands read-only mode must allow
    Setup {
        /// Skip unless your settings.json or config is newer than the cache (used by the shell init)
        #[arg(long)]
        if_stale: bool,
        /// Do the work in a detached background process and return immediately
        #[arg(long)]
        detach: bool,
    },
    /// Show the effective settings and where each value comes from
    Config,
    /// Browse history (fzf when available), or manage it with show / rm / compact / clean
    History {
        #[command(flatten)]
        filter: history::FilterArgs,
        #[command(subcommand)]
        action: Option<HistoryCmd>,
    },
    /// List sessions or show the one this directory resolves to
    Session {
        #[command(subcommand)]
        action: SessionCmd,
    },
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// Print one turn
    Show { id: i64 },
    /// Delete turns by id or filter (asks first)
    Rm {
        ids: Vec<i64>,
        #[command(flatten)]
        filter: history::FilterArgs,
        /// Skip the confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Replace old turns with model-written summaries (shows them, then asks)
    Compact {
        #[command(flatten)]
        filter: history::FilterArgs,
        #[arg(long)]
        yes: bool,
    },
    /// Let the helper model pick turns to delete from an instruction (you confirm the list)
    Clean {
        instruction: String,
        #[command(flatten)]
        filter: history::FilterArgs,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum SessionCmd {
    List,
    Current,
}

#[derive(ValueEnum, Clone)]
enum Shell {
    Zsh,
    Bash,
    Fish,
    #[value(alias = "pwsh")]
    Powershell,
    #[value(alias = "nu")]
    Nushell,
}

impl Cli {
    fn model(&self, cfg: &Config) -> Option<String> {
        if self.fast {
            Some(cfg.str("fast_model").unwrap_or_else(|| "haiku".into()))
        } else if self.deep {
            Some(cfg.str("deep_model").unwrap_or_else(|| "opus".into()))
        } else {
            self.model.clone()
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("atclaude: {e:#}");
        exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Some(Cmd::Init { shell, alias }) => {
            print!("{}", init_script(shell, alias.as_deref())?);
            return Ok(());
        }
        Some(Cmd::Id) => {
            let mut b = [0u8; 8];
            getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
            println!("{:016x}", u64::from_le_bytes(b));
            return Ok(());
        }
        _ => {}
    }

    let cfg = Config::load(&std::env::current_dir()?);
    llm::set_binary(cfg.str("claude_path"));
    match &cli.command {
        Some(Cmd::Setup { if_stale, detach }) => {
            return setup::run(*if_stale, *detach, &cfg.readonly_commands(), &cfg.files)
        }
        Some(Cmd::Config) => return show_config(&cfg),
        Some(Cmd::Session { action }) => return session_cmd(action, &cfg),
        _ => {}
    }

    let db = Database::open_default(cfg.bool("headless").unwrap_or(false))?;
    match &cli.command {
        Some(Cmd::History {
            filter,
            action: None,
        }) => return history::list(&db, filter),
        Some(Cmd::History {
            action: Some(HistoryCmd::Show { id }),
            ..
        }) => return history::show(&db, *id),
        Some(Cmd::History {
            action: Some(HistoryCmd::Rm { ids, filter, yes }),
            ..
        }) => return history::rm(&db, ids, filter, *yes),
        Some(Cmd::History {
            action: Some(HistoryCmd::Compact { filter, yes }),
            ..
        }) => return history::compact(&db, &cfg, filter, *yes),
        Some(Cmd::History {
            action:
                Some(HistoryCmd::Clean {
                    instruction,
                    filter,
                    yes,
                }),
            ..
        }) => return history::clean(&db, &cfg, instruction, filter, *yes),
        _ => {}
    }
    if let Some(q) = &cli.search {
        let model = cli
            .model(&cfg)
            .or_else(|| cfg.str("helper_model"))
            .unwrap_or_else(|| "haiku".into());
        return history::search(&db, &cfg, q, &cli.filter, &model);
    }
    ask(&cli, &db, &cfg)
}

fn ask(cli: &Cli, db: &Database, cfg: &Config) -> Result<()> {
    let tty = std::io::stdin().is_terminal();
    let mut payload = String::new();
    if !tty {
        std::io::stdin().read_to_string(&mut payload)?;
    }
    let instruction = cli.prompt.clone().unwrap_or_default();
    if instruction.is_empty() && payload.is_empty() {
        bail!("usage: @claude \"prompt\"  |  cmd | @claude \"instruction\"  |  @claude --search \"question\"");
    }
    if (cli.agent || cli.yolo) && !tty {
        eprintln!(
            r#"{{"error":"permission_denied","reason":"agent mode needs an interactive terminal","exit_code":2}}"#
        );
        exit(2);
    }

    let cwd = std::env::current_dir()?;
    let tty_id = std::env::var("ATCLAUDE_TTY").ok();
    let session = session::resolve(
        cli.session.as_deref(),
        cfg.str("session").as_deref(),
        tty.then_some(tty_id.as_deref()).flatten(),
    );

    let mode = if cli.yolo {
        "yolo"
    } else if cli.agent {
        "agent"
    } else if tty && std::io::stdout().is_terminal() {
        "interactive"
    } else {
        "pipe"
    };
    let mut system = match mode {
        "yolo" | "agent" => cfg.str("prompt_agent").unwrap_or_else(|| llm::AGENT.into()),
        "interactive" => cfg
            .str("prompt_interactive")
            .unwrap_or_else(|| llm::INTERACTIVE.into()),
        _ => cfg.str("prompt_pipe").unwrap_or_else(|| llm::PIPE.into()),
    };
    let helper_model = cfg.str("helper_model").unwrap_or_else(|| "haiku".into());
    if let Some(s) = &session {
        let (turns, budget) = (
            cfg.num("context_turns").unwrap_or(8),
            cfg.num("context_budget").unwrap_or(6000),
        );
        match session::context(db, s, turns, budget, &helper_model) {
            Ok(Some(ctx)) => system.push_str(&format!(
                "\n\nEarlier turns in this session, for continuity:\n{ctx}"
            )),
            Ok(None) => {}
            Err(e) => eprintln!("[atclaude] no context: {e}"),
        }
    }

    let model = cli.model(cfg).or_else(|| cfg.str("model"));
    let ro;
    let args: Vec<&str> = if cli.yolo {
        vec!["--permission-mode", "bypassPermissions"]
    } else if cli.agent {
        vec!["--permission-mode", "acceptEdits"]
    } else {
        ro = llm::read_only_args(&cfg.readonly_commands(), &cfg.list("readonly_bash"));
        ro.iter().map(String::as_str).collect()
    };
    let started = Instant::now();
    let response = llm::complete(
        model.as_deref(),
        Some(&system),
        &args,
        &combine(&instruction, &payload),
    )?;
    // raw (piped) output: strip a wrapping code fence; a closed pipe (`| head`) is not an error
    let response = if std::io::stdout().is_terminal() {
        response
    } else {
        strip_fence(&response)
    };
    let _ = std::io::stdout().write_all(response.as_bytes());

    let record = if cli.record {
        true
    } else if cli.no_record {
        false
    } else {
        cfg.bool("record").unwrap_or(true)
    };
    if !record {
        return Ok(());
    }
    let (repo, branch) = git_info(&cwd);
    let prompt = if instruction.is_empty() {
        "(stdin)"
    } else {
        &instruction
    };
    let rec = db.record(&NewTurn {
        session: session.as_deref(),
        kind: "turn",
        cwd: &cwd.to_string_lossy(),
        repo: repo.as_deref(),
        branch: branch.as_deref(),
        mode,
        tty: tty_id.as_deref(),
        model: model.as_deref().unwrap_or("default"),
        prompt,
        payload: &truncate(&payload, cfg.num("payload_max_chars").unwrap_or(20_000)),
        response: &response,
        duration_ms: started.elapsed().as_millis() as i64,
        at: None,
    });
    if let Err(e) = rec {
        eprintln!("[atclaude] history not saved: {e}");
    }
    if let Err(e) = db.prune(
        cfg.num("retention_days").unwrap_or(365),
        cfg.num("retention_turns").unwrap_or(10_000),
    ) {
        eprintln!("[atclaude] retention pruning failed: {e}");
    }
    Ok(())
}

fn git_info(cwd: &Path) -> (Option<String>, Option<String>) {
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let repo = run(&["rev-parse", "--show-toplevel"]).and_then(|p| {
        Path::new(&p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });
    (repo, run(&["symbolic-ref", "--short", "-q", "HEAD"]))
}

/// 0 = unlimited.
fn truncate(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if max == 0 || n <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push_str(&format!("\n[truncated {} chars]", n - max));
    t
}

fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(body) = t.strip_prefix("```").and_then(|r| r.strip_suffix("```")) {
        if let Some((_lang, inner)) = body.split_once('\n') {
            if !inner.contains("```") {
                return format!("{}\n", inner.trim_end());
            }
        }
    }
    s.to_string()
}

fn combine(instruction: &str, payload: &str) -> String {
    match (instruction.is_empty(), payload.is_empty()) {
        (false, false) => format!("{instruction}\n\n{payload}"),
        (false, true) => instruction.to_string(),
        _ => payload.to_string(),
    }
}

fn session_cmd(action: &SessionCmd, cfg: &Config) -> Result<()> {
    match action {
        SessionCmd::List => {
            for (name, n, last) in
                Database::open_default(cfg.bool("headless").unwrap_or(false))?.sessions()?
            {
                println!("{name}\t{n} turns\tlast {last}");
            }
        }
        SessionCmd::Current => {
            let tty = std::io::stdin().is_terminal();
            let tty_id = std::env::var("ATCLAUDE_TTY").ok();
            let explicit = std::env::var("ATCLAUDE_SESSION").ok();
            let name = session::resolve(
                explicit.as_deref(),
                cfg.str("session").as_deref(),
                tty.then_some(tty_id.as_deref()).flatten(),
            );
            println!("{}", name.as_deref().unwrap_or("none (stateless)"));
        }
    }
    Ok(())
}

fn show_config(cfg: &Config) -> Result<()> {
    let files: Vec<_> = cfg.files.iter().map(|p| p.display().to_string()).collect();
    println!(
        "# files (lowest to highest priority): {}",
        if files.is_empty() {
            "none".into()
        } else {
            files.join(", ")
        }
    );
    println!(
        "# global: {}",
        config::global_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    println!("# env: ATCLAUDE_<KEY> overrides any file; flags override env");
    for (key, _) in config::KEYS {
        let (value, source) = cfg.describe(key);
        let value: String = value.chars().take(60).collect();
        println!("{key:<20} {value:<62} [{source}]");
    }
    Ok(())
}

/// Shell integration. The terminal id comes from `atclaude id` at shell start (not baked into the
/// output), so a cached init script still gives every terminal its own session.
fn init_script(shell: &Shell, alias: Option<&str>) -> Result<String> {
    let default = match shell {
        Shell::Zsh | Shell::Bash | Shell::Fish => "@claude",
        Shell::Powershell | Shell::Nushell => "ask", // `@name` can't be a command there
    };
    let alias = alias.unwrap_or(default);
    if alias.is_empty()
        || alias.starts_with('-')
        || !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@_.:+-".contains(c))
    {
        bail!("bad alias {alias:?}: use letters, digits and @ _ . : + -");
    }
    Ok(match shell {
        Shell::Zsh | Shell::Bash => format!("export ATCLAUDE_TTY=\"$(atclaude id)\"\nalias {alias}=atclaude\natclaude setup --if-stale --detach\n"),
        Shell::Fish => format!("set -gx ATCLAUDE_TTY (atclaude id)\nfunction {alias}; atclaude $argv; end\natclaude setup --if-stale --detach\n"),
        Shell::Powershell => format!("$env:ATCLAUDE_TTY = (atclaude id)\nSet-Alias -Name {alias} -Value atclaude -Scope Global\natclaude setup --if-stale --detach\n"),
        Shell::Nushell => format!("$env.ATCLAUDE_TTY = (^atclaude id | str trim)\nalias {alias} = atclaude\n^atclaude setup --if-stale --detach\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_fence_truncate() {
        assert_eq!(truncate("abcdef", 3), "abc\n[truncated 3 chars]");
        assert_eq!(truncate("abc", 0), "abc");
        for sh in [
            Shell::Zsh,
            Shell::Bash,
            Shell::Fish,
            Shell::Powershell,
            Shell::Nushell,
        ] {
            let script = init_script(&sh, None).unwrap();
            assert!(
                script.contains("ATCLAUDE_TTY")
                    && script.contains("setup --if-stale --detach")
                    && !script.contains("uuidgen")
            );
            assert!(init_script(&sh, Some("bad name; rm")).is_err());
        }
        assert!(init_script(&Shell::Zsh, Some("ask"))
            .unwrap()
            .contains("alias ask=atclaude"));
        assert_eq!(combine("do x", "data"), "do x\n\ndata");
        assert_eq!(combine("", "data"), "data");
        assert_eq!(strip_fence("```json\n{\"a\":1}\n```\n"), "{\"a\":1}\n");
        assert_eq!(strip_fence("plain\n"), "plain\n");
        assert_eq!(strip_fence("a\n```\nx\n```\nb"), "a\n```\nx\n```\nb");
    }
}
