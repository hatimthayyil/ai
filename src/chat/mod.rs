pub mod claude;
mod cli;
pub mod model;
mod record;
pub mod render;
pub mod rollup;
mod store;
mod summarise;
mod sync;
mod view;

pub use cli::{Cli, NOT_ATTACHED};
pub use model::{Item, Kind, Marker, SourceId, Tree, Turn, turns};
pub use record::{Agent, Binding, Head, Key, Line, LineKind};
pub use rollup::{Branch, Part, Rollup, Source};
pub use store::{Added, Chat, FORMAT, REFS, Snapshot, Thread, ThreadInfo, Transcript, check_name};
pub use summarise::{Built, Config, Dry, DryLine, build, dry_run, rebuild, summarise};
pub use sync::{Attached, Synced, read, sync};
pub use view::VIEW;
