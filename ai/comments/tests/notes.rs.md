# tests/notes.rs

In-process tests of `ai notes` (`Cli::run_with` with a fake `Runtime`). `Fake` answers `scope of <topic>` and a gist counting the items it was given, and records each first message (order, and what the model sees) and each retry; `long` makes the first reply's scope overlong. Tests check the commit log and that the work tree stays clean, through the `git` binary.
