use serde_json::Value;

use crate::chat::model::{Item, Kind, Marker};

/// How much of an item a line shows, in bytes: of its words, and of each tool input or output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub words: usize,
    pub tool: usize,
}

/// Item `seq` as a line, `seq|kind: text`, cut by `cut`; whole if `None`. A tool call's input
/// is `key=value` per field, strings verbatim.
pub fn item_line(seq: u64, item: &Item, cut: Option<Cut>) -> String {
    let words = cut.map(|c| c.words);
    let tool = cut.map(|c| c.tool);
    let (label, text) = match &item.kind {
        Kind::User(t) => ("user".to_string(), clip(t, words, seq)),
        Kind::Work { from, text } => (format!("work from {from}"), clip(text, words, seq)),
        Kind::Text(t) => ("ai".into(), clip(t, words, seq)),
        Kind::ToolCall { name, input, .. } => {
            (format!("call {name}"), clip(&args(input), tool, seq))
        }
        Kind::ToolResult {
            output, is_error, ..
        } => {
            let label = if *is_error { "error" } else { "result" };
            (label.into(), clip(output, tool, seq))
        }
        Kind::Marker(m) => {
            let what = match m {
                Marker::Compaction => "the harness compacted its context here",
                Marker::Summary => "the harness's summary of the context it compacted",
                Marker::Interrupt => "interrupted by the user",
            };
            return format!("{seq}|[{what}]");
        }
    };
    format!("{seq}|{label}: {text}")
}

/// A tool call's input: `key=value` per field, strings verbatim.
fn args(v: &Value) -> String {
    let field = |v: &Value| match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    match v {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| format!("{k}={}", field(v)))
            .collect::<Vec<_>>()
            .join(" "),
        Value::Null => String::new(),
        v => field(v),
    }
}

/// `s` in about `max` bytes: its head and tail around a note on what was cut from item `seq`;
/// whole if `None`.
pub fn clip(s: &str, max: Option<usize>, seq: u64) -> String {
    let Some(max) = max.filter(|&m| s.len() > m) else {
        return s.into();
    };
    let head = &s[..s.floor_char_boundary(max / 2)];
    let tail = &s[s.ceil_char_boundary(s.len() - max / 2)..];
    let cut = s.len() - head.len() - tail.len();
    format!("{head} [... {cut} bytes cut: zoom {seq} ...] {tail}")
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use serde_json::json;

    use super::*;
    use crate::chat::model::SourceId;

    #[test]
    fn clips_keep_head_and_tail() {
        assert_eq!(clip("short", Some(10), 3), "short");
        assert_eq!(clip("short", None, 3), "short");
        let s = format!("{}{}", "a".repeat(10), "b".repeat(10));
        assert_eq!(
            clip(&s, Some(8), 3),
            "aaaa [... 12 bytes cut: zoom 3 ...] bbbb"
        );
        let c = clip(&"é".repeat(10), Some(7), 3);
        assert!(c.starts_with("é [") && c.ends_with("] é"), "{c}");
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
    fn items_are_one_line_each() {
        let cut = Some(Cut {
            words: 100,
            tool: 40,
        });
        let r = |k| item_line(7, &item(k), cut);
        assert_eq!(r(Kind::User("fix it".into())), "7|user: fix it");
        assert_eq!(
            r(Kind::Work {
                from: "reviewer".into(),
                text: "done".into()
            }),
            "7|work from reviewer: done"
        );
        assert_eq!(r(Kind::Text("ok".into())), "7|ai: ok");
        let call = |input| Kind::ToolCall {
            id: "t1".into(),
            name: "Bash".into(),
            input,
        };
        assert_eq!(
            r(call(json!({"command": "cargo test", "timeout": 60}))),
            "7|call Bash: command=cargo test timeout=60"
        );
        assert_eq!(r(call(json!("ls"))), "7|call Bash: ls");
        let edit = r(call(
            json!({"old_string": "x".repeat(50), "new_string": "y"}),
        ));
        assert_eq!(
            edit,
            format!(
                "7|call Bash: new_string=y old_str [... 34 bytes cut: zoom 7 ...] {}",
                "x".repeat(20)
            )
        );
        let out = format!("{}{}", "h".repeat(30), "t".repeat(30));
        let result = |is_error| {
            r(Kind::ToolResult {
                id: "t1".into(),
                output: out.clone(),
                is_error,
            })
        };
        assert_eq!(
            result(true),
            format!(
                "7|error: {} [... 20 bytes cut: zoom 7 ...] {}",
                "h".repeat(20),
                "t".repeat(20)
            )
        );
        assert!(result(false).starts_with("7|result: hhh"));
        assert_eq!(
            item_line(7, &item(Kind::Text(out.clone())), None),
            format!("7|ai: {out}")
        );
        assert_eq!(
            r(Kind::Marker(Marker::Interrupt)),
            "7|[interrupted by the user]"
        );
    }
}
