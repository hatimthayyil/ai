# chat/mod.rs

`ai chat`, design `ai/research/20261014T002322---chat-design.md`. `model` is the harness-neutral data model; one module per harness adapter (`claude`). Re-exports the model so callers write `chat::Item`.
