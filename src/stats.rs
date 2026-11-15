use crate::history::age_secs;
use crate::store::{self, Stats};
use anyhow::Result;
use serde_json::json;

/// Usage totals. `since`: "0d" is today, "7d" the last seven days, none is all time.
pub fn show(here: bool, since: Option<&str>, json_out: bool, format: Option<&str>) -> Result<()> {
    let dir = here
        .then(|| std::env::current_dir().map(|d| store::dir_hash(&d)))
        .transpose()?;
    let days = since.map(|s| age_secs(s).map(|n| n / 86_400)).transpose()?;
    let s = store::read_stats(dir.as_deref(), days)?;
    let rate = (s.cache_hits * 100).checked_div(s.calls).unwrap_or(0);
    if json_out {
        println!(
            "{}",
            json!({
                "calls": s.calls,
                "cache_hits": s.cache_hits,
                "hit_rate": rate,
                "input_tokens": s.input_tokens,
                "output_tokens": s.output_tokens,
            })
        );
    } else if let Some(f) = format {
        println!("{}", render(f, &s, rate));
    } else {
        println!(
            "{} calls, {} answered from history ({rate}%), {} tokens in, {} out",
            s.calls, s.cache_hits, s.input_tokens, s.output_tokens
        );
    }
    Ok(())
}

fn render(format: &str, s: &Stats, rate: u64) -> String {
    [
        ("{calls}", s.calls),
        ("{cache_hits}", s.cache_hits),
        ("{hit_rate}", rate),
        ("{input_tokens}", s.input_tokens),
        ("{output_tokens}", s.output_tokens),
        ("{tokens}", s.input_tokens + s.output_tokens),
    ]
    .iter()
    .fold(format.to_string(), |f, (k, v)| f.replace(k, &v.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        let s = Stats {
            calls: 4,
            cache_hits: 1,
            input_tokens: 10,
            output_tokens: 5,
        };
        assert_eq!(render("{calls}/{hit_rate}%/{tokens}", &s, 25), "4/25%/15");
    }
}
