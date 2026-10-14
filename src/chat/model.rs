use std::{collections::HashMap, ops::Range};

use jiff::Timestamp;
use serde_json::Value;

/// One entry of one harness session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceId {
    pub harness: String,
    pub session: String,
    pub entry: String,
}

/// What an item says.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// The user's words.
    User(String),
    /// A report or message from another agent, which the assistant answers.
    Work {
        from: String,
        text: String,
    },
    /// The assistant's words.
    Text(String),
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    Marker(Marker),
}

/// A harness event that is not content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// The harness compacted its context here.
    Compaction,
    /// The harness's own summary of the context it compacted.
    Summary,
    /// The user stopped the run.
    Interrupt,
}

/// One normalised source entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: SourceId,
    pub parent: Option<SourceId>,
    pub ts: Timestamp,
    pub kind: Kind,
    /// The source line, verbatim.
    pub raw: String,
}

/// Items linked by their parents. A parent that is not an earlier item makes its item a root.
pub struct Tree {
    items: Vec<Item>,
    parents: Vec<Option<usize>>,
    tips: Vec<bool>,
}

impl Tree {
    pub fn new(items: Vec<Item>) -> Tree {
        let mut at = HashMap::with_capacity(items.len());
        let mut parents = Vec::with_capacity(items.len());
        let mut tips = vec![true; items.len()];
        for (i, item) in items.iter().enumerate() {
            let p = item.parent.as_ref().and_then(|p| at.get(p).copied());
            if let Some(p) = p {
                tips[p] = false;
            }
            parents.push(p);
            at.insert(&item.id, i);
        }
        Tree {
            items,
            parents,
            tips,
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The items nothing follows, in item order: the last one is the newest item.
    pub fn tips(&self) -> Vec<usize> {
        (0..self.items.len()).filter(|&i| self.tips[i]).collect()
    }

    /// The items from the root down to `tip`.
    pub fn path(&self, tip: usize) -> Vec<&Item> {
        let mut out = Vec::new();
        let mut at = Some(tip);
        while let Some(i) = at {
            out.push(&self.items[i]);
            at = self.parents[i];
        }
        out.reverse();
        out
    }
}

/// A message (from the user or another agent) and the steps that answer it, up to the next
/// message on the path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    /// Positions on the path. It starts with the message, unless the path does not.
    pub span: Range<usize>,
    /// The rest of the span, cut before each assistant text.
    pub steps: Vec<Range<usize>>,
    /// A message follows it on the path.
    pub closed: bool,
}

/// The exchanges along `path`, in order.
pub fn exchanges(path: &[&Item]) -> Vec<Exchange> {
    let user = |i: usize| matches!(path[i].kind, Kind::User(_) | Kind::Work { .. });
    let mut starts: Vec<usize> = (0..path.len()).filter(|&i| user(i)).collect();
    if !path.is_empty() && !user(0) {
        starts.insert(0, 0);
    }
    let ends: Vec<usize> = starts.iter().skip(1).copied().chain([path.len()]).collect();
    starts
        .iter()
        .zip(&ends)
        .map(|(&start, &end)| {
            let mut steps: Vec<Range<usize>> = Vec::new();
            let body = start + usize::from(user(start));
            for (i, item) in (body..end).zip(&path[body..end]) {
                match steps.last_mut() {
                    Some(s) if !matches!(item.kind, Kind::Text(_)) => s.end = i + 1,
                    _ => steps.push(i..i + 1),
                }
            }
            Exchange {
                span: start..end,
                steps,
                closed: end < path.len(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(entry: &str) -> SourceId {
        SourceId {
            harness: "test".into(),
            session: "s".into(),
            entry: entry.into(),
        }
    }

    /// Items from `(entry, parent, kind)`: `u` user, `w` work, `t` text, `c` call, `r` result, `m` marker.
    fn items(spec: &[(&str, &str, char)]) -> Vec<Item> {
        spec.iter()
            .map(|&(e, p, k)| Item {
                id: id(e),
                parent: (!p.is_empty()).then(|| id(p)),
                ts: Timestamp::UNIX_EPOCH,
                kind: match k {
                    'u' => Kind::User(e.into()),
                    'w' => Kind::Work {
                        from: "agent".into(),
                        text: e.into(),
                    },
                    't' => Kind::Text(e.into()),
                    'c' => Kind::ToolCall {
                        id: e.into(),
                        name: "Read".into(),
                        input: Value::Null,
                    },
                    'r' => Kind::ToolResult {
                        id: e.into(),
                        output: String::new(),
                        is_error: false,
                    },
                    _ => Kind::Marker(Marker::Compaction),
                },
                raw: String::new(),
            })
            .collect()
    }

    fn entries(path: &[&Item]) -> Vec<String> {
        path.iter().map(|i| i.id.entry.clone()).collect()
    }

    #[test]
    fn tips_and_paths_follow_parents() {
        let tree = Tree::new(items(&[
            ("u1", "", 'u'),
            ("t1", "u1", 't'),
            ("u2", "t1", 'u'),
            ("t2", "u2", 't'),
            ("u2b", "t1", 'u'),
            ("t3", "u2b", 't'),
            ("x", "gone", 't'),
            ("y", "z", 't'),
            ("z", "", 't'),
        ]));
        assert_eq!(tree.tips(), [3, 5, 6, 7, 8]);
        assert_eq!(entries(&tree.path(3)), ["u1", "t1", "u2", "t2"]);
        assert_eq!(entries(&tree.path(5)), ["u1", "t1", "u2b", "t3"]);
        assert_eq!(entries(&tree.path(6)), ["x"]);
        assert_eq!(entries(&tree.path(7)), ["y"], "a parent must come first");
    }

    #[test]
    fn exchanges_and_steps() {
        let all = items(&[
            ("t0", "", 't'),
            ("u1", "", 'u'),
            ("t1", "", 't'),
            ("c1", "", 'c'),
            ("r1", "", 'r'),
            ("c2", "", 'c'),
            ("r2", "", 'r'),
            ("t2", "", 't'),
            ("c3", "", 'c'),
            ("r3", "", 'r'),
            ("t3", "", 't'),
            ("m", "", 'm'),
            ("u2", "", 'u'),
            ("c4", "", 'c'),
            ("r4", "", 'r'),
            ("t4", "", 't'),
            ("w1", "", 'w'),
        ]);
        let path: Vec<&Item> = all.iter().collect();
        let x = |span: Range<usize>, steps: &[(usize, usize)], closed: bool| Exchange {
            span,
            steps: steps.iter().map(|&(a, b)| a..b).collect(),
            closed,
        };
        assert_eq!(
            exchanges(&path),
            [
                x(0..1, &[(0, 1)], true),
                x(1..12, &[(2, 7), (7, 10), (10, 12)], true),
                x(12..16, &[(13, 15), (15, 16)], true),
                x(16..17, &[], false),
            ]
        );
        assert_eq!(exchanges(&path[1..7]), [x(0..6, &[(1, 6)], false)]);
        assert_eq!(exchanges(&[]), []);
    }
}
