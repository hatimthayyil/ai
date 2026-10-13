use std::{fmt, path::PathBuf};

use anyhow::{Result, bail};

/// A directory of the store, named by its slugs from the root: `linguistics.syntax`.
/// The root is the empty topic.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Topic(Vec<String>);

/// A topic segment or a note's slug: `a-z`, `0-9` and inner `-`.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Topic {
    pub fn parse(s: &str) -> Result<Topic> {
        let segs: Vec<String> = s.split('.').map(str::to_string).collect();
        if !segs.iter().all(|g| is_slug(g)) {
            bail!(
                "'{s}' is not a topic: slugs of a-z, 0-9 and '-', joined by dots, like linguistics.syntax."
            );
        }
        Ok(Topic(segs))
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// Its last slug; empty for the root.
    pub fn name(&self) -> &str {
        self.0.last().map_or("", String::as_str)
    }

    pub fn child(&self, slug: &str) -> Topic {
        let mut segs = self.0.clone();
        segs.push(slug.into());
        Topic(segs)
    }

    pub fn parent(&self) -> Option<Topic> {
        let (_, up) = self.0.split_last()?;
        Some(Topic(up.to_vec()))
    }

    /// Whether `self` is `other` or lies under it.
    pub fn within(&self, other: &Topic) -> bool {
        self.0.starts_with(&other.0)
    }

    /// Its directory, relative to the store.
    pub fn rel(&self) -> PathBuf {
        self.0.iter().collect()
    }

    /// Its breadcrumb: `physics › qm › decoherence`.
    pub fn crumbs(&self) -> String {
        self.0.join(" › ")
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_are_dotted_slugs() {
        let t = Topic::parse("linguistics.syntax.minimalism").unwrap();
        assert_eq!(t.to_string(), "linguistics.syntax.minimalism");
        assert_eq!(t.rel(), PathBuf::from("linguistics/syntax/minimalism"));
        assert_eq!(t.crumbs(), "linguistics › syntax › minimalism");
        assert_eq!(t.name(), "minimalism");
        assert_eq!(t.parent().unwrap().to_string(), "linguistics.syntax");
        assert!(t.within(&Topic::parse("linguistics").unwrap()));
        assert!(!Topic::parse("ling").unwrap().within(&t));
        assert!(t.within(&Topic::default()));
        assert!(Topic::parse("projects.ai-tools").is_ok());
        for bad in ["", "a..b", ".a", "A", "a_b", "-a", "a-", "a/b", "é"] {
            assert!(Topic::parse(bad).is_err(), "{bad}");
        }
    }
}
