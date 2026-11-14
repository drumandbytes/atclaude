use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use config::Config;
use provider::{Provider, Request, Tier};
use std::io::{IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio, exit};
use std::time::Instant;
use store::{Content, Meta, Store};

mod config;
mod history;
mod key;
mod keychain;
mod provider;
mod store;

const PROMPT_PIPE: &str = "You are a command-line tool in a UNIX pipeline. Output ONLY the raw text or data requested. No markdown, no code fences, no greetings, no commentary.";
const PROMPT_INTERACTIVE: &str = "You are an inline terminal companion. Answer directly, no pleasantries. Keep it under 15 lines unless asked for more. Prefer commands, snippets and paths over background.";

#[derive(Parser)]
#[command(
    name = "atclaude",
    version,
    about = "Call AI models from the command line like any other filter"
)]
struct Cli {
    /// The instruction; piped stdin is the data it applies to
    prompt: Option<String>,

    /// Provider to use (a built-in: anthropic, openai; or a [providers.NAME] from the config)
    #[arg(short, long)]
    provider: Option<String>,

    /// Fast model tier
    #[arg(short, long, conflicts_with_all = ["deep", "model"])]
    fast: bool,

    /// Reasoning model tier
    #[arg(short, long, conflicts_with = "model")]
    deep: bool,

    /// An exact model name
    #[arg(short, long)]
    model: Option<String>,

    /// Call the model even if an identical call is in the history
    #[arg(long)]
    no_cache: bool,

    /// Save this turn to history (overrides `record = false`)
    #[arg(long, conflicts_with = "no_record")]
    record: bool,

    /// Do not save this turn to history
    #[arg(long)]
    no_record: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the effective settings and where each value comes from
    Config,
    /// Browse history (fzf when available), or show / delete turns
    History {
        #[command(flatten)]
        filter: history::FilterArgs,
        #[command(subcommand)]
        action: Option<HistoryCmd>,
    },
    /// Back up, restore or rotate the key that protects the history
    Key {
        #[command(subcommand)]
        action: KeyCmd,
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
}

#[derive(Subcommand)]
enum KeyCmd {
    /// Print the key as 24 words, to write down or move to another machine
    Show,
    /// Restore a key from its 24 words (read from stdin)
    Import,
    /// Replace the key with a new one; the history stays readable
    Rotate,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("atclaude: {e:#}");
        exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&std::env::current_dir()?);
    let keychain = cfg.bool("keychain").unwrap_or(false);
    match &cli.command {
        Some(Cmd::Config) => show_config(&cfg),
        Some(Cmd::Key { action }) => key_cmd(action, keychain),
        Some(Cmd::History {
            filter,
            action: None,
        }) => history::list(&Store::open(keychain)?, filter),
        Some(Cmd::History {
            action: Some(HistoryCmd::Show { id }),
            ..
        }) => history::show(&Store::open(keychain)?, *id),
        Some(Cmd::History {
            action: Some(HistoryCmd::Rm { ids, filter, yes }),
            ..
        }) => history::rm(&Store::open(keychain)?, ids, filter, *yes),
        None => ask(&cli, &cfg, keychain),
    }
}

fn ask(cli: &Cli, cfg: &Config, keychain: bool) -> Result<()> {
    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let mut payload = String::new();
    if !stdin_tty {
        std::io::stdin().read_to_string(&mut payload)?;
    }
    let instruction = cli.prompt.clone().unwrap_or_default();
    if instruction.is_empty() && payload.trim().is_empty() {
        bail!("nothing to do. Usage: atclaude \"question\"   or   cmd | atclaude \"instruction\"");
    }

    // Raw output whenever either end is a pipe.
    let piped = !stdin_tty || !stdout_tty;
    let mode = if piped { "pipe" } else { "interactive" };
    let system = if piped {
        cfg.str("prompt_pipe").unwrap_or_else(|| PROMPT_PIPE.into())
    } else {
        cfg.str("prompt_interactive")
            .unwrap_or_else(|| PROMPT_INTERACTIVE.into())
    };

    let provider = Provider::resolve(cfg, cli.provider.as_deref())?;
    let tier = if cli.fast {
        Tier::Fast
    } else if cli.deep {
        Tier::Deep
    } else {
        Tier::Regular
    };
    let model = match &cli.model {
        Some(m) => m.clone(),
        None => provider.model(cfg, tier)?,
    };
    let effort = provider.effort(cfg, tier);
    let max_tokens = cfg.num("max_tokens");
    let user = combine(&instruction, &payload);

    let record = if cli.record {
        true
    } else if cli.no_record {
        false
    } else {
        cfg.bool("record").unwrap_or(true)
    };
    let cache_mode = cfg.str("cache").unwrap_or_else(|| "pipe".into());
    if !["pipe", "always", "off"].contains(&cache_mode.as_str()) {
        bail!("`cache` must be \"pipe\", \"always\" or \"off\"");
    }
    let use_cache = !cli.no_cache && (cache_mode == "always" || (cache_mode == "pipe" && piped));

    let cwd = std::env::current_dir()?;
    let dir = store::dir_hash(&cwd);
    let store = if record || use_cache {
        Some(Store::open(keychain)?)
    } else {
        None
    };
    let cache_key = store
        .as_ref()
        .map(|s| {
            s.cache_key(&[
                &provider.identity(),
                &model,
                &system,
                &user,
                effort.as_deref().unwrap_or(""),
                &max_tokens.map(|m| m.to_string()).unwrap_or_default(),
            ])
        })
        .unwrap_or_default();

    if use_cache && let Some(s) = &store {
        let ttl = history::age_secs(&cfg.str("cache_ttl").unwrap_or_else(|| "7d".into()))?;
        if let Some(hit) = s.cached(&cache_key, ttl, &dir)? {
            let _ = std::io::stdout().write_all(hit.as_bytes());
            return Ok(());
        }
    }

    let started = Instant::now();
    let req = Request {
        model: &model,
        system: &system,
        user: &user,
        effort: effort.as_deref(),
        max_tokens,
    };
    // A terminal gets the answer as it arrives; a pipe gets it whole, so a wrapping code fence
    // can be stripped. NOTE: filter the fence while streaming if piped latency ever matters.
    let reply = provider.complete(&req, &mut |piece| {
        if !piped {
            print!("{piece}");
            let _ = std::io::stdout().flush();
        }
    })?;
    if reply.stop == "refusal" {
        bail!("the model declined this request");
    }
    let mut text = if piped {
        strip_fence(&reply.text)
    } else {
        reply.text.clone()
    };
    if !text.ends_with('\n') {
        text.push('\n');
        if !piped {
            println!();
        }
    }
    if piped {
        // a closed pipe (`| head`) is not an error
        let _ = std::io::stdout().write_all(text.as_bytes());
    }
    let _ = std::io::stdout().flush();
    if matches!(reply.stop.as_str(), "max_tokens" | "length") {
        eprintln!("[atclaude] the answer was cut off at the output limit (raise `max_tokens`)");
    }

    // Bookkeeping happens after the output is out, so nothing downstream waits for it.
    if record && let Some(s) = &store {
        let (repo, branch) = git_info(&cwd);
        let meta = Meta {
            dir: &dir,
            provider: &provider.name,
            model: &reply.model,
            mode,
            tier: if cli.model.is_some() {
                "custom"
            } else {
                tier.name()
            },
            input_tokens: reply.input_tokens,
            output_tokens: reply.output_tokens,
            duration_ms: started.elapsed().as_millis() as u64,
            cache_key: &cache_key,
        };
        let content = Content {
            instruction,
            payload,
            response: text,
            cwd: cwd.to_string_lossy().into_owned(),
            repo,
            branch,
        };
        if let Err(e) = s.record(&meta, &content) {
            eprintln!("[atclaude] history not saved: {e:#}");
        }
    }
    Ok(())
}

fn combine(instruction: &str, payload: &str) -> String {
    match (instruction.is_empty(), payload.is_empty()) {
        (false, false) => format!("{instruction}\n\n{payload}"),
        (false, true) => instruction.to_string(),
        _ => payload.to_string(),
    }
}

fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(body) = t.strip_prefix("```").and_then(|r| r.strip_suffix("```"))
        && let Some((_lang, inner)) = body.split_once('\n')
        && !inner.contains("```")
    {
        return format!("{}\n", inner.trim_end());
    }
    s.to_string()
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

fn key_cmd(action: &KeyCmd, keychain: bool) -> Result<()> {
    let missing =
        || anyhow!("there is no key yet; one is created the first time history is written");
    match action {
        KeyCmd::Show => {
            let k = key::load(keychain)?.ok_or_else(missing)?;
            println!("{}", key::to_words(&k)?);
            eprintln!(
                "\nAnyone with these words and your history file can read it. Keep them somewhere safe."
            );
        }
        KeyCmd::Import => {
            let mut words = String::new();
            std::io::stdin().read_to_string(&mut words)?;
            let k = key::from_words(&words)?;
            if !Store::key_fits(&store::path()?, &k)? {
                bail!("that key does not unlock the existing history; nothing was changed");
            }
            key::save(&k, keychain)?;
            println!("key imported");
        }
        KeyCmd::Rotate => {
            key::load(keychain)?.ok_or_else(missing)?;
            let store = Store::open(keychain)?;
            let new = key::random()?;
            store.rewrap(&new, keychain)?;
            println!(
                "key rotated; the old key no longer works. Back up the new one with `atclaude key show`."
            );
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
    println!(
        "# providers: anthropic, openai (built in){}",
        cfg.provider_names()
            .iter()
            .map(|n| format!(", {n}"))
            .collect::<String>()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_and_fence() {
        assert_eq!(combine("do x", "data"), "do x\n\ndata");
        assert_eq!(combine("", "data"), "data");
        assert_eq!(strip_fence("```json\n{\"a\":1}\n```\n"), "{\"a\":1}\n");
        assert_eq!(strip_fence("plain\n"), "plain\n");
        assert_eq!(strip_fence("a\n```\nx\n```\nb"), "a\n```\nx\n```\nb");
    }
}
