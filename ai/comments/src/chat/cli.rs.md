# chat/cli.rs

Design: `ai/research/20261014T002322---chat-design.md`, Commands and Contract. Caller: `plugins/claude/hooks/register.ts` (register.ts.md).

Sessions
- Every in-session and internal command takes `--harness` (default `claude`), `--session`, `--transcript`. Flags win: processes the plugin starts do not have `CLAUDE_CODE_SESSION_ID`. Without `--session`, the harness's environment names it (`claude::session_id`).
- Not attached (no session, no repository, no attached binding): exit `NOT_ATTACHED` (3) and print nothing. Other failures: an error, which `main` prints on stderr, exit 1.
- The transcript is looked up (`claude::transcript`) only by `attach`. `sync` and `compact` pass the given path, else the binding's: `attach` and the launcher record it.

Terminal
- `claude <name> [-- args]`: a version-4 UUID (getrandom), bound to `name` with its future transcript `<config>/projects/<project_dir(cwd)>/<uuid>.jsonl` (the file does not exist yet, so nothing is ingested; the graft is the thread's newest item), then `exec claude --session-id <uuid> args…`. Seeding a fresh session with the view is the plugin's.
- `resume <name>`: the thread's newest Claude Code binding, by tip then attach time. Resumed only if its transcript exists in the project directory of the current directory, the only one `claude --resume` searches: re-attached (catching up), then `exec claude --resume <id>`. Otherwise one line on stderr says why, and it launches a new session as above. No guard against running inside a session: Claude Code refuses a nested launch itself.
- `resume` with neither name nor session lists the threads: name, harness and UTC minute of the newest item, branch count.

In session
- `resume`: the attached thread's view, paged by `PART` (20,000 bytes, under the 30,000-character cap on a Claude Code command's output) with memory's `page`. Every part comes from one commit of the thread: the footer names it (`--at <commit>`), because the background compactor writes lines between parts and a fresh fold would shift the page boundaries.
- `zoom <id> [part]`: `view::zoom`, paged by part number, not pinned: what it shows is mostly items, which never change.
- `attach`/`detach`: one-line confirmations. `attach` reports a new thread and the thread it moved from.

Internal
- `sync`: no output; when items were added, starts the background compactor.
- `compact`: sync, the fold's unbuilt lines built by the roll-up until `WAIT` (60 s), then the view on stdout, whole (the plugin sends it as one message). A build error fails the command, so the plugin compacts natively and shows the reason.
- `nap --thread <name>`: the roll-up's entry point.
- `AI_CHAT_NAP=0` stops `sync` and `attach` from starting the background compactor (tests set it, as `AI_MEMORY_NAP`).

The roll-up's functions are stubs until its branch lands (stub.rs.md).
