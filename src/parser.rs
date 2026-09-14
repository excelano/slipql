//! Tokens to a [`Query`].
//!
//! Keywords are lowercase and case-sensitive. A word that would be a keyword
//! in another case gets an error saying so, because that is the one mistake
//! everyone arriving from SQL makes first.

use std::path::PathBuf;

use crate::ast::{Op, Path, Predicate, Projection, Query, Segment, Select, Source};
use crate::error::{Error, Result};
use crate::lexer::{tokenize, Token, TokenKind};
use crate::value::Value;

/// Parse a query. `from` is required.
pub fn parse(text: &str) -> Result<Query> {
    parse_with(text, None)
}

/// Parse a query, supplying the `from` clause when the text omits one.
///
/// This is how a session bound to a directory lets the prompt skip `from`.
/// A `from` written in the text wins over the default.
pub fn parse_with(text: &str, default_source: Option<&Source>) -> Result<Query> {
    let tokens = tokenize(text)?;
    let mut parser = Parser { tokens, pos: 0 };
    let query = parser.query(default_source)?;
    parser.expect_end()?;
    Ok(query)
}

const KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "recursive",
    "as",
    "and",
    "or",
    "not",
    "exists",
    "contains",
    "in",
    "like",
    "ilike",
    "limit",
    "true",
    "false",
    "inf",
    "nan",
];

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek_kind(), TokenKind::Word(w) if w == word)
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if self.is_word(word) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.peek_kind() == kind {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error_here(&self, message: impl Into<String>) -> Error {
        Error::parse(self.peek().offset, message)
    }

    /// An error for the token here, naming what was wanted and, when the
    /// token is a keyword in the wrong case, saying that instead.
    fn expected(&self, what: &str) -> Error {
        if let TokenKind::Word(w) = self.peek_kind() {
            let lower = w.to_ascii_lowercase();
            if lower != *w && KEYWORDS.contains(&lower.as_str()) {
                return self.error_here(format!(
                    "keywords are lowercase: write `{lower}`, not `{w}`"
                ));
            }
        }
        self.error_here(format!(
            "expected {what}, found {}",
            self.peek_kind().describe()
        ))
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.eat_word(word) {
            Ok(())
        } else {
            Err(self.expected(&format!("`{word}`")))
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<()> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.expected(what))
        }
    }

    fn expect_end(&self) -> Result<()> {
        match self.peek_kind() {
            TokenKind::End => Ok(()),
            _ => Err(self.expected("end of query")),
        }
    }

    fn query(&mut self, default_source: Option<&Source>) -> Result<Query> {
        self.expect_word("select")?;
        let select = self.select_list()?;
        let from_written = self.is_word("from");
        let from = if from_written {
            self.advance();
            self.source()?
        } else if let Some(source) = default_source {
            source.clone()
        } else {
            return Err(self.expected("`from`"));
        };
        let filter = if self.eat_word("where") {
            Some(self.predicate()?)
        } else {
            None
        };
        let limit = if self.eat_word("limit") {
            Some(self.limit()?)
        } else {
            None
        };
        if !matches!(self.peek_kind(), TokenKind::End) {
            let mut clauses = Vec::new();
            if filter.is_none() && limit.is_none() {
                if !from_written && default_source.is_some() {
                    clauses.push("`from`");
                }
                clauses.push("`where`");
            }
            if limit.is_none() {
                clauses.push("`limit`");
            }
            clauses.push("end of query");
            return Err(self.expected(&clauses.join(", ")));
        }
        Ok(Query {
            select,
            from,
            filter,
            limit,
        })
    }

    fn select_list(&mut self) -> Result<Select> {
        if self.eat(&TokenKind::Star) {
            return Ok(Select::All);
        }
        let mut columns = Vec::new();
        loop {
            let path = self.path()?;
            let alias = if self.eat_word("as") {
                Some(self.column_name()?)
            } else {
                None
            };
            columns.push(Projection { path, alias });
            if !self.eat(&TokenKind::Comma) {
                return Ok(Select::Columns(columns));
            }
        }
    }

    /// The name after `as`: a bare word or a quoted string.
    fn column_name(&mut self) -> Result<String> {
        match self.peek_kind().clone() {
            TokenKind::Word(w) => {
                self.advance();
                Ok(w)
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(s)
            }
            _ => Err(self.expected("a column name after `as`")),
        }
    }

    fn source(&mut self) -> Result<Source> {
        let TokenKind::Str(root) = self.peek_kind().clone() else {
            return Err(self.expected("a quoted directory path after `from`"));
        };
        self.advance();
        let recursive = self.eat_word("recursive");
        Ok(Source {
            root: PathBuf::from(root),
            recursive,
        })
    }

    fn limit(&mut self) -> Result<usize> {
        match *self.peek_kind() {
            TokenKind::Integer(n) if n >= 0 => {
                self.advance();
                usize::try_from(n).map_err(|_| self.error_here("limit is too large"))
            }
            _ => Err(self.expected("a non-negative integer after `limit`")),
        }
    }

    /// A path: `@path`, or key segments joined by `.` with `[n]` indexes.
    ///
    /// A quoted string in path position is a quoted key, as it is in TOML.
    fn path(&mut self) -> Result<Path> {
        if let TokenKind::At(name) = self.peek_kind().clone() {
            if name != "path" {
                return Err(self.error_here(format!(
                    "unknown built-in column `@{name}`; the only one is `@path`"
                )));
            }
            self.advance();
            return Ok(Path::ContainerPath);
        }
        let mut segments = vec![Segment::Key(self.key_segment()?)];
        loop {
            if self.eat(&TokenKind::Dot) {
                segments.push(Segment::Key(self.key_segment()?));
            } else if self.eat(&TokenKind::LBracket) {
                let index = match *self.peek_kind() {
                    TokenKind::Integer(n) if n >= 0 => {
                        self.advance();
                        usize::try_from(n).map_err(|_| self.error_here("index is too large"))?
                    }
                    _ => return Err(self.expected("a non-negative integer index")),
                };
                self.expect(&TokenKind::RBracket, "`]`")?;
                segments.push(Segment::Index(index));
            } else {
                return Ok(Path::Keys(segments));
            }
        }
    }

    fn key_segment(&mut self) -> Result<String> {
        match self.peek_kind().clone() {
            TokenKind::Word(w) => {
                if KEYWORDS.contains(&w.as_str()) && !self.word_reads_as_key() {
                    return Err(self.error_here(format!(
                        "`{w}` is a keyword here; quote it to use it as a key"
                    )));
                }
                self.advance();
                Ok(w)
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(s)
            }
            // TOML allows a bare key of digits, which the lexer reads as an
            // integer. Only a plain decimal one can be a key.
            TokenKind::Integer(n) if n >= 0 => {
                let token = self.advance();
                let raw = format!("{n}");
                if raw.len() != token.len {
                    return Err(Error::parse(
                        token.offset,
                        "a numeric key must be plain decimal digits",
                    ));
                }
                Ok(raw)
            }
            _ => Err(self.expected("a key")),
        }
    }

    /// Whether the keyword-shaped word here is followed by something only a
    /// key could be followed by, which makes it a key after all. `from` and
    /// `where` are never keys unquoted, since they end the projection list.
    fn word_reads_as_key(&self) -> bool {
        let TokenKind::Word(w) = self.peek_kind() else {
            return false;
        };
        if matches!(
            w.as_str(),
            "from" | "where" | "and" | "or" | "not" | "exists" | "limit"
        ) {
            return false;
        }
        matches!(
            self.tokens.get(self.pos + 1).map(|t| &t.kind),
            Some(TokenKind::Dot | TokenKind::LBracket)
        )
    }

    fn predicate(&mut self) -> Result<Predicate> {
        self.disjunction()
    }

    fn disjunction(&mut self) -> Result<Predicate> {
        let mut left = self.conjunction()?;
        while self.eat_word("or") {
            let right = self.conjunction()?;
            left = Predicate::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn conjunction(&mut self) -> Result<Predicate> {
        let mut left = self.negation()?;
        while self.eat_word("and") {
            let right = self.negation()?;
            left = Predicate::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn negation(&mut self) -> Result<Predicate> {
        if self.eat_word("not") {
            return Ok(Predicate::Not(Box::new(self.negation()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Predicate> {
        if self.eat(&TokenKind::LParen) {
            let inner = self.predicate()?;
            self.expect(&TokenKind::RParen, "`)`")?;
            return Ok(inner);
        }
        if self.eat_word("exists") {
            return Ok(Predicate::Exists(self.path()?));
        }
        if matches!(self.peek_kind(), TokenKind::End) {
            return Err(self.expected("a condition"));
        }
        let path = self.path()?;
        let negated = self.eat_word("not");
        let test = if let Some(op) = self.comparison_op() {
            if negated {
                return Err(self.error_here(
                    "`not` goes before the whole comparison, not before the operator",
                ));
            }
            self.advance();
            let value = self.scalar_literal("a value to compare against")?;
            Predicate::Compare { path, op, value }
        } else if self.eat_word("in") {
            self.expect(&TokenKind::LParen, "`(` after `in`")?;
            let mut values = vec![self.scalar_literal("a value")?];
            while self.eat(&TokenKind::Comma) {
                values.push(self.scalar_literal("a value")?);
            }
            self.expect(&TokenKind::RParen, "`)`")?;
            Predicate::In { path, values }
        } else if self.is_word("like") || self.is_word("ilike") {
            let case_insensitive = self.is_word("ilike");
            self.advance();
            let TokenKind::Str(pattern) = self.peek_kind().clone() else {
                return Err(self.expected("a quoted pattern"));
            };
            self.advance();
            Predicate::Like {
                path,
                pattern,
                case_insensitive,
            }
        } else if self.eat_word("contains") {
            let value = self.scalar_literal("a value after `contains`")?;
            Predicate::Contains { path, value }
        } else if self.is_word("exists") {
            return Err(self.error_here("`exists` goes before the path: `exists tags`"));
        } else if self.is_word("is") {
            return Err(self.error_here(
                "there is no `is null`: TOML has no null, so a key is present or absent; use `exists`",
            ));
        } else {
            return Err(self.expected("a comparison, `in`, `like`, `ilike`, or `contains`"));
        };
        Ok(if negated {
            Predicate::Not(Box::new(test))
        } else {
            test
        })
    }

    fn comparison_op(&self) -> Option<Op> {
        Some(match self.peek_kind() {
            TokenKind::Eq => Op::Eq,
            TokenKind::Ne => Op::Ne,
            TokenKind::Lt => Op::Lt,
            TokenKind::Gt => Op::Gt,
            TokenKind::Le => Op::Le,
            TokenKind::Ge => Op::Ge,
            _ => return None,
        })
    }

    /// A scalar literal in TOML spelling.
    fn scalar_literal(&mut self, what: &str) -> Result<Value> {
        let value = match self.peek_kind().clone() {
            TokenKind::Str(s) => Value::String(s),
            TokenKind::Integer(i) => Value::Integer(i),
            TokenKind::Float(x) => Value::Float(x),
            TokenKind::Datetime(d) => Value::Datetime(d),
            TokenKind::Word(w) => match w.as_str() {
                "true" => Value::Boolean(true),
                "false" => Value::Boolean(false),
                "inf" => Value::Float(f64::INFINITY),
                "nan" => Value::Float(f64::NAN),
                _ => {
                    return Err(self.error_here(format!(
                        "expected {what}, found `{w}`; a string literal needs quotes",
                    )))
                }
            },
            TokenKind::LBracket => {
                return Err(
                    self.error_here("array literals are not supported; use `contains` or `in`")
                )
            }
            _ => return Err(self.expected(what)),
        };
        self.advance();
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> Path {
        Path::Keys(
            names
                .iter()
                .map(|n| Segment::Key((*n).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn full_shape() {
        let q = parse("select title, owner.name as who from './docs' recursive where status = \"final\" limit 5").unwrap();
        assert_eq!(
            q.select,
            Select::Columns(vec![
                Projection {
                    path: keys(&["title"]),
                    alias: None
                },
                Projection {
                    path: keys(&["owner", "name"]),
                    alias: Some("who".into())
                },
            ])
        );
        assert_eq!(
            q.from,
            Source {
                root: "./docs".into(),
                recursive: true
            }
        );
        assert_eq!(
            q.filter,
            Some(Predicate::Compare {
                path: keys(&["status"]),
                op: Op::Eq,
                value: Value::String("final".into())
            })
        );
        assert_eq!(q.limit, Some(5));
    }

    #[test]
    fn star_and_default_source() {
        let src = Source {
            root: ".".into(),
            recursive: false,
        };
        let q = parse_with("select *", Some(&src)).unwrap();
        assert_eq!(q.select, Select::All);
        assert_eq!(q.from, src);
        let q = parse_with("select * from 'x'", Some(&src)).unwrap();
        assert_eq!(q.from.root, PathBuf::from("x"));
        assert!(parse("select *").is_err());
    }

    #[test]
    fn precedence() {
        let q = parse("select * from '.' where not a = 1 and b = 2 or c = 3").unwrap();
        let Some(Predicate::Or(left, right)) = q.filter else {
            panic!()
        };
        assert!(matches!(*right, Predicate::Compare { .. }));
        let Predicate::And(l, _) = *left else {
            panic!()
        };
        assert!(matches!(*l, Predicate::Not(_)));
        let q = parse("select * from '.' where a = 1 and (b = 2 or c = 3)").unwrap();
        assert!(matches!(q.filter, Some(Predicate::And(_, _))));
    }

    #[test]
    fn predicate_forms() {
        let f = |s: &str| {
            parse(&format!("select * from '.' where {s}"))
                .unwrap()
                .filter
                .unwrap()
        };
        assert_eq!(f("exists a.b"), Predicate::Exists(keys(&["a", "b"])));
        assert_eq!(
            f("not exists a"),
            Predicate::Not(Box::new(Predicate::Exists(keys(&["a"]))))
        );
        assert_eq!(
            f("tags contains \"x\""),
            Predicate::Contains {
                path: keys(&["tags"]),
                value: Value::String("x".into())
            }
        );
        assert_eq!(
            f("n in (1, 2)"),
            Predicate::In {
                path: keys(&["n"]),
                values: vec![Value::Integer(1), Value::Integer(2)]
            }
        );
        assert_eq!(
            f("n not in (1)"),
            Predicate::Not(Box::new(Predicate::In {
                path: keys(&["n"]),
                values: vec![Value::Integer(1)]
            }))
        );
        assert_eq!(
            f("t ilike '%a%'"),
            Predicate::Like {
                path: keys(&["t"]),
                pattern: "%a%".into(),
                case_insensitive: true
            }
        );
        assert!(matches!(f("t not like 'a'"), Predicate::Not(_)));
        assert_eq!(
            f("@path like '%.pdf.slpc'"),
            Predicate::Like {
                path: Path::ContainerPath,
                pattern: "%.pdf.slpc".into(),
                case_insensitive: false
            }
        );
        assert_eq!(
            f("created >= 2026-01-01"),
            Predicate::Compare {
                path: keys(&["created"]),
                op: Op::Ge,
                value: Value::Datetime("2026-01-01".parse().unwrap())
            }
        );
        assert_eq!(
            f("ok = true"),
            Predicate::Compare {
                path: keys(&["ok"]),
                op: Op::Eq,
                value: Value::Boolean(true)
            }
        );
    }

    #[test]
    fn paths() {
        let q = parse("select tags[0], \"my key\".sub, a.\"b.c\" from '.'").unwrap();
        let Select::Columns(cols) = q.select else {
            panic!()
        };
        assert_eq!(
            cols[0].path,
            Path::Keys(vec![Segment::Key("tags".into()), Segment::Index(0)])
        );
        assert_eq!(cols[0].column(), "tags[0]");
        assert_eq!(cols[1].column(), "\"my key\".sub");
        assert_eq!(cols[2].column(), "a.\"b.c\"");
        let q = parse("select in.x, 2024 from '.'").unwrap();
        let Select::Columns(cols) = q.select else {
            panic!()
        };
        assert_eq!(cols[0].column(), "in.x");
        assert_eq!(cols[1].column(), "2024");
    }

    #[test]
    fn helpful_errors() {
        let msg = |s: &str| parse(s).unwrap_err().to_string();
        assert!(
            msg("SELECT * FROM '.'").contains("keywords are lowercase"),
            "{}",
            msg("SELECT * FROM '.'")
        );
        assert!(msg("select * from '.' where a is null").contains("no `is null`"));
        assert!(msg("select * from '.' where a = b").contains("needs quotes"));
        assert!(msg("select @size from '.'").contains("@path"));
        assert!(msg("select * from '.' where a exists").contains("before the path"));
        assert!(msg("select * from '.' where in = 1").contains("keyword"));
        assert!(msg("select * from '.' where a = [1]").contains("array literals"));
        assert!(msg("select * from '.' where a not = 1").contains("`not` goes before"));
        assert!(msg("select * from '.' where").contains("condition"));
        assert!(msg("select * from '.' extra").contains("`where`, `limit`, end of query"));
        let src = Source {
            root: ".".into(),
            recursive: false,
        };
        let bound = parse_with("select * frm '.'", Some(&src))
            .unwrap_err()
            .to_string();
        assert!(
            bound.contains("`from`, `where`, `limit`, end of query"),
            "{bound}"
        );
        let after_where = parse("select * from '.' where a = 1 b")
            .unwrap_err()
            .to_string();
        assert!(
            after_where.contains("expected `limit`, end of query"),
            "{after_where}"
        );
        assert!(msg("select * from . ").contains("quoted directory"));
        assert!(msg("select * from '.' limit -1").contains("non-negative"));
    }
}
