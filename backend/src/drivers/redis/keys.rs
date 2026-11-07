//! The key browser: keys found with `SCAN`, each with its type and time to
//! live, and the value of one key read a page at a time. Keys and values are
//! bytes; they are shown as text when they are UTF-8, else as hex.

use redis::aio::MultiplexedConnection;
use serde::Serialize;

use super::browse;
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// `SCAN` rounds per page: a pattern that matches few keys of a large database
/// returns a short page rather than reading the whole keyspace at once.
const MAX_SCAN_ROUNDS: usize = 50;
/// Keys asked of the server per `SCAN` round.
const SCAN_COUNT: usize = 1000;
/// The most of a string value read.
const MAX_STRING_BYTES: i64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub name: String,
    /// `string`, `hash`, `list`, `set`, `zset`, `stream`, or a module's type.
    pub kind: String,
    /// Milliseconds left; `None` when the key never expires.
    pub ttl_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanPage {
    pub keys: Vec<KeyInfo>,
    /// Where the next page starts; `"0"` when the scan is complete.
    pub cursor: String,
}

/// Up to about `limit` keys matching `pattern` (and `kind`, when given),
/// starting at `cursor` (`"0"` for the first page).
#[allow(clippy::too_many_arguments)]
pub async fn scan(
    connection: &Connection,
    password: Option<&str>,
    db: i64,
    pattern: &str,
    kind: Option<&str>,
    cursor: &str,
    limit: usize,
) -> Result<ScanPage> {
    let mut conn = browse(connection, password, Some(db)).await?;
    let pattern = if pattern.is_empty() { "*" } else { pattern };
    let mut cursor = cursor.to_string();
    let mut names: Vec<Vec<u8>> = Vec::new();
    for _ in 0..MAX_SCAN_ROUNDS {
        let mut command = redis::cmd("SCAN");
        command.arg(&cursor).arg("MATCH").arg(pattern).arg("COUNT").arg(SCAN_COUNT);
        if let Some(kind) = kind {
            command.arg("TYPE").arg(kind);
        }
        let (next, found): (String, Vec<Vec<u8>>) = command.query_async(&mut conn).await?;
        names.extend(found);
        cursor = next;
        if cursor == "0" || names.len() >= limit {
            break;
        }
    }

    let mut pipe = redis::pipe();
    for name in &names {
        pipe.cmd("TYPE").arg(name).cmd("PTTL").arg(name);
    }
    let details: Vec<(String, i64)> = if names.is_empty() { Vec::new() } else { pipe.query_async(&mut conn).await? };
    let mut keys: Vec<KeyInfo> = names
        .iter()
        .zip(details)
        // `none` / -2: deleted between SCAN and TYPE.
        .filter(|(_, (kind, ttl))| kind != "none" && *ttl != -2)
        .map(|(name, (kind, ttl))| KeyInfo { name: text(name).text, kind, ttl_ms: (ttl >= 0).then_some(ttl) })
        .collect();
    keys.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ScanPage { keys, cursor })
}

/// Bytes for display: the text when it is UTF-8, else lowercase hex.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub text: String,
    pub binary: bool,
}

pub fn text(bytes: &[u8]) -> Text {
    match std::str::from_utf8(bytes) {
        Ok(text) => Text { text: text.to_string(), binary: false },
        Err(_) => Text { text: bytes.iter().map(|b| format!("{b:02x}")).collect(), binary: true },
    }
}

/// One element of a value: a hash field, list item, set member, sorted-set
/// member with its score, stream entry, or a string's whole value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The hash field, list index or stream entry id; `None` for sets and strings.
    pub key: Option<Text>,
    /// A stream entry's fields are written as a JSON object.
    pub value: Text,
    pub score: Option<f64>,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValue {
    pub name: String,
    pub kind: String,
    pub ttl_ms: Option<i64>,
    /// Fields, items, members or entries; a string's length in bytes.
    pub length: i64,
    /// `MEMORY USAGE`, where the server allows it.
    pub bytes: Option<i64>,
    /// `OBJECT ENCODING`, e.g. `listpack`.
    pub encoding: Option<String>,
    pub entries: Vec<Entry>,
    /// Pass back to read the next page; `None` after the last one.
    pub next: Option<String>,
}

/// One page of `key`'s value from `cursor` (`None` for the first page).
/// `filter` is a glob over fields or members (hashes, sets and sorted sets).
#[allow(clippy::too_many_arguments)]
pub async fn read_key(
    connection: &Connection,
    password: Option<&str>,
    db: i64,
    key: &str,
    cursor: Option<&str>,
    filter: &str,
    limit: usize,
) -> Result<KeyValue> {
    let mut conn = browse(connection, password, Some(db)).await?;
    let (kind, ttl): (String, i64) =
        redis::pipe().cmd("TYPE").arg(key).cmd("PTTL").arg(key).query_async(&mut conn).await?;
    if kind == "none" {
        return Err(AppError::Invalid(format!("The key {key} no longer exists.")));
    }
    let bytes: Option<i64> = redis::cmd("MEMORY").arg("USAGE").arg(key).query_async(&mut conn).await.ok();
    let encoding: Option<String> = redis::cmd("OBJECT").arg("ENCODING").arg(key).query_async(&mut conn).await.ok();
    let pattern = if filter.is_empty() { "*".to_string() } else { format!("*{filter}*") };
    let offset: i64 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);

    let (length, entries, next) = match kind.as_str() {
        "string" => {
            let length: i64 = redis::cmd("STRLEN").arg(key).query_async(&mut conn).await?;
            let value: Vec<u8> =
                redis::cmd("GETRANGE").arg(key).arg(0).arg(MAX_STRING_BYTES - 1).query_async(&mut conn).await?;
            (length, vec![Entry { key: None, size: length as usize, value: text(&value), score: None }], None)
        }
        "hash" => {
            let length = count(&mut conn, "HLEN", key).await?;
            let (pairs, next) = scan_value(&mut conn, "HSCAN", key, cursor, &pattern, limit).await?;
            let entries = pairs
                .chunks(2)
                .filter_map(|pair| match pair {
                    [field, value] => {
                        Some(Entry { key: Some(text(field)), size: value.len(), value: text(value), score: None })
                    }
                    _ => None,
                })
                .collect();
            (length, entries, next)
        }
        "set" => {
            let length = count(&mut conn, "SCARD", key).await?;
            let (members, next) = scan_value(&mut conn, "SSCAN", key, cursor, &pattern, limit).await?;
            let mut entries: Vec<Entry> =
                members.iter().map(|m| Entry { key: None, size: m.len(), value: text(m), score: None }).collect();
            entries.sort_by(|a, b| a.value.text.cmp(&b.value.text));
            (length, entries, next)
        }
        "zset" if filter.is_empty() => {
            let length = count(&mut conn, "ZCARD", key).await?;
            let pairs: Vec<(Vec<u8>, f64)> = redis::cmd("ZRANGE")
                .arg(key)
                .arg(offset)
                .arg(offset + limit as i64 - 1)
                .arg("WITHSCORES")
                .query_async(&mut conn)
                .await?;
            let next = (offset + (pairs.len() as i64) < length && pairs.len() == limit)
                .then(|| (offset + limit as i64).to_string());
            (length, pairs.iter().map(|(m, score)| member(m, *score)).collect(), next)
        }
        "zset" => {
            let length = count(&mut conn, "ZCARD", key).await?;
            let (pairs, next) = scan_value(&mut conn, "ZSCAN", key, cursor, &pattern, limit).await?;
            let mut entries: Vec<Entry> = pairs
                .chunks(2)
                .filter_map(|pair| match pair {
                    [m, score] => Some(member(m, std::str::from_utf8(score).ok()?.parse().ok()?)),
                    _ => None,
                })
                .collect();
            entries.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
            (length, entries, next)
        }
        "list" => {
            let length = count(&mut conn, "LLEN", key).await?;
            let items: Vec<Vec<u8>> =
                redis::cmd("LRANGE").arg(key).arg(offset).arg(offset + limit as i64 - 1).query_async(&mut conn).await?;
            let entries = items
                .iter()
                .enumerate()
                .map(|(i, item)| Entry {
                    key: Some(Text { text: (offset + i as i64).to_string(), binary: false }),
                    size: item.len(),
                    value: text(item),
                    score: None,
                })
                .collect();
            let end = offset + items.len() as i64;
            (length, entries, (end < length).then(|| end.to_string()))
        }
        "stream" => {
            let length = count(&mut conn, "XLEN", key).await?;
            let start = cursor.unwrap_or("-");
            let found: Vec<(String, Vec<Vec<u8>>)> = redis::cmd("XRANGE")
                .arg(key)
                .arg(start)
                .arg("+")
                .arg("COUNT")
                .arg(limit)
                .query_async(&mut conn)
                .await?;
            let next = (found.len() == limit).then(|| found.last().map(|(id, _)| format!("({id}")).unwrap_or_default());
            let entries = found
                .iter()
                .map(|(id, fields)| {
                    let object: serde_json::Map<String, serde_json::Value> = fields
                        .chunks(2)
                        .filter_map(|pair| match pair {
                            [field, value] => Some((text(field).text, text(value).text.into())),
                            _ => None,
                        })
                        .collect();
                    let json = serde_json::Value::Object(object).to_string();
                    Entry {
                        key: Some(Text { text: id.clone(), binary: false }),
                        size: fields.iter().map(Vec::len).sum(),
                        value: Text { text: json, binary: false },
                        score: None,
                    }
                })
                .collect();
            (length, entries, next)
        }
        // A module's type (JSON, time series, …): shown by name only.
        _ => (0, Vec::new(), None),
    };
    Ok(KeyValue {
        name: key.to_string(),
        kind,
        ttl_ms: (ttl >= 0).then_some(ttl),
        length,
        bytes,
        encoding,
        entries,
        next,
    })
}

fn member(name: &[u8], score: f64) -> Entry {
    Entry { key: None, size: name.len(), value: text(name), score: Some(score) }
}

async fn count(conn: &mut MultiplexedConnection, command: &str, key: &str) -> Result<i64> {
    Ok(redis::cmd(command).arg(key).query_async(conn).await?)
}

/// `HSCAN`/`SSCAN`/`ZSCAN` rounds until about `limit` items are read; the
/// reply's flat list of items and the cursor to continue from.
async fn scan_value(
    conn: &mut MultiplexedConnection,
    command: &str,
    key: &str,
    cursor: Option<&str>,
    pattern: &str,
    limit: usize,
) -> Result<(Vec<Vec<u8>>, Option<String>)> {
    // Hashes and sorted sets reply with field/value pairs.
    let per_item = if command == "SSCAN" { 1 } else { 2 };
    let mut cursor = cursor.unwrap_or("0").to_string();
    let mut items = Vec::new();
    for _ in 0..MAX_SCAN_ROUNDS {
        let (next, found): (String, Vec<Vec<u8>>) = redis::cmd(command)
            .arg(key)
            .arg(&cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(limit)
            .query_async(&mut *conn)
            .await?;
        items.extend(found);
        cursor = next;
        if cursor == "0" || items.len() / per_item >= limit {
            break;
        }
    }
    Ok((items, (cursor != "0").then_some(cursor)))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, database};
    use super::*;

    fn values(value: &KeyValue) -> Vec<(Option<&str>, &str, Option<f64>)> {
        value
            .entries
            .iter()
            .map(|e| (e.key.as_ref().map(|k| k.text.as_str()), e.value.text.as_str(), e.score))
            .collect()
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn scans_keys_by_pattern_and_type_with_their_ttl() {
        let mut setup: Vec<Vec<String>> =
            (0..30).map(|i| vec!["SET".into(), format!("session:{i:02}"), "x".into()]).collect();
        setup.push(vec!["HSET".into(), "session:hash".into(), "a".into(), "1".into()]);
        setup.push(vec!["EXPIRE".into(), "session:hash".into(), "600".into()]);
        setup.push(vec!["SET".into(), "cart:1".into(), "y".into()]);
        let setup: Vec<Vec<&str>> = setup.iter().map(|c| c.iter().map(String::as_str).collect()).collect();
        let setup: Vec<&[&str]> = setup.iter().map(Vec::as_slice).collect();
        let (connection, _) = database(2, &setup).await;

        let mut found = Vec::new();
        let mut cursor = "0".to_string();
        loop {
            let page = scan(&connection, PASSWORD, 2, "session:*", None, &cursor, 10).await.unwrap();
            found.extend(page.keys);
            cursor = page.cursor;
            if cursor == "0" {
                break;
            }
        }
        assert_eq!(found.len(), 31);
        let hash = found.iter().find(|k| k.name == "session:hash").unwrap();
        assert_eq!(hash.kind, "hash");
        assert!(hash.ttl_ms.is_some_and(|ttl| ttl > 590_000), "{hash:?}");
        assert!(found.iter().filter(|k| k.kind == "string").all(|k| k.ttl_ms.is_none()));

        let hashes = scan(&connection, PASSWORD, 2, "", Some("hash"), "0", 100).await.unwrap();
        assert_eq!((hashes.keys.len(), hashes.cursor.as_str()), (1, "0"));
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn reads_each_type_a_page_at_a_time() {
        let (connection, _) = database(
            3,
            &[
                &["SET", "s", "hello"],
                &["HSET", "h", "user_id", "u_1", "plan", "pro", "ip", "1.2.3.4"],
                &["RPUSH", "l", "a", "b", "c", "d", "e"],
                &["SADD", "set", "x", "y"],
                &["ZADD", "z", "3", "c", "1", "a", "2", "b"],
                &["XADD", "st", "1-1", "event", "login"],
                &["XADD", "st", "2-1", "event", "logout"],
                &["SET", "bin", "\u{1}"],
            ],
        )
        .await;
        let read = |key: &'static str, cursor: Option<&'static str>, filter: &'static str, limit| {
            let connection = connection.clone();
            async move { read_key(&connection, PASSWORD, 3, key, cursor, filter, limit).await.unwrap() }
        };

        let string = read("s", None, "", 100).await;
        assert_eq!((string.kind.as_str(), string.length, values(&string)), ("string", 5, vec![(None, "hello", None)]));
        assert!(string.bytes.is_some() && string.encoding.is_some());

        let hash = read("h", None, "", 100).await;
        assert_eq!(hash.length, 3);
        assert!(values(&hash).contains(&(Some("plan"), "pro", None)));
        let filtered = read("h", None, "user", 100).await;
        assert_eq!(values(&filtered), [(Some("user_id"), "u_1", None)]);

        let list = read("l", None, "", 2).await;
        assert_eq!(values(&list), [(Some("0"), "a", None), (Some("1"), "b", None)]);
        let rest = read("l", Some("4"), "", 2).await;
        assert_eq!((values(&rest), rest.next.clone()), (vec![(Some("4"), "e", None)], None));
        assert_eq!(list.next.as_deref(), Some("2"));

        assert_eq!(values(&read("set", None, "", 100).await), [(None, "x", None), (None, "y", None)]);
        let zset = read("z", None, "", 2).await;
        assert_eq!(values(&zset), [(None, "a", Some(1.0)), (None, "b", Some(2.0))]);
        assert_eq!(zset.next.as_deref(), Some("2"));

        let stream = read("st", None, "", 1).await;
        assert_eq!(values(&stream), [(Some("1-1"), r#"{"event":"login"}"#, None)]);
        let next = stream.next.clone().unwrap();
        let more = read_key(&connection, PASSWORD, 3, "st", Some(&next), "", 1).await.unwrap();
        assert_eq!(values(&more)[0].0, Some("2-1"));

        let binary = read("bin", None, "", 1).await;
        assert_eq!(binary.entries[0].value, Text { text: "\u{1}".into(), binary: false });

        let error = read_key(&connection, PASSWORD, 3, "gone", None, "", 10).await.unwrap_err();
        assert_eq!(error.to_string(), "The key gone no longer exists.");
    }

    #[test]
    fn bytes_that_are_not_utf8_show_as_hex() {
        assert_eq!(text(&[0xff, 0x01]), Text { text: "ff01".into(), binary: true });
    }
}
