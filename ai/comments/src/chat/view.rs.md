# chat/view.rs

Design: `ai/research/20261014T002322---chat-design.md`, View and Roll-up. Fold: `/hatimthayyil/code/optchat/optchat.md` §3.2 and `20261008T074205---optchat-cache-revision-review.md` (the corrected due rule).

Branch
- `Branch`: the path from the root to a session's tip, its exchanges and their chunks as the roll-up cuts them (`rollup::branch`), and every line of the thread (`Snapshot::all_lines`), loaded once into a map. A session with no item yet uses its graft as tip.
- Fold coordinates are memory's `Coord` over exchange positions along the branch: `(l, i)` covers exchanges `[i·2^l, (i+1)·2^l)`, its line is key `(1 + l, last item of its last exchange)`, the contract with the roll-up. The fold covers the closed exchanges, all but the last.

Fold
- Computed fresh at every call: start from one line per closed exchange, merge the most due aligned pair, due = (T − last)/2^l with `last` the pair's last exchange, oldest first on ties, until the lines fit. No high/low-water batches and no persisted view: the view is sent once per compaction, so its prefix never has to stay stable from one call to the next; only the merge order matters.
- `plan` reckons an unbuilt line at `LINE` (512) bytes plus its id: the fold the view wants if everything were built. `needed` is its unbuilt lines; `compact` asks the roll-up to build them before rendering.
- `render`: each planned line if built; else what is built under it (a node's halves, recursively; an exchange's message cut to `LINE`, its chunk lines, a placeholder for the rest); else one placeholder naming its id. Fallbacks can outgrow the plan, so while over budget the oldest multi-line fallbacks collapse back to one placeholder each. A placeholder is shorter than the plan's `LINE` estimate, so the collapsed fold always fits.

Ids
- A line is named by the items it covers, `a-b`, along the branch to `b`; an item by its number, `n`. `b`'s ancestors fix the path, so an id names one range on any branch, needs no session to resolve, and shares its numbers with the items in the view. The level is implied: a whole exchange, an aligned power-of-two run of exchanges, or part of one exchange.
- An exchange of one item prints as `n`, like the item, and zooming it shows the item, which is all it holds.

Chunks
- An exchange's chunks are the roll-up's own cut (`Part::chunks`, every chunk, the open last one included): the view shows the built chunk lines consecutive from the first, each with its exact range, so the compactor may write them in any order. A chunk line after an unbuilt one is left out until the gap is built. On a path cut at a zoomed id's last item the exchange there is open, but its last chunk still shows if built: a built chunk line names the same range on any path through it (rollup.rs.md).

Sections and budgets
- `VIEW` 128,000 bytes in all. The instruction, the user rail (`RAIL` 8,000) and the open exchange (`OPEN` 32,000) are rendered first; the fold gets what is left, at least ~86 KB.
- User rail: the user's messages that open closed exchanges, newest first while they fit, shown oldest first; the newest always, cut to `RAIL`. The open exchange's message is in its own section, so it is not repeated. Work items (other agents) are not the user's words.
- Open exchange: its message (cut to 8,000), then its chunk lines and the steps after the last of them, the newest that fit in `OPEN`, whole steps; a step too big alone splits into items so the newest item always shows. What is left out is one `(not shown: zoom a-b)` line. On auto compaction the prompt that triggered it is this message (register.ts.md).
- Item lines: `render::item_line`, the one renderer the compactor's input uses too (render.rs.md). In listings, words are cut to 8,000 bytes (fold fallbacks: 512) and tool input or output to 2,000. Zooming an item prints it whole.
- The instruction tells the agent that `#n` in a summary names item n: the compactor's prompt asks each finding to cite the item holding its detail that way.

Zoom
- One level down: a node into its two halves (with the same fallback as the fold); a whole exchange into its message, chunk lines and items; part of an exchange (a chunk, or a `not shown` span) into the chunk lines strictly inside it and its other items; an item whole. Memory's two-levels-down zoom does not fit here: under a node over two exchanges the next level is chunk lines, which most exchanges lack.
- `zoom` loads the branch to the id's last item; a single item reads only that item.
