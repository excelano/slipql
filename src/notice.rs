//! What a query has to say besides its rows.
//!
//! A skipped file and a comparison across types are not errors: the query
//! still answers. They are things the person asking would want to know, so
//! they are collected here and reported once the rows are out.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ast::Path;
use crate::value::Kind;

/// A file or directory the scan could not use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The path, relative to the `from` root.
    pub path: PathBuf,
    /// Why, in lowercase, with no trailing period.
    pub reason: String,
}

/// Everything a scan noticed on the way.
#[derive(Debug, Default, Clone)]
pub struct Tally {
    skipped: Vec<Skipped>,
    mismatches: BTreeMap<(String, Kind, Kind), (Path, usize)>,
}

impl Tally {
    pub(crate) fn skip(&mut self, path: PathBuf, reason: impl Into<String>) {
        self.skipped.push(Skipped {
            path,
            reason: reason.into(),
        });
    }

    pub(crate) fn mismatch(&mut self, path: &Path, found: Kind, against: Kind) {
        self.mismatches
            .entry((path.to_string(), found, against))
            .or_insert_with(|| (path.clone(), 0))
            .1 += 1;
    }

    /// Files and directories the scan skipped, in the order met.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// Comparisons that crossed type classes: the path, the kind found in the
    /// metadata, the kind of the literal it was compared with, and how many
    /// rows did it. Sorted by path.
    #[must_use]
    pub fn mismatches(&self) -> Vec<(&Path, Kind, Kind, usize)> {
        self.mismatches
            .iter()
            .map(|((_, found, against), (path, count))| (path, *found, *against, *count))
            .collect()
    }

    /// Whether there is anything to report.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skipped.is_empty() && self.mismatches.is_empty()
    }
}
