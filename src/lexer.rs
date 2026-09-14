//! Query text to tokens.
//!
//! Literals are TOML's: strings in double or single quotes, integers with
//! underscores and radix prefixes, floats, `true`, `false`, and the four
//! datetime forms written as TOML writes them. Keywords are ordinary
//! identifiers here; the parser decides what a word means.

use slpc::toml_edit::Datetime;

use crate::error::{Error, Result};

/// A token and where it starts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub offset: usize,
    /// Bytes of query text the token spans.
    pub len: usize,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenKind {
    /// A bare word: a keyword, a bare key, `true`, `false`, `inf`, `nan`.
    Word(String),
    /// A quoted string, escapes already applied.
    Str(String),
    Integer(i64),
    Float(f64),
    Datetime(Datetime),
    /// `@name`
    At(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Star,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// End of input. Always the last token.
    End,
}

impl TokenKind {
    /// How the token reads in an error message.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Word(w) => format!("`{w}`"),
            Self::Str(s) => format!("string {s:?}"),
            Self::Integer(i) => format!("integer {i}"),
            Self::Float(x) => format!("float {x}"),
            Self::Datetime(d) => format!("datetime {d}"),
            Self::At(name) => format!("`@{name}`"),
            Self::LParen => "`(`".into(),
            Self::RParen => "`)`".into(),
            Self::LBracket => "`[`".into(),
            Self::RBracket => "`]`".into(),
            Self::Comma => "`,`".into(),
            Self::Dot => "`.`".into(),
            Self::Star => "`*`".into(),
            Self::Eq => "`=`".into(),
            Self::Ne => "`!=`".into(),
            Self::Lt => "`<`".into(),
            Self::Gt => "`>`".into(),
            Self::Le => "`<=`".into(),
            Self::Ge => "`>=`".into(),
            Self::End => "end of query".into(),
        }
    }
}

/// Tokenize a whole query. The last token is always [`TokenKind::End`].
pub(crate) fn tokenize(text: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer {
        text,
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let done = token.kind == TokenKind::End;
        tokens.push(token);
        if done {
            return Ok(tokens);
        }
    }
}

struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn skip_space_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'#') => self.skip_line(),
                Some(b'-') if self.peek_at(1) == Some(b'-') => self.skip_line(),
                _ => return,
            }
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'\n' {
                return;
            }
        }
    }

    fn next_token(&mut self) -> Result<Token> {
        self.skip_space_and_comments();
        let offset = self.pos;
        let Some(c) = self.peek() else {
            return Ok(Token {
                offset,
                len: 0,
                kind: TokenKind::End,
            });
        };
        let kind = match c {
            b'(' => self.single(TokenKind::LParen),
            b')' => self.single(TokenKind::RParen),
            b'[' => self.single(TokenKind::LBracket),
            b']' => self.single(TokenKind::RBracket),
            b',' => self.single(TokenKind::Comma),
            b'.' => self.single(TokenKind::Dot),
            b'*' => self.single(TokenKind::Star),
            b'=' => self.single(TokenKind::Eq),
            b'!' if self.peek_at(1) == Some(b'=') => self.double(TokenKind::Ne),
            b'<' if self.peek_at(1) == Some(b'=') => self.double(TokenKind::Le),
            b'>' if self.peek_at(1) == Some(b'=') => self.double(TokenKind::Ge),
            b'<' => self.single(TokenKind::Lt),
            b'>' => self.single(TokenKind::Gt),
            b'"' => self.basic_string()?,
            b'\'' => self.literal_string()?,
            b'@' => self.at()?,
            b'0'..=b'9' => self.number_or_datetime()?,
            b'+' | b'-' if matches!(self.peek_at(1), Some(b'0'..=b'9' | b'i' | b'n')) => {
                self.number_or_datetime()?
            }
            c if is_word_byte(c) => self.word(),
            _ => {
                let ch = self.text[self.pos..].chars().next().unwrap_or('?');
                return Err(Error::parse(offset, format!("unexpected character {ch:?}")));
            }
        };
        Ok(Token {
            offset,
            len: self.pos - offset,
            kind,
        })
    }

    fn single(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    fn double(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 2;
        kind
    }

    fn word(&mut self) -> TokenKind {
        let start = self.pos;
        while self.peek().is_some_and(is_word_byte) {
            self.pos += 1;
        }
        TokenKind::Word(self.text[start..self.pos].to_owned())
    }

    fn at(&mut self) -> Result<TokenKind> {
        let offset = self.pos;
        self.pos += 1;
        match self.word() {
            TokenKind::Word(name) if !name.is_empty() => Ok(TokenKind::At(name)),
            _ => Err(Error::parse(
                offset,
                "`@` must be followed by a name, as in `@path`",
            )),
        }
    }

    /// A basic string: `"..."` with TOML's escapes. Multi-line strings are
    /// not accepted; a query is one line.
    fn basic_string(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.text[self.pos..].chars().next() else {
                return Err(Error::parse(start, "unterminated string"));
            };
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(TokenKind::Str(out)),
                '\n' => return Err(Error::parse(start, "unterminated string")),
                '\\' => {
                    let escape_at = self.pos - 1;
                    let Some(e) = self.text[self.pos..].chars().next() else {
                        return Err(Error::parse(start, "unterminated string"));
                    };
                    self.pos += e.len_utf8();
                    match e {
                        'b' => out.push('\u{8}'),
                        't' => out.push('\t'),
                        'n' => out.push('\n'),
                        'f' => out.push('\u{c}'),
                        'r' => out.push('\r'),
                        'e' => out.push('\u{1b}'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'u' => out.push(self.unicode_escape(escape_at, 4)?),
                        'U' => out.push(self.unicode_escape(escape_at, 8)?),
                        other => {
                            return Err(Error::parse(
                                escape_at,
                                format!("unknown escape `\\{other}` in string"),
                            ))
                        }
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn unicode_escape(&mut self, escape_at: usize, digits: usize) -> Result<char> {
        let hex = self
            .text
            .get(self.pos..self.pos + digits)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some(hex) = hex else {
            return Err(Error::parse(
                escape_at,
                format!(
                    "`\\{}` needs {digits} hex digits",
                    if digits == 4 { 'u' } else { 'U' }
                ),
            ));
        };
        self.pos += digits;
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| Error::parse(escape_at, "escape is not a unicode scalar value"))
    }

    /// A literal string: `'...'`, taken verbatim.
    fn literal_string(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        self.pos += 1;
        let rest = &self.text[self.pos..];
        match rest.find(['\'', '\n']) {
            Some(i) if rest.as_bytes()[i] == b'\'' => {
                let s = rest[..i].to_owned();
                self.pos += i + 1;
                Ok(TokenKind::Str(s))
            }
            _ => Err(Error::parse(start, "unterminated string")),
        }
    }

    /// Anything starting with a digit or a sign: a datetime if it has the
    /// shape of one, otherwise a number.
    fn number_or_datetime(&mut self) -> Result<TokenKind> {
        if let Some(len) = self.datetime_len() {
            let start = self.pos;
            let s = &self.text[start..start + len];
            self.pos += len;
            return s
                .parse::<Datetime>()
                .map(TokenKind::Datetime)
                .map_err(|e| Error::parse(start, format!("bad datetime `{s}`: {e}")));
        }
        self.number()
    }

    /// The length of a TOML datetime starting here, if there is one.
    ///
    /// The lexer decides the extent and `toml_datetime` decides validity; a
    /// date may be followed by a time after `T`, `t`, or a single space, and a
    /// time by a fraction and an offset.
    fn datetime_len(&self) -> Option<usize> {
        let b = &self.bytes[self.pos..];
        let digits = |from: usize, n: usize| {
            b.len() >= from + n && b[from..from + n].iter().all(u8::is_ascii_digit)
        };
        let time_len = |from: usize| -> Option<usize> {
            if !(digits(from, 2) && b.get(from + 2) == Some(&b':') && digits(from + 3, 2)) {
                return None;
            }
            let mut end = from + 5;
            if b.get(end) == Some(&b':') && digits(end + 1, 2) {
                end += 3;
                if b.get(end) == Some(&b'.') {
                    let mut e = end + 1;
                    while e < b.len() && b[e].is_ascii_digit() {
                        e += 1;
                    }
                    if e > end + 1 {
                        end = e;
                    }
                }
            }
            Some(end)
        };
        if digits(0, 4)
            && b.get(4) == Some(&b'-')
            && digits(5, 2)
            && b.get(7) == Some(&b'-')
            && digits(8, 2)
        {
            let mut end = 10;
            let joiner = b.get(10).copied();
            if matches!(joiner, Some(b'T' | b't'))
                || (joiner == Some(b' ') && digits(11, 2) && b.get(13) == Some(&b':'))
            {
                if let Some(t) = time_len(11) {
                    end = t;
                    match b.get(end) {
                        Some(b'Z' | b'z') => end += 1,
                        Some(b'+' | b'-')
                            if digits(end + 1, 2)
                                && b.get(end + 3) == Some(&b':')
                                && digits(end + 4, 2) =>
                        {
                            end += 6;
                        }
                        _ => {}
                    }
                }
            }
            return Some(end);
        }
        time_len(0)
    }

    /// A TOML integer or float.
    fn number(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        let mut end = start;
        let b = self.bytes;
        if matches!(b.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        while end < b.len() {
            let c = b[end];
            let continues = c.is_ascii_alphanumeric()
                || c == b'_'
                || c == b'.'
                || (matches!(c, b'+' | b'-')
                    && matches!(b[end - 1], b'e' | b'E')
                    && !self.text[start..end].starts_with("0x"));
            if !continues {
                break;
            }
            end += 1;
        }
        let raw = &self.text[start..end];
        self.pos = end;
        parse_number(raw).ok_or_else(|| Error::parse(start, format!("bad number `{raw}`")))
    }
}

/// TOML's bare-key alphabet, which is also the keyword alphabet.
fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

/// Parse TOML's integer and float spellings.
fn parse_number(raw: &str) -> Option<TokenKind> {
    let (negative, body) = match raw.as_bytes().first() {
        Some(b'-') => (true, &raw[1..]),
        Some(b'+') => (false, &raw[1..]),
        _ => (false, raw),
    };
    let sign = if negative { -1.0 } else { 1.0 };
    match body {
        "inf" => return Some(TokenKind::Float(sign * f64::INFINITY)),
        "nan" => return Some(TokenKind::Float(f64::NAN)),
        _ => {}
    }
    if body.starts_with('_') || body.ends_with('_') || body.contains("__") {
        return None;
    }
    let clean: String = body.chars().filter(|&c| c != '_').collect();
    let radix = |prefix: &str, radix: u32| {
        if negative {
            return None;
        }
        i64::from_str_radix(clean.strip_prefix(prefix)?, radix).ok()
    };
    if let Some(i) = radix("0x", 16)
        .or_else(|| radix("0o", 8))
        .or_else(|| radix("0b", 2))
    {
        return Some(TokenKind::Integer(i));
    }
    if clean.bytes().all(|c| c.is_ascii_digit()) {
        if clean.len() > 1 && clean.starts_with('0') {
            return None;
        }
        let magnitude: i64 = clean.parse().ok()?;
        return Some(TokenKind::Integer(if negative {
            -magnitude
        } else {
            magnitude
        }));
    }
    if clean.contains(['.', 'e', 'E']) && !clean.starts_with('.') && !clean.ends_with('.') {
        let x: f64 = clean.parse().ok()?;
        return Some(TokenKind::Float(if negative { -x } else { x }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<TokenKind> {
        tokenize(text)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn words_and_punctuation() {
        assert_eq!(
            kinds("select a.b[0], * from"),
            vec![
                TokenKind::Word("select".into()),
                TokenKind::Word("a".into()),
                TokenKind::Dot,
                TokenKind::Word("b".into()),
                TokenKind::LBracket,
                TokenKind::Integer(0),
                TokenKind::RBracket,
                TokenKind::Comma,
                TokenKind::Star,
                TokenKind::Word("from".into()),
                TokenKind::End,
            ]
        );
        assert_eq!(kinds("a != b <= c >= d < e > f = g")[1], TokenKind::Ne);
        assert_eq!(
            kinds("@path"),
            vec![TokenKind::At("path".into()), TokenKind::End]
        );
    }

    #[test]
    fn strings() {
        assert_eq!(
            kinds(r#""a\"b\n""#),
            vec![TokenKind::Str("a\"b\n".into()), TokenKind::End]
        );
        assert_eq!(
            kinds(r"'C:\x'"),
            vec![TokenKind::Str(r"C:\x".into()), TokenKind::End]
        );
        assert_eq!(
            kinds(r#""\u00e9""#),
            vec![TokenKind::Str("é".into()), TokenKind::End]
        );
        assert!(tokenize("\"open").is_err());
        assert!(tokenize(r#""\q""#).is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(kinds("1_000")[0], TokenKind::Integer(1000));
        assert_eq!(kinds("-7")[0], TokenKind::Integer(-7));
        assert_eq!(kinds("+7")[0], TokenKind::Integer(7));
        assert_eq!(kinds("0xff")[0], TokenKind::Integer(255));
        assert_eq!(kinds("0o17")[0], TokenKind::Integer(15));
        assert_eq!(kinds("0b101")[0], TokenKind::Integer(5));
        assert_eq!(kinds("1.5")[0], TokenKind::Float(1.5));
        assert_eq!(kinds("1e3")[0], TokenKind::Float(1000.0));
        assert_eq!(kinds("6.02e-23")[0], TokenKind::Float(6.02e-23));
        assert_eq!(kinds("-inf")[0], TokenKind::Float(f64::NEG_INFINITY));
        assert!(matches!(kinds("nan")[0], TokenKind::Word(_)));
        assert!(tokenize("01").is_err());
        assert!(tokenize("1__0").is_err());
        assert!(tokenize("1.").is_err());
    }

    #[test]
    fn datetimes() {
        let d = |s: &str| TokenKind::Datetime(s.parse().unwrap());
        assert_eq!(kinds("2026-01-02")[0], d("2026-01-02"));
        assert_eq!(kinds("2026-01-02T03:04:05Z")[0], d("2026-01-02T03:04:05Z"));
        assert_eq!(
            kinds("2026-01-02 03:04:05.5+02:00")[0],
            d("2026-01-02T03:04:05.5+02:00")
        );
        assert_eq!(kinds("2026-01-02t03:04:05")[0], d("2026-01-02T03:04:05"));
        assert_eq!(kinds("03:04:05")[0], d("03:04:05"));
        assert_eq!(kinds("03:04")[0], d("03:04"));
        // A date followed by a space and a non-time is two tokens.
        assert_eq!(kinds("2026-01-02 and").len(), 3);
        assert!(tokenize("2026-13-01").is_err());
    }

    #[test]
    fn comments() {
        assert_eq!(kinds("a -- comment\n b # another\n"), kinds("a b"));
        assert_eq!(kinds("x = -1")[2], TokenKind::Integer(-1));
    }

    #[test]
    fn unexpected_character() {
        let err = tokenize("a ; b").unwrap_err();
        assert!(err.to_string().contains("offset 2"), "{err}");
    }
}
