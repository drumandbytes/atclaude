# prinia

Call AI models from the command line the way you call `jq`, `grep` or `curl`: data on stdin, an instruction as the argument, the result on stdout. Every call is saved to an encrypted local history, so identical calls are answered instantly and nothing you asked is lost.

```bash
prinia "where does a unix pipe buffer its data?"

journalctl -u app | prinia "output a JSON array of distinct error messages" | jq -r '.[]'

git diff | prinia -f "write a one-line commit message" | pbcopy

prinia -d "review this design for race conditions" < design.md
```

- **A real Unix filter.** When input or output is piped you get raw text: no markdown, no code fences, errors on stderr, a non-zero exit code on failure.
- **Any provider.** Anthropic and any OpenAI-compatible endpoint, which covers OpenAI, Ollama and other local runners, and most hosted services. No vendor CLI, no SDK.
- **Fast.** Answers stream to your terminal as they arrive. Starting up takes milliseconds, and a call that is already in your history returns without touching the network.
- **Fast, regular and deep tiers.** `-f` and `-d` pick a model for the job; each provider maps them to its own models.
- **History you own.** Prompts, input and responses are compressed and encrypted on disk. Browse it, delete from it, back the key up as 24 words.
- **Configurable.** Every setting works as a flag, an environment variable, a project file or a global file.

## Install

```bash
curl -fsSL https://github.com/drumandbytes/prinia/releases/latest/download/install.sh | sh
# or
brew install drumandbytes/tap/prinia
```

Then give it a provider:

```bash
export ANTHROPIC_API_KEY=...      # or OPENAI_API_KEY, or configure a local model
prinia "hello"
```

See [Install](docs/install.md) for other platforms and building from source, and [Configuration](docs/configuration.md) for local models and other providers.

## Usage

```bash
prinia "question"                    # ask; the answer streams to the terminal
cmd | prinia "instruction" | cmd2    # a pipeline step
prinia -f "..."                      # fast tier
prinia -d "..."                      # reasoning tier
prinia -p local "..."                # a configured provider
prinia -a "..."                     # agent: the model reads and searches files here (--write, --run to allow more)
prinia -c "..."                     # include earlier turns from this directory
prinia --no-cache "..."              # ask again even if the call is in your history
prinia --no-record "..."             # don't save this call

prinia history                       # browse everything you asked (fzf when available)
prinia history search kestrel        # find turns by words in prompt, input or answer
prinia history ask "what did I ask about TLS?"   # a fast model answers from your history
prinia history rm --older 90d        # delete by age (asks first)
prinia key show                      # your history key as 24 words
prinia stats                         # calls, cache hits, tokens (also --json, --format for prompts)
prinia config                        # every setting and where its value comes from
```

More in [Usage](docs/usage.md) and [History](docs/history.md).

## Documentation

| | |
| :--- | :--- |
| [Install](docs/install.md) | Script, Homebrew, source; platforms |
| [Usage](docs/usage.md) | Pipelines, tiers, caching, flags |
| [Configuration](docs/configuration.md) | Providers, models, every setting, environment variables |
| [History](docs/history.md) | What is stored, encryption and keys, deleting |
| [Agent](docs/agent.md) | Tools, permissions, limits |
| [Stats](docs/stats.md) | Usage totals, Starship and Oh My Posh examples |
| [Development](docs/development.md) | Building, tests, how it is put together, releases |

## Status

prinia is beta software.

- macOS and Linux are tested. Windows is untested.
- Agent mode has run against a mock server and one OpenAI-compatible provider; the Anthropic tool-call format has not run against the real API yet.

## License

[MIT](LICENSE)

Part of [Drumandbytes](https://drumandbytes.com/projects/).
