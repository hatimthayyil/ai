# ai-tools

The `ai` CLI: an all-in-one AI tool.

- `ai memory`: permanent memory for AI agents. An agent records what it learns as one-line notes. Claude Code folds older notes into summaries, and `ai memory wake` prints the whole memory as one short view.
- `ai notes`: a global knowledge store for AI agents. Each note is one claim, filed in a tree of topics; Claude Code summarises every topic from its children, so an agent can browse the tree or search it.
- `ai chat`: named conversations that outlive harness sessions. A thread copies its Claude Code sessions into the repository's git, summarises them into a zoomable tree, and replaces the session's compaction with a view of the whole thread.

## Install

```sh
cargo install ai-tools
```

or with Nix:

```sh
nix profile add github:hatimthayyil/ai-tools
```

## Use

```sh
ai memory init                  # create the memory; prints the block for your agent instructions
ai memory wake                  # read the memory
ai memory note "what happened"  # record one line
ai memory zoom <id>             # open a line of the view
ai memory grep <regex>          # search every message
```

`ai` stores memories in `~/.ai/memory`, or in `$AI_MEMORY_DIR` if you set it. A newer `ai` upgrades the memory on first use and prints the commit holding its previous state; older releases then fail on it without writing.

```sh
# file a note (or pipe its text on stdin)
ai notes add physics.qm "the claim, in detail" --title "Decoherence explains collapse" --confidence verified
ai notes                        # the top of the tree: each topic's scope
ai notes zoom physics.qm        # a topic's subtopics and notes
ai notes grep <regex>           # search every note and summary
ai notes show <id>              # one note, whole
ai notes mv <id|topic> <topic>  # move a note or a topic
ai notes link <id> <id>         # link one note to another
```

Notes are Markdown files in a git repository at `~/.ai/notes`, or at `$AI_NOTES_DIR` if you set it. Every change is a commit.

### ai chat

```sh
ai chat claude dev [-- claude args]  # Claude Code in a new session of thread dev; an existing thread's view is its context
ai chat resume dev [-- claude args]  # continue dev's newest session
ai chat resume                       # list the threads here
```

Inside a session, `/ai attach <name>`, `/ai detach` and `/ai compress`; the agent reads its thread with `ai chat resume` and `ai chat zoom <id>`. An attached session's `/compact` and auto-compaction replace its context with the thread's view, not Claude Code's summary. Threads live under `refs/ai/chat/` in the repository; `git config ai.chat.model` picks the summariser's model.

This needs the Claude Code plugin in `share/ai/plugins/claude` of the package (`plugins/claude` here). Load it in every session with the `env` block of `~/.claude/settings.json`:

```json
"env": { "CLAUDE_CODE_PLUGIN_DIRS": "/home/<you>/.nix-profile/share/ai/plugins/claude" }
```

or for one session with `claude --plugin-dir <dir>`.

## Credits

`ai memory` ports Victor Taelin's [OptMem](https://github.com/VictorTaelin/OptMem) to Rust. A separate Claude Code model writes its summaries, using the compaction prompt from his [UniiChat](https://gist.github.com/VictorTaelin/91837951a5ce5b38f341ec1ba1df6449) (aka OptChat).

## Licence

[CC0-1.0](LICENSE)
