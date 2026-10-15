# chat/view.rs

Design: `ai/research/20261014T002322---chat-design.md`, View and Roll-up. Fold: `/hatimthayyil/code/optchat/optchat.md` §3.2 and `20261008T074205---optchat-cache-revision-review.md` (the corrected due rule).

Branch
- `Branch`: the path from the root to a session's tip, its turns and their chunks as the roll-up cuts them (`rollup::branch`), and every line of the thread (`Snapshot::all_lines`), loaded once into a map. A session with no item yet uses its graft as tip.
- Fold coordinates are memory's `Coord` over turn positions along the branch: `(l, i)` covers turns `[i·2^l, (i+1)·2^l)`, its line is key `(1 + l, last item of its last turn)`, the contract with the roll-up. The fold covers the closed turns, all but the last.

Fold
- Computed fresh at every call: start from one line per closed turn, merge the most due aligned pair, due = (T − last)/2^l with `last` the pair's last turn, oldest first on ties, until the lines fit. No high/low-water batches and no persisted view: the view is sent once per compaction, so its prefix never has to stay stable from one call to the next; only the merge order matters.
- `plan` reckons an unbuilt line at `LINE` (512) bytes plus its id: the fold the view wants if everything were built. `needed` is its unbuilt lines; `compact` asks the roll-up to build them before rendering.
- `render`: each planned line if built; else what is built under it (a node's halves, recursively; a turn's message cut to `LINE`, its chunk lines, a placeholder for the rest); else one placeholder naming its id. Fallbacks can outgrow the plan, so while over budget the oldest multi-line fallbacks collapse back to one placeholder each. A placeholder is shorter than the plan's `LINE` estimate, so the collapsed fold always fits.

Ids
- Lines and items print by id (id.rs.md): a node `t+n` from its fold coordinate, a turn line `t`, a chunk or `not shown` span `t.i+n` from the place of its first item, an item `t.i`. Ids are read on the session's branch, so zoom loads the session's branch, not the branch to an item.

Chunks
- A turn's chunks are the roll-up's own cut (`Part::chunks`, every chunk, the open last one included): the view shows the built chunk lines consecutive from the first, each with its exact range, so the compactor may write them in any order. A chunk line after an unbuilt one is left out until the gap is built.

Sections and budgets
- `VIEW` 128,000 bytes in all. The instruction, the user rail (`RAIL` 8,000) and the open turn (`OPEN` 32,000) are rendered first; the fold gets what is left, at least ~86 KB.
- User rail: the user's messages that open closed turns, newest first while they fit, shown oldest first; the newest always, cut to `RAIL`. The open turn's message is in its own section, so it is not repeated. Work items (other agents) are not the user's words.
- Open turn: its message (cut to 8,000), then its chunk lines and the steps after the last of them, the newest that fit in `OPEN`, whole steps; a step too big alone splits into items so the newest item always shows. What is left out is one `t.i+n|(not shown)` line. On auto compaction the prompt that triggered it is this message (register.ts.md).
- Item lines: `render::item_line`, the one renderer the compactor's input uses too (render.rs.md). Fold fallbacks cut words to 512 bytes; the open turn to 8,000, tool input and output to 2,000: it is the agent's working context when an auto compaction lands mid-task, so recent output stays.
- The instruction says a summary cites the item holding a detail as `[12.3]`, as the compactor's prompt asks.

Zoom (`Branch::zoom`)
- A node opens two levels down, as `ai memory zoom` does: four lines of `n/4` turns; `t+2` its two turn lines (below a turn line are its items, not lines). A turn opens into its message, chunk lines and items; items `t.i+n` into the chunk lines strictly inside them and their other items; one item prints whole. A line not built shows what is built under it, as in the fold.
- Any range zooms: `5+3` is no node, so it prints the fewest aligned nodes that tile it (`tile`: `5`, `6+2`).
- Listings print messages and replies whole, each tool call on one line cut to 400 bytes, and tool output as its size only; `--tool-output` prints calls and output whole. Normal use never pulls raw output into the context by accident; it stays one flag away.
