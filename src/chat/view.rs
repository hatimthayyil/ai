use std::{collections::HashMap, ops::Range};

use anyhow::Result;

use crate::{
    chat::{
        id::Id,
        model::{Item, Kind, Tree, Turn},
        record::Key,
        render::{Cut, WHOLE, item_line},
        rollup,
        store::Snapshot,
    },
    memory::{Coord, flat},
};

/// The most a view prints, in bytes.
pub const VIEW: usize = 128_000;
/// The user rail's share of the view.
pub const RAIL: usize = 8_000;
/// The open turn's share of the view.
pub const OPEN: usize = 32_000;
/// The longest summary line; the fold reckons an unbuilt line at this size.
pub const LINE: usize = 512;
/// The most of one message's words a listing prints.
const WORDS: usize = 8_000;
/// The most of one tool call or output the open turn prints.
const TOOL: usize = 2_000;
/// The most of a tool call's input a zoom lists, on one line.
const CALL: usize = 400;

const FOLD: &str = "Earlier turns, summarized";
const USER: &str = "The user's latest messages";
const CURRENT: &str = "The turn in progress";

const INSTRUCTION: &str = "\
This is the thread `{name}`: the conversation so far, oldest first, in place of the context you \
had. Where two lines disagree, the later one holds.

A turn is one message from the user or another agent, and everything you did to answer it. \
Turns are numbered from 0, and the items of a turn from 0, item 0 being its message. Each line \
is `id|text`:
- `12|` summarizes turn 12, and `8+4|` turns 8 to 11.
- `12.3|` is item 3 of turn 12, and `12.3+7|` summarizes its items 3 to 9.
An item's text continues on lines indented by two spaces. A summary cites the item that holds \
a detail in brackets, as [12.3].

To see what a line was made from, zoom it: `ai chat zoom 8+4`. Any turn, item or range of them \
zooms. A zoom lists tool calls without their output; add `--tool-output` to see it. Long output \
comes in parts: run the command each part ends with.

Summaries keep little tool output: say in your replies what you learned that will matter later.
";

/// The fold over `t` closed turns: from one line each, merges the most due aligned pair,
/// due = (t - last)/2^l, oldest first on a tie, until the lines' `cost` fits `budget`.
pub fn fold(t: u64, budget: usize, cost: impl Fn(Coord) -> usize) -> Vec<Coord> {
    let mut view: Vec<Coord> = (0..t).map(Coord::leaf).collect();
    let mut size: usize = view.iter().map(|&c| cost(c)).sum();
    while size > budget {
        let mut best: Option<(usize, f64)> = None;
        for (k, w) in view.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if a.l != b.l || a.i % 2 != 0 || b.i != a.i + 1 {
                continue;
            }
            let due = (t - (b.end() - 1)) as f64 / b.n() as f64;
            if best.is_none_or(|(_, d)| due > d) {
                best = Some((k, due));
            }
        }
        let Some((k, _)) = best else {
            break;
        };
        let p = view[k].parent();
        size = size + cost(p) - cost(view[k]) - cost(view[k + 1]);
        view.splice(k..k + 2, [p]);
    }
    view
}

/// The nodes `d` levels under `c`, in order.
fn under(c: Coord, d: u32) -> Vec<Coord> {
    let l = c.l - d;
    (c.i << d..(c.i + 1) << d)
        .map(|i| Coord::new(l, i))
        .collect()
}

/// The fewest aligned nodes that tile turns `first` to `end - 1`, in order.
fn tile(first: u64, end: u64) -> Vec<Coord> {
    let mut out = Vec::new();
    let mut at = first;
    while at < end {
        let mut l = 0;
        while at.is_multiple_of(2 << l) && at + (2 << l) <= end {
            l += 1;
        }
        out.push(Coord::new(l, at >> l));
        at += 1 << l;
    }
    out
}

/// A session's branch of a thread: its items from the root to its tip, cut into turns, with
/// the lines built over them. Coordinates of the fold count turns along the branch: (l, i)
/// covers turns [i·2^l, (i+1)·2^l), the line of level 1+l.
pub struct Branch {
    tree: Tree,
    /// The sequence numbers from the root to the tip, increasing.
    path: Vec<usize>,
    xs: Vec<Turn>,
    /// Each turn's chunks, as the roll-up cuts it.
    cuts: Vec<Vec<Range<usize>>>,
    lines: HashMap<Key, String>,
}

impl Branch {
    /// The branch ending at item `tip`, none for no item, with `lines` by key.
    pub fn new(tree: Tree, tip: Option<u64>, lines: HashMap<Key, String>) -> Branch {
        let b = tip.map(|t| rollup::branch(&tree, t as usize));
        let (path, parts) = b.map_or_else(Default::default, |b| (b.seqs, b.parts));
        let (xs, cuts) = parts.into_iter().map(|p| (p.turn, p.chunks)).unzip();
        Branch {
            tree,
            path,
            xs,
            cuts,
            lines,
        }
    }

    /// The branch of `s` ending at item `tip`, with every line of the thread.
    pub fn load(s: &Snapshot, tip: Option<u64>) -> Result<Branch> {
        let lines = s
            .all_lines()?
            .into_iter()
            .map(|(k, l)| (k, l.text))
            .collect();
        Ok(Branch::new(s.tree()?, tip, lines))
    }

    /// The view: the instruction, the fold over the closed turns, the user rail and the
    /// open turn, in at most [`VIEW`] bytes.
    pub fn view(&self, thread: &str) -> String {
        let (head, rail, open) = self.frame(thread);
        let fold = section(FOLD, &self.render(self.budget(&head, &rail, &open)));
        format!("{head}{fold}{rail}{open}")
    }

    /// The keys of the lines the fold wants and lacks.
    pub fn needed(&self, thread: &str) -> Vec<Key> {
        let (head, rail, open) = self.frame(thread);
        self.plan(self.budget(&head, &rail, &open))
            .into_iter()
            .filter(|&c| self.text(c).is_none())
            .map(|c| self.key(c))
            .collect()
    }

    /// What `id` holds, one or two levels down: a node into the lines two levels under it
    /// (`t+2` into its two turn lines), a turn into its chunk lines and items, items into the
    /// chunk lines inside them and their other items, one item whole; turns that are no node
    /// into the fewest nodes that tile them. A line not built shows what is built under it.
    /// Tool output only with `tool_output`. `None` if `id` is not on this branch.
    pub fn zoom(&self, id: Id, tool_output: bool) -> Option<String> {
        let listing = match tool_output {
            true => Cut::WHOLE,
            false => Cut {
                words: WHOLE,
                input: CALL,
                output: None,
            },
        };
        let lines = match id {
            Id::Turns { first, n } => {
                let end = first.checked_add(n)?;
                if end > self.xs.len() as u64 {
                    return None;
                }
                let nodes = match Coord::at(first, n, end) {
                    Some(c) if c.l == 0 => {
                        let span = self.xs[first as usize].span.clone();
                        return Some(self.inside(span, listing).join("\n"));
                    }
                    Some(c) => under(c, c.l.min(2)),
                    None => tile(first, end),
                };
                let mut out = Vec::new();
                for c in nodes {
                    self.push(c, &mut out);
                }
                out
            }
            Id::Items { turn, first, n } => {
                let x = self.xs.get(turn as usize)?;
                let a = x.span.start.checked_add(first as usize)?;
                let b = a.checked_add(n as usize)?;
                if b > x.span.end {
                    return None;
                }
                match n {
                    1 => vec![self.item_line(
                        a,
                        Cut {
                            output: tool_output.then_some(WHOLE),
                            ..Cut::WHOLE
                        },
                    )],
                    _ => self.inside(a..b, listing),
                }
            }
        };
        Some(lines.join("\n"))
    }

    fn frame(&self, thread: &str) -> (String, String, String) {
        let mut head = format!(
            "# Thread `{thread}`\n\n{}",
            INSTRUCTION.replace("{name}", thread)
        );
        if self.xs.is_empty() {
            head += "\nThe thread holds nothing yet.\n";
        }
        (
            head,
            section(USER, &self.rail()),
            section(CURRENT, &self.current()),
        )
    }

    /// The fold's share: what the rest leaves of [`VIEW`].
    fn budget(&self, head: &str, rail: &str, open: &str) -> usize {
        VIEW.saturating_sub(head.len() + rail.len() + open.len() + FOLD.len() + 5)
    }

    fn closed(&self) -> u64 {
        self.xs.len().saturating_sub(1) as u64
    }

    fn item(&self, p: usize) -> &Item {
        &self.tree.items()[self.path[p]]
    }

    /// The id of the items at positions `r`, all in one turn.
    fn items_id(&self, r: &Range<usize>) -> Id {
        let p = self.tree.place(self.path[r.start]);
        Id::Items {
            turn: p.turn,
            first: p.item,
            n: r.len() as u64,
        }
    }

    /// The positions on the path of the turns `c` covers.
    fn span(&self, c: Coord) -> Range<usize> {
        self.xs[c.id() as usize].span.start..self.xs[c.end() as usize - 1].span.end
    }

    fn key(&self, c: Coord) -> Key {
        Key {
            level: 1 + c.l,
            last: self.path[self.span(c).end - 1] as u64,
        }
    }

    fn text(&self, c: Coord) -> Option<&str> {
        self.lines.get(&self.key(c)).map(String::as_str)
    }

    fn turn_at(&self, p: usize) -> usize {
        self.xs.partition_point(|x| x.span.end <= p)
    }

    /// Where turn `e`'s steps start: after its message.
    fn body(&self, e: usize) -> usize {
        let s = self.xs[e].span.start;
        s + usize::from(self.item(s).kind.opens())
    }

    /// The chunk lines built in turn `e`, consecutive from its first step, each with the
    /// positions it covers.
    fn chunks(&self, e: usize) -> Vec<(Range<usize>, &str)> {
        self.cuts[e]
            .iter()
            .map_while(|r| {
                let key = Key {
                    level: 0,
                    last: self.path[r.end - 1] as u64,
                };
                Some((r.clone(), self.lines.get(&key)?.as_str()))
            })
            .collect()
    }

    /// Item `p` as a line, cut by `cut`.
    fn item_line(&self, p: usize, cut: Cut) -> String {
        let id = Id::item(self.tree.place(self.path[p]));
        item_line(id, self.item(p), cut)
    }

    fn line(id: Id, text: &str) -> String {
        format!("{id}|{}", flat(text))
    }

    fn placeholder(id: Id) -> String {
        format!("{id}|(not summarized yet)")
    }

    /// The fold with an unbuilt line reckoned at [`LINE`] bytes: the lines the view wants.
    fn plan(&self, budget: usize) -> Vec<Coord> {
        fold(self.closed(), budget, |c| {
            let id = Id::node(c).to_string();
            id.len() + 2 + self.text(c).map_or(LINE, |t| flat(t).len())
        })
    }

    /// The fold's lines: each built line of the plan, else what is built under it; the oldest
    /// of those fallbacks become placeholders while over `budget`.
    fn render(&self, budget: usize) -> Vec<String> {
        let plan = self.plan(budget);
        let mut parts: Vec<Vec<String>> = plan
            .iter()
            .map(|&c| {
                let mut out = Vec::new();
                self.push(c, &mut out);
                out
            })
            .collect();
        let mut size: usize = parts.iter().map(|p| bytes(p)).sum();
        for (part, &c) in parts.iter_mut().zip(&plan) {
            if size <= budget {
                break;
            }
            if part.len() > 1 {
                let one = vec![Branch::placeholder(Id::node(c))];
                size = size + bytes(&one) - bytes(part);
                *part = one;
            }
        }
        parts.concat()
    }

    /// Line `c`, else the lines built under it, else a placeholder naming it.
    fn push(&self, c: Coord, out: &mut Vec<String>) {
        match self.text(c) {
            Some(t) => out.push(Branch::line(Id::node(c), t)),
            None if !self.built_under(c) => out.push(Branch::placeholder(Id::node(c))),
            None if c.l > 0 => {
                for k in c.children() {
                    self.push(k, out);
                }
            }
            None => {
                let e = c.i as usize;
                let span = self.span(c);
                let body = self.body(e);
                out.extend((span.start..body).map(|p| self.item_line(p, cut(LINE))));
                let chunks = self.chunks(e);
                out.extend(
                    chunks
                        .iter()
                        .map(|(r, t)| Branch::line(self.items_id(r), t)),
                );
                let rest = chunks.last().map_or(body, |c| c.0.end)..span.end;
                if !rest.is_empty() {
                    out.push(Branch::placeholder(self.items_id(&rest)));
                }
            }
        }
    }

    fn built_under(&self, c: Coord) -> bool {
        match c.l {
            0 => !self.chunks(c.i as usize).is_empty(),
            _ => c
                .children()
                .iter()
                .any(|&k| self.text(k).is_some() || self.built_under(k)),
        }
    }

    /// The newest messages of the user in closed turns, oldest first, in about [`RAIL`]
    /// bytes; the newest always.
    fn rail(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut size = 0;
        for x in self.xs[..self.closed() as usize].iter().rev() {
            let p = x.span.start;
            if !matches!(self.item(p).kind, Kind::User(_)) {
                continue;
            }
            let l = self.item_line(p, cut(RAIL));
            if !out.is_empty() && size + l.len() + 1 > RAIL {
                break;
            }
            size += l.len() + 1;
            out.push(l);
        }
        out.reverse();
        out
    }

    /// The open turn: its message, then its chunk lines and the steps after them, the
    /// newest that fit in [`OPEN`] bytes; the newest item always.
    fn current(&self) -> Vec<String> {
        let Some(e) = self.xs.len().checked_sub(1) else {
            return Vec::new();
        };
        let x = &self.xs[e];
        let body = self.body(e);
        let mut out: Vec<String> = (x.span.start..body)
            .map(|p| self.item_line(p, cut(WORDS)))
            .collect();
        let chunks = self.chunks(e);
        let rest = chunks.last().map_or(body, |c| c.0.end);
        let mut units: Vec<(Range<usize>, Vec<String>)> = chunks
            .iter()
            .map(|(r, t)| (r.clone(), vec![Branch::line(self.items_id(r), t)]))
            .collect();
        for s in &x.steps {
            let r = s.start.max(rest)..s.end;
            if !r.is_empty() {
                let lines = r.clone().map(|p| self.item_line(p, cut(WORDS))).collect();
                units.push((r, lines));
            }
        }
        let mut size = bytes(&out);
        let mut keep = units.len();
        while keep > 0 {
            let (r, lines) = &units[keep - 1];
            let b = bytes(lines);
            if size + b > OPEN {
                if keep < units.len() {
                    break;
                }
                if lines.len() > 1 {
                    let items: Vec<_> = r
                        .clone()
                        .zip(lines.clone())
                        .map(|(p, l)| (p..p + 1, vec![l]))
                        .collect();
                    units.splice(keep - 1..keep, items);
                    keep = units.len();
                    continue;
                }
            }
            size += b;
            keep -= 1;
        }
        if keep > 0 {
            let id = self.items_id(&(units[0].0.start..units[keep - 1].0.end));
            out.push(format!("{id}|(not shown)"));
        }
        out.extend(units.drain(keep..).flat_map(|u| u.1));
        out
    }

    /// What `r`, within one turn, holds: the chunk lines inside it other than itself, and
    /// its other items, cut by `cut`.
    fn inside(&self, r: Range<usize>, cut: Cut) -> Vec<String> {
        let chunks: Vec<_> = self
            .chunks(self.turn_at(r.start))
            .into_iter()
            .filter(|(c, _)| r.start <= c.start && c.end <= r.end && *c != r)
            .collect();
        let mut out = Vec::new();
        let mut cs = chunks.iter().peekable();
        let mut p = r.start;
        while p < r.end {
            match cs.next_if(|c| c.0.start == p) {
                Some((c, t)) => {
                    out.push(Branch::line(self.items_id(c), t));
                    p = c.end;
                }
                None => {
                    out.push(self.item_line(p, cut));
                    p += 1;
                }
            }
        }
        out
    }
}

/// An item's words cut to `words` bytes, and its tool input and output to [`TOOL`].
fn cut(words: usize) -> Cut {
    Cut {
        words,
        input: words.min(TOOL),
        output: Some(words.min(TOOL)),
    }
}

/// The bytes `lines` take, one newline each.
fn bytes(lines: &[String]) -> usize {
    lines.iter().map(|l| l.len() + 1).sum()
}

fn section(title: &str, lines: &[String]) -> String {
    match lines.is_empty() {
        true => String::new(),
        false => format!("\n## {title}\n{}\n", lines.join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use serde_json::Value;

    use super::*;
    use crate::chat::model::{Marker, SourceId};

    fn source(entry: usize) -> SourceId {
        SourceId {
            harness: "test".into(),
            session: "s".into(),
            entry: entry.to_string(),
        }
    }

    /// Items in one chain, from kinds: `u` user, `w` work, `t` text, `c` call, `r` result,
    /// each saying `text`.
    fn chain(spec: &str, text: impl Fn(usize) -> String) -> Vec<Item> {
        spec.chars()
            .enumerate()
            .map(|(i, k)| Item {
                id: source(i),
                parent: i.checked_sub(1).map(source),
                ts: Timestamp::UNIX_EPOCH,
                kind: match k {
                    'u' => Kind::User(text(i)),
                    'w' => Kind::Work {
                        from: "researcher".into(),
                        text: text(i),
                    },
                    't' => Kind::Text(text(i)),
                    'c' => Kind::ToolCall {
                        id: i.to_string(),
                        name: "Bash".into(),
                        input: Value::String(text(i)),
                    },
                    'r' => Kind::ToolResult {
                        id: i.to_string(),
                        output: text(i),
                        is_error: false,
                    },
                    _ => Kind::Marker(Marker::Interrupt),
                },
                raw: String::new(),
            })
            .collect()
    }

    fn branch(items: Vec<Item>, lines: &[(u32, u64, &str)]) -> Branch {
        let tip = items.len().checked_sub(1).map(|t| t as u64);
        let lines = lines
            .iter()
            .map(|&(level, last, t)| (Key { level, last }, t.to_string()))
            .collect();
        Branch::new(Tree::new(items), tip, lines)
    }

    fn short(i: usize) -> String {
        format!("m{i}")
    }

    /// The lines of one section of `view`.
    fn section_lines<'v>(view: &'v str, title: &str) -> Vec<&'v str> {
        let Some((_, rest)) = view.split_once(&format!("\n## {title}\n")) else {
            return Vec::new();
        };
        rest.split("\n\n## ").next().unwrap().lines().collect()
    }

    fn ids(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.split('|').next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn the_fold_merges_the_most_due_pair_first() {
        let names = |v: Vec<Coord>| -> Vec<String> { v.iter().map(|c| c.to_string()).collect() };
        assert_eq!(
            names(fold(10, 600, |_| 100)),
            ["0+4", "4+2", "6", "7", "8", "9"]
        );
        assert_eq!(
            names(fold(10, 300, |_| 100)),
            ["0+4", "4+4", "8+2"],
            "due counts from a pair's last turn: 8-9 merges before 0-7"
        );
        assert_eq!(names(fold(10, 1000, |_| 100)).len(), 10);
        assert_eq!(
            names(fold(3, 0, |_| 100)),
            ["0+2", "2"],
            "stops when no pair is left"
        );
        assert!(fold(0, 0, |_| 100).is_empty());
    }

    #[test]
    fn ranges_tile_with_the_fewest_nodes() {
        let names = |v: Vec<Coord>| -> Vec<String> { v.iter().map(|c| c.to_string()).collect() };
        assert_eq!(names(tile(5, 8)), ["5", "6+2"]);
        assert_eq!(names(tile(0, 7)), ["0+4", "4+2", "6"]);
        assert_eq!(names(tile(3, 13)), ["3", "4+4", "8+4", "12"]);
        assert_eq!(names(tile(8, 16)), ["8+8"]);
        assert_eq!(
            names(under(Coord::new(3, 1), 2)),
            ["8+2", "10+2", "12+2", "14+2"]
        );
        assert_eq!(names(under(Coord::new(1, 2), 1)), ["4", "5"]);
    }

    #[test]
    fn built_lines_fill_the_fold_and_the_rest_fall_back() {
        let b = branch(chain("utututututututut", short), &[]);
        assert_eq!(b.closed(), 7);
        let all = b.render(VIEW);
        assert_eq!(all.len(), 7);
        assert_eq!(all[0], "0|(not summarized yet)");
        let b = branch(
            chain("utututututututut", short),
            &[
                (1, 1, "first"),
                (1, 3, "second"),
                (1, 7, "fourth"),
                (2, 3, "first two"),
            ],
        );
        assert_eq!(
            b.render(150),
            [
                "0+2|first two",
                "2|(not summarized yet)",
                "3|fourth",
                "4+2|(not summarized yet)",
                "6|(not summarized yet)",
            ],
            "0+4 falls back to what is built under it"
        );
        let key = |last| Key { level: 1, last };
        assert_eq!(
            b.needed("dev"),
            [key(5), key(9), key(11), key(13)],
            "every turn fits: the fold wants the unbuilt turn lines"
        );
    }

    /// Item `i` says `m<i>`, padded to about `pad(i)` bytes.
    fn padded(pad: impl Fn(usize) -> usize) -> impl Fn(usize) -> String {
        move |i| format!("m{i}{}", " x".repeat(pad(i) / 2))
    }

    #[test]
    fn an_unbuilt_turn_shows_its_message_and_chunks() {
        // Three steps of a 20 KB reply: over the roll-up's 48 KB input, cut 1-6 and 7-9.
        let text = padded(|i| if [1, 4, 7].contains(&i) { 20_000 } else { 0 });
        let render =
            |lines: &[(u32, u64, &str)]| branch(chain("utcrtcrtcrut", &text), lines).render(VIEW);
        assert_eq!(
            render(&[(0, 6, "read the config")]),
            [
                "0.0|user: m0",
                "0.1+6|read the config",
                "0.7+3|(not summarized yet)",
            ]
        );
        assert_eq!(
            render(&[(0, 6, "read the config"), (0, 9, "ran the tests")]),
            [
                "0.0|user: m0",
                "0.1+6|read the config",
                "0.7+3|ran the tests"
            ]
        );
        assert_eq!(
            render(&[(0, 9, "ran the tests")]),
            ["0|(not summarized yet)"],
            "chunk lines count from the first"
        );
    }

    /// Level-1 lines over each of `n` two-item turns, each `text`.
    fn turn_lines(n: u64, text: &str) -> Vec<(u32, u64, String)> {
        (0..n)
            .map(|e| (1, 2 * e + 1, format!("{e} {text}")))
            .collect()
    }

    fn as_refs(lines: &[(u32, u64, String)]) -> Vec<(u32, u64, &str)> {
        lines.iter().map(|(l, k, t)| (*l, *k, t.as_str())).collect()
    }

    #[test]
    fn fallbacks_collapse_to_placeholders_over_budget() {
        let lines = turn_lines(64, &"x".repeat(200));
        let b = branch(chain(&"ut".repeat(65), short), &as_refs(&lines));
        let fold = b.render(4_000);
        assert!(bytes(&fold) <= 4_000, "{}", bytes(&fold));
        assert!(fold[0].starts_with("0+") && fold[0].contains("not summarized yet"));
        assert!(
            fold.last().unwrap().starts_with("63|63 x"),
            "the newest stay built"
        );
    }

    #[test]
    fn the_view_keeps_its_budget() {
        let spec = "ut".repeat(3000) + "u";
        let lines = turn_lines(3000, &"y".repeat(500));
        let big = |i: usize| format!("{i} {}", "z".repeat(20_000));
        for b in [
            branch(chain(&spec, short), &as_refs(&lines)),
            branch(chain(&spec, big), &as_refs(&lines)),
            branch(chain(&spec, big), &[]),
        ] {
            let v = b.view("dev");
            assert!(v.len() <= VIEW, "{}", v.len());
            assert!(v.starts_with("# Thread `dev`\n\nThis is the thread `dev`"));
            assert!(section_lines(&v, FOLD).len() > 10);
            assert!(!section_lines(&v, USER).is_empty());
            assert!(section_lines(&v, CURRENT)[0].starts_with("3000.0|user: "));
        }
        let empty = branch(Vec::new(), &[]).view("dev");
        assert!(empty.ends_with("The thread holds nothing yet.\n"));
    }

    #[test]
    fn the_open_turn_keeps_its_message_and_newest_steps() {
        let text = |i: usize| match i {
            2 => "run the benchmarks".to_string(),
            _ => format!("{i}:{}", "o".repeat(9_000)),
        };
        let open = branch(chain("utuctcrcrcrtcrt", text), &[]).current();
        assert_eq!(open[0], "1.0|user: run the benchmarks");
        assert_eq!(open[1], "1.1+8|(not shown)");
        assert_eq!(
            ids(&open[2..]),
            ["1.9", "1.10", "1.11", "1.12"],
            "whole steps"
        );
        assert!(open[3].starts_with("1.10|tool Bash: ") && open[3].contains("bytes cut ...]"));
        assert!(bytes(&open) <= OPEN);

        let flood = "u".to_string() + &"c".repeat(40) + &"r".repeat(40);
        let open = branch(chain(&flood, text), &[]).current();
        assert!(bytes(&open) <= OPEN, "{}", bytes(&open));
        assert!(open[1].starts_with("0.1+") && open[1].ends_with("|(not shown)"));
        assert!(
            open.last().unwrap().starts_with("0.80|output: "),
            "a step too big for the budget keeps its newest items"
        );

        // Four steps of a 20 KB reply: chunks 1-6 and 7-12, the second still open.
        let text = padded(|i| if i % 3 == 1 { 20_000 } else { 0 });
        let lines = [(0, 6, "read it")];
        let open = branch(chain("utcrtcrtcrtcr", text), &lines).current();
        assert_eq!(
            ids(&open),
            ["0.0", "0.1+6", "0.7", "0.8", "0.9", "0.10", "0.11", "0.12"]
        );
        assert_eq!(open[1], "0.1+6|read it");
    }

    #[test]
    fn the_rail_holds_the_newest_user_messages() {
        let text = |i: usize| match i % 4 {
            0 => format!("ask {i} {}", "q".repeat(1_500)),
            _ => format!("say {i}"),
        };
        let rail = branch(chain(&"utwt".repeat(20), text), &[]).rail();
        assert_eq!(rail.len(), 5);
        assert!(rail[0].starts_with("30.0|user: ask 60"));
        assert!(rail[4].starts_with("38.0|user: ask 76"));
        let huge = |i: usize| format!("{i}{}", "q".repeat(20_000));
        let rail = branch(chain("utu", huge), &[]).rail();
        assert_eq!(rail.len(), 1, "the newest always, cut");
        assert!(rail[0].len() < RAIL + 100 && rail[0].contains("bytes cut ...]"));
    }

    #[test]
    fn zoom_opens_nodes_turns_chunks_and_items() {
        // Turn 2 (items 4-8) holds two steps of about 28 and 24 KB: over 48 KB, cut 5-7 and 8.
        let text = padded(|i| match i {
            5 | 8 => 24_000,
            7 => 10_000,
            _ => 0,
        });
        let b = branch(
            chain("utututcrtutut", &text),
            &[
                (1, 1, "one"),
                (1, 3, "two"),
                (2, 3, "one and two"),
                (0, 7, "ran"),
            ],
        );
        let zoom = |id: &str, out: bool| b.zoom(Id::parse(id).unwrap(), out);
        let heads = |id: &str| -> Vec<String> {
            zoom(id, false)
                .unwrap()
                .lines()
                .map(|l| l.chars().take_while(|&c| c != ' ').collect())
                .collect()
        };
        assert_eq!(zoom("0+2", false).unwrap(), "0|one\n1|two");
        assert_eq!(
            zoom("0+4", false).unwrap(),
            "0|one\n1|two\n2.0|user: m4\n2.1+3|ran\n2.4|(not summarized yet)\n\
             3|(not summarized yet)",
            "four turn lines; turn 2 is not built, so what is built under it"
        );
        assert_eq!(
            heads("2"),
            ["2.0|user:", "2.1+3|ran", "2.4|ai:"],
            "a turn: its message, chunk lines, items"
        );
        assert_eq!(
            zoom("2.1+3", false).unwrap(),
            format!(
                "2.1|ai: {}\n2.2|tool Bash: m6\n2.3|output: (1 line, {} bytes)",
                text(5),
                text(7).len()
            ),
            "a chunk: its items, the output as its size"
        );
        assert!(
            zoom("2.1+3", true)
                .unwrap()
                .ends_with(&format!("2.3|output: {}", text(7)))
        );
        assert_eq!(
            zoom("2.3", false).unwrap(),
            format!("2.3|output: (1 line, {} bytes)", text(7).len())
        );
        assert_eq!(
            zoom("2.3", true).unwrap(),
            format!("2.3|output: {}", text(7))
        );
        assert_eq!(
            heads("1+3"),
            ["1|two", "2.0|user:", "2.1+3|ran", "2.4|(not", "3|(not"]
        );
        assert_eq!(heads("4"), ["4.0|user:", "4.1|ai:"], "the open turn");
        for bad in ["5", "4+2", "0+8", "2.5", "2.3+3", "9.0"] {
            assert_eq!(zoom(bad, false), None, "{bad}");
        }

        let lines = [(2, 3, "a"), (2, 7, "b"), (2, 11, "c"), (2, 15, "d")];
        let b = branch(chain(&"ut".repeat(9), short), &lines);
        assert_eq!(
            b.zoom(Id::parse("0+8").unwrap(), false).unwrap(),
            "0+2|a\n2+2|b\n4+2|c\n6+2|d",
            "two levels down"
        );
    }
}
