# main.rs

Top-level clap dispatcher for memory and chat. Each module owns its CLI and returns an exit code or error. Model/default flags live in `chat`, avoiding protocol logic in the binary.
