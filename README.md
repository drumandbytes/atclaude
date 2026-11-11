# `atclaude` (`@claude`)

Inline shell companion for Claude Code. Quick questions and pipelines from the prompt, with session context and an encrypted, searchable history of everything you asked.

Needs the `claude` CLI (Claude Code) on PATH. Uses your Claude login, no API key.

## Setup

```bash
cargo build --release
cp target/release/atclaude /usr/local/bin/atclaude   # or anywhere on PATH
```

Then add the shell integration to your shell's startup file. It sets a per-terminal session id, defines the command and refreshes the Bash-hook cache in the background when it is stale:

| Shell | Startup line | Command |
| :--- | :--- | :--- |
| zsh (`~/.zshrc`), bash (`~/.bashrc`) | `eval "$(atclaude init zsh)"` | `@claude` |
| fish (`config.fish`) | `atclaude init fish \| source` | `@claude` |
| PowerShell (`$PROFILE`) | `atclaude init powershell \| Out-String \| Invoke-Expression` | `ask` |
| Nushell | in `env.nu`: `atclaude init nushell \| save -f ~/.cache/atclaude.nu`; in `config.nu`: `source ~/.cache/atclaude.nu` | `ask` |

`atclaude init <shell> --alias NAME` picks another command name. `@name` isn't a valid command in PowerShell and Nushell, hence `ask` there. The init output can be cached (the terminal id is generated when the shell starts, not baked in). Run `atclaude setup` once yourself if you use Bash hooks such as `rtk`.

Platforms: macOS and Linux are tested (zsh, bash, fish, PowerShell and Nushell run on Linux; see the matrix in `ARCHITECTURE.md`). Windows is theoretical: the code paths exist but have never been built or run (no test machine). This is a beta-stage tool; expect rough edges. Building needs a C toolchain and Perl for the vendored OpenSSL behind the encrypted database.

## Usage

```bash
@claude "where is auth handled?"
cat error.log | @claude "extract 500 status codes" | jq .

@claude -f "quick question"          # haiku
@claude -d "hard question"           # opus
@claude -m sonnet "..."              # any model/alias
@claude -a "add a --verbose flag"    # may edit files; terminal only
@claude -y "bump deps and commit"    # yolo: any edit or shell command, no checks; terminal only

@claude --search "what was the issue with logs in the recent deployment?" --since 2w
@claude history                      # fzf over all past turns
```

## In pipelines

stdin is data, the argument is the instruction. Whenever stdin or stdout is not a terminal you get raw output: no markdown, code fence stripped, diagnostics on stderr only, exit code 1 on failure.

```bash
journalctl -u app | @claude "output a JSON array of distinct error messages" | jq -r '.[]'
git diff | @claude "write a one-line commit message" | pbcopy
```

It reads all of stdin before answering and prints once at the end (no streaming), at about 2s per stage. Name the output format in the instruction when another tool will parse it. Every stage is recorded in history (piped input included).

## Sessions

Context carries over inside a session; first match wins:

1. `--session NAME` / `ATCLAUDE_SESSION=NAME`: across directories and terminals
2. `session = "name"` in `.atclauderc` in the directory or a parent (or the global config)
3. the current terminal (set up by `init`)
4. otherwise none: no context, but the turn is still recorded in history

`--session none` forces stateless. Piped input is stateless unless a session is explicit.

## Configuration

Every setting can come from a flag, an env var (`ATCLAUDE_<KEY>`), `.atclauderc` files (TOML; this directory and every parent, nearest wins) or a global `~/.config/atclaude/config.toml` (`ATCLAUDE_CONFIG` points elsewhere). Priority: flag, env, `.atclauderc`, global, default. `atclaude config` shows each effective value and where it came from. Unknown keys and wrong types are reported on stderr and ignored.

```toml
# ~/.config/atclaude/config.toml, or .atclauderc in a project
headless = false               # true: keep the database key in a 0600 file in the config dir instead of the OS keychain (servers, containers)
claude_path = "claude"         # the Claude Code binary (custom location, or a .cmd shim on Windows)
record = true                  # false: don't save turns to history (flags: --record / --no-record)
session = "myproject"          # "none" = no context
model = "sonnet"               # default model (flags -f -d -m win)
helper_model = "haiku"         # summaries and --search
context_turns = 8              # turns injected from the session
context_budget = 6000          # chars; above this, the helper model summarizes
search_budget = 300000         # chars of history index sent for --search
readonly_commands = ["git status", "git diff", "git log", "git show", "ls", "pwd"]   # what plain @claude may run
readonly_bash = ["Bash(foo git status:*)"]   # extra raw rules (env lists are comma-separated)
fast_model = "haiku"           # what -f means
deep_model = "opus"            # what -d means
search_prompt_chars = 200      # per-turn excerpt lengths in the history index used by --search / clean
search_response_chars = 400
retention_days = 365           # turns older than this are deleted on write; 0 = keep forever
retention_turns = 10000        # keep at most this many turns; 0 = unlimited
payload_max_chars = 20000      # piped input stored per turn is cut to this; 0 = unlimited
prompt_pipe = "..."            # override the built-in system prompts
prompt_interactive = "..."
prompt_agent = "..."
```

A `.atclauderc` containing just a name (`myproject`) still works as the session name.

## History

Stored encrypted (SQLCipher). The key lives in the OS credential store (macOS Keychain, Linux Secret Service via `secret-tool` (GNOME Keyring tested end to end; KWallet and KeePassXC provide the same API but need their GUI set up first, see `ARCHITECTURE.md`), Windows Credential Manager), or with `headless = true` in a `db.key` file (mode 600) next to the config file. The database is at `~/Library/Application Support/atclaude/history.db` on macOS, `~/.local/share/atclaude/history.db` on Linux. There is one history per user, shared by every directory; the location is not configurable. The key never comes from an environment variable.

Everything is recorded by default, piped input included, so pipe secrets only with `--no-record`, `ATCLAUDE_RECORD=0` or `record = false` (for example in a project's `.atclauderc`). Turns that aren't recorded are also missing from later session context.

## Searching and managing history

Each turn stores: time, session, repo and branch (when in a git repo), directory, mode (`interactive`, `pipe`, `agent`, `yolo`), model, prompt, piped input and response. Filters work the same everywhere (`--search`, `history`, `rm`, `compact`, `clean`):

```bash
@claude --search "what broke the deploy?" --in backend --repo api --since 2w
atclaude history --repo api --since 1w        # fzf browser; Enter prints the turn
atclaude history show 42
```

**Limits.** Defaults are meant to keep search and context injection small enough for a helper model: 365 days / 10000 turns retained, 8 turns or 6000 characters of session context, a 300000-character search index (newest turns first; older ones are skipped with a notice). Raise any of them, or set retention to 0, at your own risk: a bigger index costs more per search and can exceed the model's context window, which makes `--search` and `clean` fail.

**Delete by hand.**

```bash
atclaude history rm 12 13                      # by id
atclaude history rm --older 90d --repo api     # by filter (a filter is required)
```

**Let Claude clean up.** Both commands show what they will do and ask before deleting anything (`--yes` skips the question; without a terminal `--yes` is required):

```bash
atclaude history clean "anything mentioning the staging password" --in ops
atclaude history compact --older 90d           # replace old turns with a digest per session
```

`compact` writes one summary turn per session chunk (kept at the time of the newest turn it replaces, exempt from retention and still used as session context) and deletes the originals, including their stored piped input. Compact before retention drops old turns if you want to keep what they said.

See `ARCHITECTURE.md` and `PERMISSIONS.md` for the design.
