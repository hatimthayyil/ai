# Backlog

## Memory

- `ai memory` compactor prompt defects fixed (retry asks for a complete rewrite; `scale.txt` is lorem ipsum): truncation 37→2 (Sonnet), 23→6 (Haiku), no leaks. Haiku 5.5 still distorts merges (`ai/evals/compactor`, 2026-10-08b); Sonnet stays default.
- `ai memory`: test `parallel_processes_lose_nothing` is flaky (timestamp ordering); failed once in 4 runs.
- Multi-machine sync for the memory (rebase-style merge: pushed messages never move), deferred.
- Repo-level memory, alongside the user memory. Deferred. Decided (2026-10-04):
  - Purpose: portable with the repo, shared with collaborators, focus at wake. Inside a repo, repo memory has priority; user memory stays reachable.
  - Routing: every note goes to user memory. Notes that are repo-local and not personal also go to the repo memory, as the same record (same key).
  - Wake: both covers in one `WAKE_LINES` budget, repo first.
  - Sharing: one ref per person (`refs/ai/memory/<person>`) on the project remote; teammates' refs readable.
  - Open: who decides the repo copy (agent `note --repo` vs classifier, below); budget split; how teammates' refs appear (wake, grep/zoom, or not yet); creation and push policy; `<person>` identity; scope prefixes on ids (`u#`, `r#`).
- Classifier ("Jev type model") to pick the notes that also go to repo memory: repo-local and not personal. Deferred until repo memory exists.
- `ai memory` summaries carry `note:`/`user:` tags copied from the leaves (code labels each leaf `note: text`); Sonnet ignores an instruction to drop them. Wasted bytes per item; decide whether leaves should be unlabelled or tags kept only for `user:` words. See ai/evals/compactor/README.md 2026-10-08c.
- Before a public release: version the store format and give `ai memory` an upgrade path (e.g. rebuild old leaves that still carry `note:` tags). Not needed while only the user runs it.
- Before a public release: `ai hooks install` to add the memory reminder hooks (Claude Code, Codex, pi) on a new machine; on this machine the Nix config does it.

## Chat

- `ai chat` layer 1 runs end to end in Claude Code (register.ts.md, End to end). To enable: `CLAUDE_CODE_PLUGIN_DIRS` in the `env` of `~/.claude/settings.json` (README). Open from that run:
  - Haiku overruns the 512-byte line often (6 retries in 16 calls); Sonnet stays the default model.
  - A seed's last section is titled "The exchange in progress", though it is the previous session's last exchange; the seeded model read it as the current request.
  - A native summary is a marker: after `detach`, native `/compact`, `attach`, its text is neither in the view nor in `zoom`.
  - After an `ai chat` compaction, nested `CLAUDE.md`/`AGENTS.md` files are not re-attached (register.ts.md). Not tested: the run had `CLAUDE_CODE_DISABLE_CLAUDE_MDS`.
  - Auto-compaction was not run live (it needs a full context); the plugin tests cover the `auto` trigger.

## Notes

- `ai notes` layer 1 merged (design `ai/research/20261013T222521---notes-design.md`). User not yet on board with the model: a topic is a bare directory and only notes (files) carry content, IDs and links, so `linguistics` itself cannot be a note or a link target. Next: make every node a note (Luhmann: any card can have sub-cards; Dendron: `linguistics.syntax` is itself a note), with the derived summary attached to any node that has children and links to any node by ID. Then layer 2 (curator, supersedes, staleness lint).
