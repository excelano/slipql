//! The `slipql` command: a one-shot query, a script on standard input, or a
//! prompt bound to a directory.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;
use std::process::exit;

use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use slipql::ast::Source;
use slipql::render::{Format, Writer};
use slipql::{execute, parse_with, Error, Tally};

#[derive(Parser, Debug)]
#[command(
    name = "slipql",
    version,
    about = "Query Slipcase metadata with select, from, and where",
    long_about = "Runs select/from/where queries over the metadata of Slipcase containers in a \
directory. Each container is a row and each metadata key a column.\n\n\
With a directory, queries may leave out `from`; a `from` in the query wins. \
With --exec the query runs and the command exits. With standard input not a \
terminal, each line is run as a query. Otherwise a prompt opens.\n\n\
Exit status is 0 when every query ran, 1 when one failed, 2 for a bad command line.",
    after_help = "Formats: table (the default at a terminal), tsv (the default in a pipe), csv, json."
)]
struct Cli {
    /// Directory to scan; queries may then omit `from`
    #[arg(value_name = "DIR")]
    dir: Option<PathBuf>,

    /// Descend into subdirectories
    #[arg(short, long)]
    recursive: bool,

    /// Run one query and exit
    #[arg(short = 'e', long, value_name = "QUERY")]
    exec: Option<String>,

    /// Output format
    #[arg(long, value_name = "FORMAT", value_parser = Format::NAMES)]
    mode: Option<String>,

    /// Shorthand for --mode json
    #[arg(long)]
    json: bool,

    /// Leave out the header row
    #[arg(long)]
    no_header: bool,
}

/// What the session knows between queries.
struct Session {
    source: Option<Source>,
    format: Format,
    headers: bool,
}

fn main() {
    let cli = Cli::parse();
    let format = match (cli.mode.as_deref(), cli.json) {
        (Some(mode), true) if mode != "json" => {
            eprintln!("slipql: --json and --mode {mode} disagree; give one");
            exit(2);
        }
        (Some(mode), _) => mode.parse().unwrap_or(Format::Table),
        (None, true) => Format::Json,
        (None, false) if io::stdout().is_terminal() => Format::Table,
        (None, false) => Format::Tsv,
    };
    let source = cli.dir.map(|root| Source {
        root,
        recursive: cli.recursive,
    });
    if source.is_none() && cli.recursive {
        eprintln!("slipql: --recursive needs a directory to apply to; write `recursive` in the query's from clause instead");
        exit(2);
    }
    let mut session = Session {
        source,
        format,
        headers: !cli.no_header,
    };

    if let Some(query) = cli.exec {
        if let Err(e) = run(&session, &query) {
            eprintln!("slipql: {e}");
            exit(1);
        }
        return;
    }
    if !io::stdin().is_terminal() {
        exit(script(&session));
    }
    if let Err(e) = repl(&mut session) {
        eprintln!("slipql: {e}");
        exit(1);
    }
}

/// Run one query and print its rows and then its notices.
fn run(session: &Session, text: &str) -> Result<(), Error> {
    let query = parse_with(text, session.source.as_ref())?;
    let mut results = execute(&query)?;
    let stdout = io::stdout();
    let mut writer = Writer::new(
        stdout.lock(),
        session.format,
        session.headers,
        results.columns(),
    );
    for row in results.by_ref() {
        if let Err(e) = writer.row(row) {
            return finish_output(e);
        }
    }
    if let Err(e) = writer.finish() {
        return finish_output(e);
    }
    report(results.tally());
    Ok(())
}

/// A closed pipe is the reader having seen enough, not a failure.
fn finish_output(e: io::Error) -> Result<(), Error> {
    if e.kind() == io::ErrorKind::BrokenPipe {
        exit(0);
    }
    eprintln!("slipql: cannot write output: {e}");
    exit(1);
}

/// Notices go to standard error after the rows, so a pipe stays clean.
fn report(tally: &Tally) {
    for skipped in tally.skipped() {
        eprintln!(
            "note: skipped {}: {}",
            skipped.path.display(),
            skipped.reason
        );
    }
    for (path, found, against, count) in tally.mismatches() {
        let rows = if count == 1 {
            "1 row".to_owned()
        } else {
            format!("{count} rows")
        };
        eprintln!("note: {path}: {found} in {rows}, {against} in the query; not matched");
    }
}

/// Queries from standard input, one per line. Every line runs; the exit
/// status says whether any failed.
fn script(session: &Session) -> i32 {
    let mut status = 0;
    for line in io::stdin().lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("slipql: cannot read standard input: {e}");
                return 1;
            }
        };
        let text = line.trim();
        if text.is_empty() || text.starts_with("--") || text.starts_with('#') {
            continue;
        }
        if let Err(e) = run(session, text) {
            eprintln!("slipql: {e}");
            status = 1;
        }
    }
    status
}

fn repl(session: &mut Session) -> Result<(), Box<dyn std::error::Error>> {
    let mut editor = DefaultEditor::new()?;
    let history = history_path();
    if let Some(path) = &history {
        let _ = editor.load_history(path);
    }
    match &session.source {
        Some(source) => eprintln!(
            "Connected to: {}{}. Type \"help\" for commands, \"quit\" to exit.",
            source.root.display(),
            if source.recursive { " (recursive)" } else { "" }
        ),
        None => eprintln!("No directory bound, so each query needs a from clause. Type \"help\" for commands, \"quit\" to exit."),
    }
    loop {
        let line = match editor.readline("slipql> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(text);
        match meta(session, text) {
            Meta::Quit => break,
            Meta::Handled => continue,
            Meta::Query => {}
        }
        if let Err(e) = run(session, text) {
            eprintln!("error: {e}");
            if let Error::Parse { offset, .. } = e {
                point_at(text, offset);
            }
        }
    }
    if let Some(path) = &history {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = editor.save_history(path);
    }
    Ok(())
}

/// What a line at the prompt turned out to be.
enum Meta {
    Quit,
    Handled,
    Query,
}

/// The prompt's own commands, plain words with no leading dot.
fn meta(session: &mut Session, text: &str) -> Meta {
    let mut words = text.split_whitespace();
    let word = words.next().unwrap_or_default().to_ascii_lowercase();
    let arg = words.next();
    let extra = words.next().is_some();
    match (word.as_str(), arg, extra) {
        ("quit" | "exit", None, false) => Meta::Quit,
        ("help" | "?", None, false) => {
            eprintln!("{HELP}");
            Meta::Handled
        }
        ("mode", None, false) => {
            eprintln!("mode {}", session.format.name());
            Meta::Handled
        }
        ("mode", Some(name), false) => {
            match name.parse() {
                Ok(format) => session.format = format,
                Err(e) => eprintln!("error: {e}"),
            }
            Meta::Handled
        }
        ("headers", None, false) => {
            eprintln!("headers {}", if session.headers { "on" } else { "off" });
            Meta::Handled
        }
        ("headers", Some("on"), false) => {
            session.headers = true;
            Meta::Handled
        }
        ("headers", Some("off"), false) => {
            session.headers = false;
            Meta::Handled
        }
        ("headers", Some(_), _) => {
            eprintln!("error: headers takes on or off");
            Meta::Handled
        }
        _ => Meta::Query,
    }
}

const HELP: &str = "\
Queries
  select <columns> [from '<dir>' [recursive]] [where <condition>] [limit <n>]
  select * from '.' where tags contains \"draft\" and priority > 2
  select @path, title, owner.name where exists owner and created >= 2026-01-01

Columns are metadata keys with dots into tables and [n] into arrays; @path is
the container's path under the from directory. Literals are written as TOML:
\"strings\", 42, 1.5, true, 2026-01-01, 09:30:00. Conditions: = != < > <= >=,
in (...), like and ilike with % and _, contains for arrays, exists for keys.

Commands
  mode [table|tsv|csv|json]   show or set the output format
  headers [on|off]            show or set whether the header row prints
  help                        this text
  quit                        leave";

/// Print a caret under the byte offset an error named.
fn point_at(text: &str, offset: usize) {
    let column = text[..offset.min(text.len())].chars().count();
    eprintln!("  {text}");
    eprintln!("  {}^", " ".repeat(column));
}

/// Where the prompt keeps its history: the platform's configuration
/// directory, under this tool's name.
fn history_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("slipql").join("history"))
}
