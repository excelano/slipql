//! What can go wrong, and how it reads.

use std::fmt;
use std::path::PathBuf;

/// Every fallible operation in this crate returns this.
pub type Result<T> = std::result::Result<T, Error>;

/// The ways a query can fail before or while it runs.
///
/// A container that cannot be read is not one of them: the scan skips it and
/// records it in the [`Tally`](crate::Tally), because one bad file in a directory
/// should not decide the query for the rest.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The query text does not parse. `offset` is a byte index into the text
    /// where the problem was noticed, for a caller that wants to point at it.
    Parse {
        /// Byte offset into the query text.
        offset: usize,
        /// What was wrong, in lowercase, with no trailing period.
        message: String,
    },
    /// The `from` root cannot be scanned at all: it does not exist, is not a
    /// directory, or cannot be opened.
    Source {
        /// The root as the query named it.
        root: PathBuf,
        /// Why it cannot be scanned.
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { offset, message } => {
                write!(f, "parse error at offset {offset}: {message}")
            }
            Self::Source { root, source } => {
                write!(f, "cannot scan {}: {source}", root.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse { .. } => None,
            Self::Source { source, .. } => Some(source),
        }
    }
}

impl Error {
    pub(crate) fn parse(offset: usize, message: impl Into<String>) -> Self {
        Self::Parse {
            offset,
            message: message.into(),
        }
    }
}
