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

use super::logged::{Find, LoggedClient, LoggedCollection, LoggedSession, Update, Write};
use super::shell::{date, parse_value};
use super::{columns_of, connect, literal, type_name};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{changed_elsewhere, summarize};
use crate::drivers::{ApplySummary, CellValue, ImportBatches, RowChange, TableRef};
use crate::error::{AppError, Result};
use crate::statement_log;

/// Documents read to learn the type of a field a document doesn't have.
const SAMPLE: i64 = 100;

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

/// The writes [`apply_changes`] would send, as mongosh runs them.
pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    let client = connect(connection, password).await?;
    let writes = prepare(&client.database(&table.database).collection(&table.name), changes).await;
    client.shutdown().await;
    Ok(writes?.iter().map(|write| format!("{};", write.text(&table.name))).collect())
}

async fn apply(client: &LoggedClient, table: &TableRef, changes: &[RowChange]) -> Result<ApplySummary> {
    let database = client.database(&table.database);
    let collection = database.collection(&table.name);
    let writes = prepare(&collection, changes).await?;

    let mut session = database.start_session().await?;
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

/// Sends `write`, which must find exactly the one document it was prepared for.
async fn send(collection: &LoggedCollection, session: &mut LoggedSession, write: &Write) -> Result<()> {
    if session.write(collection, write).await?.matched != 1 {
        let id = write.filter().and_then(|filter| filter.get("_id")).unwrap_or(&Bson::Null);
        return Err(changed_elsewhere(&format!("_id {}", literal(id))));
    }
    Ok(())
}

/// Replica set members and `mongos` routers run transactions; standalone servers can't.
async fn supports_transactions(client: &LoggedClient) -> Result<bool> {
    let hello = client.database("admin").run_command(doc! { "hello": 1 }).await?;
    Ok(hello.contains_key("setName") || hello.get_str("msg") == Ok("isdbgrid"))
}

fn id_filter(id: &Bson) -> Document {
    doc! { "_id": id.clone() }
}

/// The type of each field that has one in the collection's first documents.
async fn known_types(collection: &LoggedCollection) -> Result<BTreeMap<String, String>> {
    let sample: Vec<Document> =
        collection.find(Find::new(doc! {}).sort(doc! { "_id": 1 }).limit(SAMPLE)).await?.try_collect().await?;
    Ok(columns_of(&sample)
        .into_iter()
        .filter(|c| !matches!(c.data_type.as_str(), "mixed" | "null"))
        .map(|c| (c.name, c.data_type))
        .collect())
}

/// Finds every edited document and converts every value; nothing is written.
async fn prepare(collection: &LoggedCollection, changes: &[RowChange]) -> Result<Vec<Write>> {
    let known = known_types(collection).await?;

    let mut writes = Vec::new();
    for change in changes {
        writes.extend(match change {
            RowChange::Update { key, values } => {
                let document = find(collection, key).await?;
                let mut set = Document::new();
                let mut unset = Document::new();
                for (field, text) in values {
                    if field == "_id" {
                        return Err(id_unchangeable());
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
                let mut update = Document::new();
                if !set.is_empty() {
                    update.insert("$set", set);
                }
                if !unset.is_empty() {
                    update.insert("$unset", unset);
                }
                // Nothing to change: nothing to send.
                (!update.is_empty()).then(|| {
                    let filter = id_filter(document.get("_id").unwrap_or(&Bson::Null));
                    Write::UpdateOne(Update { filter, update: update.into(), upsert: false })
                })
            }
            RowChange::Insert { values } => {
                let fields = values.iter().map(|(field, value)| match value {
                    CellValue::Text(text) => (field.as_str(), Some(text.as_str())),
                    CellValue::Null | CellValue::Default => (field.as_str(), None),
                });
                Some(Write::InsertOne(new_document(fields, &known)?))
            }
            RowChange::Delete { key } => {
                let document = find(collection, key).await?;
                Some(Write::DeleteOne(id_filter(document.get("_id").unwrap_or(&Bson::Null))))
            }
            RowChange::Replace { key, document } => {
                let id = find(collection, key).await?.get("_id").cloned().unwrap_or(Bson::Null);
                let replacement = parse_document(document)?;
                let document = match replacement.get("_id") {
                    Some(given) if *given != id => return Err(id_unchangeable()),
                    Some(_) => replacement,
                    // Left out, the _id is kept, first as the server keeps it.
                    None => {
                        let mut with_id = doc! { "_id": id.clone() };
                        with_id.extend(replacement);
                        with_id
                    }
                };
                Some(Write::ReplaceOne { filter: id_filter(&id), replacement: document, upsert: false })
            }
            RowChange::InsertDocument { document } => Some(Write::InsertOne(parse_document(document)?)),
        });
    }
    Ok(writes)
}

/// A new document from text values, converted as [`convert`] does; a NULL
/// leaves the field out. `_id` goes first, as the server would put it.
fn new_document<'a>(
    fields: impl Iterator<Item = (&'a str, Option<&'a str>)>,
    known: &BTreeMap<String, String>,
) -> Result<Document> {
    let mut fields: Vec<_> = fields.filter_map(|(field, text)| Some((field, text?))).collect();
    fields.sort_by_key(|(field, _)| *field != "_id");
    let mut document = Document::new();
    for (field, text) in fields {
        document.insert(field, convert(field, text, known.get(field).map(String::as_str))?);
    }
    Ok(document)
}

/// Inserts an import's rows a batch at a time, each value converted as a
/// grid insert converts it. There is no transaction: the first error stops
/// the import, and says how many rows were already in.
pub async fn import_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    names: &[String],
    batches: &mut ImportBatches,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<u64> {
    let client = connect(connection, password).await?;
    let collection = client.database(&table.database).collection(&table.name);
    let mut inserted = 0;
    let result = import(&collection, table, names, batches, &mut inserted, progress).await;
    client.shutdown().await;
    result.map_err(|error| imported_before(inserted, error))?;
    Ok(inserted)
}

async fn import(
    collection: &LoggedCollection,
    table: &TableRef,
    names: &[String],
    batches: &mut ImportBatches,
    inserted: &mut u64,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<()> {
    let known = known_types(collection).await?;
    while let Some(batch) = batches.recv().await {
        let documents = batch?
            .iter()
            .map(|row| new_document(names.iter().map(String::as_str).zip(row.iter().map(Option::as_deref)), &known))
            .collect::<Result<Vec<_>>>()?;
        let count = documents.len() as u64;
        // The log shows the call, not every document.
        let shown = format!(
            "db.getCollection({}).insertMany([…]) // {count} documents",
            literal(&Bson::String(table.name.clone()))
        );
        match statement_log::shown_as(shown, collection.write(&Write::InsertMany(documents))).await {
            Ok(_) => *inserted += count,
            Err(error) => {
                // An ordered insertMany stops at its first failing document, after the ones before it.
                *inserted += first_failure(&error).unwrap_or(0);
                return Err(error.into());
            }
        }
        progress(*inserted);
    }
    Ok(())
}

/// Where in its batch an insertMany failed: how many of the batch went in.
fn first_failure(error: &mongodb::error::Error) -> Option<u64> {
    let mongodb::error::ErrorKind::InsertMany(failure) = error.kind.as_ref() else { return None };
    failure.write_errors.as_ref()?.iter().map(|e| e.index as u64).min()
}

/// The error an import stopped with, saying how many rows it had added.
fn imported_before(inserted: u64, error: AppError) -> AppError {
    AppError::Invalid(format!("{inserted} rows were imported before it stopped: {error}"))
}

fn id_unchangeable() -> AppError {
    AppError::Invalid("A document's _id can't be changed. Insert a copy with the new _id and delete this one.".into())
}

/// A document from the JSON view, in mongosh syntax: `{name: "Ann", _id: ObjectId("…")}`.
fn parse_document(text: &str) -> Result<Document> {
    match parse_value(text) {
        Ok(Bson::Document(document)) => Ok(document),
        Ok(_) => Err(AppError::Invalid("A document is written in braces: {name: \"Ann\"}.".into())),
        Err(error) => Err(AppError::Invalid(format!("The document can't be read: {error}"))),
    }
}

/// The document whose `_id` the grid showed as `key["_id"]`. The grid shows
/// ids as text, so every value that reads back as that text is looked for, and
/// exactly one document must match.
async fn find(collection: &LoggedCollection, key: &BTreeMap<String, Option<String>>) -> Result<Document> {
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
        .find(Find::new(doc! { "_id": { "$in": candidates } }).limit(2))
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
    use crate::drivers::import_batches;

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

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn imports_batches_and_says_how_many_went_in_before_an_error() {
        let connection = people("dbforge_import").await;
        let table = table("dbforge_import", "people");
        let names: Vec<String> = ["_id", "name", "age"].map(String::from).to_vec();
        let mut sent = import_batches(vec![
            Ok(vec![vec![Some("10"), Some("Di"), Some("50")], vec![Some("11"), Some("Ed"), None]]),
            // The second document repeats an _id: the one before it goes in, then it stops.
            Ok(vec![vec![Some("12"), Some("Flo"), Some("7")], vec![Some("10"), Some("Again"), None]]),
            Ok(vec![vec![Some("13"), Some("Never"), None]]),
        ]);
        let mut seen = Vec::new();
        let error =
            import_rows(&connection, PASSWORD, &table, &names, &mut sent, &mut |n| seen.push(n)).await.unwrap_err();
        assert!(error.to_string().starts_with("3 rows were imported before it stopped:"), "{error}");
        assert_eq!(seen, [2]);
        let found = documents(&connection, "dbforge_import").await;
        assert_eq!(found.len(), 6);
        // Values take the type the field has in the collection; a NULL leaves the field out.
        let di = found.iter().find(|d| d.get_str("name") == Ok("Di")).unwrap();
        assert_eq!(di.get("age"), Some(&Bson::Int64(50)));
        assert!(!found.iter().find(|d| d.get_str("name") == Ok("Ed")).unwrap().contains_key("age"));
    }

    async fn documents(connection: &Connection, name: &str) -> Vec<Document> {
        let client = connect(connection, PASSWORD).await.unwrap();
        let found = client
            .unlogged()
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
    async fn records_each_write_with_its_values_written_in() {
        use crate::statement_log::store::tests::Collected;
        use crate::statement_log::{self, Origin, StatementLog};

        let connection = people("dbforge_edit_log").await;
        let changes = vec![
            RowChange::Update { key: map(&[("_id", Some(ID))]), values: map(&[("name", Some("it's Ann"))]) },
            RowChange::Delete { key: map(&[("_id", Some("3"))]) },
        ];
        let log = StatementLog::new(50, Collected::default());
        let target = table("dbforge_edit_log", "people");

        statement_log::scoped(&log, "mongo", Origin::Edit, apply_changes(&connection, PASSWORD, &target, &changes))
            .await
            .unwrap();

        let entries = log.entries();
        let texts: Vec<_> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[0], r#"db.getCollection("people").find({}).sort({"_id": 1}).limit(100)"#);
        let writes: Vec<_> = entries
            .iter()
            .filter(|e| e.text.contains(".updateOne(") || e.text.contains(".deleteOne("))
            .map(|e| (e.text.as_str(), e.database.as_deref(), e.rows))
            .collect();
        assert_eq!(
            writes,
            [
                (
                    r#"db.getCollection("people").updateOne({"_id": ObjectId("65a0f0f0f0f0f0f0f0f0f0f0")}, {"$set": {"name": "it's Ann"}})"#,
                    Some("dbforge_edit_log"),
                    Some(1)
                ),
                (r#"db.getCollection("people").deleteOne({"_id": "3"})"#, Some("dbforge_edit_log"), Some(1)),
            ]
        );
        // The documents are found before anything is written, and the server is asked whether it runs transactions.
        let finds = texts.iter().filter(|t| t.contains(r#"find({"_id": {"$in": "#)).count();
        assert_eq!(finds, 2, "{texts:#?}");
        assert!(texts.contains(&r#"db.runCommand({"hello": 1})"#), "{texts:#?}");
        let hello = entries.iter().find(|e| e.text.contains("hello")).unwrap();
        assert_eq!((hello.database.as_deref(), hello.rows), (Some("admin"), Some(1)));
        assert!(entries.iter().all(|e| e.connection_id == "mongo" && e.origin == Origin::Edit && e.error.is_none()));
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn a_document_keyed_by_a_uuid_is_found_and_keeps_its_types() {
        use mongodb::bson::spec::BinarySubtype;
        use mongodb::bson::{Binary, Timestamp};

        let id = Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() };
        let stamp = Timestamp { time: 5, increment: 1 };
        let document = doc! { "_id": id.clone(), "name": "Ann", "raw": Binary { subtype: BinarySubtype::Generic, bytes: vec![0xFF, 0] }, "at": stamp };
        let connection = database("dbforge_edit_uuid", &[("people", vec![document])]).await;
        let people = table("dbforge_edit_uuid", "people");
        let page = super::super::fetch_rows(&connection, PASSWORD, &people, &Default::default(), 0, 10).await.unwrap();
        // The key as the grid shows it.
        let shown = page.rows[0][0].clone().unwrap();
        assert_eq!(shown, r#"UUID("00010203-0405-0607-0809-0a0b0c0d0e0f")"#);

        let rename = [RowChange::Update { key: map(&[("_id", Some(&shown))]), values: map(&[("name", Some("Bea"))]) }];
        apply_changes(&connection, PASSWORD, &people, &rename).await.unwrap();
        // The whole document, saved as the JSON view writes it, keeps every type.
        let text =
            super::super::pretty_literal(&Bson::Document(documents(&connection, "dbforge_edit_uuid").await[0].clone()));
        let replace = [RowChange::Replace { key: map(&[("_id", Some(&shown))]), document: text.replace("Bea", "Cy") }];
        apply_changes(&connection, PASSWORD, &people, &replace).await.unwrap();

        let saved = documents(&connection, "dbforge_edit_uuid").await;
        assert_eq!(
            saved,
            [
                doc! { "_id": id, "name": "Cy", "raw": Binary { subtype: BinarySubtype::Generic, bytes: vec![0xFF, 0] }, "at": stamp }
            ]
        );
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

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn replaces_and_adds_whole_documents() {
        let connection = people("dbforge_edit_documents").await;
        let table = table("dbforge_edit_documents", "people");
        let changes = vec![
            // Without its _id, the document keeps the one it has; fields left out are gone.
            RowChange::Replace {
                key: map(&[("_id", Some(ID))]),
                document: r#"{name: "Ann", age: NumberLong(31), joined: ISODate("2024-01-01T00:00:00Z")}"#.into(),
            },
            RowChange::Replace { key: map(&[("_id", Some("2"))]), document: "{_id: 2, name: 'Bob'}".into() },
            RowChange::InsertDocument { document: "{name: 'Dee', tags: ['x', {deep: true}]}".into() },
        ];
        let preview = preview_changes(&connection, PASSWORD, &table, &changes).await.unwrap();
        assert_eq!(preview[1], r#"db.getCollection("people").replaceOne({"_id": 2}, {"_id": 2, "name": "Bob"});"#);
        let summary = apply_changes(&connection, PASSWORD, &table, &changes).await.unwrap();
        assert_eq!(summary, ApplySummary { updated: 2, inserted: 1, deleted: 0 });

        let all = documents(&connection, "dbforge_edit_documents").await;
        let ann = all.iter().find(|d| d.get_str("name") == Ok("Ann")).unwrap();
        assert_eq!(ann.keys().collect::<Vec<_>>(), ["_id", "name", "age", "joined"]);
        assert_eq!(ann.get_i64("age"), Ok(31));
        assert!(all.iter().any(|d| d.get_str("name") == Ok("Bob") && d.len() == 2));
        assert!(all.iter().any(|d| d.get_str("name") == Ok("Dee")));

        let bad =
            |document: &str| vec![RowChange::Replace { key: map(&[("_id", Some("3"))]), document: document.into() }];
        let error = apply_changes(&connection, PASSWORD, &table, &bad("{name: ")).await.unwrap_err();
        assert!(error.to_string().starts_with("The document can't be read"), "{error}");
        let error = apply_changes(&connection, PASSWORD, &table, &bad("[1]")).await.unwrap_err();
        assert!(error.to_string().starts_with("A document is written in braces"), "{error}");
        let error = apply_changes(&connection, PASSWORD, &table, &bad("{_id: 4}")).await.unwrap_err();
        assert!(error.to_string().starts_with("A document's _id can't be changed"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn pages_carry_each_document_as_mongosh_writes_it() {
        let connection = people("dbforge_page_documents").await;
        let filter = crate::drivers::RowFilter { filter: "{name: 'Bob'}".into(), order: String::new() };
        let page =
            super::super::fetch_rows(&connection, PASSWORD, &table("dbforge_page_documents", "people"), &filter, 0, 10)
                .await
                .unwrap();
        assert_eq!(
            page.documents.unwrap(),
            ["{\n  \"_id\": 2,\n  \"name\": \"Bob\",\n  \"age\": NumberLong(41),\n  \"score\": 1.5\n}"]
        );
    }
}
