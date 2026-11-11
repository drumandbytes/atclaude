# Usage

`@claude` is the command the [shell integration](install.md#shell-integration) defines; it runs `atclaude`. Both work.

```bash
@claude "where is auth handled in this repo?"
```

## Pipelines

stdin is data and the argument is the instruction. With only stdin, the input itself is the prompt.

```bash
cat error.log | @claude "extract the 500 status codes" | jq .
git diff | @claude "write a one-line commit message" | pbcopy
journalctl -u app | @claude "output a JSON array of distinct error messages" | jq -r '.[]'
```

Whenever stdin **or** stdout is not a terminal you get raw output: a system prompt asks for plain text with no greeting or commentary, a single wrapping code fence is stripped, diagnostics go to stderr, and failures return a non-zero exit code. A reader that exits early (`| head -n 1`) is fine.

Things to know when chaining:

- atclaude reads all of stdin before answering and prints once at the end. `tail -f log | @claude` waits for the stream to end.
- Each stage costs about two seconds, mostly Claude Code's startup.
- Model output is not deterministic. When another tool parses the result, name the format in the instruction ("output a JSON array of ...").
- Piped runs are [recorded](history.md) like any other, piped input included. Use `--no-record` for sensitive input.

## Models

| Flag | Model |
| :--- | :--- |
| `-f`, `--fast` | `fast_model`, default `haiku` |
| `-d`, `--deep` | `deep_model`, default `opus` |
| `-m NAME`, `--model NAME` | any model name or alias `claude --model` accepts |

Without a flag, the `model` setting applies, and without that Claude Code's own default. Summaries and `--search` use `helper_model` (default `haiku`) unless you pass a model flag. See [Configuration](configuration.md).

## Sessions

A session is a name. Turns in the same session share context: the last 8 turns (prompts and responses) are added to the system prompt, and when they exceed 6000 characters the helper model summarizes them first. Piped input is never part of the context.

The session is the first of these that applies:

1. `--session NAME` or `ATCLAUDE_SESSION`: one session across directories and terminals.
2. `session = "name"` in a `.atclauderc` in the current directory or any parent directory, or in the global config.
3. The current terminal (the shell integration sets `ATCLAUDE_TTY`), as `tty-<id>`. Only when stdin is a terminal.
4. None: no context, but the turn is still recorded.

`none` at any level means no session. Piped input skips step 3, so a pipeline has no context unless you name a session.

```bash
atclaude session current    # the session this directory resolves to
atclaude session list       # every session in your history
```

## Changing files

By default atclaude is read-only. `-a` allows file edits and basic file commands, `-y` skips every permission check. Both need a terminal. See [Permissions](permissions.md).

## Searching history

```bash
@claude --search "what broke the deploy?" --in backend --repo api --since 2w
```

See [History](history.md).

## Reference

| Flag | |
| :--- | :--- |
| `-s`, `--session NAME` | Session for this call (`ATCLAUDE_SESSION`) |
| `-f`, `-d`, `-m` | Model, see above |
| `-a`, `--agent` | Allow file edits (terminal only) |
| `-y`, `--yolo` | Skip all permission checks (terminal only) |
| `--record`, `--no-record` | Save or don't save this turn |
| `--search QUESTION` | Answer from your history, with `--since`, `--older`, `--in`, `--repo` |

| Command | |
| :--- | :--- |
| `atclaude init <shell>` | Print the shell integration |
| `atclaude setup` | Refresh the read-only command cache (see [Permissions](permissions.md)) |
| `atclaude config` | Show every setting and where its value comes from |
| `atclaude history` | Browse, `show`, `rm`, `compact`, `clean` |
| `atclaude session` | `list`, `current` |

Exit codes: `0` success, `1` failure (model, database, usage), `2` `-a` or `-y` without a terminal.
