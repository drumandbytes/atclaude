# Configuration

Every setting can be given four ways. The first that applies wins:

1. A **flag** (where one exists, such as `--no-record` or `-m`)
2. An **environment variable**: `ATCLAUDE_` plus the setting name in capitals, e.g. `ATCLAUDE_RETENTION_DAYS=30`
3. A **`.atclauderc`** file in the current directory or a parent. Closer files win over farther ones
4. The **global config**: `~/.config/atclaude/config.toml` (`$XDG_CONFIG_HOME` is respected; `ATCLAUDE_CONFIG` points at another file)

```toml
# ~/.config/atclaude/config.toml, or .atclauderc in a project
record = true
session = "myproject"
model = "sonnet"
retention_days = 180
```

`atclaude config` prints every setting, its effective value and its source (`env`, `config` or `default`) and the files that were read. Unknown keys and values of the wrong type are reported on stderr and ignored, so a typo never silently changes behaviour.

Lists in environment variables are comma-separated. Booleans accept `1/0`, `true/false`, `yes/no`, `on/off`.

A `.atclauderc` containing only a name (`myproject`) is still read as `session = "myproject"`.

## Settings

| Setting | Default | |
| :--- | :--- | :--- |
| `record` | `true` | Save turns to history. `--record` / `--no-record` override per call |
| `session` | none | Session name. `none` disables context |
| `model` | Claude Code's default | Model for ordinary calls |
| `fast_model` | `haiku` | What `-f` selects |
| `deep_model` | `opus` | What `-d` selects |
| `helper_model` | `haiku` | Model for summaries, `--search`, `history clean` and `compact` |
| `context_turns` | `8` | Turns of the session added as context |
| `context_budget` | `6000` | Characters of context before it is summarized |
| `search_budget` | `300000` | Characters of history sent for `--search` and `clean` |
| `search_prompt_chars` | `200` | Per turn, how much of the prompt the search index shows |
| `search_response_chars` | `400` | Per turn, how much of the response it shows |
| `retention_days` | `365` | Delete turns older than this; `0` = keep |
| `retention_turns` | `10000` | Keep at most this many turns; `0` = unlimited |
| `payload_max_chars` | `20000` | Stored piped input per turn is cut to this; `0` = unlimited |
| `headless` | `false` | Keep the database key in a file instead of the OS credential store ([History](history.md#headless-machines)) |
| `claude_path` | `claude` | The Claude Code binary: a custom location, or a `.cmd` shim |
| `readonly_commands` | `git status`, `git diff`, `git log`, `git show`, `ls`, `pwd` | Shell commands plain `@claude` may run ([Permissions](permissions.md)) |
| `readonly_bash` | none | Extra raw permission rules for read-only mode, e.g. `Bash(foo git status:*)` |
| `prompt_pipe`, `prompt_interactive`, `prompt_agent` | see below | System prompts, one per mode |

`session` and `record` can sit in a project's `.atclauderc` for per-directory behaviour; a project that should never be recorded just says `record = false`.

## System prompts

atclaude appends one of these to Claude Code's own prompt, depending on the mode. Replace any with the setting above; multi-line TOML strings work.

**Pipe** (stdin or stdout is not a terminal):

> You are a POSIX command-line tool in a UNIX pipeline. Output ONLY the raw text or data requested. No markdown, no code fences, no greetings, no commentary.

**Interactive** (terminal in and out):

> You are an inline terminal companion. Answer directly, no pleasantries. Keep it under 15 lines unless asked for more. Prefer commands, snippets and paths over background.

**Agent** (`-a` or `-y`):

> You are a workspace agent. After edits or test runs, output ONLY a concise ledger: `[ACTION] <path> | [STATUS] <ok/fail> | [SUMMARY] <one line>`. No diff dumps unless asked.

When a session has history, the context is appended as "Earlier turns in this session, for continuity:" followed by the turns or their summary.

## Environment variables that are not settings

| Variable | |
| :--- | :--- |
| `ATCLAUDE_SESSION` | Same as `--session` |
| `ATCLAUDE_TTY` | The terminal id; set by the [shell integration](install.md#shell-integration), not by hand |
| `ATCLAUDE_CONFIG` | Path of the global config file |
