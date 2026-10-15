use std::{borrow::Cow, collections::BTreeMap, ops::Range};

use serde_json::Value;

use crate::{
    chat::{
        model::{Exchange, Item, Kind, Marker, Tree, exchanges},
        record::{Key, LineKind},
    },
    memory::{NODE, flat, ruler},
};

/// The system prompt of every roll-up call.
pub const PROMPT: &str = include_str!("prompts/compact.txt");
/// The most one call reads of an exchange, in bytes; a longer exchange is cut into chunks.
pub const INPUT: usize = 48 * 1024;
/// The most a call reads of one tool call's input or one tool output: its head and tail.
pub const CLIP: usize = 4 * 1024;
/// The most a call reads of one message or reply, so no item alone fills a chunk.
const TEXT: usize = INPUT / 2;

/// `s` within about `n` bytes: its head and tail around a note of how much was cut.
pub fn clip(s: &str, n: usize) -> Cow<'_, str> {
    if s.len() <= n {
        return Cow::Borrowed(s);
    }
    let head = s.floor_char_boundary(n / 2);
    let tail = s.ceil_char_boundary(s.len() - n / 2);
    Cow::Owned(format!(
        "{}\n[{} bytes cut]\n{}",
        &s[..head],
        tail - head,
        &s[tail..]
    ))
}

/// A tool call's input: `key=value` per field, strings verbatim, each clipped.
fn input(v: &Value) -> String {
    let value = |v: &Value| match v {
        Value::String(s) => clip(s, CLIP / 2).into_owned(),
        v => clip(&v.to_string(), CLIP / 2).into_owned(),
    };
    match v {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| format!("{k}={}", value(v)))
            .collect::<Vec<_>>()
            .join(" "),
        Value::Null => String::new(),
        v => value(v),
    }
}

/// Item `seq` as a call reads it: `#seq`, its kind, its content clipped; one entry, ending in a
/// newline.
pub fn render(seq: u64, item: &Item) -> String {
    let body = match &item.kind {
        Kind::User(t) => format!("user: {}", clip(t, TEXT)),
        Kind::Work { from, text } => format!("work from {from}: {}", clip(text, TEXT)),
        Kind::Text(t) => format!("ai: {}", clip(t, TEXT)),
        Kind::ToolCall { name, input: i, .. } => format!("call {name}: {}", clip(&input(i), CLIP)),
        Kind::ToolResult {
            output, is_error, ..
        } => {
            let what = if *is_error { "error" } else { "result" };
            format!("{what}: {}", clip(output, CLIP))
        }
        Kind::Marker(Marker::Compaction) => "[the harness compacted its context here]".into(),
        Kind::Marker(Marker::Summary) => "[the harness's summary of what it compacted]".into(),
        Kind::Marker(Marker::Interrupt) => "[the user interrupted the run]".into(),
    };
    format!("#{seq} {body}\n")
}

/// Cuts `steps`, consecutive ranges of positions, into chunks of at most `budget` bytes,
/// greedily from the first: whole steps while they fit. A step over the budget starts a chunk
/// and is cut the same way before its tool calls; a piece still over it is a chunk alone.
/// `size` gives a position's bytes, `call` whether it is a tool call.
pub fn chunk(
    steps: &[Range<usize>],
    budget: usize,
    size: impl Fn(usize) -> usize,
    call: impl Fn(usize) -> bool,
) -> Vec<Range<usize>> {
    let bytes = |r: &Range<usize>| r.clone().map(&size).sum::<usize>();
    let mut p = Pack {
        budget,
        out: Vec::new(),
        open: None,
    };
    for s in steps {
        let n = bytes(s);
        if n <= budget {
            p.add(s.clone(), n);
            continue;
        }
        p.cut();
        let mut start = s.start;
        for i in s.start + 1..=s.end {
            if i == s.end || call(i) {
                p.add(start..i, bytes(&(start..i)));
                start = i;
            }
        }
    }
    p.cut();
    p.out
}

/// Consecutive pieces packed into chunks of at most `budget` bytes, a piece over it alone.
struct Pack {
    budget: usize,
    out: Vec<Range<usize>>,
    open: Option<(Range<usize>, usize)>,
}

impl Pack {
    fn add(&mut self, piece: Range<usize>, n: usize) {
        if let Some((r, m)) = &mut self.open
            && *m + n <= self.budget
        {
            r.end = piece.end;
            *m += n;
            return;
        }
        self.cut();
        self.open = Some((piece, n));
    }

    fn cut(&mut self) {
        self.out.extend(self.open.take().map(|(r, _)| r));
    }
}

/// What a line is made from. Items are sequence numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Consecutive items of an exchange over [`INPUT`], read with the message it answers.
    Chunk {
        message: Option<u64>,
        items: Vec<u64>,
    },
    /// A whole exchange.
    Exchange(Vec<u64>),
    /// An exchange over [`INPUT`]: its message, then its chunk lines.
    Chunked {
        message: Option<u64>,
        chunks: Vec<Key>,
    },
    /// Two adjacent lines, the earlier first.
    Node([Key; 2]),
}

impl Source {
    pub fn kind(&self) -> LineKind {
        match self {
            Source::Chunk { .. } => LineKind::Chunk,
            Source::Exchange(_) | Source::Chunked { .. } => LineKind::Exchange,
            Source::Node(_) => LineKind::Node,
        }
    }

    /// The lines it is made from.
    pub fn children(&self) -> &[Key] {
        match self {
            Source::Chunked { chunks, .. } => chunks,
            Source::Node(pair) => pair,
            Source::Chunk { .. } | Source::Exchange(_) => &[],
        }
    }
}

/// One exchange of a branch, with its chunks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub exchange: Exchange,
    /// Positions on the path, when the exchange is over [`INPUT`]: every chunk once it is
    /// closed, else all but the last, which may still grow.
    pub chunks: Vec<Range<usize>>,
}

/// The exchanges of one branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The sequence number of each position on the path, root first.
    pub seqs: Vec<usize>,
    pub parts: Vec<Part>,
}

impl Branch {
    /// The key of the line of `level` that ends at position `at`.
    pub fn key(&self, level: u32, at: usize) -> Key {
        Key {
            level,
            last: self.seqs[at] as u64,
        }
    }

    pub fn chunk_keys(&self, p: &Part) -> Vec<Key> {
        p.chunks.iter().map(|c| self.key(0, c.end - 1)).collect()
    }

    /// The exchange line of `p`, once it is closed.
    pub fn line(&self, p: &Part) -> Option<Key> {
        let x = &p.exchange;
        x.closed.then(|| self.key(1, x.span.end - 1))
    }

    /// The exchange lines of the closed exchanges, in order.
    pub fn lines(&self) -> Vec<Key> {
        self.parts.iter().filter_map(|p| self.line(p)).collect()
    }
}

/// The binary nodes over the adjacent lines `lines`, aligned from the first, each with its two
/// halves; lowest level first.
pub fn nodes(lines: &[Key]) -> Vec<(Key, [Key; 2])> {
    let mut out = Vec::new();
    let mut row = lines.to_vec();
    while row.len() > 1 {
        row = row
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[a, b]| {
                let k = Key {
                    level: b.level + 1,
                    last: b.last,
                };
                out.push((k, [a, b]));
                k
            })
            .collect();
    }
    out
}

const EXCHANGE: &str = "Compress this exchange";
const CHUNK: &str = "Compress this part of a long exchange";
const CHUNKED: &str =
    "Compress this long exchange, given as its message and the lines of its parts in order,";
const MERGE: &str = "Merge these two adjacent lines, the earlier first,";

/// A call's message: the size of a line, what to do, the context and the input.
fn task(what: &str, context: Option<&str>, input: &str) -> String {
    let context = context.map_or(String::new(), |c| format!("<context>\n{c}</context>\n"));
    format!(
        "{}{what} into one line of at most {NODE} bytes.\n{context}<input>\n{input}</input>",
        ruler()
    )
}

/// The roll-up of a thread's main tree: every line of every branch, with what it is made from.
pub struct Rollup {
    texts: Vec<String>,
    lines: BTreeMap<Key, Source>,
}

impl Rollup {
    pub fn new(tree: &Tree) -> Rollup {
        let texts = (0..)
            .zip(tree.items())
            .map(|(seq, item)| render(seq, item))
            .collect();
        let mut r = Rollup {
            texts,
            lines: BTreeMap::new(),
        };
        for tip in tree.tips() {
            let b = r.branch(tree, tip);
            r.add(tree, &b);
        }
        r
    }

    /// The exchanges from the root down to `tip`, with their chunks.
    pub fn branch(&self, tree: &Tree, tip: usize) -> Branch {
        let seqs = tree.branch(tip);
        let path: Vec<&Item> = seqs.iter().map(|&i| &tree.items()[i]).collect();
        let size = |p: usize| self.texts[seqs[p]].len();
        let call = |p: usize| matches!(path[p].kind, Kind::ToolCall { .. });
        let parts = exchanges(&path)
            .into_iter()
            .map(|x| {
                let over = x.span.clone().map(size).sum::<usize>() > INPUT;
                let mut chunks = match over {
                    true => chunk(&x.steps, INPUT, size, call),
                    false => Vec::new(),
                };
                if !x.closed {
                    chunks.pop();
                }
                Part {
                    exchange: x,
                    chunks,
                }
            })
            .collect();
        Branch { seqs, parts }
    }

    fn add(&mut self, tree: &Tree, b: &Branch) {
        let seqs = |r: &Range<usize>| r.clone().map(|p| b.seqs[p] as u64).collect::<Vec<_>>();
        for p in &b.parts {
            let first = b.seqs[p.exchange.span.start];
            let message = matches!(tree.items()[first].kind, Kind::User(_) | Kind::Work { .. })
                .then_some(first as u64);
            let keys = b.chunk_keys(p);
            for (c, &k) in p.chunks.iter().zip(&keys) {
                self.lines.entry(k).or_insert_with(|| Source::Chunk {
                    message,
                    items: seqs(c),
                });
            }
            if let Some(k) = b.line(p) {
                self.lines
                    .entry(k)
                    .or_insert_with(|| match keys.is_empty() {
                        true => Source::Exchange(seqs(&p.exchange.span)),
                        false => Source::Chunked {
                            message,
                            chunks: keys.clone(),
                        },
                    });
            }
        }
        for (k, pair) in nodes(&b.lines()) {
            self.lines.entry(k).or_insert(Source::Node(pair));
        }
    }

    /// Every line, by key.
    pub fn lines(&self) -> &BTreeMap<Key, Source> {
        &self.lines
    }

    pub fn source(&self, key: Key) -> Option<&Source> {
        self.lines.get(&key)
    }

    /// Item `seq` as a call reads it.
    pub fn text(&self, seq: u64) -> &str {
        &self.texts[seq as usize]
    }

    /// What line `key` is made from, as a call reads it; `None` until every line it is made
    /// from is `built`.
    fn input<'a>(&self, key: Key, built: impl Fn(Key) -> Option<&'a str>) -> Option<String> {
        Some(match self.lines.get(&key)? {
            Source::Chunk { items, .. } | Source::Exchange(items) => {
                items.iter().map(|&s| self.text(s)).collect()
            }
            Source::Chunked { message, chunks } => {
                let mut s = message.map_or(String::new(), |m| self.text(m).into());
                for &k in chunks {
                    let first = match self.lines.get(&k) {
                        Some(Source::Chunk { items, .. }) => items[0],
                        _ => k.last,
                    };
                    s += &format!("part #{first}-#{}: {}\n", k.last, built(k)?);
                }
                s
            }
            Source::Node([a, b]) => format!("{}\n{}\n", built(*a)?, built(*b)?),
        })
    }

    /// The text of line `key` when what it is made from fits a line, so it needs no call;
    /// `None` when it needs one, or until every line it is made from is `built`.
    pub fn free<'a>(&self, key: Key, built: impl Fn(Key) -> Option<&'a str>) -> Option<String> {
        if let Source::Chunk { items, .. } | Source::Exchange(items) = self.lines.get(&key)?
            && items.iter().map(|&s| self.text(s).len()).sum::<usize>() > NODE
        {
            return None;
        }
        let input = self.input(key, built)?;
        (input.len() <= NODE).then(|| flat(input.trim_end()))
    }

    /// The message of the call that writes line `key`; `None` until every line it is made
    /// from is `built`.
    pub fn task<'a>(&self, key: Key, built: impl Fn(Key) -> Option<&'a str>) -> Option<String> {
        let source = self.lines.get(&key)?;
        let input = self.input(key, built)?;
        Some(match source {
            Source::Chunk { message, .. } => task(
                CHUNK,
                message.map(|m| clip(self.text(m), CLIP)).as_deref(),
                &input,
            ),
            Source::Exchange(_) => task(EXCHANGE, None, &input),
            Source::Chunked { .. } => task(CHUNKED, None, &input),
            Source::Node(_) => task(MERGE, None, &input),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::SourceId;
    use jiff::Timestamp;
    use serde_json::json;

    #[test]
    fn clips_keep_head_and_tail() {
        assert_eq!(clip("short", 10), "short");
        let s = format!("{}{}", "a".repeat(10), "b".repeat(10));
        assert_eq!(clip(&s, 8), "aaaa\n[12 bytes cut]\nbbbb");
        let u = "é".repeat(10);
        let c = clip(&u, 7);
        assert!(c.starts_with("é\n") && c.ends_with("\né"), "{c}");
    }

    fn item(kind: Kind) -> Item {
        Item {
            id: SourceId {
                harness: "test".into(),
                session: "s".into(),
                entry: "e".into(),
            },
            parent: None,
            ts: Timestamp::UNIX_EPOCH,
            kind,
            raw: String::new(),
        }
    }

    #[test]
    fn items_render_one_entry_each() {
        let r = |k| render(7, &item(k));
        assert_eq!(r(Kind::User("fix it".into())), "#7 user: fix it\n");
        assert_eq!(
            r(Kind::Work {
                from: "reviewer".into(),
                text: "done".into()
            }),
            "#7 work from reviewer: done\n"
        );
        assert_eq!(r(Kind::Text("ok".into())), "#7 ai: ok\n");
        let call = Kind::ToolCall {
            id: "t1".into(),
            name: "Bash".into(),
            input: json!({"command": "cargo test", "timeout": 60}),
        };
        assert_eq!(r(call), "#7 call Bash: command=cargo test timeout=60\n");
        let out = format!("{}{}", "h".repeat(CLIP), "t".repeat(CLIP));
        let result = r(Kind::ToolResult {
            id: "t1".into(),
            output: out,
            is_error: true,
        });
        assert!(result.starts_with(&format!("#7 error: {}\n[", "h".repeat(CLIP / 2))));
        assert!(result.ends_with(&format!("cut]\n{}\n", "t".repeat(CLIP / 2))));
        let edit = Kind::ToolCall {
            id: "t2".into(),
            name: "Edit".into(),
            input: json!({"file_path": "/a.rs", "old_string": "x".repeat(CLIP), "new_string": "y"}),
        };
        let e = r(edit);
        assert!(e.starts_with("#7 call Edit: file_path=/a.rs new_string=y old_string=xx"));
        assert!(e.len() < CLIP, "each field is clipped: {}", e.len());
        assert_eq!(
            r(Kind::Marker(Marker::Interrupt)),
            "#7 [the user interrupted the run]\n"
        );
    }

    /// Chunks of `sizes` (one per position), steps cut where `steps` start, calls at `calls`.
    fn chunks(
        sizes: &[usize],
        steps: &[usize],
        calls: &[usize],
        budget: usize,
    ) -> Vec<(usize, usize)> {
        let ends: Vec<usize> = steps.iter().skip(1).copied().chain([sizes.len()]).collect();
        let steps: Vec<Range<usize>> = steps.iter().zip(&ends).map(|(&a, &b)| a..b).collect();
        chunk(&steps, budget, |i| sizes[i], |i| calls.contains(&i))
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect()
    }

    #[test]
    fn chunks_are_whole_steps_while_they_fit() {
        assert_eq!(chunks(&[3, 3, 4], &[0, 1, 2], &[], 10), [(0, 3)]);
        assert_eq!(chunks(&[3, 3, 5], &[0, 1, 2], &[], 10), [(0, 2), (2, 3)]);
        assert_eq!(
            chunks(&[4, 4, 4, 4, 4], &[0, 2, 4], &[], 8),
            [(0, 2), (2, 4), (4, 5)]
        );
        assert_eq!(chunks(&[], &[], &[], 8), []);
    }

    #[test]
    fn a_step_over_the_budget_is_cut_before_its_calls() {
        // step 0: [0]; step 1: text 1, call 2, result 3, call 4, result 5, call 6, result 7
        let sizes = [2, 1, 3, 3, 3, 3, 1, 1];
        assert_eq!(
            chunks(&sizes, &[0, 1], &[2, 4, 6], 8),
            [(0, 1), (1, 4), (4, 8)],
            "the step starts a chunk; pieces pack greedily"
        );
        assert_eq!(
            chunks(&[2, 1, 20, 1, 2], &[0, 1, 4], &[2, 3], 8),
            [(0, 1), (1, 2), (2, 3), (3, 5)],
            "a piece over the budget is a chunk alone; the next step joins the last piece"
        );
    }

    #[test]
    fn chunks_but_the_last_stay_as_items_are_appended() {
        let mut sizes = Vec::new();
        let mut last = Vec::new();
        for n in 0..40 {
            sizes.push(1 + (n * 7) % 5);
            let steps: Vec<usize> = (0..=n).filter(|i| i % 5 == 0).collect();
            let calls: Vec<usize> = (0..=n).filter(|i| i % 5 == 1 || i % 5 == 3).collect();
            let now = chunks(&sizes, &steps, &calls, 8);
            let done = &last[..last.len().saturating_sub(1)];
            assert_eq!(&now[..done.len()], done, "at {n}");
            last = now;
        }
        assert!(last.len() > 5);
    }

    #[test]
    fn nodes_pair_aligned_lines_upwards() {
        let l = |last| Key { level: 1, last };
        let k = |level, last| Key { level, last };
        let n = nodes(&[l(3), l(7), l(9), l(12), l(20)]);
        assert_eq!(
            n,
            [
                (k(2, 7), [l(3), l(7)]),
                (k(2, 12), [l(9), l(12)]),
                (k(3, 12), [k(2, 7), k(2, 12)]),
            ]
        );
        assert!(nodes(&[l(1)]).is_empty());
    }
}
