//! End-to-end tests: the real binary against a local server that speaks both wire formats.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Step {
    Text(&'static str),
    /// text, ended by the output limit (OpenAI-style `length`)
    Cut(&'static str),
    /// tool name, arguments as JSON text
    Call(&'static str, &'static str),
}

fn tool_events(anthropic: bool, name: &str, args: &str, n: usize) -> String {
    let (a, b) = args.split_at(args.len() / 2);
    let q = |s: &str| serde_json::to_string(s).unwrap();
    if anthropic {
        format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"model\":\"served-model\",\"usage\":{{\"input_tokens\":11}}}}}}\n\n\
             event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"call_{n}\",\"name\":\"{name}\",\"input\":{{}}}}}}\n\n\
             event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{}}}}}\n\n\
             event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{}}}}}\n\n\
             event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"tool_use\"}},\"usage\":{{\"output_tokens\":7}}}}\n\n",
            q(a),
            q(b)
        )
    } else {
        format!(
            "data: {{\"model\":\"served-model\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_{n}\",\"type\":\"function\",\"function\":{{\"name\":\"{name}\",\"arguments\":{}}}}}]}}}}]}}\n\n\
             data: {{\"model\":\"served-model\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"function\":{{\"arguments\":{}}}}}]}},\"finish_reason\":\"tool_calls\"}}]}}\n\n\
             data: {{\"model\":\"served-model\",\"choices\":[],\"usage\":{{\"prompt_tokens\":5,\"completion_tokens\":3}}}}\n\n\
             data: [DONE]\n\n",
            q(a),
            q(b)
        )
    }
}

struct Mock {
    port: u16,
    hits: Arc<AtomicUsize>,
    /// (request head, lower-cased; body) per request
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl Mock {
    fn start(text: &'static str, status: u16) -> Mock {
        Mock::start_with(text, status, Vec::new())
    }

    /// The nth request is answered by the nth step (the last repeats).
    fn start_with(text: &'static str, status: u16, script: Vec<Step>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (h, s) = (hits.clone(), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head, body) = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let head = text[..i].to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .map(|v| v.trim().parse().unwrap())
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break (
                                head,
                                String::from_utf8_lossy(&buf[i + 4..i + 4 + len]).to_string(),
                            );
                        }
                    }
                    assert!(n > 0, "request ended early");
                };
                let n = h.fetch_add(1, Ordering::SeqCst);
                let step = script.get(n).or(script.last()).cloned();
                let anthropic = head.starts_with("post /v1/messages");
                s.lock().unwrap().push((head, body));
                let out = if status != 200 {
                    format!(
                        "HTTP/1.1 {status} Nope\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{{\"type\":\"error\",\"error\":{{\"type\":\"authentication_error\",\"message\":\"bad key\"}}}}"
                    )
                } else {
                    let shown = match &step {
                        Some(Step::Text(t)) | Some(Step::Cut(t)) => *t,
                        _ => text,
                    };
                    let (a, b) = shown.split_at(shown.len() / 2);
                    let q = |s: &str| serde_json::to_string(s).unwrap();
                    let events = if let Some(Step::Call(name, args)) = &step {
                        tool_events(anthropic, name, args, n)
                    } else if anthropic {
                        format!(
                            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"model\":\"served-model\",\"usage\":{{\"input_tokens\":11}}}}}}\n\n\
                             event: ping\ndata: {{\"type\":\"ping\"}}\n\n\
                             event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{}}}}}\n\n\
                             event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{}}}}}\n\n\
                             event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":7}}}}\n\n\
                             event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
                            q(a),
                            q(b)
                        )
                    } else {
                        format!(
                            "data: {{\"model\":\"served-model\",\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\n\
                             data: {{\"model\":\"served-model\",\"choices\":[{{\"delta\":{{\"content\":{}}},\"finish_reason\":\"{}\"}}]}}\n\n\
                             data: {{\"model\":\"served-model\",\"choices\":[],\"usage\":{{\"prompt_tokens\":5,\"completion_tokens\":3}}}}\n\n\
                             data: [DONE]\n\n",
                            q(a),
                            q(b),
                            if matches!(step, Some(Step::Cut(_))) {
                                "length"
                            } else {
                                "stop"
                            }
                        )
                    };
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{events}"
                    )
                };
                let _ = stream.write_all(out.as_bytes());
            }
        });
        Mock { port, hits, seen }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn nth(&self, i: usize) -> serde_json::Value {
        serde_json::from_str(&self.seen.lock().unwrap()[i].1).unwrap()
    }

    fn last(&self) -> (String, serde_json::Value) {
        let seen = self.seen.lock().unwrap();
        let (head, body) = seen.last().unwrap();
        (head.clone(), serde_json::from_str(body).unwrap())
    }
}

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(name: &str, mock: &Mock) -> Env {
        let dir = std::env::temp_dir().join(format!("prinia-it-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::write(
            dir.join("cfg/config.toml"),
            format!(
                "[providers.anthropic]\nbase_url = \"http://127.0.0.1:{p}\"\n\n[providers.local]\ntype = \"openai\"\nbase_url = \"http://127.0.0.1:{p}/v1\"\nmodel = \"local-model\"\n",
                p = mock.port
            ),
        )
        .unwrap();
        Env { dir }
    }

    fn db(&self) -> PathBuf {
        self.dir.join("history.db")
    }

    /// (stdout, stderr, exit code)
    fn run(&self, args: &[&str], stdin: &str, extra_env: &[(&str, &str)]) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_prinia"));
        cmd.args(args)
            .current_dir(self.dir.join("work"))
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.dir)
            .env("PRINIA_CONFIG", self.dir.join("cfg/config.toml"))
            .env("PRINIA_DB", self.db())
            .env("ANTHROPIC_API_KEY", "test-key")
            .envs(extra_env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into(),
            String::from_utf8_lossy(&out.stderr).into(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn anthropic_pipe_cache_and_encryption() {
    let mock = Mock::start("Hello world", 200);
    let env = Env::new("anthropic", &mock);

    let (out, err, code) = env.run(&["extract the thing"], "zebra-quartz-77", &[]);
    assert_eq!((out.as_str(), code), ("Hello world\n", 0), "stderr: {err}");
    let (head, body) = mock.last();
    assert!(
        head.contains("x-api-key: test-key") && head.contains("anthropic-version: 2026-10-21"),
        "{head}"
    );
    assert_eq!(body["model"], "claude-sonnet-5-5");
    assert_eq!(body["stream"], true);
    assert_eq!(body["output_config"]["effort"], "medium");
    assert_eq!(
        body["messages"][0]["content"],
        "extract the thing\n\nzebra-quartz-77"
    );
    assert!(body["system"].as_str().unwrap().contains("pipeline"));
    assert!(
        body.get("fallbacks").is_none(),
        "fallbacks are only for the official endpoint"
    );

    // content is encrypted at rest
    let raw = String::from_utf8_lossy(&std::fs::read(env.db()).unwrap()).to_string();
    assert!(
        !raw.contains("zebra-quartz-77")
            && !raw.contains("Hello world")
            && !raw.contains("extract the thing")
    );

    // an identical call is answered from history
    let (out, _, _) = env.run(&["extract the thing"], "zebra-quartz-77", &[]);
    assert_eq!((out.as_str(), mock.hits()), ("Hello world\n", 1));
    env.run(&["--no-cache", "extract the thing"], "zebra-quartz-77", &[]);
    assert_eq!(mock.hits(), 2);
    // another tier is another call
    env.run(&["-f", "extract the thing"], "zebra-quartz-77", &[]);
    assert_eq!(
        (
            mock.hits(),
            mock.last().1["model"].as_str().unwrap().to_string()
        ),
        (3, "claude-haiku-5-5".to_string())
    );
    assert_eq!(mock.last().1["output_config"]["effort"], "low");

    let (list, _, _) = env.run(&["history"], "", &[]);
    assert_eq!(list.lines().count(), 3, "{list}");
    let (shown, _, _) = env.run(&["history", "show", "1"], "", &[]);
    assert!(
        shown.contains("zebra-quartz-77")
            && shown.contains("Hello world")
            && shown.contains("11 in / 7 out")
            && shown.contains("served-model"),
        "{shown}"
    );

    let (_, _, code) = env.run(&["history", "rm", "1", "--yes"], "", &[]);
    assert_eq!(code, 0);
    assert_eq!(env.run(&["history"], "", &[]).0.lines().count(), 2);
}

#[test]
fn openai_compatible_provider() {
    let mock = Mock::start("local answer", 200);
    let env = Env::new("openai", &mock);
    let (out, err, code) = env.run(&["-p", "local", "question"], "", &[]);
    assert_eq!((out.as_str(), code), ("local answer\n", 0), "stderr: {err}");
    let (head, body) = mock.last();
    assert!(
        head.starts_with("post /v1/chat/completions") && !head.contains("authorization:"),
        "{head}"
    );
    assert_eq!(body["model"], "local-model");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "question");
    assert!(body.get("output_config").is_none() && body.get("max_tokens").is_none());
    let (shown, _, _) = env.run(&["history", "show", "1"], "", &[]);
    assert!(
        shown.contains("5 in / 3 out") && shown.contains("local served-model"),
        "{shown}"
    );
}

#[test]
fn piped_output_loses_its_code_fence() {
    let mock = Mock::start("```json\n{\"a\":1}\n```", 200);
    let env = Env::new("fence", &mock);
    assert_eq!(env.run(&["give json"], "", &[]).0, "{\"a\":1}\n");
}

#[test]
fn provider_errors_are_reported() {
    let mock = Mock::start("", 401);
    let env = Env::new("error", &mock);
    let (out, err, code) = env.run(&["hi"], "", &[]);
    assert_eq!((out.as_str(), code, mock.hits()), ("", 1, 1));
    assert!(err.contains("HTTP 401") && err.contains("bad key"), "{err}");
    assert!(
        !env.db().exists() || env.run(&["history"], "", &[]).0.is_empty(),
        "a failed call is not recorded"
    );
}

#[test]
fn no_record_leaves_nothing_behind() {
    let mock = Mock::start("ok", 200);
    let env = Env::new("norecord", &mock);
    let (out, _, code) = env.run(&["--no-record", "hi"], "", &[("PRINIA_CACHE", "off")]);
    assert_eq!((out.as_str(), code), ("ok\n", 0));
    assert!(!env.db().exists() && !env.dir.join("cfg/key").exists());
}

#[test]
fn key_backup_rotation_and_import() {
    let mock = Mock::start("secret answer", 200);
    let env = Env::new("key", &mock);
    env.run(&["remember"], "", &[]);
    let (words, _, code) = env.run(&["key", "show"], "", &[]);
    assert_eq!((words.split_whitespace().count(), code), (24, 0));

    // rotation keeps the history readable under a new key
    assert_eq!(env.run(&["key", "rotate"], "", &[]).2, 0);
    let (new_words, _, _) = env.run(&["key", "show"], "", &[]);
    assert_ne!(words, new_words);
    assert!(
        env.run(&["history", "show", "1"], "", &[])
            .0
            .contains("secret answer")
    );

    // a lost key is an error, never a silent restart
    std::fs::remove_file(env.dir.join("cfg/key")).unwrap();
    let (_, err, code) = env.run(&["history"], "", &[]);
    assert!(code == 1 && err.contains("key is missing"), "{err}");
    // the old key no longer fits; the new one restores access
    let (_, err, code) = env.run(&["key", "import"], &words, &[]);
    assert!(code == 1 && err.contains("does not unlock"), "{err}");
    assert_eq!(env.run(&["key", "import"], &new_words, &[]).2, 0);
    assert!(
        env.run(&["history", "show", "1"], "", &[])
            .0
            .contains("secret answer")
    );
}

#[test]
fn context_comes_from_history_of_this_directory() {
    let mock = Mock::start("answer", 200);
    let env = Env::new("context", &mock);
    let args = |extra: &'static str, q: &'static str| {
        let mut v = vec!["-p", "local"];
        if !extra.is_empty() {
            v.push(extra);
        }
        v.push(q);
        v
    };

    env.run(&args("", "first question"), "", &[]);
    // off by default: no earlier turns are sent
    env.run(&args("", "second question"), "", &[]);
    assert_eq!(mock.last().1["messages"].as_array().unwrap().len(), 2);

    // -c sends the earlier turns, oldest first, between the system prompt and the new message
    env.run(&args("-c", "third question"), "", &[]);
    let m = mock.last().1["messages"].clone();
    let roles: Vec<_> = m
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["system", "user", "assistant", "user", "assistant", "user"]
    );
    assert_eq!(m[1]["content"], "first question");
    assert_eq!(m[2]["content"], "answer");
    assert_eq!(m[5]["content"], "third question");

    // the cache key covers the context: without it, or with a longer history, it is a new call
    let hits = mock.hits();
    env.run(&args("", "third question"), "", &[]);
    assert_eq!(mock.hits(), hits + 1);
    env.run(&args("", "third question"), "", &[]);
    assert_eq!(mock.hits(), hits + 1, "same call, same context: cached");
    env.run(&args("-c", "third question"), "", &[]);
    assert_eq!(
        mock.hits(),
        hits + 2,
        "the history grew, so the context differs"
    );

    // the `context` setting sets how many turns -c sends, and turns it on by itself
    env.run(
        &args("-c", "fourth question"),
        "",
        &[("PRINIA_CONTEXT", "1")],
    );
    assert_eq!(mock.last().1["messages"].as_array().unwrap().len(), 4);
    env.run(&args("", "fifth question"), "", &[("PRINIA_CONTEXT", "1")]);
    assert_eq!(mock.last().1["messages"].as_array().unwrap().len(), 4);
    env.run(
        &args("--no-context", "sixth"),
        "",
        &[("PRINIA_CONTEXT", "1")],
    );
    assert_eq!(mock.last().1["messages"].as_array().unwrap().len(), 2);

    // another directory has its own history; anthropic carries history in `messages` too
    std::fs::create_dir_all(env.dir.join("work/other")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_prinia"))
        .args(["-c", "seventh question"])
        .current_dir(env.dir.join("work/other"))
        .env_clear()
        .env("HOME", &env.dir)
        .env("PRINIA_CONFIG", env.dir.join("cfg/config.toml"))
        .env("PRINIA_DB", env.db())
        .env("ANTHROPIC_API_KEY", "test-key")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    let body = mock.last().1;
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["content"], "seventh question");
}

#[test]
fn search_finds_words_across_prompt_input_and_answer() {
    let mock = Mock::start("the answer", 200);
    let env = Env::new("search", &mock);
    env.run(&["tell me about herons"], "", &[]);
    env.run(&["summarise"], "kestrel nests in barns\nsecond line", &[]);
    let (out, _, _) = env.run(&["history", "search", "KESTREL"], "", &[]);
    assert!(
        out.contains("summarise") && out.contains("kestrel nests in barns"),
        "{out}"
    );
    assert!(!out.contains("herons"));
    let (out, _, _) = env.run(&["history", "search", "answer", "heron"], "", &[]);
    assert!(
        out.contains("herons") && !out.contains("summarise"),
        "{out}"
    );
    let (out, _, _) = env.run(&["history", "search", "nothing-like-this"], "", &[]);
    assert_eq!(out.trim(), "nothing matches");
}

#[test]
fn history_stays_within_its_size_limit() {
    let mock = Mock::start("ok", 200);
    let env = Env::new("limits", &mock);
    // varied text so it does not compress to nothing
    let blob = |seed: u64| -> String {
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (0..6000)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                char::from(b'a' + (x >> 59) as u8)
            })
            .collect()
    };
    for i in 1..=5 {
        env.run(&[&format!("q{i}")], &blob(i), &[("PRINIA_MAX_SIZE", "8k")]);
    }
    let (list, _, _) = env.run(&["history"], "", &[]);
    let n = list.lines().count();
    assert!((1..5).contains(&n), "{list}");
    assert!(list.contains("q5"), "the newest turn is kept:\n{list}");
    assert!(
        !list.contains("q1\n") && !list.lines().any(|l| l.ends_with("q1")),
        "{list}"
    );

    // `history clean` applies the limits to an existing history
    let (out, _, code) = env.run(
        &["history", "clean", "--yes"],
        "",
        &[("PRINIA_MAX_SIZE", "1k")],
    );
    assert_eq!(code, 0, "{out}");
    let (list, _, _) = env.run(&["history"], "", &[]);
    assert!(list.lines().count() < n, "{list}");
}

#[test]
fn history_agent_deletes_only_what_the_model_names_and_only_when_confirmed() {
    let mock = Mock::start("[1]", 200);
    let env = Env::new("agent", &mock);
    env.run(&["-p", "local", "first"], "", &[]);
    env.run(&["-p", "local", "second"], "", &[]);

    // without a terminal, confirmation must be explicit
    let (_, err, code) = env.run(
        &["history", "agent", "drop the first"],
        "",
        &[("PRINIA_PROVIDER", "local")],
    );
    assert!(code != 0 && err.contains("--yes"), "{err}");
    assert_eq!(env.run(&["history"], "", &[]).0.lines().count(), 2);
    let (out, err, code) = env.run(
        &["history", "agent", "--yes", "drop the first"],
        "",
        &[("PRINIA_PROVIDER", "local")],
    );
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("deleted 1"), "{out}");
    let (list, _, _) = env.run(&["history"], "", &[]);
    assert!(list.contains("second") && !list.contains("first"), "{list}");
    // the request carried excerpts and the instruction, with ids
    let body = mock.last().1;
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(
        user.contains("Q: first") && user.contains("Instruction: drop the first"),
        "{user}"
    );
}

#[test]
fn history_ask_sends_excerpts_and_prints_the_answer() {
    let mock = Mock::start("see #1", 200);
    let env = Env::new("ask", &mock);
    env.run(&["-p", "local", "how do herons fish"], "", &[]);
    let (out, err, code) = env.run(
        &["history", "ask", "what did I ask about birds?"],
        "",
        &[("PRINIA_PROVIDER", "local")],
    );
    assert_eq!((out.as_str(), code), ("see #1\n", 0), "{err}");
    let user = mock.last().1["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        user.contains("how do herons fish") && user.contains("Question: what did I ask"),
        "{user}"
    );
    // the question itself is not added to the history
    let (list, _, _) = env.run(&["history"], "", &[]);
    assert_eq!(list.lines().count(), 1, "{list}");
}

#[test]
fn size_rules_pick_the_tier() {
    let mock = Mock::start("ok", 200);
    let env = Env::new("tier", &mock);
    let model = || mock.last().1["model"].as_str().unwrap().to_string();
    let rules = [("PRINIA_FAST_UNDER", "20"), ("PRINIA_DEEP_OVER", "60")];
    env.run(&["short"], "", &rules);
    assert_eq!(model(), "claude-haiku-5-5");
    env.run(&["a medium length question here"], "", &rules);
    assert_eq!(model(), "claude-sonnet-5-5");
    env.run(
        &["a very long question that goes well beyond the sixty characters"],
        "",
        &rules,
    );
    assert_eq!(model(), "claude-opus-5-5");
    // explicit choices win
    env.run(&["-d", "short"], "", &rules);
    assert_eq!(model(), "claude-opus-5-5");
    env.run(&["-m", "custom-x", "short"], "", &rules);
    assert_eq!(model(), "custom-x");
}

#[test]
fn stats_count_calls_hits_and_tokens_without_the_key() {
    let mock = Mock::start("ok", 200);
    let env = Env::new("stats", &mock);
    let (out, _, code) = env.run(&["stats", "--format", "{calls}"], "", &[]);
    assert_eq!((out.trim(), code), ("0", 0), "no history yet is all zeros");

    env.run(&["one"], "", &[]);
    env.run(&["one"], "", &[]); // answered from history
    let (out, _, _) = env.run(&["stats", "--json"], "", &[]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (
            v["calls"].as_u64(),
            v["cache_hits"].as_u64(),
            v["hit_rate"].as_u64()
        ),
        (Some(2), Some(1), Some(50)),
        "{out}"
    );
    assert_eq!(
        (v["input_tokens"].as_u64(), v["output_tokens"].as_u64()),
        (Some(11), Some(7))
    );

    let (out, _, _) = env.run(
        &[
            "stats",
            "--here",
            "--since",
            "0d",
            "--format",
            "{calls} calls, {hit_rate}%, {tokens} tok",
        ],
        "",
        &[],
    );
    assert_eq!(out.trim(), "2 calls, 50%, 18 tok");
    // another directory has no calls
    std::fs::create_dir_all(env.dir.join("work/elsewhere")).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_prinia"))
        .args(["stats", "--here", "--json"])
        .current_dir(env.dir.join("work/elsewhere"))
        .env_clear()
        .env("HOME", &env.dir)
        .env("PRINIA_CONFIG", env.dir.join("cfg/config.toml"))
        .env("PRINIA_DB", env.db())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"calls\":0"));

    // the key is not needed: remove it and stats still work
    let key = env.dir.join("cfg/key");
    std::fs::remove_file(&key).expect("the key file is next to the config");
    let (out, _, code) = env.run(&["stats", "--format", "{calls}"], "", &[]);
    assert_eq!((out.trim(), code), ("2", 0));
}

fn tool_names(body: &serde_json::Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .or(t["function"]["name"].as_str())
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn agent_reads_files_with_tool_calls_on_both_wire_formats() {
    for (anthropic, extra) in [(true, vec![]), (false, vec!["-p", "local"])] {
        let mock = Mock::start_with(
            "",
            200,
            vec![
                Step::Call("read_file", r#"{"path":"notes.txt"}"#),
                Step::Text("It says: heron."),
            ],
        );
        let env = Env::new(if anthropic { "agent-a" } else { "agent-o" }, &mock);
        std::fs::write(env.dir.join("work/notes.txt"), "heron nests\n").unwrap();
        let mut args = extra.clone();
        args.extend(["-a", "what does notes.txt say"]);
        let (out, err, code) = env.run(&args, "", &[]);
        assert_eq!((out.as_str(), code), ("It says: heron.\n", 0), "{err}");
        assert!(
            err.contains("read_file notes.txt"),
            "progress on stderr: {err}"
        );
        assert_eq!(mock.hits(), 2);
        assert_eq!(
            tool_names(&mock.nth(0)),
            ["read_file", "list_dir", "search"]
        );

        let m = mock.nth(1)["messages"].clone();
        let m = m.as_array().unwrap();
        if anthropic {
            assert_eq!(m[1]["content"][0]["type"], "tool_use");
            assert_eq!(m[1]["content"][0]["input"]["path"], "notes.txt");
            assert_eq!(m[2]["content"][0]["type"], "tool_result");
            assert_eq!(m[2]["content"][0]["tool_use_id"], "call_0");
            assert!(
                m[2]["content"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("1\theron nests")
            );
        } else {
            // system, user, assistant (tool call), tool
            let f = &m[2]["tool_calls"][0]["function"];
            assert_eq!(f["name"], "read_file");
            assert_eq!(f["arguments"], r#"{"path":"notes.txt"}"#);
            assert_eq!(m[3]["role"], "tool");
            assert_eq!(m[3]["tool_call_id"], "call_0");
            assert!(m[3]["content"].as_str().unwrap().contains("1\theron nests"));
        }
        // the session is one recorded turn, in agent mode
        let (list, _, _) = env.run(&["history"], "", &[]);
        assert_eq!(list.lines().count(), 1, "{list}");
    }
}

fn result_of_first_call(mock: &Mock) -> String {
    let m = mock.nth(1)["messages"].clone();
    m[2]["content"][0]["content"].as_str().unwrap().to_string()
}

#[test]
fn agent_changes_files_and_runs_commands_only_when_allowed() {
    let script = |name, args| vec![Step::Call(name, args), Step::Text("done")];
    let write = r#"{"path":"out.txt","content":"hi"}"#;

    let mock = Mock::start_with("", 200, script("write_file", write));
    let env = Env::new("agent-write-off", &mock);
    env.run(&["-a", "write it"], "", &[]);
    assert!(!env.dir.join("work/out.txt").exists());
    assert!(result_of_first_call(&mock).contains("not available"));

    let mock = Mock::start_with("", 200, script("write_file", write));
    let env = Env::new("agent-write-on", &mock);
    env.run(&["-a", "--write", "write it"], "", &[]);
    assert_eq!(
        std::fs::read_to_string(env.dir.join("work/out.txt")).unwrap(),
        "hi"
    );
    assert_eq!(
        tool_names(&mock.nth(0)),
        ["read_file", "list_dir", "search", "write_file", "replace"]
    );

    let mock = Mock::start_with(
        "",
        200,
        script("replace", r#"{"path":"out.txt","old":"hi","new":"bye"}"#),
    );
    let env2 = Env::new("agent-replace", &mock);
    std::fs::write(env2.dir.join("work/out.txt"), "hi there hi").unwrap();
    env2.run(&["-a", "--write", "edit"], "", &[]);
    assert!(result_of_first_call(&mock).contains("appears 2 times"));
    assert_eq!(
        std::fs::read_to_string(env2.dir.join("work/out.txt")).unwrap(),
        "hi there hi"
    );

    let run = r#"{"command":"echo hi-from-run"}"#;
    let mock = Mock::start_with("", 200, script("run_command", run));
    let env = Env::new("agent-run-off", &mock);
    env.run(&["-a", "go"], "", &[]);
    assert!(result_of_first_call(&mock).contains("not available"));
    let mock = Mock::start_with("", 200, script("run_command", run));
    let env = Env::new("agent-run-on", &mock);
    env.run(&["-a", "--run", "go"], "", &[]);
    let r = result_of_first_call(&mock);
    assert!(r.contains("hi-from-run") && r.contains("[exit 0]"), "{r}");

    // --write and --run only make sense with --agent
    let (_, err, code) = env.run(&["--write", "x"], "", &[]);
    assert!(code != 0 && err.contains("--agent"), "{err}");
}

#[test]
fn agent_stays_inside_the_working_directory_and_stops_after_its_step_limit() {
    let mock = Mock::start_with(
        "",
        200,
        vec![
            Step::Call("read_file", r#"{"path":"../cfg/config.toml"}"#),
            Step::Text("x"),
        ],
    );
    let env = Env::new("agent-escape", &mock);
    env.run(&["-a", "peek"], "", &[]);
    assert!(result_of_first_call(&mock).contains("outside the working directory"));

    let mock = Mock::start_with("", 200, vec![Step::Call("list_dir", "{}")]);
    let env = Env::new("agent-loop", &mock);
    let (_, err, code) = env.run(&["-a", "loop"], "", &[("PRINIA_AGENT_STEPS", "3")]);
    assert!(code != 0 && err.contains("stopped after 3 steps"), "{err}");
    assert_eq!(mock.hits(), 3);
}

#[test]
fn json_and_check_retry_bad_answers_and_never_cache_them() {
    // the first answer is not JSON, the second is
    let mock = Mock::start_with(
        "",
        200,
        vec![Step::Text("sure! here: [1"), Step::Text("[1, 2]")],
    );
    let env = Env::new("json", &mock);
    let (out, err, code) = env.run(&["--json", "numbers"], "", &[]);
    assert_eq!(
        (out.as_str(), code, mock.hits()),
        ("[1, 2]\n", 0, 2),
        "{err}"
    );
    assert!(err.contains("failed the check"), "{err}");
    // the good answer is what is cached
    let (out, _, _) = env.run(&["--json", "numbers"], "", &[]);
    assert_eq!((out.as_str(), mock.hits()), ("[1, 2]\n", 2));
    // the JSON requirement is part of the request
    assert!(
        mock.nth(0)["system"]
            .as_str()
            .unwrap()
            .contains("valid JSON")
    );

    // a stored answer that fails the check is skipped, and the new one replaces it
    if cfg!(unix) {
        let mock = Mock::start_with(
            "",
            200,
            vec![Step::Text("bad answer"), Step::Text("good answer")],
        );
        let env = Env::new("heal", &mock);
        env.run(&["q"], "", &[]); // cached without any check
        let (out, _, _) = env.run(&["--check", "grep -q good", "q"], "", &[]);
        assert_eq!((out.as_str(), mock.hits()), ("good answer\n", 2));
        let (out, _, _) = env.run(&["--check", "grep -q good", "q"], "", &[]);
        assert_eq!(
            (out.as_str(), mock.hits()),
            ("good answer\n", 2),
            "the fresh answer is now cached"
        );
    }

    // when every attempt fails: no output, an error, nothing saved
    let mock = Mock::start("never json", 200);
    let env = Env::new("json-fail", &mock);
    let (out, err, code) = env.run(&["--json", "q"], "", &[("PRINIA_CHECK_RETRIES", "1")]);
    assert!(
        out.is_empty() && code != 0 && err.contains("failed the check 2 time"),
        "{out}{err}"
    );
    assert_eq!(mock.hits(), 2);
    assert!(env.run(&["history"], "", &[]).0.trim().is_empty());

    // --check runs a command on the answer
    if cfg!(unix) {
        let mock = Mock::start_with("", 200, vec![Step::Text("nope"), Step::Text("all ok")]);
        let env = Env::new("check", &mock);
        let (out, _, code) = env.run(&["--check", "grep -q ok", "q"], "", &[]);
        assert_eq!((out.as_str(), code, mock.hits()), ("all ok\n", 0, 2));
    }
}

#[test]
fn answers_cut_off_by_the_limit_are_not_served_from_the_cache() {
    let mock = Mock::start_with("", 200, vec![Step::Cut("partial"), Step::Text("complete")]);
    let env = Env::new("cut", &mock);
    let (out, err, _) = env.run(&["-p", "local", "q"], "", &[]);
    assert_eq!(out, "partial\n");
    assert!(err.contains("cut off"), "{err}");
    let (out, _, _) = env.run(&["-p", "local", "q"], "", &[]);
    assert_eq!(
        (out.as_str(), mock.hits()),
        ("complete\n", 2),
        "the cut-off answer was not cached"
    );
    // it is still in the history
    assert_eq!(env.run(&["history"], "", &[]).0.lines().count(), 2);
}

#[test]
fn a_server_that_is_not_running_gets_a_hint() {
    let mock = Mock::start("", 200);
    let env = Env::new("down", &mock);
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    std::fs::write(
        env.dir.join("cfg/config.toml"),
        format!("[providers.down]\ntype = \"openai\"\nbase_url = \"http://127.0.0.1:{port}/v1\"\nmodel = \"m\"\n"),
    )
    .unwrap();
    let (out, err, code) = env.run(&["-p", "down", "hi"], "", &[]);
    assert!(out.is_empty() && code == 1, "{out}{err}");
    assert!(
        err.contains("cannot reach") && err.contains("Is the server running?"),
        "{err}"
    );
}
