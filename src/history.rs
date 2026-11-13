use crate::config::Config;
use crate::db::{Database, Filter, NewTurn, Turn};
use crate::llm;
use anyhow::{Result, anyhow, bail};
use clap::Args;
use std::collections::BTreeMap;
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
    /// Only this session
    #[arg(long = "in", value_name = "SESSION")]
    pub in_session: Option<String>,
    /// Only this repository (directory name of the git root)
    #[arg(long)]
    pub repo: Option<String>,
}

impl FilterArgs {
    pub fn filter(&self) -> Result<Filter> {
        Ok(Filter {
            since: self.since.as_deref().map(since_modifier).transpose()?,
            older: self.older.as_deref().map(since_modifier).transpose()?,
            session: self.in_session.clone(),
            repo: self.repo.clone(),
        })
    }

    pub fn any(&self) -> bool {
        self.since.is_some()
            || self.older.is_some()
            || self.in_session.is_some()
            || self.repo.is_some()
    }
}

pub fn since_modifier(s: &str) -> Result<String> {
    let bad = || anyhow!("bad age {s:?}; use e.g. 12h, 3d, 2w, 1m");
    if s.len() < 2 || !s.is_char_boundary(s.len() - 1) {
        return Err(bad());
    }
    let (n, unit) = s.split_at(s.len() - 1);
    let n: u32 = n.parse().map_err(|_| bad())?;
    Ok(match unit {
        "h" => format!("-{n} hours"),
        "d" => format!("-{n} days"),
        "w" => format!("-{} days", n * 7),
        "m" => format!("-{n} months"),
        _ => return Err(bad()),
    })
}

pub fn oneline(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

fn helper_model(cfg: &Config) -> String {
    cfg.str("helper_model").unwrap_or_else(|| "haiku".into())
}

/// One line per turn for a model to read, newest turns first until the budget is spent,
/// returned oldest first with the turn id. The bool says whether older turns were cut.
fn index(turns: &[Turn], cfg: &Config) -> (Vec<(i64, String)>, bool) {
    let (pc, rc) = (
        cfg.num("search_prompt_chars").unwrap_or(200),
        cfg.num("search_response_chars").unwrap_or(400),
    );
    let budget = cfg.num("search_budget").unwrap_or(300_000);
    let mut out = Vec::new();
    let mut used = 0;
    let mut cut = false;
    for t in turns {
        let mut meta = vec![t.session.clone().unwrap_or_else(|| "-".into())];
        if let Some(r) = &t.repo {
            meta.push(match &t.branch {
                Some(b) => format!("{r}@{b}"),
                None => r.clone(),
            });
        }
        meta.push(if t.kind == "summary" {
            "summary".into()
        } else {
            t.mode.clone()
        });
        let line = format!(
            "#{} {} [{}] Q: {} | A: {}",
            t.id,
            t.ts,
            meta.join(" "),
            oneline(&t.prompt, pc),
            oneline(&t.response, rc)
        );
        used += line.chars().count();
        if used > budget {
            cut = true;
            break;
        }
        out.push((t.id, line));
    }
    out.reverse();
    (out, cut)
}

pub fn search(
    db: &Database,
    cfg: &Config,
    question: &str,
    f: &FilterArgs,
    model: &str,
) -> Result<()> {
    let turns = db.query(&f.filter()?, 5000)?;
    let (lines, cut) = index(&turns, cfg);
    if lines.is_empty() {
        bail!("no history in range");
    }
    if cut {
        eprintln!(
            "[atclaude] history too large for one search, using the most recent {} turns (narrow with --since/--in/--repo)",
            lines.len()
        );
    }
    let body = lines
        .iter()
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let out = llm::complete(
        Some(model),
        Some(
            "You answer questions about the user's past command-line assistant history. Use only the history entries. Cite entry ids like #12. If the history does not contain the answer, say so. Be brief.",
        ),
        llm::HELPER_ARGS,
        &format!(
            "Now: {}\n\nHistory:\n{body}\n\nQuestion: {question}",
            db.now()?
        ),
    )?;
    println!("{}", out.trim_end());
    Ok(())
}

pub fn list(db: &Database, f: &FilterArgs) -> Result<()> {
    let lines: Vec<String> = db
        .query(&f.filter()?, 2000)?
        .iter()
        .map(|t| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                t.id,
                t.ts,
                t.session.as_deref().unwrap_or("-"),
                t.repo.as_deref().unwrap_or("-"),
                oneline(&t.prompt, 120)
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
        show(db, id)?;
    }
    Ok(())
}

pub fn show(db: &Database, id: i64) -> Result<()> {
    let Some(t) = db.get(id)? else {
        bail!("no turn #{id}")
    };
    let place = match (&t.repo, &t.branch) {
        (Some(r), Some(b)) => format!(" {r}@{b}"),
        (Some(r), None) => format!(" {r}"),
        _ => String::new(),
    };
    println!(
        "#{} {} [{}]{place} {} ({}, {}, {})",
        t.id,
        t.ts,
        t.session.as_deref().unwrap_or("-"),
        t.cwd,
        t.model,
        t.mode,
        t.kind
    );
    println!("\n> {}", t.prompt);
    if !t.payload.is_empty() {
        println!("\n--- stdin ({} bytes) ---\n{}", t.payload.len(), t.payload);
    }
    println!("\n{}", t.response);
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

fn preview(turns: &[&Turn]) {
    for t in turns.iter().take(8) {
        println!(
            "  #{} {} [{}] {}",
            t.id,
            t.ts,
            t.session.as_deref().unwrap_or("-"),
            oneline(&t.prompt, 80)
        );
    }
    if turns.len() > 8 {
        println!("  ... and {} more", turns.len() - 8);
    }
}

fn delete_after_confirm(db: &Database, turns: &[&Turn], yes: bool) -> Result<()> {
    if turns.is_empty() {
        println!("nothing matches");
        return Ok(());
    }
    preview(turns);
    if !confirm(
        &format!("Delete {} turn(s)? This cannot be undone.", turns.len()),
        yes,
    )? {
        println!("aborted");
        return Ok(());
    }
    println!(
        "deleted {}",
        db.delete_ids(&turns.iter().map(|t| t.id).collect::<Vec<_>>())?
    );
    Ok(())
}

/// Delete by id, or by filter (a filter is required so a bare `rm` can't wipe everything).
pub fn rm(db: &Database, ids: &[i64], f: &FilterArgs, yes: bool) -> Result<()> {
    let turns: Vec<Turn> = if !ids.is_empty() {
        ids.iter()
            .filter_map(|i| db.get(*i).transpose())
            .collect::<Result<_>>()?
    } else if f.any() {
        db.query(&f.filter()?, 1_000_000)?
    } else {
        bail!("give turn ids or a filter (--older 90d, --in SESSION, --repo NAME, --since ...)");
    };
    delete_after_confirm(db, &turns.iter().collect::<Vec<_>>(), yes)
}

/// Ask the helper model which turns to delete. It only sees the index; you confirm the list.
pub fn clean(
    db: &Database,
    cfg: &Config,
    instruction: &str,
    f: &FilterArgs,
    yes: bool,
) -> Result<()> {
    let turns = db.query(&f.filter()?, 5000)?;
    let (lines, cut) = index(&turns, cfg);
    if lines.is_empty() {
        bail!("no history in range");
    }
    if cut {
        eprintln!(
            "[atclaude] only the most recent {} turns fit; narrow with --older/--in/--repo to reach older ones",
            lines.len()
        );
    }
    let body = lines
        .iter()
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let out = llm::complete(
        Some(&helper_model(cfg)),
        Some(
            "Select history entries to delete according to the instruction. Output ONLY a JSON array of integer entry ids from the list. Be conservative: when unsure, leave an entry out. Output [] if nothing matches.",
        ),
        llm::HELPER_ARGS,
        &format!("Instruction: {instruction}\n\nHistory:\n{body}"),
    )?;
    let (a, b) = (out.find('['), out.rfind(']'));
    let picked: Vec<i64> = match (a, b) {
        (Some(a), Some(b)) if a < b => serde_json::from_str(&out[a..=b])
            .map_err(|e| anyhow!("model returned unusable ids: {e}"))?,
        _ => bail!("model returned no id list"),
    };
    let shown: Vec<i64> = lines.iter().map(|(id, _)| *id).collect();
    let targets: Vec<&Turn> = turns
        .iter()
        .filter(|t| picked.contains(&t.id) && shown.contains(&t.id))
        .collect();
    delete_after_confirm(db, &targets, yes)
}

const COMPACT_CHUNK: usize = 60_000; // chars per summarization call
const COMPACT_RESPONSE: usize = 1500; // chars of each response the summarizer sees

struct Digest<'a> {
    session: Option<String>,
    turns: Vec<&'a Turn>,
    text: String,
}

/// Replace old turns with one model-written digest per session chunk. Summaries are shown
/// before anything is deleted; piped payloads of compacted turns are dropped.
// NOTE: chunks are consecutive turns of one session capped by size; no cross-chunk merge.
pub fn compact(db: &Database, cfg: &Config, f: &FilterArgs, yes: bool) -> Result<()> {
    if !f.any() {
        bail!("give a filter, e.g. --older 90d (optionally --in SESSION / --repo NAME)");
    }
    let mut turns = db.query(&f.filter()?, 1_000_000)?;
    turns.retain(|t| t.kind == "turn");
    turns.reverse(); // oldest first

    let mut by_session: BTreeMap<Option<String>, Vec<&Turn>> = BTreeMap::new();
    for t in &turns {
        by_session.entry(t.session.clone()).or_default().push(t);
    }
    let mut chunks: Vec<(Option<String>, Vec<&Turn>)> = Vec::new();
    for (session, ts) in by_session {
        let (mut cur, mut size) = (Vec::new(), 0);
        for t in ts {
            size += t.prompt.len() + t.response.len().min(COMPACT_RESPONSE);
            cur.push(t);
            if size > COMPACT_CHUNK {
                chunks.push((session.clone(), std::mem::take(&mut cur)));
                size = 0;
            }
        }
        chunks.push((session.clone(), cur));
    }
    chunks.retain(|(_, c)| c.len() >= 2);
    if chunks.is_empty() {
        println!("nothing to compact (needs at least 2 turns in a session)");
        return Ok(());
    }

    let model = helper_model(cfg);
    let mut digests = Vec::new();
    for (session, ts) in chunks {
        let body = ts
            .iter()
            .map(|t| {
                format!(
                    "[#{} {}] Q: {}\nA: {}",
                    t.id,
                    t.ts,
                    t.prompt,
                    t.response
                        .chars()
                        .take(COMPACT_RESPONSE)
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = llm::complete(
            Some(&model),
            Some(
                "Condense these assistant turns into a compact digest of at most 25 lines. Keep facts, decisions, commands, file paths, names, errors and outcomes; drop pleasantries and repetition. Output only the digest.",
            ),
            llm::HELPER_ARGS,
            &body,
        )?;
        digests.push(Digest {
            session,
            turns: ts,
            text: text.trim().to_string(),
        });
    }

    for d in &digests {
        println!(
            "--- {} turns of [{}] #{}..#{} ---\n{}\n",
            d.turns.len(),
            d.session.as_deref().unwrap_or("-"),
            d.turns[0].id,
            d.turns[d.turns.len() - 1].id,
            d.text
        );
    }
    let total: usize = digests.iter().map(|d| d.turns.len()).sum();
    if !confirm(
        &format!(
            "Replace {total} turn(s) with {} summary(ies)? The originals are deleted.",
            digests.len()
        ),
        yes,
    )? {
        println!("aborted");
        return Ok(());
    }
    for d in &digests {
        let (first, last) = (d.turns[0], d.turns[d.turns.len() - 1]);
        let prompt = format!(
            "(summary of {} turns, #{}..#{})",
            d.turns.len(),
            first.id,
            last.id
        );
        let ids: Vec<i64> = d.turns.iter().map(|t| t.id).collect();
        db.replace(
            &NewTurn {
                session: d.session.as_deref(),
                kind: "summary",
                cwd: "",
                repo: None,
                branch: None,
                mode: "compact",
                tty: None,
                model: &model,
                prompt: &prompt,
                payload: "",
                response: &d.text,
                duration_ms: 0,
                at: Some(&last.ts),
            },
            &ids,
        )?;
    }
    println!(
        "compacted {total} turn(s) into {} summary(ies)",
        digests.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(since_modifier("2w").unwrap(), "-14 days");
        assert_eq!(since_modifier("12h").unwrap(), "-12 hours");
        assert!(
            since_modifier("").is_err()
                && since_modifier("d").is_err()
                && since_modifier("3x").is_err()
        );
    }
}
