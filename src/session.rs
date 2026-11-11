use crate::db::Database;
use crate::llm;
use anyhow::Result;

/// Cascade: explicit name (flag/env) > configured (`.atclauderc` files, global config) > terminal id > none.
/// `tty` is the per-shell id, passed only when stdin is a terminal so piped runs stay stateless.
/// "none" anywhere means stateless (the turn is still recorded, just without context).
pub fn resolve(explicit: Option<&str>, configured: Option<&str>, tty: Option<&str>) -> Option<String> {
    let name = explicit
        .filter(|s| !s.is_empty())
        .or(configured.filter(|s| !s.is_empty()))
        .map(str::to_owned)
        .or_else(|| tty.filter(|s| !s.is_empty()).map(|t| format!("tty-{t}")))?;
    (name != "none").then_some(name)
}

/// Recent turns of the session, verbatim when small, summarized by the helper model when over budget.
/// Piped payloads are left out; only prompts and responses carry over.
pub fn context(db: &Database, session: &str, turns: usize, budget: usize, helper_model: &str) -> Result<Option<String>> {
    let turns = db.recent(session, turns)?;
    if turns.is_empty() {
        return Ok(None);
    }
    let raw = turns
        .iter()
        .map(|t| format!("User: {}\nAssistant: {}", t.prompt, t.response))
        .collect::<Vec<_>>()
        .join("\n---\n");
    if raw.chars().count() <= budget {
        return Ok(Some(raw));
    }
    let summary = llm::complete(
        Some(helper_model),
        Some("Summarize the prior conversation in at most 10 lines. Keep facts, names, paths, decisions and open questions. Output only the summary."),
        llm::HELPER_ARGS,
        &raw,
    );
    Ok(Some(summary.unwrap_or_else(|e| {
        eprintln!("[atclaude] context summary failed ({e}); using truncated history");
        let n = raw.chars().count();
        raw.chars().skip(n.saturating_sub(budget)).collect()
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade() {
        assert_eq!(resolve(None, None, Some("x")).as_deref(), Some("tty-x"));
        assert_eq!(resolve(None, None, None), None);
        assert_eq!(resolve(None, Some("proj"), Some("x")).as_deref(), Some("proj"));
        assert_eq!(resolve(Some("g"), Some("proj"), Some("x")).as_deref(), Some("g"));
        assert_eq!(resolve(Some("none"), Some("proj"), Some("x")), None);
        assert_eq!(resolve(None, Some("none"), Some("x")), None);
    }
}
