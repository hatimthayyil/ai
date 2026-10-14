# lib.rs

The lib target exposes `memory`, `notes` and `chat`, so the binary and the integration tests in `tests/` use their public API. Shared by them: `file` (paths, locks), `git` (a store under one ref) and `record` (named headers, column segments).
