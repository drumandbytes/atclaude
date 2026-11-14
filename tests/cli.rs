//! End-to-end tests: the real binary against a local server that speaks both wire formats.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Mock {
    port: u16,
    hits: Arc<AtomicUsize>,
    /// (request head, lower-cased; body) per request
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl Mock {
    fn start(text: &'static str, status: u16) -> Mock {
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
                h.fetch_add(1, Ordering::SeqCst);
                let anthropic = head.starts_with("post /v1/messages");
                s.lock().unwrap().push((head, body));
                let out = if status != 200 {
                    format!(
                        "HTTP/1.1 {status} Nope\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{{\"type\":\"error\",\"error\":{{\"type\":\"authentication_error\",\"message\":\"bad key\"}}}}"
                    )
                } else {
                    let (a, b) = text.split_at(text.len() / 2);
                    let q = |s: &str| serde_json::to_string(s).unwrap();
                    let events = if anthropic {
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
                             data: {{\"model\":\"served-model\",\"choices\":[{{\"delta\":{{\"content\":{}}},\"finish_reason\":\"stop\"}}]}}\n\n\
                             data: {{\"model\":\"served-model\",\"choices\":[],\"usage\":{{\"prompt_tokens\":5,\"completion_tokens\":3}}}}\n\n\
                             data: [DONE]\n\n",
                            q(a),
                            q(b)
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
        let dir = std::env::temp_dir().join(format!("atclaude-it-{name}-{}", std::process::id()));
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
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_atclaude"));
        cmd.args(args)
            .current_dir(self.dir.join("work"))
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", &self.dir)
            .env("ATCLAUDE_CONFIG", self.dir.join("cfg/config.toml"))
            .env("ATCLAUDE_DB", self.db())
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
    let (out, _, code) = env.run(&["--no-record", "hi"], "", &[("ATCLAUDE_CACHE", "off")]);
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
