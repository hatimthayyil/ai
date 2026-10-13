mod v1;

use anyhow::{Result, bail};

use crate::memory::store::{Changes, Snapshot};

/// The store format this ai reads and writes.
pub const FORMAT: u32 = 2;
/// The blob at the tree root naming the format.
pub const PATH: &str = "format";

type Upgrade = fn(&Snapshot) -> Result<Changes>;

/// `UPGRADES[v - 1]` takes a tree of format `v` to format `v + 1`.
const UPGRADES: [Upgrade; FORMAT as usize - 1] = [v1::to_v2];

/// The format a `format` blob names; none is format 1.
pub fn parse(b: &[u8]) -> Result<u32> {
    if b.is_empty() {
        return Ok(1);
    }
    match std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.trim_end().parse().ok())
    {
        Some(v) if v > 0 => Ok(v),
        _ => bail!("This memory's format blob is corrupt."),
    }
}

/// The `format` blob of format `v`.
pub fn blob(v: u32) -> Vec<u8> {
    format!("{v}\n").into_bytes()
}

/// Refuses a format newer than this ai's.
pub fn readable(v: u32) -> Result<()> {
    if v > FORMAT {
        bail!("This memory is format {v}; this ai reads up to {FORMAT}. Upgrade ai.");
    }
    Ok(())
}

/// The changes taking `s`, a tree of format `v`, to format `v + 1`.
pub fn step(s: &Snapshot, v: u32) -> Result<Changes> {
    let mut ch = UPGRADES[v as usize - 1](s)?;
    ch.put(PATH.into(), blob(v + 1));
    Ok(ch)
}
