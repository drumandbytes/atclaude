# Usage

```bash
atclaude "where does a unix pipe buffer its data?"
```

On a terminal the answer streams as it is generated.

## Pipelines

stdin is data and the argument is the instruction. With only stdin, the input itself is the prompt.

```bash
cat error.log | atclaude "extract the 500 status codes" | jq .
git diff | atclaude "write a one-line commit message" | pbcopy
journalctl -u app | atclaude "output a JSON array of distinct error messages" | jq -r '.[]'
```

Whenever stdin **or** stdout is not a terminal, atclaude behaves as a filter: a system prompt asks for plain text with no greeting or commentary, a single wrapping code fence is stripped, diagnostics go to stderr, and the output is written once, when the answer is complete. A reader that exits early (`| head -n 1`) is fine.

Things to know when chaining:

- atclaude reads all of stdin before calling the model. `tail -f log | atclaude` waits for the stream to end.
- Model output is not deterministic. When another tool parses the result, name the format in the instruction ("output a JSON array of ...").
- If the answer is cut off at the output limit, atclaude says so on stderr; raise [`max_tokens`](configuration.md).
- The exit code is `0` on success and `1` on any failure (network, provider error, a refused request, configuration).

## Tiers and models

| Flag | Tier | Anthropic default |
| :--- | :--- | :--- |
| `-f`, `--fast` | fast | `claude-haiku-5-5`, effort `low` |
| (none) | regular | `claude-sonnet-5-5`, effort `medium` |
| `-d`, `--deep` | deep | `claude-opus-5-5`, effort `high` |
| `-m NAME`, `--model NAME` | an exact model name | |

Other providers have no built-in models: set `model` (and optionally `fast_model` and `deep_model`) for the provider. A provider with only `model` set uses it for every tier. See [Configuration](configuration.md).

Without a flag, the regular tier is used. To choose by the size of the call instead, set `fast_under` (calls with fewer characters than this use the fast tier) and `deep_over` (calls with more use the deep tier). `-f`, `-d` and `-m` always win. The rule counts the instruction plus the piped input, so a short question about a large file goes to the regular or deep tier.

## Caching

A call identical to one already in your history is answered from the history, with no network request and no cost. "Identical" means the same provider, model, system prompt, instruction, input, effort and token limit. Entries expire after `cache_ttl` (default 7 days).

By default the cache is used for piped calls (`cache = "pipe"`), where repeating a script should repeat its result. Interactive questions always go to the model, since you usually ask again to get a fresh answer. Set `cache = "always"` to use it everywhere or `"off"` to never. `--no-cache` skips it for one call.

A call can only be served from history if it was recorded, so `record = false` and `--no-record` calls never become cache entries.

## Context

Models have no memory between calls. `-c` (`--context`) sends the most recent turns from your history in this directory along with the new question, so a follow-up knows what came before:

```bash
atclaude "what does SIGPIPE do?"
atclaude -c "and how do I ignore it in Rust?"
```

The context comes from your own history, not from a provider's session, so it works with any provider and survives a switch of model. Only turns from the current directory are used, oldest first, and long turns are cut. Set `context = 5` (a number of turns) to include it on every call and `--no-context` to leave it out once. `context_chars` (default 16000) caps its size; the oldest turns are dropped first. Calls made with `--no-record` are not in the history, so they are never part of a context.

A call with context is cached separately from the same question without it.

## Recording

Every successful call is saved to the [history](history.md) after its output has been written. `--no-record` skips it for one call; `record = false` (or `ATCLAUDE_RECORD=0`) turns it off for a directory or everywhere, and `--record` overrides that for one call.

## Providers

`-p NAME` (`--provider`) selects a provider for one call. Without it, atclaude uses the `provider` setting, then whichever of `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` is set. See [Configuration](configuration.md#providers).

## Reference

| Flag | |
| :--- | :--- |
| `-p`, `--provider NAME` | Provider for this call |
| `-f`, `--fast` / `-d`, `--deep` | Tier |
| `-m`, `--model NAME` | Exact model |
| `-c`, `--context` | Include earlier turns from this directory |
| `--no-context` | Send no earlier turns, even if `context` is set |
| `--no-cache` | Skip the history cache |
| `--record` / `--no-record` | Save or don't save this call |

| Command | |
| :--- | :--- |
| `atclaude config` | Show every setting and where its value comes from |
| `atclaude history` | Browse, `show`, `rm` ([History](history.md)) |
| `atclaude key` | `show`, `import`, `rotate` ([History](history.md#the-key)) |
