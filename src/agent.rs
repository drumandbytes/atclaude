//! Agent mode: the model may call tools to read and search files in the working directory.
//! Changing files and running commands are off unless asked for (`--write`, `--run`).
use crate::config::Config;
use crate::provider::{Call, Msg, Provider, Reply, Request, Tool};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Opts<'a> {
    pub cfg: &'a Config,
    pub provider: &'a Provider,
    pub model: &'a str,
    pub effort: Option<&'a str>,
    pub max_tokens: Option<u64>,
    pub user: &'a str,
    pub write: bool,
    pub run: bool,
}

const OUTPUT_CAP: usize = 20_000;

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    Tool {
        name,
        description,
        schema,
    }
}

fn tools(write: bool, run: bool) -> Vec<Tool> {
    let path = json!({"type": "string", "description": "Relative to the working directory"});
    let mut t = vec![
        tool(
            "read_file",
            "Read a text file, with line numbers. Optional start and end line (1-based).",
            json!({"type": "object", "properties": {"path": path, "start": {"type": "integer"}, "end": {"type": "integer"}}, "required": ["path"]}),
        ),
        tool(
            "list_dir",
            "List a directory. Directories end with /.",
            json!({"type": "object", "properties": {"path": path}}),
        ),
        tool(
            "search",
            "Find lines containing a text (case-insensitive) in files under a path. Skips hidden and build directories.",
            json!({"type": "object", "properties": {"pattern": {"type": "string"}, "path": path}, "required": ["pattern"]}),
        ),
    ];
    if write {
        t.push(tool(
            "write_file",
            "Create or overwrite a file. The directory must exist.",
            json!({"type": "object", "properties": {"path": path, "content": {"type": "string"}}, "required": ["path", "content"]}),
        ));
        t.push(tool(
            "replace",
            "Replace one exact occurrence of `old` with `new` in a file. It is an error if `old` is missing or appears more than once.",
            json!({"type": "object", "properties": {"path": path, "old": {"type": "string"}, "new": {"type": "string"}}, "required": ["path", "old", "new"]}),
        ));
    }
    if run {
        t.push(tool(
            "run_command",
            "Run a shell command in the working directory and return its output and exit code (60 second limit).",
            json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
        ));
    }
    t
}

/// An existing path, or a new file in an existing directory, that stays inside `root`.
fn resolve(root: &Path, p: &str) -> Result<PathBuf> {
    let joined = root.join(p);
    let canon = match joined.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            let name = joined
                .file_name()
                .ok_or_else(|| anyhow!("bad path {p:?}"))?;
            joined
                .parent()
                .ok_or_else(|| anyhow!("bad path {p:?}"))?
                .canonicalize()
                .map_err(|_| anyhow!("{p:?} does not exist and neither does its directory"))?
                .join(name)
        }
    };
    if !canon.starts_with(root) {
        bail!("{p:?} is outside the working directory");
    }
    Ok(canon)
}

fn cap(mut s: String) -> String {
    if s.chars().count() > OUTPUT_CAP {
        s = s.chars().take(OUTPUT_CAP).collect();
        s.push_str("\n[...output cut]");
    }
    s
}

fn arg<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a[k].as_str()
        .ok_or_else(|| anyhow!("missing text argument `{k}`"))
}

fn read_file(root: &Path, a: &Value) -> Result<String> {
    let path = resolve(root, arg(a, "path")?)?;
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("cannot read as text: {e}"))?;
    let start = a["start"].as_u64().unwrap_or(1).max(1) as usize;
    let end = a["end"].as_u64().map(|e| e as usize).unwrap_or(usize::MAX);
    let out: Vec<String> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| (start..=end).contains(&(i + 1)))
        .map(|(i, l)| format!("{}\t{l}", i + 1))
        .collect();
    Ok(cap(out.join("\n")))
}

fn list_dir(root: &Path, a: &Value) -> Result<String> {
    let path = resolve(root, a["path"].as_str().unwrap_or("."))?;
    let mut names: Vec<String> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            format!(
                "{}{}",
                e.file_name().to_string_lossy(),
                if dir { "/" } else { "" }
            )
        })
        .collect();
    names.sort();
    names.truncate(200);
    Ok(names.join("\n"))
}

fn walk(dir: &Path, needle: &str, out: &mut Vec<String>, root: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= 100 {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if name.starts_with('.') || ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if name != "target" && name != "node_modules" {
                walk(&path, needle, out, root);
            }
        } else if e.metadata().map(|m| m.len() < 1_000_000).unwrap_or(false)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let rel = path.strip_prefix(root).unwrap_or(&path).display();
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(needle) {
                    out.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
                if out.len() >= 100 {
                    return;
                }
            }
        }
    }
}

fn search(root: &Path, a: &Value) -> Result<String> {
    let start = resolve(root, a["path"].as_str().unwrap_or("."))?;
    let needle = arg(a, "pattern")?.to_lowercase();
    let mut out = Vec::new();
    if start.is_file() {
        let text = std::fs::read_to_string(&start)?;
        for (i, l) in text.lines().enumerate() {
            if l.to_lowercase().contains(&needle) {
                out.push(format!(
                    "{}:{}: {}",
                    a["path"].as_str().unwrap_or(""),
                    i + 1,
                    l.trim()
                ));
            }
        }
    } else {
        walk(&start, &needle, &mut out, root);
    }
    Ok(if out.is_empty() {
        "no matches".into()
    } else {
        cap(out.join("\n"))
    })
}

fn write_file(root: &Path, a: &Value) -> Result<String> {
    let path = resolve(root, arg(a, "path")?)?;
    let content = arg(a, "content")?;
    std::fs::write(&path, content)?;
    Ok(format!("wrote {} bytes", content.len()))
}

fn replace(root: &Path, a: &Value) -> Result<String> {
    let path = resolve(root, arg(a, "path")?)?;
    let (old, new) = (arg(a, "old")?, arg(a, "new")?);
    let text = std::fs::read_to_string(&path)?;
    match text.matches(old).count() {
        0 => bail!("`old` was not found"),
        1 => {
            std::fs::write(&path, text.replacen(old, new, 1))?;
            Ok("replaced".into())
        }
        n => bail!("`old` appears {n} times; include more context so it is unique"),
    }
}

fn run_command(root: &Path, a: &Value) -> Result<String> {
    let cmd = arg(a, "command")?;
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &format!("({cmd}) 2>&1")]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &format!("({cmd}) 2>&1")]);
        c
    };
    let mut child = c
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut pipe = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = pipe.read_to_end(&mut b);
        b
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let out = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    let tail = match status {
        Some(s) => format!("\n[exit {}]", s.code().unwrap_or(-1)),
        None => "\n[killed after 60 seconds]".into(),
    };
    Ok(cap(out) + &tail)
}

/// Runs one call; a failure is the model's result, not the end of the loop.
fn exec(call: &Call, root: &Path, write: bool, run: bool) -> String {
    let a = &call.args;
    let result = match call.name.as_str() {
        "read_file" => read_file(root, a),
        "list_dir" => list_dir(root, a),
        "search" => search(root, a),
        "write_file" if write => write_file(root, a),
        "replace" if write => replace(root, a),
        "run_command" if run => run_command(root, a),
        other => Err(anyhow!("tool `{other}` is not available")),
    };
    result.unwrap_or_else(|e| format!("error: {e}"))
}

fn show(c: &Call) -> String {
    let arg = c
        .args
        .as_object()
        .and_then(|o| o.values().find_map(|v| v.as_str()))
        .unwrap_or("");
    format!(
        "{} {}",
        c.name,
        arg.lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>()
    )
}

/// Runs the tool loop until the model answers without calling a tool. The answer goes to
/// `on_text`; progress (one line per tool call) goes to stderr.
pub fn run(o: &Opts, on_text: &mut dyn FnMut(&str)) -> Result<Reply> {
    let root = std::env::current_dir()?.canonicalize()?;
    let tools = tools(o.write, o.run);
    let system = format!(
        "You are a coding assistant in a terminal, working in {}. Look at files with the tools before answering. Be concise; your final reply is printed as is.",
        root.display()
    );
    let max = o.cfg.num("agent_steps").unwrap_or(25);
    let mut follow: Vec<Msg> = Vec::new();
    let mut total = Reply::default();
    for _ in 0..max {
        let req = Request {
            model: o.model,
            system: &system,
            user: o.user,
            history: &[],
            tools: &tools,
            follow: &follow,
            effort: o.effort,
            max_tokens: o.max_tokens,
        };
        let r = o.provider.complete(&req, &mut |_| {})?;
        total.input_tokens += r.input_tokens;
        total.output_tokens += r.output_tokens;
        total.model = r.model.clone();
        total.stop = r.stop.clone();
        if r.calls.is_empty() {
            on_text(&r.text);
            total.text = r.text;
            return Ok(total);
        }
        if !r.text.trim().is_empty() {
            eprintln!("{}", r.text.trim());
        }
        let results = r
            .calls
            .iter()
            .map(|c| {
                eprintln!("[atclaude] {}", show(c));
                (c.id.clone(), exec(c, &root, o.write, o.run))
            })
            .collect();
        follow.push(Msg::Assistant {
            text: r.text,
            calls: r.calls,
        });
        follow.push(Msg::Results(results));
    }
    bail!("stopped after {max} steps without an answer (raise `agent_steps`)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_leave_the_directory() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(resolve(&root, "../x").is_err());
        assert!(resolve(&root, "/etc/passwd").is_err());
        assert!(resolve(&root, ".").is_ok());
    }
}
