# slipql grammar

The language `slipql` accepts, and what each construct means. Anything outside this grammar is a parse error that points at the offending token, never a silent reinterpretation.

## Notation

`:=` defines a rule, `|` separates alternatives, `( )?` is optional, `( )*` is zero or more, `( )+` is one or more. Terminals are in double quotes. Whitespace separates tokens and is otherwise ignored. A comment runs from `--` or `#` to the end of the line.

## Statement

```ebnf
query      := "select" projection "from" source ( "where" predicate )? ( "limit" integer )?
projection := "*" | column ( "," column )*
column     := path ( "as" name )?
source     := string ( "recursive" )?
name       := word | string
```

Keywords are lowercase and case-sensitive. `SELECT` is not a keyword; the error for it says so.

`from` names a directory as a string. Only that directory is scanned unless `recursive` is written, in which case every subdirectory is too, depth first. Symbolic links to directories are not followed. Entries are visited in name order, so the same tree yields the same rows in the same order every time.

A session bound to a directory at startup may omit `from`; the library's `parse_with` is the same allowance. A `from` written in the query wins. The library's plain `parse` requires it.

`limit` stops the scan after that many rows. There is no `order by`: rows arrive in scan order, and sorting would mean holding every row before returning the first.

## Paths

```ebnf
path    := "@path" | segment ( "." segment | "[" integer "]" )*
segment := word | string
word    := ( letter | digit | "_" | "-" )+
```

A path is a route into the metadata document: keys joined by `.` into nested tables, `[n]` into arrays, zero-based. A quoted string in path position is a quoted key, as it is in TOML, so `"my key".sub` reaches a key with a space in it. A keyword can be a key when quoted, or when followed by `.` or `[`.

`@path` is the container's path relative to the `from` directory, with the platform's separators. It is the only built-in column. TOML keys never contain `@`, so it cannot collide with one.

`select *` produces `@path` and then every leaf of the metadata as a dotted column, in document order. An array is a leaf, arrays of tables included; a cell holding one renders the array inline. The column set is the union across the rows returned, in the order first seen.

## Literals

```ebnf
literal  := string | integer | float | boolean | datetime
string   := '"' basic-string '"' | "'" literal-string "'"
boolean  := "true" | "false"
```

Literals are TOML's, spelled as a metadata document spells them. A basic string takes TOML's escapes (`\n`, `\t`, `\"`, `\\`, `\uXXXX`, `\UXXXXXXXX`); a literal string takes nothing, which suits Windows paths. Integers allow underscores and the `0x`, `0o`, and `0b` prefixes; floats allow a fraction, an exponent, `inf`, and `nan`. Datetimes are TOML's four forms: `2026-01-01T09:00:00Z`, `2026-01-01T09:00:00`, `2026-01-01`, and `09:00:00`, with `T` replaceable by a space and seconds optional. Multi-line strings and array literals are not accepted.

## Predicates

```ebnf
predicate   := disjunction
disjunction := conjunction ( "or" conjunction )*
conjunction := negation ( "and" negation )*
negation    := "not" negation | atom
atom        := "(" predicate ")"
             | "exists" path
             | path "not"? test
test        := op literal
             | "in" "(" literal ( "," literal )* ")"
             | ( "like" | "ilike" ) string
             | "contains" literal
op          := "=" | "!=" | "<" | ">" | "<=" | ">="
```

Precedence from loosest to tightest is `or`, `and`, `not`. `not` may also sit between the path and its test, as SQL allows: `status not in ("draft")` and `not status in ("draft")` mean the same.

The left side of a test is always a path and the right side always a literal. There is no column-to-column comparison and no arithmetic.

## Semantics

**Three values.** A condition is true, false, or unknown, and a row is returned only when its whole `where` clause is true. `not` unknown is unknown; `and` with a false side is false and with an unknown side is otherwise unknown; `or` with a true side is true and with an unknown side is otherwise unknown. This is SQL's rule for null, applied to absence.

**Absence.** A path that does not resolve, because a key is missing, an index is past the end, or a step goes through something that is not a table or array, makes every comparison, `in`, `like`, and `contains` unknown. `exists` is the exception: it is true when the path resolves and false when it does not, never unknown. There is no `is null`. TOML has no null, so a key is present or absent and nothing else, and `exists` is the one test for it.

**Type classes.** Values compare only within a class. Integer and float are one class and compare numerically. Strings compare by code point. Booleans compare with false below true. Each of TOML's four datetime forms is its own class: an offset datetime never compares with a local one, and a local date never compares with a local datetime. Offset datetimes compare as instants, so `09:00:00+02:00` equals `07:00:00Z`. A comparison across classes is unknown, and the run tallies it by path and the two types involved, so a `"3"` where a `3` was meant is reported rather than hidden. There is no coercion between strings and numbers. `nan` compares with nothing, itself included.

**`in`** is true when the value equals any listed literal, false when it equals none of them and every comparison was within class, and unknown otherwise.

**`like`** applies to strings only; `%` matches any run of characters, `_` one character, and a backslash makes the next pattern character literal. Matching is by character, not byte, and case-sensitive. `ilike` folds both sides to lowercase first. A non-string on the left is a class mismatch.

**`contains`** applies to arrays only and is true when any element equals the literal within its class. Elements of another class simply do not match. A non-array on the left is a class mismatch.

## Output

A `select` with named columns renders those columns in the order written, an absent value as an empty cell. Strings render bare; every other value renders as TOML would write it inline, so a nested array or table reads back as TOML. JSON output keeps integers, floats, booleans, arrays, and tables as their JSON counterparts and renders datetimes and non-finite floats as strings.

## Not in the language

Writes of any kind. Aggregates, `group by`, `order by`, `distinct`, joins, subqueries. Unfolding an array of tables into rows, which is a different query shape from a predicate. Searching payload content, which is another engine's job. Each is an absence rather than a refusal; the parser names what it did not expect.
