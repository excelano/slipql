//! End to end: pack containers with `slpc`, run queries, read rows.

use std::fs;
use std::path::Path;

use slipql::render::{Format, Writer};
use slipql::{execute, parse, Kind, Value};

/// A tree of containers and bystanders under a temporary directory.
struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        pack(root, "alpha.pdf", "title = \"Alpha\"\npriority = 1\ntags = [\"draft\", \"finance\"]\ncreated = 2026-01-10T09:00:00Z\n[owner]\nname = \"Kim\"\n");
        pack(root, "beta.docx", "title = \"Beta\"\npriority = 3\ntags = [\"final\"]\ncreated = 2026-02-01T10:00:00+02:00\n[owner]\nname = \"Lee\"\nunit = \"ops\"\n");
        pack(
            root,
            "gamma.txt",
            "title = \"Gamma\"\npriority = \"2\"\nscore = 4.5\n",
        );
        fs::create_dir(root.join("sub")).unwrap();
        pack(&root.join("sub"), "delta.md", "title = \"Delta\"\npriority = 2\n[[history]]\nwhen = 2026-03-01\n[[history]]\nwhen = 2026-03-02\n");
        fs::write(root.join("notes.txt"), "not a container").unwrap();
        fs::write(root.join("broken.slpc"), "not a zip either").unwrap();
        Self { dir }
    }

    fn root(&self) -> String {
        self.dir.path().to_str().unwrap().replace('\\', "/")
    }

    fn query(&self, text: &str) -> slipql::Results {
        let q = parse(&text.replace("ROOT", &self.root())).unwrap();
        execute(&q).unwrap()
    }

    fn render(&self, text: &str, format: Format) -> String {
        let mut results = self.query(text);
        let mut out = Vec::new();
        let mut w = Writer::new(&mut out, format, true, results.columns());
        for row in results.by_ref() {
            w.row(row).unwrap();
        }
        w.finish().unwrap();
        String::from_utf8(out).unwrap()
    }
}

fn pack(dir: &Path, payload: &str, metadata: &str) {
    let doc = format!("slipcase_version = \"1.0\"\n{metadata}[payload]\nfile = \"{payload}\"\n");
    let doc: slpc::toml_edit::DocumentMut = doc.parse().unwrap();
    let out = fs::File::create(dir.join(format!("{payload}.slpc"))).unwrap();
    slpc::pack_reader(payload, &b"payload bytes"[..], doc, out).unwrap();
}

#[test]
fn rows_come_in_path_order_and_skip_what_is_not_a_container() {
    let tree = Tree::new();
    let mut results = tree.query("select title from 'ROOT'");
    let titles: Vec<String> = results
        .by_ref()
        .map(|r| r.cells[0].value.clone().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Alpha", "Beta", "Gamma"]);
    let tally = results.into_tally();
    assert_eq!(tally.skipped().len(), 1);
    assert_eq!(tally.skipped()[0].path.to_str().unwrap(), "broken.slpc");
    assert!(tally.mismatches().is_empty());
}

#[test]
fn recursive_reaches_the_subdirectory() {
    let tree = Tree::new();
    let rows: Vec<_> = tree
        .query("select @path, title from 'ROOT' recursive")
        .collect();
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "alpha.pdf.slpc",
            "beta.docx.slpc",
            "gamma.txt.slpc",
            "sub/delta.md.slpc"
        ]
    );
    assert_eq!(
        rows[0].cells[0].value,
        Some(Value::String("alpha.pdf.slpc".into()))
    );
}

#[test]
fn where_filters_and_limit_stops() {
    let tree = Tree::new();
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' recursive where priority >= 2")
        .collect();
    let titles: Vec<String> = rows
        .iter()
        .map(|r| r.cells[0].value.clone().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Beta", "Delta"]);
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' recursive limit 2")
        .collect();
    assert_eq!(rows.len(), 2);
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' where tags contains 'draft' and owner.name = 'Kim'")
        .collect();
    assert_eq!(rows.len(), 1);
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' where created > 2026-02-01T07:00:00Z")
        .collect();
    assert_eq!(rows.len(), 1);
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' recursive where not exists owner")
        .collect();
    assert_eq!(rows.len(), 2);
    let rows: Vec<_> = tree
        .query("select title from 'ROOT' where @path like '%.docx.slpc'")
        .collect();
    assert_eq!(rows.len(), 1);
}

#[test]
fn mismatches_are_tallied_and_the_row_is_excluded() {
    let tree = Tree::new();
    let mut results = tree.query("select title from 'ROOT' where priority = 2");
    assert_eq!(results.by_ref().count(), 0);
    let tally = results.into_tally();
    let m = tally.mismatches();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].0.to_string(), "priority");
    assert_eq!((m[0].1, m[0].2, m[0].3), (Kind::String, Kind::Integer, 1));
}

#[test]
fn absent_columns_render_empty() {
    let tree = Tree::new();
    let out = tree.render("select title, owner.unit, score from 'ROOT'", Format::Csv);
    assert_eq!(
        out,
        "title,owner.unit,score\nAlpha,,\nBeta,ops,\nGamma,,4.5\n"
    );
}

#[test]
fn star_flattens_leaves_and_unions_columns() {
    let tree = Tree::new();
    let out = tree.render(
        "select * from 'ROOT' recursive where @path like 'sub/%'",
        Format::Csv,
    );
    assert_eq!(
        out,
        "@path,slipcase_version,title,priority,history,payload.file\nsub/delta.md.slpc,1.0,Delta,2,\"[{ when = 2026-03-01 }, { when = 2026-03-02 }]\",delta.md\n"
    );
    let out = tree.render(
        "select * from 'ROOT' where priority = 1 or priority = 3",
        Format::Csv,
    );
    let header = out.lines().next().unwrap();
    assert_eq!(
        header,
        "@path,slipcase_version,title,priority,tags,created,owner.name,payload.file,owner.unit"
    );
}

#[test]
fn table_and_json_output() {
    let tree = Tree::new();
    let out = tree.render(
        "select title as t, priority from 'ROOT' where title in ('Alpha', 'Gamma')",
        Format::Table,
    );
    assert_eq!(out, "| t     | priority |\n| ----- | -------- |\n| Alpha | 1        |\n| Gamma | 2        |\n(2 rows)\n");
    let out = tree.render(
        "select tags, priority from 'ROOT' where title = 'Gamma'",
        Format::Json,
    );
    assert_eq!(out, "[\n  {\"tags\": null, \"priority\": \"2\"}\n]\n");
}

#[test]
fn a_missing_root_is_an_error_up_front() {
    let q = parse("select * from '/nonexistent/dir/for/slipql'").unwrap();
    let err = execute(&q).unwrap_err();
    assert!(
        err.to_string().starts_with("cannot scan /nonexistent"),
        "{err}"
    );
}
