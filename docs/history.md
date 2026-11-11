# History

Every turn that goes through atclaude is saved to a local, encrypted database, so you can search it, browse it and clean it up. Turns without a session are recorded too; they just get no context.

## What is stored

| | |
| :--- | :--- |
| time, session | when, and the session name (empty when there was none) |
| repo, branch, directory | the git repository name and branch you were in, and the working directory |
| mode | `interactive`, `pipe`, `agent`, `yolo` (or `compact` for a summary) |
| model, prompt, response | what was asked and answered |
| piped input | stored separately, cut to `payload_max_chars` (default 20000) |

## Where it is stored

One database per user, shared by every directory. The location is not configurable.

| OS | Path |
| :--- | :--- |
| macOS | `~/Library/Application Support/atclaude/history.db` |
| Linux | `~/.local/share/atclaude/history.db` |

## Encryption

The database is encrypted with SQLCipher. The key is generated on first use and stored in your operating system's credential store:

| OS | Where |
| :--- | :--- |
| macOS | Keychain (read through `/usr/bin/security`) |
| Linux | Secret Service through `secret-tool` (GNOME Keyring tested; KWallet and KeePassXC provide the same API but need their GUI set up first) |
| Windows | Credential Manager (not tested) |

The key never comes from an environment variable. atclaude never creates a new key for an existing database, so a lost key shows an error instead of silently starting over; without the key the history cannot be read.

### Headless machines

Servers and containers often have no keyring. Set `headless = true` (in the global config, or `ATCLAUDE_HEADLESS=1`) and the key is kept in a file instead: `db.key` next to your config file (`~/.config/atclaude/db.key`), created with mode 600. atclaude refuses a key file that other users can read. Anyone who can read both the file and the database can read your history, so this protects against copies of the database, not against access to your account.

## Not recording

```bash
@claude --no-record "..."          # this call
ATCLAUDE_RECORD=0 @claude "..."    # this shell
```

`record = false` in a project's `.atclauderc` or the global config turns it off for a directory or everywhere; `--record` overrides it for one call. A turn that isn't recorded is also missing from later session context.

## Searching

```bash
@claude --search "what was the issue with the logs in the recent deployment?"
@claude --search "how did I set up the proxy?" --in infra --repo cluster --since 3m
```

The helper model reads one line per turn (time, session, repo and branch, mode, the start of the prompt and of the response), newest first until the size budget (`search_budget`, 300000 characters) is used, and answers with the turn ids it relied on. When history is larger than the budget you get a note, and `--since`, `--older`, `--in` and `--repo` narrow what is searched.

Ages are a number and a unit: `12h`, `3d`, `2w`, `1m`.

## Browsing

```bash
atclaude history                      # fzf over everything; Enter prints the turn
atclaude history --repo api --since 1w
atclaude history show 42
```

Without fzf, or when output is piped, it prints a plain list (`id`, time, session, repo, prompt).

## Cleaning up

Every command that deletes shows what it will delete and asks first. `--yes` skips the question, and without a terminal `--yes` is required.

```bash
atclaude history rm 12 13                       # by id
atclaude history rm --older 90d --repo api      # by filter (a filter is required)
atclaude history clean "anything mentioning the staging password" --in ops
atclaude history compact --older 90d
```

- **`clean`** lets the helper model pick turns to delete from an instruction. It sees the same one-line summaries as search and you confirm the list.
- **`compact`** replaces old turns with a model-written summary per session. The summary keeps the time of the newest turn it replaces, is still used as session context and is exempt from retention. The original turns, including their stored piped input, are deleted.

## Retention

After each recorded turn, turns older than `retention_days` (default 365) and beyond the newest `retention_turns` (default 10000) are deleted. `0` disables a limit. Compact summaries are never pruned. Compact before retention removes old turns if you want to keep what they said.

The defaults keep searches and session context small enough for the helper model. Raising them, or setting retention to 0, works, but a larger index costs more per search and can exceed the model's context window, which makes `--search` and `clean` fail. Narrow with filters or lower `search_budget` if that happens.
