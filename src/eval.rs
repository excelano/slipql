//! Deciding a `where` clause for one row.
//!
//! Three-valued: a comparison against an absent key, or across type classes,
//! is unknown rather than false, and a row is kept only when the whole clause
//! is true. `exists` is the one test that is never unknown.

use std::borrow::Cow;
use std::cmp::Ordering;

use crate::ast::{Op, Path, Predicate, Segment};
use crate::notice::Tally;
use crate::value::{Kind, Value};

/// The result of a predicate under three-valued logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    fn from_bool(b: bool) -> Self {
        if b {
            Self::True
        } else {
            Self::False
        }
    }

    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }

    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}

/// One row as the evaluator sees it: the metadata and the container's path.
pub(crate) struct Row<'a> {
    pub metadata: &'a Value,
    pub path: &'a str,
}

impl Row<'_> {
    /// The value at a path, or `None` when any step of it is absent.
    pub(crate) fn lookup(&self, path: &Path) -> Option<Cow<'_, Value>> {
        match path {
            Path::ContainerPath => Some(Cow::Owned(Value::String(self.path.to_owned()))),
            Path::Keys(segments) => resolve(self.metadata, segments).map(Cow::Borrowed),
        }
    }
}

/// Walk a route of keys and indexes through a value.
pub(crate) fn resolve<'a>(root: &'a Value, segments: &[Segment]) -> Option<&'a Value> {
    let mut current = root;
    for segment in segments {
        current = match (segment, current) {
            (Segment::Key(key), Value::Table(entries)) => {
                &entries.iter().find(|(k, _)| k == key)?.1
            }
            (Segment::Index(i), Value::Array(items)) => items.get(*i)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Evaluate a predicate for a row, recording type mismatches in `tally`.
pub(crate) fn eval(predicate: &Predicate, row: &Row<'_>, tally: &mut Tally) -> Truth {
    match predicate {
        Predicate::Or(a, b) => eval(a, row, tally).or(eval(b, row, tally)),
        Predicate::And(a, b) => eval(a, row, tally).and(eval(b, row, tally)),
        Predicate::Not(inner) => eval(inner, row, tally).not(),
        Predicate::Exists(path) => Truth::from_bool(row.lookup(path).is_some()),
        Predicate::Compare { path, op, value } => {
            let Some(actual) = row.lookup(path) else {
                return Truth::Unknown;
            };
            match compare(&actual, value, path, tally) {
                Some(ordering) => Truth::from_bool(match op {
                    Op::Eq => ordering == Ordering::Equal,
                    Op::Ne => ordering != Ordering::Equal,
                    Op::Lt => ordering == Ordering::Less,
                    Op::Gt => ordering == Ordering::Greater,
                    Op::Le => ordering != Ordering::Greater,
                    Op::Ge => ordering != Ordering::Less,
                }),
                None => Truth::Unknown,
            }
        }
        Predicate::In { path, values } => {
            let Some(actual) = row.lookup(path) else {
                return Truth::Unknown;
            };
            let mut result = Truth::False;
            for candidate in values {
                match compare(&actual, candidate, path, tally) {
                    Some(Ordering::Equal) => return Truth::True,
                    Some(_) => {}
                    None => result = Truth::Unknown,
                }
            }
            result
        }
        Predicate::Contains { path, value } => {
            let Some(actual) = row.lookup(path) else {
                return Truth::Unknown;
            };
            let Value::Array(items) = actual.as_ref() else {
                tally.mismatch(path, actual.kind(), Kind::Array);
                return Truth::Unknown;
            };
            Truth::from_bool(
                items
                    .iter()
                    .any(|item| item.compare(value) == Some(Ordering::Equal)),
            )
        }
        Predicate::Like {
            path,
            pattern,
            case_insensitive,
        } => {
            let Some(actual) = row.lookup(path) else {
                return Truth::Unknown;
            };
            let Value::String(text) = actual.as_ref() else {
                tally.mismatch(path, actual.kind(), Kind::String);
                return Truth::Unknown;
            };
            Truth::from_bool(if *case_insensitive {
                like(&text.to_lowercase(), &pattern.to_lowercase())
            } else {
                like(text, pattern)
            })
        }
    }
}

/// Compare a row's value with a literal, tallying a class mismatch.
fn compare(actual: &Value, literal: &Value, path: &Path, tally: &mut Tally) -> Option<Ordering> {
    let ordering = actual.compare(literal);
    if ordering.is_none() && !same_class(actual.kind(), literal.kind()) {
        tally.mismatch(path, actual.kind(), literal.kind());
    }
    ordering
}

fn same_class(a: Kind, b: Kind) -> bool {
    matches!(
        (a, b),
        (Kind::Integer | Kind::Float, Kind::Integer | Kind::Float)
    ) || a == b
}

/// SQL `like`: `%` matches any run, `_` one character, and a backslash
/// takes the next pattern character literally.
pub(crate) fn like(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    like_at(&text, &pattern)
}

fn like_at(text: &[char], pattern: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('%') => (0..=text.len()).any(|i| like_at(&text[i..], &pattern[1..])),
        Some('_') => !text.is_empty() && like_at(&text[1..], &pattern[1..]),
        Some('\\') => match pattern.get(1) {
            Some(&escaped) => text.first() == Some(&escaped) && like_at(&text[1..], &pattern[2..]),
            None => text.is_empty(),
        },
        Some(&literal) => text.first() == Some(&literal) && like_at(&text[1..], &pattern[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const DOC: &str = r#"
slipcase_version = "1.0"
title = "Q3 report"
priority = 3
score = 2.5
final = true
created = 2026-03-01T09:00:00+01:00
due = 2026-04-01
tags = ["draft", "finance"]
[payload]
file = "report.pdf"
[owner]
name = "Kim"
"#;

    fn doc() -> Value {
        let parsed: slpc::toml_edit::DocumentMut = DOC.parse().unwrap();
        Value::from_item(parsed.as_item()).unwrap()
    }

    fn run(clause: &str) -> (Truth, Tally) {
        let q = parse(&format!("select * from '.' where {clause}")).unwrap();
        let metadata = doc();
        let row = Row {
            metadata: &metadata,
            path: "a/report.pdf.slpc",
        };
        let mut tally = Tally::default();
        let truth = eval(q.filter.as_ref().unwrap(), &row, &mut tally);
        (truth, tally)
    }

    fn truth(clause: &str) -> Truth {
        run(clause).0
    }

    #[test]
    fn comparisons() {
        assert_eq!(truth("priority = 3"), Truth::True);
        assert_eq!(truth("priority != 3"), Truth::False);
        assert_eq!(truth("priority > 2.5"), Truth::True);
        assert_eq!(truth("score <= 2.5"), Truth::True);
        assert_eq!(truth("title = \"Q3 report\""), Truth::True);
        assert_eq!(truth("title < \"R\""), Truth::True);
        assert_eq!(truth("final = true"), Truth::True);
        assert_eq!(truth("owner.name = 'Kim'"), Truth::True);
        assert_eq!(truth("payload.file like '%.pdf'"), Truth::True);
        assert_eq!(truth("@path like 'a/%'"), Truth::True);
    }

    #[test]
    fn datetimes() {
        assert_eq!(truth("created = 2026-03-01T08:00:00Z"), Truth::True);
        assert_eq!(truth("created < 2026-03-01T08:00:01Z"), Truth::True);
        assert_eq!(truth("due >= 2026-04-01"), Truth::True);
        assert_eq!(truth("due = 2026-04-01T00:00:00"), Truth::Unknown);
    }

    #[test]
    fn absent_is_unknown_and_exists_is_not() {
        assert_eq!(truth("missing = 1"), Truth::Unknown);
        assert_eq!(truth("missing != 1"), Truth::Unknown);
        assert_eq!(truth("not missing = 1"), Truth::Unknown);
        assert_eq!(truth("exists missing"), Truth::False);
        assert_eq!(truth("not exists missing"), Truth::True);
        assert_eq!(truth("exists owner.name"), Truth::True);
        assert_eq!(truth("exists owner.name.deeper"), Truth::False);
        assert_eq!(truth("exists tags[1]"), Truth::True);
        assert_eq!(truth("exists tags[2]"), Truth::False);
        assert_eq!(truth("missing = 1 or priority = 3"), Truth::True);
        assert_eq!(truth("missing = 1 and priority = 3"), Truth::Unknown);
        assert_eq!(truth("missing = 1 and priority = 4"), Truth::False);
    }

    #[test]
    fn mismatches_are_unknown_and_counted() {
        let (t, tally) = run("priority = \"3\"");
        assert_eq!(t, Truth::Unknown);
        let counts = tally.mismatches();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].0.to_string(), "priority");
        assert_eq!(
            (counts[0].1, counts[0].2, counts[0].3),
            (Kind::Integer, Kind::String, 1)
        );
        let (t, tally) = run("tags = \"draft\"");
        assert_eq!(t, Truth::Unknown);
        assert_eq!(tally.mismatches()[0].1, Kind::Array);
        let (t, tally) = run("priority like '3'");
        assert_eq!(t, Truth::Unknown);
        assert_eq!(tally.mismatches()[0].2, Kind::String);
        let (t, tally) = run("title contains 'Q'");
        assert_eq!(t, Truth::Unknown);
        assert_eq!(tally.mismatches()[0].2, Kind::Array);
        let (t, tally) = run("score = nan");
        assert_eq!(t, Truth::Unknown);
        assert!(tally.mismatches().is_empty());
    }

    #[test]
    fn membership() {
        assert_eq!(truth("tags contains 'draft'"), Truth::True);
        assert_eq!(truth("tags contains 'x'"), Truth::False);
        assert_eq!(truth("tags contains 1"), Truth::False);
        assert_eq!(truth("tags[0] = 'draft'"), Truth::True);
        assert_eq!(truth("priority in (1, 2, 3)"), Truth::True);
        assert_eq!(truth("priority in (1, 2)"), Truth::False);
        assert_eq!(truth("priority not in (1, 2)"), Truth::True);
        assert_eq!(truth("priority in (1, '3')"), Truth::Unknown);
        assert_eq!(truth("priority in ('3', 3)"), Truth::True);
    }

    #[test]
    fn like_patterns() {
        assert!(like("report.pdf", "%.pdf"));
        assert!(like("report.pdf", "report%"));
        assert!(like("report.pdf", "r_port.pdf"));
        assert!(!like("report.pdf", "r_ort.pdf"));
        assert!(like("100%", "100\\%"));
        assert!(!like("1000", "100\\%"));
        assert!(like("a_b", "a\\_b"));
        assert!(like("", "%"));
        assert!(!like("", "_"));
        assert!(like("abc", "%%c"));
        assert!(like("ünïcode", "_n_code"));
        assert_eq!(truth("title ilike 'q3%'"), Truth::True);
        assert_eq!(truth("title like 'q3%'"), Truth::False);
        assert_eq!(truth("title not like 'q3%'"), Truth::True);
    }
}
