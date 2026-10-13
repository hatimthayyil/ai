# cli.rs

- `ai notes` with no subcommand is `view`.
- `Runtime`: backend, `nap_on_change` (unless `AI_NOTES_NAP=0`), stdin. Smaller than memory's: no compactor options or lock seam.
- Summaries run as in memory: a change (`add`, `mv`) commits, then, if a summary is due, spawns a detached `ai notes nap` (memory's `spawn_nap`, logging to `.git/nap.log`) and returns. `link` never dirties a summary, so it spawns nothing.
- `nap`: one nap per store (`.git/ai-notes-nap.lock`, try-lock: exit if held). Rounds: build every due summary at once (up to memory's `JOBS` threads), write each under the write lock only if the topic's input still equals the one summarised (else it is redone next round), one commit `summary <topic>` each. After releasing the lock it re-checks, as memory's nap does, so a change whose nap found the lock taken is still built. A failed call fails the nap; the next change retries.
- `add <topic> [text]`: body from the argument, else (or `-`) stdin. `--confidence` required; `--source`, `--checked`, `--decay` one line each; `--link` ids must exist. One commit `add <topic>`.
- `mv <from> <to>`: an id moves a note into topic `to`; a topic is renamed to `to`, which must not exist (no merge semantics) nor lie inside it. Emptied source topics are pruned. One commit.
- `link <a> <b>`: appends `b` to `a`'s `links`, directional (backlinks are a lint, layer 2). Idempotent: an existing link makes no commit.
- `view`: topic lines `topic (valid notes below): scope`, indented by depth, deepest whole level that fits `VIEW_MAX` (8000 bytes, ~2k tokens); a level that alone overflows is cut, with a count. An unsummarised topic shows memory's `PLACEHOLDER`.
- `zoom <topic>`: the topic's line and gist, each subtopic's line and gist, then its notes as `id [confidence] body` (cut at 300 bytes).
- `grep`: memory's `Pattern` (smart case, `-F`, `-i`, `-s`) over note bodies and summary scope+gist, every hit with its breadcrumb (`a › b: …`, notes `a › b › <id>: …`), excerpts by memory's `excerpt`. Tree order.
- `show <id>`: the file's path, its text verbatim, then each link resolved to `breadcrumb › id: first line`.
- No usage log (memory's `usage.rs`) yet.
