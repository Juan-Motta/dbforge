//! The subset of mongosh the query console reads: `use name` and method calls
//! on `db` or a collection, such as
//! `db.orders.find({total: {$gt: 100}}).sort({total: -1}).limit(5)`.
//!
//! Arguments are JavaScript literals as mongosh writes them: unquoted keys,
//! either quote style, trailing commas, `/regex/i`, and the shell's
//! constructors (`ObjectId("…")`, `ISODate("…")`, `new Date(…)`,
//! `NumberLong(…)`, `NumberInt(…)`, `NumberDecimal("…")`). Numbers follow
//! mongosh: whole numbers that fit 32 bits are `int`, every other number is a
//! `double`. Nothing is evaluated; variables and functions are not supported.

use chrono::{DateTime as ChronoDateTime, NaiveDate, NaiveDateTime};
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{Bson, DateTime, Decimal128, Document, Regex};

/// One statement of a script.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// `use name`: later statements run in that database.
    Use(String),
    /// `db.method(…)` when `collection` is `None`, else `db.collection.method(…)`,
    /// each followed by the methods chained on its result, such as `.sort(…)`.
    Call { collection: Option<String>, method: Method, chain: Vec<Method> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Method {
    pub name: String,
    pub args: Vec<Bson>,
}

/// Reads a whole script. Statements are separated by `;` or simply follow each
/// other; `//` and `/* */` comments are skipped.
pub fn parse(script: &str) -> Result<Vec<Statement>, String> {
    let mut parser = Parser { text: script, at: 0 };
    let mut statements = Vec::new();
    loop {
        parser.skip_blank();
        while parser.eat(';') {
            parser.skip_blank();
        }
        if parser.at == script.len() {
            return Ok(statements);
        }
        statements.push(parser.statement()?);
    }
}

/// Reads one value, such as a cell typed into the grid.
pub fn parse_value(text: &str) -> Result<Bson, String> {
    let mut parser = Parser { text, at: 0 };
    let value = parser.value()?;
    parser.skip_blank();
    if parser.at < text.len() {
        return Err(parser.error("Unexpected text after the value"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    /// Byte offset of the next character.
    at: usize,
}

impl Parser<'_> {
    fn statement(&mut self) -> Result<Statement, String> {
        let start = self.at;
        let word = self.identifier();
        if word == "use" {
            self.skip_inline_blank();
            let name: String = self.take_while(|c| !c.is_whitespace() && c != ';');
            if name.is_empty() {
                return Err(self.error("Expected a database name after `use`"));
            }
            return Ok(Statement::Use(name.trim_matches(['"', '\'']).to_string()));
        }
        if word != "db" {
            self.at = start;
            return Err(self.error("Expected a statement starting with `db.` or `use`"));
        }

        // `db.a.b.find(` names collection `a.b`: the last name before `(` is the method.
        let mut path: Vec<String> = Vec::new();
        let method = loop {
            self.skip_blank();
            if self.eat('[') {
                self.skip_blank();
                path.push(self.string()?);
                self.skip_blank();
                self.expect(']')?;
                continue;
            }
            self.expect('.')?;
            self.skip_blank();
            let name = self.identifier();
            if name.is_empty() {
                return Err(self.error("Expected a collection or method name"));
            }
            self.skip_blank();
            if self.peek() == Some('(') {
                let method = Method { args: self.arguments()?, name };
                if path.is_empty() && method.name == "getCollection" {
                    match method.args.as_slice() {
                        [Bson::String(name)] => path.push(name.clone()),
                        _ => return Err(self.error("getCollection takes the collection's name")),
                    }
                    continue;
                }
                break method;
            }
            path.push(name);
        };
        let collection = (!path.is_empty()).then(|| path.join("."));

        let mut chain = Vec::new();
        loop {
            let before = self.at;
            self.skip_blank();
            if !self.eat('.') {
                self.at = before;
                break;
            }
            self.skip_blank();
            let name = self.identifier();
            self.skip_blank();
            if name.is_empty() || self.peek() != Some('(') {
                return Err(self.error("Expected a method call"));
            }
            chain.push(Method { args: self.arguments()?, name });
        }
        Ok(Statement::Call { collection, method, chain })
    }

    /// `(a, b, …)` with an optional trailing comma.
    fn arguments(&mut self) -> Result<Vec<Bson>, String> {
        self.expect('(')?;
        let mut args = Vec::new();
        loop {
            self.skip_blank();
            if self.eat(')') {
                return Ok(args);
            }
            args.push(self.value()?);
            self.skip_blank();
            if !self.eat(',') {
                self.skip_blank();
                self.expect(')')?;
                return Ok(args);
            }
        }
    }

    fn value(&mut self) -> Result<Bson, String> {
        self.skip_blank();
        match self.peek() {
            Some('{') => self.object().map(Bson::Document),
            Some('[') => self.array(),
            Some('"' | '\'') => self.string().map(Bson::String),
            Some('/') => self.regex(),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => self.number(),
            Some(c) if is_identifier_start(c) => self.word_value(),
            Some(_) => Err(self.error("Expected a value")),
            None => Err(self.error("Expected a value, found the end of the script")),
        }
    }

    fn object(&mut self) -> Result<Document, String> {
        self.expect('{')?;
        let mut document = Document::new();
        loop {
            self.skip_blank();
            if self.eat('}') {
                return Ok(document);
            }
            let key = match self.peek() {
                Some('"' | '\'') => self.string()?,
                Some(c) if is_identifier_start(c) || c.is_ascii_digit() => {
                    self.take_while(|c| is_identifier_part(c) || c == '.')
                }
                _ => return Err(self.error("Expected a field name")),
            };
            self.skip_blank();
            self.expect(':')?;
            let value = self.value()?;
            document.insert(key, value);
            self.skip_blank();
            if !self.eat(',') {
                self.skip_blank();
                self.expect('}')?;
                return Ok(document);
            }
        }
    }

    fn array(&mut self) -> Result<Bson, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            if self.eat(']') {
                return Ok(Bson::Array(items));
            }
            items.push(self.value()?);
            self.skip_blank();
            if !self.eat(',') {
                self.skip_blank();
                self.expect(']')?;
                return Ok(Bson::Array(items));
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = match self.peek() {
            Some(q @ ('"' | '\'')) => q,
            _ => return Err(self.error("Expected a string")),
        };
        let start = self.at;
        self.at += 1;
        let mut text = String::new();
        loop {
            let Some(c) = self.next() else {
                self.at = start;
                return Err(self.error("This string is never closed"));
            };
            match c {
                c if c == quote => return Ok(text),
                '\\' => {
                    let escaped = self.next().ok_or_else(|| self.error("This string is never closed"))?;
                    text.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        '0' => '\0',
                        'u' => {
                            let hex: String = (0..4).filter_map(|_| self.next()).collect();
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.error("Invalid \\u escape"))?
                        }
                        other => other,
                    });
                }
                c => text.push(c),
            }
        }
    }

    fn regex(&mut self) -> Result<Bson, String> {
        let start = self.at;
        self.expect('/')?;
        let mut pattern = String::new();
        loop {
            match self.next() {
                None | Some('\n') => {
                    self.at = start;
                    return Err(self.error("This regular expression is never closed"));
                }
                Some('/') => break,
                Some('\\') => {
                    pattern.push('\\');
                    if let Some(c) = self.next() {
                        pattern.push(c);
                    }
                }
                Some(c) => pattern.push(c),
            }
        }
        let mut options: Vec<char> = self.take_while(|c| c.is_ascii_alphabetic()).chars().collect();
        if let Some(bad) = options.iter().find(|c| !"imsxlu".contains(**c)) {
            return Err(self.error(&format!("Unknown regular expression flag `{bad}`")));
        }
        // The server expects the flags in alphabetical order.
        options.sort_unstable();
        Ok(Bson::RegularExpression(Regex { pattern, options: options.into_iter().collect() }))
    }

    fn number(&mut self) -> Result<Bson, String> {
        let start = self.at;
        let negative = self.eat('-');
        if !negative {
            self.eat('+');
        }
        if self.identifier_ahead("Infinity") {
            return Ok(Bson::Double(if negative { f64::NEG_INFINITY } else { f64::INFINITY }));
        }
        let mut previous = ' ';
        let digits = self.take_while(|c| {
            let ok = c.is_ascii_digit()
                || c == '.'
                || c == 'e'
                || c == 'E'
                || (matches!(previous, 'e' | 'E') && matches!(c, '+' | '-'));
            previous = c;
            ok
        });
        let text = format!("{}{digits}", if negative { "-" } else { "" });
        if !digits.contains(['.', 'e', 'E'])
            && let Ok(whole) = text.parse::<i64>()
        {
            return Ok(i32::try_from(whole).map_or(Bson::Double(whole as f64), Bson::Int32));
        }
        text.parse::<f64>().map(Bson::Double).map_err(|_| {
            self.at = start;
            self.error("Invalid number")
        })
    }

    /// `true`, `null`, `ObjectId("…")`, `new Date(…)` and the like.
    fn word_value(&mut self) -> Result<Bson, String> {
        let start = self.at;
        let mut word = self.identifier();
        if word == "new" {
            self.skip_blank();
            word = self.identifier();
        }
        let simple = match word.as_str() {
            "true" => Some(Bson::Boolean(true)),
            "false" => Some(Bson::Boolean(false)),
            "null" | "undefined" => Some(Bson::Null),
            "Infinity" => Some(Bson::Double(f64::INFINITY)),
            "NaN" => Some(Bson::Double(f64::NAN)),
            _ => None,
        };
        if let Some(value) = simple {
            return Ok(value);
        }
        self.skip_blank();
        if self.peek() != Some('(') {
            self.at = start;
            return Err(self.error(&format!("Unknown name `{word}`: variables aren't supported")));
        }
        let args = self.arguments()?;
        let fail = |parser: &mut Self, message: &str| {
            parser.at = start;
            Err(parser.error(message))
        };
        match (word.as_str(), args.as_slice()) {
            ("ObjectId", []) => Ok(Bson::ObjectId(ObjectId::new())),
            ("ObjectId", [Bson::String(hex)]) => match ObjectId::parse_str(hex) {
                Ok(id) => Ok(Bson::ObjectId(id)),
                Err(_) => fail(self, "ObjectId takes 24 hexadecimal characters"),
            },
            ("ISODate" | "Date", []) => Ok(Bson::DateTime(DateTime::now())),
            ("ISODate" | "Date", [Bson::String(text)]) => match date(text) {
                Some(date) => Ok(Bson::DateTime(date)),
                None => fail(self, &format!("Unreadable date \"{text}\"")),
            },
            ("Date", [Bson::Int32(_) | Bson::Double(_)]) => {
                Ok(Bson::DateTime(DateTime::from_millis(whole(&args[0]).unwrap_or_default())))
            }
            ("NumberLong" | "Long", [value]) => match whole(value) {
                Some(n) => Ok(Bson::Int64(n)),
                None => fail(self, &format!("{word} takes a whole number")),
            },
            ("NumberInt" | "Int32", [value]) => match whole(value).and_then(|n| i32::try_from(n).ok()) {
                Some(n) => Ok(Bson::Int32(n)),
                None => fail(self, &format!("{word} takes a whole number that fits in 32 bits")),
            },
            ("NumberDecimal" | "Decimal128", [Bson::String(text)]) => match text.parse::<Decimal128>() {
                Ok(n) => Ok(Bson::Decimal128(n)),
                Err(_) => fail(self, &format!("Unreadable decimal \"{text}\"")),
            },
            (
                "ObjectId" | "ISODate" | "Date" | "NumberLong" | "Long" | "NumberInt" | "Int32" | "NumberDecimal"
                | "Decimal128",
                _,
            ) => fail(self, &format!("Unexpected arguments for {word}")),
            _ => fail(self, &format!("Unknown function `{word}`")),
        }
    }

    fn identifier(&mut self) -> String {
        if self.peek().is_some_and(is_identifier_start) { self.take_while(is_identifier_part) } else { String::new() }
    }

    fn identifier_ahead(&mut self, word: &str) -> bool {
        let rest = &self.text[self.at..];
        let follows = rest.strip_prefix(word).is_some_and(|after| !after.starts_with(is_identifier_part));
        if follows {
            self.at += word.len();
        }
        follows
    }

    fn take_while(&mut self, mut keep: impl FnMut(char) -> bool) -> String {
        let rest = &self.text[self.at..];
        let end = rest.find(|c| !keep(c)).unwrap_or(rest.len());
        self.at += end;
        rest[..end].to_string()
    }

    /// Skips whitespace and comments.
    fn skip_blank(&mut self) {
        loop {
            self.take_while(char::is_whitespace);
            let rest = &self.text[self.at..];
            if rest.starts_with("//") {
                self.at += rest.find('\n').unwrap_or(rest.len());
            } else if rest.starts_with("/*") {
                self.at += rest.find("*/").map_or(rest.len(), |end| end + 2);
            } else {
                return;
            }
        }
    }

    fn skip_inline_blank(&mut self) {
        self.take_while(|c| c == ' ' || c == '\t');
    }

    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, expected: char) -> bool {
        let found = self.peek() == Some(expected);
        if found {
            self.at += expected.len_utf8();
        }
        found
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.eat(expected) { Ok(()) } else { Err(self.error(&format!("Expected `{expected}`"))) }
    }

    /// `message` with where it happened: `Expected `)` (line 2, column 7)`.
    fn error(&self, message: &str) -> String {
        let before = &self.text[..self.at];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!("{message} (line {line}, column {column})")
    }
}

fn is_identifier_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_identifier_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// A whole number from a literal: `5`, `5.0` or `"5"`.
fn whole(value: &Bson) -> Option<i64> {
    match value {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(n) if n.fract() == 0.0 && n.abs() < 9.2e18 => Some(*n as i64),
        Bson::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Dates as mongosh reads them; without a zone they are UTC.
pub fn date(text: &str) -> Option<DateTime> {
    let millis = if let Ok(date) = ChronoDateTime::parse_from_rfc3339(text) {
        date.timestamp_millis()
    } else if let Ok(date) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
    {
        date.and_utc().timestamp_millis()
    } else {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()
    };
    Some(DateTime::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{bson, doc};

    use super::*;

    fn call(collection: &str, name: &str, args: Vec<Bson>, chain: Vec<Method>) -> Statement {
        Statement::Call { collection: Some(collection.into()), method: method(name, args), chain }
    }

    fn method(name: &str, args: Vec<Bson>) -> Method {
        Method { name: name.into(), args }
    }

    #[test]
    fn reads_find_with_chained_methods() {
        let script = "db.orders.find({total: {$gt: 100}, 'status': \"paid\",}).sort({total: -1}).limit(5)";
        assert_eq!(
            parse(script).unwrap(),
            [call(
                "orders",
                "find",
                vec![bson!({ "total": { "$gt": 100 }, "status": "paid" })],
                vec![method("sort", vec![bson!({ "total": -1 })]), method("limit", vec![Bson::Int32(5)])],
            )]
        );
    }

    #[test]
    fn reads_several_statements_with_or_without_semicolons() {
        let script = "use shop\n// orders first\ndb.orders.countDocuments()\n\ndb.getCollection('my.items').find(); /* done */ db.stats();";
        let statements = parse(script).unwrap();
        assert_eq!(statements[0], Statement::Use("shop".into()));
        assert_eq!(statements[1], call("orders", "countDocuments", vec![], vec![]));
        assert_eq!(statements[2], call("my.items", "find", vec![], vec![]));
        assert_eq!(statements[3], Statement::Call { collection: None, method: method("stats", vec![]), chain: vec![] });
        assert_eq!(statements.len(), 4);
        // Dotted names and brackets name the collection.
        assert_eq!(parse("db.system.profile.find()").unwrap()[0], call("system.profile", "find", vec![], vec![]));
        assert_eq!(parse("db['order items'].find()").unwrap()[0], call("order items", "find", vec![], vec![]));
    }

    #[test]
    fn reads_mongosh_literals() {
        let value = parse_value(
            r#"{ id: ObjectId("65a0f0f0f0f0f0f0f0f0f0f0"), at: ISODate("2024-01-02T03:04:05Z"), day: new Date("2024-01-02"),
                 n: 1, big: 3000000000, x: 1.5, neg: -2, e: 1e3, long: NumberLong("9007199254740993"),
                 int: NumberInt(7), dec: NumberDecimal("10.50"), name: /^ab.c/mi, list: [true, null, 'it\'s'], "a.b": {} }"#,
        )
        .unwrap();
        let Bson::Document(document) = value else { panic!("{value:?}") };
        assert_eq!(document.get_object_id("id").unwrap().to_hex(), "65a0f0f0f0f0f0f0f0f0f0f0");
        assert_eq!(document.get_datetime("at").unwrap().timestamp_millis(), 1_704_164_645_000);
        assert_eq!(document.get_datetime("day").unwrap().timestamp_millis(), 1_704_153_600_000);
        assert_eq!(document.get("n"), Some(&Bson::Int32(1)));
        assert_eq!(document.get("big"), Some(&Bson::Double(3e9)));
        assert_eq!(document.get("x"), Some(&Bson::Double(1.5)));
        assert_eq!(document.get("neg"), Some(&Bson::Int32(-2)));
        assert_eq!(document.get("e"), Some(&Bson::Double(1000.0)));
        assert_eq!(document.get("long"), Some(&Bson::Int64(9_007_199_254_740_993)));
        assert_eq!(document.get("int"), Some(&Bson::Int32(7)));
        assert_eq!(document.get("dec").map(ToString::to_string), Some("10.50".into()));
        assert_eq!(
            document.get("name"),
            Some(&Bson::RegularExpression(Regex { pattern: "^ab.c".into(), options: "im".into() }))
        );
        assert_eq!(document.get("list"), Some(&bson!([true, null, "it's"])));
        assert_eq!(document.get("a.b"), Some(&Bson::Document(doc! {})));
    }

    #[test]
    fn errors_say_what_and_where() {
        assert_eq!(parse("db.orders.find({a: 1)").unwrap_err(), "Expected `}` (line 1, column 21)");
        assert_eq!(
            parse("show dbs").unwrap_err(),
            "Expected a statement starting with `db.` or `use` (line 1, column 1)"
        );
        assert_eq!(
            parse("db.orders.find({a: x})").unwrap_err(),
            "Unknown name `x`: variables aren't supported (line 1, column 20)"
        );
        assert_eq!(parse("db.t.find({a: 'open})").unwrap_err(), "This string is never closed (line 1, column 15)");
        assert_eq!(
            parse("db.t.find(\n  ObjectId('nope'))").unwrap_err(),
            "ObjectId takes 24 hexadecimal characters (line 2, column 3)"
        );
        assert_eq!(parse("db.orders").unwrap_err(), "Expected `.` (line 1, column 10)");
    }
}
