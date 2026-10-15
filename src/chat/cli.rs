use std::{
    env,
    io::{self, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{self, ExitCode},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use gix::ObjectId;

use crate::{
    chat::{
        claude,
        record::Binding,
        store::{Chat, Thread, check_name},
        stub,
        sync::Synced,
        view::{Branch, Id, zoom},
    },
    memory::{broken_pipe, page, plural},
};

/// The exit code of a command for a session attached to no thread; it prints nothing.
pub const NOT_ATTACHED: u8 = 3;
/// The largest part of a paged output, in bytes: under Claude Code's 30,000-character cap on
/// a command's output.
pub const PART: u64 = 20_000;
/// How long `compact` waits for the lines its view needs.
pub const WAIT: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(
    name = "chat",
    about = "Named, repo-local conversations that outlive harness sessions.",
    after_help = "Threads live in the repository's git, under refs/ai/chat/."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(about = "start Claude Code in a new session attached to a thread.")]
    Claude {
        #[arg(help = "the thread: letters, digits, '-' and '_'")]
        name: String,
        #[arg(last = true, help = "arguments for claude, after --")]
        args: Vec<String>,
    },
    #[command(
        about = "with a name, continue the thread's newest Claude Code session; inside a session, print its thread's view; else list the threads."
    )]
    Resume {
        name: Option<String>,
        #[arg(long, help = "the part of the view to print, as printed")]
        part: Option<u64>,
        #[arg(long, help = "the thread commit the parts come from, as printed")]
        at: Option<String>,
        #[command(flatten)]
        session: Session,
    },
    #[command(about = "attach this session to a thread, creating it if new.")]
    Attach {
        name: String,
        #[command(flatten)]
        session: Session,
    },
    #[command(about = "stop tracking this session; /compact is native again.")]
    Detach {
        #[command(flatten)]
        session: Session,
    },
    #[command(
        about = "open a line of the view: a-b into the lines it was made from, n for item n."
    )]
    Zoom {
        #[arg(help = "a line's id, as printed: 120-245, or 7 for item 7")]
        id: String,
        #[arg(help = "the part of a long output to show, as printed")]
        part: Option<u64>,
        #[command(flatten)]
        session: Session,
    },
    #[command(about = "for the harness plugin: copy this session's new entries into its thread.")]
    Sync {
        #[command(flatten)]
        session: Session,
    },
    #[command(
        about = "for the harness plugin: sync, then print the view this session compacts to."
    )]
    Compact {
        #[command(flatten)]
        session: Session,
    },
    #[command(about = "build the thread's summaries; sync starts it in the background.")]
    Nap {
        #[arg(long)]
        thread: String,
    },
}

/// The harness session a command is for: its flags, else the harness's environment.
#[derive(Args, Debug)]
struct Session {
    #[arg(long, default_value = claude::HARNESS, help = "the harness running the session")]
    harness: String,
    #[arg(long, help = "the session's id [default: $CLAUDE_CODE_SESSION_ID]")]
    session: Option<String>,
    #[arg(long, help = "the session's transcript [default: found from its id]")]
    transcript: Option<PathBuf>,
}

/// A session a command names.
struct Current {
    harness: String,
    session: String,
    transcript: Option<PathBuf>,
}

impl Session {
    fn current(self) -> Option<Current> {
        let session = self.session.or_else(|| match self.harness.as_str() {
            claude::HARNESS => claude::session_id(),
            _ => None,
        })?;
        Some(Current {
            harness: self.harness,
            session,
            transcript: self.transcript,
        })
    }
}

impl Current {
    /// Its transcript: as given, else found by its harness's adapter.
    fn transcript(&self, cwd: &Path) -> Option<PathBuf> {
        self.transcript
            .clone()
            .or_else(|| match self.harness.as_str() {
                claude::HARNESS => claude::transcript(&claude_config()?, &self.session, cwd),
                _ => None,
            })
    }
}

fn claude_config() -> Option<PathBuf> {
    claude::config_dir(|k| env::var(k).ok())
}

impl Cli {
    /// Runs in the current directory, printing to stdout.
    pub fn exec(self) -> Result<ExitCode> {
        let cwd = env::current_dir().context("Cannot read the current directory.")?;
        match self.run(&cwd, &mut io::stdout().lock()) {
            Err(e) if broken_pipe(&e) => Ok(ExitCode::SUCCESS),
            r => r,
        }
    }

    pub fn run(self, cwd: &Path, out: &mut dyn Write) -> Result<ExitCode> {
        match self.command {
            Command::Claude { name, args } => launch(cwd, &name, &args),
            Command::Resume {
                name: Some(name), ..
            } => resume(cwd, &name),
            Command::Resume {
                name: None,
                part,
                at,
                session,
            } => match session.current() {
                Some(c) => view(cwd, out, &c, part.unwrap_or(1), at.as_deref()),
                None => list(cwd, out),
            },
            Command::Attach { name, session } => attach(cwd, out, &name, session),
            Command::Detach { session } => detach(cwd, out, session),
            Command::Zoom { id, part, session } => {
                zoom_cmd(cwd, out, &id, part.unwrap_or(1), session)
            }
            Command::Sync { session } => sync(cwd, session),
            Command::Compact { session } => compact(cwd, out, session),
            Command::Nap { thread } => stub::nap(cwd, check_name(&thread)?, out),
        }
    }
}

fn not_attached() -> Result<ExitCode> {
    Ok(ExitCode::from(NOT_ATTACHED))
}

/// The threads of the repository at `cwd`.
fn find(cwd: &Path) -> Result<Chat> {
    Chat::find(cwd)?.context("Not in a git repository: ai chat keeps its threads in one.")
}

/// The thread session `c` is attached to, with its binding; none outside a repository.
fn attached(cwd: &Path, c: &Current) -> Result<Option<(Thread, Binding)>> {
    match Chat::find(cwd)? {
        Some(chat) => chat.attached(&c.harness, &c.session),
        None => Ok(None),
    }
}

/// Starts the thread's background compactor, unless `AI_CHAT_NAP=0`.
fn start_nap(cwd: &Path, thread: &str) -> Result<()> {
    match env::var_os("AI_CHAT_NAP").is_none_or(|v| v != "0") {
        true => stub::spawn_nap(cwd, thread),
        false => Ok(()),
    }
}

/// A random (version 4) UUID, as Claude Code's session ids.
fn uuid() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

/// Runs `claude args`, in place of this process.
fn exec_claude(args: &[&str]) -> Result<ExitCode> {
    let e = process::Command::new("claude").args(args).exec();
    Err(e).context("Cannot start claude.")
}

/// A new Claude Code session attached to `name`, bound with its future transcript.
fn launch(cwd: &Path, name: &str, args: &[String]) -> Result<ExitCode> {
    let chat = find(cwd)?;
    check_name(name)?;
    let session = uuid()?;
    let config = claude_config().context("No home directory: set CLAUDE_CONFIG_DIR.")?;
    let path = config
        .join("projects")
        .join(claude::project_dir(cwd))
        .join(format!("{session}.jsonl"));
    chat.attach(name, claude::HARNESS, &session, Some(&path))?;
    let mut argv = vec!["--session-id", &session];
    argv.extend(args.iter().map(String::as_str));
    exec_claude(&argv)
}

/// The thread's newest Claude Code session, resumed attached when it can be; else a new one.
fn resume(cwd: &Path, name: &str) -> Result<ExitCode> {
    let chat = find(cwd)?;
    let t = chat.thread(name)?;
    if !t.exists()? {
        bail!("No thread {name}. Start one: ai chat claude {name}");
    }
    let newest = t
        .snapshot()?
        .bindings()?
        .into_iter()
        .filter(|b| b.harness == claude::HARNESS)
        .max_by_key(|b| (b.tip, b.attached));
    let here = claude::project_dir(cwd);
    let why = match &newest {
        None => "it has no Claude Code session".to_string(),
        Some(b) => match &b.transcript {
            Some(p) if !p.is_file() => format!("the transcript of session {} is gone", b.session),
            Some(p)
                if p.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|d| d == here.as_str()) =>
            {
                chat.attach(name, claude::HARNESS, &b.session, Some(p))?;
                return exec_claude(&["--resume", &b.session]);
            }
            Some(_) => format!("session {} ran in another directory", b.session),
            None => format!("session {} has no transcript", b.session),
        },
    };
    eprintln!("Cannot resume {name} here: {why}. Starting a new session.");
    launch(cwd, name, &[])
}

/// Every thread: name, the harness and time of its newest item, its branches.
fn list(cwd: &Path, out: &mut dyn Write) -> Result<ExitCode> {
    let threads = find(cwd)?.list()?;
    if threads.is_empty() {
        writeln!(out, "No threads here. Start one: ai chat claude <name>")?;
    }
    let w = threads.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for t in threads {
        let (harness, when) = match &t.newest {
            Some(h) => (
                h.harness.clone(),
                h.ts.strftime("%Y-%m-%d %H:%M").to_string(),
            ),
            None => ("-".into(), "-".into()),
        };
        let branches = plural(t.tips.len() as u64, "branch");
        writeln!(out, "{:<w$}  {harness:<6}  {when:<16}  {branches}", t.name)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn attach(cwd: &Path, out: &mut dyn Write, name: &str, s: Session) -> Result<ExitCode> {
    let Some(c) = s.current() else {
        bail!("No session: run this inside a harness session, or pass --session.");
    };
    let a = find(cwd)?.attach(name, &c.harness, &c.session, c.transcript(cwd).as_deref())?;
    let mut msg = format!("Attached to {}", a.thread);
    if a.created {
        msg += ", a new thread";
    }
    if let Some(old) = &a.moved_from {
        msg += &format!("; detached from {old}");
    }
    writeln!(out, "{msg}.")?;
    if !a.added.is_empty() {
        start_nap(cwd, &a.thread)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn detach(cwd: &Path, out: &mut dyn Write, s: Session) -> Result<ExitCode> {
    let (Some(c), Some(chat)) = (s.current(), Chat::find(cwd)?) else {
        return not_attached();
    };
    match chat.detach(&c.harness, &c.session)? {
        Some(t) => writeln!(out, "Detached from {t}.")?,
        None => return not_attached(),
    }
    Ok(ExitCode::SUCCESS)
}

fn sync(cwd: &Path, s: Session) -> Result<ExitCode> {
    let Some(c) = s.current() else {
        return not_attached();
    };
    match super::sync(cwd, &c.harness, &c.session, c.transcript.as_deref())? {
        Synced::NotAttached => not_attached(),
        Synced::Attached { thread, added } => {
            if !added.is_empty() {
                start_nap(cwd, &thread)?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Sync, the lines the view needs built within [`WAIT`], then the view.
fn compact(cwd: &Path, out: &mut dyn Write, s: Session) -> Result<ExitCode> {
    let (Some(c), Some(chat)) = (s.current(), Chat::find(cwd)?) else {
        return not_attached();
    };
    if chat.sync(&c.harness, &c.session, c.transcript.as_deref())? == Synced::NotAttached {
        return not_attached();
    }
    let Some((t, b)) = chat.attached(&c.harness, &c.session)? else {
        return not_attached();
    };
    let tip = b.tip.or(b.graft);
    let mut branch = Branch::load(&t.snapshot()?, tip)?;
    let keys = branch.needed(t.name());
    if !keys.is_empty() {
        stub::build(&t, &keys, Instant::now() + WAIT)?;
        branch = Branch::load(&t.snapshot()?, tip)?;
    }
    write!(out, "{}", branch.view(t.name()))?;
    Ok(ExitCode::SUCCESS)
}

/// The view of the session's thread, in parts from one commit of it.
fn view(
    cwd: &Path,
    out: &mut dyn Write,
    c: &Current,
    part: u64,
    at: Option<&str>,
) -> Result<ExitCode> {
    let Some((t, _)) = attached(cwd, c)? else {
        return not_attached();
    };
    let snap = match at {
        Some(at) => {
            let id = ObjectId::from_hex(at.as_bytes())
                .ok()
                .with_context(|| format!("'{at}' is not a commit id. Copy it as printed."))?;
            t.snapshot_at(id)?
        }
        None => t.snapshot()?,
    };
    let Some(b) = snap.binding(&c.harness, &c.session)? else {
        return not_attached();
    };
    let commit = snap.as_ref().commit().context("The thread is empty.")?;
    let text = Branch::load(&snap, b.tip.or(b.graft))?.view(t.name());
    let (text, n) = page(&text, PART, part)?;
    writeln!(out, "{}", text.strip_suffix('\n').unwrap_or(text))?;
    if part < n {
        let next = part + 1;
        writeln!(
            out,
            "Part {part} of {n}. Next: ai chat resume --part {next} --at {commit}"
        )?;
    } else if n > 1 {
        writeln!(out, "Part {part} of {n}, the last.")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn zoom_cmd(cwd: &Path, out: &mut dyn Write, id: &str, part: u64, s: Session) -> Result<ExitCode> {
    let Some(c) = s.current() else {
        return not_attached();
    };
    let Some((t, _)) = attached(cwd, &c)? else {
        return not_attached();
    };
    let Some(parsed) = Id::parse(id) else {
        bail!("'{id}' is not a line id. Copy one from the view, like 120-245 or 7.");
    };
    let Some(text) = zoom(&t.snapshot()?, parsed)? else {
        bail!("No line {id}.");
    };
    let (text, n) = page(&text, PART, part)?;
    writeln!(out, "{}", text.strip_suffix('\n').unwrap_or(text))?;
    if part < n {
        writeln!(
            out,
            "Part {part} of {n}. Next: ai chat zoom {parsed} {}",
            part + 1
        )?;
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_version_4() {
        let u = uuid().unwrap();
        assert_eq!(u.len(), 36);
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!("89ab".contains(&parts[3][..1]));
        assert_ne!(uuid().unwrap(), u);
    }
}
