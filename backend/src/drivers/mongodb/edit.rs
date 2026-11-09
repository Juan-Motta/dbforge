//! Row edits from the table grid. Documents are found by `_id`, and every
//! change is read and converted before anything is written, so a bad value or
//! a document that is gone stops the whole batch. On replica sets and sharded
//! clusters the writes then run in one transaction; a standalone server has
//! none, so a write the server refuses midway leaves the earlier ones applied.
//!
//! Grid cells are text. An edited value keeps the type the field already has
//! (in the document, else in the collection's first documents); a field
//! without one reads the text as a mongosh literal (`42`, `true`,
//! `ObjectId("…")`, `{a: 1}`), and anything else is a string. A NULL cell
//! removes the field.

use std::collections::BTreeMap;

use futures_util::TryStreamExt;
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{Bson, Decimal128, Document, doc};
use mongodb::{Client, ClientSession, Collection};

use super::shell::{date, parse_value};
use super::{columns_of, connect, literal, type_name};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{changed_elsewhere, summarize};
use crate::drivers::{ApplySummary, CellValue, RowChange, TableRef};
use crate::error::{AppError, Result};

/// Documents read to learn the type of a field a document doesn't have.
const SAMPLE: i64 = 100;

/// One change, ready to send.
enum Write {
    Update { id: Bson, set: Document, unset: Document },
    Insert(Document),
    Delete { id: Bson },
}

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let client = connect(connection, password).await?;
    let result = apply(&client, table, changes).await;
    client.shutdown().await;
    result
}

pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    let client = connect(connection, password).await?;
    let writes = prepare(&collection(&client, table), changes).await;
    client.shutdown().await;
    let target = format!("db.getCollection({})", literal(&Bson::String(table.name.clone())));
    Ok(writes?
        .iter()
        .map(|write| match write {
            Write::Update { id, set, unset } => {
                let mut update = Document::new();
                if !set.is_empty() {
                    update.insert("$set", set.clone());
                }
                if !unset.is_empty() {
                    update.insert("$unset", unset.clone());
                }
                format!("{target}.updateOne({}, {});", literal(&id_filter(id).into()), literal(&update.into()))
            }
            Write::Insert(document) => format!("{target}.insertOne({});", literal(&document.clone().into())),
            Write::Delete { id } => format!("{target}.deleteOne({});", literal(&id_filter(id).into())),
        })
        .collect())
}

fn collection(client: &Client, table: &TableRef) -> Collection<Document> {
    client.database(&table.database).collection(&table.name)
}

async fn apply(client: &Client, table: &TableRef, changes: &[RowChange]) -> Result<ApplySummary> {
    let collection = collection(client, table);
    let writes = prepare(&collection, changes).await?;

    let mut session = client.start_session().await?;
    let transaction = supports_transactions(client).await?;
    if transaction {
        session.start_transaction().await?;
    }
    for write in &writes {
        if let Err(error) = send(&collection, &mut session, write).await {
            if transaction {
                session.abort_transaction().await?;
            }
            return Err(error);
        }
    }
    if transaction {
        session.commit_transaction().await?;
    }
    Ok(summarize(changes))
}

async fn send(collection: &Collection<Document>, session: &mut ClientSession, write: &Write) -> Result<()> {
    let matched = match write {
        Write::Update { id, set, unset } => {
            let mut update = Document::new();
            if !set.is_empty() {
                update.insert("$set", set.clone());
            }
            if !unset.is_empty() {
                update.insert("$unset", unset.clone());
            }
            if update.is_empty() {
                return Ok(());
            }
            collection.update_one(id_filter(id), update).session(&mut *session).await?.matched_count
        }
        Write::Insert(document) => {
            collection.insert_one(document).session(&mut *session).await?;
            1
        }
        Write::Delete { id } => collection.delete_one(id_filter(id)).session(&mut *session).await?.deleted_count,
    };
    if matched != 1 {
        let id = match write {
            Write::Update { id, .. } | Write::Delete { id } => id,
            Write::Insert(_) => unreachable!("inserts always count one"),
        };
        return Err(changed_elsewhere(&format!("_id {}", literal(id))));
    }
    Ok(())
}

/// Replica set members and `mongos` routers run transactions; standalone servers can't.
async fn supports_transactions(client: &Client) -> Result<bool> {
    let hello = client.database("admin").run_command(doc! { "hello": 1 }).await?;
    Ok(hello.contains_key("setName") || hello.get_str("msg") == Ok("isdbgrid"))
}

fn id_filter(id: &Bson) -> Document {
    doc! { "_id": id.clone() }
}

/// Finds every edited document and converts every value; nothing is written.
async fn prepare(collection: &Collection<Document>, changes: &[RowChange]) -> Result<Vec<Write>> {
    let sample: Vec<Document> =
        collection.find(doc! {}).sort(doc! { "_id": 1 }).limit(SAMPLE).await?.try_collect().await?;
    let known: BTreeMap<String, String> = columns_of(&sample)
        .into_iter()
        .filter(|c| !matches!(c.data_type.as_str(), "mixed" | "null"))
        .map(|c| (c.name, c.data_type))
        .collect();

    let mut writes = Vec::new();
    for change in changes {
        writes.push(match change {
            RowChange::Update { key, values } => {
                let document = find(collection, key).await?;
                let mut set = Document::new();
                let mut unset = Document::new();
                for (field, text) in values {
                    if field == "_id" {
                        return Err(AppError::Invalid(
                            "A document's _id can't be changed. Insert a copy with the new _id and delete this one."
                                .into(),
                        ));
                    }
                    match text {
                        // A document has no column defaults: a field at its default is a field it lacks.
                        CellValue::Null | CellValue::Default => {
                            unset.insert(field, "");
                        }
                        CellValue::Text(text) => {
                            let current = document.get(field).filter(|v| !matches!(v, Bson::Null | Bson::Undefined));
                            let kind = current.map(type_name).or_else(|| known.get(field).map(String::as_str));
                            set.insert(field, convert(field, text, kind)?);
                        }
                    }
                }
                Write::Update { id: document.get("_id").cloned().unwrap_or(Bson::Null), set, unset }
            }
            RowChange::Insert { values } => {
                let mut document = Document::new();
                // `_id` first, as the server would put it.
                let mut fields: Vec<_> = values.iter().collect();
                fields.sort_by_key(|(field, _)| field.as_str() != "_id");
                for (field, text) in fields {
                    if let CellValue::Text(text) = text {
                        document.insert(field, convert(field, text, known.get(field).map(String::as_str))?);
                    }
                }
                Write::Insert(document)
            }
            RowChange::Delete { key } => {
                let document = find(collection, key).await?;
                Write::Delete { id: document.get("_id").cloned().unwrap_or(Bson::Null) }
            }
        });
    }
    Ok(writes)
}

/// The document whose `_id` the grid showed as `key["_id"]`. The grid shows
/// ids as text, so every value that reads back as that text is looked for, and
/// exactly one document must match.
async fn find(collection: &Collection<Document>, key: &BTreeMap<String, Option<String>>) -> Result<Document> {
    let Some(Some(text)) = key.get("_id") else {
        return Err(AppError::Invalid("Documents are edited by _id, and this one has none.".into()));
    };
    let mut candidates = vec![Bson::String(text.clone())];
    if let Ok(id) = ObjectId::parse_str(text) {
        candidates.push(Bson::ObjectId(id));
    }
    if let Ok(value) = parse_value(text) {
        candidates.push(value);
    }
    if let Ok(n) = text.parse::<i64>() {
        candidates.push(Bson::Int64(n));
    }
    if let Some(when) = date(text) {
        candidates.push(Bson::DateTime(when));
    }
    let found: Vec<Document> = collection
        .find(doc! { "_id": { "$in": candidates } })
        .limit(2)
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        // Numbers of any type match each other; keep what reads back the same.
        .filter(|document| document.get("_id").and_then(super::text).as_deref() == Some(text.as_str()))
        .collect();
    match found.as_slice() {
        [document] => Ok(document.clone()),
        [] => Err(changed_elsewhere(&format!("_id {text}"))),
        _ => Err(AppError::Invalid(format!(
            "More than one document has an _id that reads {text} (of different types), so the right one can't be told apart."
        ))),
    }
}

/// `text` as a value of type `kind` (a `$type` name), or as a mongosh literal
/// when the field has no single type yet.
fn convert(field: &str, text: &str, kind: Option<&str>) -> Result<Bson> {
    let literal = parse_value(text).ok();
    let Some(kind) = kind else {
        return Ok(literal.unwrap_or_else(|| Bson::String(text.to_string())));
    };
    let plain = text.trim();
    let value = match kind {
        "string" => Some(Bson::String(text.to_string())),
        "int" => plain.parse().ok().map(Bson::Int32),
        "long" => plain.parse().ok().map(Bson::Int64),
        "double" => plain.parse().ok().map(Bson::Double),
        "decimal" => plain.parse::<Decimal128>().ok().map(Bson::Decimal128),
        "bool" => plain.parse().ok().map(Bson::Boolean),
        "objectId" => ObjectId::parse_str(plain).ok().map(Bson::ObjectId),
        "date" => date(plain).map(Bson::DateTime),
        _ => None,
    };
    // Also take the type written the mongosh way: `NumberLong(5)`, `ObjectId("…")`, `{a: 1}`.
    value.or_else(|| literal.filter(|value| type_name(value) == kind)).ok_or_else(|| {
        AppError::Invalid(format!("Field {field} holds {kind} values, and \"{text}\" can't be read as one."))
    })
}

#[cfg(test)]
mod tests {
    use mongodb::bson::DateTime;

    use super::super::tests::{PASSWORD, database, table};
    use super::*;

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
    }

    const ID: &str = "65a0f0f0f0f0f0f0f0f0f0f0";

    async fn people(name: &str) -> Connection {
        let id = ObjectId::parse_str(ID).unwrap();
        database(
            name,
            &[(
                "people",
                vec![
                    doc! { "_id": id, "name": "Ann", "age": 30_i64, "joined": DateTime::from_millis(0), "tags": ["a"] },
                    doc! { "_id": 2, "name": "Bob", "age": 41_i64, "score": 1.5 },
                    doc! { "_id": "3", "name": "Cy", "age": 22_i64 },
                ],
            )],
        )
        .await
    }

    async fn documents(connection: &Connection, name: &str) -> Vec<Document> {
        let client = connect(connection, PASSWORD).await.unwrap();
        let found = client
            .database(name)
            .collection::<Document>("people")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        client.shutdown().await;
        found
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn applies_changes_keeping_each_fields_type() {
        let connection = people("dbforge_edit").await;
        let changes = vec![
            RowChange::Update {
                key: map(&[("_id", Some(ID))]),
                values: map(&[
                    ("age", Some("31")),
                    ("joined", Some("2024-05-01T10:00:00Z")),
                    ("tags", Some("['a', 'b']")),
                    ("name", None),
                ]),
            },
            // Ids that read like numbers find numbers, and `"3"` finds the string.
            RowChange::Update { key: map(&[("_id", Some("2"))]), values: map(&[("score", Some("2"))]) },
            RowChange::Delete { key: map(&[("_id", Some("3"))]) },
            RowChange::Insert {
                values: map(&[("_id", Some("4")), ("name", Some("42")), ("age", Some("7")), ("extra", Some("{a: 1}"))]),
            },
        ];
        let summary = apply_changes(&connection, PASSWORD, &table("dbforge_edit", "people"), &changes).await.unwrap();
        assert_eq!((summary.updated, summary.inserted, summary.deleted), (2, 1, 1));

        let found = documents(&connection, "dbforge_edit").await;
        assert_eq!(found.len(), 3);
        let ann = &found[0];
        assert_eq!(ann.get("age"), Some(&Bson::Int64(31)));
        assert_eq!(ann.get_datetime("joined").unwrap().timestamp_millis(), 1_714_557_600_000);
        assert_eq!(ann.get_array("tags").unwrap().len(), 2);
        assert!(!ann.contains_key("name"));
        assert_eq!(found[1].get("score"), Some(&Bson::Double(2.0)));
        // A new document: known fields keep their type (`_id` is mixed, so it's read as a literal).
        assert_eq!(found[2], doc! { "_id": 4, "name": "42", "age": 7_i64, "extra": { "a": 1 } });
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn a_bad_value_or_a_missing_document_stops_everything() {
        let connection = people("dbforge_edit_reject").await;
        let before = documents(&connection, "dbforge_edit_reject").await;
        let target = table("dbforge_edit_reject", "people");

        let changes = vec![
            RowChange::Delete { key: map(&[("_id", Some("2"))]) },
            RowChange::Update { key: map(&[("_id", Some(ID))]), values: map(&[("age", Some("old"))]) },
        ];
        let error = apply_changes(&connection, PASSWORD, &target, &changes).await.unwrap_err();
        assert_eq!(error.to_string(), "Field age holds long values, and \"old\" can't be read as one.");

        let changes = vec![
            RowChange::Delete { key: map(&[("_id", Some("2"))]) },
            RowChange::Delete { key: map(&[("_id", Some("99"))]) },
        ];
        let error = apply_changes(&connection, PASSWORD, &target, &changes).await.unwrap_err();
        assert!(error.to_string().starts_with("The row with _id 99 was changed or deleted"), "{error}");

        let changes = vec![RowChange::Update { key: map(&[("_id", Some("2"))]), values: map(&[("_id", Some("5"))]) }];
        let error = apply_changes(&connection, PASSWORD, &target, &changes).await.unwrap_err();
        assert!(error.to_string().starts_with("A document's _id can't be changed"), "{error}");
        assert_eq!(documents(&connection, "dbforge_edit_reject").await, before);
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn previews_the_exact_mongosh_statements() {
        let connection = people("dbforge_edit_preview").await;
        let changes = vec![
            RowChange::Update { key: map(&[("_id", Some(ID))]), values: map(&[("age", Some("31")), ("name", None)]) },
            RowChange::Insert { values: map(&[("name", Some("Dee"))]) },
            RowChange::Delete { key: map(&[("_id", Some("3"))]) },
        ];
        let statements =
            preview_changes(&connection, PASSWORD, &table("dbforge_edit_preview", "people"), &changes).await.unwrap();
        assert_eq!(
            statements,
            [
                format!(
                    r#"db.getCollection("people").updateOne({{"_id": ObjectId("{ID}")}}, {{"$set": {{"age": NumberLong(31)}}, "$unset": {{"name": ""}}}});"#
                ),
                r#"db.getCollection("people").insertOne({"name": "Dee"});"#.to_string(),
                r#"db.getCollection("people").deleteOne({"_id": "3"});"#.to_string(),
            ]
        );
    }
}
