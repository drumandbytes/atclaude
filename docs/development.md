# Development

## Build and test

```bash
cargo build --release
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

Building needs a Rust toolchain and a C compiler (bundled SQLite and zstd). The end-to-end tests in `tests/cli.rs` run the real binary against a local mock server that speaks both wire formats, so they need no network or API key.

Debug builds read `ATCLAUDE_DB` (the database path) so development and tests never touch your real history. Release builds ignore it. `ATCLAUDE_CONFIG` points any build at a throwaway config file.

## Layout

| File | |
| :--- | :--- |
| `src/main.rs` | Command line and the `ask` flow |
| `src/provider.rs` | The two wire formats, streaming, tool calls, retries, model and effort selection |
| `src/store.rs` | The history: schema, encryption, cache lookups, listing, deleting |
| `src/key.rs` | The master key: file storage, 24-word form |
| `src/keychain.rs` | The OS credential store, one backend per OS |
| `src/config.rs` | Layered settings and `[providers.NAME]` tables |
| `src/history.rs` | Browsing, searching, deleting, size and age limits |
| `src/context.rs` | Earlier turns from a directory's history, sent as context |
| `src/curate.rs` | `history ask` and `history agent` (a model reads excerpts of the history) |
| `src/agent.rs` | Agent mode: the tool loop and its tools |
| `src/stats.rs` | `stats` output (reads the stats table, never the content) |
| `scripts/install.sh` | The installer, attached to every release |
| `packaging/` | Template for the Homebrew formula |

## Notes for contributors

- All model calls go through `provider.rs`. Adding a provider with a new wire format means a new `Kind` and its request body and stream parser; OpenAI-compatible services need no code.
- History content is compressed, then encrypted per record; metadata stays plain so tools can read statistics without the key. Keep prompt and response text out of the plain columns.
- The cache key is a keyed hash (BLAKE3 with the data key), so the stored value reveals nothing about the call.
- The data key never changes; the master key wraps it. `key rotate` re-wraps, which is why it is instant.
- The schema is versioned with `PRAGMA user_version`. Add a migration before changing it after a release.
- On macOS the optional credential-store key is read through `/usr/bin/security` rather than a keychain library: macOS ties "Always Allow" to the calling binary, so a library would prompt again after every upgrade, while `security` never changes.
- Bookkeeping (recording the turn, updating statistics) happens after the output is written and flushed, so a pipeline downstream never waits for it.

## Releases

Commits follow [Conventional Commits](https://www.conventionalcommits.org/); release-please keeps a release pull request open and merging it tags a release. The tag triggers `.github/workflows/release.yml`, which builds on native runners for macOS and Linux (x86-64 and arm64), attaches the tarballs, `checksums.txt` and `install.sh` to the GitHub release, updates the formula in `drumandbytes/homebrew-tap` and smoke-tests the published installer on each platform.
