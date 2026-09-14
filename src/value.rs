//! Values as a query sees them: TOML's types, owned, and comparable.

use std::cmp::Ordering;
use std::fmt;

use slpc::toml_edit::{self, Datetime, Item, Offset};

/// A value read from metadata or written in a query.
///
/// The variants are TOML's. A datetime keeps TOML's own type, which carries
/// all four calendar forms; [`Value::kind`] tells them apart.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A string.
    String(String),
    /// A 64-bit integer.
    Integer(i64),
    /// A 64-bit float.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// One of TOML's four datetime forms.
    Datetime(Datetime),
    /// An array, in document order.
    Array(Vec<Value>),
    /// A table, in document order.
    Table(Vec<(String, Value)>),
}

/// The type of a value, at the granularity comparison cares about.
///
/// Integer and float are distinct kinds but one comparison class; the four
/// datetime forms are four kinds and four classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// [`Value::String`]
    String,
    /// [`Value::Integer`]
    Integer,
    /// [`Value::Float`]
    Float,
    /// [`Value::Boolean`]
    Boolean,
    /// A datetime with a date, a time, and an offset.
    OffsetDatetime,
    /// A datetime with a date and a time and no offset.
    LocalDatetime,
    /// A date alone.
    LocalDate,
    /// A time alone.
    LocalTime,
    /// [`Value::Array`]
    Array,
    /// [`Value::Table`]
    Table,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::Boolean => "boolean",
            Self::OffsetDatetime => "offset datetime",
            Self::LocalDatetime => "local datetime",
            Self::LocalDate => "local date",
            Self::LocalTime => "local time",
            Self::Array => "array",
            Self::Table => "table",
        })
    }
}

impl Kind {
    /// Whether two kinds compare with each other at all.
    fn comparable(self, other: Kind) -> bool {
        match (self, other) {
            (Self::Integer | Self::Float, Self::Integer | Self::Float) => true,
            (a, b) => a == b,
        }
    }
}

impl Value {
    /// The value's kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::String(_) => Kind::String,
            Self::Integer(_) => Kind::Integer,
            Self::Float(_) => Kind::Float,
            Self::Boolean(_) => Kind::Boolean,
            Self::Datetime(d) => datetime_kind(d),
            Self::Array(_) => Kind::Array,
            Self::Table(_) => Kind::Table,
        }
    }

    /// Whether this is a scalar: anything but an array or a table.
    #[must_use]
    pub fn is_scalar(&self) -> bool {
        !matches!(self, Self::Array(_) | Self::Table(_))
    }

    /// Compare two values within their class.
    ///
    /// `None` when the two are of different classes, when either is a NaN, or
    /// when either is an array or a table, none of which order. Offset
    /// datetimes compare as instants, so two spellings of one moment in
    /// different offsets are equal.
    #[must_use]
    pub fn compare(&self, other: &Value) -> Option<Ordering> {
        if !self.kind().comparable(other.kind()) {
            return None;
        }
        match (self, other) {
            (Self::String(a), Self::String(b)) => Some(a.cmp(b)),
            (Self::Integer(a), Self::Integer(b)) => Some(a.cmp(b)),
            (Self::Float(a), Self::Float(b)) => a.partial_cmp(b),
            (Self::Integer(a), Self::Float(b)) => compare_int_float(*a, *b),
            (Self::Float(a), Self::Integer(b)) => compare_int_float(*b, *a).map(Ordering::reverse),
            (Self::Boolean(a), Self::Boolean(b)) => Some(a.cmp(b)),
            (Self::Datetime(a), Self::Datetime(b)) => Some(compare_datetimes(a, b)),
            _ => None,
        }
    }

    /// Build a value from a parsed TOML item, dropping formatting.
    ///
    /// `None` for [`Item::None`], which `toml_edit` uses for an absent slot.
    #[must_use]
    pub fn from_item(item: &Item) -> Option<Value> {
        match item {
            Item::None => None,
            Item::Value(v) => Some(Self::from_toml(v)),
            Item::Table(t) => Some(Self::Table(
                t.iter()
                    .filter_map(|(k, v)| Self::from_item(v).map(|v| (k.to_owned(), v)))
                    .collect(),
            )),
            Item::ArrayOfTables(a) => Some(Self::Array(
                a.iter()
                    .map(|t| {
                        Self::from_item(&Item::Table(t.clone())).unwrap_or(Self::Table(Vec::new()))
                    })
                    .collect(),
            )),
        }
    }

    /// Build a value from a TOML value, dropping formatting.
    #[must_use]
    pub fn from_toml(value: &toml_edit::Value) -> Value {
        match value {
            toml_edit::Value::String(s) => Self::String(s.value().clone()),
            toml_edit::Value::Integer(i) => Self::Integer(*i.value()),
            toml_edit::Value::Float(f) => Self::Float(*f.value()),
            toml_edit::Value::Boolean(b) => Self::Boolean(*b.value()),
            toml_edit::Value::Datetime(d) => Self::Datetime(*d.value()),
            toml_edit::Value::Array(a) => Self::Array(a.iter().map(Self::from_toml).collect()),
            toml_edit::Value::InlineTable(t) => Self::Table(
                t.iter()
                    .map(|(k, v)| (k.to_owned(), Self::from_toml(v)))
                    .collect(),
            ),
        }
    }

    /// The value as TOML would write it inline, strings included.
    ///
    /// This is the rendering for a nested value inside a text cell, where the
    /// quotes are what tells `"1"` from `1`.
    pub fn write_toml(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Self::String(s) => write_basic_string(f, s),
            Self::Integer(i) => write!(f, "{i}"),
            Self::Float(x) => write_float(f, *x),
            Self::Boolean(b) => write!(f, "{b}"),
            Self::Datetime(d) => write!(f, "{d}"),
            Self::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    item.write_toml(f)?;
                }
                f.write_str("]")
            }
            Self::Table(entries) => {
                if entries.is_empty() {
                    return f.write_str("{}");
                }
                f.write_str("{ ")?;
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    crate::ast::write_key(f, key)?;
                    f.write_str(" = ")?;
                    value.write_toml(f)?;
                }
                f.write_str(" }")
            }
        }
    }

    /// The value as text for a cell: a string bare, everything else as TOML.
    ///
    /// A bare string is what a table or CSV reader wants to see; the quotes
    /// come back only inside an array or a table, where they carry type.
    pub fn write_text(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Self::String(s) => f.write_str(s),
            other => other.write_toml(f),
        }
    }
}

impl fmt::Display for Value {
    /// [`Value::write_text`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_text(f)
    }
}

/// Which of TOML's four datetime forms this is.
fn datetime_kind(d: &Datetime) -> Kind {
    match (d.date.is_some(), d.time.is_some(), d.offset.is_some()) {
        (true, true, true) => Kind::OffsetDatetime,
        (true, true, false) => Kind::LocalDatetime,
        (true, false, _) => Kind::LocalDate,
        (false, _, _) => Kind::LocalTime,
    }
}

/// Compare an integer with a float without going through a lossy cast.
///
/// An `i64` past 2^53 does not survive `as f64`, so the float is checked
/// against the integer range and truncated instead.
fn compare_int_float(a: i64, b: f64) -> Option<Ordering> {
    if b.is_nan() {
        return None;
    }
    if b >= 9_223_372_036_854_775_808.0 {
        return Some(Ordering::Less);
    }
    if b < -9_223_372_036_854_775_808.0 {
        return Some(Ordering::Greater);
    }
    #[allow(clippy::cast_possible_truncation)]
    let whole = b.trunc() as i64;
    match a.cmp(&whole) {
        Ordering::Equal => {
            let frac = b - b.trunc();
            Some(if frac > 0.0 {
                Ordering::Less
            } else if frac < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        other => Some(other),
    }
}

/// Compare two datetimes of the same kind.
///
/// Offset datetimes compare as instants. The other three forms have no
/// offset to reconcile, and `toml_datetime`'s derived order is field by
/// field, which is the calendar order for them.
fn compare_datetimes(a: &Datetime, b: &Datetime) -> Ordering {
    match (a.date, a.time, a.offset, b.date, b.time, b.offset) {
        (Some(ad), Some(at), Some(ao), Some(bd), Some(bt), Some(bo)) => {
            instant(ad, at, ao).cmp(&instant(bd, bt, bo))
        }
        _ => a.cmp(b),
    }
}

/// Seconds since the epoch and nanoseconds within the second, at UTC.
fn instant(date: toml_edit::Date, time: toml_edit::Time, offset: Offset) -> (i64, u32) {
    let days = days_from_civil(i64::from(date.year), date.month, date.day);
    let seconds = days * 86_400
        + i64::from(time.hour) * 3_600
        + i64::from(time.minute) * 60
        + i64::from(time.second.unwrap_or(0));
    let offset_minutes = match offset {
        Offset::Z => 0,
        Offset::Custom { minutes } => i64::from(minutes),
    };
    (seconds - offset_minutes * 60, time.nanosecond.unwrap_or(0))
}

/// Days since 1970-01-01 for a proleptic Gregorian date. Howard Hinnant's
/// algorithm, which is exact over the whole range TOML can spell.
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Write a float the way TOML spells one: always with a fraction or an
/// exponent, so it cannot be mistaken for an integer when read back.
pub(crate) fn write_float(f: &mut impl fmt::Write, x: f64) -> fmt::Result {
    if x.is_nan() {
        f.write_str("nan")
    } else if x.is_infinite() {
        f.write_str(if x > 0.0 { "inf" } else { "-inf" })
    } else if x.fract() == 0.0 && x.abs() < 1e16 {
        write!(f, "{x:.1}")
    } else {
        write!(f, "{x}")
    }
}

/// Write a string as a TOML basic string, escaping what TOML requires.
pub(crate) fn write_basic_string(f: &mut impl fmt::Write, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            '\u{8}' => f.write_str("\\b")?,
            '\u{c}' => f.write_str("\\f")?,
            c if c < ' ' || c == '\u{7f}' => write!(f, "\\u{:04X}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_str("\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> Value {
        Value::Datetime(s.parse().unwrap())
    }

    #[test]
    fn integers_and_floats_share_a_class() {
        assert_eq!(
            Value::Integer(1).compare(&Value::Float(1.5)),
            Some(Ordering::Less)
        );
        assert_eq!(
            Value::Float(2.0).compare(&Value::Integer(2)),
            Some(Ordering::Equal)
        );
        assert_eq!(
            Value::Integer(3).compare(&Value::Float(2.5)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            Value::Integer(i64::MAX).compare(&Value::Float(1e300)),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn strings_do_not_compare_with_numbers() {
        assert_eq!(Value::String("1".into()).compare(&Value::Integer(1)), None);
        assert_eq!(Value::Integer(1).compare(&Value::String("1".into())), None);
    }

    #[test]
    fn nan_orders_nothing() {
        assert_eq!(Value::Float(f64::NAN).compare(&Value::Float(1.0)), None);
        assert_eq!(Value::Integer(1).compare(&Value::Float(f64::NAN)), None);
    }

    #[test]
    fn offset_datetimes_compare_as_instants() {
        let a = dt("2026-01-01T10:00:00+02:00");
        let b = dt("2026-01-01T09:00:00+01:00");
        assert_eq!(a.compare(&b), Some(Ordering::Equal));
        let c = dt("2026-01-01T08:00:00Z");
        assert_eq!(a.compare(&c), Some(Ordering::Equal));
        let later = dt("2026-01-01T08:00:01Z");
        assert_eq!(a.compare(&later), Some(Ordering::Less));
        let earlier_day = dt("2025-12-31T23:59:59-05:00");
        assert_eq!(earlier_day.compare(&c), Some(Ordering::Less));
        let same_instant = dt("2026-01-01T03:00:00-05:00");
        assert_eq!(same_instant.compare(&c), Some(Ordering::Equal));
    }

    #[test]
    fn datetime_forms_are_separate_classes() {
        assert_eq!(dt("2026-01-01").compare(&dt("2026-01-01T00:00:00")), None);
        assert_eq!(
            dt("2026-01-01T00:00:00").compare(&dt("2026-01-01T00:00:00Z")),
            None
        );
        assert_eq!(dt("10:00:00").compare(&dt("2026-01-01")), None);
        assert_eq!(
            dt("2026-01-02").compare(&dt("2026-01-01")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            dt("10:00:00").compare(&dt("09:59:59")),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2026, 9, 14), 20_710);
    }

    #[test]
    fn toml_rendering() {
        let s = |v: &Value| {
            let mut out = String::new();
            v.write_toml(&mut out).unwrap();
            out
        };
        assert_eq!(s(&Value::Float(1.0)), "1.0");
        assert_eq!(s(&Value::Float(1.5)), "1.5");
        assert_eq!(s(&Value::Float(f64::INFINITY)), "inf");
        assert_eq!(s(&Value::String("a\"b".into())), "\"a\\\"b\"");
        assert_eq!(
            s(&Value::Array(vec![
                Value::String("x".into()),
                Value::Integer(1)
            ])),
            "[\"x\", 1]"
        );
        assert_eq!(
            s(&Value::Table(vec![
                ("a".into(), Value::Boolean(true)),
                ("b c".into(), Value::Integer(2))
            ])),
            "{ a = true, \"b c\" = 2 }"
        );
        assert_eq!(s(&Value::Table(vec![])), "{}");
        assert_eq!(Value::String("plain".into()).to_string(), "plain");
    }
}
