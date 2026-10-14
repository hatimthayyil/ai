# chat/mod.rs

`ai chat`, design `ai/research/20261014T002322---chat-design.md`. `model` is the harness-neutral data model; one module per harness adapter (`claude`); `store` keeps threads in the repository's git, `record` encodes their records, `sync` binds sessions and ingests them. Re-exports the model and the store API so callers write `chat::Item`, `chat::Chat`.
