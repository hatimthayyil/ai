/// A record with named headers: one `key value` line per non-empty field, a blank line, then
/// the body verbatim. A newline in a value becomes a space.
pub fn encode<'k>(
    fields: impl IntoIterator<Item = (&'k str, impl AsRef<str>)>,
    body: &str,
) -> Vec<u8> {
    let mut s = String::new();
    for (k, v) in fields {
        let v = v.as_ref();
        if v.is_empty() {
            continue;
        }
        s += k;
        s.push(' ');
        s += &v.replace("\r\n", " ").replace(['\r', '\n'], " ");
        s.push('\n');
    }
    s.push('\n');
    s += body;
    s.into_bytes()
}

/// The fields and body of a record; `None` if it is not UTF-8, has no blank line after its
/// header, or a header line without a value.
pub fn decode(b: &[u8]) -> Option<(Vec<(&str, &str)>, &str)> {
    let s = std::str::from_utf8(b).ok()?;
    let (head, body) = match s.strip_prefix('\n') {
        Some(body) => ("", body),
        None => s.split_once("\n\n")?,
    };
    let fields = match head {
        "" => Vec::new(),
        _ => head
            .split('\n')
            .map(|l| l.split_once(' '))
            .collect::<Option<_>>()?,
    };
    Some((fields, body))
}

/// Records as lines under a column row: the first line names the columns, each next line
/// holds one record's values, space-separated. Only the last column may hold spaces.
#[derive(Debug)]
pub struct Seg {
    pub cols: Vec<String>,
    pub lines: Vec<String>,
}

impl Seg {
    pub fn new(cols: &[&str]) -> Seg {
        Seg {
            cols: cols.iter().map(|c| c.to_string()).collect(),
            lines: Vec::new(),
        }
    }

    /// The segment stored as `b`; none is an empty one under `cols`. `None` if not UTF-8.
    pub fn parse(b: &[u8], cols: &[&str]) -> Option<Seg> {
        if b.is_empty() {
            return Some(Seg::new(cols));
        }
        let s = std::str::from_utf8(b).ok()?;
        let mut lines = s.strip_suffix('\n').unwrap_or(s).split('\n');
        let cols = lines.next().unwrap_or("").split(' ').map(String::from);
        Some(Seg {
            cols: cols.collect(),
            lines: lines.map(String::from).collect(),
        })
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut s = self.cols.join(" ");
        for l in &self.lines {
            s.push('\n');
            s += l;
        }
        s.push('\n');
        s.into_bytes()
    }

    /// The values of `line` by column; a column the line lacks is empty.
    pub fn values<'a>(&'a self, line: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
        let mut vs = line.splitn(self.cols.len(), ' ');
        self.cols
            .iter()
            .map(move |c| (c.as_str(), vs.next().unwrap_or("")))
    }

    /// A line under this segment's columns, each value from `value`; unknown columns empty.
    pub fn line(&self, value: impl Fn(&str) -> Option<String>) -> String {
        let vs: Vec<String> = self
            .cols
            .iter()
            .map(|c| value(c).unwrap_or_default())
            .collect();
        vs.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let b = encode([("a", "1"), ("b", ""), ("c", "x\ny")], "body\n\nmore");
        assert_eq!(b, b"a 1\nc x y\n\nbody\n\nmore");
        assert_eq!(
            decode(&b),
            Some((vec![("a", "1"), ("c", "x y")], "body\n\nmore"))
        );
        assert_eq!(encode([("a", "")], "x"), b"\nx");
        assert_eq!(decode(b"\nx"), Some((vec![], "x")));
        assert_eq!(decode(b"\n\nx"), Some((vec![], "\nx")));
        for bad in [&b"a 1\nx"[..], b"a\n\nx", &[0xff, b'\n', b'\n']] {
            assert_eq!(decode(bad), None);
        }
    }

    #[test]
    fn segments_keep_their_columns() {
        let mut seg = Seg::parse(b"", &["a", "b"]).unwrap();
        seg.lines.push(seg.line(|c| Some(format!("{c}1"))));
        assert_eq!(seg.bytes(), b"a b\na1 b1\n");
        let wider = Seg::parse(b"a later b\na1 l1 b 1\n\n", &["a", "b"]).unwrap();
        assert_eq!(wider.lines, ["a1 l1 b 1", ""]);
        let line = wider.line(|c| (c != "later").then(|| c.to_string()));
        assert_eq!(line, "a  b");
        let vs: Vec<_> = wider.values(&wider.lines[0]).collect();
        assert_eq!(vs, [("a", "a1"), ("later", "l1"), ("b", "b 1")]);
        let vs: Vec<_> = wider.values("").collect();
        assert_eq!(vs, [("a", ""), ("later", ""), ("b", "")]);
        assert!(Seg::parse(&[0xff], &["a"]).is_none());
    }
}
