# chat/store.rs

Design: `ai/research/20261014T002322---chat-design.md` (Threads and sessions, Data model, Roll-up keys, Storage). Git plumbing: `src/git.rs` (git.rs.md), shared with `ai memory`.

Where
- One ref per thread, `refs/ai/chat/<name>`, in the repository found from a directory (`gix::discover`). Refs other than `HEAD` live in the common git dir, so every worktree of a repository sees the same threads; nothing pushes them unless asked. The committer is the user's git identity, else `ai chat <ai@localhost>`.
- Names: letters, digits, `-`, `_`, starting with a letter or digit: a safe ref component and lock file name. Harness, session and agent ids become tree path names: letters, digits, `.`, `_`, `-`, not starting with `.` (record.rs, `check_id`).
- Compactor lock per thread: `<common git dir>/ai/chat/<name>.lock` (`file::try_lock`), outside `refs/` so git never mistakes it for a ref lock.

Layout of a thread's tree (format 1)
- `format`: `1`, written by the first commit (`Ref::mutate`); a newer thread is refused before any write ("This thread is format N; this ai reads up to 1. Upgrade ai.").
- `index/<fan s>`: segment `s` holds items `[256s, 256s+256)`: the column row `ts harness session entry parent`, then one line per item (record.rs `Head`). Dense: positions are sequence numbers. `parent` is a sequence number, empty for a root.
- `items/<fan i>`: item `i`'s normalised fields: a named-header record of its kind and the kind's fields, its content as the body (record.rs).
- `raw/<fan i>`: item `i`'s source line, verbatim. Several items from one entry share a line; git stores the blob once.
- `sessions/<harness>/<session>`: a binding (record.rs).
- `agents/<harness>/<agent>/`: a subagent: `agent` (type, description, `call`: the sequence number of the Agent tool call that started it) and its own `index/`, `items/`, `raw/`, numbered from 0. Its roots have no parent; `call` links it to the main tree.
- `lines/<level>/<fan last>`: a summary line keyed by level and the last item it covers (record.rs `Line`).
- `<fan n>` = `{n>>8:04x}/{n&255:02x}` (git.rs `fan_path`).

Why this split
- Sync must know every entry the thread holds (dedupe) and resolve parents; the index gives both from one small blob per 256 items (3,669 items: 15 blobs, 2.7 ms). Content and raw lines are variable size (tool output, base64 images: 13 MB of raw lines for 3,669 items), so each is its own blob: an appended item never rewrites an older one, and a sync reads none of them.
- The index holds no content and the item blob no structure, so nothing is stored twice. `heads` is the cheap structural read (listing, tips, sync); `items`/`tree` read every blob (154 ms for 3,669 items, release), for the roll-up and the view.
- Subagents get their own numbering so the main tree's positions stay its sequence numbers: `Snapshot::tree` is `model::Tree` over the main items, and a subagent's items never show as a branch or tip of the conversation.
- Lines are one blob each: written once, never rewritten, few per exchange.

Items
- Append-only. An item's sequence number is its position; its parent is an earlier number, so the tree has no cycles and `model::Tree` rebuilt from `items` gives back the stored links (each stored item's `parent` is the source id of its stored parent).
- Dedupe: by harness and entry id, not session: Claude Code copies entries under a new session id when it forks, so a fork attached to the thread adds only its new entries, hung on the copies already there. Values in the index have whitespace as `_` (`word`); keys are compared in that form.
- `ingest`: every item of the transcript the sequence lacks, in transcript order; its parent is its source parent's number, and a root (or an unknown parent) hangs on the binding's `graft`. The adapter gives a longer file the same items plus new ones (claude.rs.md), so re-reading the whole file and keeping the unknown entries is the whole sync. `tip` moves to the item of the transcript's last entry.
- Subagents: each `Subagent` from the adapter goes to its own sequence, deduped the same way; `call` is found by the Agent tool call id among the session's items. The agent record is rewritten only when it changes (a call found later).

Lines
- `put_line`: first write wins and returns the stored line; a key past the last item is refused. Kind (`chunk`, `exchange`, `node`) and who wrote it (`memory::Who`: agent, model, session) are stored with the text; the level numbering is the roll-up's.

Queries
- `Chat::list`: per thread its newest item (harness, session, time) and tips, from the index only. `Snapshot::tree` and `model::Tree::path`/`branch` give the item tree and the path to a tip; `Snapshot::tip(harness, session)` the newest tip of a session; `Snapshot::line`/`lines` lines by key and by level.
- `Thread::snapshot_at`: the thread as of one of its commits (`Snapshot::as_ref().commit()`), so the parts of a paged view all read one state.
- A `Snapshot` borrows its `Thread`: bind the thread first (`let t = chat.thread(name)?; let s = t.snapshot()?;`).

Measured (release, read-only on the user's transcripts; counts and timings only)
- Largest transcript, 18 MB, 3,669 items: parse 65-75 ms; attach of its whole history 730 ms (one-time); sync of 13 new items 83 ms; sync with nothing new 70 ms. Sessions with 8-10 subagents (~900-1,000 subagent items): sync 50 ms. An unattached session: 0.3 ms.
- Under the 200 ms bound, so sync re-parses the whole file; no byte offset or parser state is kept.
