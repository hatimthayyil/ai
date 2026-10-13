# leaf.rs

- `Front`: frontmatter as ordered `key: value` lines, kept verbatim, unknown keys included, so a parse-render round trip changes nothing. No YAML dependency: values are single lines, lists are `[a, b]`. Layer 1 writes `source`, `confidence`, `checked`, `decay`, `links`; it reads `invalid_at` (validity) and `links`; `supersedes` is only preserved (layer 2).
- Ids: Denote `YYYYMMDDThhmmss` in UTC (permanent ids must not depend on the machine's zone), bumped a second past any id in the store; taken under the write lock, so parallel adds get distinct ids. The id lives only in the file name.
- Slug: first six ASCII words of the body's first line, at most 48 bytes, else `note`. Not editable: the id names a note, the slug only helps a human scanning a directory.
- `links()`: frontmatter `links` plus every `[[id]]` in the body (hub notes), once each.
- `LEAF_MAX` = 1024 bytes of body, a constant: memory's `ENTRY_CHARS` knob is per-memory config tied to the view's line size; notes have no view line to fit.
- `Confidence`: the four design values, required on `add`.
