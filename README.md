# atclaude

Ask [Claude Code](https://docs.claude.com/en/docs/claude-code) questions from your shell prompt, use it as a step in a pipeline, and keep an encrypted, searchable history of everything you asked.

[Website](https://atclaude.drumandbytes.dev/?ref=atclaude-readme) · [Install](docs/install.md) · [Usage](docs/usage.md) · [Configuration](docs/configuration.md)

```bash
@claude "where is auth handled in this repo?"

journalctl -u app | @claude "output a JSON array of distinct error messages" | jq -r '.[]'

@claude --search "what was the issue with the deployment logs last week?" --since 2w
```

- **Inline.** One command, no TUI. Answers print and the prompt returns.
- **Pipeline-friendly.** stdin is data, the argument is the instruction. When input or output is piped you get raw text: no markdown, no code fences, errors on stderr, a real exit code.
- **Safe by default.** Read-only unless you pass `-a` (edits) or `-y` (everything).
- **Sessions.** Context carries over per terminal, per directory (a `.atclauderc` file), or under a name you choose.
- **History you own.** Every turn is stored in an encrypted local database with its repo, branch, session and mode. Search it in plain language, browse it with fzf, delete or compact it.
- **Configurable.** Every setting works as a flag, an environment variable, a project file or a global file.

atclaude runs on top of your existing Claude Code login. It needs no API key.

## Install

```bash
brew install drumandbytes/tap/atclaude          # macOS, Linux with Homebrew
# or
curl -fsSL https://atclaude.drumandbytes.dev/install.sh | sh
```

Then add the shell integration to your startup file, for zsh:

```bash
eval "$(atclaude init zsh)"
```

bash, fish, PowerShell and Nushell are supported too. See [Install](docs/install.md) for every shell, building from source and requirements.

## Quick tour

```bash
@claude "explain this error: connection reset by peer"   # a one-off question
@claude -f "quick question"                               # fast model (haiku)
@claude -d "hard question"                                # deep model (opus)
git diff | @claude "write a one-line commit message"      # as a pipeline step
@claude -a "add a --verbose flag to the CLI"              # may edit files
```

Context carries over inside a session. By default each terminal is its own session; a `.atclauderc` containing `session = "myproject"` makes a directory share one. `atclaude config` shows every setting and where its value comes from.

```bash
atclaude history                  # fzf over everything you asked; Enter prints the turn
atclaude history rm --older 90d   # delete by age, session or repo (asks first)
atclaude history compact --older 90d   # replace old turns with a summary
```

## Documentation

| | |
| :--- | :--- |
| [Install](docs/install.md) | Homebrew, script, source; every supported shell; platforms |
| [Usage](docs/usage.md) | Pipelines, models, sessions, all flags |
| [History](docs/history.md) | What is stored, search, cleanup, retention, encryption |
| [Configuration](docs/configuration.md) | All settings, layering, environment variables, prompts |
| [Permissions](docs/permissions.md) | Read-only mode, `-a`, `-y`, hooks |
| [Development](docs/development.md) | Building, tests, how it is put together, releases |

## Status

atclaude is beta software. Things to know:

- Each call goes through `claude -p`, which adds about two seconds of startup. Output is printed when the answer is complete, not streamed.
- macOS and Linux are supported. Windows is not: the code has paths for it, but it has never been built or run.
- On Linux the encryption key is stored through the Secret Service (GNOME Keyring is tested). On machines without one, set `headless = true`.

## License

[MIT](LICENSE)

Part of [Drumandbytes](https://drumandbytes.com/projects/).
