# chat/summarise.rs

The summariser: builds the lines `rollup.rs` plans. Design: `ai/research/20261014T002322---chat-design.md`, Roll-up (background compactor) and Contract.

Config (`Chat::config`)
- From the repository's git config, so global (`~/.gitconfig`) or per repo: `ai.chat.model` (default `memory::claude::model()`: `$AI_MEMORY_MODEL`, else Sonnet, as memory's compactor), `ai.chat.summarise` (bool, default true). A value that is not a boolean is an error. Memory's knobs are numbers in the memory repo's config; chat's live with the repository the threads are in.
- `backend(model)`: `ClaudeCode::compactor_at` with `model` (`summarise --model`), else the configured one (medium effort, isolated `claude -p`).

Runs (`Run`)
- One pass reads the latest snapshot: items (`Rollup::new`) and every built line (`all_lines`). Pending lines are ordered by (last item, level): oldest first, a line before the node above it that ends at the same item.
- Each round first writes every ready free line in one commit (`put_lines`), repeating while new ones become ready, then starts the ready calls up to `memory::JOBS` (8). Each call runs `memory::write_line` under `PROMPT` on its own thread (same dispatcher and span as memory's pump; a panic becomes a failure so a pass never waits forever) and reports on a channel; each finished call is one commit (`put_line`, first write wins).
- A failed call leaves its line unbuilt and is not retried in that run; lines above it wait. After `JOBS` failures in a row no call starts: a dead backend costs at most about two waves of calls per run (test: 8–15). The next summariser tries again. Memory's per-node backoff is not needed: a summariser ends instead of waiting, and the next sync starts another.
- Dropping a run with calls in flight calls `Backend::stop`.

`build(thread, backend, keys, deadline)` (for `ai chat compact-session`)
- The keys and the unbuilt lines they are made from; a key the roll-up does not have and that is not built is an error (`<key> is no line of thread <name>.`). Returns at the deadline with what is built; running calls are stopped.
- Under the thread's lock. While a summariser holds it, waits for it until the deadline (`Thread::lock_before`, tried every 50 ms; logs `busy: waiting`), then builds only what the summariser left unbuilt: no line costs two calls. Still held at the deadline: builds nothing and logs it; the view shows what is built. Unlocked before dropped, as a summariser.
- A sync's summariser that starts while a build holds the lock exits `busy`; lines the view did not need wait for the next sync.
- Logs to the thread's log like a summariser, under a `build` span, so a compaction's failed lines are on record.

`summarise(thread, backend)` (for `ai chat summarise -t <name>`)
- Logs to `Thread::log_path` (`log::init`; INFO `start`, `built` (line, bytes, secs), `end`; WARN `failed`; `busy`).
- Under the thread's lock (`try_lock`; `None` at once when held): passes until one starts nothing, then releases the lock and compares the head with the last pass's commit. Moved (a sync landed meanwhile): take the lock again and go on, or exit if another summariser has it. This closes the window in which a sync's summariser finds the lock held just before this one exits.
- The lock is `unlock`ed before it is dropped: a process forked by another thread between our open and close (a `claude -p` call, a test's `git`) holds a copy of the descriptor until its exec, and the flock stays with it until then.

`rebuild(thread, backend, deadline)` (for `ai chat summarise --rebuild`)
- Under the thread's lock, waiting for it until the deadline (`None` if still held: nothing dropped): drops every line in one commit (`Thread::drop_lines`), then passes as `summarise` does until one starts nothing. Dropping first keeps one rule for the store (a line is written once, first write wins): a rewrite is a fresh build. Lines that fail stay unbuilt until the next summariser.
- Rewriting one line and the lines above it (`summarise <id> --rebuild`) needs the turn ids of the view; for now every line.

`dry_run(thread, backend, rebuild)` (for `--dry-run`)
- One pass with `Run::dry` set: free lines and model replies go to the pass's in-memory map, so lines above them are built from them, and to the run's list instead of the store. With `rebuild` (`Run::fresh`) the pass reads no stored line, so it builds every line anew. No lock: it writes nothing. Each line comes back with its first item (for its printed id) and the stored line it would replace. Logged under a `dry_run` span.

`Chat::spawn_summariser(thread)` (for `ai chat sync-session`)
- Unless `ai.chat.summarise` is false: this executable as `chat summarise -t <name>` from the repository's work tree (else its git dir), detached by `memory::log::spawn` (stdin/stdout null, stderr to the thread's log, own process group). No readiness check: a summariser with nothing to do exits after one pass, and sync stays fast.

Smoke (2026-10-15, haiku, throwaway repo, synthetic 4-turn thread): 3 calls, 1 free turn line, 1 node; the spawning process's piped stdout closed at once (spawn 0.3 ms); a second summariser exited `busy`.
