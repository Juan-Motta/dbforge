//! Azure Cosmos DB, API for NoSQL, through its REST API (see [`client`]). An
//! account holds databases that hold containers of JSON items, so the explorer
//! lists databases and then their containers, and a container's `TableRef`
//! carries the database name in both `database` and `schema`, as MongoDB's
//! collections do.
//!
//! Items are shown as rows: each top-level property is a column, `id` and the
//! partition key first, and nested values are written as JSON.

mod admin;
mod client;
mod edit;
mod explain;
mod logged;
mod query;
mod read_only;
mod session;
mod structure;

pub use admin::create_database;
pub use edit::{apply_changes, preview_changes};
pub use explain::explain;
pub use read_only::run_read_only;
pub use session::{Session, open_session};
pub use structure::table_structure;

use std::collections::BTreeSet;

use serde_json::Value;

use self::client::{Client, Target};
use self::query::{Items, Select};
use super::row_filter::RowFilter;
use super::{
    Column, DbObject, ObjectKind, Relation, RelationList, RowSink, RowsPage, TableRef, each_concurrently, with_timeout,
};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Properties the service adds to every item; never shown.
const SYSTEM_PROPERTIES: [&str; 5] = ["_rid", "_self", "_etag", "_attachments", "_ts"];
/// Items asked for per request when reading a whole container.
const PAGE: u32 = 1000;

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let client = Client::new(connection, password)?;
    let account = client.get(&Target::account(), &[]).await?.body;
    // Listing databases also proves the key is right.
    client.list(&Target::databases(), "Databases").await?;
    let name = account.get("id").and_then(Value::as_str).unwrap_or("?");
    Ok(format!("Azure Cosmos DB, account {name}"))
}

/// `[]` → databases, `[database]` → its containers.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    let client = Client::new(connection, password)?;
    match path {
        [] => {
            let mut names = ids(client.list(&Target::databases(), "Databases").await?);
            names.sort();
            Ok(names.into_iter().map(|name| DbObject::new(name, ObjectKind::Database)).collect())
        }
        [database] => {
            // One request per container, so they are sent a few at a time.
            each_concurrently(containers(&client, database).await?, |name| {
                let client = &client;
                async move {
                    let row_estimate = item_count(client, database, &name).await?;
                    Ok(DbObject { name, kind: ObjectKind::Table, row_estimate })
                }
            })
            .await
        }
        _ => Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    }
}

/// Every container of `database` (the connection's own when `None`), for search.
pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    let database = database
        .filter(|d| !d.is_empty())
        .or(Some(connection.database.as_str()).filter(|d| !d.is_empty()))
        .ok_or_else(|| AppError::Invalid("Select a database to search its containers.".into()))?
        .to_string();
    let client = Client::new(connection, password)?;
    let relations = containers(&client, &database)
        .await?
        .into_iter()
        .map(|name| Relation { schema: database.clone(), name, kind: ObjectKind::Table })
        .collect();
    Ok(RelationList { database, relations })
}

async fn containers(client: &Client, database: &str) -> Result<Vec<String>> {
    let mut names = ids(client.list(&Target::containers(database), "DocumentCollections").await?);
    names.sort();
    Ok(names)
}

/// The item count the account keeps for a container: instant, and like
/// reltuples only an estimate, which Azure updates in the background.
async fn item_count(client: &Client, database: &str, container: &str) -> Result<Option<i64>> {
    let reply =
        client.get(&Target::container(database, container), &[("x-ms-documentdb-populatequotainfo", "true")]).await?;
    Ok(reply.header("x-ms-resource-usage").and_then(|usage| usage_value(usage, "documentsCount")))
}

/// One value of a usage header: `documentsCount=6;documentsSize=1`.
fn usage_value(usage: &str, name: &str) -> Option<i64> {
    usage.split(';').find_map(|pair| pair.split_once('=').filter(|(key, _)| *key == name)?.1.parse().ok())
}

/// Reads one page of a container. The columns are the properties found in that page.
pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let select = Select::of(filter);
    let client = Client::new(connection, password)?;
    let keys = partition_key(&client, table).await?;
    // Reaching a later page reads the earlier ones: Cosmos DB has no cheap skip.
    let wanted = offset.saturating_add(u64::from(limit) + 1);
    let mut items = Items::open(&client, &table.database, &table.name, &select, PAGE.min(wanted as u32)).await?;
    for _ in 0..offset {
        if items.next().await?.is_none() {
            break;
        }
    }
    let mut page = Vec::new();
    // One extra item tells whether another page exists.
    while page.len() <= limit as usize {
        match items.next().await? {
            Some(item) => page.push(item),
            None => break,
        }
    }
    let has_more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let columns = columns_of(&page, &keys);
    let rows = page.iter().map(|item| row(item, &columns)).collect();
    let documents = page.iter().map(document_text).collect();
    Ok(RowsPage { columns, rows, has_more, documents: Some(documents) })
}

/// Sends every item to `sink`. The columns are every top-level property used
/// anywhere in the result, so the items are read twice: once for the names.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    sink: &mut impl RowSink,
) -> Result<()> {
    let select = Select::of(filter);
    let client = Client::new(connection, password)?;
    let keys = with_timeout(partition_key(&client, table)).await?;
    let mut names = BTreeSet::new();
    let mut first = None;
    let mut items = Items::open(&client, &table.database, &table.name, &select.unordered(), PAGE).await?;
    while let Some(item) = items.next().await? {
        if let Value::Object(properties) = &item {
            names.extend(properties.keys().cloned());
        }
        first.get_or_insert(item);
    }
    let columns = ordered_columns(first.iter(), &keys, names);
    sink.columns(&columns)?;

    let mut items = Items::open(&client, &table.database, &table.name, &select, PAGE).await?;
    while let Some(item) = items.next().await? {
        sink.row(&row(&item, &columns))?;
    }
    Ok(())
}

/// The top-level properties of the container's partition key. A nested path
/// (`/address/city`) has no column of its own.
async fn partition_key(client: &Client, table: &TableRef) -> Result<Vec<String>> {
    let container = client.get(&Target::container(&table.database, &table.name), &[]).await?.body;
    Ok(key_properties(&container))
}

fn key_properties(container: &Value) -> Vec<String> {
    let paths = container.pointer("/partitionKey/paths").and_then(Value::as_array).cloned().unwrap_or_default();
    paths
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|path| path.strip_prefix('/'))
        .filter(|name| !name.contains('/'))
        .map(|name| name.trim_matches('"').to_string())
        .collect()
}

/// The properties of `items`, `id` and the partition key first and then in
/// the order they first appear, each with the JSON type its values share
/// (`mixed` when they differ).
fn columns_of(items: &[Value], keys: &[String]) -> Vec<Column> {
    typed(ordered_columns(items.iter(), keys, BTreeSet::new()), items)
}

/// `columns` with the JSON type their values share in `items`.
fn typed(mut columns: Vec<Column>, items: &[Value]) -> Vec<Column> {
    for column in &mut columns {
        column.data_type = shared_type(items.iter().filter_map(|item| item.get(&column.name)));
    }
    columns
}

/// The JSON type `values` share, ignoring nulls: `mixed` when they differ.
fn shared_type<'a>(values: impl Iterator<Item = &'a Value>) -> String {
    let types: BTreeSet<&str> = values.filter(|value| !value.is_null()).map(type_name).collect();
    match types.len() {
        0 => "null".into(),
        1 => types.into_iter().next().expect("one").into(),
        _ => "mixed".into(),
    }
}

/// Property names in order of first appearance in `items`, then `extra` ones,
/// with `id` and the partition key first and the service's own left out. They
/// are the key a row is found by, so they are marked as its primary key.
fn ordered_columns<'a>(
    items: impl Iterator<Item = &'a Value>,
    keys: &[String],
    extra: BTreeSet<String>,
) -> Vec<Column> {
    let mut names: Vec<String> = vec!["id".into()];
    let found = items.filter_map(Value::as_object).flat_map(|properties| properties.keys().cloned());
    for name in keys.iter().cloned().chain(found).chain(extra) {
        if !names.contains(&name) && !SYSTEM_PROPERTIES.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| Column {
            primary_key: name == "id" || keys.contains(&name),
            name,
            data_type: String::new(),
            generated: false,
            default: None,
        })
        .collect()
}

/// An item as the JSON view shows it: formatted, without the service's properties.
fn document_text(item: &Value) -> String {
    serde_json::to_string_pretty(&without_system_properties(item.clone())).unwrap_or_default()
}

fn without_system_properties(mut item: Value) -> Value {
    if let Some(properties) = item.as_object_mut() {
        properties.retain(|name, _| !SYSTEM_PROPERTIES.contains(&name.as_str()));
    }
    item
}

fn row(item: &Value, columns: &[Column]) -> Vec<Option<String>> {
    columns.iter().map(|column| item.get(&column.name).and_then(text)).collect()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A top-level value as a grid cell: strings, numbers and booleans as they
/// are, so they read naturally; arrays and objects as JSON.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn ids(resources: Vec<Value>) -> Vec<String> {
    resources.iter().filter_map(|resource| resource.get("id").and_then(Value::as_str).map(String::from)).collect()
}

/// Runs against the Linux emulator. Start it and run `cargo test -- --ignored`:
/// `docker run -d --rm --name dbforge-test-cosmos -p 58081:8081 mcr.microsoft.com/cosmosdb/linux/azure-cosmos-emulator:vnext-preview`
/// Each test creates its own database.
#[cfg(test)]
pub(super) mod tests {
    use reqwest::Method;
    use serde_json::json;

    use super::*;
    use crate::connections::model::Engine;
    use crate::drivers::TableRef;

    /// The emulator's well-known key, the same in every copy of it.
    pub const KEY: Option<&str> =
        Some("C2y6yDjf5/R+ob0N8A7Cgv30VRDJIWEHLM+4QDU5DE2nQ9nDuVTqobD4b8mGGyPMbIZnqyMsEcaGQy67XIw/Jw==");

    pub fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Cosmosdb,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 58081,
            database: String::new(),
            username: String::new(),
            tls: false,
            allow_self_signed: false,
            srv: false,
            agent_access: Default::default(),
        }
    }

    /// A fresh database named `name`, with each container's partition key path and items.
    pub async fn database(name: &str, containers: &[(&str, &str, Vec<Value>)]) -> Connection {
        let client = Client::new(&local(), KEY).unwrap();
        match client.send(Method::DELETE, &Target::database(name), &[], None).await {
            Ok(_) => {}
            Err(error) if error.to_string().contains("not found") => {}
            Err(error) => panic!("{error}"),
        }
        client.send(Method::POST, &Target::databases(), &[], Some(&json!({ "id": name }))).await.unwrap();
        for (container, key, items) in containers {
            let definition = json!({ "id": container, "partitionKey": { "paths": [key], "kind": "Hash" } });
            client.send(Method::POST, &Target::containers(name), &[], Some(&definition)).await.unwrap();
            for item in items {
                let value = item.pointer(key).cloned().unwrap_or(Value::Null);
                let partition = super::logged::partition_header(&json!([value]));
                client
                    .send(
                        Method::POST,
                        &Target::items(name, container),
                        &[(super::logged::PARTITION_KEY, &partition)],
                        Some(item),
                    )
                    .await
                    .unwrap();
            }
        }
        Connection { database: name.into(), ..local() }
    }

    pub fn table(database: &str, name: &str) -> TableRef {
        TableRef { database: database.into(), schema: database.into(), name: name.into() }
    }

    fn names(objects: &[DbObject]) -> Vec<&str> {
        objects.iter().map(|o| o.name.as_str()).collect()
    }

    #[test]
    fn reads_usage_headers() {
        let usage = "documentSize=0;documentsSize=1;documentsCount=6;collectionSize=2";
        assert_eq!(usage_value(usage, "documentsCount"), Some(6));
        assert_eq!(usage_value(usage, "missing"), None);
    }

    #[test]
    fn rows_put_the_key_first_and_hide_system_properties() {
        let items = [
            json!({"id": "1", "total": 5, "tags": ["a"], "customer": "x", "_rid": "r", "_ts": 1}),
            json!({"id": "2", "total": "n/a", "note": null, "customer": "y", "address": {"city": "Lima"}}),
        ];
        let columns = columns_of(&items, &["customer".into()]);
        let described: Vec<_> =
            columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.primary_key)).collect();
        assert_eq!(
            described,
            [
                ("id", "string", true),
                ("customer", "string", true),
                ("total", "mixed", false),
                ("tags", "array", false),
                ("note", "null", false),
                ("address", "object", false),
            ]
        );
        assert_eq!(
            row(&items[1], &columns),
            [Some("2".into()), Some("y".into()), Some("n/a".into()), None, None, Some(r#"{"city":"Lima"}"#.into())]
        );
        assert_eq!(row(&items[0], &columns)[2..4], [Some("5".into()), Some(r#"["a"]"#.into())]);
    }

    #[test]
    fn partition_keys_are_columns_only_at_the_top_level() {
        let single = json!({"partitionKey": {"paths": ["/customer"], "kind": "Hash"}});
        assert_eq!(key_properties(&single), ["customer"]);
        let hierarchical = json!({"partitionKey": {"paths": ["/tenant", "/address/city"], "kind": "MultiHash"}});
        assert_eq!(key_properties(&hierarchical), ["tenant"]);
    }

    fn orders() -> Vec<Value> {
        (0..7).map(|i| json!({"id": i.to_string(), "customer": format!("c{}", i % 3), "total": i * 10})).collect()
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn reads_pages_filtered_and_sorted() {
        let connection = database("dbforge_rows", &[("orders", "/customer", orders())]).await;
        let table = table("dbforge_rows", "orders");
        let filter = |filter: &str, order: &str| RowFilter { filter: filter.into(), order: order.into() };

        let first = fetch_rows(&connection, KEY, &table, &filter("", "c.total DESC"), 0, 3).await.unwrap();
        let names: Vec<_> = first.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "customer", "total"]);
        assert_eq!(first.rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>(), ["6", "5", "4"]);
        // The JSON view's documents: formatted, without the service's properties.
        let documents = first.documents.as_ref().unwrap();
        assert_eq!(documents[0], "{\n  \"id\": \"6\",\n  \"customer\": \"c0\",\n  \"total\": 60\n}");
        assert!(first.has_more);

        let last = fetch_rows(&connection, KEY, &table, &filter("", "c.total DESC"), 6, 3).await.unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(!last.has_more);

        let some = fetch_rows(&connection, KEY, &table, &filter("c.total >= 30", "c.customer, c.total DESC"), 0, 10)
            .await
            .unwrap();
        let rows: Vec<_> = some.rows.iter().map(|r| (r[1].clone().unwrap(), r[2].clone().unwrap())).collect();
        assert_eq!(rows, [("c0", "60"), ("c0", "30"), ("c1", "40"), ("c2", "50")].map(|(a, b)| (a.into(), b.into())));

        let error = fetch_rows(&connection, KEY, &table, &filter("nope", ""), 0, 10).await.unwrap_err();
        assert_eq!(error.to_string(), "Identifier 'nope' could not be resolved.");
    }

    struct Collect(Vec<String>, Vec<Vec<Option<String>>>);

    impl RowSink for Collect {
        fn columns(&mut self, columns: &[Column]) -> Result<()> {
            self.0 = columns.iter().map(|c| c.name.clone()).collect();
            Ok(())
        }
        fn row(&mut self, values: &[Option<String>]) -> Result<()> {
            self.1.push(values.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn streams_every_item_with_every_property() {
        let mut items = orders();
        items.push(json!({"id": "late", "customer": "c9", "extra": [1, 2]}));
        let connection = database("dbforge_stream", &[("orders", "/customer", items)]).await;
        let filter = RowFilter { filter: String::new(), order: "c.id".into() };
        let mut sink = Collect(Vec::new(), Vec::new());
        stream_table(&connection, KEY, &table("dbforge_stream", "orders"), &filter, &mut sink).await.unwrap();
        assert_eq!(sink.0, ["id", "customer", "total", "extra"]);
        assert_eq!(sink.1.len(), 8);
        assert_eq!(sink.1.last().unwrap(), &[Some("late".into()), Some("c9".into()), None, Some("[1,2]".into())]);
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn lists_databases_and_containers() {
        let connection = database(
            "dbforge_list",
            &[
                ("orders", "/customer", vec![json!({"id": "1", "customer": "a"}), json!({"id": "2", "customer": "b"})]),
                ("empty", "/id", vec![]),
            ],
        )
        .await;

        assert!(server_version(&connection, KEY).await.unwrap().starts_with("Azure Cosmos DB, account "));
        let databases = list_objects(&connection, KEY, &[]).await.unwrap();
        assert!(names(&databases).contains(&"dbforge_list"));
        assert!(databases.iter().all(|d| d.kind == ObjectKind::Database));

        let containers = list_objects(&connection, KEY, &["dbforge_list".into()]).await.unwrap();
        assert_eq!(names(&containers), ["empty", "orders"]);
        assert_eq!(containers[1].kind, ObjectKind::Table);
        // The emulator reports 0 items; Azure updates the count in the background.
        assert!(containers[1].row_estimate.is_some());

        let relations = list_relations(&connection, KEY, None).await.unwrap();
        assert_eq!(relations.database, "dbforge_list");
        assert_eq!(relations.relations.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["empty", "orders"]);
        assert_eq!(relations.relations[0].schema, "dbforge_list");
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn explains_a_bad_key_a_missing_database_or_no_server() {
        // The emulator takes any key, so a wrong one is covered by `client`'s tests.
        let error = server_version(&local(), Some("not base64!")).await.unwrap_err().to_string();
        assert!(error.contains("account key isn't valid"), "{error}");

        let error = list_objects(&local(), KEY, &["dbforge_missing".into()]).await.unwrap_err().to_string();
        assert!(!error.contains("ActivityId"), "{error}");

        let unreachable = Connection { port: 1, ..local() };
        let error = server_version(&unreachable, KEY).await.unwrap_err().to_string();
        assert!(error.starts_with("Could not reach the server"), "{error}");
    }
}
