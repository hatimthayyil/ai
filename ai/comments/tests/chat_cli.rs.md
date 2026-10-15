# tests/chat_cli.rs

`ai chat` as a binary. `Sandbox`: a temporary directory (canonical, as `getcwd` gives it) holding a repository (`gix::init`), a Claude Code config dir (`CLAUDE_CONFIG_DIR`, so transcripts go to `projects/<project dir of the repo>/`), `HOME`, and a fake `claude` first on `PATH` that writes its arguments to `out/argv`, installed as in tests/cli.rs. `GIT_CEILING_DIRECTORIES` stops repository discovery at the sandbox; `CLAUDE_CODE_SESSION_ID` is removed and set only by `ai_in`, as a session's Bash tool has it; `AI_CHAT_NAP=0`. The real Claude Code never starts.

Lines are written through the library (`Thread::put_line`), keyed by `exchange_ends`, as the roll-up would.

- Exit 3 and no output for every in-session and internal command of an unattached session, by flags or environment, and outside a repository.
- Attach (transcript found from the session id), sync by flags, re-attach, moving threads, detach; errors without a session, with a bad name, outside a repository.
- Compact syncs first, then prints the instruction, the fold (a built line, placeholders), the rail and the open exchange.
- Compact of a session whose harness cannot be read: exit 1, the reason on stderr, nothing on stdout.
- Listing; the view in three parts whose footers chain, joined equal to `compact`'s output, although a line is built between parts.
- Zoom of a node, a node with an unbuilt half, an exchange, an item, a long item in two parts; bad ids.
- Launcher: argv, binding with the future transcript path, graft of a second session.
- Resume: unknown thread; the newest session resumed and re-attached; from another directory and with the transcript gone, a new session with the reason on stderr.
