# chat/model.rs

Design: `ai/research/20261014T002322---chat-design.md`, Data model.

- Harness-neutral and pure. An adapter (`claude.rs`) turns a session into items; the store, roll-up and view see items only.
- `SourceId` names the entry an item came from. One entry can give several items (a Claude Code tool result with the user's mid-run words appended); the adapter gives each its own entry id.
- `Kind` holds content only. Thinking is dropped: on Opus 5.5 about 93% of thinking blocks are empty signatures. Compaction points, native summaries and interrupts are `Marker`s, never content; their text stays in `raw`.
- `raw` is the item's own line. Dropped entries (attachments, thinking, metadata lines) are not kept: they are most of a transcript's bytes (skill listings, prompt snapshots, re-read files) and repeat every turn.
- `Tree` links a parent only if it comes earlier, as an append-only source and store give. An unknown or later parent makes a root, so malformed input cannot make a cycle. The last tip is the newest item, the end of the active branch.
- `exchanges` splits a path before each user message. Items before the first user message form an exchange without one (a graft, a session that starts mid-run). Steps are cut before each assistant text: tool calls before any text form the first step, and a marker stays in the step it follows. `closed` is positional: a later user message exists on the path.
