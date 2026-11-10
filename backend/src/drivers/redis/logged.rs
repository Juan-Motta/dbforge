//! A Redis connection whose every command is recorded in the statement log,
//! as the redis-cli line that sends it again.
//!
//! The driver holds a [`LoggedConnection`] in place of the crate's own
//! connection, which it can't reach. It is a [`ConnectionLike`], the trait
//! every command and pipeline is sent through, so the driver still writes
//! `redis::cmd(..).query_async(&mut conn)` and `pipe.query_async(&mut conn)`,
//! and nothing it sends can skip the log.
//!
//! What the crate sends while connecting (`HELLO`, `AUTH`, `SELECT`) goes out
//! before the connection is wrapped, so it is never recorded; a password typed
//! in a command (`AUTH`, `ACL SETUSER`, …) is written as `***`.
//!
//! Each entry records:
//! - the database as its number, following `SELECT`;
//! - as rows, the items of an array, set or map reply (the keys of a `SCAN`
//!   page), 1 for any other reply, and none for nil, as redis-cli numbers them;
//! - a pipeline as one entry, its commands one per line (inside `MULTI` …
//!   `EXEC` when it is a transaction): it is one round trip, with one time and
//!   one outcome, and pasted into redis-cli it runs the same commands.

use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::{Arg, Cmd, Pipeline, RedisFuture, ServerError, Value};

use crate::error::redis_message;
use crate::statement_log::Recording;

/// What a password is written as.
const HIDDEN: &[u8] = b"***";

/// A Redis connection whose commands are recorded, with the database it is on.
pub struct LoggedConnection {
    inner: MultiplexedConnection,
    db: i64,
}

impl LoggedConnection {
    /// `inner` connected to database `db`.
    pub(super) fn new(inner: MultiplexedConnection, db: i64) -> Self {
        Self { inner, db }
    }

    fn start(&self, text: impl FnOnce() -> String) -> Recording {
        Recording::start(Some(&self.db.to_string()), text)
    }
}

impl ConnectionLike for LoggedConnection {
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, Value> {
        Box::pin(async move {
            let mut recording = self.start(|| line(cmd));
            let reply = self.inner.req_packed_command(cmd).await;
            match &reply {
                Ok(reply) => match server_error(reply) {
                    Some(error) => recording.fail(redis_message(&error.clone().into())),
                    None => {
                        recording.rows(items(reply));
                        if let Some(db) = selected(cmd) {
                            self.db = db;
                        }
                    }
                },
                Err(error) => recording.fail(redis_message(error)),
            }
            reply
        })
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut recording = self.start(|| script(pipeline));
            let replies = self.inner.req_packed_commands(pipeline, offset, count).await;
            match &replies {
                Ok(replies) => {
                    // A transaction's only reply is EXEC's: its commands' replies, or nil when aborted.
                    let replies = match replies.as_slice() {
                        [Value::Array(each)] if pipeline.is_transaction() => each.as_slice(),
                        other => other,
                    };
                    match replies.iter().find_map(server_error) {
                        Some(error) => recording.fail(redis_message(&error.clone().into())),
                        None => recording.rows(replies.iter().map(items).sum()),
                    }
                }
                Err(error) => recording.fail(redis_message(error)),
            }
            replies
        })
    }

    fn get_db(&self) -> i64 {
        self.db
    }
}

/// The database a successful `SELECT n` moved the connection to.
fn selected(cmd: &Cmd) -> Option<i64> {
    let mut args = cmd.args_iter();
    match (args.next(), args.next(), args.next()) {
        (Some(Arg::Simple(name)), Some(Arg::Simple(db)), None) if name.eq_ignore_ascii_case(b"SELECT") => {
            std::str::from_utf8(db).ok()?.parse().ok()
        }
        _ => None,
    }
}

/// The first error the server answered with, in a reply or the replies of an array.
fn server_error(reply: &Value) -> Option<&ServerError> {
    match reply {
        Value::ServerError(error) => Some(error),
        Value::Array(items) | Value::Set(items) => items.iter().find_map(server_error),
        _ => None,
    }
}

/// The rows a reply counts for: see the module's docs.
fn items(reply: &Value) -> u64 {
    match reply {
        Value::Nil => 0,
        Value::Array(page) if reply.looks_like_cursor() => page.get(1).map_or(0, items),
        Value::Array(items) | Value::Set(items) | Value::Push { data: items, .. } => items.len() as u64,
        Value::Map(pairs) => pairs.len() as u64,
        Value::Attribute { data, .. } => items(data),
        _ => 1,
    }
}

/// A pipeline as redis-cli lines, one command per line.
fn script(pipeline: &Pipeline) -> String {
    let commands = pipeline.cmd_iter().map(line);
    if pipeline.is_transaction() {
        std::iter::once("MULTI".to_string()).chain(commands).chain(["EXEC".to_string()]).collect::<Vec<_>>().join("\n")
    } else {
        commands.collect::<Vec<_>>().join("\n")
    }
}

/// A command as the redis-cli line that sends it, passwords hidden.
fn line(cmd: &Cmd) -> String {
    let mut words: Vec<&[u8]> = cmd
        .args_iter()
        .map(|arg| match arg {
            Arg::Simple(word) => word,
            // Only the crate's own `SCAN` iterators use a cursor placeholder; the driver never does.
            _ => b"0".as_slice(),
        })
        .collect();
    let hidden = hide_passwords(&words);
    for (word, hide) in words.iter_mut().zip(&hidden) {
        if let Some(prefix) = hide {
            *word = prefix;
        }
    }
    words.iter().enumerate().map(|(i, word)| written(word, i == 0)).collect::<Vec<_>>().join(" ")
}

/// For each word of a command, what to write in its place when it is a
/// password: `***`, or for `ACL SETUSER` rules, the rule's sign then `***`.
fn hide_passwords(words: &[&[u8]]) -> Vec<Option<&'static [u8]>> {
    let mut hidden = vec![None; words.len()];
    let is = |i: usize, name: &str| words.get(i).is_some_and(|w| w.eq_ignore_ascii_case(name.as_bytes()));
    let mut hide = |i: usize| {
        if let Some(slot) = hidden.get_mut(i) {
            *slot = Some(HIDDEN);
        }
    };
    if is(0, "AUTH") {
        (1..words.len()).for_each(hide);
    } else if is(0, "HELLO") || is(0, "MIGRATE") {
        // `HELLO 3 AUTH user pass`, `MIGRATE … AUTH pass`, `MIGRATE … AUTH2 user pass`.
        for i in 1..words.len() {
            if is(i, "AUTH") && is(0, "MIGRATE") {
                hide(i + 1);
            } else if is(i, "AUTH") || is(i, "AUTH2") {
                hide(i + 2);
            }
        }
    } else if is(0, "CONFIG") && is(1, "SET") {
        for i in (2..words.len()).step_by(2) {
            let secret = ["requirepass", "masterauth", "tls-key-file-pass", "tls-client-key-file-pass"];
            if secret.iter().any(|name| is(i, name)) {
                hide(i + 1);
            }
        }
    } else if is(0, "ACL") && is(1, "SETUSER") {
        // Rules after the user name: `>pass` and `<pass` are passwords, `#hash` and `!hash` their hashes.
        for (i, word) in words.iter().enumerate().skip(3) {
            hidden[i] = match word.first() {
                Some(b'>') => Some(b">***"),
                Some(b'<') => Some(b"<***"),
                Some(b'#') => Some(b"#***"),
                Some(b'!') => Some(b"!***"),
                _ => None,
            };
        }
    }
    hidden
}

/// One word of a command line: the command's name, numbers and upper-case
/// options (`MATCH`, `WITHSCORES`) as they are; keys, values and anything
/// else in double quotes, with redis-cli's escapes.
fn written(word: &[u8], name: bool) -> String {
    let number = |w: &[u8]| {
        let digits = w.strip_prefix(b"-").unwrap_or(w);
        let (whole, fraction) = match digits.iter().position(|&b| b == b'.') {
            Some(dot) => (&digits[..dot], Some(&digits[dot + 1..])),
            None => (digits, None),
        };
        let all_digits = |part: &[u8]| !part.is_empty() && part.iter().all(u8::is_ascii_digit);
        all_digits(whole) && fraction.is_none_or(all_digits)
    };
    let keyword = |w: &[u8]| {
        w.first().is_some_and(u8::is_ascii_uppercase)
            && w.iter().all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    if (name && !word.is_empty() && word.iter().all(u8::is_ascii_graphic)) || number(word) || keyword(word) {
        String::from_utf8_lossy(word).into_owned()
    } else {
        quote(word)
    }
}

/// Bytes in double quotes, with redis-cli's escapes: `\"`, `\\`, `\n`, `\r`,
/// `\t`, and `\xHH` for other control characters and bytes that aren't UTF-8.
/// Shared by command lines and the replies the console prints.
pub(super) fn quote(bytes: &[u8]) -> String {
    let mut text = String::from("\"");
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => text.push_str("\\\""),
                '\\' => text.push_str("\\\\"),
                '\n' => text.push_str("\\n"),
                '\r' => text.push_str("\\r"),
                '\t' => text.push_str("\\t"),
                c if c.is_control() => text.push_str(&format!("\\x{:02x}", c as u32)),
                c => text.push(c),
            }
        }
        for byte in chunk.invalid() {
            text.push_str(&format!("\\x{byte:02x}"));
        }
    }
    text.push('"');
    text
}

#[cfg(test)]
mod tests {
    use super::super::cli::split;
    use super::super::tests::{PASSWORD, database};
    use super::super::{edit_key, read_key, scan};
    use super::*;
    use crate::drivers::redis::KeyEdit;
    use crate::statement_log::store::tests::Collected;
    use crate::statement_log::{self, Origin, StatementLog};

    fn command(words: &[&[u8]]) -> Cmd {
        let mut cmd = redis::cmd(std::str::from_utf8(words[0]).unwrap());
        cmd.arg(&words[1..]);
        cmd
    }

    #[test]
    fn writes_commands_as_redis_cli_lines() {
        let scan = command(&[b"SCAN", b"0", b"MATCH", b"cart:*", b"COUNT", b"200"]);
        assert_eq!(line(&scan), r#"SCAN 0 MATCH "cart:*" COUNT 200"#);
        assert_eq!(line(&command(&[b"HGETALL", b"user:1"])), r#"HGETALL "user:1""#);
        assert_eq!(line(&command(&[b"SET", b"k", br#"it's "x""#])), r#"SET "k" "it's \"x\"""#);
        assert_eq!(line(&command(&[b"SET", b"a b", b"line\none\ttab\\"])), r#"SET "a b" "line\none\ttab\\""#);
        assert_eq!(line(&command(&[b"SET", b"bin", b"\x01\xff\xfe"])), r#"SET "bin" "\x01\xff\xfe""#);
        assert_eq!(line(&command(&[b"ZADD", b"z", b"-1.5", b"m"])), r#"ZADD "z" -1.5 "m""#);
        assert_eq!(line(&command(&[b"SET", b"k", b""])), r#"SET "k" """#);
        // Lower-case words and signs are quoted: they read as values, and still paste the same.
        assert_eq!(line(&command(&[b"XRANGE", b"st", b"-", b"+"])), r#"XRANGE "st" "-" "+""#);
        assert_eq!(line(&command(&[b"CONFIG", b"GET", b"databases"])), r#"CONFIG GET "databases""#);
    }

    #[test]
    fn a_written_line_reads_back_as_the_same_words() {
        let words: [&[u8]; 4] = [b"SET", b"k \"q\" 'a'", b"\x00\x7f\xff\n", "ünï".as_bytes()];
        let written = line(&command(&words));
        assert_eq!(split(&written).unwrap(), words.map(<[u8]>::to_vec));
    }

    #[test]
    fn never_writes_a_password() {
        assert_eq!(line(&command(&[b"AUTH", b"s3cret"])), r#"AUTH "***""#);
        assert_eq!(line(&command(&[b"AUTH", b"admin", b"s3cret"])), r#"AUTH "***" "***""#);
        assert_eq!(line(&command(&[b"HELLO", b"3", b"AUTH", b"admin", b"s3cret"])), r#"HELLO 3 AUTH "admin" "***""#);
        assert_eq!(
            line(&command(&[b"MIGRATE", b"h", b"6379", b"", b"0", b"5000", b"AUTH", b"s3cret", b"KEYS", b"a"])),
            r#"MIGRATE "h" 6379 "" 0 5000 AUTH "***" KEYS "a""#
        );
        assert_eq!(
            line(&command(&[b"MIGRATE", b"h", b"6379", b"k", b"0", b"5000", b"AUTH2", b"u", b"s3cret"])),
            r#"MIGRATE "h" 6379 "k" 0 5000 AUTH2 "u" "***""#
        );
        assert_eq!(
            line(&command(&[b"ACL", b"SETUSER", b"bob", b"on", b">s3cret", b"<old", b"#abc", b"~*"])),
            r##"ACL SETUSER "bob" "on" ">***" "<***" "#***" "~*""##
        );
        assert_eq!(
            line(&command(&[b"CONFIG", b"SET", b"maxmemory", b"1", b"requirepass", b"s3cret"])),
            r#"CONFIG SET "maxmemory" 1 "requirepass" "***""#
        );
    }

    #[test]
    fn writes_a_pipeline_one_command_per_line_and_a_transaction_inside_multi() {
        let mut pipe = redis::pipe();
        pipe.cmd("TYPE").arg("a").cmd("PTTL").arg("a");
        assert_eq!(script(&pipe), "TYPE \"a\"\nPTTL \"a\"");
        pipe.atomic();
        assert_eq!(script(&pipe), "MULTI\nTYPE \"a\"\nPTTL \"a\"\nEXEC");
    }

    #[test]
    fn counts_rows_as_redis_cli_numbers_them() {
        let bulk = |s: &str| Value::BulkString(s.as_bytes().to_vec());
        assert_eq!(items(&Value::Nil), 0);
        assert_eq!(items(&Value::Okay), 1);
        assert_eq!(items(&Value::Int(7)), 1);
        assert_eq!(items(&Value::Array(vec![bulk("a"), bulk("b"), bulk("c")])), 3);
        // A SCAN page counts its keys, not its cursor.
        let page = Value::Array(vec![bulk("17"), Value::Array(vec![bulk("k1"), bulk("k2")])]);
        assert_eq!(items(&page), 2);
        assert_eq!(items(&Value::Map(vec![(bulk("f"), bulk("v"))])), 1);
    }

    #[test]
    fn follows_select() {
        assert_eq!(selected(&command(&[b"SELECT", b"8"])), Some(8));
        assert_eq!(selected(&command(&[b"select", b"2"])), Some(2));
        assert_eq!(selected(&command(&[b"SELECT", b"x"])), None);
        assert_eq!(selected(&command(&[b"GET", b"8"])), None);
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn records_reads_and_edits_as_redis_cli_lines() {
        let (connection, _) = database(11, &[&["SET", "cart:1", "x"], &["SET", "k", "old"]]).await;
        let log = StatementLog::new(50, Collected::default());

        statement_log::scoped(&log, "redis", Origin::Edit, async {
            let edit = KeyEdit::SetString { value: r#"it's "x""#.into() };
            edit_key(&connection, PASSWORD, 11, "k", "string", &edit).await.unwrap();
            scan(&connection, PASSWORD, 11, "cart:*", None, "0", 10).await.unwrap();
            read_key(&connection, PASSWORD, 11, "nothing", None, "", 10).await.unwrap_err();
            edit_key(&connection, PASSWORD, 11, "cart:1", "hash", &edit).await.unwrap_err();
        })
        .await;

        let entries = log.entries();
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                r#"TYPE "k""#,
                r#"SET "k" "it's \"x\"" KEEPTTL"#,
                r#"SCAN 0 MATCH "cart:*" COUNT 1000"#,
                "TYPE \"cart:1\"\nPTTL \"cart:1\"",
                "TYPE \"nothing\"\nPTTL \"nothing\"",
                r#"TYPE "cart:1""#,
            ]
        );
        let rows: Vec<Option<u64>> = entries.iter().map(|e| e.rows).collect();
        assert_eq!(rows, [Some(1), Some(1), Some(1), Some(2), Some(2), Some(1)]);
        assert!(entries.iter().all(|e| {
            (e.connection_id.as_str(), e.database.as_deref(), e.origin, e.error.as_deref())
                == ("redis", Some("11"), Origin::Edit, None)
        }));
        // Connecting signed in with the password; neither AUTH nor the password was recorded.
        let password = PASSWORD.unwrap();
        assert!(entries.iter().all(|e| !e.text.contains("AUTH") && !e.text.contains(password)));
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn records_console_lines_with_errors_and_the_database_selected() {
        database(13, &[]).await;
        let (connection, _) = database(12, &[]).await;
        let mut session = super::super::open_session(&connection, PASSWORD, None).await.unwrap();
        let log = StatementLog::new(50, Collected::default());

        statement_log::scoped(&log, "redis", Origin::Console, async {
            session.run("SET greeting \"hello you\"\nINCR greeting\nSELECT 13\nGET greeting", 100).await.unwrap();
        })
        .await;

        let entries = log.entries();
        let summary: Vec<_> =
            entries.iter().map(|e| (e.text.as_str(), e.database.as_deref(), e.rows, e.error.as_deref())).collect();
        assert_eq!(
            summary,
            [
                (r#"SET "greeting" "hello you""#, Some("12"), Some(1), None),
                (r#"INCR "greeting""#, Some("12"), None, Some("ERR value is not an integer or out of range")),
                ("SELECT 13", Some("12"), Some(1), None),
                (r#"GET "greeting""#, Some("13"), None, None),
            ]
        );
    }
}
