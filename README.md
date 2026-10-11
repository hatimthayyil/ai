# ai-tools

The `ai` CLI: an all-in-one AI tool.

- `ai memory`: permanent memory for AI agents. An agent records what it learns as one-line notes. Claude Code folds older notes into summaries, and `ai memory wake` prints the whole memory as one short view.

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
ai memory zoom <id+n>           # open a line of the view
ai memory grep <regex>          # search every message
```

`ai` stores memories in `~/.ai/memory`, or in `$AI_MEMORY_DIR` if you set it.

## Licence

[CC0-1.0](LICENSE)
