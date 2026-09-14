//! Running a query: scan, filter, project, one row at a time.

use std::fs::File;

use slpc::Limits;

use crate::ast::{Path, Projection, Query, Segment, Select};
use crate::error::{Error, Result};
use crate::eval::{self, Truth};
use crate::notice::Tally;
use crate::scan::Walker;
use crate::value::Value;

/// Knobs for a run.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    /// How much of a metadata member to read before giving up on the file.
    /// A container over the bound is skipped, not failed.
    pub limits: Limits,
}

/// One result row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The container's path relative to the `from` root, whether or not the
    /// query projected it.
    pub path: String,
    /// The projected cells, in column order.
    pub cells: Vec<Cell>,
}

/// One cell of a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The column name.
    pub column: String,
    /// The value, or `None` when the row has no such key.
    pub value: Option<Value>,
}

/// Run a query with default options.
///
/// Fails only when the `from` root cannot be scanned at all. Everything after
/// that is a row or a notice.
pub fn execute(query: &Query) -> Result<Results> {
    execute_with(query, Options::default())
}

/// Run a query.
pub fn execute_with(query: &Query, options: Options) -> Result<Results> {
    let walker =
        Walker::open(&query.from.root, query.from.recursive).map_err(|source| Error::Source {
            root: query.from.root.clone(),
            source,
        })?;
    Ok(Results {
        query: query.clone(),
        options,
        walker,
        tally: Tally::default(),
        yielded: 0,
    })
}

/// The rows of a running query, produced as the scan reaches them.
///
/// Dropping it stops the scan. Once it is exhausted, [`Results::tally`] holds
/// what the scan skipped and which comparisons crossed types.
#[derive(Debug)]
pub struct Results {
    query: Query,
    options: Options,
    walker: Walker,
    tally: Tally,
    yielded: usize,
}

impl Results {
    /// The column names, when the query fixes them. `select *` does not: its
    /// columns are the union over the rows, known only once they are all in.
    #[must_use]
    pub fn columns(&self) -> Option<Vec<String>> {
        match &self.query.select {
            Select::All => None,
            Select::Columns(projections) => {
                Some(projections.iter().map(Projection::column).collect())
            }
        }
    }

    /// What the scan has noticed so far. Complete once the iterator is done.
    #[must_use]
    pub fn tally(&self) -> &Tally {
        &self.tally
    }

    /// Take the tally, leaving an empty one.
    #[must_use]
    pub fn into_tally(self) -> Tally {
        self.tally
    }
}

impl Iterator for Results {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        if self.query.limit.is_some_and(|limit| self.yielded >= limit) {
            return None;
        }
        loop {
            let candidate = self.walker.next(&mut self.tally)?;
            let metadata = match File::open(&candidate.absolute)
                .map_err(slpc::Error::from)
                .and_then(|file| slpc::metadata_of_with(file, self.options.limits))
            {
                Ok(doc) => Value::from_item(doc.as_item()).unwrap_or(Value::Table(Vec::new())),
                Err(e) => {
                    self.tally.skip(candidate.relative, e.to_string());
                    continue;
                }
            };
            let path = candidate.relative.to_string_lossy().into_owned();
            let row = eval::Row {
                metadata: &metadata,
                path: &path,
            };
            if let Some(filter) = &self.query.filter {
                if eval::eval(filter, &row, &mut self.tally) != Truth::True {
                    continue;
                }
            }
            let cells = match &self.query.select {
                Select::All => flatten(&metadata, &path),
                Select::Columns(projections) => projections
                    .iter()
                    .map(|p| Cell {
                        column: p.column(),
                        value: row.lookup(&p.path).map(std::borrow::Cow::into_owned),
                    })
                    .collect(),
            };
            self.yielded += 1;
            return Some(Row { path, cells });
        }
    }
}

/// Every leaf of the metadata as a dotted column, after `@path`.
///
/// Arrays are leaves: an array of tables is one cell, rendered inline, since
/// unfolding it into rows is a different query shape.
fn flatten(metadata: &Value, path: &str) -> Vec<Cell> {
    let mut cells = vec![Cell {
        column: Path::ContainerPath.to_string(),
        value: Some(Value::String(path.to_owned())),
    }];
    let mut route = Vec::new();
    flatten_into(metadata, &mut route, &mut cells);
    cells
}

fn flatten_into(value: &Value, route: &mut Vec<Segment>, cells: &mut Vec<Cell>) {
    match value {
        Value::Table(entries) if !entries.is_empty() => {
            for (key, child) in entries {
                route.push(Segment::Key(key.clone()));
                flatten_into(child, route, cells);
                route.pop();
            }
        }
        _ if route.is_empty() => {}
        leaf => cells.push(Cell {
            column: Path::Keys(route.clone()).to_string(),
            value: Some(leaf.clone()),
        }),
    }
}
