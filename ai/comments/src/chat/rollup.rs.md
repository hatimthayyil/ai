# chat/rollup.rs

Design: `ai/research/20261014T002322---chat-design.md`, Roll-up. Pure: what lines a thread has and what each is made from; nap.rs builds them. Sizes from `20261014T200729---step-compaction-cost.md`; prompt points from `20261014T002032---chat-rollup-prior-art.md` and OptChat's "Compactions" (`/hatimthayyil/code/optchat/optchat.md`).

Rendering
- `render`: one entry per item, `#<seq> <kind>: <content>\n`: `user`, `work from <sender>`, `ai`, `call <name>: key=value …`, `result`/`error`, markers as a bracketed note. The `#seq` is the item's sequence number: the prompt asks each finding to name the item holding its detail, so a line points where to zoom. Memory's compactor hides ids because the model copied them; here copying them is the point.
- Clipping (`clip`, head and tail around `[N bytes cut]`): tool outputs at `CLIP` = 4 KB, each tool-input field at 2 KB and the whole input at 4 KB (so an Edit keeps both old and new strings), messages and replies at `TEXT` = 24 KB (half the input, so no single item fills a chunk). Raw items stay whole in the store.
- Tool inputs are generic `key=value` (serde_json's sorted keys, strings verbatim): no per-tool table to maintain; long fields are what the clip handles.

Chunking (`chunk`)
- Only for an exchange whose rendering (message and steps) is over `INPUT` = 48 KB. Greedy over its steps: whole steps while they fit; a step over the budget starts a chunk and is cut before each tool call the same way; a piece still over it (a tool call with many parallel results) is a chunk alone. The message is not in a chunk: it is each chunk call's `<context>`, clipped to 4 KB.
- The cut before an over-budget step keeps chunks stable as items are appended: a chunk is final once another follows it (test `chunks_but_the_last_stay_as_items_are_appended`). So an open exchange's chunks but the last are built while it runs; the last waits until it is closed.
- Keys: chunk `0@last`. A chunk's range is fixed by the items above `last` except in one case: a fork inside an over-budget step that is itself split into pieces could give two branches different chunks ending at the same item. Claude Code forks at user messages (claude.rs.md), so the key's first writer wins and the case is left.

Lines (`Rollup`)
- `Rollup::new(tree)`: renders every item once, then for every tip (`Tree::tips`) walks its branch (`branch`): `model::exchanges` on the path, chunks per exchange, `Branch::lines` (exchange lines of the closed exchanges) and `nodes` over them. Lines are merged by key across branches (first wins), so a shared prefix gives one record. Subagent items are not rolled up.
- `Source`: `Chunk` (items, message as context), `Exchange` (all its items), `Chunked` (message plus chunk lines: the exchange line of an exchange that has chunks), `Node` (two lines). `children` are the lines it needs.
- `nodes`: aligned pairs from the first closed exchange of the path, level + 1, keyed by the right half's last item. A node over exchange lines is level 2; position 0 is the first exchange from the root, whichever session it came from.
- `free`: a line whose input fits `NODE` (512 B) is that input flattened: no call. Exchange and chunk sizes are summed before any string is built, as the compactor asks this for every pending line after each call.
- `task`: the ruler (memory's 512-byte `SCALE`), the order (`Compress this exchange` / `this part of a long exchange` / `this long exchange, given as its message and the lines of its parts` / `Merge these two adjacent lines`), `<context>` for chunks, `<input>`. Chunk lines in a `Chunked` input are labelled `part #first-#last:`.

For the view (agent D): `Rollup::branch(tree, tip)` gives the exchanges of the current branch with `chunk_keys` and `line`; `nodes(&branch.lines())` the binary nodes; `source(key).children()` what a line opens into.
