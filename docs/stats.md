# Stats and prompt integration

```bash
atclaude stats                    # all time: calls, answered from history, tokens
atclaude stats --here --since 0d  # this directory, today
atclaude stats --since 7d --json  # for scripts
atclaude stats --format '{calls} calls, {hit_rate}% cached'
```

`--since` counts days: `0d` is today, `7d` the last seven days, `4w` four weeks. `--here` limits it to the current directory. With no options it covers everything in your history.

Placeholders for `--format`: `{calls}`, `{cache_hits}`, `{hit_rate}` (percent of calls answered from history), `{input_tokens}`, `{output_tokens}`, `{tokens}`. `--json` prints `calls`, `cache_hits`, `hit_rate`, `input_tokens` and `output_tokens`.

Stats come from a small table that is updated after each call, and they never read your history's content, so `stats` is fast (a few milliseconds) and does not need the history key. That makes it safe to run in a shell prompt. Cost is not shown, because prices change and differ per provider; multiply the token counts by your provider's current prices.

## Starship

Add to `~/.config/starship.toml`. This shows today's calls in this directory and hides itself when there are none:

```toml
[custom.atclaude]
command = "atclaude stats --here --since 0d --format '{calls} calls · {hit_rate}% cached'"
when = "test \"$(atclaude stats --here --since 0d --format '{calls}')\" -gt 0"
symbol = "◆ "
style = "dimmed"
format = "[$symbol$output]($style) "
```

## Oh My Posh

A `command` segment in your theme:

```json
{
  "type": "command",
  "style": "plain",
  "foreground": "#888888",
  "properties": {
    "shell": "bash",
    "command": "atclaude stats --here --since 0d --format '{calls} calls'"
  }
}
```

## Plain shell

Any shell can call it in its prompt. In bash:

```bash
PS1='$(atclaude stats --here --since 0d --format "{calls} ")\$ '
```

In PowerShell, use `atclaude stats --here --since 0d --format '{calls}'` inside your `prompt` function.

The Starship example is tested. The others follow those tools' documented syntax but have not been run.
