# codex-history

`codex-history` is a companion CLI for Codex. It fuzzy-searches local Codex
session history and opens a terminal UI for browsing transcripts.

It uses the same kind of terminal-native Rust stack as
[`claude-history`](https://github.com/raine/claude-history): `clap` for CLI
flags, `ratatui` + `crossterm` for the TUI, and `serde_json` for JSONL
transcript parsing.

## Features

- Discover Codex sessions from `~/.codex/sessions/YYYY/MM/DD/*.jsonl`
- Use `~/.codex/history.jsonl` prompt text as conversation titles when available
- Fuzzy search across title, session id, cwd, model, user/assistant text, and
  optional tool output
- TUI list view with workspace scope toggle
- Built-in transcript viewer with scrolling, search, and tool/reasoning toggles
- Direct JSONL file rendering
- Resume or fork the selected session with `codex resume` / `codex fork`

## Install

```sh
cargo install --path .
```

Or download a prebuilt binary from the GitHub Releases page when available:

- Linux x86_64: `codex-history-<version>-x86_64-unknown-linux-gnu.tar.gz`
- Linux ARM64: `codex-history-<version>-aarch64-unknown-linux-gnu.tar.gz`
- macOS Intel: `codex-history-<version>-x86_64-apple-darwin.tar.gz`
- macOS Apple Silicon: `codex-history-<version>-aarch64-apple-darwin.tar.gz`
- Windows x86_64: `codex-history-<version>-x86_64-pc-windows-msvc.zip`
- Windows ARM64: `codex-history-<version>-aarch64-pc-windows-msvc.zip`

## Usage

```sh
codex-history
codex-history --query "deploy error"
codex-history --local
codex-history --plain --query "auth" --limit 10
codex-history --show-path --query "session title"
codex-history /path/to/session.jsonl
codex-history /path/to/session.jsonl --show-tools --show-reasoning
codex-history --version
```

Set `CODEX_HOME` or pass `--codex-home` if your Codex data is not under
`~/.codex`.

The version reported by `codex-history --version` is declared in `Cargo.toml`
and should match release tags such as `v0.1.0`.

## Key Bindings

List mode:

| Key | Action |
| --- | --- |
| Type | Update search |
| `↑` / `↓` | Move selection |
| `Ctrl+P` / `Ctrl+N` | Move selection |
| `Page Up` / `Page Down` | Page selection |
| `Enter` | Open conversation |
| `Tab` | Toggle all/local workspace scope |
| `D` / `Delete` | Delete selected session, with confirmation |
| `Ctrl+O` | Select and exit |
| `Ctrl+R` | Resume selected session |
| `Ctrl+F` | Fork selected session |
| `?` | Help |
| `Esc` / `Ctrl+C` | Quit |

Viewer mode:

| Key | Action |
| --- | --- |
| `j` / `k` | Scroll |
| `g` / `G` | Top/bottom |
| `/` | Search in transcript |
| `n` / `N` | Next/previous match |
| `t` | Toggle tool calls/output |
| `T` | Toggle reasoning summaries |
| `D` / `Delete` | Delete current session, with confirmation |
| `Ctrl+R` | Resume session |
| `Ctrl+F` | Fork session |
| `q` / `Esc` | Back to list |

## Release

GitHub Releases are created automatically when a version tag is pushed.

1. Update the version in `Cargo.toml`.
2. Validate locally:

   ```sh
   cargo test
   cargo package
   ```

   If the working tree is intentionally dirty while testing packaging, use
   `cargo package --allow-dirty`.

3. Commit the release changes, then tag and push:

   ```sh
   git tag v0.1.0
   git push origin main --tags
   ```

The release workflow builds and uploads archives for Linux, macOS, and Windows.
To publish the crate to crates.io as well, run:

```sh
cargo publish
```

## License

MIT. This project includes code and/or design derived from
[`claude-history`](https://github.com/raine/claude-history), which is also
licensed under MIT. Copyright (c) 2024 Raine.
