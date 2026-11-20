// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn pack(dir: &Path, content: &str, flyleaf: &str) {
    let doc = format!("slipcase_version = \"1.1\"\n{flyleaf}[content]\nfile = \"{content}\"\n");
    let doc: slpc::toml_edit::DocumentMut = doc.parse().unwrap();
    let out = fs::File::create(dir.join(format!("{content}.slpc"))).unwrap();
    slpc::pack_reader(content, &b"content bytes"[..], doc, out).unwrap();
}

fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    pack(dir.path(), "alpha.pdf", "title = \"Alpha\"\npriority = 1\n");
    pack(dir.path(), "beta.pdf", "title = \"Beta\"\npriority = 3\n");
    fs::create_dir(dir.path().join("sub")).unwrap();
    pack(
        &dir.path().join("sub"),
        "gamma.pdf",
        "title = \"Gamma\"\npriority = 2\n",
    );
    fs::write(dir.path().join("broken.slpc"), "not a zip").unwrap();
    dir
}

fn slipql(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_slipql"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn one_query_prints_tsv_in_a_pipe_and_its_notices_on_standard_error() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    let out = slipql(
        &[root, "-e", "select title, priority where priority > 1"],
        "",
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "title\tpriority\nBeta\t3\n");
    assert!(
        text(&out.stderr).contains("note: skipped"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_bound_directory_is_searched_below_with_recursive() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    let out = slipql(
        &[
            root,
            "-r",
            "--no-header",
            "-e",
            "select title where priority = 2",
        ],
        "",
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "Gamma\n");
}

#[test]
fn json_is_chosen_by_either_flag() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    for flag in [&["--json"][..], &["--mode", "json"][..]] {
        let mut args = vec![root, "-e", "select title where priority = 1"];
        args.extend_from_slice(flag);
        let out = slipql(&args, "");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let got = text(&out.stdout);
        assert!(
            got.contains("\"title\"") && got.contains("\"Alpha\""),
            "{got}"
        );
    }
}

#[test]
fn a_query_that_fails_exits_one() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    let out = slipql(&[root, "-e", "select from where"], "");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).starts_with("slipql: "),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_bad_command_line_exits_two() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    let cases: [&[&str]; 4] = [
        &[root, "--json", "--mode", "csv", "-e", "select title"],
        &["-r", "-e", "select title"],
        &[root, "--mode", "xml", "-e", "select title"],
        &["--no-such-flag"],
    ];
    for args in cases {
        let out = slipql(args, "");
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
}

#[test]
fn a_script_runs_every_line_and_exits_one_when_any_failed() {
    let dir = tree();
    let root = dir.path().to_str().unwrap();
    let script = "\
-- a comment
# another

select title where priority = 1
select from where
select title where priority = 3
";
    let out = slipql(&[root, "--no-header"], script);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stdout), "Alpha\nBeta\n");
    assert_eq!(
        text(&out.stderr).matches("slipql: ").count(),
        1,
        "{}",
        text(&out.stderr)
    );

    let clean = slipql(&[root, "--no-header"], "select title where priority = 1\n");
    assert_eq!(clean.status.code(), Some(0), "{}", text(&clean.stderr));
}
