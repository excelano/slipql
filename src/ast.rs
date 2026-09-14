//! The shape of a parsed query.
//!
//! Everything here is data. [`crate::parse`] builds it and [`crate::execute`]
//! runs it; a program embedding the library can also build one directly.

use std::fmt;
use std::path::PathBuf;

use crate::value::Value;

/// One `select ... from ... where ...` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// What each row carries.
    pub select: Select,
    /// Where the rows come from.
    pub from: Source,
    /// Which rows are kept. `None` keeps every readable container.
    pub filter: Option<Predicate>,
    /// At most this many rows. `None` is no bound.
    pub limit: Option<usize>,
}

/// The projection list.
#[derive(Debug, Clone, PartialEq)]
pub enum Select {
    /// `*`: the container's path, then every leaf value in the metadata, as
    /// dotted columns. The column set is the union over all rows returned.
    All,
    /// Named columns, in the order written.
    Columns(Vec<Projection>),
}

/// One entry of a projection list.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    /// What to read from each row.
    pub path: Path,
    /// The column name to render, when `as` gave one.
    pub alias: Option<String>,
}

impl Projection {
    /// The column name this projection renders under.
    #[must_use]
    pub fn column(&self) -> String {
        self.alias.clone().unwrap_or_else(|| self.path.to_string())
    }
}

/// The `from` clause: a directory, scanned one level deep unless `recursive`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The directory to scan.
    pub root: PathBuf,
    /// Whether to descend into subdirectories.
    pub recursive: bool,
}

/// Something a row can be asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Path {
    /// `@path`: the container's path relative to the `from` root.
    ContainerPath,
    /// A route into the metadata document: keys and array indexes.
    Keys(Vec<Segment>),
}

/// One step of a [`Path::Keys`] route.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Segment {
    /// A table key.
    Key(String),
    /// An array element, zero-based.
    Index(usize),
}

impl fmt::Display for Path {
    /// Renders the way the query would spell it: `a.b[0]`, with a segment
    /// quoted when it is not a bare key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContainerPath => f.write_str("@path"),
            Self::Keys(segments) => {
                let mut first = true;
                for segment in segments {
                    match segment {
                        Segment::Key(key) => {
                            if !first {
                                f.write_str(".")?;
                            }
                            write_key(f, key)?;
                        }
                        Segment::Index(i) => write!(f, "[{i}]")?,
                    }
                    first = false;
                }
                Ok(())
            }
        }
    }
}

/// Whether a key can be written without quotes: TOML's bare-key alphabet.
pub(crate) fn is_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Write a key the way TOML would: bare when it can be, quoted otherwise.
pub(crate) fn write_key(f: &mut impl fmt::Write, key: &str) -> fmt::Result {
    if is_bare_key(key) {
        f.write_str(key)
    } else {
        crate::value::write_basic_string(f, key)
    }
}

/// A `where` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    /// Either side holds.
    Or(Box<Predicate>, Box<Predicate>),
    /// Both sides hold.
    And(Box<Predicate>, Box<Predicate>),
    /// The inner predicate does not hold.
    Not(Box<Predicate>),
    /// The value at `path`, compared to a literal.
    Compare {
        /// What to read.
        path: Path,
        /// How to compare.
        op: Op,
        /// What to compare against. Always a scalar.
        value: Value,
    },
    /// `exists path`: the path resolves to a value.
    Exists(Path),
    /// `path contains literal`: the path is an array holding an equal element.
    Contains {
        /// What to read.
        path: Path,
        /// The element looked for.
        value: Value,
    },
    /// `path in (a, b, ...)`: the value equals one of the literals.
    In {
        /// What to read.
        path: Path,
        /// The candidates, never empty.
        values: Vec<Value>,
    },
    /// `path like pattern`: the value is a string matching an SQL pattern.
    Like {
        /// What to read.
        path: Path,
        /// The pattern, with `%`, `_`, and backslash escapes.
        pattern: String,
        /// `ilike`: fold case on both sides first.
        case_insensitive: bool,
    },
}

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
        })
    }
}
