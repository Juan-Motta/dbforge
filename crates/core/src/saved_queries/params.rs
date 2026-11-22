//! `:name` parameters in a saved query's SQL: finding them, writing their
//! values as SQL literals of the connection's dialect, and putting those
//! literals in their place. The console runs raw text with no bound
//! parameters, so values go into the SQL as literals.

use std::collections::BTreeMap;
use std::ops::Range;

use super::{ParamKind, ParamValue};
use crate::connections::model::Engine;

/// Why a parameter's value can't be written as a literal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParamError {
    #[error("Enter a value for :{0}.")]
    Missing(String),
    #[error("\"{0}\" is not a number.")]
    NotANumber(String),
    #[error("\"{0}\" is not a date (YYYY-MM-DD).")]
    NotADate(String),
    #[error("\"{0}\" is not a date and time (YYYY-MM-DD HH:MM[:SS]).")]
    NotADateTime(String),
    #[error("\"{0}\" is not true or false.")]
    NotABoolean(String),
    /// A value of parameter `name` that broke one of the rules above.
    #[error(":{name}: {error}")]
    Invalid { name: String, error: Box<ParamError> },
}

/// SQL with the parameters' values in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substituted {
    pub sql: String,
    /// The byte ranges of the inserted literals in `sql`, in order.
    pub spans: Vec<Range<usize>>,
}

/// The `:name` parameters in `sql`, each once, in order of first use. Only
/// SQL engines have them.
pub fn parameters(sql: &str, engine: Engine) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for placeholder in placeholders(sql, engine) {
        let name = &sql[placeholder.name];
        if !names.iter().any(|seen| seen == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// `value` written as a SQL literal of `engine`'s dialect.
pub fn literal(value: &ParamValue, engine: Engine) -> Result<String, ParamError> {
    let text = value.value.as_str();
    match value.kind {
        ParamKind::Text => Ok(text_literal(text, engine)),
        ParamKind::Number if is_number(text.trim()) => Ok(text.trim().to_string()),
        ParamKind::Number => Err(ParamError::NotANumber(text.into())),
        ParamKind::Date if is_date(text) => Ok(format!("'{text}'")),
        ParamKind::Date => Err(ParamError::NotADate(text.into())),
        ParamKind::DateTime if is_date_time(text) => Ok(format!("'{text}'")),
        ParamKind::DateTime => Err(ParamError::NotADateTime(text.into())),
        ParamKind::Boolean => boolean_literal(text, engine),
        ParamKind::Null => Ok("NULL".into()),
    }
}

/// `sql` with every `:name` parameter replaced by its value in `values`, as a literal.
pub fn substitute(sql: &str, engine: Engine, values: &BTreeMap<String, ParamValue>) -> Result<Substituted, ParamError> {
    let found = placeholders(sql, engine);
    let mut literals: BTreeMap<&str, String> = BTreeMap::new();
    for placeholder in &found {
        let name = &sql[placeholder.name.clone()];
        if literals.contains_key(name) {
            continue;
        }
        let value = values.get(name).ok_or_else(|| ParamError::Missing(name.into()))?;
        let written = literal(value, engine)
            .map_err(|error| ParamError::Invalid { name: name.into(), error: Box::new(error) })?;
        literals.insert(name, written);
    }
    let mut out = Substituted { sql: String::with_capacity(sql.len()), spans: Vec::with_capacity(found.len()) };
    let mut copied = 0;
    for placeholder in &found {
        out.sql.push_str(&sql[copied..placeholder.whole.start]);
        let start = out.sql.len();
        out.sql.push_str(&literals[&sql[placeholder.name.clone()]]);
        out.spans.push(start..out.sql.len());
        copied = placeholder.whole.end;
    }
    out.sql.push_str(&sql[copied..]);
    Ok(out)
}

/// Text in single quotes, with quotes doubled; MySQL and MariaDB also read
/// backslashes as escapes, and SQL Server takes Unicode text as `N'…'`.
fn text_literal(text: &str, engine: Engine) -> String {
    let mut escaped = text.replace('\'', "''");
    if matches!(engine, Engine::Mysql | Engine::Mariadb) {
        escaped = escaped.replace('\\', "\\\\");
    }
    match engine {
        Engine::Sqlserver => format!("N'{escaped}'"),
        _ => format!("'{escaped}'"),
    }
}

/// `true`/`false`, or `1`/`0` where the dialect has no boolean literal.
fn boolean_literal(text: &str, engine: Engine) -> Result<String, ParamError> {
    let value = match text.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => return Err(ParamError::NotABoolean(text.into())),
    };
    let written = match (engine, value) {
        (Engine::Sqlserver | Engine::Sqlite, true) => "1",
        (Engine::Sqlserver | Engine::Sqlite, false) => "0",
        (_, true) => "true",
        (_, false) => "false",
    };
    Ok(written.into())
}

/// A decimal number as SQL writes one: `-?(digits[.digits]|.digits)[e[±]digits]`, finite.
fn is_number(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(at) => (&unsigned[..at], Some(&unsigned[at + 1..])),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok = all_digits(whole) && all_digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !digits.is_empty() && all_digits(digits)
    });
    mantissa_ok && exponent_ok && text.parse::<f64>().is_ok_and(f64::is_finite)
}

fn all_digits(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_digit())
}

/// The digits of `text` as a number when it is exactly `width` ASCII digits.
fn digits(text: &str, width: usize) -> Option<u32> {
    (text.len() == width && all_digits(text)).then(|| text.parse().ok()).flatten()
}

/// A real calendar day written `YYYY-MM-DD`.
fn is_date(text: &str) -> bool {
    let mut parts = text.split('-');
    let (Some(year), Some(month), Some(day), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    match (digits(year, 4), digits(month, 2), digits(day, 2)) {
        (Some(year), Some(month), Some(day)) => chrono::NaiveDate::from_ymd_opt(year as i32, month, day).is_some(),
        _ => false,
    }
}

/// A real day and time written `YYYY-MM-DD HH:MM[:SS[.fff]]`.
fn is_date_time(text: &str) -> bool {
    let Some((date, time)) = text.split_once(' ') else { return false };
    is_date(date) && is_time(time)
}

/// `HH:MM[:SS[.fff]]`, with up to nine digits of fraction.
fn is_time(text: &str) -> bool {
    let (clock, fraction) = match text.split_once('.') {
        Some((clock, fraction)) => (clock, Some(fraction)),
        None => (text, None),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let fraction_ok = fraction.is_none_or(|digits| (1..=9).contains(&digits.len()) && all_digits(digits));
    let numbers: Option<Vec<u32>> = parts.iter().map(|part| digits(part, 2)).collect();
    match (numbers.as_deref(), fraction) {
        (Some([hour, minute]), None) => *hour < 24 && *minute < 60,
        (Some([hour, minute, second]), _) => fraction_ok && *hour < 24 && *minute < 60 && *second < 60,
        _ => false,
    }
}

/// A `:name` placeholder found in SQL.
struct Placeholder {
    /// The bytes of `:name`.
    whole: Range<usize>,
    /// The bytes of `name`.
    name: Range<usize>,
}

/// What a dialect quotes and comments with, which hides placeholders.
#[derive(Clone, Copy, Default)]
struct Dialect {
    /// Backslashes escape the next character in quoted text (MySQL, MariaDB).
    backslash_escapes: bool,
    /// `E'…'` strings take backslash escapes (PostgreSQL).
    escape_strings: bool,
    /// `` `name` `` quotes an identifier.
    backticks: bool,
    /// `[name]` quotes an identifier.
    brackets: bool,
    /// `#` starts a comment to the end of the line.
    hash_comments: bool,
    /// `--` starts a comment only when whitespace follows (MySQL, MariaDB).
    dash_comments_need_space: bool,
    /// `/* */` comments nest.
    nested_comments: bool,
    /// `$tag$…$tag$` quotes a body.
    dollar_quotes: bool,
}

impl Dialect {
    /// The dialect of a SQL engine; `None` for engines without SQL.
    fn of(engine: Engine) -> Option<Self> {
        let dialect = match engine {
            Engine::Postgres => {
                Dialect { escape_strings: true, nested_comments: true, dollar_quotes: true, ..Dialect::default() }
            }
            Engine::Mysql | Engine::Mariadb => Dialect {
                backslash_escapes: true,
                backticks: true,
                hash_comments: true,
                dash_comments_need_space: true,
                ..Dialect::default()
            },
            Engine::Sqlite => Dialect { backticks: true, brackets: true, ..Dialect::default() },
            Engine::Sqlserver => Dialect { brackets: true, ..Dialect::default() },
            Engine::Mongodb | Engine::Redis | Engine::Cosmosdb => return None,
        };
        Some(dialect)
    }

    /// The end of the string, quoted identifier, comment, dollar-quoted body
    /// or `::` cast that starts at `at`, or `None` when none starts there.
    fn skip(self, sql: &[u8], at: usize) -> Option<usize> {
        let next = sql.get(at + 1).copied();
        match sql[at] {
            b'\'' => Some(enclosed(sql, at + 1, b'\'', self.backslash_escapes || self.is_escape_string(sql, at))),
            b'"' => Some(enclosed(sql, at + 1, b'"', self.backslash_escapes)),
            b'`' if self.backticks => Some(enclosed(sql, at + 1, b'`', false)),
            b'[' if self.brackets => Some(enclosed(sql, at + 1, b']', false)),
            b'-' if next == Some(b'-') && self.starts_dash_comment(sql, at) => Some(line_end(sql, at)),
            b'#' if self.hash_comments => Some(line_end(sql, at)),
            b'/' if next == Some(b'*') => Some(self.block_comment_end(sql, at)),
            b'$' if self.dollar_quotes => dollar_body_end(sql, at),
            b':' if next == Some(b':') => Some(at + 2),
            _ => None,
        }
    }

    /// Whether the quote at `at` opens a PostgreSQL `E'…'` string.
    fn is_escape_string(self, sql: &[u8], at: usize) -> bool {
        self.escape_strings
            && at > 0
            && matches!(sql[at - 1], b'E' | b'e')
            && (at < 2 || !is_identifier_byte(sql[at - 2]))
    }

    fn starts_dash_comment(self, sql: &[u8], at: usize) -> bool {
        !self.dash_comments_need_space || sql.get(at + 2).is_none_or(|byte| byte.is_ascii_whitespace())
    }

    /// The end of the `/* */` comment starting at `at`, nested ones included
    /// where the dialect nests them; the end of the SQL when it never closes.
    fn block_comment_end(self, sql: &[u8], at: usize) -> usize {
        let mut depth = 0;
        let mut index = at;
        while index + 1 < sql.len() {
            match (sql[index], sql[index + 1]) {
                (b'/', b'*') if depth == 0 || self.nested_comments => {
                    depth += 1;
                    index += 2;
                }
                (b'*', b'/') => {
                    depth -= 1;
                    index += 2;
                    if depth == 0 {
                        return index;
                    }
                }
                _ => index += 1,
            }
        }
        sql.len()
    }
}

/// Every `:name` placeholder in `sql` outside strings, quoted identifiers,
/// comments, dollar-quoted bodies and casts, in order.
fn placeholders(sql: &str, engine: Engine) -> Vec<Placeholder> {
    let Some(dialect) = Dialect::of(engine) else { return Vec::new() };
    let bytes = sql.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = dialect.skip(bytes, index) {
            index = end;
        } else if let Some(placeholder) = placeholder_at(bytes, index) {
            index = placeholder.whole.end;
            found.push(placeholder);
        } else {
            index += 1;
        }
    }
    found
}

/// The `:name` placeholder starting at `at`, if one does.
fn placeholder_at(sql: &[u8], at: usize) -> Option<Placeholder> {
    if sql[at] != b':' || !sql.get(at + 1).is_some_and(|byte| is_identifier_start(*byte)) {
        return None;
    }
    let end = identifier_end(sql, at + 1);
    Some(Placeholder { whole: at..end, name: at + 1..end })
}

/// The end of quoted text whose content starts at `from` and closes with
/// `close`, a doubled `close` standing for itself; the end of the SQL when it
/// never closes.
fn enclosed(sql: &[u8], from: usize, close: u8, backslash_escapes: bool) -> usize {
    let mut index = from;
    while index < sql.len() {
        match sql[index] {
            b'\\' if backslash_escapes => index += 2,
            byte if byte == close && sql.get(index + 1) == Some(&close) => index += 2,
            byte if byte == close => return index + 1,
            _ => index += 1,
        }
    }
    sql.len()
}

/// Just past the end of the line `at` is on.
fn line_end(sql: &[u8], at: usize) -> usize {
    sql[at..].iter().position(|byte| *byte == b'\n').map_or(sql.len(), |offset| at + offset + 1)
}

/// The end of the `$tag$…$tag$` body starting at `at`, or `None` when the
/// `$` starts none (a `$1` parameter, or part of an identifier).
fn dollar_body_end(sql: &[u8], at: usize) -> Option<usize> {
    if at > 0 && is_identifier_byte(sql[at - 1]) {
        return None;
    }
    let tag_end = match sql.get(at + 1) {
        Some(byte) if is_identifier_start(*byte) => identifier_end(sql, at + 1),
        _ => at + 1,
    };
    if sql.get(tag_end) != Some(&b'$') {
        return None;
    }
    let delimiter = &sql[at..=tag_end];
    let body = tag_end + 1;
    let close = sql[body..].windows(delimiter.len()).position(|window| window == delimiter);
    Some(close.map_or(sql.len(), |offset| body + offset + delimiter.len()))
}

/// The end of the `[A-Za-z0-9_]*` name starting at `from`: a placeholder's or a dollar quote's tag.
fn identifier_end(sql: &[u8], from: usize) -> usize {
    sql[from..].iter().position(|byte| !is_name_byte(*byte)).map_or(sql.len(), |offset| from + offset)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A byte an unquoted identifier may hold, `$` included as PostgreSQL allows.
fn is_identifier_byte(byte: u8) -> bool {
    is_name_byte(byte) || byte == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(sql: &str, engine: Engine) -> Vec<String> {
        parameters(sql, engine)
    }

    fn value(kind: ParamKind, value: &str) -> ParamValue {
        ParamValue { kind, value: value.into() }
    }

    const SQL_ENGINES: [Engine; 5] =
        [Engine::Postgres, Engine::Mysql, Engine::Mariadb, Engine::Sqlite, Engine::Sqlserver];

    #[test]
    fn a_name_stops_at_characters_it_cannot_hold() {
        assert_eq!(params("select :a$b, :c-1, :d.e", Engine::Postgres), ["a", "c", "d"]);
    }

    #[test]
    fn finds_each_parameter_once_in_order_of_first_use() {
        for engine in SQL_ENGINES {
            assert_eq!(
                params("select * from t where b = :beta and a = :alpha_1 or b = :beta and c=:_c", engine),
                ["beta", "alpha_1", "_c"],
                "{engine:?}"
            );
        }
    }

    #[test]
    fn engines_without_sql_have_no_parameters() {
        for engine in [Engine::Mongodb, Engine::Redis, Engine::Cosmosdb] {
            assert!(params("db.orders.find({a: :x})", engine).is_empty());
        }
    }

    #[test]
    fn a_colon_without_a_name_is_not_a_parameter() {
        assert!(params("select :1, : x, a:=2", Engine::Mysql).is_empty());
    }

    #[test]
    fn skips_single_quoted_strings_with_doubled_quotes() {
        for engine in SQL_ENGINES {
            assert_eq!(params("select ':no', 'it''s :no' , :yes", engine), ["yes"], "{engine:?}");
        }
    }

    #[test]
    fn mysql_strings_take_backslash_escapes() {
        for engine in [Engine::Mysql, Engine::Mariadb] {
            assert_eq!(params(r"select 'a\' :no', :yes", engine), ["yes"]);
            assert_eq!(params(r#"select "a\" :no", :yes"#, engine), ["yes"]);
        }
        // Elsewhere a backslash is an ordinary character: the string ends at the quote.
        assert_eq!(params(r"select 'a\', :yes", Engine::Postgres), ["yes"]);
    }

    #[test]
    fn postgres_escape_strings_take_backslash_escapes() {
        assert_eq!(params(r"select E'a\' :no', e'\\', :yes", Engine::Postgres), ["yes"]);
    }

    #[test]
    fn skips_double_quoted_identifiers() {
        for engine in SQL_ENGINES {
            assert_eq!(params(r#"select "a "" :no" from t where x = :yes"#, engine), ["yes"], "{engine:?}");
        }
    }

    #[test]
    fn skips_backticks_where_the_dialect_quotes_with_them() {
        for engine in [Engine::Mysql, Engine::Mariadb, Engine::Sqlite] {
            assert_eq!(params("select `a `` :no` from t where x = :yes", engine), ["yes"], "{engine:?}");
        }
        assert_eq!(params("select `:a`", Engine::Postgres), ["a"]);
    }

    #[test]
    fn skips_brackets_where_the_dialect_quotes_with_them() {
        for engine in [Engine::Sqlserver, Engine::Sqlite] {
            assert_eq!(params("select [a ]] :no] from t where x = :yes", engine), ["yes"], "{engine:?}");
        }
        assert_eq!(params("select a[1:hi]", Engine::Postgres), ["hi"]);
    }

    #[test]
    fn skips_comments() {
        for engine in SQL_ENGINES {
            assert_eq!(params("select 1 -- :no\n, /* :no \n :no */ :yes", engine), ["yes"], "{engine:?}");
        }
        for engine in [Engine::Mysql, Engine::Mariadb] {
            assert_eq!(params("select 1 # :no\n, :yes", engine), ["yes"]);
        }
        assert_eq!(params("select 1 # :a", Engine::Postgres), ["a"]);
    }

    #[test]
    fn postgres_block_comments_nest() {
        assert_eq!(params("/* a /* b */ :no */ :yes", Engine::Postgres), ["yes"]);
    }

    #[test]
    fn mysql_double_dash_comments_need_a_space() {
        assert_eq!(params("select 1--:a\n", Engine::Mysql), ["a"]);
        assert_eq!(params("select 1-- :no\n", Engine::Mysql), Vec::<String>::new());
    }

    #[test]
    fn skips_postgres_dollar_quoted_bodies() {
        let sql = "do $$ begin perform :no; end $$; select $fn$ :no $x$ :no $fn$, $1, :yes";
        assert_eq!(params(sql, Engine::Postgres), ["yes"]);
    }

    #[test]
    fn skips_casts() {
        for engine in SQL_ENGINES {
            assert_eq!(params("select a::int, '1'::text, x ::date, :yes::bigint", engine), ["yes"], "{engine:?}");
        }
    }

    #[test]
    fn an_unterminated_string_hides_the_rest() {
        assert!(params("select 'open :no", Engine::Postgres).is_empty());
    }

    #[test]
    fn writes_text_quoted_and_escaped_for_each_dialect() {
        let text = value(ParamKind::Text, r"it's a\b");
        assert_eq!(literal(&text, Engine::Postgres).unwrap(), r"'it''s a\b'");
        assert_eq!(literal(&text, Engine::Sqlite).unwrap(), r"'it''s a\b'");
        assert_eq!(literal(&text, Engine::Mysql).unwrap(), r"'it''s a\\b'");
        assert_eq!(literal(&text, Engine::Mariadb).unwrap(), r"'it''s a\\b'");
        assert_eq!(literal(&text, Engine::Sqlserver).unwrap(), r"N'it''s a\b'");
    }

    #[test]
    fn writes_numbers_as_typed_and_refuses_others() {
        for number in ["0", "-12", "3.5", "-0.25", ".5", "1e10", "6.02E-23", " 42 "] {
            for engine in SQL_ENGINES {
                assert_eq!(literal(&value(ParamKind::Number, number), engine).unwrap(), number.trim());
            }
        }
        for bad in ["", "abc", "1,5", "--1", "1e", "0x10", "NaN", "inf", "1e999", "+1", "1 2", "1;drop"] {
            assert_eq!(
                literal(&value(ParamKind::Number, bad), Engine::Postgres),
                Err(ParamError::NotANumber(bad.into())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn writes_dates_quoted_and_refuses_invalid_ones() {
        assert_eq!(literal(&value(ParamKind::Date, "2024-02-29"), Engine::Mysql).unwrap(), "'2024-02-29'");
        assert_eq!(literal(&value(ParamKind::Date, "2024-02-29"), Engine::Sqlserver).unwrap(), "'2024-02-29'");
        for bad in ["2023-02-29", "2024-13-01", "2024-1-5", "24-01-05", "2024/01/05", "", "2024-01-05x"] {
            assert_eq!(literal(&value(ParamKind::Date, bad), Engine::Postgres), Err(ParamError::NotADate(bad.into())));
        }
    }

    #[test]
    fn writes_dates_and_times_quoted_and_refuses_invalid_ones() {
        for good in ["2024-01-05 09:30", "2024-01-05 23:59:59", "2024-01-05 00:00:00.125"] {
            assert_eq!(literal(&value(ParamKind::DateTime, good), Engine::Postgres).unwrap(), format!("'{good}'"));
        }
        for bad in [
            "2024-01-05",
            "2024-01-05 24:00",
            "2024-01-05 9:30",
            "2024-01-05 09:60",
            "2024-01-05 09:30:5",
            "2024-01-05 09:30:00.",
            "2024-02-30 10:00",
        ] {
            assert_eq!(
                literal(&value(ParamKind::DateTime, bad), Engine::Postgres),
                Err(ParamError::NotADateTime(bad.into())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn writes_booleans_for_each_dialect() {
        let yes = value(ParamKind::Boolean, "true");
        let no = value(ParamKind::Boolean, "FALSE");
        for engine in [Engine::Postgres, Engine::Mysql, Engine::Mariadb] {
            assert_eq!(
                (literal(&yes, engine).unwrap(), literal(&no, engine).unwrap()),
                ("true".into(), "false".into())
            );
        }
        for engine in [Engine::Sqlserver, Engine::Sqlite] {
            assert_eq!((literal(&yes, engine).unwrap(), literal(&no, engine).unwrap()), ("1".into(), "0".into()));
        }
        assert_eq!(
            literal(&value(ParamKind::Boolean, "yes"), Engine::Postgres),
            Err(ParamError::NotABoolean("yes".into()))
        );
    }

    #[test]
    fn writes_null_whatever_the_value() {
        for engine in SQL_ENGINES {
            assert_eq!(literal(&value(ParamKind::Null, "anything"), engine).unwrap(), "NULL");
        }
    }

    #[test]
    fn substitutes_every_use_and_reports_where_each_literal_went() {
        let values = BTreeMap::from([
            ("name".to_string(), value(ParamKind::Text, "O'Brien")),
            ("id".to_string(), value(ParamKind::Number, "7")),
        ]);
        let sql = "select ':id' from t where id = :id and name = :name or parent = :id::int -- :id";
        let done = substitute(sql, Engine::Postgres, &values).unwrap();
        assert_eq!(done.sql, "select ':id' from t where id = 7 and name = 'O''Brien' or parent = 7::int -- :id");
        let inserted: Vec<_> = done.spans.iter().map(|span| &done.sql[span.clone()]).collect();
        assert_eq!(inserted, ["7", "'O''Brien'", "7"]);
    }

    #[test]
    fn substitutes_nothing_in_sql_without_parameters() {
        let done = substitute("select 1", Engine::Sqlite, &BTreeMap::new()).unwrap();
        assert_eq!(done, Substituted { sql: "select 1".into(), spans: vec![] });
    }

    #[test]
    fn a_parameter_without_a_value_is_named() {
        let values = BTreeMap::from([("a".to_string(), value(ParamKind::Number, "1"))]);
        assert_eq!(substitute("select :a, :b", Engine::Postgres, &values), Err(ParamError::Missing("b".into())));
        assert_eq!(ParamError::Missing("b".into()).to_string(), "Enter a value for :b.");
    }

    #[test]
    fn an_invalid_value_names_its_parameter() {
        let values = BTreeMap::from([("n".to_string(), value(ParamKind::Number, "x"))]);
        let error = substitute("select :n", Engine::Postgres, &values).unwrap_err();
        assert_eq!(
            error,
            ParamError::Invalid { name: "n".into(), error: Box::new(ParamError::NotANumber("x".into())) }
        );
        assert_eq!(error.to_string(), ":n: \"x\" is not a number.");
    }
}
