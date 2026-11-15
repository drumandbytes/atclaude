use crate::store::{Filter, Store};
use anyhow::Result;

const USER_CAP: usize = 2_000;
const REPLY_CAP: usize = 4_000;

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n[...cut]")
}

/// The newest `turns` turns from this directory as (user, assistant) messages, oldest first,
/// kept within `budget` characters (the oldest turns are dropped first).
pub fn build(
    store: &Store,
    dir: &str,
    turns: usize,
    budget: usize,
) -> Result<Vec<(String, String)>> {
    let filter = Filter {
        dir: Some(dir.to_string()),
        ..Filter::default()
    };
    let mut out = Vec::new();
    let mut used = 0;
    for t in store.list(&filter, turns)? {
        let c = t.content;
        let user = match (c.instruction.is_empty(), c.payload.is_empty()) {
            (false, false) => format!("{}\n\n{}", c.instruction, c.payload),
            (false, true) => c.instruction,
            _ => c.payload,
        };
        let pair = (cut(&user, USER_CAP), cut(c.response.trim_end(), REPLY_CAP));
        let size = pair.0.chars().count() + pair.1.chars().count();
        if used + size > budget {
            break;
        }
        used += size;
        out.push(pair);
    }
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_long_text() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abcdef", 3), "abc\n[...cut]");
    }
}
