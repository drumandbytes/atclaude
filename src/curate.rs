//! `history ask` and `history agent`: a model reads excerpts of the history to answer a question
//! about it or to pick turns to delete. These calls are not recorded.
use crate::config::Config;
use crate::history::{FilterArgs, confirm, oneline};
use crate::provider::{Provider, Request, Tier};
use crate::store::{Store, Turn};
use anyhow::{Result, bail};
use std::io::Write;

const ASK_SYSTEM: &str = "You answer questions about a user's command-line AI history. Each entry is `id | date | place | Q: prompt | A: answer`. Answer briefly and cite entry ids as #id. If the history does not contain the answer, say so.";
const AGENT_SYSTEM: &str = "You help clean up a user's command-line AI history. Each entry is `id | date | place | Q: prompt | A: answer`. Follow the user's instruction and reply with ONLY a JSON array of the integer ids of the entries to delete, for example [3,8]. Use only ids that are listed. If nothing matches, reply [].";

fn listing(turns: &[Turn], cfg: &Config) -> String {
    let q = cfg.num("excerpt_prompt").unwrap_or(200) as usize;
    let a = cfg.num("excerpt_reply").unwrap_or(400) as usize;
    turns
        .iter()
        .map(|t| {
            let c = &t.content;
            let place = c.repo.as_deref().unwrap_or(&c.cwd);
            format!(
                "{} | {} | {} | Q: {} | A: {}",
                t.id,
                t.when,
                place,
                oneline(&crate::history::label(t), q),
                oneline(&c.response, a)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn call(cfg: &Config, tier: Tier, system: &str, user: &str, show: bool) -> Result<String> {
    let provider = Provider::resolve(cfg, None)?;
    let model = provider.model(cfg, tier)?;
    let effort = provider.effort(cfg, tier);
    let req = Request {
        model: &model,
        system,
        user,
        history: &[],
        effort: effort.as_deref(),
        max_tokens: cfg.num("max_tokens"),
    };
    let reply = provider.complete(&req, &mut |piece| {
        if show {
            print!("{piece}");
            let _ = std::io::stdout().flush();
        }
    })?;
    Ok(reply.text)
}

fn scan(store: &Store, f: &FilterArgs, cfg: &Config) -> Result<Vec<Turn>> {
    let turns = store.list(
        &f.filter()?,
        cfg.num("history_scan").unwrap_or(200) as usize,
    )?;
    if turns.is_empty() {
        bail!("the history has no matching turns");
    }
    Ok(turns)
}

/// Answer a question about the history, using the fast tier.
pub fn ask(store: &Store, cfg: &Config, f: &FilterArgs, question: &str) -> Result<()> {
    let turns = scan(store, f, cfg)?;
    let user = format!("{}\n\nQuestion: {question}", listing(&turns, cfg));
    let text = call(cfg, Tier::Fast, ASK_SYSTEM, &user, true)?;
    if !text.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// The ids the model picked, restricted to turns that were offered.
fn picked(reply: &str, offered: &[Turn]) -> Vec<i64> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else {
        return Vec::new();
    };
    let ids: Vec<i64> = serde_json::from_str(&reply[a..=b]).unwrap_or_default();
    offered
        .iter()
        .map(|t| t.id)
        .filter(|id| ids.contains(id))
        .collect()
}

/// Let the model propose deletions for an instruction; nothing is deleted without confirmation.
pub fn agent(
    store: &Store,
    cfg: &Config,
    f: &FilterArgs,
    instruction: &str,
    yes: bool,
) -> Result<()> {
    let turns = scan(store, f, cfg)?;
    let user = format!("{}\n\nInstruction: {instruction}", listing(&turns, cfg));
    let reply = call(cfg, Tier::Regular, AGENT_SYSTEM, &user, false)?;
    let ids = picked(&reply, &turns);
    if ids.is_empty() {
        println!("nothing to delete");
        return Ok(());
    }
    for t in turns.iter().filter(|t| ids.contains(&t.id)).take(15) {
        println!("  #{} {} {}", t.id, t.when, crate::history::label(t));
    }
    if ids.len() > 15 {
        println!("  ... and {} more", ids.len() - 15);
    }
    if !confirm(
        &format!(
            "The model proposes deleting {} turn(s). Delete? This cannot be undone.",
            ids.len()
        ),
        yes,
    )? {
        println!("aborted");
        return Ok(());
    }
    println!("deleted {}", store.delete(&ids)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_offered_ids_are_picked() {
        let turn = |id| Turn {
            id,
            when: String::new(),
            provider: String::new(),
            model: String::new(),
            mode: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            content: Default::default(),
        };
        let offered = [turn(1), turn(2), turn(3)];
        assert_eq!(picked("```json\n[3, 1, 99]\n```", &offered), [1, 3]);
        assert!(picked("no idea", &offered).is_empty());
        assert!(picked("[\"x\"]", &offered).is_empty());
    }
}
