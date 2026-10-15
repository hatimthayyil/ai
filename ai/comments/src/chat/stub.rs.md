# chat/stub.rs

The roll-up's entry points (agent C of the layer-1 plan), stubbed so the CLI is complete before C's branch lands: `build` returns at once, `spawn_nap` starts nothing, `nap` builds nothing. Each is called once, in `cli.rs`:

- `stub::build(&t, &keys, Instant::now() + WAIT)` in `compact`;
- `stub::spawn_nap(cwd, thread)` in `start_nap`;
- `stub::nap(cwd, name, out)` in `Cli::run`, arm `Command::Nap`.

Wiring C in: point those three calls at C's functions, then delete this file and `mod stub;`.
