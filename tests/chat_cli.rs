use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use ai_tools::{
    chat::{Chat, Key, LineKind, NOT_ATTACHED, Tree, VIEW, claude, exchanges},
    memory::Who,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const ISOLATE: [&str; 6] = [
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDECODE",
    "GIT_DIR",
    "GIT_AUTHOR_NAME",
    "GIT_COMMITTER_NAME",
    "AI_CHAT_NAP",
];

/// A git repository, a Claude Code config dir and a fake `claude`, in a temporary directory.
struct Sandbox {
    _tmp: TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        gix::init(root.join("repo")).unwrap();
        for d in ["home", "claude", "out", "bin"] {
            fs::create_dir(root.join(d)).unwrap();
        }
        let s = Sandbox { _tmp: tmp, root };
        s.fake_claude();
        s
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    /// Where Claude Code keeps the sessions started in `dir`.
    fn project(&self, dir: &Path) -> PathBuf {
        self.root
            .join("claude/projects")
            .join(claude::project_dir(dir))
    }

    /// Writes the transcript of `session` as Claude Code would, for a session started in the
    /// repository.
    fn transcript(&self, session: &str, text: &str) -> PathBuf {
        let dir = self.project(&self.repo());
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{session}.jsonl"));
        fs::write(&p, text).unwrap();
        p
    }

    /// A `claude` that records its arguments, one per line, in `out/argv`.
    fn fake_claude(&self) {
        let path = std::env::var("PATH").unwrap();
        let bash = std::env::split_paths(&path)
            .map(|d| d.join("bash"))
            .find(|p| p.is_file())
            .expect("bash on PATH");
        let src = self.root.join("claude.sh");
        let argv = self.root.join("out/argv");
        let body = format!(
            "#!{}\nprintf '%s\\n' \"$@\" > {}\n",
            bash.display(),
            argv.display()
        );
        fs::write(&src, body).unwrap();
        let dst = self.root.join("bin/claude");
        assert!(
            Command::new("cp")
                .arg(&src)
                .arg(&dst)
                .status()
                .unwrap()
                .success()
        );
        fs::set_permissions(&dst, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }

    /// The arguments the fake `claude` was last run with.
    fn argv(&self) -> Vec<String> {
        let p = self.root.join("out/argv");
        let argv = fs::read_to_string(&p)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        fs::remove_file(p).unwrap();
        argv
    }

    fn cmd(&self, dir: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ai"));
        for k in ISOLATE {
            c.env_remove(k);
        }
        let path = std::env::var("PATH").unwrap();
        c.current_dir(dir)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env("AI_CHAT_NAP", "0")
            .env(
                "PATH",
                format!("{}:{path}", self.root.join("bin").display()),
            )
            .arg("chat");
        c
    }

    fn ai(&self, args: &[&str]) -> Output {
        self.cmd(&self.repo()).args(args).output().unwrap()
    }

    /// `ai chat args` run outside any repository.
    fn outside(&self, args: &[&str]) -> Output {
        self.cmd(&self.root.join("out"))
            .args(args)
            .output()
            .unwrap()
    }

    /// `ai chat args` run inside session `session`, as its Bash tool runs it.
    fn ai_in(&self, session: &str, args: &[&str]) -> Output {
        self.cmd(&self.repo())
            .env("CLAUDE_CODE_SESSION_ID", session)
            .args(args)
            .output()
            .unwrap()
    }

    fn chat(&self) -> Chat {
        Chat::find(&self.repo()).unwrap().unwrap()
    }

    /// Writes line `level@last` of thread `name`.
    fn line(&self, name: &str, level: u32, last: u64, text: &str) {
        let kind = match level {
            0 => LineKind::Chunk,
            1 => LineKind::Exchange,
            _ => LineKind::Node,
        };
        let t = self.chat().thread(name).unwrap();
        t.put_line(Key { level, last }, kind, &Who::default(), text)
            .unwrap();
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

/// Exit 0 with this stdout.
fn ok(o: Output, want: &str) {
    assert_eq!((code(&o), stderr(&o).as_str()), (0, ""), "{}", stdout(&o));
    assert_eq!(stdout(&o), want);
}

/// Exit 3, nothing printed.
fn unattached(o: Output) {
    assert_eq!(code(&o), i32::from(NOT_ATTACHED), "{}", stderr(&o));
    assert_eq!((stdout(&o).as_str(), stderr(&o).as_str()), ("", ""));
}

fn fixture(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude");
    fs::read_to_string(p.join(name)).unwrap()
}

/// A transcript of `n` exchanges: the user's `ask(k)` and a reply.
fn talk(n: usize, ask: impl Fn(usize) -> String) -> String {
    let mut out = String::new();
    let mut parent = Value::Null;
    for k in 0..n {
        let (u, a) = (format!("u{k}"), format!("a{k}"));
        let ts = format!("2026-10-14T10:{:02}:{:02}.000Z", k / 60, k % 60);
        let user = json!({
            "type": "user", "uuid": u, "parentUuid": parent, "timestamp": ts,
            "origin": {"kind": "human"},
            "message": {"role": "user", "content": ask(k)},
        });
        let reply = json!({
            "type": "assistant", "uuid": a, "parentUuid": u, "timestamp": ts,
            "message": {"model": "claude-opus-5-5", "role": "assistant",
                "content": [{"type": "text", "text": format!("reply {k}")}]},
        });
        out += &format!("{user}\n{reply}\n");
        parent = json!(a);
    }
    out
}

/// The sequence number of the last item of each exchange on the newest branch of `name`.
fn exchange_ends(s: &Sandbox, name: &str) -> Vec<u64> {
    let t = s.chat().thread(name).unwrap();
    let tree: Tree = t.snapshot().unwrap().tree().unwrap();
    let tip = *tree.tips().last().unwrap();
    let branch = tree.branch(tip);
    exchanges(&tree.path(tip))
        .iter()
        .map(|x| branch[x.span.end - 1] as u64)
        .collect()
}

/// The lines of one section of a view.
fn section<'v>(view: &'v str, title: &str) -> Vec<&'v str> {
    let Some((_, rest)) = view.split_once(&format!("\n## {title}\n")) else {
        return Vec::new();
    };
    rest.split("\n\n## ").next().unwrap().lines().collect()
}

#[test]
fn commands_for_an_unattached_session_exit_3_silently() {
    let s = Sandbox::new();
    let flags = ["--harness", "claude", "--session", "s-1"];
    for cmd in [
        &["sync"][..],
        &["compact"],
        &["detach"],
        &["zoom", "0"],
        &["resume"],
    ] {
        unattached(s.ai(&[cmd, &flags[..]].concat()));
        unattached(s.ai_in("s-1", cmd));
    }
    for cmd in [&["sync"][..], &["compact"], &["detach"], &["zoom", "0"]] {
        unattached(s.ai(cmd));
    }
    unattached(s.outside(&["sync", "--session", "s-1"]));
}

#[test]
fn attach_sync_and_detach() {
    let s = Sandbox::new();
    let all = fixture("interject.jsonl");
    let half: String = all.split_inclusive('\n').take(8).collect();
    s.transcript("s-1", &half);
    ok(
        s.ai_in("s-1", &["attach", "dev"]),
        "Attached to dev, a new thread.\n",
    );
    let t = s.chat().thread("dev").unwrap();
    let n = t.snapshot().unwrap().len().unwrap();
    assert!(n > 0, "the transcript is found from the session id");
    let path = s.transcript("s-1", &all);
    ok(
        s.ai(&[
            "sync",
            "--session",
            "s-1",
            "--transcript",
            path.to_str().unwrap(),
        ]),
        "",
    );
    assert!(t.snapshot().unwrap().len().unwrap() > n);
    ok(s.ai_in("s-1", &["attach", "dev"]), "Attached to dev.\n");
    ok(
        s.ai(&["attach", "other", "--harness", "claude", "--session", "s-1"]),
        "Attached to other, a new thread; detached from dev.\n",
    );
    ok(s.ai_in("s-1", &["detach"]), "Detached from other.\n");
    unattached(s.ai_in("s-1", &["detach"]));

    let o = s.ai(&["attach", "dev"]);
    assert_eq!(code(&o), 1);
    assert_eq!(
        stderr(&o),
        "No session: run this inside a harness session, or pass --session.\n"
    );
    let o = s.ai_in("s-1", &["attach", "a/b"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("is not a thread name"));
    let o = s.outside(&["attach", "dev", "--session", "s-1"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).starts_with("Not in a git repository"));
}

#[test]
fn compact_syncs_and_prints_the_view() {
    let s = Sandbox::new();
    let all = fixture("interject.jsonl");
    let first: String = all.split_inclusive('\n').take(14).collect();
    s.transcript("s-1", &first);
    ok(
        s.ai_in("s-1", &["attach", "dev"]),
        "Attached to dev, a new thread.\n",
    );
    s.transcript("s-1", &all);
    let ends = exchange_ends(&s, "dev");
    s.line("dev", 1, ends[0], "user asked to run the tests; two failed");
    let o = s.ai(&["compact", "--harness", "claude", "--session", "s-1"]);
    assert_eq!((code(&o), stderr(&o).as_str()), (0, ""));
    let view = stdout(&o);
    assert!(view.starts_with("# Thread `dev`\n\nThis is your memory of the thread `dev`"));
    assert!(view.contains("`ai chat zoom a-b`"));
    let ends = exchange_ends(&s, "dev");
    assert_eq!(ends.len(), 4, "compact synced the rest of the transcript");
    let fold = section(&view, "Earlier exchanges, summarized");
    assert_eq!(fold.len(), 3);
    assert!(fold[0].ends_with("|user asked to run the tests; two failed"));
    assert!(fold[1].ends_with("(not summarized yet: zoom 4-10)"));
    let rail = section(&view, "The user's latest messages");
    assert!(
        rail.iter()
            .any(|l| l.ends_with("|user: Skip the network tests."))
    );
    let open = section(&view, "The exchange in progress");
    assert!(open[0].starts_with(&format!("{}|user: ", ends[2] + 1)));
    assert!(view.len() <= VIEW);
}

#[test]
fn compact_fails_with_its_reason() {
    let s = Sandbox::new();
    let path = s.root.join("out/p-1.jsonl");
    let p = path.to_str().unwrap();
    let flags = ["--harness", "pi", "--session", "p-1", "--transcript", p];
    ok(
        s.ai(&[&["attach", "dev"][..], &flags].concat()),
        "Attached to dev, a new thread.\n",
    );
    fs::write(&path, "{}\n").unwrap();
    let o = s.ai(&[&["compact"][..], &flags].concat());
    assert_eq!(code(&o), 1);
    assert_eq!(
        (stdout(&o).as_str(), stderr(&o).as_str()),
        ("", "ai chat cannot read pi sessions.\n")
    );
}

#[test]
fn resume_lists_the_threads_or_pages_the_view() {
    let s = Sandbox::new();
    ok(
        s.ai(&["resume"]),
        "No threads here. Start one: ai chat claude <name>\n",
    );
    s.transcript("s-1", &fixture("rewind.jsonl"));
    ok(
        s.ai_in("s-1", &["attach", "dev"]),
        "Attached to dev, a new thread.\n",
    );
    s.transcript("s-2", &talk(100, |k| format!("question {k}")));
    ok(
        s.ai_in("s-2", &["attach", "long-one"]),
        "Attached to long-one, a new thread.\n",
    );
    ok(
        s.ai(&["resume"]),
        "dev       claude  2026-10-14 13:02  2 branches\n\
         long-one  claude  2026-10-14 10:01  1 branch\n",
    );

    let ends = exchange_ends(&s, "long-one");
    let text = |k: usize| format!("{k} {}", "w".repeat(500));
    for (k, &end) in ends.iter().enumerate().filter(|e| e.0 != 60) {
        s.line("long-one", 1, end, &text(k));
    }
    let o = s.ai(&["compact", "--session", "s-2"]);
    let view = stdout(&o);
    assert!(
        view.len() > 2 * 20_000 && view.len() < 3 * 19_000,
        "{}",
        view.len()
    );
    let o = s.ai_in("s-2", &["resume"]);
    let first = stdout(&o);
    let (body, footer) = first.trim_end().rsplit_once('\n').unwrap();
    let next = footer.strip_prefix("Part 1 of 3. Next: ai chat ").unwrap();
    assert!(next.starts_with("resume --part 2 --at "));
    let mut parts = body.to_string() + "\n";
    s.line("long-one", 1, ends[60], &text(60));
    let args: Vec<&str> = next.split(' ').collect();
    let o = s.ai_in("s-2", &args);
    let second = stdout(&o);
    let (body, footer) = second.trim_end().rsplit_once('\n').unwrap();
    parts += &(body.to_string() + "\n");
    let args: Vec<&str> = footer
        .split_once("Next: ai chat ")
        .unwrap()
        .1
        .split(' ')
        .collect();
    let third = stdout(&s.ai_in("s-2", &args));
    let (body, footer) = third.trim_end().rsplit_once('\n').unwrap();
    assert_eq!(footer, "Part 3 of 3, the last.");
    parts += &(body.to_string() + "\n");
    assert_eq!(parts, view, "the parts come from one commit");
    let o = s.ai_in("s-2", &["resume", "--part", "2", "--at", "xyz"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("'xyz' is not a commit id"));
}

#[test]
fn zoom_opens_lines_and_items() {
    let s = Sandbox::new();
    let mut text = talk(4, |k| format!("question {k}"));
    let long = json!({
        "type": "user", "uuid": "u4", "parentUuid": "a3",
        "timestamp": "2026-10-14T11:00:00.000Z", "origin": {"kind": "human"},
        "message": {"role": "user", "content": "paste ".repeat(5_000)},
    });
    text += &format!("{long}\n");
    s.transcript("s-1", &text);
    ok(
        s.ai_in("s-1", &["attach", "dev"]),
        "Attached to dev, a new thread.\n",
    );
    s.line("dev", 1, 1, "first");
    s.line("dev", 1, 3, "second");
    s.line("dev", 2, 3, "first two");
    ok(s.ai_in("s-1", &["zoom", "0-3"]), "0-1|first\n2-3|second\n");
    ok(
        s.ai_in("s-1", &["zoom", "0-7"]),
        "0-3|first two\n4-7|(not summarized yet: zoom 4-7)\n",
    );
    ok(
        s.ai_in("s-1", &["zoom", "4-5"]),
        "4|user: question 2\n5|ai: reply 2\n",
    );
    ok(s.ai_in("s-1", &["zoom", "5"]), "5|ai: reply 2\n");
    let o = s.ai_in("s-1", &["zoom", "8"]);
    let out = stdout(&o);
    assert!(out.ends_with("\nPart 1 of 2. Next: ai chat zoom 8 2\n"));
    let rest = stdout(&s.ai_in("s-1", &["zoom", "8", "2"]));
    let whole = out.rsplit_once("\nPart 1").unwrap().0.to_string() + &rest;
    assert_eq!(whole, format!("8|user: {}\n", "paste ".repeat(5_000)));
    for (id, err) in [
        (
            "x",
            "'x' is not a line id. Copy one from the view, like 120-245 or 7.\n",
        ),
        ("0-5", "No line 0-5.\n"),
        ("9", "No line 9.\n"),
    ] {
        let o = s.ai_in("s-1", &["zoom", id]);
        assert_eq!((code(&o), stderr(&o).as_str()), (1, err));
    }
}

#[test]
fn the_launcher_binds_a_new_session_and_starts_claude() {
    let s = Sandbox::new();
    ok(s.ai(&["claude", "dev", "--", "--model", "haiku"]), "");
    let argv = s.argv();
    assert_eq!(argv[0], "--session-id");
    assert_eq!(argv[2..], ["--model", "haiku"]);
    let id = &argv[1];
    assert_eq!(id.len(), 36);
    let t = s.chat().thread("dev").unwrap();
    let b = t
        .snapshot()
        .unwrap()
        .binding("claude", id)
        .unwrap()
        .unwrap();
    assert!(b.is_attached());
    let want = s.project(&s.repo()).join(format!("{id}.jsonl"));
    assert_eq!(b.transcript, Some(want.clone()));
    assert_eq!((b.graft, b.tip), (None, None));

    fs::create_dir_all(want.parent().unwrap()).unwrap();
    fs::write(&want, fixture("plain.jsonl")).unwrap();
    unattached(s.ai_in("other", &["sync"]));
    ok(s.ai_in(id, &["sync"]), "");
    ok(s.ai(&["claude", "dev"]), "");
    let second = &s.argv()[1];
    assert_ne!(second, id);
    let b = t
        .snapshot()
        .unwrap()
        .binding("claude", second)
        .unwrap()
        .unwrap();
    assert_eq!(
        b.graft,
        Some(3),
        "a new session hangs on the thread's newest item"
    );

    let o = s.ai(&["claude", "a b"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("is not a thread name"));
}

#[test]
fn resume_continues_the_newest_session_or_starts_one() {
    let s = Sandbox::new();
    let o = s.ai(&["resume", "dev"]);
    assert_eq!(
        (code(&o), stderr(&o).as_str()),
        (1, "No thread dev. Start one: ai chat claude dev\n")
    );
    s.transcript("s-1", &fixture("plain.jsonl"));
    let newest = s.transcript("s-2", &talk(1, |_| "later".into()));
    ok(
        s.ai_in("s-1", &["attach", "dev"]),
        "Attached to dev, a new thread.\n",
    );
    ok(s.ai_in("s-2", &["attach", "dev"]), "Attached to dev.\n");
    ok(s.ai_in("s-2", &["detach"]), "Detached from dev.\n");

    ok(s.ai(&["resume", "dev"]), "");
    assert_eq!(s.argv(), ["--resume", "s-2"]);
    let t = s.chat().thread("dev").unwrap();
    let b = t
        .snapshot()
        .unwrap()
        .binding("claude", "s-2")
        .unwrap()
        .unwrap();
    assert!(b.is_attached(), "resuming re-attaches it");

    let elsewhere = s.root.join("repo/sub");
    fs::create_dir(&elsewhere).unwrap();
    let o = s.cmd(&elsewhere).args(["resume", "dev"]).output().unwrap();
    assert_eq!(code(&o), 0);
    assert_eq!(
        stderr(&o),
        "Cannot resume dev here: session s-2 ran in another directory. Starting a new session.\n"
    );
    assert_eq!(s.argv()[0], "--session-id");

    fs::remove_file(newest).unwrap();
    let o = s.ai(&["resume", "dev"]);
    assert_eq!(
        stderr(&o),
        "Cannot resume dev here: the transcript of session s-2 is gone. Starting a new session.\n"
    );
    let argv = s.argv();
    assert_eq!(argv[0], "--session-id");
    let b = t
        .snapshot()
        .unwrap()
        .binding("claude", &argv[1])
        .unwrap()
        .unwrap();
    assert!(b.is_attached() && b.graft.is_some());
}
