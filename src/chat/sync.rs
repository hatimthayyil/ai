use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use jiff::Timestamp;

use crate::{
    chat::{
        claude,
        record::{Binding, check_id},
        store::{Added, Chat, Thread, Transcript, put_binding},
    },
    git::Changes,
};

/// The outcome of [`sync`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Synced {
    /// The session is attached to no thread of the repository, or there is no repository.
    NotAttached,
    Attached {
        thread: String,
        added: Added,
    },
}

/// The outcome of [`Chat::attach`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub thread: String,
    /// The thread is new.
    pub created: bool,
    /// The thread the session was attached to before.
    pub moved_from: Option<String>,
    pub added: Added,
}

/// The session's transcript at `path`, read by its harness's adapter; empty if it does not
/// exist yet.
pub fn read(harness: &str, session: &str, path: Option<&Path>) -> Result<Transcript> {
    let Some(path) = path.filter(|p| p.exists()) else {
        return Ok(Transcript::default());
    };
    match harness {
        claude::HARNESS => Ok(Transcript {
            items: claude::read(path, session)?,
            agents: claude::subagents(path, session)?,
        }),
        _ => bail!("ai chat cannot read {harness} sessions."),
    }
}

/// Ingests the new entries of an attached session, in one commit; a session attached to no
/// thread of the repository at `dir` is [`Synced::NotAttached`].
pub fn sync(dir: &Path, harness: &str, session: &str, transcript: Option<&Path>) -> Result<Synced> {
    match Chat::find(dir)? {
        Some(chat) => chat.sync(harness, session, transcript),
        None => Ok(Synced::NotAttached),
    }
}

impl Chat {
    /// The thread `session` is attached to, with its binding there; the latest attach wins if
    /// two threads claim it.
    pub fn attached(&self, harness: &str, session: &str) -> Result<Option<(Thread, Binding)>> {
        let mut found: Option<(Thread, Binding)> = None;
        for t in self.threads()? {
            let b = t.snapshot()?.binding(harness, session)?;
            if let Some(b) = b.filter(Binding::is_attached)
                && found.as_ref().is_none_or(|f| b.attached > f.1.attached)
            {
                found = Some((t, b));
            }
        }
        Ok(found)
    }

    /// Ingests the new entries of an attached session, from `transcript` or the path its
    /// binding holds, in one commit; none if nothing is new.
    pub fn sync(&self, harness: &str, session: &str, transcript: Option<&Path>) -> Result<Synced> {
        check_id(harness, "harness")?;
        check_id(session, "session")?;
        let Some((thread, bound)) = self.attached(harness, session)? else {
            return Ok(Synced::NotAttached);
        };
        let path = transcript.map(Path::to_path_buf).or(bound.transcript);
        let t = read(harness, session, path.as_deref())?;
        let msg = format!("sync {harness} {session}");
        let (added, _) = thread.mutate(&msg, |s| {
            let Some(old) = s.binding(harness, session)?.filter(Binding::is_attached) else {
                return Ok((None, Changes::default()));
            };
            let mut b = Binding {
                transcript: path.clone().or(old.transcript.clone()),
                ..old.clone()
            };
            let mut ch = Changes::default();
            let added = s.ingest(&mut b, &t, &mut ch)?;
            if b != old {
                put_binding(&mut ch, &b)?;
            }
            Ok((Some(added), ch))
        })?;
        Ok(match added {
            Some(added) => Synced::Attached {
                thread: thread.name().into(),
                added,
            },
            None => Synced::NotAttached,
        })
    }

    /// Attaches a session to the thread `name`, creating it if new, and ingests its whole
    /// history. A new binding grafts the session's roots onto the thread's newest item; a
    /// session attached to another thread is detached there first.
    pub fn attach(
        &self,
        name: &str,
        harness: &str,
        session: &str,
        transcript: Option<&Path>,
    ) -> Result<Attached> {
        check_id(harness, "harness")?;
        check_id(session, "session")?;
        let target = self.thread(name)?;
        let current = self
            .attached(harness, session)?
            .filter(|(t, _)| t.name() != name);
        let bound = target.snapshot()?.binding(harness, session)?;
        let path: Option<PathBuf> = transcript
            .map(Path::to_path_buf)
            .or_else(|| current.as_ref()?.1.transcript.clone())
            .or_else(|| bound?.transcript);
        let t = read(harness, session, path.as_deref())?;
        let moved_from = match current {
            Some((old, _)) => {
                detach(&old, harness, session)?;
                Some(old.name().to_string())
            }
            None => None,
        };
        let created = !target.exists()?;
        let msg = format!("attach {harness} {session}");
        let (added, _) = target.mutate(&msg, |s| {
            let now = Timestamp::now();
            let mut b = match s.binding(harness, session)? {
                Some(b) => Binding {
                    transcript: path.clone().or(b.transcript),
                    attached: now,
                    detached: None,
                    ..b
                },
                None => Binding {
                    harness: harness.into(),
                    session: session.into(),
                    transcript: path.clone(),
                    graft: s.len()?.checked_sub(1),
                    tip: None,
                    attached: now,
                    detached: None,
                },
            };
            let mut ch = Changes::default();
            let added = s.ingest(&mut b, &t, &mut ch)?;
            put_binding(&mut ch, &b)?;
            Ok((added, ch))
        })?;
        Ok(Attached {
            thread: name.into(),
            created,
            moved_from,
            added,
        })
    }

    /// Detaches a session from its thread; returns the thread, none if it was not attached.
    pub fn detach(&self, harness: &str, session: &str) -> Result<Option<String>> {
        check_id(harness, "harness")?;
        check_id(session, "session")?;
        let Some((t, _)) = self.attached(harness, session)? else {
            return Ok(None);
        };
        detach(&t, harness, session)?;
        Ok(Some(t.name().into()))
    }
}

fn detach(t: &Thread, harness: &str, session: &str) -> Result<()> {
    t.mutate(&format!("detach {harness} {session}"), |s| {
        let mut ch = Changes::default();
        if let Some(b) = s.binding(harness, session)?.filter(Binding::is_attached) {
            let b = Binding {
                detached: Some(Timestamp::now()),
                ..b
            };
            put_binding(&mut ch, &b)?;
        }
        Ok(((), ch))
    })?;
    Ok(())
}
