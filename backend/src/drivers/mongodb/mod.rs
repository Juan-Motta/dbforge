//! MongoDB, through the official driver. A server holds databases that hold
//! collections, so the explorer lists databases and then their collections, and
//! a collection's `TableRef` carries the database name in both `database` and
//! `schema`, as MySQL's tables do.
//!
//! Documents are shown as rows: each top-level field is a column and nested
//! values are written as JSON, in the syntax mongosh reads (see [`literal`]).

mod edit;
mod explain;
mod session;
mod shell;
mod structure;

pub use edit::{apply_changes, preview_changes};
pub use explain::explain;
pub use session::{Session, cancel, open_session};
pub use structure::table_structure;

use std::collections::BTreeSet;
use std::time::Duration;

use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::{Client, Database};

use super::{Column, DbObject, ObjectKind, Relation, RelationList, RowSink, RowsPage, TableRef, with_timeout};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Databases the server keeps for itself; never shown.
const SYSTEM_DATABASES: [&str; 3] = ["admin", "config", "local"];

/// A client for the connection. Users authenticate against `admin`, where
/// mongod's users usually live. Hosts saved as a whole `mongodb://` or
/// `mongodb+srv://` URL, before the form split URLs into fields, still work.
async fn connect(connection: &Connection, password: Option<&str>) -> Result<Client> {
    Ok(Client::with_options(options(connection, password).await?)?)
}

async fn options(connection: &Connection, password: Option<&str>) -> Result<ClientOptions> {
    let uri = is_uri(&connection.host);
    if connection.tls || connection.srv || uri {
        super::install_crypto();
    }
    let mut options = if uri {
        ClientOptions::parse(&connection.host).await?
    } else if connection.srv {
        // The seed list's DNS records name the servers, their port and options.
        ClientOptions::parse(format!("mongodb+srv://{}/", connection.host)).await?
    } else {
        let mut options = ClientOptions::default();
        options.hosts = vec![ServerAddress::Tcp { host: connection.host.clone(), port: Some(connection.port) }];
        // Talk to this server even when it belongs to a replica set whose
        // members advertise names only reachable from inside their network.
        options.direct_connection = Some(true);
        options
    };
    if !uri && !connection.username.is_empty() {
        options.credential = Some(
            Credential::builder()
                .username(connection.username.clone())
                .password(password.map(String::from))
                .source("admin".to_string())
                .build(),
        );
    }
    if password.is_some() && options.credential.as_ref().is_some_and(|c| c.password.is_none()) {
        options.credential.as_mut().expect("checked").password = password.map(String::from);
    }
    if connection.tls && !uri {
        let mut tls = TlsOptions::default();
        tls.allow_invalid_certificates = Some(connection.allow_self_signed);
        options.tls = Some(Tls::Enabled(tls));
    }
    options.app_name = Some("DBForge".into());
    // Fail fast when the server is unreachable instead of waiting 30 seconds.
    options.server_selection_timeout = Some(Duration::from_secs(8));
    options.connect_timeout = Some(Duration::from_secs(8));
    Ok(options)
}

fn is_uri(host: &str) -> bool {
    host.starts_with("mongodb://") || host.starts_with("mongodb+srv://")
}

/// The database to use when none is named: the connection's, or the one in its
/// connection string.
fn default_database(connection: &Connection, client: &Client) -> Option<String> {
    Some(connection.database.clone())
        .filter(|d| !d.is_empty())
        .or_else(|| client.default_database().map(|d| d.name().to_string()))
}

pub async fn server_version(connection: &Connection, password: Option<&str>) -> Result<String> {
    let client = connect(connection, password).await?;
    let info = client.database("admin").run_command(doc! { "buildInfo": 1 }).await?;
    // Listing databases also proves the credentials are right.
    client.list_database_names().await?;
    client.shutdown().await;
    Ok(format!("MongoDB {}", info.get_str("version").unwrap_or("?")))
}

/// `[]` → databases, `[database]` → its collections and views.
pub async fn list_objects(connection: &Connection, password: Option<&str>, path: &[String]) -> Result<Vec<DbObject>> {
    let client = connect(connection, password).await?;
    let objects = match path {
        [] => {
            let mut names = client.list_database_names().await?;
            names.retain(|name| !SYSTEM_DATABASES.contains(&name.as_str()));
            names.sort();
            names.into_iter().map(|name| DbObject::new(name, ObjectKind::Database)).collect()
        }
        [database] => {
            let db = client.database(database);
            let mut objects = Vec::new();
            for (name, is_view) in collections(&db).await? {
                // The count kept in collection metadata: instant, like reltuples.
                let row_estimate = if is_view {
                    None
                } else {
                    Some(db.collection::<Document>(&name).estimated_document_count().await? as i64)
                };
                let kind = if is_view { ObjectKind::View } else { ObjectKind::Table };
                objects.push(DbObject { name, kind, row_estimate });
            }
            objects
        }
        _ => return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    };
    client.shutdown().await;
    Ok(objects)
}

/// Collections and views by name, without the server's own `system.*` ones.
async fn collections(db: &Database) -> Result<Vec<(String, bool)>> {
    let mut specs: Vec<(String, bool)> = db
        .list_collections()
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .filter(|spec| !spec.name.starts_with("system."))
        .map(|spec| (spec.name, matches!(spec.collection_type, mongodb::results::CollectionType::View)))
        .collect();
    specs.sort();
    Ok(specs)
}

/// Every collection and view of `database` (the connection's own when `None`), for search.
pub async fn list_relations(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
) -> Result<RelationList> {
    let client = connect(connection, password).await?;
    let database = database
        .map(String::from)
        .or_else(|| default_database(connection, &client))
        .ok_or_else(|| AppError::Invalid("Select a database to search its collections.".into()))?;
    let relations = collections(&client.database(&database))
        .await?
        .into_iter()
        .map(|(name, is_view)| Relation {
            schema: database.clone(),
            name,
            kind: if is_view { ObjectKind::View } else { ObjectKind::Table },
        })
        .collect();
    client.shutdown().await;
    Ok(RelationList { database, relations })
}

/// Reads one page of a collection in `_id` order. The columns are the fields
/// found in that page.
pub async fn fetch_rows(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let client = connect(connection, password).await?;
    let mut documents: Vec<Document> = client
        .database(&table.database)
        .collection::<Document>(&table.name)
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .skip(offset)
        // One extra document tells whether another page exists.
        .limit(i64::from(limit) + 1)
        .await?
        .try_collect()
        .await?;
    client.shutdown().await;

    let has_more = documents.len() > limit as usize;
    documents.truncate(limit as usize);
    let columns = columns_of(&documents);
    let rows = documents.iter().map(|document| row(document, &columns)).collect();
    Ok(RowsPage { columns, rows, has_more })
}

/// Sends every document to `sink`. The columns are every top-level field used
/// anywhere in the collection, found first by the server in one pass.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    sink: &mut impl RowSink,
) -> Result<()> {
    let client = with_timeout(connect(connection, password)).await?;
    let collection = client.database(&table.database).collection::<Document>(&table.name);
    let pipeline = vec![
        doc! { "$project": { "fields": { "$objectToArray": "$$ROOT" } } },
        doc! { "$unwind": "$fields" },
        doc! { "$group": { "_id": "$fields.k" } },
    ];
    let names: BTreeSet<String> = collection
        .aggregate(pipeline)
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .iter()
        .filter_map(|group| group.get_str("_id").ok().map(String::from))
        .collect();
    let first = collection.find_one(doc! {}).sort(doc! { "_id": 1 }).await?;
    let columns = ordered_columns(first.iter(), names);
    sink.columns(&columns)?;

    let mut cursor = collection.find(doc! {}).sort(doc! { "_id": 1 }).await?;
    while let Some(document) = cursor.try_next().await? {
        sink.row(&row(&document, &columns))?;
    }
    // shutdown() waits for every cursor of the client to be dropped.
    drop(cursor);
    client.shutdown().await;
    Ok(())
}

/// The fields of `documents`, `_id` first and then in the order they first
/// appear, each with the type its values share (`mixed` when they differ).
fn columns_of(documents: &[Document]) -> Vec<Column> {
    let mut columns = ordered_columns(documents.iter(), BTreeSet::new());
    for column in &mut columns {
        let types: BTreeSet<&str> =
            documents.iter().filter_map(|d| d.get(&column.name)).filter(|v| !is_null(v)).map(type_name).collect();
        column.data_type = match types.len() {
            0 => "null".into(),
            1 => types.into_iter().next().expect("one").into(),
            _ => "mixed".into(),
        };
    }
    columns
}

/// Field names in order of first appearance in `documents`, then `extra` ones,
/// with `_id` always first.
fn ordered_columns<'a>(documents: impl Iterator<Item = &'a Document>, extra: BTreeSet<String>) -> Vec<Column> {
    let mut names: Vec<String> = vec!["_id".into()];
    for document in documents {
        for key in document.keys() {
            if !names.contains(key) {
                names.push(key.clone());
            }
        }
    }
    for name in extra {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| Column { primary_key: name == "_id", name, data_type: String::new(), generated: false })
        .collect()
}

fn row(document: &Document, columns: &[Column]) -> Vec<Option<String>> {
    columns.iter().map(|column| document.get(&column.name).and_then(text)).collect()
}

fn is_null(value: &Bson) -> bool {
    matches!(value, Bson::Null | Bson::Undefined)
}

/// The type's name as MongoDB's `$type` spells it.
fn type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Boolean(_) => "bool",
        Bson::Null | Bson::Undefined => "null",
        Bson::RegularExpression(_) => "regex",
        Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => "javascript",
        Bson::Int32(_) => "int",
        Bson::Int64(_) => "long",
        Bson::Timestamp(_) => "timestamp",
        Bson::Binary(_) => "binData",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::Symbol(_) => "symbol",
        Bson::Decimal128(_) => "decimal",
        Bson::MaxKey => "maxKey",
        Bson::MinKey => "minKey",
        Bson::DbPointer(_) => "dbPointer",
    }
}

/// A top-level value as a grid cell: strings, numbers, dates and ids as they
/// are, so they read naturally; anything else as a mongosh literal.
fn text(value: &Bson) -> Option<String> {
    Some(match value {
        Bson::Null | Bson::Undefined => return None,
        Bson::String(text) => text.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Decimal128(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::ObjectId(id) => id.to_hex(),
        Bson::DateTime(date) => date.try_to_rfc3339_string().unwrap_or_else(|_| date.timestamp_millis().to_string()),
        other => literal(other),
    })
}

/// A value in the syntax mongosh reads back: JSON plus `ObjectId("…")`,
/// `ISODate("…")`, `NumberDecimal("…")` and friends for the types JSON lacks.
pub fn literal(value: &Bson) -> String {
    let quote = |text: &str| serde_json::Value::from(text).to_string();
    match value {
        Bson::Null | Bson::Undefined => "null".into(),
        Bson::String(text) => quote(text),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => format!("NumberLong({n})"),
        Bson::Double(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.1}"),
        Bson::Double(n) => n.to_string(),
        Bson::Decimal128(n) => format!("NumberDecimal({})", quote(&n.to_string())),
        Bson::Boolean(b) => b.to_string(),
        Bson::ObjectId(id) => format!("ObjectId({})", quote(&id.to_hex())),
        Bson::DateTime(date) => match date.try_to_rfc3339_string() {
            Ok(iso) => format!("ISODate({})", quote(&iso)),
            Err(_) => format!("new Date({})", date.timestamp_millis()),
        },
        Bson::Array(items) => format!("[{}]", items.iter().map(literal).collect::<Vec<_>>().join(", ")),
        Bson::Document(document) => {
            let fields: Vec<String> =
                document.iter().map(|(key, value)| format!("{}: {}", quote(key), literal(value))).collect();
            format!("{{{}}}", fields.join(", "))
        }
        // The rest has no short mongosh form worth reading; Extended JSON says it exactly.
        other => other.clone().into_relaxed_extjson().to_string(),
    }
}

/// Runs against a real server. Start one and run `cargo test -- --ignored`:
/// `docker run -d --name dbforge-test-mongo -e MONGO_INITDB_ROOT_USERNAME=root -e MONGO_INITDB_ROOT_PASSWORD=dbforge -p 57017:27017 mongo:8`
/// Each test creates its own database.
#[cfg(test)]
pub(super) mod tests {
    use mongodb::bson::oid::ObjectId;
    use mongodb::bson::{DateTime, Decimal128};

    use super::*;
    use crate::connections::model::Engine;

    pub const PASSWORD: Option<&str> = Some("dbforge");

    pub fn local() -> Connection {
        Connection {
            id: "test".into(),
            name: "test".into(),
            engine: Engine::Mongodb,
            environment: None,
            // Not `localhost`: Docker on macOS answers slowly on IPv6, which is tried first.
            host: "127.0.0.1".into(),
            port: 57017,
            database: String::new(),
            username: "root".into(),
            tls: false,
            allow_self_signed: false,
            srv: false,
        }
    }

    /// A fresh database named `name` with `documents` in each collection.
    pub async fn database(name: &str, collections: &[(&str, Vec<Document>)]) -> Connection {
        let client = connect(&local(), PASSWORD).await.unwrap();
        let db = client.database(name);
        db.drop().await.unwrap();
        for (collection, documents) in collections {
            db.create_collection(*collection).await.unwrap();
            if !documents.is_empty() {
                db.collection::<Document>(collection).insert_many(documents).await.unwrap();
            }
        }
        client.shutdown().await;
        Connection { database: name.into(), ..local() }
    }

    pub fn table(database: &str, name: &str) -> TableRef {
        TableRef { database: database.into(), schema: database.into(), name: name.into() }
    }

    fn names(objects: &[DbObject]) -> Vec<&str> {
        objects.iter().map(|o| o.name.as_str()).collect()
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn lists_databases_collections_and_views() {
        let connection =
            database("dbforge_list", &[("items", vec![doc! { "a": 1 }, doc! { "a": 2 }]), ("empty", vec![])]).await;
        let client = connect(&connection, PASSWORD).await.unwrap();
        client
            .database("dbforge_list")
            .create_collection("big")
            .view_on("items".to_string())
            .pipeline(vec![doc! { "$match": { "a": { "$gt": 1 } } }])
            .await
            .unwrap();
        client.shutdown().await;

        assert!(server_version(&connection, PASSWORD).await.unwrap().starts_with("MongoDB 8."));
        let databases = list_objects(&connection, PASSWORD, &[]).await.unwrap();
        assert!(names(&databases).contains(&"dbforge_list"));
        assert!(!names(&databases).contains(&"admin"));

        let objects = list_objects(&connection, PASSWORD, &["dbforge_list".into()]).await.unwrap();
        assert_eq!(
            objects,
            [
                DbObject { name: "big".into(), kind: ObjectKind::View, row_estimate: None },
                DbObject { name: "empty".into(), kind: ObjectKind::Table, row_estimate: Some(0) },
                DbObject { name: "items".into(), kind: ObjectKind::Table, row_estimate: Some(2) },
            ]
        );

        let relations = list_relations(&connection, PASSWORD, None).await.unwrap();
        assert_eq!(relations.database, "dbforge_list");
        assert_eq!(relations.relations.len(), 3);
    }

    #[tokio::test]
    async fn tls_options_reach_the_driver() {
        let plain = options(&local(), PASSWORD).await.unwrap();
        assert!(plain.tls.is_none());
        let tls = Connection { tls: true, allow_self_signed: true, ..local() };
        let Some(Tls::Enabled(settings)) = options(&tls, PASSWORD).await.unwrap().tls else { panic!("no TLS") };
        assert_eq!(settings.allow_invalid_certificates, Some(true));
        let verified = Connection { allow_self_signed: false, ..tls };
        let Some(Tls::Enabled(settings)) = options(&verified, PASSWORD).await.unwrap().tls else { panic!("no TLS") };
        assert_eq!(settings.allow_invalid_certificates, Some(false));
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn wrong_password_is_reported() {
        let error = server_version(&local(), Some("wrong")).await.unwrap_err();
        assert!(error.to_string().contains("Authentication failed"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn reads_documents_as_rows() {
        let id = ObjectId::parse_str("65a0f0f0f0f0f0f0f0f0f0f0").unwrap();
        let documents = vec![
            doc! {
                "_id": 1, "name": "Ana", "total": Decimal128::from_bytes([0; 16]), "tags": ["vip"],
                "address": { "city": "CDMX" }, "ref": id, "seen": DateTime::from_millis(1_700_000_000_000),
            },
            doc! { "_id": 2, "name": "Bob", "total": 5.5, "extra": true, "tags": [] },
            doc! { "_id": 3, "name": null },
        ];
        let connection = database("dbforge_rows", &[("people", documents)]).await;
        let people = table("dbforge_rows", "people");

        let page = fetch_rows(&connection, PASSWORD, &people, 0, 2).await.unwrap();
        let columns: Vec<_> =
            page.columns.iter().map(|c| (c.name.as_str(), c.data_type.as_str(), c.primary_key)).collect();
        assert_eq!(
            columns,
            [
                ("_id", "int", true),
                ("name", "string", false),
                ("total", "mixed", false),
                ("tags", "array", false),
                ("address", "object", false),
                ("ref", "objectId", false),
                ("seen", "date", false),
                ("extra", "bool", false),
            ]
        );
        let some = |v: &str| Some(v.to_string());
        assert_eq!(
            page.rows[0],
            [
                some("1"),
                some("Ana"),
                some("0E-6176"),
                some(r#"["vip"]"#),
                some(r#"{"city": "CDMX"}"#),
                some("65a0f0f0f0f0f0f0f0f0f0f0"),
                some("2023-11-14T22:13:20Z"),
                None,
            ]
        );
        assert_eq!(page.rows[1][2], some("5.5"));
        assert!(page.has_more);

        let last = fetch_rows(&connection, PASSWORD, &people, 2, 2).await.unwrap();
        assert_eq!((last.rows.len(), last.has_more), (1, false));
    }

    #[derive(Default)]
    struct Collected(Vec<Column>, Vec<Vec<Option<String>>>);

    impl RowSink for Collected {
        fn columns(&mut self, columns: &[Column]) -> Result<()> {
            self.0 = columns.to_vec();
            Ok(())
        }
        fn row(&mut self, values: &[Option<String>]) -> Result<()> {
            self.1.push(values.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn export_streams_every_field_of_every_document() {
        let documents = vec![doc! { "_id": 1, "b": 1 }, doc! { "_id": 2, "z": "late", "a": 2 }];
        let connection = database("dbforge_stream", &[("things", documents)]).await;
        let mut sink = Collected::default();
        stream_table(&connection, PASSWORD, &table("dbforge_stream", "things"), &mut sink).await.unwrap();
        let names: Vec<_> = sink.0.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["_id", "b", "a", "z"]);
        assert_eq!(sink.1[1], [Some("2".into()), None, Some("2".into()), Some("late".into())]);
    }

    #[test]
    fn literals_read_back_in_mongosh() {
        let id = ObjectId::parse_str("65a0f0f0f0f0f0f0f0f0f0f0").unwrap();
        let value = Bson::Document(doc! {
            "id": id, "n": 2_i64, "x": 1.0, "s": "a\"b", "d": DateTime::from_millis(0), "list": [1, null],
        });
        assert_eq!(
            literal(&value),
            r#"{"id": ObjectId("65a0f0f0f0f0f0f0f0f0f0f0"), "n": NumberLong(2), "x": 1.0, "s": "a\"b", "d": ISODate("1970-01-01T00:00:00Z"), "list": [1, null]}"#
        );
    }
}
