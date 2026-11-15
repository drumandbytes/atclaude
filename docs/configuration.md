# Configuration

Every setting can be given four ways. The first that applies wins:

1. A **flag** (where one exists, such as `-m` or `--no-record`)
2. An **environment variable**: `ATCLAUDE_` plus the setting name in capitals, e.g. `ATCLAUDE_MAX_TOKENS=2000`
3. A **`.atclauderc`** file in the current directory or a parent. Closer files win over farther ones
4. The **global config**: `~/.config/atclaude/config.toml` (`$XDG_CONFIG_HOME` is respected, on Windows `%APPDATA%\atclaude`; `ATCLAUDE_CONFIG` points at another file)

```toml
# ~/.config/atclaude/config.toml, or .atclauderc in a project
provider = "anthropic"
record = true
cache_ttl = "3d"
```

`atclaude config` prints every setting, its effective value and its source (`env`, `config` or `default`), the files that were read and the configured providers. Unknown keys and values of the wrong type are reported on stderr and ignored, so a typo never silently changes behaviour. Booleans in environment variables accept `1/0`, `true/false`, `yes/no`, `on/off`.

A `.atclauderc` is how a project gets its own behaviour: a repository that should never be recorded says `record = false`; one that must use a local model says `provider = "local"`.

## Providers

Two wire formats are supported: Anthropic's Messages API (`type = "anthropic"`) and OpenAI-compatible chat completions (`type = "openai"`). Anthropic and OpenAI are built in and need only their API key in `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`. Anything else is a table in the config:

```toml
provider = "local"                       # the default provider

[providers.local]                        # Ollama
type = "openai"
base_url = "http://localhost:11434/v1"
model = "llama3.2"                       # used for every tier

[providers.together]                     # any OpenAI-compatible host
type = "openai"
base_url = "https://api.together.xyz/v1"
api_key_env = "TOGETHER_API_KEY"         # which environment variable holds the key
model = "meta-llama/Llama-3.3-70B-Instruct-Turbo"
fast_model = "meta-llama/Llama-3.2-3B-Instruct-Turbo"
```

| Field | |
| :--- | :--- |
| `type` | `anthropic` or `openai` |
| `base_url` | The API endpoint. Defaults to the official one for the type |
| `api_key_env` | Environment variable holding the key. Defaults to `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`. Not needed for a local endpoint |
| `model`, `fast_model`, `deep_model` | Models for the three tiers |

Without `-p` or `provider`, atclaude uses the first of `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` that is set.

API keys are read from the environment and are never written to the history or the config. Calls to the official Anthropic endpoint with a model that can decline a request ask the API to retry on a fallback model automatically.

## Settings

| Setting | Default | |
| :--- | :--- | :--- |
| `provider` | see above | Default provider |
| `model` | provider's default | Model for the regular tier |
| `fast_model` / `deep_model` | provider's default | Models for `-f` / `-d` |
| `effort`, `fast_effort`, `deep_effort` | `medium`, `low`, `high` | Reasoning effort per tier (Anthropic only; `none` omits it) |
| `max_tokens` | Anthropic: 64000, others: the provider's | Output limit |
| `record` | `true` | Save calls to the history |
| `cache` | `pipe` | When identical calls are answered from history: `pipe`, `always`, `off` |
| `context` | `0` | Earlier turns from this directory sent with every call; `-c` uses it, or 5 when unset |
| `context_chars` | `16000` | Size cap for that context (characters; oldest turns are dropped first) |
| `cache_ttl` | `7d` | How long a history entry can answer a call (`12h`, `3d`, `2w`, `1m`) |
| `keychain` | `false` | Keep the history key in the OS credential store instead of a file ([History](history.md#where-the-key-lives)) |
| `prompt_pipe`, `prompt_interactive` | see below | The system prompts |

Provider tables override the top-level `model`, `fast_model` and `deep_model`.

## System prompts

atclaude sends one of these as the system prompt, depending on whether input or output is piped. Replace either with the setting above; multi-line TOML strings work.

**Pipe** (stdin or stdout is not a terminal):

> You are a command-line tool in a UNIX pipeline. Output ONLY the raw text or data requested. No markdown, no code fences, no greetings, no commentary.

**Interactive** (terminal in and out):

> You are an inline terminal companion. Answer directly, no pleasantries. Keep it under 15 lines unless asked for more. Prefer commands, snippets and paths over background.

## Environment variables that are not settings

| Variable | |
| :--- | :--- |
| `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` (or the variable named by `api_key_env`) | Provider credentials |
| `ATCLAUDE_CONFIG` | Path of the global config file |
