# summary.rs

- `_summary.md`: frontmatter `hash`, then the scope line, a blank line, the gist.
- `input`: what the model sees for a topic: `Topic: <name>`, then one line per subtopic summary (`- subtopic <name>: <scope> | <gist>`) and per valid note (`- note [<confidence>]: <body>`). Invalid notes (`invalid_at`) are left out. Links are not in it, so `link` never dirties a summary.
- Dirty: the sha1 blob id (gix `compute_hash`) of the current input differs from the stored `hash`, or a summary exists with no input (it is removed). Only the topic's own name is in the input, not its path, so moving a subtree dirties only the two parents: the moved summaries stay valid.
- `due`: dirty topics with nothing dirty below them. A nap builds those, reloads, repeats: bottom-up, only dirty nodes.
- `summarise`: one conversation per summary through memory's `Backend` (the Claude Code compactor, `AI_MEMORY_MODEL`). Limits: scope 160 bytes, gist `NODE` (512). Retries in the same conversation while the reply is malformed or over a limit, up to memory's `TRIES`; then the shortest well-formed try. No filler for scale (memory's `SCALE`): the limits are generous and a retry fixes overshoot.
- Prompt (`prompts/summary.txt`): modelled on memory's compactor prompt (name searchable items, shorten by dropping detail, record faithfully), plus keeping each claim's confidence.
