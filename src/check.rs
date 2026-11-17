use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

/// Whether `text` satisfies `--json` (parses as one JSON value) and `--check` (the command
/// reads it on stdin and exits 0).
pub fn passes(text: &str, json: bool, cmd: Option<&str>) -> bool {
    (!json || serde_json::from_str::<Value>(text.trim()).is_ok())
        && cmd.is_none_or(|c| run(c, text))
}

fn run(cmd: &str, text: &str) -> bool {
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    let Ok(mut child) = c
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // a checker that stops reading early is not an error here; its exit status decides
    let _ = child
        .stdin
        .take()
        .expect("piped")
        .write_all(text.as_bytes());
    child.wait().map(|s| s.success()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_command_checks() {
        assert!(passes("[1, 2]\n", true, None));
        assert!(!passes("[1, 2", true, None));
        assert!(!passes("sure! [1]", true, None));
        assert!(passes("anything", false, None));
        if cfg!(unix) {
            assert!(passes("ok\n", false, Some("grep -q ok")));
            assert!(!passes("nope\n", false, Some("grep -q ok")));
            assert!(!passes("[1]", true, Some("false")));
        }
    }
}
