use std::{
    collections::{BTreeSet, HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    process,
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use gix::ObjectId;
use jiff::Timestamp;
use tracing::{Span, dispatcher, info, info_span, warn};

use crate::{
    chat::{
        record::{Key, Line},
        rollup::{PROMPT, Rollup},
        store::{Chat, Thread},
    },
    memory::{Backend, Block, ClaudeCode, JOBS, Who, claude, log, write_line},
};

const MODEL: &str = "ai.chat.model";
const NAP: &str = "ai.chat.nap";

/// The compactor's settings, from the repository's git config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// `ai.chat.model`; default the memory compactor's.
    pub model: String,
    /// `ai.chat.nap`; default true: sync starts the background compactor.
    pub nap: bool,
}

impl Chat {
    pub fn config(&self) -> Result<Config> {
        let c = self.repo().config_snapshot();
        let model = c
            .string(MODEL)
            .map(|m| m.to_string())
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(claude::model);
        let nap = c
            .try_boolean(NAP)
            .with_context(|| format!("{NAP} is true or false"))?
            .unwrap_or(true);
        Ok(Config { model, nap })
    }

    /// The configured model, through the `claude` on `PATH`.
    pub fn backend(&self) -> Result<Arc<dyn Backend>> {
        let model = self.config()?.model;
        Ok(Arc::new(ClaudeCode::compactor_at(claude::find()?, &model)?))
    }

    /// Starts `ai chat nap --thread <name>` in the background, unless `ai.chat.nap` is false;
    /// whether it started.
    pub fn spawn_nap(&self, thread: &Thread) -> Result<bool> {
        if !self.config()?.nap {
            return Ok(false);
        }
        let repo = self.repo();
        let cwd = repo.workdir().unwrap_or(repo.git_dir());
        log::spawn(&thread.log_path()?, |c| {
            c.args(["chat", "nap", "--thread", thread.name()])
                .current_dir(cwd)
        })?;
        Ok(true)
    }
}

/// What a compactor run did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Built {
    /// Lines written, free ones included.
    pub lines: u64,
    /// Lines a model wrote.
    pub calls: u64,
    /// Lines whose call failed, and why.
    pub failed: Vec<(Key, String)>,
}

/// Under the thread's lock, waiting for it until `deadline`, builds `keys` and the unbuilt
/// lines they are made from, up to [`JOBS`] calls at once, until they are built, `deadline`
/// passes, or failures leave nothing to start; logs to [`Thread::log_path`]. Calls still
/// running at the deadline are stopped through `backend`. A key that is no line of the thread
/// is an error.
pub fn build(
    thread: &Thread,
    backend: Arc<dyn Backend>,
    keys: &[Key],
    deadline: Instant,
) -> Result<Built> {
    let _log = log::init(&thread.log_path()?)?;
    let _build = info_span!("build", pid = process::id(), thread = thread.name()).entered();
    let lock = match thread.try_lock()? {
        Some(lock) => lock,
        None => {
            info!("busy: waiting for the compactor that holds the lock");
            let Some(lock) = thread.lock_before(deadline)? else {
                warn!("the lock was still held at the deadline; built nothing");
                return Ok(Built::default());
            };
            lock
        }
    };
    let mut run = Run::new(thread, backend);
    run.pass(Some(keys), Some(deadline))?;
    let built = run.built.clone();
    drop(run);
    lock.unlock()?;
    Ok(built)
}

/// The background compactor: under the thread's lock, builds every line that is ready on any
/// branch, over each newer snapshot, until none is; logs to [`Thread::log_path`]. `None` if
/// another compactor holds the lock.
pub fn nap(thread: &Thread, backend: Arc<dyn Backend>) -> Result<Option<Built>> {
    let _log = log::init(&thread.log_path()?)?;
    let _nap = info_span!(
        "nap",
        pid = process::id(),
        thread = thread.name(),
        model = backend.model()
    )
    .entered();
    let start = Instant::now();
    let mut run = Run::new(thread, backend);
    let mut napped = false;
    loop {
        let Some(lock) = thread.try_lock()? else {
            info!("busy: another compactor holds the lock; exiting");
            return Ok(napped.then(|| run.built.clone()));
        };
        if !napped {
            info!("start");
            napped = true;
        }
        let seen = loop {
            let tried = run.tried();
            let seen = run.pass(None, None)?;
            if run.tried() == tried {
                break seen;
            }
        };
        lock.unlock()?;
        drop(lock);
        if thread.snapshot()?.commit() == seen {
            let b = &run.built;
            let secs = log::secs(start.elapsed());
            info!(
                lines = b.lines,
                calls = b.calls,
                failed = b.failed.len(),
                secs,
                "end"
            );
            return Ok(Some(run.built.clone()));
        }
    }
}

type Done = (Key, Duration, Result<(Who, String)>);

/// Builds lines of one thread, each call on its own thread.
struct Run<'t> {
    thread: &'t Thread,
    backend: Arc<dyn Backend>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    busy: HashSet<Key>,
    failed: HashSet<Key>,
    /// Calls failed since the last that succeeded; at [`JOBS`] no more start.
    streak: usize,
    built: Built,
}

/// Build order: oldest first, and a line before the node over it that ends where it ends.
fn order(k: Key) -> (u64, u32) {
    (k.last, k.level)
}

fn key((last, level): (u64, u32)) -> Key {
    Key { level, last }
}

impl<'t> Run<'t> {
    fn new(thread: &'t Thread, backend: Arc<dyn Backend>) -> Run<'t> {
        let (tx, rx) = channel();
        Run {
            thread,
            backend,
            tx,
            rx,
            busy: HashSet::new(),
            failed: HashSet::new(),
            streak: 0,
            built: Built::default(),
        }
    }

    /// Lines written or failed so far.
    fn tried(&self) -> usize {
        self.built.lines as usize + self.built.failed.len()
    }

    /// Builds on the latest snapshot what is ready, of `want` and what it is made from if
    /// given, until nothing more can start or `deadline` passes; returns the commit it read.
    fn pass(
        &mut self,
        want: Option<&[Key]>,
        deadline: Option<Instant>,
    ) -> Result<Option<ObjectId>> {
        let snap = self.thread.snapshot()?;
        let roll = Rollup::new(&snap.tree()?);
        let mut lines: HashMap<Key, String> = snap
            .all_lines()?
            .into_iter()
            .map(|(k, l)| (k, l.text))
            .collect();
        let mut todo: BTreeSet<(u64, u32)> = match want {
            Some(keys) => self.closure(&roll, &lines, keys)?,
            None => roll
                .lines()
                .keys()
                .filter(|k| !lines.contains_key(k))
                .map(|&k| order(k))
                .collect(),
        };
        loop {
            self.free(&roll, &mut lines, &mut todo)?;
            self.start(&roll, &lines, &todo);
            if self.busy.is_empty() {
                return Ok(snap.commit());
            }
            let done = match deadline {
                Some(d) => match self
                    .rx
                    .recv_timeout(d.saturating_duration_since(Instant::now()))
                {
                    Ok(done) => done,
                    Err(RecvTimeoutError::Timeout) => return Ok(snap.commit()),
                    Err(RecvTimeoutError::Disconnected) => unreachable!("the run holds a sender"),
                },
                None => self.rx.recv().expect("the run holds a sender"),
            };
            self.finish(&roll, done, &mut lines, &mut todo)?;
            while let Ok(done) = self.rx.try_recv() {
                self.finish(&roll, done, &mut lines, &mut todo)?;
            }
        }
    }

    /// The unbuilt lines among `keys` and those they are made from.
    fn closure(
        &self,
        roll: &Rollup,
        lines: &HashMap<Key, String>,
        keys: &[Key],
    ) -> Result<BTreeSet<(u64, u32)>> {
        let mut out = BTreeSet::new();
        let mut stack = keys.to_vec();
        while let Some(k) = stack.pop() {
            if lines.contains_key(&k) || !out.insert(order(k)) {
                continue;
            }
            let Some(s) = roll.source(k) else {
                bail!("{k} is no line of thread {}.", self.thread.name());
            };
            stack.extend(s.children());
        }
        Ok(out)
    }

    /// Writes every line of `todo` that needs no call, in one commit per round, until none.
    fn free(
        &mut self,
        roll: &Rollup,
        lines: &mut HashMap<Key, String>,
        todo: &mut BTreeSet<(u64, u32)>,
    ) -> Result<()> {
        loop {
            let free: Vec<(Key, Line)> = todo
                .iter()
                .map(|&o| key(o))
                .filter_map(|k| {
                    let text = roll.free(k, |c| lines.get(&c).map(String::as_str))?;
                    let line = Line {
                        ts: Timestamp::now(),
                        kind: roll.source(k)?.kind(),
                        who: Who::default(),
                        text,
                    };
                    Some((k, line))
                })
                .collect();
            if free.is_empty() {
                return Ok(());
            }
            let keys: Vec<Key> = free.iter().map(|f| f.0).collect();
            let stored = self.thread.put_lines(free)?;
            for (k, line) in keys.into_iter().zip(stored) {
                lines.insert(k, line.text);
                todo.remove(&order(k));
                self.built.lines += 1;
            }
        }
    }

    /// Starts the calls of `todo` that are ready, in order, up to [`JOBS`] running.
    fn start(&mut self, roll: &Rollup, lines: &HashMap<Key, String>, todo: &BTreeSet<(u64, u32)>) {
        if self.streak >= JOBS {
            return;
        }
        for k in todo.iter().map(|&o| key(o)) {
            if self.busy.len() >= JOBS {
                return;
            }
            if self.busy.contains(&k) || self.failed.contains(&k) {
                continue;
            }
            let Some(task) = roll.task(k, |c| lines.get(&c).map(String::as_str)) else {
                continue;
            };
            self.busy.insert(k);
            let (backend, tx) = (self.backend.clone(), self.tx.clone());
            let (span, dispatch) = (Span::current(), dispatcher::get_default(Clone::clone));
            thread::spawn(move || {
                let _d = dispatcher::set_default(&dispatch);
                let _s = span.enter();
                let at = Instant::now();
                let r = catch_unwind(AssertUnwindSafe(|| {
                    write_line(&*backend, PROMPT, &[Block::new(task, false)], &k)
                }))
                .unwrap_or_else(|_| Err(anyhow!("the call panicked")));
                let _ = tx.send((k, at.elapsed(), r));
            });
        }
    }

    fn finish(
        &mut self,
        roll: &Rollup,
        (k, took, r): Done,
        lines: &mut HashMap<Key, String>,
        todo: &mut BTreeSet<(u64, u32)>,
    ) -> Result<()> {
        self.busy.remove(&k);
        let secs = log::secs(took);
        match r {
            Ok((who, text)) => {
                info!(line = %k, bytes = text.len(), secs, "built");
                let kind = roll.source(k).expect("a planned line").kind();
                let line = self.thread.put_line(k, kind, &who, &text)?;
                lines.insert(k, line.text);
                todo.remove(&order(k));
                self.built.lines += 1;
                self.built.calls += 1;
                self.streak = 0;
            }
            Err(e) => {
                let error = format!("{e:#}");
                warn!(line = %k, error, secs, "failed");
                self.failed.insert(k);
                self.built.failed.push((k, error));
                self.streak += 1;
                if self.streak == JOBS {
                    warn!(failures = JOBS, "stopping: calls keep failing");
                }
            }
        }
        Ok(())
    }
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        if !self.busy.is_empty() {
            self.backend.stop();
        }
    }
}
