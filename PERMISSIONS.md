# Permissions

Enforced through `claude -p` flags, not by scanning output.

| Mode | Tools | Needs terminal |
| :--- | :--- | :--- |
| Default (`@claude "..."`, pipes) | `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`, plus Bash limited to `git status/diff/log/show`, `ls`, `pwd` | no |
| Agent (`-a`) | `claude`'s normal tools, `--permission-mode acceptEdits`: file edits plus basic file commands (touch, mkdir, rm, mv, cp, sed); other commands only if your own settings allow them | yes |
| Yolo (`-y`) | `--permission-mode bypassPermissions`: every edit and shell command, no checks | yes |
| Helper calls (summaries, `--search`) | none, `--safe-mode` | no |

`-a` or `-y` with piped stdin exits with code `2` and prints to stderr:

```json
{"error":"permission_denied","reason":"agent mode needs an interactive terminal","exit_code":2}
```

Exit codes: `0` success, `1` failure (model, database, usage), `2` `-a`/`-y` without a terminal.

## Read-only Bash and hooks

Default mode uses `--permission-mode dontAsk` plus an allowlist (`readonly_commands`, default `git status`, `git diff`, `git log`, `git show`, `ls`, `pwd`), so anything not listed is denied (writes, redirects, `tee`, compound commands with a forbidden part, `git diff --output=...`; all tested).

Your Claude settings stay on, so hooks run. A hook that rewrites commands changes what the rules see: RTK turns `git status` into `rtk git status`, which a plain `Bash(git status:*)` rule never matches (denied even though the command is harmless).

`atclaude setup` handles this without any hook-specific code. It reads the `PreToolUse` hooks that apply to Bash from `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`), runs each one with a synthetic payload for every allowed command, and caches the rewritten forms (`readonly-rules.txt` next to the database). No AI is involved. `atclaude init` starts it in the background, but only when the cache is missing or older than `settings.json`. Without a cache, read-only Bash fails closed and a one-line hint is printed.

Rewrites containing anything but plain words are ignored, and only the allowed read-only commands are ever probed, so a hook cannot widen the allowlist beyond its rewrite of those commands. For cases the probe can't see, the `readonly_bash` setting (or `ATCLAUDE_READONLY_BASH="Bash(foo git status:*),Bash(...)"`, comma-separated) adds rules by hand. Changing `readonly_commands` or your config makes the cache stale, so `init` re-probes on the next shell start (or run `atclaude setup`). Allowing commands that can write or execute (`find`, `rg --pre`, `sort -o`, `sed -i`) defeats the read-only mode. Project-level settings and chained hooks are not handled.
