# History

Every successful call is saved to a local database, so you can browse it, delete from it, and identical calls can be answered from it ([Caching](usage.md#caching)). Calls with `--no-record` or `record = false` are not saved.

## What is stored

| | Readable without the key | |
| :--- | :--- | :--- |
| time, provider, model, tier, mode (`pipe` or `interactive`) | yes | |
| input and output token counts, duration | yes | |
| a hash of the working directory | yes | the path itself is not stored in the clear |
| the instruction, the piped input, the response | no | encrypted |
| the working directory path, git repository and branch | no | encrypted |
| a keyed hash of the call, used for the cache | yes | it reveals nothing about the call |

The readable part is what lets statistics and prompt integrations work without ever opening the encrypted content.

## Where things are stored

| | macOS | Linux |
| :--- | :--- | :--- |
| History database | `~/Library/Application Support/atclaude/history.db` | `~/.local/share/atclaude/history.db` |
| Key | `~/.config/atclaude/key` | `~/.config/atclaude/key` |

One database per user, shared by every directory. Neither location is configurable.

## Encryption

Each record's content is compressed (zstd) and encrypted with XChaCha20-Poly1305, an authenticated cipher, using a random data key. The data key is stored in the database wrapped by your **master key**, so the master key can be replaced without rewriting the history.

What this protects: the history when the file is copied, synced or backed up by itself. It does not protect a machine or account that is already compromised, because whoever can read your key file can read your history. Treat the key like an SSH private key.

### The key

The master key is random and 256 bits long. It is created the first time history is written, and atclaude never creates a new key for an existing history: if the key is missing you get an error, not an empty history.

```bash
atclaude key show      # print the key as 24 words
atclaude key import    # restore it (the words are read from stdin)
atclaude key rotate    # replace it with a new one; the history stays readable
```

Write the 24 words down. They are the only way back in if the key file is lost, and the way to move your history to another machine: copy the database, then `atclaude key import` there. `key import` refuses a key that does not unlock the existing history. After `key rotate`, the old words no longer work, so show and store the new ones.

### Where the key lives

By default in a file next to the config, created with mode 600. atclaude refuses a key file that other users can read.

With `keychain = true` it is kept in the operating system's credential store instead: macOS Keychain, Linux Secret Service (GNOME Keyring; KWallet and KeePassXC provide the same API), Windows Credential Manager (untested). On Linux this needs the `secret-tool` command (package `libsecret-tools`). To switch an existing history, run `atclaude key show` under the old setting, change the setting, then `atclaude key import`.

## Browsing

```bash
atclaude history                       # fzf over everything; Enter prints the turn
atclaude history --here --since 1w     # only this directory, last week
atclaude history show 42
```

Without fzf, or when output is piped, it prints a plain list. Filters: `--since 12h|3d|2w|1m`, `--older ...`, `--here` (current directory).

## Searching

```bash
atclaude history search kestrel nest      # turns containing every word, any case, in the prompt, input or answer
atclaude history ask "what did I ask about TLS last month?"
```

`search` is plain text matching and makes no network call. `ask` sends excerpts of your most recent turns to the fast model and prints its answer, so those excerpts leave your machine like any other prompt. The excerpt size and how many turns are scanned are settings (`excerpt_prompt`, `excerpt_reply`, `history_scan`). Neither command is recorded in the history. Filters (`--since`, `--older`, `--here`) narrow what is searched.

## Deleting

Deleting asks first. `--yes` skips the question, and without a terminal `--yes` is required.

```bash
atclaude history rm 12 13                 # by id
atclaude history rm --older 90d           # by filter (a filter is required)
atclaude history rm --here --since 1d     # everything from this directory today
```

By instruction, with a model proposing what to delete:

```bash
atclaude history agent "remove the throwaway test questions from last week"
```

The model sees the same excerpts as `ask` and returns turn ids. You see the list and confirm before anything is deleted, and only ids that were offered to the model are ever deleted.

## Size and age limits

The history keeps itself within `max_size` (default `500m` of stored content, measured after compression and encryption). When a call pushes it over, the oldest turns are removed first and a line on stderr says so. `max_age` (for example `52w`) additionally drops anything older. Raise or remove the limits in your configuration if you want a longer history; the defaults keep it from growing without bound.

```bash
atclaude history clean        # apply the limits now and shrink the file (asks first)
```
