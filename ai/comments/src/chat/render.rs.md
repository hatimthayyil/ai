# chat/render.rs

One item as one text line, for the view (view.rs) and the compactor's input (rollup.rs): the agent and the compactor read items the same way, and an item's number in a summary (`#n`) is the `n` the view prints and `ai chat zoom n` takes.

- `item_line`: `n|user: …`, `n|work from <sender>: …`, `n|ai: …`, `n|call <tool>: key=value …`, `n|result: …` / `n|error: …`, markers as `n|[…]`. Text keeps its newlines; the view flattens only summary lines.
- Tool input: `key=value` per field (serde_json's sorted keys), strings verbatim, other values as JSON: readable without escapes, and no per-tool table to maintain.
- `Cut`: the bytes kept of an item's words and of a tool input or output; `None` is whole (zoom). Callers: the view (words 8,000 in listings and 512 in fold fallbacks, tools at most 2,000), the compactor (words 24 KB, tools 4 KB).
- `clip`: head and tail around `[... N bytes cut: zoom n ...]`, at char boundaries; the note says where the rest is.
