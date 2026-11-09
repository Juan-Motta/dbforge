//! Item edits from the grid. A row is found by its `id` and its partition key
//! (the key columns, read back as text). Every change is prepared first: each
//! item is read and every value converted, so a missing item or a bad value
//! means nothing is written. Writes then go one by one, each replacement and
//! delete carrying the etag the item was read with, so an item changed by
//! someone else in between fails instead of being overwritten. Cosmos DB has
//! no transaction across partitions: if a write fails, the ones before it stay.

use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::{Map, Value, json};

use super::client::{Client, Target};
use super::query::{Feed, Items, Select, documents};
use super::{shared_type, text, type_name, without_system_properties};
use crate::connections::model::Connection;
use crate::drivers::edit_sql::{changed_elsewhere, summarize};
use crate::drivers::{ApplySummary, CellValue, RowChange, RowFilter, TableRef};
use crate::error::{AppError, Result};

/// Items read to learn each property's type for new items.
const SAMPLE: usize = 100;

/// One write, prepared.
#[derive(Debug, PartialEq)]
enum Write {
    Replace { id: String, partition: Value, etag: String, item: Value },
    Create { partition: Value, item: Value },
    Delete { id: String, partition: Value, etag: String },
}

pub async fn apply_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<ApplySummary> {
    let client = Client::new(connection, password)?;
    let writes = prepare(&client, table, changes).await?;
    for (done, write) in writes.iter().enumerate() {
        if let Err(error) = execute(&client, table, write).await {
            return Err(if done == 0 {
                error
            } else {
                AppError::Invalid(format!(
                    "Applied {done} of {} changes, then: {error} Reload to see the items as they are now.",
                    writes.len()
                ))
            });
        }
    }
    Ok(summarize(changes))
}

/// The writes `apply_changes` would send, one per line.
pub async fn preview_changes(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    changes: &[RowChange],
) -> Result<Vec<String>> {
    let client = Client::new(connection, password)?;
    let writes = prepare(&client, table, changes).await?;
    Ok(writes.iter().map(|write| describe(&table.name, write)).collect())
}

fn describe(container: &str, write: &Write) -> String {
    match write {
        Write::Replace { id, partition, item, .. } => format!("REPLACE {container}/{id} PARTITION {partition} {item}"),
        Write::Create { partition, item } => format!("CREATE {container} PARTITION {partition} {item}"),
        Write::Delete { id, partition, .. } => format!("DELETE {container}/{id} PARTITION {partition}"),
    }
}

async fn prepare(client: &Client, table: &TableRef, changes: &[RowChange]) -> Result<Vec<Write>> {
    let container = client.get(&Target::container(&table.database, &table.name), &[]).await?.body;
    let paths: Vec<String> = container
        .pointer("/partitionKey/paths")
        .and_then(Value::as_array)
        .map(|paths| paths.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    let types = if changes.iter().any(|change| matches!(change, RowChange::Insert { .. })) {
        sample_types(client, table).await?
    } else {
        BTreeMap::new()
    };

    let mut writes = Vec::new();
    for change in changes {
        writes.push(match change {
            RowChange::Update { key, values } => {
                let item = find(client, table, key).await?;
                let mut updated = item.clone();
                for (property, value) in values {
                    let kind = item.get(property).filter(|v| !v.is_null()).map(type_name);
                    set(&mut updated, property, value, kind)?;
                }
                stays_in_place(&item, &updated, &paths)?;
                Write::Replace {
                    id: id_of(&item),
                    partition: partition(&item, &paths),
                    etag: etag_of(&item),
                    item: without_system_properties(updated),
                }
            }
            RowChange::Insert { values } => {
                let mut item = Value::Object(Map::new());
                for (property, value) in id_first(values) {
                    let kind = if property == "id" { Some("string") } else { types.get(property).map(String::as_str) };
                    set(&mut item, property, value, kind)?;
                }
                with_id(&mut item);
                Write::Create { partition: partition(&item, &paths), item }
            }
            RowChange::Replace { key, document } => {
                let item = find(client, table, key).await?;
                let replacement = parse_document(document)?;
                stays_in_place(&item, &replacement, &paths)?;
                Write::Replace {
                    id: id_of(&item),
                    partition: partition(&item, &paths),
                    etag: etag_of(&item),
                    item: replacement,
                }
            }
            RowChange::InsertDocument { document } => {
                let mut item = parse_document(document)?;
                with_id(&mut item);
                Write::Create { partition: partition(&item, &paths), item }
            }
            RowChange::Delete { key } => {
                let item = find(client, table, key).await?;
                Write::Delete { id: id_of(&item), partition: partition(&item, &paths), etag: etag_of(&item) }
            }
        });
    }
    Ok(writes)
}

async fn execute(client: &Client, table: &TableRef, write: &Write) -> Result<()> {
    let (database, container) = (&table.database, &table.name);
    match write {
        Write::Replace { id, partition, etag, item } => {
            let key = partition.to_string();
            let headers = [("x-ms-documentdb-partitionkey", key.as_str()), ("if-match", etag.as_str())];
            client.send(Method::PUT, &Target::item(database, container, id), &headers, Some(item)).await?;
        }
        Write::Create { partition, item } => {
            let key = partition.to_string();
            let headers = [("x-ms-documentdb-partitionkey", key.as_str())];
            client.send(Method::POST, &Target::items(database, container), &headers, Some(item)).await?;
        }
        Write::Delete { id, partition, etag } => {
            let key = partition.to_string();
            let headers = [("x-ms-documentdb-partitionkey", key.as_str()), ("if-match", etag.as_str())];
            client.send(Method::DELETE, &Target::item(database, container, id), &headers, None).await?;
        }
    }
    Ok(())
}

/// The item a row was loaded from: the one with its id whose key columns still
/// read as they did. Exactly one must match.
async fn find(client: &Client, table: &TableRef, key: &BTreeMap<String, Option<String>>) -> Result<Value> {
    let described = id_first(key)
        .map(|(name, value)| format!("{name} = {}", value.as_deref().unwrap_or("NULL")))
        .collect::<Vec<_>>()
        .join(", ");
    let Some(Some(id)) = key.get("id") else {
        return Err(AppError::Invalid("A row without an id can't be found to edit it.".into()));
    };
    let mut feed = Feed::new(client, &table.database, &table.name, "SELECT * FROM c WHERE c.id = @id", &[])
        .with_parameters(&[("@id", json!(id))]);
    let mut matches = Vec::new();
    while let Some(reply) = feed.next().await? {
        matches.extend(
            documents(&reply)
                .iter()
                .filter(|item| key.iter().all(|(name, value)| item.get(name).and_then(text) == *value))
                .cloned(),
        );
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(changed_elsewhere(&described)),
        _ => Err(AppError::Invalid(format!(
            "Several items have {described}: their partition key isn't a column, so the row can't tell them apart."
        ))),
    }
}

/// The JSON type each property's values share in a sample, for new items.
async fn sample_types(client: &Client, table: &TableRef) -> Result<BTreeMap<String, String>> {
    let all = Select::of(&RowFilter::default());
    let mut items = Items::open(client, &table.database, &table.name, &all, SAMPLE as u32).await?;
    let mut sample = Vec::new();
    while sample.len() < SAMPLE {
        match items.next().await? {
            Some(item) => sample.push(item),
            None => break,
        }
    }
    let mut names: Vec<&String> = sample.iter().filter_map(Value::as_object).flat_map(|item| item.keys()).collect();
    names.sort();
    names.dedup();
    Ok(names
        .into_iter()
        .map(|name| (name.clone(), shared_type(sample.iter().filter_map(|item| item.get(name)))))
        .collect())
}

/// Writes `value` into `item`: NULL is JSON null, DEFAULT leaves the property out.
fn set(item: &mut Value, property: &str, value: &CellValue, kind: Option<&str>) -> Result<()> {
    let properties = item.as_object_mut().expect("items are objects");
    match value {
        CellValue::Default => {
            properties.remove(property);
        }
        CellValue::Null => {
            properties.insert(property.to_string(), Value::Null);
        }
        CellValue::Text(text) => {
            properties.insert(property.to_string(), convert(property, text, kind)?);
        }
    }
    Ok(())
}

/// `text` as a value of JSON type `kind`, or as JSON when the property has no
/// single type yet, else as a string.
fn convert(property: &str, text: &str, kind: Option<&str>) -> Result<Value> {
    let json = serde_json::from_str::<Value>(text).ok();
    match kind {
        None | Some("null" | "mixed") => Ok(json.unwrap_or_else(|| Value::String(text.to_string()))),
        Some("string") => Ok(Value::String(text.to_string())),
        Some(kind) => json.filter(|value| type_name(value) == kind).ok_or_else(|| {
            AppError::Invalid(format!("Property {property} holds {kind} values, and \"{text}\" can't be read as one."))
        }),
    }
}

/// The item's partition key value as the REST API takes it: one entry per
/// path, `{}` where the item has none.
fn partition(item: &Value, paths: &[String]) -> Value {
    Value::Array(
        paths
            .iter()
            .map(|path| {
                let pointer: String = path.split('/').map(|part| part.trim_matches('"')).collect::<Vec<_>>().join("/");
                item.pointer(&pointer).cloned().unwrap_or_else(|| json!({}))
            })
            .collect(),
    )
}

/// The entries of a row, `id` first as in the grid.
fn id_first<V>(row: &BTreeMap<String, V>) -> impl Iterator<Item = (&String, &V)> {
    row.get_key_value("id").into_iter().chain(row.iter().filter(|(name, _)| *name != "id"))
}

fn id_of(item: &Value) -> String {
    item.get("id").and_then(Value::as_str).unwrap_or_default().to_string()
}

fn etag_of(item: &Value) -> String {
    item.get("_etag").and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Cosmos DB moves an item only by deleting it and adding another.
fn stays_in_place(item: &Value, changed: &Value, paths: &[String]) -> Result<()> {
    if changed.get("id") != item.get("id") || partition(changed, paths) != partition(item, paths) {
        return Err(AppError::Invalid(
            "Changing an item's id or partition key moves it, which Cosmos DB can't do in place: \
             add a new item and delete this one."
                .into(),
        ));
    }
    Ok(())
}

/// Cosmos DB needs an id; the SDKs make one up the same way.
fn with_id(item: &mut Value) {
    if item.get("id").is_none_or(Value::is_null) {
        item["id"] = uuid::Uuid::new_v4().to_string().into();
    }
}

/// A document from the JSON view: one JSON object, the service's properties left out.
fn parse_document(text: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| AppError::Invalid(format!("The document isn't valid JSON: {error}.")))?;
    if !value.is_object() {
        return Err(AppError::Invalid("A document is a JSON object, in braces.".into()));
    }
    Ok(without_system_properties(value))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{KEY, database, table};
    use super::*;
    use crate::drivers::cosmos::fetch_rows;

    /// Keys (`Option<String>`) or values (`CellValue`) from text, `None` being NULL.
    fn map<V: From<Option<String>>>(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, V> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.map(String::from).into())).collect()
    }

    #[test]
    fn values_keep_their_type_or_read_as_json() {
        assert_eq!(convert("n", "5", Some("number")).unwrap(), json!(5));
        assert_eq!(convert("s", "5", Some("string")).unwrap(), json!("5"));
        assert_eq!(convert("b", "true", Some("boolean")).unwrap(), json!(true));
        assert_eq!(convert("o", r#"{"a": [1]}"#, Some("object")).unwrap(), json!({"a": [1]}));
        assert_eq!(convert("x", "[1, 2]", None).unwrap(), json!([1, 2]));
        assert_eq!(convert("x", "plain words", Some("mixed")).unwrap(), json!("plain words"));
        let error = convert("total", "ten", Some("number")).unwrap_err();
        assert_eq!(error.to_string(), "Property total holds number values, and \"ten\" can't be read as one.");
    }

    #[test]
    fn partition_keys_read_every_path() {
        let item = json!({"tenant": "t", "address": {"city": "Lima"}});
        assert_eq!(partition(&item, &["/tenant".into(), "/address/city".into()]), json!(["t", "Lima"]));
        assert_eq!(partition(&item, &["/missing".into()]), json!([{}]));
        assert_eq!(partition(&json!({"a b": 1}), &["/\"a b\"".into()]), json!([1]));
    }

    fn orders() -> Vec<Value> {
        vec![
            json!({"id": "1", "customer": "a", "total": 10, "note": "x", "tags": ["t"]}),
            json!({"id": "2", "customer": "b", "total": 20}),
            json!({"id": "3", "customer": "a", "total": 30}),
        ]
    }

    async fn rows(connection: &Connection, name: &str) -> Vec<Vec<Option<String>>> {
        let filter = RowFilter { filter: String::new(), order: "c.id".into() };
        fetch_rows(connection, KEY, &table(name, "orders"), &filter, 0, 50).await.unwrap().rows
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn applies_updates_inserts_and_deletes() {
        let connection = database("dbforge_edit", &[("orders", "/customer", orders())]).await;
        let table = table("dbforge_edit", "orders");
        let changes = vec![
            RowChange::Update {
                key: map(&[("id", Some("1")), ("customer", Some("a"))]),
                values: [
                    ("total".to_string(), CellValue::Text("15".into())),
                    ("note".to_string(), CellValue::Default),
                    ("tags".to_string(), CellValue::Null),
                ]
                .into(),
            },
            RowChange::Delete { key: map(&[("id", Some("2")), ("customer", Some("b"))]) },
            RowChange::Insert { values: map(&[("id", Some("4")), ("customer", Some("c")), ("total", Some("40"))]) },
            RowChange::Insert { values: map(&[("customer", Some("d"))]) },
        ];
        let preview = preview_changes(&connection, KEY, &table, &changes).await.unwrap();
        assert_eq!(
            preview[..3],
            [
                r#"REPLACE orders/1 PARTITION ["a"] {"id":"1","customer":"a","total":15,"tags":null}"#,
                r#"DELETE orders/2 PARTITION ["b"]"#,
                r#"CREATE orders PARTITION ["c"] {"id":"4","customer":"c","total":40}"#,
            ]
        );

        let summary = apply_changes(&connection, KEY, &table, &changes).await.unwrap();
        assert_eq!(summary, ApplySummary { updated: 1, inserted: 2, deleted: 1 });
        let rows = rows(&connection, "dbforge_edit").await;
        assert_eq!(rows.len(), 4);
        // The columns are id, customer, total, tags: `note` is gone, `tags` is null.
        assert_eq!(rows[0], [Some("1".into()), Some("a".into()), Some("15".into()), None]);
        assert_eq!(rows[2][..3], [Some("4".into()), Some("c".into()), Some("40".into())]);
        // The new item without an id got one.
        assert!(rows.iter().any(|row| row[1].as_deref() == Some("d") && row[0].as_ref().unwrap().len() == 36));
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn replaces_and_adds_whole_documents() {
        let connection = database("dbforge_edit_documents", &[("orders", "/customer", orders())]).await;
        let table = table("dbforge_edit_documents", "orders");
        let changes = vec![
            RowChange::Replace {
                key: map(&[("id", Some("1")), ("customer", Some("a"))]),
                // The service's properties are dropped; a property left out is gone.
                document: r#"{"id": "1", "customer": "a", "total": 11, "items": [{"sku": "x"}], "_etag": "old"}"#
                    .into(),
            },
            RowChange::InsertDocument { document: r#"{"customer": "n", "nested": {"deep": true}}"#.into() },
        ];
        let preview = preview_changes(&connection, KEY, &table, &changes).await.unwrap();
        assert_eq!(
            preview[0],
            r#"REPLACE orders/1 PARTITION ["a"] {"id":"1","customer":"a","total":11,"items":[{"sku":"x"}]}"#
        );
        let summary = apply_changes(&connection, KEY, &table, &changes).await.unwrap();
        assert_eq!(summary, ApplySummary { updated: 1, inserted: 1, deleted: 0 });

        let filter = RowFilter { filter: String::new(), order: "c.id".into() };
        let page = fetch_rows(&connection, KEY, &table, &filter, 0, 50).await.unwrap();
        let documents = page.documents.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&documents[0]).unwrap(),
            json!({"id": "1", "customer": "a", "total": 11, "items": [{"sku": "x"}]})
        );
        assert!(documents.iter().any(|d| d.contains("\"deep\": true")));

        let bad = |document: &str| {
            vec![RowChange::Replace {
                key: map(&[("id", Some("2")), ("customer", Some("b"))]),
                document: document.into(),
            }]
        };
        let error = apply_changes(&connection, KEY, &table, &bad("{\"id\": ")).await.unwrap_err();
        assert!(error.to_string().starts_with("The document isn't valid JSON"), "{error}");
        let error = apply_changes(&connection, KEY, &table, &bad("[1]")).await.unwrap_err();
        assert_eq!(error.to_string(), "A document is a JSON object, in braces.");
        let error = apply_changes(&connection, KEY, &table, &bad(r#"{"id": "2", "customer": "z"}"#)).await.unwrap_err();
        assert!(error.to_string().starts_with("Changing an item's id or partition key"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn refuses_without_writing_anything() {
        let connection = database("dbforge_edit_refused", &[("orders", "/customer", orders())]).await;
        let table = table("dbforge_edit_refused", "orders");
        let update = |key: &[(&str, Option<&str>)], property: &str, value: &str| RowChange::Update {
            key: map(key),
            values: [(property.to_string(), CellValue::Text(value.into()))].into(),
        };

        let stale = vec![
            RowChange::Delete { key: map(&[("id", Some("3")), ("customer", Some("a"))]) },
            update(&[("id", Some("1")), ("customer", Some("other"))], "total", "1"),
        ];
        let error = apply_changes(&connection, KEY, &table, &stale).await.unwrap_err();
        assert!(error.to_string().contains("with id = 1, customer = other was changed or deleted"), "{error}");

        let moved = vec![update(&[("id", Some("1")), ("customer", Some("a"))], "customer", "z")];
        let error = apply_changes(&connection, KEY, &table, &moved).await.unwrap_err();
        assert!(error.to_string().starts_with("Changing an item's id or partition key moves it"), "{error}");

        let wrong = vec![update(&[("id", Some("1")), ("customer", Some("a"))], "total", "ten")];
        let error = apply_changes(&connection, KEY, &table, &wrong).await.unwrap_err();
        assert!(error.to_string().contains("holds number values"), "{error}");

        // Nothing was written: item 3 is still there and 1 kept its total.
        let rows = rows(&connection, "dbforge_edit_refused").await;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][2].as_deref(), Some("10"));
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn a_failed_write_says_how_many_went_through() {
        let connection = database("dbforge_edit_partial", &[("orders", "/customer", orders())]).await;
        let table = table("dbforge_edit_partial", "orders");
        // The second insert reuses an id in the same partition, which only the write finds out.
        let changes = vec![
            RowChange::Insert { values: map(&[("id", Some("9")), ("customer", Some("a"))]) },
            RowChange::Insert { values: map(&[("id", Some("1")), ("customer", Some("a"))]) },
        ];
        let error = apply_changes(&connection, KEY, &table, &changes).await.unwrap_err().to_string();
        assert!(error.starts_with("Applied 1 of 2 changes, then: "), "{error}");
        assert_eq!(rows(&connection, "dbforge_edit_partial").await.len(), 4);
    }
}
