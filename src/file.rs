use std::{
    fs::{File, TryLockError},
    io,
    path::Path,
};

use anyhow::{Result, anyhow};

/// A path as the user would type it: fold `$HOME` to `~`.
pub fn pretty(p: &Path) -> String {
    let p = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    match std::env::home_dir().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

pub trait AtPath<T> {
    fn at(self, p: &Path) -> Result<T>;
}

impl<T> AtPath<T> for io::Result<T> {
    fn at(self, p: &Path) -> Result<T> {
        self.map_err(|e| {
            let msg = e.to_string();
            let reason = msg.split(" (os error").next().unwrap_or(&msg);
            anyhow!("{}: {reason}.", pretty(p))
        })
    }
}

/// The lock on the file `p`; `None` while another process holds it.
pub fn try_lock(p: &Path) -> Result<Option<File>> {
    let f = File::create(p).at(p)?;
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e).at(p),
    }
}

/// The lock on the file `p`, waiting for it.
pub fn lock(p: &Path) -> Result<File> {
    let f = File::create(p).at(p)?;
    f.lock().at(p)?;
    Ok(f)
}
