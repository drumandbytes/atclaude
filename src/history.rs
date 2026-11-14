use crate::store::{Filter, Store, Turn, dir_hash};
use anyhow::{Result, anyhow, bail};
use clap::Args;
use std::io::{BufRead, IsTerminal, Write};
use std::process::{Command, Stdio};

#[derive(Args, Default, Clone)]
pub struct FilterArgs {
    /// Only turns newer than this (12h, 3d, 2w, 1m)
    #[arg(long)]
    pub since: Option<String>,
    /// Only turns older than this
    #[arg(long)]
    pub older: Option<String>,
    /// Only turns from the current directory
    #[arg(long)]
    pub here: bool,
}

impl FilterArgs {
    pub fn filter(&self) -> Result<Filter> {
        Ok(Filter {
            since: self.since.as_deref().map(age_secs).transpose()?,
            older: self.older.as_deref().map(age_secs).transpose()?,
            dir: if self.here {
                Some(dir_hash(&std::env::current_dir()?))
            } else {
                None
            },
        })
    }

    pub fn any(&self) -> bool {
        self.since.is_some() || self.older.is_some() || self.here
    }
}

/// "12h", "3d", "2w", "1m" (30 days) as seconds.
pub fn age_secs(s: &str) -> Result<i64> {
    let bad = || anyhow!("bad age {s:?}; use e.g. 12h, 3d, 2w, 1m");
    if s.len() < 2 || !s.is_char_boundary(s.len() - 1) {
        return Err(bad());
    }
    let (n, unit) = s.split_at(s.len() - 1);
    let n: i64 = n.parse().map_err(|_| bad())?;
    Ok(n * match unit {
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        "m" => 30 * 86_400,
        _ => return Err(bad()),
    })
}

fn oneline(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn label(t: &Turn) -> String {
    if t.content.instruction.is_empty() {
        format!("(stdin) {}", oneline(&t.content.payload, 100))
    } else {
        oneline(&t.content.instruction, 120)
    }
}

pub fn list(store: &Store, f: &FilterArgs) -> Result<()> {
    let lines: Vec<String> = store
        .list(&f.filter()?, 2000)?
        .iter()
        .map(|t| {
            format!(
                "{}\t{}\t{}\t{}",
                t.id,
                t.when,
                t.content.repo.as_deref().unwrap_or("-"),
                label(t)
            )
        })
        .collect();

    let fzf = if std::io::stdout().is_terminal() {
        let exe = std::env::current_exe()?;
        Command::new("fzf")
            .args([
                "--delimiter",
                "\t",
                "--with-nth",
                "2..",
                "--preview-window",
                "right:60%:wrap",
                "--preview",
            ])
            .arg(if cfg!(windows) {
                format!("\"{}\" history show {{1}}", exe.display())
            } else {
                format!("'{}' history show {{1}}", exe.display())
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .ok()
    } else {
        None
    };
    let Some(mut fzf) = fzf else {
        if !lines.is_empty() {
            println!("{}", lines.join("\n"));
        }
        return Ok(());
    };
    // fzf may exit early (esc); a broken pipe on write is fine
    let _ = fzf
        .stdin
        .take()
        .expect("piped")
        .write_all(lines.join("\n").as_bytes());
    let out = fzf.wait_with_output()?;
    let picked = String::from_utf8_lossy(&out.stdout);
    if let Some(id) = picked
        .split('\t')
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
    {
        show(store, id)?;
    }
    Ok(())
}

pub fn show(store: &Store, id: i64) -> Result<()> {
    let Some(t) = store.get(id)? else {
        bail!("no turn #{id}")
    };
    let c = &t.content;
    let place = match (&c.repo, &c.branch) {
        (Some(r), Some(b)) => format!(" {r}@{b}"),
        (Some(r), None) => format!(" {r}"),
        _ => String::new(),
    };
    println!(
        "#{} {}{place} {} ({} {}, {}, {} in / {} out tokens)",
        t.id, t.when, c.cwd, t.provider, t.model, t.mode, t.input_tokens, t.output_tokens
    );
    println!("\n> {}", c.instruction);
    if !c.payload.is_empty() {
        println!("\n--- stdin ({} bytes) ---\n{}", c.payload.len(), c.payload);
    }
    println!("\n{}", c.response);
    Ok(())
}

fn confirm(question: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("{question}\nnot a terminal; pass --yes to confirm");
    }
    eprint!("{question} [y/N] ");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

/// Delete by id, or by filter (a filter is required so a bare `rm` can't wipe everything).
pub fn rm(store: &Store, ids: &[i64], f: &FilterArgs, yes: bool) -> Result<()> {
    let turns: Vec<Turn> = if !ids.is_empty() {
        ids.iter()
            .filter_map(|i| store.get(*i).transpose())
            .collect::<Result<_>>()?
    } else if f.any() {
        store.list(&f.filter()?, 1_000_000)?
    } else {
        bail!("give turn ids or a filter (--older 90d, --since 1d, --here)");
    };
    if turns.is_empty() {
        println!("nothing matches");
        return Ok(());
    }
    for t in turns.iter().take(8) {
        println!("  #{} {} {}", t.id, t.when, label(t));
    }
    if turns.len() > 8 {
        println!("  ... and {} more", turns.len() - 8);
    }
    if !confirm(
        &format!("Delete {} turn(s)? This cannot be undone.", turns.len()),
        yes,
    )? {
        println!("aborted");
        return Ok(());
    }
    println!(
        "deleted {}",
        store.delete(&turns.iter().map(|t| t.id).collect::<Vec<_>>())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(age_secs("2w").unwrap(), 14 * 86_400);
        assert_eq!(age_secs("12h").unwrap(), 43_200);
        assert!(age_secs("").is_err() && age_secs("d").is_err() && age_secs("3x").is_err());
    }
}
