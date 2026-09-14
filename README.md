# slipql

A query language for [Slipcase](https://slipcaseformat.org) metadata. Point it at a directory of `.slpc` containers and ask questions in `select`, `from`, and `where`: each container is a row, each metadata key a column.

```text
$ slipql ./contracts --recursive
Connected to: ./contracts (recursive). Type "help" for commands, "quit" to exit.

slipql> select @path, title, governance.owner where status = "draft" or tags contains "legal"
| @path                  | title                     | governance.owner |
| ---------------------- | ------------------------- | ---------------- |
| 2026/renewal.docx.slpc | Renewal, 2026             |                  |
| msa.pdf.slpc           | Master services agreement | Kim              |
| q3.xlsx.slpc           | Q3 report                 | Lee              |
(3 rows)
```

## Why

A Slipcase container carries a TOML document describing its payload, and that description travels with the file. Once a directory holds a few hundred of them, the question stops being "what is in this file" and becomes "which files say this". Unpacking every container to find out, or keeping an index that goes stale the moment someone copies a file in, both defeat the point of metadata that lives with the document.

`slipql` reads each container's metadata in place, through the same library the `slipcase` command uses, and never unpacks a payload. It keeps no index and no state: every query is a fresh scan, so the answer is what is on disk now. It changes nothing.

The language borrows SQL's clause shape and TOML's literals, so anyone who can write a metadata document can write a query against one without learning a second date syntax or a second way to quote a string.

## Install

### Debian and Ubuntu

Add the [Excelano apt repository](https://excelano.com/apt/) once:

```sh
curl -fsSL https://excelano.com/apt/setup.sh | sudo sh
```

Then install it, so `apt upgrade` keeps it current:

```sh
sudo apt install slipql
```

### Homebrew

```sh
brew install excelano/tap/slipql
```

### crates.io

```sh
cargo install slipql
```

### Windows

```powershell
winget install Excelano.slipql
```

Without winget, in PowerShell:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/excelano/slipql/releases/latest/download/slipql-installer.ps1 | iex"
```

### Curl (any Linux or macOS)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/excelano/slipql/releases/latest/download/slipql-installer.sh | sh
```

Every release also carries plain archives for macOS and Linux on both Intel and ARM, and Windows on Intel, each with a `.sha256` beside it.

## The command

`slipql <dir>` opens a prompt bound to a directory, so queries can leave out `from`. Add `--recursive` to descend into subdirectories. `--exec` runs one query and exits, and a query on standard input runs without a prompt, one per line, which is how a script uses it:

```sh
slipql ./contracts -r -e 'select title, signed where signed >= 2026-01-01' --mode csv > signed.csv
```

Output is a table at a terminal and tab-separated in a pipe; `--mode` picks `table`, `tsv`, `csv`, or `json` explicitly. CSV output is meant as a feed for the rest of the tabular family, [xled](https://github.com/excelano/xled) among them. JSON keeps TOML's types where JSON has them. A container that cannot be read is skipped and reported on standard error after the rows, as is any comparison the query made across types, so a pipe stays clean and nothing is silently ignored.

`slipql --help` states the flags and the exit status contract.

## The language

```text
select <columns> from '<dir>' [recursive] [where <condition>] [limit <n>]
```

Columns are metadata keys, with dots into nested tables and `[n]` into arrays: `governance.privacy_flag`, `tags[0]`. `@path` is the container's path under the `from` directory. `select *` gives `@path` and then every leaf value across the rows returned, as dotted columns. A column a row does not have renders empty rather than failing the query, because metadata keys are ad hoc by design.

Conditions compare a column with a literal using `=`, `!=`, `<`, `>`, `<=`, and `>=`, test membership with `in (...)`, match strings with `like` and `ilike`, look inside arrays with `contains`, and test for a key with `exists`. Combine with `and`, `or`, `not`, and parentheses. Literals are TOML's: `"strings"` or `'literal strings'`, `42`, `1.5`, `true`, and datetimes as TOML writes them, `2026-01-01` or `2026-01-01T09:00:00+02:00`.

Keywords are lowercase. TOML has no null, so there is no `is null`; a key is present or it is absent, and `exists` is the one test for that. Comparing a column that is absent, or one whose type does not match the literal, is neither true nor false and does not select the row. Values compare within their type: integers with floats, strings with strings, each of TOML's four datetime forms with itself, and offset datetimes as instants. There is no coercion between strings and numbers, on purpose, since a `"3"` where a `3` was meant is a mistake worth seeing rather than one worth hiding.

[GRAMMAR.md](GRAMMAR.md) is the exact grammar and the semantics of every operator.

## The library

The crate is the query engine, and the command is a thin shell over it. A program that embeds it takes the crate with `default-features = false`, which leaves out the command's own dependencies.

```rust
use slipql::{execute, parse};

fn main() -> slipql::Result<()> {
    let query = parse("select @path, title from '.' recursive where exists title")?;
    let mut results = execute(&query)?;
    for row in results.by_ref() {
        println!("{}: {:?}", row.path, row.cells[1].value);
    }
    for skipped in results.tally().skipped() {
        eprintln!("skipped {}: {}", skipped.path.display(), skipped.reason);
    }
    Ok(())
}
```

Rows come out of an iterator as the scan reaches them, so a query over a large tree starts answering at once, `limit` stops the scan early, and dropping the iterator cancels it. Once the rows are out, the tally says what was skipped and which comparisons crossed types. [docs.rs/slipql](https://docs.rs/slipql) is the library's own page.

## License

MIT. See [LICENSE](https://github.com/excelano/slipql/blob/main/LICENSE).
