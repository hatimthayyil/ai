# chat/nap.rs

The roll-up compactor: builds the lines `rollup.rs` plans. Design: `ai/research/20261014T002322---chat-design.md`, Roll-up (background compactor) and Contract.

Config (`Chat::config`)
- From the repository's git config, so global (`~/.gitconfig`) or per repo: `ai.chat.model` (default `memory::claude::model()`: `$AI_MEMORY_MODEL`, else Sonnet, as memory's compactor), `ai.chat.nap` (bool, default true). A value that is not a boolean is an error. Memory's knobs are numbers in the memory repo's config; chat's live with the repository the threads are in.
- `backend`: `ClaudeCode::compactor_at` with the configured model (medium effort, isolated `claude -p`).

Runs (`Run`)
- One pass reads the latest snapshot: items (`Rollup::new`) and every built line (`all_lines`). Pending lines are ordered by (last item, level): oldest first, a line before the node above it that ends at the same item.
- Each round first writes every ready free line in one commit (`put_lines`), repeating while new ones become ready, then starts the ready calls up to `memory::JOBS` (8). Each call runs `memory::write_line` under `PROMPT` on its own thread (same dispatcher and span as memory's pump; a panic becomes a failure so a pass never waits forever) and reports on a channel; each finished call is one commit (`put_line`, first write wins).
- A failed call leaves its line unbuilt and is not retried in that run; lines above it wait. After `JOBS` failures in a row no call starts: a dead backend costs at most about two waves of calls per run (test: 8–15). The next nap tries again. Memory's per-node backoff is not needed: a nap ends instead of waiting, and the next sync starts another.
- Dropping a run with calls in flight calls `Backend::stop`.

`build(thread, backend, keys, deadline)` (for `ai chat compact`)
- The keys and the unbuilt lines they are made from; a key the roll-up does not have and that is not built is an error (`<key> is no line of thread <name>.`). Returns at the deadline with what is built; running calls are stopped.
- Under the thread's lock. While a nap holds it, waits for it until the deadline (`Thread::lock_before`, tried every 50 ms; logs `busy: waiting`), then builds only what the nap left unbuilt: no line costs two calls. Still held at the deadline: builds nothing and logs it; the view shows what is built. Unlocked before dropped, as a nap.
- A sync's nap that starts while a build holds the lock exits `busy`; lines the view did not need wait for the next sync.
- Logs to the thread's log like a nap, under a `build` span, so a compaction's failed lines are on record.

`nap(thread, backend)` (for `ai chat nap --thread <name>`)
- Logs to `Thread::log_path` (`log::init`; INFO `start`, `built` (line, bytes, secs), `end`; WARN `failed`; `busy`).
- Under the thread's lock (`try_lock`; `None` at once when held): passes until one starts nothing, then releases the lock and compares the head with the last pass's commit. Moved (a sync landed meanwhile): take the lock again and go on, or exit if another nap has it. This closes the window in which a sync's nap finds the lock held just before this one exits.
- The lock is `unlock`ed before it is dropped: a process forked by another thread between our open and close (a `claude -p` call, a test's `git`) holds a copy of the descriptor until its exec, and the flock stays with it until then.

`Chat::spawn_nap(thread)` (for `ai chat sync`)
- Unless `ai.chat.nap` is false: this executable as `chat nap --thread <name>` from the repository's work tree (else its git dir), detached by `memory::log::spawn` (stdin/stdout null, stderr to the thread's log, own process group). No readiness check: a nap with nothing to do exits after one pass, and sync stays fast.

Smoke (2026-10-15, haiku, throwaway repo, synthetic 4-exchange thread): 3 calls, 1 free exchange line, 1 node; the spawning process's piped stdout closed at once (spawn 0.3 ms); a second nap exited `busy`.
