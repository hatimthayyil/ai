use std::{collections::HashMap, fmt, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use serde_json::Value;

use crate::{
    chat::model::{Item, Kind, Marker, SourceId},
    memory::Who,
    record::{self, Seg},
};

/// The columns of an index segment.
pub const COLUMNS: [&str; 5] = ["ts", "harness", "session", "entry", "parent"];

/// An item's place in its thread: its number, its parent's and its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub seq: u64,
    pub parent: Option<u64>,
    pub harness: String,
    pub session: String,
    pub entry: String,
    pub ts: Timestamp,
}

/// A harness session bound to a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub harness: String,
    pub session: String,
    pub transcript: Option<PathBuf>,
    /// The item its roots hang on: the thread's newest item when it was first attached.
    pub graft: Option<u64>,
    /// The item of its transcript's newest entry: the end of its branch.
    pub tip: Option<u64>,
    /// When it was last attached.
    pub attached: Timestamp,
    /// When it was detached; none while attached.
    pub detached: Option<Timestamp>,
}

/// A subagent of an attached session, its items kept beside the main tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub harness: String,
    pub agent: String,
    pub agent_type: String,
    pub description: String,
    /// The Agent tool call that started it, in the main tree.
    pub call: Option<u64>,
}

/// What a summary line covers: its level, and the last item of its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub level: u32,
    pub last: u64,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}@{}", self.level, self.last)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Consecutive steps of an exchange too long for one call.
    Chunk,
    /// One exchange.
    Exchange,
    /// Two adjacent lines merged.
    Node,
}

const LINE_KINDS: [(LineKind, &str); 3] = [
    (LineKind::Chunk, "chunk"),
    (LineKind::Exchange, "exchange"),
    (LineKind::Node, "node"),
];

impl LineKind {
    pub fn name(self) -> &'static str {
        LINE_KINDS
            .iter()
            .find(|k| k.0 == self)
            .expect("every kind")
            .1
    }

    fn parse(s: &str) -> Option<LineKind> {
        LINE_KINDS.iter().find(|k| k.1 == s).map(|k| k.0)
    }
}

/// A summary line of a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub ts: Timestamp,
    pub kind: LineKind,
    pub who: Who,
    pub text: String,
}

/// A value of one word: whitespace becomes `_`.
pub fn word(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect()
}

/// `s` as a name in a tree path: letters, digits, `.`, `_`, `-`, not starting with `.`.
pub fn check_id<'s>(s: &'s str, what: &str) -> Result<&'s str> {
    let ok = !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if !ok {
        bail!("'{s}' is not a {what} id: letters, digits, '.', '_' and '-'.");
    }
    Ok(s)
}

fn fields(b: &[u8]) -> Result<(HashMap<&str, &str>, &str)> {
    let (fields, body) = record::decode(b).ok_or_else(|| anyhow!("corrupt"))?;
    Ok((fields.into_iter().collect(), body))
}

fn ts(v: Option<&&str>) -> Result<Option<Timestamp>> {
    v.map(|v| v.parse().context("a bad time")).transpose()
}

fn parse_seq(v: Option<&&str>) -> Result<Option<u64>> {
    v.filter(|v| !v.is_empty())
        .map(|v| v.parse().context("a bad item number"))
        .transpose()
}

fn show_seq(n: Option<u64>) -> String {
    n.map_or_else(String::new, |n| n.to_string())
}

impl Head {
    /// Its line under the columns of `seg`.
    pub fn line(&self, seg: &Seg) -> String {
        seg.line(|c| {
            Some(match c {
                "ts" => self.ts.to_string(),
                "harness" => word(&self.harness),
                "session" => word(&self.session),
                "entry" => word(&self.entry),
                "parent" => show_seq(self.parent),
                _ => return None,
            })
        })
    }

    /// Item `seq`, on `line` of `seg`.
    pub fn parse(seq: u64, seg: &Seg, line: &str) -> Result<Head> {
        let vs: HashMap<&str, &str> = seg.values(line).collect();
        let text = |k: &str| vs.get(k).map_or_else(String::new, |v| v.to_string());
        Ok(Head {
            seq,
            parent: parse_seq(vs.get("parent"))?,
            harness: text("harness"),
            session: text("session"),
            entry: text("entry"),
            ts: ts(vs.get("ts"))?.context("no time")?,
        })
    }

    /// The source id it was made from.
    pub fn source(&self) -> SourceId {
        SourceId {
            harness: self.harness.clone(),
            session: self.session.clone(),
            entry: self.entry.clone(),
        }
    }
}

const MARKERS: [(Marker, &str); 3] = [
    (Marker::Compaction, "compaction"),
    (Marker::Summary, "summary"),
    (Marker::Interrupt, "interrupt"),
];

/// An item's blob: its kind and the kind's fields as headers, its content as the body.
pub fn encode_kind(kind: &Kind) -> Vec<u8> {
    let (name, fields, body): (&str, Vec<(&str, &str)>, String) = match kind {
        Kind::User(t) => ("user", vec![], t.clone()),
        Kind::Work { from, text } => ("work", vec![("from", from.as_str())], text.clone()),
        Kind::Text(t) => ("text", vec![], t.clone()),
        Kind::ToolCall { id, name, input } => (
            "call",
            vec![("call", id.as_str()), ("name", name.as_str())],
            input.to_string(),
        ),
        Kind::ToolResult {
            id,
            output,
            is_error,
        } => (
            "result",
            vec![
                ("call", id.as_str()),
                ("error", if *is_error { "true" } else { "" }),
            ],
            output.clone(),
        ),
        Kind::Marker(m) => {
            let m = MARKERS.iter().find(|x| x.0 == *m).expect("every marker");
            ("marker", vec![("marker", m.1)], String::new())
        }
    };
    record::encode([("kind", name)].into_iter().chain(fields), &body)
}

pub fn decode_kind(b: &[u8]) -> Result<Kind> {
    let (f, body) = fields(b)?;
    let get = |k: &str| f.get(k).map_or_else(String::new, |v| v.to_string());
    Ok(match f.get("kind").copied() {
        Some("user") => Kind::User(body.into()),
        Some("work") => Kind::Work {
            from: get("from"),
            text: body.into(),
        },
        Some("text") => Kind::Text(body.into()),
        Some("call") => Kind::ToolCall {
            id: get("call"),
            name: get("name"),
            input: serde_json::from_str::<Value>(body).context("a bad tool input")?,
        },
        Some("result") => Kind::ToolResult {
            id: get("call"),
            output: body.into(),
            is_error: f.get("error") == Some(&"true"),
        },
        Some("marker") => Kind::Marker(
            MARKERS
                .iter()
                .find(|m| Some(&m.1) == f.get("marker"))
                .context("an unknown marker")?
                .0,
        ),
        _ => bail!("an unknown kind"),
    })
}

/// The item `head` names, from its blob and its raw line; `parent` is its parent's source.
pub fn item(head: &Head, parent: Option<SourceId>, blob: &[u8], raw: &[u8]) -> Result<Item> {
    Ok(Item {
        id: head.source(),
        parent,
        ts: head.ts,
        kind: decode_kind(blob)?,
        raw: String::from_utf8(raw.to_vec()).context("a raw line not UTF-8")?,
    })
}

impl Binding {
    pub fn is_attached(&self) -> bool {
        self.detached.is_none()
    }

    pub fn encode(&self) -> Vec<u8> {
        let transcript = self
            .transcript
            .as_ref()
            .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
        let detached = self.detached.map_or_else(String::new, |t| t.to_string());
        record::encode(
            [
                ("transcript", transcript),
                ("graft", show_seq(self.graft)),
                ("tip", show_seq(self.tip)),
                ("attached", self.attached.to_string()),
                ("detached", detached),
            ],
            "",
        )
    }

    pub fn decode(harness: &str, session: &str, b: &[u8]) -> Result<Binding> {
        let (f, _) = fields(b)?;
        Ok(Binding {
            harness: harness.into(),
            session: session.into(),
            transcript: f.get("transcript").map(PathBuf::from),
            graft: parse_seq(f.get("graft"))?,
            tip: parse_seq(f.get("tip"))?,
            attached: ts(f.get("attached"))?.context("no attach time")?,
            detached: ts(f.get("detached"))?,
        })
    }
}

impl Agent {
    pub fn encode(&self) -> Vec<u8> {
        record::encode(
            [
                ("type", self.agent_type.clone()),
                ("description", self.description.clone()),
                ("call", show_seq(self.call)),
            ],
            "",
        )
    }

    pub fn decode(harness: &str, agent: &str, b: &[u8]) -> Result<Agent> {
        let (f, _) = fields(b)?;
        let get = |k: &str| f.get(k).map_or_else(String::new, |v| v.to_string());
        Ok(Agent {
            harness: harness.into(),
            agent: agent.into(),
            agent_type: get("type"),
            description: get("description"),
            call: parse_seq(f.get("call"))?,
        })
    }
}

impl Line {
    pub fn encode(&self) -> Vec<u8> {
        let w = &self.who;
        record::encode(
            [
                ("ts", self.ts.to_string().as_str()),
                ("kind", self.kind.name()),
                ("agent", &w.agent),
                ("model", &w.model),
                ("session", &w.session),
            ],
            &self.text,
        )
    }

    pub fn decode(b: &[u8]) -> Result<Line> {
        let (f, text) = fields(b)?;
        let get = |k: &str| f.get(k).map_or_else(String::new, |v| v.to_string());
        Ok(Line {
            ts: ts(f.get("ts"))?.context("no time")?,
            kind: f
                .get("kind")
                .and_then(|k| LineKind::parse(k))
                .context("an unknown kind")?,
            who: Who {
                agent: get("agent"),
                model: get("model"),
                session: get("session"),
            },
            text: text.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn kinds_round_trip() {
        for kind in [
            Kind::User("hi\n\nthere".into()),
            Kind::Work {
                from: "research agent".into(),
                text: "found".into(),
            },
            Kind::Text(String::new()),
            Kind::ToolCall {
                id: "toolu_1".into(),
                name: "Bash".into(),
                input: json!({"command": "ls\n", "n": 1.5}),
            },
            Kind::ToolResult {
                id: "toolu_1".into(),
                output: "a\nb".into(),
                is_error: true,
            },
            Kind::ToolResult {
                id: "toolu_2".into(),
                output: String::new(),
                is_error: false,
            },
            Kind::Marker(Marker::Interrupt),
        ] {
            assert_eq!(decode_kind(&encode_kind(&kind)).unwrap(), kind);
        }
        assert_eq!(
            encode_kind(&Kind::Work {
                from: "a".into(),
                text: "x".into()
            }),
            b"kind work\nfrom a\n\nx"
        );
        assert!(decode_kind(b"kind thought\n\nx").is_err());
        assert!(decode_kind(b"kind marker\nmarker nap\n\n").is_err());
    }

    #[test]
    fn heads_keep_their_columns() {
        let mut seg = Seg::new(&COLUMNS);
        let head = Head {
            seq: 7,
            parent: Some(3),
            harness: "claude".into(),
            session: "s 1".into(),
            entry: "u1.1".into(),
            ts: t("2026-10-14T10:00:00.123Z"),
        };
        let line = head.line(&seg);
        assert_eq!(line, "2026-10-14T10:00:00.123Z claude s_1 u1.1 3");
        seg.lines.push(line);
        let back = Head::parse(7, &seg, &seg.lines[0]).unwrap();
        assert_eq!(back.session, "s_1");
        assert_eq!(
            back,
            Head {
                session: "s_1".into(),
                ..head.clone()
            }
        );
        let root = Head {
            parent: None,
            ..head
        };
        assert_eq!(Head::parse(0, &seg, &root.line(&seg)).unwrap().parent, None);
    }

    #[test]
    fn bindings_agents_and_lines_round_trip() {
        let b = Binding {
            harness: "claude".into(),
            session: "s-1".into(),
            transcript: Some("/p/s-1.jsonl".into()),
            graft: None,
            tip: Some(4),
            attached: t("2026-10-14T10:00:00Z"),
            detached: None,
        };
        assert_eq!(
            b.encode(),
            b"transcript /p/s-1.jsonl\ntip 4\nattached 2026-10-14T10:00:00Z\n\n"
        );
        assert_eq!(Binding::decode("claude", "s-1", &b.encode()).unwrap(), b);
        let d = Binding {
            detached: Some(t("2026-10-14T11:00:00Z")),
            graft: Some(0),
            transcript: None,
            ..b
        };
        assert_eq!(Binding::decode("claude", "s-1", &d.encode()).unwrap(), d);
        assert!(Binding::decode("claude", "s-1", b"tip 4\n\n").is_err());
        let a = Agent {
            harness: "claude".into(),
            agent: "a1".into(),
            agent_type: "general-purpose".into(),
            description: "Survey\nstorage".into(),
            call: Some(2),
        };
        let back = Agent::decode("claude", "a1", &a.encode()).unwrap();
        assert_eq!(back.description, "Survey storage");
        let l = Line {
            ts: t("2026-10-14T10:00:00Z"),
            kind: LineKind::Exchange,
            who: Who {
                agent: "claude-code".into(),
                model: "claude-haiku-5-5".into(),
                session: String::new(),
            },
            text: "user: fix it; ai: fixed".into(),
        };
        assert_eq!(Line::decode(&l.encode()).unwrap(), l);
        assert!(Line::decode(b"ts 2026-10-14T10:00:00Z\nkind leaf\n\nx").is_err());
    }

    #[test]
    fn ids_are_path_names() {
        for ok in ["a87491ed-08a1", "claude", "s_1.2"] {
            assert!(check_id(ok, "session").is_ok());
        }
        for bad in ["", ".", "..", ".x", "a/b", "a b", "ü"] {
            assert!(check_id(bad, "session").is_err(), "{bad}");
        }
    }
}
