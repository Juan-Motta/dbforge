//! One redis-cli line that may only read, for callers that must not change data.
//!
//! Redis has no read-only connection, but it says what each command does:
//! `COMMAND INFO` lists a command's flags, and only commands the server flags
//! `readonly` run here. Of those, the ones also flagged `admin`, `blocking`
//! (which wait for data, e.g. `XREAD`, even without `BLOCK`) or `pubsub` are
//! refused, as are the commands that turn the connection into a stream. For a
//! command with subcommands (`OBJECT ENCODING`, `CONFIG GET`), the subcommand's
//! own flags decide, where the server reports them (Redis 7 and later).
//!
//! `EVAL_RO` and `FCALL_RO` are flagged `readonly`: the server runs their
//! scripts in read-only mode, refusing any write they attempt.

use std::time::Instant;

use redis::Value;

use super::browse;
use super::cli::{STREAMING, run_command, split};
use super::logged::LoggedConnection;
use crate::connections::model::Connection;
use crate::drivers::QueryOutcome;
use crate::drivers::with_timeout;
use crate::error::{AppError, Result};

/// Flags that refuse a command even when it is `readonly`.
const REFUSED_FLAGS: [&str; 3] = ["admin", "blocking", "pubsub"];

/// Where `COMMAND INFO` puts a command's flags and its subcommands.
const FLAGS: usize = 2;
const SUBCOMMANDS: usize = 9;

/// Checks the command with the server, then runs it on a new connection that
/// is closed afterwards. Replies longer than `row_limit` lines are cut.
pub async fn run_read_only(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    statement: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    let words = read_line(statement)?;
    let started = Instant::now();
    let db = database.map(super::database_number).transpose()?;
    let mut conn = with_timeout(browse(connection, password, db)).await?;
    let (name, flags) = command_flags(&mut conn, &words).await?;
    check_flags(&name, &flags)?;
    let result = run_command(&mut conn, &words, row_limit).await?;
    Ok(QueryOutcome::complete(vec![result], started))
}

/// The words of the one command in `statement`.
fn read_line(statement: &str) -> Result<Vec<Vec<u8>>> {
    let mut lines = statement.lines().filter(|line| !line.trim().is_empty());
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Err(AppError::Invalid("Read-only queries run one command at a time.".into()));
    };
    split(line).map_err(AppError::Invalid)
}

/// The command's name as the server knows it (`config|get` for a subcommand)
/// and its flags.
async fn command_flags(conn: &mut LoggedConnection, words: &[Vec<u8>]) -> Result<(String, Vec<String>)> {
    let name = String::from_utf8_lossy(&words[0]).to_ascii_lowercase();
    let info = command_info(conn, &name).await?;
    let has_subcommands =
        matches!(info.as_ref().and_then(|info| info.get(SUBCOMMANDS)), Some(Value::Array(items)) if !items.is_empty());
    let (name, info) = match words.get(1) {
        Some(subcommand) if has_subcommands => {
            let full = format!("{name}|{}", String::from_utf8_lossy(subcommand).to_ascii_lowercase());
            let info = command_info(conn, &full).await?;
            (full, info)
        }
        _ => (name, info),
    };
    let info = info.ok_or_else(|| AppError::Invalid(format!("Redis doesn't know the command `{name}`.")))?;
    Ok((name, info.get(FLAGS).map(strings).unwrap_or_default()))
}

/// `COMMAND INFO name`'s description of the command, `None` when the server
/// doesn't know it.
async fn command_info(conn: &mut LoggedConnection, name: &str) -> Result<Option<Vec<Value>>> {
    let reply: Value = redis::cmd("COMMAND").arg("INFO").arg(name).query_async(conn).await.map_err(|error| {
        if error.code().is_some() {
            AppError::Invalid(format!(
                "This server doesn't answer COMMAND INFO ({}), so read-only commands can't be checked.",
                AppError::from(error)
            ))
        } else {
            error.into()
        }
    })?;
    Ok(match reply {
        Value::Array(mut commands) if commands.len() == 1 => match commands.remove(0) {
            Value::Array(info) => Some(info),
            _ => None,
        },
        _ => None,
    })
}

/// The text items of a flags reply, lowercased.
fn strings(value: &Value) -> Vec<String> {
    let items = match value {
        Value::Array(items) | Value::Set(items) => items.as_slice(),
        _ => &[],
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::SimpleString(text) => Some(text.to_ascii_lowercase()),
            Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).to_ascii_lowercase()),
            Value::Okay => Some("ok".into()),
            _ => None,
        })
        .collect()
}

/// Allows a command flagged `readonly` and none of [`REFUSED_FLAGS`].
fn check_flags(name: &str, flags: &[String]) -> Result<()> {
    let shown = name.replace('|', " ").to_ascii_uppercase();
    let streaming = STREAMING.contains(&shown.as_str());
    if !flags.iter().any(|flag| flag == "readonly") || streaming {
        return Err(AppError::Invalid(format!(
            "{shown} isn't allowed in a read-only query: only commands that read are."
        )));
    }
    if let Some(flag) = flags.iter().find(|flag| REFUSED_FLAGS.contains(&flag.as_str())) {
        return Err(AppError::Invalid(format!("{shown} isn't allowed in a read-only query: Redis flags it `{flag}`.")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, database};
    use super::*;

    fn flags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn allows_readonly_commands_without_refused_flags() {
        assert!(check_flags("get", &flags(&["readonly", "fast"])).is_ok());
        assert!(check_flags("object|encoding", &flags(&["readonly"])).is_ok());
        assert_eq!(
            check_flags("set", &flags(&["write", "denyoom"])).unwrap_err().to_string(),
            "SET isn't allowed in a read-only query: only commands that read are."
        );
        assert_eq!(
            check_flags("xread", &flags(&["readonly", "blocking"])).unwrap_err().to_string(),
            "XREAD isn't allowed in a read-only query: Redis flags it `blocking`."
        );
        assert!(check_flags("config|get", &flags(&["admin", "noscript"])).is_err());
        assert!(check_flags("monitor", &flags(&["readonly"])).is_err());
        assert!(check_flags("unknown", &[]).is_err());
    }

    #[test]
    fn reads_one_line() {
        assert_eq!(read_line("  GET \"a b\"  \n\n").unwrap(), [b"GET".to_vec(), b"a b".to_vec()]);
        assert!(read_line("GET a\nDEL a").is_err());
        assert!(read_line("GET \"a").is_err());
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn read_only_reads_and_refuses_every_write() {
        let (connection, mut conn) =
            database(13, &[&["SET", "greeting", "hi"], &["RPUSH", "list", "a", "b", "c"]]).await;
        let outcome = run_read_only(&connection, PASSWORD, None, "GET greeting", 10).await.unwrap();
        assert_eq!(outcome.results[0].rows, [[Some("\"hi\"".into())]]);
        let range = run_read_only(&connection, PASSWORD, None, "LRANGE list 0 -1", 2).await.unwrap();
        assert_eq!(range.results[0].rows, [[Some("1) \"a\"".into())], [Some("2) \"b\"".into())]]);
        assert!(range.results[0].truncated);
        let encoding = run_read_only(&connection, PASSWORD, None, "OBJECT ENCODING list", 10).await.unwrap();
        assert_eq!(encoding.results[0].rows.len(), 1);
        let other = run_read_only(&super::super::tests::local(), PASSWORD, Some("13"), "DBSIZE", 10).await;
        assert_eq!(other.unwrap().results[0].rows, [[Some("(integer) 2".into())]]);

        for attempt in [
            "SET greeting bye",
            "DEL greeting",
            "FLUSHDB",
            "RPOP list",
            "EXPIRE greeting 1",
            "CONFIG SET maxmemory 1",
            "CONFIG GET maxmemory",
            "BLPOP list 1",
            "XREAD STREAMS s 0",
            "SUBSCRIBE news",
            "MULTI",
            "EVAL \"return redis.call('DEL', 'greeting')\" 0",
            "GET greeting\nDEL greeting",
            "NOSUCHCOMMAND x",
        ] {
            assert!(run_read_only(&connection, PASSWORD, None, attempt, 10).await.is_err(), "ran: {attempt}");
        }
        // EVAL_RO is readonly: the server refuses the write inside it.
        let script =
            run_read_only(&connection, PASSWORD, None, "EVAL_RO \"return redis.call('DEL', KEYS[1])\" 1 greeting", 10);
        assert!(script.await.unwrap().results[0].rows[0][0].as_deref().unwrap().starts_with("(error)"));

        let greeting: String = redis::cmd("GET").arg("greeting").query_async(&mut conn).await.unwrap();
        let length: i64 = redis::cmd("LLEN").arg("list").query_async(&mut conn).await.unwrap();
        assert_eq!((greeting.as_str(), length), ("hi", 3));
    }
}
