use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs::{self, File, TryLockError},
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use gix::{
    ObjectId,
    lock::acquire::Fail,
    objs::tree::EntryKind,
    refs::{
        Target,
        transaction::{PreviousValue, RefEdit},
    },
};

use crate::memory::{
    config::{Config, SECTION, SUBSECTION},
    meta::stamp,
    record::{COLUMNS, Message, Node, Who, now},
    tree::{Coord, free, joined},
    upgrade::{self, FORMAT},
};

pub const ME: &str = "ai memory";
pub const REF: &str = "refs/ai/memory";
pub const LOG: &str = "log";
const SEG: u64 = 256;
const ORIGIN: &str = "origin";
/// The compactor's lock file; an upgrade takes it too.
const LOCK: &str = "compact.lock";

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

/// Entry `n` under `dir`, fanned out 256 per tree.
pub fn fan_path(dir: &str, n: u64) -> String {
    format!("{dir}/{:04x}/{:02x}", n >> 8, n & 255)
}

pub fn level_dir(l: u32) -> String {
    format!("tree/{l}")
}

fn seg_of(c: Coord) -> (String, usize) {
    (fan_path(&level_dir(c.l), c.i / SEG), (c.i % SEG) as usize)
}

/// Applies `edit` to the git config file `p` under its lock.
pub fn edit_config<R>(
    p: &Path,
    edit: impl FnOnce(&mut gix::config::File) -> Result<R>,
) -> Result<R> {
    let wait = Fail::AfterDurationWithBackoff(Duration::from_secs(3));
    let mut lock = gix::lock::File::acquire_to_update_resource(p, wait, None)?;
    let mut file =
        gix::config::File::from_path_no_includes(p.to_path_buf(), gix::config::Source::Local)?;
    let r = edit(&mut file)?;
    file.write_to(&mut lock).at(p)?;
    lock.commit().map_err(|e| e.error).at(p)?;
    Ok(r)
}

/// New blobs by path.
#[derive(Debug, Default)]
pub struct Changes(BTreeMap<String, Vec<u8>>);

impl Changes {
    pub fn put(&mut self, path: String, bytes: Vec<u8>) {
        self.0.insert(path, bytes);
    }
}

/// Nodes a mutation wrote, with their stored text.
pub type Built = Vec<(Coord, String)>;

pub struct Store {
    repo: gix::Repository,
    dir: PathBuf,
    pub cfg: Config,
    origin: String,
}

/// `len` Crockford base-32 digits of `bits`.
fn crockford(bits: u32, len: usize) -> String {
    const ABC: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    (0..len)
        .map(|i| ABC[(bits >> (5 * i) & 31) as usize] as char)
        .collect()
}

impl Store {
    /// Opens an existing memory; only [`Store::create`] makes one.
    pub fn open(dir: &Path) -> Result<Store> {
        match Store::find(dir)? {
            Some(s) => Ok(s),
            None => bail!(
                "No memory at {}.\nTo create one, run: {ME} init\n\
                 To use an existing one, point AI_MEMORY_DIR at it.",
                dir.display()
            ),
        }
    }

    /// Opens the memory at `dir`, creating it if absent; returns whether it was new.
    pub fn create(dir: &Path) -> Result<(Store, bool)> {
        if let Some(s) = Store::find(dir)? {
            return Ok((s, false));
        }
        if fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
            bail!(
                "{} is not a memory, and not empty. Point AI_MEMORY_DIR elsewhere.",
                pretty(dir)
            );
        }
        fs::create_dir_all(dir).at(dir)?;
        gix::init_bare(dir)?;
        Ok((Store::open(dir)?, true))
    }

    fn find(dir: &Path) -> Result<Option<Store>> {
        if !dir.is_dir() {
            return Ok(None);
        }
        let Ok(mut repo) = gix::open(dir) else {
            return Ok(None);
        };
        if !repo.is_bare() {
            return Ok(None);
        }
        repo.committer_or_set_fallback(ME, "ai@localhost")?;
        let cfg = Config::load(&repo.config_snapshot(), &pretty(&dir.join("config")))?;
        let mut s = Store {
            repo,
            dir: dir.to_path_buf(),
            cfg,
            origin: String::new(),
        };
        s.upgrade()?;
        s.origin = s.load_origin()?;
        Ok(Some(s))
    }

    /// Brings the memory to [`FORMAT`] in one commit on its head; refuses a newer one.
    fn upgrade(&self) -> Result<()> {
        let v = self.raw(self.head()?)?.format()?;
        upgrade::readable(v)?;
        if v == FORMAT {
            return Ok(());
        }
        let _lock = match self.try_lock()? {
            Some(l) => l,
            None => {
                eprintln!("Waiting for the running nap to finish, to upgrade this memory.");
                self.lock()?
            }
        };
        loop {
            let head = self.head()?;
            let base = self.raw(head)?;
            let from = base.format()?;
            upgrade::readable(from)?;
            if from == FORMAT {
                return Ok(());
            }
            let mut tree = base.tree;
            for v in from..FORMAT {
                let s = Snapshot {
                    tree,
                    ..self.raw(head)?
                };
                tree = self.write_tree(tree, upgrade::step(&s, v)?)?;
            }
            let msg = format!("upgrade format {from} -> {FORMAT}");
            let commit = self.new_commit(&msg, tree, head)?;
            if self.cas(head, commit, &msg)? {
                let prev = head.map_or_else(String::new, |h| h.to_string());
                eprintln!("Upgraded memory to format {FORMAT}; previous state is commit {prev}.");
                return Ok(());
            }
        }
    }

    /// The compactor's lock on this memory; `None` while another process holds it.
    pub fn try_lock(&self) -> Result<Option<File>> {
        let p = self.dir.join(LOCK);
        let f = File::create(&p).at(&p)?;
        match f.try_lock() {
            Ok(()) => Ok(Some(f)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e).at(&p),
        }
    }

    /// The compactor's lock, waiting for it.
    fn lock(&self) -> Result<File> {
        let p = self.dir.join(LOCK);
        let f = File::create(&p).at(&p)?;
        f.lock().at(&p)?;
        Ok(f)
    }

    /// The origin in this store's config, created by the first open.
    fn load_origin(&self) -> Result<String> {
        let key = format!("{SECTION}.{SUBSECTION}.{ORIGIN}");
        if let Some(o) = self.repo.config_snapshot().string(key.as_str()) {
            return Ok(o.to_string());
        }
        self.edit_config(|f| {
            if let Some(o) = f.string_by(SECTION, Some(SUBSECTION.into()), ORIGIN) {
                return Ok(o.to_string());
            }
            let o = crockford(getrandom::u32()?, 6);
            f.set_raw_value_by(SECTION, Some(SUBSECTION.into()), ORIGIN, o.as_str())?;
            Ok(o)
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// This store's id, stamped on every message and node it writes.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Replaces this store's id in its config.
    pub fn set_origin(&mut self, origin: &str) -> Result<()> {
        if origin.is_empty() || origin.contains(char::is_whitespace) {
            bail!("'{origin}' is not an origin: one word.");
        }
        self.edit_config(|f| {
            f.set_raw_value_by(SECTION, Some(SUBSECTION.into()), ORIGIN, origin)?;
            Ok(())
        })?;
        self.origin = origin.into();
        Ok(())
    }

    pub fn head(&self) -> Result<Option<ObjectId>> {
        Ok(self
            .repo
            .try_find_reference(REF)?
            .and_then(|r| r.try_id().map(|id| id.detach())))
    }

    /// The memory as of the latest commit.
    pub fn snapshot(&self) -> Result<Snapshot<'_>> {
        self.at(self.head()?)
    }

    /// The memory as of `commit`, which must be of [`FORMAT`]; `None` is the empty memory.
    fn at(&self, commit: Option<ObjectId>) -> Result<Snapshot<'_>> {
        let s = self.raw(commit)?;
        let v = s.format()?;
        upgrade::readable(v)?;
        if v < FORMAT {
            bail!(
                "This memory is format {v}, older than this ai's {FORMAT}. Run {ME} wake to upgrade it."
            );
        }
        Ok(s)
    }

    /// The tree of `commit`, of any format.
    fn raw(&self, commit: Option<ObjectId>) -> Result<Snapshot<'_>> {
        let tree = match commit {
            Some(c) => self.repo.find_commit(c)?.tree_id()?.detach(),
            None => ObjectId::empty_tree(self.repo.object_hash()),
        };
        Ok(Snapshot {
            store: self,
            commit,
            tree,
            cache: RefCell::default(),
        })
    }

    /// Applies `change` to the latest snapshot as one commit, redoing it until the ref update wins.
    pub fn mutate<R>(
        &self,
        msg: &str,
        mut change: impl FnMut(&Snapshot) -> Result<(R, Changes)>,
    ) -> Result<(R, Snapshot<'_>)> {
        loop {
            let base = self.snapshot()?;
            let (r, ch) = change(&base)?;
            if ch.0.is_empty() {
                return Ok((r, base));
            }
            let commit = self.commit(&base, ch, msg)?;
            if self.cas(base.commit, commit, msg)? {
                return Ok((r, self.at(Some(commit))?));
            }
        }
    }

    /// `ch` over `base`, the first commit naming the format.
    fn commit(&self, base: &Snapshot, mut ch: Changes, msg: &str) -> Result<ObjectId> {
        if base.commit.is_none() {
            ch.put(upgrade::PATH.into(), upgrade::blob(FORMAT));
        }
        let tree = self.write_tree(base.tree, ch)?;
        self.new_commit(msg, tree, base.commit)
    }

    fn write_tree(&self, base: ObjectId, Changes(ch): Changes) -> Result<ObjectId> {
        let mut ed = self.repo.edit_tree(base)?;
        for (p, bytes) in ch {
            let id = self.repo.write_blob(bytes)?;
            ed.upsert(&p, EntryKind::Blob, id)?;
        }
        Ok(ed.write()?.detach())
    }

    fn new_commit(&self, msg: &str, tree: ObjectId, parent: Option<ObjectId>) -> Result<ObjectId> {
        let me = self.repo.committer().context("No committer.")??;
        Ok(self.repo.new_commit_as(me, me, msg, tree, parent)?.id)
    }

    /// Moves the memory from `from` to `to`; false if another writer moved it first.
    fn cas(&self, from: Option<ObjectId>, to: ObjectId, msg: &str) -> Result<bool> {
        let expected = match from {
            Some(c) => PreviousValue::MustExistAndMatch(Target::Object(c)),
            None => PreviousValue::MustNotExist,
        };
        let edit = RefEdit::update(REF.try_into()?, to, expected, msg);
        match self.repo.edit_reference(edit) {
            Ok(_) => Ok(true),
            Err(_) if self.head()? != from => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn edit_config<R>(&self, edit: impl FnOnce(&mut gix::config::File) -> Result<R>) -> Result<R> {
        edit_config(&self.dir.join("config"), edit)
    }

    pub fn save_config(&self, cfg: &Config) -> Result<()> {
        self.edit_config(|f| cfg.save(f))
    }

    /// Appends `items` in order, with every free node they complete; returns the first new id.
    /// A message without an origin gets this store's, one without a ts the time it lands, never
    /// before the last message's: the log is in time order. Every other field is kept as given.
    pub fn append(&self, msg: &str, items: &[Message]) -> Result<(u64, Built, Snapshot<'_>)> {
        let ((first, built), snap) = self.mutate(msg, |s| {
            let mut ed = Edit::new(s)?;
            let first = ed.t;
            for m in items {
                ed.push(m)?;
            }
            let (built, ch) = ed.finish();
            Ok(((first, built), ch))
        })?;
        Ok((first, built, snap))
    }

    /// Writes node `c` unless it is built, with every free node above it it completes.
    pub fn put_node(&self, c: Coord, who: &Who, text: &str) -> Result<(Built, Snapshot<'_>)> {
        self.mutate(&format!("node {c}"), |s| {
            let mut ed = Edit::new(s)?;
            if c.end() > ed.t {
                bail!("Node {c} is beyond the log: it holds {} messages.", ed.t);
            }
            if ed.text(c)?.is_none() {
                ed.put(c, who, text)?;
            }
            Ok(ed.finish())
        })
    }
}

/// One mutation: new messages and nodes over a snapshot, free nodes completed as they become ready.
struct Edit<'a, 's> {
    snap: &'a Snapshot<'s>,
    t: u64,
    ch: Changes,
    segs: BTreeMap<String, Seg>,
    built: Built,
    last: Option<String>,
}

impl<'a, 's> Edit<'a, 's> {
    fn new(snap: &'a Snapshot<'s>) -> Result<Self> {
        Ok(Edit {
            snap,
            t: snap.log_len()?,
            ch: Changes::default(),
            segs: BTreeMap::new(),
            built: Vec::new(),
            last: None,
        })
    }

    fn seg(&mut self, path: &str) -> Result<&mut Seg> {
        if !self.segs.contains_key(path) {
            let seg = Seg::parse(&self.snap.read(path)?)?;
            self.segs.insert(path.into(), seg);
        }
        Ok(self.segs.get_mut(path).expect("inserted above"))
    }

    fn text(&mut self, c: Coord) -> Result<Option<String>> {
        let (path, k) = seg_of(c);
        Ok(self.seg(&path)?.node(k).map(|n| n.text))
    }

    fn put(&mut self, c: Coord, who: &Who, text: &str) -> Result<()> {
        let node = Node {
            ts: now(),
            origin: self.snap.store.origin.clone(),
            who: who.clone(),
            text: text.into(),
        };
        let (path, k) = seg_of(c);
        let seg = self.seg(&path)?;
        let line = node.encode(&seg.cols);
        let text = Node::decode(&line, &seg.cols)
            .context("An empty node.")?
            .text;
        if seg.lines.len() <= k {
            seg.lines.resize(k + 1, String::new());
        }
        seg.lines[k] = line;
        self.built.push((c, text));
        self.cascade(c)
    }

    fn cascade(&mut self, c: Coord) -> Result<()> {
        let p = c.parent();
        if p.end() > self.t || self.text(p)?.is_some() {
            return Ok(());
        }
        let [a, b] = p.children();
        let (Some(a), Some(b)) = (self.text(a)?, self.text(b)?) else {
            return Ok(());
        };
        if a.is_empty() || b.is_empty() {
            return self.put(p, &Who::default(), &joined(&a, &b));
        }
        match free(joined(&a, &b)) {
            Some(text) => self.put(p, &Who::default(), &text),
            None => Ok(()),
        }
    }

    fn push(&mut self, m: &Message) -> Result<()> {
        if m.text.is_empty() {
            bail!("An empty message.");
        }
        let i = self.t;
        let mut m = m.clone();
        if m.origin.is_empty() {
            m.origin.clone_from(&self.snap.store.origin);
        }
        let last = match self.last.take() {
            Some(ts) => ts,
            None if i == 0 => String::new(),
            None => self.snap.message(i - 1)?.ts,
        };
        if m.ts.is_empty() {
            m.ts = now().max(last);
        } else if m.ts < last {
            bail!(
                "A message of {} cannot follow one of {} (UTC): the log is in time order.",
                stamp(&m.ts),
                stamp(&last)
            );
        }
        self.last = Some(m.ts.clone());
        self.ch.put(fan_path(LOG, i), m.encode());
        self.t += 1;
        match free(m.line()) {
            Some(text) => self.put(Coord::leaf(i), &Who::default(), &text),
            None => Ok(()),
        }
    }

    fn finish(mut self) -> (Built, Changes) {
        for (path, seg) in self.segs {
            if !seg.lines.is_empty() {
                self.ch.put(path, seg.bytes());
            }
        }
        (self.built, self.ch)
    }
}

/// A node segment: a column row, then one line per node, empty where not built.
#[derive(Debug)]
pub struct Seg {
    pub cols: Vec<String>,
    pub lines: Vec<String>,
}

impl Default for Seg {
    fn default() -> Seg {
        Seg {
            cols: COLUMNS.map(String::from).to_vec(),
            lines: Vec::new(),
        }
    }
}

impl Seg {
    /// The segment stored as `b`; none is an empty one under [`COLUMNS`].
    fn parse(b: &[u8]) -> Result<Seg> {
        if b.is_empty() {
            return Ok(Seg::default());
        }
        let corrupt = || anyhow!("A tree segment is corrupt.");
        let s = std::str::from_utf8(b).map_err(|_| corrupt())?;
        let mut lines = s.strip_suffix('\n').unwrap_or(s).split('\n');
        let cols: Vec<String> = lines
            .next()
            .unwrap_or("")
            .split(' ')
            .map(String::from)
            .collect();
        if cols.last().is_none_or(|c| c != "text") || !cols.iter().any(|c| c == "ts") {
            return Err(corrupt());
        }
        Ok(Seg {
            cols,
            lines: lines.map(String::from).collect(),
        })
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut s = self.cols.join(" ");
        for l in &self.lines {
            s.push('\n');
            s += l;
        }
        s.push('\n');
        s.into_bytes()
    }

    fn node(&self, k: usize) -> Option<Node> {
        Node::decode(self.lines.get(k)?, &self.cols)
    }
}

/// The memory as of one commit: every read through it sees the same state.
pub struct Snapshot<'s> {
    store: &'s Store,
    commit: Option<ObjectId>,
    tree: ObjectId,
    cache: RefCell<HashMap<String, Rc<[u8]>>>,
}

impl Snapshot<'_> {
    pub fn cfg(&self) -> &Config {
        &self.store.cfg
    }

    /// The format its `format` blob names; the empty memory is of [`FORMAT`].
    fn format(&self) -> Result<u32> {
        match self.commit {
            Some(_) => upgrade::parse(&self.read(upgrade::PATH)?),
            None => Ok(FORMAT),
        }
    }

    fn entry(&self, path: &str) -> Result<Option<ObjectId>> {
        let root = self.store.repo.find_tree(self.tree)?;
        Ok(root.lookup_entry_by_path(path)?.map(|e| e.object_id()))
    }

    pub(crate) fn fetch(&self, id: ObjectId) -> Result<Vec<u8>> {
        Ok(self.store.repo.find_blob(id)?.take_data())
    }

    /// The blob at `path`, empty if absent; cached.
    pub fn read(&self, path: &str) -> Result<Rc<[u8]>> {
        if let Some(b) = self.cache.borrow().get(path) {
            return Ok(b.clone());
        }
        let b: Rc<[u8]> = match self.entry(path)? {
            Some(id) => self.fetch(id)?.into(),
            None => Rc::from([]),
        };
        self.cache.borrow_mut().insert(path.into(), b.clone());
        Ok(b)
    }

    /// A tree's entries by their hex names, in order.
    fn entries(&self, tree: ObjectId) -> Result<Vec<(u64, ObjectId)>> {
        let t = self.store.repo.find_tree(tree)?;
        t.decode()?
            .entries
            .iter()
            .map(|e| {
                let name = e.filename.to_string();
                let n = u64::from_str_radix(&name, 16)
                    .with_context(|| format!("An entry is misnamed: {name}."))?;
                Ok((n, e.oid.to_owned()))
            })
            .collect()
    }

    /// Every blob under the fanned-out `dir`, with its number.
    pub(crate) fn walk(
        &self,
        dir: &str,
        mut each: impl FnMut(u64, ObjectId) -> Result<()>,
    ) -> Result<()> {
        let Some(top) = self.entry(dir)? else {
            return Ok(());
        };
        for (hi, sub) in self.entries(top)? {
            for (lo, id) in self.entries(sub)? {
                each(hi << 8 | lo, id)?;
            }
        }
        Ok(())
    }

    /// How many messages the log holds.
    pub fn log_len(&self) -> Result<u64> {
        let Some(top) = self.entry(LOG)? else {
            return Ok(0);
        };
        let Some(&(hi, sub)) = self.entries(top)?.last() else {
            return Ok(0);
        };
        let Some(&(lo, _)) = self.entries(sub)?.last() else {
            return Ok(0);
        };
        Ok((hi << 8 | lo) + 1)
    }

    pub fn message(&self, i: u64) -> Result<Message> {
        let id = self
            .entry(&fan_path(LOG, i))?
            .with_context(|| format!("Message {i} is missing from the log."))?;
        Message::decode(&self.fetch(id)?).with_context(|| format!("Message {i}"))
    }

    /// Streams messages `[0, end)` in order.
    pub fn scan(&self, end: u64, mut each: impl FnMut(u64, Message) -> Result<()>) -> Result<()> {
        let mut want = 0;
        self.walk(LOG, |i, id| {
            if i >= end {
                return Ok(());
            }
            if i != want {
                bail!("Message {want} is missing from the log.");
            }
            want += 1;
            each(i, Message::decode(&self.fetch(id)?)?)
        })
    }

    /// Node `c`, if built.
    pub fn node(&self, c: Coord) -> Result<Option<Node>> {
        let (path, k) = seg_of(c);
        Ok(Seg::parse(&self.read(&path)?)?.node(k))
    }

    /// Every node of level `l`, by index; `None` where not built.
    pub fn level(&self, l: u32) -> Result<Vec<Option<Node>>> {
        let mut out = Vec::new();
        self.walk(&level_dir(l), |s, id| {
            let seg = Seg::parse(&self.fetch(id)?)?;
            for k in 0..seg.lines.len() {
                let i = (s * SEG) as usize + k;
                if out.len() <= i {
                    out.resize(i + 1, None);
                }
                out[i] = seg.node(k);
            }
            Ok(())
        })?;
        Ok(out)
    }
}
