use std::{collections::HashSet, sync::LazyLock};

use anyhow::{Result, bail};
use clap::ValueEnum;
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use regex::Regex;

use crate::notes::topic::Topic;

/// The longest note body, in bytes.
pub const LEAF_MAX: usize = 1024;
const SLUG_WORDS: usize = 6;
const SLUG_MAX: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Confidence {
    Verified,
    Reported,
    Speculative,
    Assessment,
}

impl Confidence {
    pub fn name(self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::Reported => "reported",
            Confidence::Speculative => "speculative",
            Confidence::Assessment => "assessment",
        }
    }
}

/// A file's frontmatter: its `key: value` lines, in order, kept as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front(Vec<(String, String)>);

impl Front {
    /// Splits `text` into its frontmatter and its body, trimmed.
    pub fn parse(text: &str) -> Result<(Front, &str)> {
        let Some(rest) = text.strip_prefix("---\n") else {
            return Ok((Front::default(), text.trim()));
        };
        let (head, body) = match rest.find("\n---") {
            Some(k) if rest[k + 4..].is_empty() || rest[k + 4..].starts_with('\n') => {
                (&rest[..k], &rest[k + 4..])
            }
            _ => bail!("the frontmatter has no closing '---'"),
        };
        let mut f = Front::default();
        for line in head.lines().filter(|l| !l.trim().is_empty()) {
            let Some((k, v)) = line.split_once(':') else {
                bail!("a frontmatter line is not 'key: value': {line}");
            };
            f.0.push((k.trim().into(), v.trim().into()));
        }
        Ok((f, body.trim()))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Sets `key`, in place if present, else last.
    pub fn set(&mut self, key: &str, value: String) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.into(), value)),
        }
    }

    /// A `[a, b]` list.
    pub fn list(&self, key: &str) -> Vec<String> {
        let v = self.get(key).unwrap_or_default();
        let v = v
            .strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(v);
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }

    pub fn set_list(&mut self, key: &str, items: &[String]) {
        self.set(key, format!("[{}]", items.join(", ")));
    }

    /// The frontmatter block, `---` to `---`; empty when there are no fields.
    pub fn render(&self) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        let lines: String = self.0.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
        format!("---\n{lines}---\n")
    }
}

/// One note: one claim, filed under a topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub id: String,
    pub slug: String,
    pub topic: Topic,
    pub front: Front,
    pub body: String,
}

/// A Denote timestamp: `YYYYMMDDThhmmss`.
pub fn is_id(s: &str) -> bool {
    s.len() == 15
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'T',
            _ => b.is_ascii_digit(),
        })
}

/// The id of a note made now: the UTC second, bumped past every id in `taken`.
pub fn new_id(taken: &HashSet<String>) -> String {
    let mut t = Timestamp::now();
    loop {
        let id = t
            .to_zoned(TimeZone::UTC)
            .strftime("%Y%m%dT%H%M%S")
            .to_string();
        if !taken.contains(&id) {
            return id;
        }
        t += SignedDuration::from_secs(1);
    }
}

/// A file-name slug from the first words of `text`.
pub fn slug(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default().to_lowercase();
    let mut out = String::new();
    let words = first
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty());
    for w in words.take(SLUG_WORDS) {
        if !out.is_empty() && out.len() + 1 + w.len() > SLUG_MAX {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w[..w.len().min(SLUG_MAX)]);
    }
    if out.is_empty() { "note".into() } else { out }
}

/// `<id>---<slug>.md`, split.
pub fn parse_name(name: &str) -> Option<(&str, &str)> {
    let (id, slug) = name.strip_suffix(".md")?.split_once("---")?;
    is_id(id).then_some((id, slug))
}

static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[(\d{8}T\d{6})\]\]").expect("a valid regex"));

impl Leaf {
    pub fn parse(topic: &Topic, name: &str, text: &str) -> Result<Leaf> {
        let Some((id, slug)) = parse_name(name) else {
            bail!("{name} is not named <id>---<slug>.md");
        };
        let (front, body) = Front::parse(text)?;
        Ok(Leaf {
            id: id.into(),
            slug: slug.into(),
            topic: topic.clone(),
            front,
            body: body.into(),
        })
    }

    pub fn file_name(&self) -> String {
        format!("{}---{}.md", self.id, self.slug)
    }

    pub fn render(&self) -> String {
        format!("{}{}\n", self.front.render(), self.body)
    }

    /// Absent `invalid_at`: not superseded or refuted.
    pub fn is_valid(&self) -> bool {
        self.front.get("invalid_at").is_none()
    }

    pub fn confidence(&self) -> Option<&str> {
        self.front.get("confidence")
    }

    /// Every note it points to: its `links`, then each `[[id]]` of its body, once.
    pub fn links(&self) -> Vec<String> {
        let mut out = self.front.list("links");
        for c in LINK.captures_iter(&self.body) {
            if !out.iter().any(|l| *l == c[1]) {
                out.push(c[1].into());
            }
        }
        out
    }
}

/// A note body: not empty, at most [`LEAF_MAX`] bytes.
pub fn check_body(text: &str) -> Result<&str> {
    let text = text.trim();
    if text.is_empty() {
        bail!("Empty. A note is one claim, in at most {LEAF_MAX} bytes.");
    }
    let n = text.len();
    if n > LEAF_MAX {
        bail!(
            "Too long: {n} bytes, limit {LEAF_MAX}. A note is one claim: split it, or compress it further."
        );
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_round_trips_every_field() {
        let text = "---\nsource: https://x.org/a\nconfidence: verified\nchecked: 2026-10-13\n\
                    decay: slow\nlinks: [20261013T221500, 20261013T221501]\n\
                    supersedes: 20261001T000000\ninvalid_at: 2026-10-14\nmine: kept\n---\n\
                    Merge is binary.\n\n--- not a fence\n";
        let leaf = Leaf::parse(&Topic::default(), "20261013T221502---merge.md", text).unwrap();
        assert_eq!(leaf.body, "Merge is binary.\n\n--- not a fence");
        assert_eq!(leaf.front.get("mine"), Some("kept"));
        assert_eq!(
            leaf.front.list("links"),
            ["20261013T221500", "20261013T221501"]
        );
        assert!(!leaf.is_valid());
        assert_eq!(leaf.render(), text);
        let (f, body) = Front::parse("no frontmatter\n").unwrap();
        assert_eq!((f, body), (Front::default(), "no frontmatter"));
        assert!(Front::parse("---\nconfidence: verified\n").is_err());
        assert!(Front::parse("---\nnot a field\n---\nx").is_err());
    }

    #[test]
    fn links_come_from_the_frontmatter_and_the_body() {
        let mut leaf = Leaf::parse(
            &Topic::default(),
            "20261013T221502---hub.md",
            "---\nlinks: [20261013T221500]\n---\nSee [[20261013T221501]] and [[20261013T221500]].",
        )
        .unwrap();
        assert_eq!(leaf.links(), ["20261013T221500", "20261013T221501"]);
        leaf.front.set_list("links", &["a".into(), "b".into()]);
        assert_eq!(leaf.front.get("links"), Some("[a, b]"));
    }

    #[test]
    fn ids_names_and_slugs() {
        assert!(is_id("20261013T221500"));
        for bad in ["20261013221500", "2026101T3221500", "20261013T22150x", ""] {
            assert!(!is_id(bad), "{bad}");
        }
        assert_eq!(
            parse_name("20261013T221500---merge-is-binary.md"),
            Some(("20261013T221500", "merge-is-binary"))
        );
        assert_eq!(parse_name("_summary.md"), None);
        assert_eq!(
            slug("Merge is binary, per Chomsky (1995).\nmore"),
            "merge-is-binary-per-chomsky-1995"
        );
        assert_eq!(slug("¿Qué?"), "qu");
        assert_eq!(slug("…"), "note");
        assert!(slug(&"x".repeat(100)).len() <= SLUG_MAX);
        let first = new_id(&HashSet::new());
        let next = new_id(&HashSet::from([first.clone()]));
        assert!(is_id(&first) && is_id(&next) && next > first);
    }

    #[test]
    fn bodies_are_capped() {
        assert_eq!(check_body("  a claim \n").unwrap(), "a claim");
        assert!(check_body(" \n").is_err());
        assert!(check_body(&"x".repeat(LEAF_MAX)).is_ok());
        let e = check_body(&"x".repeat(LEAF_MAX + 1))
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("Too long: 1025 bytes, limit 1024."), "{e}");
    }
}
