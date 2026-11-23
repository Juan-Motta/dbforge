//! Reads a SQL file the way the mysql client does, only to find what would
//! make it run somewhere else than the database it's restored into: `USE`,
//! `CREATE DATABASE` (or `SCHEMA`) and the client's own `use`, `connect`, `\u`
//! and `\r`. Strings, quoted names and comments don't count; the content of a
//! `/*! … */` comment does, as MySQL runs it. `DELIMITER` is followed, so a
//! routine's body is one statement.

use std::io::BufRead;

use crate::error::AppError;

/// A statement that sends the rest of a file to another database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Switch {
    /// What the statement does, as `USE`, `CREATE DATABASE`, `\u`.
    pub command: String,
    /// The database it names, when it names one.
    pub database: Option<String>,
}

impl Switch {
    /// The refusal to restore a file with this statement into `target`.
    pub fn refusal(&self, target: &str) -> AppError {
        let does = match self.command.as_str() {
            "CREATE DATABASE" | "CREATE SCHEMA" => "creates another database",
            "connect" | "\\r" => "connects to another database",
            _ => "switches to another database",
        };
        let statement = match &self.database {
            Some(name) => format!("{} {}", self.command, backticked(name)),
            None => self.command.clone(),
        };
        let mut text = format!(
            "This file {does} ({statement}), so it would not restore into {target}. Export it again without --databases"
        );
        match &self.database {
            Some(name) => text.push_str(&format!(", or restore it into {}.", backticked(name))),
            None => text.push('.'),
        }
        AppError::Invalid(text)
    }
}

fn backticked(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// The first statement in `input` that would leave `target`: one naming
/// another database, or one naming none (`connect` alone).
pub(super) fn leaves(input: impl BufRead, target: &str) -> std::io::Result<Option<Switch>> {
    let mut scanner = Scanner::default();
    let elsewhere =
        |scanner: &mut Scanner| scanner.switches.drain(..).find(|switch| switch.database.as_deref() != Some(target));
    for line in input.split(b'\n') {
        scanner.line(&String::from_utf8_lossy(&line?));
        if let Some(switch) = elsewhere(&mut scanner) {
            return Ok(Some(switch));
        }
    }
    scanner.end();
    Ok(elsewhere(&mut scanner))
}

/// Where the scanner is between characters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Code,
    /// In a string or quoted name opened with this quote.
    Quoted(char),
    /// In a `/* … */` comment that isn't run.
    Comment,
}

/// The longest start of a statement kept: enough for any `CREATE DATABASE` name.
const HEAD: usize = 512;

#[derive(Debug)]
struct Scanner {
    mode: Mode,
    /// Inside a `/*! … */` comment, whose content is code.
    executable: bool,
    delimiter: String,
    /// The current statement's code so far (comments as spaces), up to `HEAD` characters.
    head: String,
    /// The current statement's code ends with the delimiter's characters seen so far.
    tail: Vec<char>,
    /// The switches found and not yet looked at.
    switches: Vec<Switch>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            mode: Mode::Code,
            executable: false,
            delimiter: ";".into(),
            head: String::new(),
            tail: Vec::new(),
            switches: Vec::new(),
        }
    }
}

impl Scanner {
    fn line(&mut self, line: &str) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if self.mode == Mode::Code && !self.executable && self.head.trim().is_empty() && self.command(line) {
            self.head.clear();
            return;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            match self.mode {
                Mode::Quoted(quote) => {
                    self.push(c);
                    if c == '\\' && quote != '`' {
                        if let Some(next) = next {
                            self.push(next);
                        }
                        i += 1;
                    } else if c == quote {
                        self.mode = Mode::Code;
                    }
                }
                Mode::Comment => {
                    if c == '*' && next == Some('/') {
                        self.mode = Mode::Code;
                        self.push(' ');
                        i += 1;
                    }
                }
                Mode::Code => {
                    if c == '\'' || c == '"' || c == '`' {
                        self.mode = Mode::Quoted(c);
                        self.push(c);
                    } else if c == '#'
                        || (c == '-' && next == Some('-') && chars.get(i + 2).is_none_or(|c| c.is_whitespace()))
                    {
                        break;
                    } else if c == '/' && next == Some('*') {
                        let rest: String = chars[i + 2..].iter().collect();
                        let marker = if rest.starts_with('!') {
                            Some(1)
                        } else if rest.starts_with("M!") {
                            Some(2)
                        } else {
                            None
                        };
                        self.push(' ');
                        match marker {
                            Some(marker) => {
                                self.executable = true;
                                i += 2 + marker;
                                while chars.get(i).is_some_and(char::is_ascii_digit) {
                                    i += 1;
                                }
                                continue;
                            }
                            None => {
                                self.mode = Mode::Comment;
                                i += 1;
                            }
                        }
                    } else if self.executable && c == '*' && next == Some('/') {
                        self.executable = false;
                        self.push(' ');
                        i += 1;
                    } else if c == '\\' && matches!(next, Some('u' | 'r')) {
                        let rest: String = chars[i + 2..].iter().collect();
                        let command = if next == Some('u') { "\\u" } else { "\\r" };
                        self.switches.push(Switch { command: command.into(), database: first_name(&rest) });
                        return;
                    } else {
                        self.push(c);
                        if self.ends_statement() {
                            self.statement();
                        }
                    }
                }
            }
            i += 1;
        }
        if self.mode == Mode::Code {
            self.push(' ');
        } else if let Mode::Quoted(_) = self.mode {
            self.push('\n');
        }
    }

    /// A client command filling `line` at a statement's start: `DELIMITER`,
    /// `use` or `connect` (which need no delimiter). True when it was one.
    fn command(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let (word, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
        let rest = rest.trim();
        if word.eq_ignore_ascii_case("delimiter") || word == "\\d" {
            if let Some(delimiter) = rest.split_whitespace().next() {
                self.delimiter = delimiter.to_string();
            }
            return true;
        }
        if word.trim_end_matches(';').eq_ignore_ascii_case("connect") {
            self.switches.push(Switch { command: "connect".into(), database: first_name(rest) });
            return true;
        }
        false
    }

    fn push(&mut self, c: char) {
        if self.head.chars().count() < HEAD {
            self.head.push(c);
        }
        if self.mode == Mode::Code && !self.executable {
            self.tail.push(c);
            let keep = self.delimiter.chars().count();
            if self.tail.len() > keep {
                self.tail.drain(..self.tail.len() - keep);
            }
        } else {
            self.tail.clear();
        }
    }

    fn ends_statement(&self) -> bool {
        self.tail.iter().copied().eq(self.delimiter.chars())
    }

    /// The statement read so far is complete.
    fn statement(&mut self) {
        let head = std::mem::take(&mut self.head);
        self.tail.clear();
        if let Some(switch) = switch_in(head.trim_end().strip_suffix(self.delimiter.as_str()).unwrap_or(&head)) {
            self.switches.push(switch);
        }
    }

    fn end(&mut self) {
        if let Some(switch) = switch_in(&std::mem::take(&mut self.head)) {
            self.switches.push(switch);
        }
    }
}

/// The switch `statement` (comments already removed) makes, if it makes one.
fn switch_in(statement: &str) -> Option<Switch> {
    let mut words = Words(statement.trim_start());
    let first = words.word()?;
    if first.eq_ignore_ascii_case("use") {
        return Some(Switch { command: "USE".into(), database: words.name() });
    }
    if !first.eq_ignore_ascii_case("create") {
        return None;
    }
    let kind = words.word()?;
    let command = if kind.eq_ignore_ascii_case("database") {
        "CREATE DATABASE"
    } else if kind.eq_ignore_ascii_case("schema") {
        "CREATE SCHEMA"
    } else {
        return None;
    };
    let mut after = Words(words.0);
    if after.word().is_some_and(|w| w.eq_ignore_ascii_case("if"))
        && after.word().is_some_and(|w| w.eq_ignore_ascii_case("not"))
        && after.word().is_some_and(|w| w.eq_ignore_ascii_case("exists"))
    {
        words = after;
    }
    Some(Switch { command: command.into(), database: words.name() })
}

/// The database a client command's argument names: its first word, quoted or not.
fn first_name(text: &str) -> Option<String> {
    Words(text.trim_start()).name()
}

/// Reads words and names off the front of a statement.
struct Words<'a>(&'a str);

impl<'a> Words<'a> {
    /// The next bare word.
    fn word(&mut self) -> Option<&'a str> {
        let text = self.0.trim_start();
        let end = text.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).unwrap_or(text.len());
        if end == 0 {
            return None;
        }
        self.0 = &text[end..];
        Some(&text[..end])
    }

    /// The next name: quoted with backticks or double quotes, or bare.
    fn name(&mut self) -> Option<String> {
        let text = self.0.trim_start();
        let quote = text.chars().next()?;
        if quote == '`' || quote == '"' {
            let mut name = String::new();
            let mut chars = text[1..].char_indices().peekable();
            while let Some((_, c)) = chars.next() {
                if c == quote {
                    if chars.peek().is_some_and(|&(_, next)| next == quote) {
                        chars.next();
                    } else {
                        return Some(name);
                    }
                }
                name.push(c);
            }
            return None;
        }
        let end = text.find(|c: char| c.is_whitespace() || c == ';').unwrap_or(text.len());
        (end > 0).then(|| text[..end].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(script: &str) -> Option<Switch> {
        leaves(script.as_bytes(), "copy").unwrap()
    }

    fn switch(command: &str, database: &str) -> Option<Switch> {
        Some(Switch { command: command.into(), database: Some(database.into()) })
    }

    #[test]
    fn use_and_create_database_are_found_with_the_name() {
        assert_eq!(found("CREATE TABLE t (id INT);\nUSE `shop`;\n"), switch("USE", "shop"));
        assert_eq!(found("use shop;"), switch("USE", "shop"));
        assert_eq!(found("USE\n  `we``ird`\n;"), switch("USE", "we`ird"));
        assert_eq!(found("SELECT 1; USE shop; SELECT 2;"), switch("USE", "shop"));
        assert_eq!(
            found("CREATE DATABASE /*!32312 IF NOT EXISTS*/ `shop` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;"),
            switch("CREATE DATABASE", "shop")
        );
        assert_eq!(found("create schema if not exists shop;"), switch("CREATE SCHEMA", "shop"));
        assert_eq!(found("/* a note */ USE shop;"), switch("USE", "shop"));
        assert_eq!(found("/*!40101 USE shop */;"), switch("USE", "shop"));
    }

    #[test]
    fn client_commands_are_found() {
        assert_eq!(found("CREATE TABLE t (id INT);\nuse shop\nCREATE TABLE u (id INT);\n"), switch("USE", "shop"));
        assert_eq!(found("\\u shop\n"), switch("\\u", "shop"));
        assert_eq!(found("SELECT 1;\n\\r shop localhost\n"), switch("\\r", "shop"));
        assert_eq!(found("connect shop\n"), switch("connect", "shop"));
        assert_eq!(found("CONNECT\n"), Some(Switch { command: "connect".into(), database: None }));
    }

    #[test]
    fn strings_quoted_names_and_comments_dont_count() {
        let script = "-- USE shop;\n# USE shop;\n/* USE shop; CREATE DATABASE x; */\n\
             INSERT INTO t VALUES ('USE shop;', \"\\u shop\", 'it''s; USE shop;', 'a\\'; USE shop;');\n\
             CREATE TABLE `USE shop; x` (id INT);\n\
             INSERT INTO t VALUES ('multi\nUSE shop;\nline');\n\
             CREATE TABLE users (id INT);\nCREATE VIEW v AS SELECT 1 AS `use`;\n\
             SELECT 1 --not a comment\n;\n";
        assert_eq!(found(script), None);
    }

    #[test]
    fn a_routine_body_is_one_statement() {
        let script =
            "DELIMITER ;;\nCREATE PROCEDURE p() BEGIN\n  SELECT 'x;';\n  SELECT 2;\nEND ;;\nDELIMITER ;\nSELECT 3;\n";
        assert_eq!(found(script), None);
        let after = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; END $$\nUSE shop $$\n";
        assert_eq!(found(after), switch("USE", "shop"));
    }

    #[test]
    fn the_target_itself_is_allowed() {
        let file = "CREATE DATABASE IF NOT EXISTS `copy`;\nUSE `copy`;\nCREATE TABLE t (id INT);\n";
        assert_eq!(found(file), None);
        assert_eq!(found(&format!("{file}USE `shop`;\n")), switch("USE", "shop"));
    }

    #[test]
    fn the_refusal_names_the_database() {
        let refusal = switch("USE", "shop").unwrap().refusal("copy").to_string();
        assert_eq!(
            refusal,
            "This file switches to another database (USE `shop`), so it would not restore into copy. \
             Export it again without --databases, or restore it into `shop`."
        );
        let created = switch("CREATE DATABASE", "shop").unwrap().refusal("copy").to_string();
        assert!(created.starts_with("This file creates another database (CREATE DATABASE `shop`)"), "{created}");
        let unnamed = Switch { command: "connect".into(), database: None }.refusal("copy").to_string();
        assert_eq!(
            unnamed,
            "This file connects to another database (connect), so it would not restore into copy. \
             Export it again without --databases."
        );
    }
}
