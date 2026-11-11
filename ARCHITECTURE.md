# Architecture: `atclaude`

Guide, not law. `atclaude` (`@claude` in the shell) is a thin wrapper around `claude -p` that adds shell ergonomics, session context and an encrypted, searchable history of everything asked through it.

## Principles

- **The history store is the product.** Every turn that goes through `@claude` is recorded by default, including stateless and piped ones; `--no-record`, `ATCLAUDE_RECORD=0` or `record = false` opt out. Sessions are only labels on turns.
- **Claude is stateless.** Each call rebuilds its context from the store, so models, pruning and sessions can change without touching Claude Code state.
- **Subscription auth only.** All model calls go through the `claude` binary (the Messages API needs an API key, which subscriptions don't have). Everything model-related sits behind `llm::complete()`, so a direct-API backend could be added later for people who have a key.
- **Pipes are first-class.** Non-interactive means read-only and raw output; `-a` and `-y` need a terminal.

## Modules

| File | Job |
| :--- | :--- |
| `main.rs` | CLI, `ask` flow, `--search`, history browser, `init` script |
| `llm.rs` | `complete()` (spawns `claude -p`, prompt on stdin), system prompts, tool/permission flags |
| `config.rs` | layered settings: flag > env > `.atclauderc` files > global config; every key also reads `ATCLAUDE_<KEY>` |
| `session.rs` | session resolution cascade, context building and summarizing |
| `db.rs` | SQLCipher database, key handling |
| `setup.rs` | probes your Bash hooks, caches the rewritten read-only rules |

## Sessions

A session is a name. Turns with no session are stateless: recorded, but no context is injected. Resolution order:

1. `--session NAME` or `ATCLAUDE_SESSION`: persistent across directories and terminals
2. `session` in `.atclauderc` in the current directory or a parent, or the global config: directory-scoped (a bare-name `.atclauderc` is still accepted)
3. `ATCLAUDE_TTY` (set per shell by `atclaude init`), as `tty-<id>`: lasts as long as the terminal. Used only when stdin is a terminal
4. Nothing: stateless

`none` at any level means stateless. Piped input skips level 3, so pipelines stay stateless unless a session is explicit.

## Context injection

For a session, the last 8 turns (prompts and responses; piped payloads are left out) are appended to the system prompt. Over 6000 characters they are summarized by Haiku first (`--safe-mode`, no tools); if that call fails the tail of the raw history is used.

## History and search

Encrypted SQLite (SQLCipher). One `turns` table: time, session (null = stateless), cwd, model, prompt, piped payload (separate column), response, duration.

- **Location:** one global database per user, shared by every directory and not configurable. Debug builds honour `ATCLAUDE_DB` / `ATCLAUDE_DB_KEY` so development never touches real history.
- **Key:** the OS credential store (`keychain.rs`: macOS `security` CLI, Linux `secret-tool` (any Secret Service provider), Windows Credential Manager via the `keyring` crate) or, with `headless = true`, a 0600 `db.key` in the config directory, never next to the database; a world-readable key file is refused (`atclaude` / `db-key`), accessed through the `security` CLI on macOS (so "Always Allow" survives rebuilds; a keychain crate is bound to the binary and re-prompts) or `secret-tool` on Linux (untested). A key is only generated for a new database, so a lost keychain entry never silently orphans history. `headless` exists for machines with no keyring; there is deliberately no env-var key, which would land in shell history and process environments.
- **`@claude --search "question" [--since 2w]`:** Haiku (or `-f/-d/-m`) reads a compact index of the history (200 chars of prompt, 400 of response per turn, newest within a 300k-character budget) and answers, citing turn ids.
- **`@claude history [--since 2w]`:** fzf over all turns with a preview; Enter prints the turn. Plain list when fzf is missing or stdout is not a terminal. `history show ID` prints one turn.

- **Columns** (searchable context): time, session, kind (`turn` or `summary`), cwd, repo (git root name) and branch, mode (`interactive`/`pipe`/`agent`/`yolo`/`compact`), terminal id, model, prompt, payload, response, duration. The search index line carries `#id time [session repo@branch mode]` so a model can use them. `--in`, `--repo`, `--since`, `--older` filter in SQL for every history command.
- **Retention:** after each recorded turn, turns older than `retention_days` (365) and beyond the newest `retention_turns` (10000) are deleted; `0` disables a limit. Summaries are exempt. `payload_max_chars` (20000) caps stored piped input. These defaults and the search/context budgets keep what the helper model reads bounded; raising them is allowed at the user's risk.
- **Manual and agentic cleanup:** `history rm` (ids or filter), `history clean "instruction"` (helper model picks ids from the index, user confirms), `history compact` (helper model writes a digest per session chunk, user sees it, then originals are replaced by a `summary` turn). Destructive commands always confirm; no terminal means `--yes` is required.

Not built: FTS indexing, a second search pass that reads full turns for ids Haiku picks, secret redaction at capture. Add when history size or answers demand it.

## Portability

- **Shells:** zsh, bash, fish, PowerShell, Nushell. Init scripts use only `atclaude id` (terminal id, generated at shell start so cached init output stays correct) and `atclaude setup --if-stale --detach` (the binary detaches itself, so no shell needs its own backgrounding syntax).
- **OS:** keychain per OS (above); `sh -c` vs `cmd /C` for hook probing; config dir `~/.config/atclaude` (XDG) on Unix and `%APPDATA%` on Windows; `getrandom` instead of `/dev/urandom`; Unix file modes behind `cfg(unix)`; `claude_path` for non-standard installs.
- **Compatibility** (what was actually run; Linux = Debian container, fake `claude`). Shell syntax behaves the same across Unix OSes and the OS-specific parts are in the binary, so a shell checked on Linux is taken as working on macOS too:

| | macOS | Linux | Windows |
| :--- | :--- | :--- | :--- |
| Build + unit tests | yes | yes (8 pass) | not run |
| zsh, bash 3.2 and 5 (init, alias, unique terminal id) | yes | yes | n/a |
| fish 3.x, PowerShell 7.4, Nushell 0.101 | covered by Linux | yes | not run |
| Cached init output, detached refresh | yes | detach returns 0; refresh not checked | not run |
| Keychain key | yes (`security`) | yes (`secret-tool` + GNOME Keyring) | not run (Credential Manager code written, unbuilt) |
| `headless` key file (mode 600, perms check) | yes | yes | not run |
| No keyring, `headless` off | n/a | clear error pointing at `headless` | not run |

Windows is theoretical: no test machine, so none of it has been built or run, and it is not a target for now. It would need a toolchain for the vendored OpenSSL (Perl, a C compiler) and `claude_path` if `claude` is a `.cmd` shim. "Not run" means unverified, not known broken.

**Linux keyring providers.** GNOME Keyring: store, lookup, missing-key and lost-key all verified. KWallet (Debian KF5 5.103, needs `libqca-qt5-2-plugins`): owns the Secret Service name and a missing-key lookup gives the exit 1 plus empty stderr that `keychain.rs` expects; `store` blocks on KWallet's wallet dialog, so it is unverified without a desktop session. KeePassXC (Secret Service integration enabled): owns the name, but calls hung in a headless container, probably waiting on an unlock or "expose group" dialog; unverified. If a provider misbehaves, `headless = true` bypasses it.

## v2: speed and efficiency

Deferred until v1 is in use. Every `claude -p` call costs about 2s of startup (measured with `claude -p hi`, with or without trimming flags), so v2 is about removing that:

- Optional `backend = "api"` for the helper calls (context summaries, `--search`, `history clean/compact`) and plain Q&A, using `ANTHROPIC_API_KEY` against the Messages API. Agent and read-only tool modes stay on `claude -p`. Needs API credits, which a subscription doesn't include; measure before committing (the ~1-1.5s saving is an estimate).
- Other options: a warm background `claude` process fed over stream-json, prompt caching for repeated `--search` over the same history, FTS indexing and a pick-ids-then-read-full second search pass, streaming output, secret redaction at capture.

## Models

`-f` haiku, `-d` opus, `-m NAME` passthrough, otherwise `claude`'s default. Helper calls (summaries, search) default to Haiku. `claude -p` costs roughly 2s per call regardless of trimming flags, so avoid chaining calls.

## Permissions

See `PERMISSIONS.md`.
