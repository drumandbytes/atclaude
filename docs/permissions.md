# Permissions

atclaude passes your intent to `claude -p` as tool and permission flags. What Claude may do depends on the mode:

| Mode | What Claude can do | Needs a terminal |
| :--- | :--- | :--- |
| Default (`@claude "..."`, pipelines) | Read files (`Read`, `Glob`, `Grep`), fetch the web (`WebFetch`, `WebSearch`) and run a short list of read-only shell commands | no |
| `-a`, `--agent` | Claude Code's normal tools with edits accepted: file edits plus basic file commands (`touch`, `mkdir`, `rm`, `mv`, `cp`, `sed`). Other commands only if your own Claude Code settings allow them | yes |
| `-y`, `--yolo` | Everything, with no permission checks | yes |
| Helper calls (summaries, `--search`, `clean`, `compact`) | Nothing: no tools, and your Claude Code customizations are off | no |

`-a` and `-y` without a terminal (a pipeline, a script, cron) exit with code `2` and print to stderr:

```json
{"error":"permission_denied","reason":"agent mode needs an interactive terminal","exit_code":2}
```

## Read-only shell commands

The default mode denies every shell command except the ones in `readonly_commands`: `git status`, `git diff`, `git log`, `git show`, `ls` and `pwd`. Writes, redirects, `tee`, compound commands with a command outside the list and flags such as `git diff --output=file` are denied.

Change the list in your [configuration](configuration.md). Commands that can write or run other programs (`find`, `rg --pre`, `sort -o`, `sed -i`) defeat the point of read-only mode; leave them out.

## Hooks that rewrite commands

Your Claude Code settings stay active, so hooks run. A hook that rewrites commands changes what the permission rules see: a tool like [RTK](https://github.com/rtk-ai/rtk) turns `git status` into `rtk git status`, which a plain `Bash(git status:*)` rule would never match.

`atclaude setup` handles this without hook-specific code. It reads the `PreToolUse` hooks that apply to Bash from `~/.claude/settings.json` (or `$CLAUDE_CONFIG_DIR`), runs each one with a synthetic payload for every allowed command, and caches the rewritten forms next to the history database. No model is involved.

- The shell integration runs it in the background, and only when the cache is missing or older than your Claude Code settings or atclaude config.
- Rewrites containing anything but plain words are ignored, and only the commands in `readonly_commands` are ever probed.
- Without a cache, read-only Bash is denied and a one-line hint is printed; run `atclaude setup`.
- For a rewrite the probe cannot see, add rules by hand with `readonly_bash` (or `ATCLAUDE_READONLY_BASH="Bash(foo git status:*),Bash(...)"`).

Project-level Claude Code settings and chains of hooks are not considered.
