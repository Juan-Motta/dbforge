//! MongoDB, through the official driver. A server holds databases that hold
//! collections, so the explorer lists databases and then their collections, and
//! a collection's `TableRef` carries the database name in both `database` and
//! `schema`, as MySQL's tables do.
//!
//! Documents are shown as rows: each top-level field is a column and nested
//! values are written as JSON, in the syntax mongosh reads (see [`literal`]).

mod admin;
mod edit;
mod explain;
mod logged;
mod read_only;
mod session;
mod shell;
mod structure;

pub use admin::create_object;
pub use edit::{apply_changes, preview_changes};
pub use explain::explain;
pub use read_only::run_read_only;
pub use session::{Session, cancel, open_session};
pub use structure::table_structure;

use std::collections::BTreeSet;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::TryStreamExt;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{Bson, Document, doc};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};

use self::logged::{Find, LoggedClient, LoggedDatabase};

use super::row_filter::RowFilter;
use super::{
    Column, DbObject, ObjectKind, Relation, RelationList, RowSink, RowsPage, TableRef, each_concurrently, with_timeout,
};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// Databases the server keeps for itself; never shown.
const SYSTEM_DATABASES: [&str; 3] = ["admin", "config", "local"];

/// A client for the connection. Users authenticate against `admin`, where
/// mongod's users usually live. Hosts saved as a whole `mongodb://` or
/// `mongodb+srv://` URL, before the form split URLs into fields, still work.
async fn connect(connection: &Connection, password: Option<&str>) -> Result<LoggedClient> {
    Ok(LoggedClient::with_options(options(connection, password).await?)?)
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
fn default_database(connection: &Connection, client: &LoggedClient) -> Option<String> {
    Some(connection.database.clone()).filter(|d| !d.is_empty()).or_else(|| client.default_database())
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
            // The count kept in collection metadata: instant, like reltuples, but
            // one round trip each, so they are asked for a few at a time.
            each_concurrently(collections(&db).await?, |(name, is_view)| {
                let db = &db;
                async move {
                    let row_estimate = if is_view {
                        None
                    } else {
                        Some(db.collection(&name).estimated_document_count().await? as i64)
                    };
                    let kind = if is_view { ObjectKind::View } else { ObjectKind::Table };
                    Ok(DbObject { name, kind, row_estimate })
                }
            })
            .await?
        }
        _ => return Err(AppError::Invalid(format!("Unexpected path: {}", path.join("/")))),
    };
    client.shutdown().await;
    Ok(objects)
}

/// Collections and views by name, without the server's own `system.*` ones.
async fn collections(db: &LoggedDatabase) -> Result<Vec<(String, bool)>> {
    let mut specs: Vec<(String, bool)> = db
        .list_collections()
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
    filter: &RowFilter,
    offset: u64,
    limit: u32,
) -> Result<RowsPage> {
    let (query, sort) = documents_of(filter)?;
    let client = connect(connection, password).await?;
    // One extra document tells whether another page exists.
    let find = Find::new(query).sort(sort).skip(offset).limit(i64::from(limit) + 1);
    let mut documents: Vec<Document> =
        client.database(&table.database).collection(&table.name).find(find).await?.try_collect().await?;
    client.shutdown().await;

    let has_more = documents.len() > limit as usize;
    documents.truncate(limit as usize);
    let columns = columns_of(&documents);
    let rows = documents.iter().map(|document| row(document, &columns)).collect();
    let texts = documents.into_iter().map(|document| pretty_literal(&Bson::Document(document))).collect();
    Ok(RowsPage { columns, rows, has_more, documents: Some(texts) })
}

/// Sends every document to `sink`. The columns are every top-level field used
/// anywhere in the collection, found first by the server in one pass.
pub async fn stream_table(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
    filter: &RowFilter,
    sink: &mut impl RowSink,
) -> Result<()> {
    let (query, sort) = documents_of(filter)?;
    let client = with_timeout(connect(connection, password)).await?;
    let collection = client.database(&table.database).collection(&table.name);
    let pipeline = vec![
        doc! { "$match": query.clone() },
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
    let first = collection.find(Find::new(query.clone()).sort(sort.clone()).limit(1)).await?.try_next().await?;
    let columns = ordered_columns(first.iter(), names);
    sink.columns(&columns)?;

    let mut cursor = collection.find(Find::new(query).sort(sort)).await?;
    while let Some(document) = cursor.try_next().await? {
        sink.row(&row(&document, &columns))?;
    }
    // shutdown() waits for every cursor of the client to be dropped.
    drop(cursor);
    client.shutdown().await;
    Ok(())
}

/// The filter and sort documents typed in the table view, in mongosh syntax:
/// `{status: 'paid'}` and `{total: -1}`. Without a sort, documents keep `_id` order.
fn documents_of(filter: &RowFilter) -> Result<(Document, Document)> {
    let read = |text: &str, what: &str, example: &str| -> Result<Option<Document>> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        match shell::parse_value(text) {
            Ok(Bson::Document(document)) => Ok(Some(document)),
            Ok(_) => Err(AppError::Invalid(format!("The {what} must be a document, like {example}."))),
            Err(error) => Err(AppError::Invalid(format!("The {what} can't be read: {error}"))),
        }
    };
    let query = read(&filter.filter, "filter", "{status: 'paid'}")?.unwrap_or_default();
    let sort = read(&filter.order, "sort", "{total: -1}")?.unwrap_or_else(|| doc! { "_id": 1 });
    Ok((query, sort))
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
        .map(|name| Column {
            primary_key: name == "_id",
            name,
            data_type: String::new(),
            generated: false,
            default: None,
        })
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
/// `ISODate("…")`, `UUID("…")` and friends for the types JSON lacks. What is
/// written here is what `shell::parse_value` reads as the same value, since an
/// edit sends back the text the grid showed.
pub fn literal(value: &Bson) -> String {
    let quote = |text: &str| serde_json::Value::from(text).to_string();
    match value {
        Bson::Null | Bson::Undefined => "null".into(),
        Bson::String(text) => quote(text),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => format!("NumberLong({n})"),
        Bson::Double(n) if n.is_nan() => "NaN".into(),
        Bson::Double(n) if n.is_infinite() => if *n > 0.0 { "Infinity" } else { "-Infinity" }.into(),
        Bson::Double(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.1}"),
        Bson::Double(n) => n.to_string(),
        Bson::Decimal128(n) => format!("NumberDecimal({})", quote(&n.to_string())),
        Bson::Boolean(b) => b.to_string(),
        Bson::ObjectId(id) => format!("ObjectId({})", quote(&id.to_hex())),
        Bson::DateTime(date) => match date.try_to_rfc3339_string() {
            Ok(iso) => format!("ISODate({})", quote(&iso)),
            Err(_) => format!("new Date({})", date.timestamp_millis()),
        },
        Bson::Binary(binary) => match uuid::Uuid::from_slice(&binary.bytes) {
            Ok(id) if binary.subtype == BinarySubtype::Uuid => format!("UUID({})", quote(&id.to_string())),
            _ => format!("BinData({}, {})", u8::from(binary.subtype), quote(&BASE64.encode(&binary.bytes))),
        },
        Bson::Timestamp(stamp) => format!("Timestamp({{t: {}, i: {}}})", stamp.time, stamp.increment),
        Bson::RegularExpression(regex) => format!("RegExp({}, {})", quote(&regex.pattern), quote(&regex.options)),
        Bson::MinKey => "MinKey()".into(),
        Bson::MaxKey => "MaxKey()".into(),
        Bson::JavaScriptCode(code) => format!("Code({})", quote(code)),
        Bson::JavaScriptCodeWithScope(code) => {
            format!("Code({}, {})", quote(&code.code), literal(&Bson::Document(code.scope.clone())))
        }
        Bson::Symbol(symbol) => format!("BSONSymbol({})", quote(symbol)),
        Bson::Array(items) => format!("[{}]", items.iter().map(literal).collect::<Vec<_>>().join(", ")),
        Bson::Document(document) => object_literal(document.iter().map(|(key, value)| (key.as_str(), literal(value)))),
        // A pointer to another collection, long deprecated: shown, and refused if sent back.
        Bson::DbPointer(_) => format!("DBPointer({})", value.clone().into_relaxed_extjson()),
    }
}

/// A document in [`literal`]'s syntax from its fields, each value already written.
fn object_literal<'a>(fields: impl Iterator<Item = (&'a str, String)>) -> String {
    let fields: Vec<String> = fields.map(|(key, value)| format!("{}: {value}", serde_json::Value::from(key))).collect();
    format!("{{{}}}", fields.join(", "))
}

/// A document as the JSON view shows it: [`literal`]'s syntax, which mongosh
/// reads back, with one field or item per line.
pub fn pretty_literal(value: &Bson) -> String {
    pretty(value, 0)
}

fn pretty(value: &Bson, depth: usize) -> String {
    let (inner, outer) = ("  ".repeat(depth + 1), "  ".repeat(depth));
    let lines = |items: Vec<String>, open: char, close: char| {
        format!("{open}\n{inner}{}\n{outer}{close}", items.join(&format!(",\n{inner}")))
    };
    match value {
        Bson::Document(document) if !document.is_empty() => {
            let quote = |key: &str| serde_json::Value::from(key).to_string();
            let fields = document.iter().map(|(key, value)| format!("{}: {}", quote(key), pretty(value, depth + 1)));
            lines(fields.collect(), '{', '}')
        }
        Bson::Array(items) if !items.is_empty() => {
            lines(items.iter().map(|item| pretty(item, depth + 1)).collect(), '[', ']')
        }
        other => literal(other),
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
            agent_access: Default::default(),
        }
    }

    /// A fresh database named `name` with `documents` in each collection.
    pub async fn database(name: &str, collections: &[(&str, Vec<Document>)]) -> Connection {
        let client = connect(&local(), PASSWORD).await.unwrap();
        let db = client.unlogged().database(name);
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

    #[test]
    fn documents_print_one_field_per_line_in_mongosh_syntax() {
        let id = ObjectId::parse_str("65a0f0f0f0f0f0f0f0f0f0f0").unwrap();
        let document = doc! { "_id": id, "n": 5_i64, "tags": ["a", 1], "empty": {}, "nested": { "ok": true } };
        assert_eq!(
            pretty_literal(&Bson::Document(document)),
            r#"{
  "_id": ObjectId("65a0f0f0f0f0f0f0f0f0f0f0"),
  "n": NumberLong(5),
  "tags": [
    "a",
    1
  ],
  "empty": {},
  "nested": {
    "ok": true
  }
}"#
        );
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
            .unlogged()
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

        let page =
            fetch_rows(&connection, PASSWORD, &people, &crate::drivers::RowFilter::default(), 0, 2).await.unwrap();
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

        let last =
            fetch_rows(&connection, PASSWORD, &people, &crate::drivers::RowFilter::default(), 2, 2).await.unwrap();
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
        stream_table(
            &connection,
            PASSWORD,
            &table("dbforge_stream", "things"),
            &crate::drivers::RowFilter::default(),
            &mut sink,
        )
        .await
        .unwrap();
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

    /// What the grid and the JSON view show is what an edit sends back: every
    /// type must come out of the parser as it went into `literal`.
    #[test]
    fn every_type_reads_back_as_it_was_written() {
        use mongodb::bson::{Binary, JavaScriptCodeWithScope, Regex, Timestamp};

        let uuid = Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() };
        let values = [
            Bson::Null,
            Bson::String("a\"b\n".into()),
            Bson::Int32(-3),
            Bson::Int64(1 << 40),
            Bson::Double(1.0),
            Bson::Double(-2.5e-7),
            Bson::Double(f64::INFINITY),
            Bson::Double(f64::NEG_INFINITY),
            Bson::Decimal128("12.50".parse().unwrap()),
            Bson::Boolean(true),
            Bson::ObjectId(ObjectId::parse_str("65a0f0f0f0f0f0f0f0f0f0f0").unwrap()),
            Bson::DateTime(DateTime::from_millis(1_700_000_000_123)),
            Bson::Binary(uuid.clone()),
            Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes: vec![0xAC, 0xED, 0, 5] }),
            Bson::Binary(Binary { subtype: BinarySubtype::UserDefined(0x80), bytes: vec![] }),
            Bson::Timestamp(Timestamp { time: 1_700_000_000, increment: 7 }),
            Bson::RegularExpression(Regex { pattern: "^a/b\\d\"$".into(), options: "im".into() }),
            Bson::MinKey,
            Bson::MaxKey,
            Bson::JavaScriptCode("function () { return 'x' }".into()),
            Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope { code: "x".into(), scope: doc! { "n": 1 } }),
            Bson::Symbol("sym".into()),
            Bson::Array(vec![Bson::Binary(uuid.clone()), Bson::Null]),
            Bson::Document(doc! { "id": uuid, "$weird key": { "n": 1_i64 } }),
        ];
        for value in values {
            let text = literal(&value);
            assert_eq!(shell::parse_value(&text), Ok(value.clone()), "{text}");
            assert_eq!(shell::parse_value(&pretty_literal(&value)), Ok(value), "{text}");
        }
        assert_eq!(
            literal(&Bson::Binary(Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() })),
            r#"UUID("00010203-0405-0607-0809-0a0b0c0d0e0f")"#
        );
        assert!(
            shell::parse_value(&literal(&Bson::Double(f64::NAN))).is_ok_and(|v| v.as_f64().is_some_and(f64::is_nan))
        );
    }
    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn filters_and_sorts_documents_in_mongosh_syntax() {
        let documents = vec![doc! { "_id": 1, "paid": 12.5 }, doc! { "_id": 2, "paid": 0 }, doc! { "_id": 3 }];
        let connection = database("dbforge_filter", &[("items", documents)]).await;
        let items = table("dbforge_filter", "items");
        let f = |filter: &str, order: &str| crate::drivers::RowFilter { filter: filter.into(), order: order.into() };
        let ids = |page: &RowsPage| page.rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>();

        let page = fetch_rows(&connection, PASSWORD, &items, &f("{paid: {$exists: true}}", "{paid: -1}"), 0, 10)
            .await
            .unwrap();
        assert_eq!(ids(&page), ["1", "2"]);
        let error = fetch_rows(&connection, PASSWORD, &items, &f("[1]", ""), 0, 10).await.unwrap_err();
        assert_eq!(error.to_string(), "The filter must be a document, like {status: 'paid'}.");
        let error = fetch_rows(&connection, PASSWORD, &items, &f("{paid: ", ""), 0, 10).await.unwrap_err();
        assert!(error.to_string().starts_with("The filter can't be read: "), "{error}");

        let mut sink = Collected::default();
        stream_table(&connection, PASSWORD, &items, &f("{_id: {$gt: 1}}", "{_id: -1}"), &mut sink).await.unwrap();
        assert_eq!(sink.1.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>(), ["3", "2"]);
    }
}
