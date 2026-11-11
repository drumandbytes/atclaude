# Install

## Requirements

- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed, on your `PATH` (or set [`claude_path`](configuration.md)), and logged in. atclaude calls `claude -p` and uses its login.
- macOS or Linux. [fzf](https://github.com/junegunn/fzf) is optional and powers the history browser.
- Linux only: for the encryption key, a Secret Service provider (GNOME Keyring, KWallet, KeePassXC) and the `secret-tool` command (package `libsecret-tools`). Without one, use [headless mode](history.md#headless-machines).

## Homebrew

```bash
brew install drumandbytes/tap/atclaude
```

## Install script

```bash
curl -fsSL https://atclaude.drumandbytes.dev/install.sh | sh
```

The script picks the release for your OS and architecture, verifies it against the published checksums and installs `atclaude` to `~/.local/bin`. It refuses to install on a checksum mismatch.

| Variable | Effect |
| :--- | :--- |
| `ATCLAUDE_VERSION=v0.1.0` | Install a specific release instead of the latest |
| `ATCLAUDE_INSTALL_DIR=/usr/local/bin` | Install somewhere else |
| `ATCLAUDE_DOWNLOAD_URL=...` | Download from a mirror |

## From source

```bash
git clone https://github.com/drumandbytes/atclaude
cd atclaude
cargo install --path .
```

The encrypted database is built from source, which needs a C compiler and Perl. See [Development](development.md).

## Shell integration

Add one line to your shell's startup file. It gives each terminal its own session, defines the command and keeps the [read-only command cache](permissions.md#hooks-that-rewrite-commands) fresh in the background. The output can be cached.

| Shell | Startup line | Command |
| :--- | :--- | :--- |
| zsh (`~/.zshrc`), bash (`~/.bashrc`) | `eval "$(atclaude init zsh)"` | `@claude` |
| fish (`config.fish`) | `atclaude init fish \| source` | `@claude` |
| PowerShell (`$PROFILE`) | `atclaude init powershell \| Out-String \| Invoke-Expression` | `ask` |
| Nushell | in `env.nu`: `atclaude init nushell \| save -f ~/.cache/atclaude.nu`, in `config.nu`: `source ~/.cache/atclaude.nu` | `ask` |

Use `bash` instead of `zsh` in the first line for bash. macOS's stock bash 3.2 works with the `eval` form shown.

`atclaude init <shell> --alias NAME` picks another command name. PowerShell and Nushell don't accept `@claude` as a command, which is why they default to `ask`.

## Platforms

| | macOS | Linux | Windows |
| :--- | :--- | :--- | :--- |
| Supported | yes | yes | no |
| Shells | zsh, bash, fish, PowerShell, Nushell | same | not tested |
| Encryption key | Keychain | Secret Service, or `headless` key file | not tested |

macOS and Linux (glibc 2.35 or newer) have prebuilt binaries for x86-64 and arm64. Windows has code paths (Credential Manager, `cmd` for hooks) that have never been built or run.

## Upgrade and uninstall

```bash
brew upgrade atclaude              # Homebrew
curl -fsSL https://atclaude.drumandbytes.dev/install.sh | sh   # script: reinstalls the latest
```

To uninstall, remove the binary and, if you want to delete your data, the history database and key (see [History](history.md#where-it-is-stored)).
