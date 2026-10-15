# log.rs

- Structured nap log: `tracing` + `tracing-subscriber` fmt layer, no ANSI (forced off: feature unification with other crates could turn it on), default `SystemTime` timer = UTC RFC 3339 with microseconds, no extra time crate. Filter `$AI_MEMORY_LOG` (EnvFilter syntax), default `info`. No rotation; JSON would be `.json()` on the same builder (needs the `json` feature).
- Event targets are module paths (`ai_tools::memory::compact`), which `$AI_MEMORY_LOG` directives match.
- `init` uses `set_default` (thread-scoped guard), not a global subscriber: tests run many naps in one process, each against its own temp store, and a nested nap (lock-race test) restores the outer one on drop. Worker threads get the dispatcher and the nap span from `Compactor::pump`.
- `open`, `init` take the log's path: memory and notes use `nap.log` in their dir, a chat thread `<name>.log` beside its lock.
- `spawn`: the one way a nap starts in the background (memory's and notes' `spawn_nap`, chat's `Chat::spawn_nap`): this executable with the caller's arguments, stdin and stdout null, stderr appended to the log (so panics land there), own process group. The child holds none of the caller's pipes, so a caller whose output is being read (the Claude plugin waiting on `ai chat sync`) sees it close at once.
