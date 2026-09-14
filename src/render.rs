//! Rows to text: a table for a terminal, CSV and TSV for a pipe, JSON for
//! anything that wants the types kept.

use std::fmt::Write as _;
use std::io::{self, Write};

use unicode_width::UnicodeWidthStr;

use crate::exec::Row;
use crate::value::{write_float, Value};

/// An output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A markdown-pipe table with a row count under it.
    Table,
    /// Tab-separated, cells verbatim.
    Tsv,
    /// RFC 4180: comma-separated, quoted where a cell needs it.
    Csv,
    /// An array of objects, one per row, with TOML's types kept where JSON
    /// has them and rendered as strings where it does not.
    Json,
}

impl Format {
    /// The format's name as a command line spells it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Tsv => "tsv",
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    /// The names a command line accepts.
    pub const NAMES: [&'static str; 4] = ["table", "tsv", "csv", "json"];
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "table" => Ok(Self::Table),
            "tsv" => Ok(Self::Tsv),
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unknown format `{other}`; want table, tsv, csv, or json"
            )),
        }
    }
}

/// Writes rows as they arrive.
///
/// With known columns, CSV, TSV, and JSON go out row by row. A table waits
/// for every row, because column widths need them all; so does any format
/// when the columns are not known up front, which is `select *`.
pub struct Writer<W: Write> {
    out: W,
    format: Format,
    headers: bool,
    columns: Option<Vec<String>>,
    buffered: Vec<Row>,
    written: usize,
}

impl<W: Write> Writer<W> {
    /// Start writing. `columns` is what [`crate::Results::columns`] gave.
    pub fn new(out: W, format: Format, headers: bool, columns: Option<Vec<String>>) -> Self {
        Self {
            out,
            format,
            headers,
            columns,
            buffered: Vec::new(),
            written: 0,
        }
    }

    /// Write one row, or hold it until [`Writer::finish`] if the format has to.
    pub fn row(&mut self, row: Row) -> io::Result<()> {
        let streams = self.columns.is_some() && self.format != Format::Table;
        if !streams {
            self.buffered.push(row);
            return Ok(());
        }
        if self.written == 0 {
            self.start()?;
        }
        let columns = self.columns.clone().unwrap_or_default();
        self.write_row(&columns, &row)?;
        self.written += 1;
        Ok(())
    }

    /// Write whatever is held, and the closing the format needs.
    pub fn finish(mut self) -> io::Result<()> {
        if self.columns.is_none() {
            self.columns = Some(column_union(&self.buffered));
        }
        let columns = self.columns.clone().unwrap_or_default();
        if self.format == Format::Table {
            let rows = std::mem::take(&mut self.buffered);
            return self.write_table(&columns, &rows);
        }
        if self.written == 0 {
            self.start()?;
        }
        let rows = std::mem::take(&mut self.buffered);
        for row in &rows {
            self.write_row(&columns, row)?;
            self.written += 1;
        }
        if self.format == Format::Json {
            if self.written > 0 {
                self.out.write_all(b"\n")?;
            }
            self.out.write_all(b"]\n")?;
        }
        self.out.flush()
    }

    fn start(&mut self) -> io::Result<()> {
        let columns = self.columns.clone().unwrap_or_default();
        match self.format {
            Format::Json => self.out.write_all(b"["),
            Format::Csv if self.headers => {
                let line: Vec<String> = columns.iter().map(|c| csv_quote(c)).collect();
                writeln!(self.out, "{}", line.join(","))
            }
            Format::Tsv if self.headers => writeln!(self.out, "{}", columns.join("\t")),
            _ => Ok(()),
        }
    }

    fn write_row(&mut self, columns: &[String], row: &Row) -> io::Result<()> {
        match self.format {
            Format::Csv => {
                let cells: Vec<String> = columns
                    .iter()
                    .map(|c| csv_quote(&cell_text(row, c)))
                    .collect();
                writeln!(self.out, "{}", cells.join(","))
            }
            Format::Tsv => {
                let cells: Vec<String> = columns.iter().map(|c| cell_text(row, c)).collect();
                writeln!(self.out, "{}", cells.join("\t"))
            }
            Format::Json => {
                let mut s = String::new();
                if self.written > 0 {
                    s.push(',');
                }
                s.push_str("\n  {");
                for (i, column) in columns.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    write_json_string(&mut s, column);
                    s.push_str(": ");
                    match cell_value(row, column) {
                        Some(v) => write_json_value(&mut s, v),
                        None => s.push_str("null"),
                    }
                }
                s.push('}');
                self.out.write_all(s.as_bytes())
            }
            Format::Table => unreachable!("tables are written whole"),
        }
    }

    fn write_table(&mut self, columns: &[String], rows: &[Row]) -> io::Result<()> {
        if columns.is_empty() {
            return writeln!(self.out, "({})", count(rows.len()));
        }
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|row| columns.iter().map(|c| cell_text(row, c)).collect())
            .collect();
        let mut widths: Vec<usize> = columns.iter().map(|c| c.width()).collect();
        for row in &cells {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.width());
            }
        }
        if self.headers {
            self.write_table_row(columns, &widths)?;
            let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            self.write_table_row(&rule, &widths)?;
        }
        for row in &cells {
            self.write_table_row(row, &widths)?;
        }
        writeln!(self.out, "({})", count(rows.len()))?;
        self.out.flush()
    }

    fn write_table_row(&mut self, cells: &[String], widths: &[usize]) -> io::Result<()> {
        let mut line = String::from("|");
        for (cell, width) in cells.iter().zip(widths) {
            let pad = width.saturating_sub(cell.width());
            let _ = write!(line, " {cell}{} |", " ".repeat(pad));
        }
        writeln!(self.out, "{line}")
    }
}

/// The columns `select *` produces: every column any row has, in the order
/// first seen.
#[must_use]
pub fn column_union(rows: &[Row]) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        for cell in &row.cells {
            if !columns.contains(&cell.column) {
                columns.push(cell.column.clone());
            }
        }
    }
    columns
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 row".to_owned()
    } else {
        format!("{n} rows")
    }
}

fn cell_value<'a>(row: &'a Row, column: &str) -> Option<&'a Value> {
    row.cells
        .iter()
        .find(|c| c.column == column)
        .and_then(|c| c.value.as_ref())
}

fn cell_text(row: &Row, column: &str) -> String {
    cell_value(row, column)
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// Quote a CSV field when RFC 4180 says it needs it.
fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// JSON has strings, numbers, booleans, arrays, and objects. A datetime is a
/// string in TOML's spelling; a float JSON cannot spell is a string too.
fn write_json_value(out: &mut String, value: &Value) {
    match value {
        Value::String(s) => write_json_string(out, s),
        Value::Integer(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(x) if x.is_finite() => {
            let _ = write_float(out, *x);
        }
        Value::Float(x) => {
            let mut s = String::new();
            let _ = write_float(&mut s, *x);
            write_json_string(out, &s);
        }
        Value::Boolean(b) => {
            let _ = write!(out, "{b}");
        }
        Value::Datetime(d) => write_json_string(out, &d.to_string()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json_value(out, item);
            }
            out.push(']');
        }
        Value::Table(entries) => {
            out.push('{');
            for (i, (key, item)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json_string(out, key);
                out.push_str(": ");
                write_json_value(out, item);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::Cell;

    fn row(path: &str, cells: &[(&str, Option<Value>)]) -> Row {
        Row {
            path: path.into(),
            cells: cells
                .iter()
                .map(|(c, v)| Cell {
                    column: (*c).into(),
                    value: v.clone(),
                })
                .collect(),
        }
    }

    fn render(format: Format, headers: bool, columns: Option<Vec<&str>>, rows: Vec<Row>) -> String {
        let mut out = Vec::new();
        let columns = columns.map(|c| c.into_iter().map(String::from).collect());
        let mut w = Writer::new(&mut out, format, headers, columns);
        for r in rows {
            w.row(r).unwrap();
        }
        w.finish().unwrap();
        String::from_utf8(out).unwrap()
    }

    fn sample() -> Vec<Row> {
        vec![
            row(
                "a.slpc",
                &[
                    ("title", Some(Value::String("Q3, \"final\"".into()))),
                    ("n", Some(Value::Integer(1))),
                ],
            ),
            row(
                "b.slpc",
                &[("title", Some(Value::String("日本".into()))), ("n", None)],
            ),
        ]
    }

    #[test]
    fn table() {
        let out = render(Format::Table, true, Some(vec!["title", "n"]), sample());
        assert_eq!(
            out,
            "| title       | n |\n| ----------- | - |\n| Q3, \"final\" | 1 |\n| 日本        |   |\n(2 rows)\n"
        );
        let out = render(
            Format::Table,
            false,
            Some(vec!["n"]),
            sample()[..1].to_vec(),
        );
        assert_eq!(out, "| 1 |\n(1 row)\n");
        assert_eq!(render(Format::Table, true, None, vec![]), "(0 rows)\n");
    }

    #[test]
    fn csv_and_tsv() {
        let out = render(Format::Csv, true, Some(vec!["title", "n"]), sample());
        assert_eq!(out, "title,n\n\"Q3, \"\"final\"\"\",1\n日本,\n");
        let out = render(Format::Tsv, false, Some(vec!["n", "title"]), sample());
        assert_eq!(out, "1\tQ3, \"final\"\n\t日本\n");
    }

    #[test]
    fn json_keeps_types() {
        let rows = vec![row(
            "a.slpc",
            &[
                ("s", Some(Value::String("x\"y".into()))),
                ("i", Some(Value::Integer(2))),
                ("f", Some(Value::Float(2.0))),
                ("nan", Some(Value::Float(f64::NAN))),
                ("b", Some(Value::Boolean(false))),
                ("d", Some(Value::Datetime("2026-01-01".parse().unwrap()))),
                (
                    "arr",
                    Some(Value::Array(vec![
                        Value::Integer(1),
                        Value::String("a".into()),
                    ])),
                ),
                (
                    "t",
                    Some(Value::Table(vec![("k".into(), Value::Boolean(true))])),
                ),
                ("none", None),
            ],
        )];
        let columns = Some(vec!["s", "i", "f", "nan", "b", "d", "arr", "t", "none"]);
        let out = render(Format::Json, true, columns, rows);
        assert_eq!(
            out,
            "[\n  {\"s\": \"x\\\"y\", \"i\": 2, \"f\": 2.0, \"nan\": \"nan\", \"b\": false, \"d\": \"2026-01-01\", \"arr\": [1, \"a\"], \"t\": {\"k\": true}, \"none\": null}\n]\n"
        );
        assert_eq!(render(Format::Json, true, Some(vec!["a"]), vec![]), "[]\n");
    }

    #[test]
    fn star_takes_the_union_in_first_seen_order() {
        let rows = vec![
            row(
                "a",
                &[
                    ("@path", Some(Value::String("a".into()))),
                    ("x", Some(Value::Integer(1))),
                ],
            ),
            row(
                "b",
                &[
                    ("@path", Some(Value::String("b".into()))),
                    ("y", Some(Value::Integer(2))),
                    ("x", Some(Value::Integer(3))),
                ],
            ),
        ];
        assert_eq!(column_union(&rows), vec!["@path", "x", "y"]);
        let out = render(Format::Csv, true, None, rows);
        assert_eq!(out, "@path,x,y\na,1,\nb,3,2\n");
    }
}
