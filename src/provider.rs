//! Model providers. Two wire formats cover nearly everything: Anthropic's Messages API and
//! OpenAI-compatible chat completions (OpenAI, Ollama and other local runners, most hosts).
use crate::config::Config;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Anthropic,
    OpenAi,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tier {
    Fast,
    Regular,
    Deep,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Fast => "fast",
            Tier::Regular => "regular",
            Tier::Deep => "deep",
        }
    }
    fn model_key(self) -> &'static str {
        match self {
            Tier::Fast => "fast_model",
            Tier::Regular => "model",
            Tier::Deep => "deep_model",
        }
    }
    fn effort_key(self) -> &'static str {
        match self {
            Tier::Fast => "fast_effort",
            Tier::Regular => "effort",
            Tier::Deep => "deep_effort",
        }
    }
}

pub struct Provider {
    pub name: String,
    pub kind: Kind,
    pub base_url: String,
    api_key: Option<String>,
}

pub struct Request<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub user: &'a str,
    pub effort: Option<&'a str>,
    pub max_tokens: Option<u64>,
}

#[derive(Default)]
pub struct Reply {
    pub text: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub stop: String,
}

const ANTHROPIC_URL: &str = "https://api.anthropic.com";
const OPENAI_URL: &str = "https://api.openai.com/v1";

impl Provider {
    /// `--provider`, then the `provider` setting, then whichever built-in has its API key set.
    pub fn resolve(cfg: &Config, flag: Option<&str>) -> Result<Provider> {
        let has = |v: &str| std::env::var(v).is_ok_and(|k| !k.is_empty());
        let name = match flag.map(str::to_owned).or_else(|| cfg.str("provider")) {
            Some(n) => n,
            None if has("ANTHROPIC_API_KEY") => "anthropic".into(),
            None if has("OPENAI_API_KEY") => "openai".into(),
            None => bail!(
                "no provider configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY, or add one to the config, e.g. a local model:\n\n  provider = \"local\"\n  [providers.local]\n  type = \"openai\"\n  base_url = \"http://localhost:11434/v1\"\n  model = \"llama3.2\""
            ),
        };
        let field = |f: &str| cfg.provider_str(&name, f);
        let kind = match field("type").as_deref().or(match name.as_str() {
            "anthropic" => Some("anthropic"),
            "openai" => Some("openai"),
            _ => None,
        }) {
            Some("anthropic") => Kind::Anthropic,
            Some("openai") => Kind::OpenAi,
            Some(other) => {
                bail!("provider `{name}`: unknown type `{other}` (use \"anthropic\" or \"openai\")")
            }
            None => bail!(
                "unknown provider `{name}`; define [providers.{name}] with a type (configured: {:?})",
                cfg.provider_names()
            ),
        };
        let base_url = field("base_url").unwrap_or_else(|| {
            if kind == Kind::Anthropic {
                ANTHROPIC_URL
            } else {
                OPENAI_URL
            }
            .into()
        });
        let key_env = field("api_key_env").unwrap_or_else(|| {
            if kind == Kind::Anthropic {
                "ANTHROPIC_API_KEY"
            } else {
                "OPENAI_API_KEY"
            }
            .into()
        });
        let api_key = std::env::var(&key_env).ok().filter(|k| !k.is_empty());
        let official = base_url == ANTHROPIC_URL || base_url == OPENAI_URL;
        if api_key.is_none() && official {
            bail!("provider `{name}` needs an API key in ${key_env}");
        }
        Ok(Provider {
            name,
            kind,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    /// The provider's own table wins over the top-level setting, then the built-in default.
    pub fn model(&self, cfg: &Config, tier: Tier) -> Result<String> {
        let builtin = match (self.kind, tier) {
            (Kind::Anthropic, Tier::Fast) => Some("claude-haiku-5-5"),
            (Kind::Anthropic, Tier::Regular) => Some("claude-sonnet-5-5"),
            (Kind::Anthropic, Tier::Deep) => Some("claude-opus-5-5"),
            (Kind::OpenAi, _) => None,
        };
        let pick = |t: Tier| {
            cfg.provider_str(&self.name, t.model_key())
                .or_else(|| cfg.str(t.model_key()))
        };
        pick(tier)
            // a provider with one configured model uses it for every tier
            .or_else(|| if builtin.is_none() { pick(Tier::Regular) } else { None })
            .or_else(|| builtin.map(str::to_owned))
            .ok_or_else(|| anyhow!("no model set for provider `{}`; add `model = \"...\"` under [providers.{}] or pass -m", self.name, self.name))
    }

    /// Reasoning effort is an Anthropic request field; other providers get none.
    pub fn effort(&self, cfg: &Config, tier: Tier) -> Option<String> {
        if self.kind != Kind::Anthropic {
            return None;
        }
        let default = match tier {
            Tier::Fast => "low",
            Tier::Regular => "medium",
            Tier::Deep => "high",
        };
        Some(cfg.str(tier.effort_key()).unwrap_or_else(|| default.into()))
            .filter(|e| !e.is_empty() && e != "none")
    }

    /// What identifies this endpoint in a cache key.
    pub fn identity(&self) -> String {
        format!("{:?} {}", self.kind, self.base_url)
    }

    /// Streams the answer, calling `on_text` for each piece as it arrives.
    pub fn complete(&self, req: &Request, on_text: &mut dyn FnMut(&str)) -> Result<Reply> {
        let (url, body) = match self.kind {
            Kind::Anthropic => (
                format!("{}/v1/messages", self.base_url),
                self.anthropic_body(req),
            ),
            Kind::OpenAi => (
                format!("{}/chat/completions", self.base_url),
                openai_body(req),
            ),
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .build()
            .into();

        // Rate limits and server errors are retried, but only before any output exists.
        let mut attempt = 0;
        let resp = loop {
            let mut r = agent.post(&url).header("content-type", "application/json");
            match self.kind {
                Kind::Anthropic => {
                    r = r.header("anthropic-version", "2026-10-21");
                    if let Some(k) = &self.api_key {
                        r = r.header("x-api-key", k);
                    }
                    if body.get("fallbacks").is_some() {
                        r = r.header("anthropic-beta", "server-side-fallback-2026-07-01");
                    }
                }
                Kind::OpenAi => {
                    if let Some(k) = &self.api_key {
                        r = r.header("authorization", &format!("Bearer {k}"));
                    }
                }
            }
            let mut resp = r
                .send_json(&body)
                .with_context(|| format!("cannot reach {url}"))?;
            let status = resp.status().as_u16();
            if status == 200 {
                break resp;
            }
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            let retryable = status == 429 || status >= 500;
            if retryable && attempt < 2 {
                attempt += 1;
                std::thread::sleep(Duration::from_secs(if attempt == 1 { 1 } else { 3 }));
                continue;
            }
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or(text);
            bail!("{} returned HTTP {status}: {}", self.name, msg.trim());
        };

        let mut reply = Reply {
            model: req.model.to_string(),
            ..Default::default()
        };
        for line in BufReader::new(resp.into_body().into_reader()).lines() {
            let line = line.context("the response stream broke off")?;
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let piece = match self.kind {
                Kind::Anthropic => anthropic_event(&v, &mut reply)?,
                Kind::OpenAi => openai_event(&v, &mut reply)?,
            };
            if let Some(t) = piece.filter(|t| !t.is_empty()) {
                on_text(&t);
                reply.text.push_str(&t);
            }
        }
        Ok(reply)
    }

    fn anthropic_body(&self, req: &Request) -> Value {
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens.unwrap_or(64_000),
            "stream": true,
            "system": req.system,
            "messages": [{"role": "user", "content": req.user}],
        });
        if let Some(e) = req.effort {
            body["output_config"] = json!({"effort": e});
        }
        // These models can decline a request; the API then re-runs it on a fallback model.
        let declines = [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "claude-fable-5-2",
        ]
        .iter()
        .any(|m| req.model.starts_with(m));
        if declines && self.base_url == ANTHROPIC_URL {
            body["fallbacks"] = json!("default");
        }
        body
    }
}

fn openai_body(req: &Request) -> Value {
    let mut body = json!({
        "model": req.model,
        "stream": true,
        "stream_options": {"include_usage": true},
        "messages": [{"role": "system", "content": req.system}, {"role": "user", "content": req.user}],
    });
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = json!(m);
    }
    body
}

fn anthropic_event(v: &Value, reply: &mut Reply) -> Result<Option<String>> {
    match v["type"].as_str() {
        Some("message_start") => {
            let m = &v["message"];
            if let Some(model) = m["model"].as_str() {
                reply.model = model.to_string();
            }
            let u = &m["usage"];
            reply.input_tokens = [
                "input_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ]
            .iter()
            .filter_map(|k| u[*k].as_u64())
            .sum();
        }
        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
            return Ok(v["delta"]["text"].as_str().map(str::to_owned));
        }
        Some("message_delta") => {
            if let Some(n) = v["usage"]["output_tokens"].as_u64() {
                reply.output_tokens = n;
            }
            if let Some(s) = v["delta"]["stop_reason"].as_str() {
                reply.stop = s.to_string();
            }
        }
        Some("error") => bail!(
            "the provider reported an error mid-stream: {}",
            v["error"]["message"].as_str().unwrap_or("unknown")
        ),
        _ => {}
    }
    Ok(None)
}

fn openai_event(v: &Value, reply: &mut Reply) -> Result<Option<String>> {
    if let Some(msg) = v["error"]["message"].as_str() {
        bail!("the provider reported an error mid-stream: {msg}");
    }
    if let Some(model) = v["model"].as_str().filter(|m| !m.is_empty()) {
        reply.model = model.to_string();
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        reply.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0);
        reply.output_tokens = u["completion_tokens"].as_u64().unwrap_or(0);
    }
    let choice = &v["choices"][0];
    if let Some(s) = choice["finish_reason"].as_str() {
        reply.stop = s.to_string();
    }
    Ok(choice["delta"]["content"].as_str().map(str::to_owned))
}
