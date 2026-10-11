# log.rs

- Structured nap log: `tracing` + `tracing-subscriber` fmt layer, no ANSI (forced off: feature unification with other crates could turn it on), default `SystemTime` timer = UTC RFC 3339 with microseconds, no extra time crate. Filter `$AI_MEMORY_LOG` (EnvFilter syntax), default `info`. No rotation; JSON would be `.json()` on the same builder (needs the `json` feature).
- Event targets are module paths (`ai_tools::memory::compact`), which `$AI_MEMORY_LOG` directives match.
- `init` uses `set_default` (thread-scoped guard), not a global subscriber: tests run many naps in one process, each against its own temp store, and a nested nap (lock-race test) restores the outer one on drop. Worker threads get the dispatcher and the nap span from `Compactor::pump`.
- `open`: shared by `init` and `spawn_nap` (the child stderr, so panics still land in the log).
