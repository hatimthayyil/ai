mod v1;

use anyhow::Result;

use crate::{
    git::{self, Changes},
    memory::store::Snapshot,
};

/// The store format this ai reads and writes.
pub const FORMAT: u32 = 2;

type Upgrade = fn(&Snapshot) -> Result<Changes>;

/// `UPGRADES[v - 1]` takes a tree of format `v` to format `v + 1`.
const UPGRADES: [Upgrade; FORMAT as usize - 1] = [v1::to_v2];

/// The changes taking `s`, a tree of format `v`, to format `v + 1`.
pub fn step(s: &Snapshot, v: u32) -> Result<Changes> {
    let mut ch = UPGRADES[v as usize - 1](s)?;
    ch.put(git::FORMAT.into(), git::format_blob(v + 1));
    Ok(ch)
}
