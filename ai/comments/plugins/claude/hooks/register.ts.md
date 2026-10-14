# plugins/claude/hooks/register.ts

The Claude Code side of `ai chat` (design: `ai/research/20261014T002322---chat-design.md`; mod findings: `20261014T202206---chat-launch-attach.md`). Claude Code 2.1.300; the mods API is early access.

- Load per process: `claude --plugin-dir plugins/claude`, or for every session `CLAUDE_CODE_PLUGIN_DIRS=<abs path>` (environment, or `env` in `~/.claude/settings.json`). `ai` is resolved from the session's `PATH`.
- Plugin name `ai`: the command is `/ai`, the tool `mcp__ai__compress`, the state key `ai.transcript`.
- `/ai <sub>` is one command: `command.run` hands the rest of the line as `e.args`, so no `/ai-compress` fallback is needed.
- Transcript path: `$` has the session id but not the transcript path. `classic.SessionStart` (startup, resume, clear, compact) carries it; it goes to `$.state`, which outlives a module reload. `classic.Stop` carries both, so sync reads them from `e`.
- `session.compact`: exit 0 answers `{ messages: [view as one user message] }` without `next`, so the summarizer never runs (`compact_boundary`, ~20 ms, no model call). Exit 3 and failures call `next(e)`: native compaction; a failure also writes the stderr reason to the transcript (`$.ui.log`, `ui_log` under `-p`). Every trigger (`manual`, `auto`, `plugin`, `precompute`) is handled the same. A subagent's own compaction (`agentId`) stays native: the thread view is the main conversation's.
- Answering without `next` also skips the built-in `cc-plugin-agents-md` hook beneath, which marks nested instruction files for re-attachment after a compaction. After an `ai chat` compaction, nested `CLAUDE.md`/`AGENTS.md` files read before it are not re-attached until something else resets that plugin's state. No mod API lets a user-tier hook run the built-ins but skip core.
- On `auto` the prompt that triggered it is already in the transcript and inside the compacted span: the view must carry the open exchange, or the model sees no question.
- `/compact` from the mod: `$.command.run({ command: 'compact' })` from `$.clock.after(0, …)`. Called inside a `command.run` or `tool.call` hook it is refused (it would compact under the turn the hook holds). `$.session.compact()` skips the calling plugin's own `session.compact` hook, so it would compact natively. A `$.command.run` made during a turn waits for the turn to end; that is how the agent tool queues compaction.
- The agent tool probes with `ai chat sync`: exit 3 means unattached (nothing queued); exit 0 also ingests before the compaction. It is left deferred (behind ToolSearch) so unattached sessions carry no schema.
- Turn-end sync is a mod `classic.Stop` hook, not a `hooks.json` command hook: it has the session id and transcript path without parsing stdin. The `$.process.run` is not awaited, so the hook settles in ~4 ms; a `-p` run waits for it before exiting.
- `$.process.run` reads the whole output until the child exits: `ai chat sync` must detach `nap` from its stdout and stderr, or it holds the call. Its timeout is 30 s by default; `compact` and `attach` get the 10-minute maximum.
- An interactive `--plugin-dir` session writes the engine's type declarations into `.claude-plugin/types/`; `.gitignore` keeps them out.
- Tests (`claude plugin test plugins/claude`) stub `process.run` beneath the plugin; they need no model and no `ai`.
