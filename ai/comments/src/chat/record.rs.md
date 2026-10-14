# chat/record.rs

The encodings of a thread's records (layout: store.rs.md). Named headers and column segments are shared with memory (`src/record.rs`): an optional field added later needs no format bump; unknown fields and columns are kept or ignored.

- `Head`: one index line under its segment's column row. Times are jiff `Timestamp`s in RFC 3339 with the source's precision (Claude Code: milliseconds), so items keep their exact time. `ts` is required.
- Item blob (`encode_kind`/`decode_kind`): header `kind` (`user`, `work`, `text`, `call`, `result`, `marker`) and its fields (`from`; `call` and `name`; `call` and `error true`; `marker compaction|summary|interrupt`); body: the text, the tool input as compact JSON, the tool output, or nothing for a marker. Empty fields are left out, so `error` appears only on failed results.
- `Binding`: header `transcript`, `graft`, `tip`, `attached`, `detached`; harness and session are its path. Attached while `detached` is absent: re-attaching drops it and sets `attached` anew, so no separate state field can disagree with the times.
- `Agent`: header `type`, `description`, `call`; harness and agent id are its path.
- `Line`: header `ts kind agent model session`, body the text. `Key` prints `level@last`.
- `check_id`: an id that becomes a tree path name. `word`: whitespace to `_` for index values, which are space-separated.
