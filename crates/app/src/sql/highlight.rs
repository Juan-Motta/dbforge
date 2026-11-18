//! Highlighting, a line at a time: each line is split into spans that cover
//! it exactly. Strings, quoted names and block comments may run over several
//! lines, so a line starts in the state the previous one ended in
//! ([`line_states`]).

use std::ops::Range;

use super::Dialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Keyword,
    /// A document's key: coloured as a keyword, not bold.
    Key,
    /// A type name: `integer`, `varchar`.
    Type,
    String,
    Number,
    /// A name followed by `(`.
    Function,
    /// A quoted name: `"order"`, `[order]`.
    Identifier,
    Comment,
}

/// What a line starts inside of, left open by the lines before it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LineState {
    #[default]
    Normal,
    BlockComment,
    /// A string opened with this quote.
    String(char),
    /// A quoted name, closed by this character.
    Identifier(char),
    /// A dollar-quoted string (PostgreSQL), closed by this tag (`$$`, `$body$`).
    Dollar(String),
}

/// One highlighted stretch of a line, by byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

const KEYWORDS: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BEGIN",
    "BETWEEN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "DATABASE",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FOREIGN",
    "FROM",
    "FULL",
    "GRANT",
    "GROUP",
    "HAVING",
    "IF",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "NATURAL",
    "NOT",
    "NULL",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRIMARY",
    "PROCEDURE",
    "REFERENCES",
    "RENAME",
    "REPLACE",
    "RETURNING",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "SCHEMA",
    "SELECT",
    "SET",
    "TABLE",
    "THEN",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VIEW",
    "WHEN",
    "WHERE",
    "WITH",
];

const POSTGRES: &[&str] = &[
    "ANALYZE",
    "CONFLICT",
    "DO",
    "FUNCTION",
    "ILIKE",
    "LANGUAGE",
    "LATERAL",
    "MATERIALIZED",
    "NOTHING",
    "RETURNS",
    "SEQUENCE",
    "SIMILAR",
    "VACUUM",
    "WINDOW",
];
const MYSQL: &[&str] = &[
    "AUTO_INCREMENT",
    "DATABASES",
    "DESCRIBE",
    "DUPLICATE",
    "ENGINE",
    "IGNORE",
    "REGEXP",
    "SHOW",
    "STRAIGHT_JOIN",
    "TABLES",
    "USE",
];
const SQLITE: &[&str] = &["AUTOINCREMENT", "GLOB", "PRAGMA", "ROWID", "VACUUM", "WITHOUT"];
const SQLSERVER: &[&str] =
    &["DECLARE", "EXEC", "EXECUTE", "GO", "IDENTITY", "NOCOUNT", "OUTPUT", "PRINT", "TOP", "TRAN"];
const COSMOS: &[&str] = &["ARRAY", "TOP", "VALUE"];

const TYPES: &[&str] = &[
    "BIGINT",
    "BIGSERIAL",
    "BINARY",
    "BIT",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BYTEA",
    "CHAR",
    "CHARACTER",
    "DATE",
    "DATETIME",
    "DATETIME2",
    "DECIMAL",
    "DOUBLE",
    "FLOAT",
    "INT",
    "INTEGER",
    "JSON",
    "JSONB",
    "MONEY",
    "NCHAR",
    "NUMERIC",
    "NVARCHAR",
    "REAL",
    "SERIAL",
    "SMALLINT",
    "TEXT",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "UUID",
    "UNIQUEIDENTIFIER",
    "VARBINARY",
    "VARCHAR",
];

impl Dialect {
    /// Whether `word` (in upper case) is one of this dialect's keywords.
    pub fn is_keyword(self, word: &str) -> bool {
        KEYWORDS.contains(&word) || self.extra_keywords().contains(&word)
    }

    fn extra_keywords(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => POSTGRES,
            Self::Mysql => MYSQL,
            Self::Sqlite => SQLITE,
            Self::Sqlserver => SQLSERVER,
            Self::Cosmos => COSMOS,
        }
    }

    /// Every keyword, for completion.
    pub fn keywords(self) -> impl Iterator<Item = &'static str> {
        KEYWORDS.iter().chain(self.extra_keywords()).copied()
    }

    /// The characters a string opens with: MySQL and Cosmos DB take `"` too.
    fn string_quotes(self) -> &'static [char] {
        match self {
            Self::Mysql | Self::Cosmos => &['\'', '"'],
            _ => &['\''],
        }
    }
}

/// The state each line of `text` starts in.
pub fn line_states(text: &str, dialect: Dialect) -> Vec<LineState> {
    let mut states = Vec::new();
    let mut state = LineState::Normal;
    for line in text.split('\n') {
        let next = tokenize(line, state.clone(), dialect).1;
        states.push(std::mem::replace(&mut state, next));
    }
    states
}

/// Splits `line`, which starts in `state`, into spans covering it; and the
/// state the next line starts in.
pub fn tokenize(line: &str, state: LineState, dialect: Dialect) -> (Vec<Span>, LineState) {
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |range: Range<usize>, kind: Kind| {
        if range.is_empty() {
            return;
        }
        // Neighbouring spans of a kind are one, so the line has fewer runs.
        match spans.last_mut() {
            Some(last) if last.kind == kind && last.range.end == range.start => last.range.end = range.end,
            _ => spans.push(Span { range, kind }),
        }
    };
    let mut at = 0;

    // Whatever the previous line left open runs on first.
    let mut state = match state {
        LineState::Normal => LineState::Normal,
        open => {
            let (end, still_open) = close(line, 0, &open);
            let kind = if open == LineState::BlockComment {
                Kind::Comment
            } else if matches!(open, LineState::Identifier(_)) {
                Kind::Identifier
            } else {
                Kind::String
            };
            push(0..end, kind);
            at = end;
            if still_open {
                return (spans, open);
            }
            LineState::Normal
        }
    };

    while at < line.len() {
        let rest = &line[at..];
        let c = rest.chars().next().unwrap_or_default();
        let start = at;
        if rest.starts_with("--") || (c == '#' && dialect.hash_comments()) {
            push(at..line.len(), Kind::Comment);
            at = line.len();
        } else if rest.starts_with("/*") {
            let open = LineState::BlockComment;
            let (end, still_open) = close(line, at + 2, &open);
            push(at..end, Kind::Comment);
            at = end;
            if still_open {
                state = open;
            }
        } else if dialect.string_quotes().contains(&c) {
            let open = LineState::String(c);
            let (end, still_open) = close(line, at + 1, &open);
            push(at..end, Kind::String);
            at = end;
            if still_open {
                state = open;
            }
        } else if let Some(&(_, closing)) = dialect.identifier_quotes().iter().find(|(opening, _)| *opening == c) {
            let open = LineState::Identifier(closing);
            let (end, still_open) = close(line, at + 1, &open);
            push(at..end, Kind::Identifier);
            at = end;
            if still_open {
                state = open;
            }
        } else if c == '$' && dialect.dollar_strings() && dollar_tag(rest).is_some() {
            let tag = dollar_tag(rest).unwrap_or_default().to_owned();
            let open = LineState::Dollar(tag.clone());
            let (end, still_open) = close(line, at + tag.len(), &open);
            push(at..end, Kind::String);
            at = end;
            if still_open {
                state = open;
            }
        } else if c.is_ascii_digit() || (c == '.' && rest[1..].starts_with(|n: char| n.is_ascii_digit())) {
            at += rest.find(|n: char| !(n.is_ascii_alphanumeric() || n == '.' || n == '_')).unwrap_or(rest.len());
            push(start..at, Kind::Number);
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            at += rest.find(|n: char| !(n.is_alphanumeric() || n == '_' || n == '@' || n == '$')).unwrap_or(rest.len());
            let word = line[start..at].to_uppercase();
            let next = line[at..].trim_start().chars().next();
            let kind = if dialect.is_keyword(&word) {
                Kind::Keyword
            } else if TYPES.contains(&word.as_str()) {
                Kind::Type
            } else if next == Some('(') {
                Kind::Function
            } else {
                Kind::Plain
            };
            push(start..at, kind);
        } else {
            at += c.len_utf8();
            push(start..at, Kind::Plain);
        }
    }
    (spans, state)
}

/// Where what `open` started ends in `line`, searching from `from`: the
/// offset just past its closing, or the line's end and `true` when it stays open.
fn close(line: &str, from: usize, open: &LineState) -> (usize, bool) {
    let from = from.min(line.len());
    match open {
        LineState::Normal => (from, false),
        LineState::BlockComment => match line[from..].find("*/") {
            Some(end) => (from + end + 2, false),
            None => (line.len(), true),
        },
        LineState::Dollar(tag) => match line[from..].find(tag.as_str()) {
            Some(end) => (from + end + tag.len(), false),
            None => (line.len(), true),
        },
        // A doubled quote is the quote itself: 'it''s'.
        LineState::String(quote) | LineState::Identifier(quote) => {
            let mut chars = line[from..].char_indices().peekable();
            while let Some((index, c)) = chars.next() {
                if c == *quote {
                    if chars.peek().is_some_and(|(_, next)| next == quote) {
                        chars.next();
                        continue;
                    }
                    return (from + index + c.len_utf8(), false);
                }
                // A backslash escapes the next character in MySQL's strings; harmless elsewhere.
                if c == '\\' && matches!(open, LineState::String(_)) {
                    chars.next();
                }
            }
            (line.len(), true)
        }
    }
}

/// The tag `rest` opens a dollar-quoted string with: `$$` or `$name$`.
fn dollar_tag(rest: &str) -> Option<&str> {
    let inner = rest.strip_prefix('$')?;
    let end = inner.find('$')?;
    let name = &inner[..end];
    let valid =
        name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
    valid.then(|| &rest[..end + 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(line: &str, dialect: Dialect) -> Vec<(&str, Kind)> {
        tokenize(line, LineState::Normal, dialect).0.into_iter().map(|span| (&line[span.range], span.kind)).collect()
    }

    fn has(line: &str, dialect: Dialect, text: &str, kind: Kind) -> bool {
        spans(line, dialect).contains(&(text, kind))
    }

    #[test]
    fn spans_cover_the_line_and_classify_it() {
        let line = "SELECT count(*), 'it''s', 4.5 FROM t WHERE a::int = 1 -- done";
        let all = spans(line, Dialect::Postgres);
        assert_eq!(all.iter().map(|(text, _)| *text).collect::<String>(), line);
        assert!(has(line, Dialect::Postgres, "SELECT", Kind::Keyword));
        assert!(has(line, Dialect::Postgres, "count", Kind::Function));
        assert!(has(line, Dialect::Postgres, "'it''s'", Kind::String));
        assert!(has(line, Dialect::Postgres, "4.5", Kind::Number));
        assert!(has(line, Dialect::Postgres, "int", Kind::Type));
        assert!(has(line, Dialect::Postgres, "-- done", Kind::Comment));
    }

    #[test]
    fn keywords_and_quotes_follow_the_dialect() {
        assert!(has("SELECT TOP 5 [order] FROM t", Dialect::Sqlserver, "TOP", Kind::Keyword));
        assert!(has("SELECT TOP 5 [order] FROM t", Dialect::Sqlserver, "[order]", Kind::Identifier));
        assert!(!has("SELECT TOP 5", Dialect::Postgres, "TOP", Kind::Keyword));
        assert!(has("SELECT `select` # note", Dialect::Mysql, "# note", Kind::Comment));
        assert!(has("SELECT \"a\"", Dialect::Mysql, "\"a\"", Kind::String));
        assert!(has("SELECT \"select\"", Dialect::Postgres, "\"select\"", Kind::Identifier));
        assert!(!has("SELECT 1 # x", Dialect::Postgres, "# x", Kind::Comment));
    }

    #[test]
    fn block_comments_and_strings_run_over_lines() {
        let text = "SELECT /* one\ntwo */ 1, 'a\nb' FROM t\nSELECT 2";
        let states = line_states(text, Dialect::Postgres);
        assert_eq!(states, [LineState::Normal, LineState::BlockComment, LineState::String('\''), LineState::Normal]);
        let (second, _) = tokenize("two */ 1, 'a", LineState::BlockComment, Dialect::Postgres);
        assert_eq!((second[0].range.clone(), second[0].kind), (0..6, Kind::Comment));
    }

    #[test]
    fn dollar_quoted_bodies_are_strings_in_postgres() {
        let text = "CREATE FUNCTION f() RETURNS int AS $body$\nSELECT 1;\n$body$ LANGUAGE sql;";
        let states = line_states(text, Dialect::Postgres);
        assert_eq!(states[1], LineState::Dollar("$body$".into()));
        assert_eq!(states[2], LineState::Dollar("$body$".into()));
        let (third, next) = tokenize("$body$ LANGUAGE sql;", states[2].clone(), Dialect::Postgres);
        assert_eq!((third[0].kind, next), (Kind::String, LineState::Normal));
        let parameter = spans("SELECT $1", Dialect::Postgres);
        assert!(parameter.iter().all(|(_, kind)| *kind != Kind::String), "a parameter is no string");
    }
}
