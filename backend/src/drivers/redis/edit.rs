//! Edits from the key browser. Each edit checks that the key still has the
//! type the browser showed, so an edit never lands on a key that was replaced
//! by another of a different type. List items are addressed by position, so
//! they are also checked against the value shown, under `WATCH`: if the list
//! changes in between, nothing is applied.
//!
//! What names something that exists (the key, a field, an item, a member) is a
//! [`Text`], so one that isn't UTF-8 is reached by its bytes; new values are text.

use serde::Deserialize;

use super::browse;
use super::keys::Text;
use super::logged::LoggedConnection;
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// One change to an existing key. Values are text.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum KeyEdit {
    /// A string's whole value.
    SetString {
        value: String,
    },
    /// Adds or changes a hash field.
    SetField {
        field: Text,
        value: String,
    },
    RemoveField {
        field: Text,
    },
    /// Changes the list item at `index`, which must still read `old`.
    SetItem {
        index: i64,
        old: Text,
        value: String,
    },
    /// Adds an item at the head or the tail of a list.
    PushItem {
        value: String,
        head: bool,
    },
    /// Removes the list item at `index`, which must still read `old`.
    RemoveItem {
        index: i64,
        old: Text,
    },
    /// Adds a set member, or a sorted-set member with its score (or changes the score).
    AddMember {
        member: Text,
        score: Option<f64>,
    },
    RemoveMember {
        member: Text,
    },
    /// Appends a stream entry with an id the server assigns.
    AddEntry {
        fields: Vec<(String, String)>,
    },
    RemoveEntry {
        id: String,
    },
    /// Sets the time to live; `None` makes the key persistent.
    Expire {
        ttl_ms: Option<i64>,
    },
    /// Renames the key; refused when `to` already exists.
    Rename {
        to: String,
    },
    Delete,
}

/// A key to create. Redis has no empty hashes, lists, sets or streams, so a
/// new key holds its first element: `field`/`value` for hashes and streams,
/// `value` with `score` for sorted sets, `value` for the rest.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKey {
    pub name: String,
    pub kind: String,
    pub field: Option<String>,
    pub value: String,
    pub score: Option<f64>,
    pub ttl_ms: Option<i64>,
}

/// The browser shows the type of `key`; edits are applied only while it still has it.
pub async fn edit_key(
    connection: &Connection,
    password: Option<&str>,
    db: i64,
    key: &Text,
    expected_kind: &str,
    edit: &KeyEdit,
) -> Result<()> {
    let shown = key.text.as_str();
    let key = key.bytes()?;
    let key = key.as_slice();
    let mut conn = browse(connection, password, Some(db)).await?;
    let kind: String = redis::cmd("TYPE").arg(key).query_async(&mut conn).await?;
    if kind == "none" {
        return Err(AppError::Invalid(format!("The key {shown} no longer exists.")));
    }
    if kind != expected_kind {
        return Err(AppError::Invalid(format!("The key {shown} is now a {kind}, not a {expected_kind}. Reload it.")));
    }
    // An element that is gone was removed elsewhere: said, not passed over.
    let removed = |count: i64, what: &Text| match count {
        0 => Err(AppError::Invalid(format!("{} is no longer in {shown}. Reload it.", what.text))),
        _ => Ok(()),
    };
    let wrong = |what: &str| AppError::Invalid(format!("A {kind} has no {what}."));
    match edit {
        KeyEdit::SetString { value } if kind == "string" => {
            // KEEPTTL: changing the value keeps the expiry, as editing in place should.
            redis::cmd("SET").arg(key).arg(value).arg("KEEPTTL").query_async::<()>(&mut conn).await?;
        }
        KeyEdit::SetField { field, value } if kind == "hash" => {
            redis::cmd("HSET").arg(key).arg(field.bytes()?).arg(value).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::RemoveField { field } if kind == "hash" => {
            last_element(&mut conn, key, "HLEN").await?;
            removed(redis::cmd("HDEL").arg(key).arg(field.bytes()?).query_async(&mut conn).await?, field)?;
        }
        KeyEdit::SetItem { index, old, value } if kind == "list" => {
            guarded_item(&mut conn, key, shown, *index, old, |pipe| {
                pipe.cmd("LSET").arg(key).arg(*index).arg(value);
            })
            .await?;
        }
        KeyEdit::PushItem { value, head } if kind == "list" => {
            let command = if *head { "LPUSH" } else { "RPUSH" };
            redis::cmd(command).arg(key).arg(value).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::RemoveItem { index, old } if kind == "list" => {
            last_element(&mut conn, key, "LLEN").await?;
            // Lists can only drop items by value: the item is first marked
            // with a value nothing else holds, then that value is removed.
            let marker = format!("__dbforge_removed_{}", uuid::Uuid::new_v4());
            guarded_item(&mut conn, key, shown, *index, old, |pipe| {
                pipe.cmd("LSET").arg(key).arg(*index).arg(&marker).cmd("LREM").arg(key).arg(1).arg(&marker);
            })
            .await?;
        }
        KeyEdit::AddMember { member, score: None } if kind == "set" => {
            redis::cmd("SADD").arg(key).arg(member.bytes()?).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::AddMember { member, score: Some(score) } if kind == "zset" => {
            redis::cmd("ZADD").arg(key).arg(*score).arg(member.bytes()?).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::AddMember { .. } => {
            return Err(AppError::Invalid(if kind == "zset" {
                "A sorted-set member needs a score.".into()
            } else {
                format!("A {kind} has no members.")
            }));
        }
        KeyEdit::RemoveMember { member } if kind == "set" || kind == "zset" => {
            last_element(&mut conn, key, if kind == "set" { "SCARD" } else { "ZCARD" }).await?;
            let command = if kind == "set" { "SREM" } else { "ZREM" };
            removed(redis::cmd(command).arg(key).arg(member.bytes()?).query_async(&mut conn).await?, member)?;
        }
        KeyEdit::AddEntry { fields } if kind == "stream" => {
            if fields.is_empty() {
                return Err(AppError::Invalid("A stream entry needs at least one field.".into()));
            }
            let mut command = redis::cmd("XADD");
            command.arg(key).arg("*");
            for (field, value) in fields {
                command.arg(field).arg(value);
            }
            command.query_async::<()>(&mut conn).await?;
        }
        KeyEdit::RemoveEntry { id } if kind == "stream" => {
            redis::cmd("XDEL").arg(key).arg(id).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::Expire { ttl_ms: Some(ttl) } => {
            if *ttl <= 0 {
                return Err(AppError::Invalid("A time to live must be positive. Use Delete to remove the key.".into()));
            }
            redis::cmd("PEXPIRE").arg(key).arg(*ttl).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::Expire { ttl_ms: None } => {
            redis::cmd("PERSIST").arg(key).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::Rename { to } => {
            if to.is_empty() {
                return Err(AppError::Invalid("A key name can't be empty.".into()));
            }
            let renamed: bool = redis::cmd("RENAMENX").arg(key).arg(to).query_async(&mut conn).await?;
            if !renamed {
                return Err(AppError::Invalid(format!("A key named {to} already exists.")));
            }
        }
        KeyEdit::Delete => {
            redis::cmd("UNLINK").arg(key).query_async::<()>(&mut conn).await?;
        }
        KeyEdit::SetString { .. } => return Err(wrong("single value to set")),
        KeyEdit::SetField { .. } | KeyEdit::RemoveField { .. } => return Err(wrong("fields")),
        KeyEdit::SetItem { .. } | KeyEdit::PushItem { .. } | KeyEdit::RemoveItem { .. } => {
            return Err(wrong("list items"));
        }
        KeyEdit::RemoveMember { .. } => return Err(wrong("members")),
        KeyEdit::AddEntry { .. } | KeyEdit::RemoveEntry { .. } => return Err(wrong("stream entries")),
    }
    Ok(())
}

/// Redis deletes a hash, list or set when its last element goes, so removing
/// that element is refused: deleting the key is a separate, explicit action.
async fn last_element(conn: &mut LoggedConnection, key: &[u8], length: &str) -> Result<()> {
    let count: i64 = redis::cmd(length).arg(key).query_async(conn).await?;
    if count <= 1 {
        return Err(AppError::Invalid(
            "This is the key's last element, and Redis would delete the key with it. Delete the key instead.".into(),
        ));
    }
    Ok(())
}

/// Runs `change` on the list item at `index` only if it still reads `old` and
/// the list doesn't change before the change is applied.
async fn guarded_item(
    conn: &mut LoggedConnection,
    key: &[u8],
    shown: &str,
    index: i64,
    old: &Text,
    change: impl FnOnce(&mut redis::Pipeline),
) -> Result<()> {
    let old = old.bytes()?;
    let changed = || {
        AppError::Invalid(format!(
            "Item {index} of {shown} was changed since it was loaded. Nothing was applied; reload and try again."
        ))
    };
    redis::cmd("WATCH").arg(key).query_async::<()>(&mut *conn).await?;
    let current: Option<Vec<u8>> = redis::cmd("LINDEX").arg(key).arg(index).query_async(&mut *conn).await?;
    if current.as_ref() != Some(&old) {
        redis::cmd("UNWATCH").query_async::<()>(&mut *conn).await?;
        return Err(changed());
    }
    let mut pipe = redis::pipe();
    pipe.atomic();
    change(&mut pipe);
    // EXEC replies nil when a watched key changed.
    let applied: Option<Vec<redis::Value>> = pipe.query_async(&mut *conn).await?;
    applied.map(|_| ()).ok_or_else(changed)
}

pub async fn create_key(connection: &Connection, password: Option<&str>, db: i64, key: &NewKey) -> Result<()> {
    if key.name.is_empty() {
        return Err(AppError::Invalid("A key name can't be empty.".into()));
    }
    let mut conn = browse(connection, password, Some(db)).await?;
    let mut pipe = redis::pipe();
    pipe.atomic();
    match (key.kind.as_str(), &key.field, key.score) {
        ("string", _, _) => pipe.cmd("SET").arg(&key.name).arg(&key.value),
        ("hash", Some(field), _) => pipe.cmd("HSET").arg(&key.name).arg(field).arg(&key.value),
        ("list", _, _) => pipe.cmd("RPUSH").arg(&key.name).arg(&key.value),
        ("set", _, _) => pipe.cmd("SADD").arg(&key.name).arg(&key.value),
        ("zset", _, Some(score)) => pipe.cmd("ZADD").arg(&key.name).arg(score).arg(&key.value),
        ("stream", Some(field), _) => pipe.cmd("XADD").arg(&key.name).arg("*").arg(field).arg(&key.value),
        ("hash" | "stream", None, _) => return Err(AppError::Invalid(format!("A new {} needs a field.", key.kind))),
        ("zset", _, None) => return Err(AppError::Invalid("A new sorted set needs a score.".into())),
        (other, _, _) => return Err(AppError::Invalid(format!("Keys of type {other} can't be created here."))),
    };
    if let Some(ttl) = key.ttl_ms.filter(|t| *t > 0) {
        pipe.cmd("PEXPIRE").arg(&key.name).arg(ttl);
    }
    // Checked under WATCH, so a key created meanwhile isn't overwritten or added to.
    redis::cmd("WATCH").arg(&key.name).query_async::<()>(&mut conn).await?;
    let exists: bool = redis::cmd("EXISTS").arg(&key.name).query_async(&mut conn).await?;
    if exists {
        redis::cmd("UNWATCH").query_async::<()>(&mut conn).await?;
        return Err(AppError::Invalid(format!("A key named {} already exists.", key.name)));
    }
    let created: Option<Vec<redis::Value>> = pipe.query_async(&mut conn).await?;
    created
        .map(|_| ())
        .ok_or_else(|| AppError::Invalid(format!("A key named {} was just created elsewhere.", key.name)))
}

#[cfg(test)]
mod tests {
    use super::super::read_key;
    use super::super::tests::{PASSWORD, database};
    use super::*;

    async fn texts(connection: &Connection, db: i64, key: &str) -> Vec<String> {
        let value = read_key(connection, PASSWORD, db, &key.into(), None, "", 100).await.unwrap();
        value
            .entries
            .iter()
            .map(|e| match (&e.key, e.score) {
                (Some(k), _) => format!("{}={}", k.text, e.value.text),
                (None, Some(score)) => format!("{}:{score}", e.value.text),
                (None, None) => e.value.text.clone(),
            })
            .collect()
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn names_that_are_not_text_are_reached_by_their_bytes() {
        let (connection, mut conn) = database(14, &[]).await;
        // A key, a field, a member and an item as a Java client would write them.
        let raw: &[u8] = &[0xAC, 0xED, 0x00, 0x05];
        for command in ["HSET", "SADD", "RPUSH"] {
            let mut write = redis::cmd(command);
            write.arg([command.as_bytes(), raw].concat());
            if command == "HSET" {
                write.arg(raw).arg("v").arg("keep").arg("v").arg("too").arg("v");
            } else {
                write.arg(raw).arg("keep");
            }
            write.query_async::<()>(&mut conn).await.unwrap();
        }
        let page = super::super::scan(&connection, PASSWORD, 14, "*", None, "0", 100).await.unwrap();
        let binary = |hex: &str| Text { text: hex.into(), binary: true };
        let names: Vec<Text> = page.keys.iter().map(|k| Text { text: k.name.clone(), binary: k.binary }).collect();
        assert_eq!(names, [binary("48534554aced0005"), binary("5250555348aced0005"), binary("53414444aced0005")]);
        let edit = |key: &Text, kind: &'static str, edit: KeyEdit| {
            let (connection, key) = (connection.clone(), key.clone());
            async move { edit_key(&connection, PASSWORD, 14, &key, kind, &edit).await }
        };

        let hash = read_key(&connection, PASSWORD, 14, &names[0], None, "", 10).await.unwrap();
        assert!(hash.binary && hash.entries.iter().any(|e| e.key == Some(binary("aced0005"))));
        edit(&names[0], "hash", KeyEdit::SetField { field: binary("aced0005"), value: "new".into() }).await.unwrap();
        edit(&names[0], "hash", KeyEdit::RemoveField { field: binary("aced0005") }).await.unwrap();
        // Gone now: said, not passed over as done.
        let again = edit(&names[0], "hash", KeyEdit::RemoveField { field: binary("aced0005") }).await.unwrap_err();
        assert_eq!(again.to_string(), "aced0005 is no longer in 48534554aced0005. Reload it.");
        edit(&names[1], "list", KeyEdit::RemoveItem { index: 0, old: binary("aced0005") }).await.unwrap();
        edit(&names[2], "set", KeyEdit::RemoveMember { member: binary("aced0005") }).await.unwrap();
        for name in &names {
            let left = read_key(&connection, PASSWORD, 14, name, None, "", 10).await.unwrap();
            // Only what wasn't removed is left.
            assert!(left.entries.iter().all(|e| !e.value.binary && e.key.as_ref().is_none_or(|k| !k.binary)));
            edit_key(&connection, PASSWORD, 14, name, &left.kind, &KeyEdit::Delete).await.unwrap();
        }
        assert!(super::super::scan(&connection, PASSWORD, 14, "*", None, "0", 100).await.unwrap().keys.is_empty());
        assert!(edit(&binary("zz"), "hash", KeyEdit::Delete).await.unwrap_err().to_string().contains("isn't the hex"));
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn edits_each_type_in_place() {
        let (connection, mut conn) = database(
            4,
            &[
                &["SET", "s", "a", "EX", "600"],
                &["HSET", "h", "a", "1", "b", "2"],
                &["RPUSH", "l", "x", "y", "z"],
                &["SADD", "set", "m", "n"],
                &["ZADD", "z", "1", "m", "2", "n"],
                &["XADD", "st", "1-1", "f", "v"],
            ],
        )
        .await;
        let edit = |key: &'static str, kind: &'static str, edit: KeyEdit| {
            let connection = connection.clone();
            async move { edit_key(&connection, PASSWORD, 4, &key.into(), kind, &edit).await }
        };

        edit("s", "string", KeyEdit::SetString { value: "b".into() }).await.unwrap();
        assert_eq!(texts(&connection, 4, "s").await, ["b"]);
        let ttl: i64 = redis::cmd("TTL").arg("s").query_async(&mut conn).await.unwrap();
        assert!(ttl > 590, "the edit keeps the TTL: {ttl}");

        edit("h", "hash", KeyEdit::SetField { field: "c".into(), value: "3".into() }).await.unwrap();
        edit("h", "hash", KeyEdit::RemoveField { field: "a".into() }).await.unwrap();
        let mut fields = texts(&connection, 4, "h").await;
        fields.sort();
        assert_eq!(fields, ["b=2", "c=3"]);

        edit("l", "list", KeyEdit::SetItem { index: 1, old: "y".into(), value: "Y".into() }).await.unwrap();
        edit("l", "list", KeyEdit::PushItem { value: "w".into(), head: true }).await.unwrap();
        edit("l", "list", KeyEdit::RemoveItem { index: 3, old: "z".into() }).await.unwrap();
        assert_eq!(texts(&connection, 4, "l").await, ["0=w", "1=x", "2=Y"]);
        let error =
            edit("l", "list", KeyEdit::SetItem { index: 0, old: "x".into(), value: "?".into() }).await.unwrap_err();
        assert!(error.to_string().starts_with("Item 0 of l was changed since it was loaded."), "{error}");

        edit("set", "set", KeyEdit::AddMember { member: "o".into(), score: None }).await.unwrap();
        edit("set", "set", KeyEdit::RemoveMember { member: "m".into() }).await.unwrap();
        assert_eq!(texts(&connection, 4, "set").await, ["n", "o"]);

        edit("z", "zset", KeyEdit::AddMember { member: "m".into(), score: Some(5.0) }).await.unwrap();
        edit("z", "zset", KeyEdit::AddMember { member: "o".into(), score: Some(0.5) }).await.unwrap();
        edit("z", "zset", KeyEdit::RemoveMember { member: "n".into() }).await.unwrap();
        assert_eq!(texts(&connection, 4, "z").await, ["o:0.5", "m:5"]);

        edit("st", "stream", KeyEdit::AddEntry { fields: vec![("g".into(), "w".into())] }).await.unwrap();
        edit("st", "stream", KeyEdit::RemoveEntry { id: "1-1".into() }).await.unwrap();
        let entries = texts(&connection, 4, "st").await;
        assert!(entries.len() == 1 && entries[0].ends_with(r#"={"g":"w"}"#), "{entries:?}");
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn manages_keys_and_refuses_unsafe_edits() {
        let (connection, mut conn) =
            database(5, &[&["SET", "a", "1"], &["SET", "b", "2"], &["SADD", "one", "x"]]).await;
        let edit = |key: &'static str, kind: &'static str, edit: KeyEdit| {
            let connection = connection.clone();
            async move { edit_key(&connection, PASSWORD, 5, &key.into(), kind, &edit).await.map_err(|e| e.to_string()) }
        };

        edit("a", "string", KeyEdit::Expire { ttl_ms: Some(60_000) }).await.unwrap();
        let ttl: i64 = redis::cmd("PTTL").arg("a").query_async(&mut conn).await.unwrap();
        assert!(ttl > 59_000);
        edit("a", "string", KeyEdit::Expire { ttl_ms: None }).await.unwrap();
        let ttl: i64 = redis::cmd("PTTL").arg("a").query_async(&mut conn).await.unwrap();
        assert_eq!(ttl, -1);

        assert_eq!(
            edit("a", "string", KeyEdit::Rename { to: "b".into() }).await.unwrap_err(),
            "A key named b already exists."
        );
        edit("a", "string", KeyEdit::Rename { to: "c".into() }).await.unwrap();
        edit("c", "string", KeyEdit::Delete).await.unwrap();
        let exists: i64 = redis::cmd("EXISTS").arg("a").arg("c").query_async(&mut conn).await.unwrap();
        assert_eq!(exists, 0);

        assert_eq!(
            edit("b", "hash", KeyEdit::Delete).await.unwrap_err(),
            "The key b is now a string, not a hash. Reload it."
        );
        assert_eq!(edit("gone", "string", KeyEdit::Delete).await.unwrap_err(), "The key gone no longer exists.");
        assert_eq!(
            edit("b", "string", KeyEdit::SetField { field: "f".into(), value: "v".into() }).await.unwrap_err(),
            "A string has no fields."
        );
        assert!(
            edit("one", "set", KeyEdit::RemoveMember { member: "x".into() })
                .await
                .unwrap_err()
                .starts_with("This is the key's last element")
        );
    }

    #[tokio::test]
    #[ignore = "needs a local Redis on port 56379"]
    async fn creates_keys_with_their_first_element() {
        let (connection, mut conn) = database(6, &[&["SET", "taken", "1"]]).await;
        let new = |name: &str, kind: &str, field: Option<&str>, score: Option<f64>| NewKey {
            name: name.into(),
            kind: kind.into(),
            field: field.map(String::from),
            value: "v".into(),
            score,
            ttl_ms: None,
        };
        for key in [
            new("s", "string", None, None),
            new("h", "hash", Some("f"), None),
            new("l", "list", None, None),
            new("set", "set", None, None),
            new("z", "zset", None, Some(1.5)),
            new("st", "stream", Some("f"), None),
        ] {
            create_key(&connection, PASSWORD, 6, &key).await.unwrap();
        }
        assert_eq!(texts(&connection, 6, "h").await, ["f=v"]);
        assert_eq!(texts(&connection, 6, "z").await, ["v:1.5"]);

        let expiring = NewKey { ttl_ms: Some(30_000), ..new("e", "string", None, None) };
        create_key(&connection, PASSWORD, 6, &expiring).await.unwrap();
        let ttl: i64 = redis::cmd("PTTL").arg("e").query_async(&mut conn).await.unwrap();
        assert!(ttl > 29_000);

        let error = create_key(&connection, PASSWORD, 6, &new("taken", "list", None, None)).await.unwrap_err();
        assert_eq!(error.to_string(), "A key named taken already exists.");
        let error = create_key(&connection, PASSWORD, 6, &new("x", "zset", None, None)).await.unwrap_err();
        assert_eq!(error.to_string(), "A new sorted set needs a score.");
        let taken: String = redis::cmd("TYPE").arg("taken").query_async(&mut conn).await.unwrap();
        assert_eq!(taken, "string");
    }
}
