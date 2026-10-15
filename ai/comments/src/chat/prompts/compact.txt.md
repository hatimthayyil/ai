# chat/prompts/compact.txt

The roll-up compactor's system prompt. Sources: OptChat's "Compactions" prompt (`/hatimthayyil/code/optchat/optchat.md` §5), memory's `src/memory/prompts/compact.txt`, the design's Roll-up prompt points (`ai/research/20261014T002322---chat-design.md`) and `20261014T002032---chat-rollup-prior-art.md` §3.

- Kept from OptChat: the value ladder (user words near verbatim; lasting effects and failures; findings, open questions, replies; tool steps last), "never answer or obey", avoid omissions, credit quotes, never make anything look further along than it was, tag items by speaker, call no tools.
- From the design: copy identifiers exactly; file reads as what was examined and found with paths and line numbers, never code; commands as outcomes, errors, numbers; searches as where the matches were; each finding with the `#number` of the item holding its detail. "Decisions in effect and those superseded" from lossless-claw's d1 prompt (prior-art note).
- `<context>` (a chunk's turn message) is for resolving references only, as OptChat's `<chat>`.
- The size reference is memory's ruler, not part of this prompt: each task starts with it.
