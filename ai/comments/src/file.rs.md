# file.rs

Shared by memory, notes and chat.

- `pretty`, `AtPath`: paths as the user types them (`~/…`); I/O errors as `<path>: <strerror>.`.
- `try_lock`, `lock`: advisory locks on a file (`File::try_lock`/`lock`), created if absent and released by the OS when the process dies, so a crash leaves no stale lock. Compactor locks (memory's `compact.lock`, notes' nap lock, a chat thread's lock) and notes' write lock.
