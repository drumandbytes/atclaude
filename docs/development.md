# Development

## Build and test

```bash
cargo build --release
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

The history database is SQLCipher with a vendored OpenSSL, so building needs a C compiler, `make` and Perl. The first build takes a few minutes.

Debug builds read `ATCLAUDE_DB` (database path) and `ATCLAUDE_DB_KEY` (a 64-character hex key or a passphrase) so development and tests never touch your real history or credential store. Release builds ignore both. `ATCLAUDE_CONFIG` points at a throwaway config file in any build.

## Layout

| File | |
| :--- | :--- |
| `src/main.rs` | Command line, the `ask` flow, shell integration scripts |
| `src/llm.rs` | Runs `claude -p` (prompt on stdin), system prompts, tool and permission flags |
| `src/session.rs` | Session resolution and context building |
| `src/config.rs` | Layered settings; every key also reads `ATCLAUDE_<KEY>` |
| `src/db.rs` | SQLCipher database: schema, filters, retention, delete and replace |
| `src/keychain.rs` | The database key in the OS credential store, one backend per OS |
| `src/history.rs` | `--search`, browsing, `rm`, `clean`, `compact` |
| `src/setup.rs` | Probes Bash hooks and caches the rewritten read-only rules |
| `site/` | The website and `install.sh`, deployed to Cloudflare Pages |
| `packaging/` | Template for the Homebrew formula |

## Notes for contributors

- All model calls go through `llm::complete`, which shells out to `claude -p`, so atclaude works with a Claude subscription and needs no API key. Helper calls use `--safe-mode` and no tools.
- On macOS the key is read through `/usr/bin/security` instead of a keychain library: macOS ties "Always Allow" to the calling binary, so a library would prompt again after every upgrade, while `security` never changes.
- A new database key is only generated when no database exists, so a missing key can never orphan existing history.
- The shell integration calls `atclaude id` when the shell starts instead of embedding an id, so cached init output still gives every terminal its own session. `atclaude setup --detach` re-runs itself in the background, so no shell needs its own backgrounding syntax.
- The schema has no migrations yet. Add `PRAGMA user_version` migrations before changing it after a release.

## Releases

Commits follow [Conventional Commits](https://www.conventionalcommits.org/); release-please keeps a release pull request open and merging it tags a release. The tag triggers `.github/workflows/release.yml`, which builds on native runners for macOS and Linux (x86-64 and arm64), attaches the tarballs and `checksums.txt` to the GitHub release, updates the formula in `drumandbytes/homebrew-tap` and smoke-tests the published `install.sh` on each platform. The website in `site/` deploys when it changes on `main`.
