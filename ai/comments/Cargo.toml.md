# Cargo.toml

- One package, `ai-tools`, the only crate published to crates.io: a lib target (`src/lib.rs`, modules `memory`, `notes` and `chat`) and the `ai` bin. No workspace, no path dependencies, no cargo features.
- `include`: the published package holds only `src` (with the prompts), `tests` (with fixtures), `LICENSE` and the cargo files.
- `license = "CC0-1.0"`; `LICENSE` is the CC0 1.0 Universal legal code verbatim.
- `[profile.dev.package."*"] opt-level = 3`: dependencies build optimized in dev/test. gix in debug (sha1dc, zlib-rs, odb) made the in-process test suite ~5x slower (54 s vs 10 s). Our crate stays unoptimized for debugging.

Memory:
- `gix`: `default-features = false`, `sha1` and `anyhow`. No network: sync is gone. `index`: notes rewrite the index of their work tree after each commit, so `git status` stays clean.
- `serde_json`: stream-json with Claude Code. `tempfile`: the backend's system-prompt files.
- `getrandom`: the store's origin.
- `libc`: Linux child process groups and parent-death signals.
- `tracing`, `tracing-subscriber` (`fmt`, `env-filter`, `std`; no `ansi`, no proc-macro attributes): the nap log, see `src/memory/log.rs.md`.
- `backon` (`default-features = false`, `std`): retry backoff for the compactor, used as an iterator of delays; no Tokio or timers pulled in. Its async `Retryable` (feature `tokio-sleep`) is the path if the compactor moves to Tokio. `backoff` is unmaintained (RUSTSEC-2025-0012, which points to backon). See ai/research/*rust-retry-crates.md.
- `serde` (`derive`): the usage log record, written and read as JSON Lines with `serde_json`.

Notes: reuse memory's backend (Claude Code compactor), retry and job limits, grep pattern, nap spawning and log; gix (with `index`) for the repo, jiff for ids, regex for `[[id]]` links, tempfile for atomic repo creation. No YAML crate: frontmatter is `key: value` lines.

Chat: serde_json for transcripts and tool inputs, jiff for item times, memchr to cut a file at its last newline, tempfile in tests.
