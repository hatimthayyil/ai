use std::{io::Write, path::Path, process::ExitCode, time::Instant};

use anyhow::Result;

use crate::chat::{record::Key, store::Thread};

/// Builds the lines `keys` name, and the lines they are made from, until `deadline`.
pub fn build(_thread: &Thread, _keys: &[Key], _deadline: Instant) -> Result<()> {
    Ok(())
}

/// Starts `ai chat nap --thread <thread>` in the repository at `dir`, detached from this
/// process and its output.
pub fn spawn_nap(_dir: &Path, _thread: &str) -> Result<()> {
    Ok(())
}

/// `ai chat nap`: builds every line of `thread` that can be built now.
pub fn nap(_dir: &Path, _thread: &str, _out: &mut dyn Write) -> Result<ExitCode> {
    Ok(ExitCode::SUCCESS)
}
