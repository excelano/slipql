//! Finding containers under a directory.
//!
//! Entries are visited in name order, depth first, so two runs over the same
//! tree yield the same rows in the same order. Only files whose extension is
//! `slpc` are candidates. A symbolic link to a directory is not followed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::notice::Tally;

/// One candidate container.
pub(crate) struct Candidate {
    /// Relative to the root, as the `@path` column reports it.
    pub relative: PathBuf,
    /// Where to open it.
    pub absolute: PathBuf,
}

/// A depth-first walk that yields candidates one at a time.
#[derive(Debug)]
pub(crate) struct Walker {
    root: PathBuf,
    recursive: bool,
    /// Directories still to visit, each with the entries not yet handled, in
    /// reverse name order so the next one pops off the end.
    stack: Vec<Vec<fs::DirEntry>>,
}

impl Walker {
    /// Start at `root`. Fails when the root itself cannot be listed.
    pub(crate) fn open(root: &Path, recursive: bool) -> io::Result<Self> {
        let entries = list(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            recursive,
            stack: vec![entries],
        })
    }

    /// The next candidate, recording anything skipped on the way in `tally`.
    pub(crate) fn next(&mut self, tally: &mut Tally) -> Option<Candidate> {
        loop {
            let entry = loop {
                let top = self.stack.last_mut()?;
                match top.pop() {
                    Some(entry) => break entry,
                    None => {
                        self.stack.pop();
                    }
                }
            };
            let absolute = entry.path();
            let relative = absolute
                .strip_prefix(&self.root)
                .map_or_else(|_| absolute.clone(), Path::to_path_buf);
            // `metadata` follows a link, `file_type` does not; a link to a
            // directory is the one thing deliberately not followed.
            let Ok(kind) = fs::metadata(&absolute) else {
                tally.skip(relative, "cannot read");
                continue;
            };
            if kind.is_dir() {
                let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
                if self.recursive && !is_link {
                    match list(&absolute) {
                        Ok(entries) => self.stack.push(entries),
                        Err(e) => tally.skip(relative, format!("cannot list directory: {e}")),
                    }
                }
                continue;
            }
            let is_container = absolute
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("slpc"));
            if kind.is_file() && is_container {
                return Some(Candidate { relative, absolute });
            }
        }
    }
}

/// A directory's entries, sorted so that the first by name is last.
fn list(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    Ok(entries)
}
