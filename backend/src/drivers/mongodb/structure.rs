//! The structure view of a collection or view. MongoDB declares no columns, so
//! the fields come from a sample of documents, each with the types found and
//! how often it appears. Indexes, sizes and the validator come from the server,
//! and the script rebuilds the collection and its indexes in mongosh.

use std::collections::BTreeSet;

use futures_util::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};

use super::{connect, literal, ordered_columns, type_name};
use crate::connections::model::Connection;
use crate::drivers::{ColumnInfo, IndexInfo, ObjectKind, TableRef, TableStructure};
use crate::error::{AppError, Result};

/// Documents read to find the fields.
const SAMPLE: i32 = 1000;

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    let client = connect(connection, password).await?;
    let result = read(&client, table).await;
    client.shutdown().await;
    result
}

async fn read(client: &mongodb::Client, table: &TableRef) -> Result<TableStructure> {
    let database = client.database(&table.database);
    // The raw command keeps the options exactly as the server stores them.
    let reply = database.run_command(doc! { "listCollections": 1, "filter": { "name": &table.name } }).await?;
    let info = reply
        .get_document("cursor")
        .and_then(|cursor| cursor.get_array("firstBatch"))
        .ok()
        .and_then(|batch| batch.first()?.as_document().cloned())
        .ok_or_else(|| AppError::Invalid(format!("Collection {} not found", table.name)))?;
    let view = info.get_str("type") == Ok("view");
    let options = info.get_document("options").cloned().unwrap_or_default();
    let collection = database.collection::<Document>(&table.name);

    let sample: Vec<Document> =
        collection.aggregate([doc! { "$sample": { "size": SAMPLE } }]).await?.try_collect().await?;
    let columns = fields(&sample);

    let (stats, indexes) = if view {
        (Document::new(), Vec::new())
    } else {
        let stats = collection
            .aggregate([doc! { "$collStats": { "storageStats": {} } }])
            .await?
            .try_next()
            .await?
            .and_then(|reply| reply.get_document("storageStats").ok().cloned())
            .unwrap_or_default();
        let specs: Vec<Document> = database
            .run_command(doc! { "listIndexes": &table.name })
            .await?
            .get_document("cursor")
            .and_then(|cursor| cursor.get_array("firstBatch"))
            .map(|batch| batch.iter().filter_map(Bson::as_document).cloned().collect())
            .unwrap_or_default();
        (stats, specs)
    };
    let sizes = stats.get_document("indexSizes").ok();
    let index_info = indexes
        .iter()
        .map(|spec| {
            let name = spec.get_str("name").unwrap_or_default().to_string();
            let keys = spec.get_document("key").cloned().unwrap_or_default();
            let method = keys
                .values()
                .find_map(|direction| direction.as_str().map(String::from))
                .unwrap_or_else(|| "btree".into());
            IndexInfo {
                bytes: sizes.and_then(|s| whole(s.get(&name))).unwrap_or_default(),
                primary: name == "_id_",
                unique: name == "_id_" || spec.get_bool("unique").unwrap_or(false),
                columns: keys
                    .iter()
                    .map(|(field, direction)| match direction {
                        Bson::String(kind) => format!("{field} ({kind})"),
                        direction if whole(Some(direction)) == Some(-1) => format!("{field} DESC"),
                        _ => field.clone(),
                    })
                    .collect(),
                condition: spec
                    .get_document("partialFilterExpression")
                    .ok()
                    .map(|f| literal(&Bson::Document(f.clone()))),
                method,
                name,
            }
        })
        .collect();

    Ok(TableStructure {
        kind: if view { ObjectKind::View } else { ObjectKind::Table },
        row_estimate: whole(stats.get("count")),
        data_bytes: whole(stats.get("storageSize")).unwrap_or_default(),
        index_bytes: whole(stats.get("totalIndexSize")).unwrap_or_default(),
        comment: None,
        columns,
        indexes: index_info,
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        ddl: script(&table.name, &options, &indexes),
    })
}

/// Every top-level field of `sample`: its types (`int | string` when they
/// differ), and whether some documents lack it or hold `null`.
fn fields(sample: &[Document]) -> Vec<ColumnInfo> {
    ordered_columns(sample.iter(), BTreeSet::new())
        .into_iter()
        .filter(|column| column.name != "_id" || sample.is_empty() || sample.iter().any(|d| d.contains_key("_id")))
        .map(|column| {
            let values: Vec<&Bson> = sample.iter().filter_map(|d| d.get(&column.name)).collect();
            let types: BTreeSet<&str> =
                values.iter().copied().filter(|v| !matches!(v, Bson::Null | Bson::Undefined)).map(type_name).collect();
            let present = values.len();
            let comment = (present < sample.len()).then(|| {
                let share = (present * 100 / sample.len()).max(1);
                format!("In {share}% of {} sampled documents", sample.len())
            });
            ColumnInfo {
                options: Vec::new(),
                nullable: present < sample.len() || values.iter().any(|v| matches!(v, Bson::Null | Bson::Undefined)),
                data_type: if types.is_empty() {
                    "null".into()
                } else {
                    types.into_iter().collect::<Vec<_>>().join(" | ")
                },
                default: None,
                comment,
                name: column.name,
            }
        })
        .collect()
}

/// A mongosh script that creates the collection, or view, and its indexes.
fn script(name: &str, options: &Document, indexes: &[Document]) -> String {
    let quoted = literal(&Bson::String(name.to_string()));
    let mut statements = Vec::new();
    if let (Ok(source), Ok(pipeline)) = (options.get_str("viewOn"), options.get_array("pipeline")) {
        let source = literal(&Bson::String(source.to_string()));
        statements.push(format!("db.createView({quoted}, {source}, {})", literal(&Bson::Array(pipeline.clone()))));
    } else {
        let options: Document =
            options.iter().filter(|(_, v)| !matches!(v, Bson::Null)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if options.is_empty() {
            statements.push(format!("db.createCollection({quoted})"));
        } else {
            statements.push(format!("db.createCollection({quoted}, {})", literal(&Bson::Document(options))));
        }
    }
    let target = if is_identifier(name) { format!("db.{name}") } else { format!("db.getCollection({quoted})") };
    for index in indexes.iter().filter(|spec| spec.get_str("name") != Ok("_id_")) {
        let keys = index.get_document("key").cloned().unwrap_or_default();
        let settings: Document = index
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "v" | "key" | "ns"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        statements.push(format!(
            "{target}.createIndex({}, {})",
            literal(&Bson::Document(keys)),
            literal(&Bson::Document(settings))
        ));
    }
    statements.iter().map(|statement| format!("{statement};")).collect::<Vec<_>>().join("\n")
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !matches!(name, "getCollection" | "runCommand" | "createCollection" | "createView")
}

fn whole(value: Option<&Bson>) -> Option<i64> {
    match value? {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(n) => Some(*n as i64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, database, table};
    use super::*;

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn reads_fields_indexes_and_a_rebuild_script() {
        let connection = database(
            "dbforge_structure",
            &[(
                "people",
                vec![
                    doc! { "_id": 1, "name": "Ann", "age": 30, "email": "ann@x.io" },
                    doc! { "_id": 2, "name": "Bob", "age": "unknown" },
                    doc! { "_id": 3, "name": "Cy", "age": null, "email": "cy@x.io" },
                ],
            )],
        )
        .await;
        let client = connect(&connection, PASSWORD).await.unwrap();
        let db = client.database("dbforge_structure");
        db.run_command(doc! { "collMod": "people", "validator": { "name": { "$type": "string" } } }).await.unwrap();
        db.run_command(doc! {
            "createIndexes": "people",
            "indexes": [
                { "key": { "email": 1 }, "name": "email_1", "unique": true, "partialFilterExpression": { "email": { "$exists": true } } },
                { "key": { "age": -1, "name": "text" }, "name": "age_text" },
            ],
        })
        .await
        .unwrap();
        db.create_collection("adults")
            .view_on("people".to_string())
            .pipeline(vec![doc! { "$match": { "age": { "$gte": 18 } } }])
            .await
            .unwrap();
        client.shutdown().await;

        let structure = table_structure(&connection, PASSWORD, &table("dbforge_structure", "people")).await.unwrap();
        assert_eq!(structure.kind, ObjectKind::Table);
        assert_eq!(structure.row_estimate, Some(3));
        assert!(structure.data_bytes > 0 && structure.index_bytes > 0, "{structure:?}");
        let columns: Vec<_> = structure
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable, c.comment.as_deref()))
            .collect();
        assert_eq!(
            columns,
            [
                ("_id", "int", false, None),
                ("name", "string", false, None),
                ("age", "int | string", true, None),
                ("email", "string", true, Some("In 66% of 3 sampled documents")),
            ]
        );
        let indexes: Vec<_> = structure
            .indexes
            .iter()
            .map(|i| (i.name.as_str(), i.primary, i.unique, i.columns.clone(), i.method.as_str()))
            .collect();
        assert_eq!(indexes[0], ("_id_", true, true, vec!["_id".to_string()], "btree"));
        assert_eq!(indexes[1], ("email_1", false, true, vec!["email".to_string()], "btree"));
        assert_eq!(
            indexes[2],
            (
                "age_text",
                false,
                false,
                vec!["age DESC".to_string(), "_fts (text)".to_string(), "_ftsx".to_string()],
                "text"
            )
        );
        assert_eq!(structure.indexes[1].condition.as_deref(), Some(r#"{"email": {"$exists": true}}"#));
        assert!(structure.indexes.iter().all(|i| i.bytes > 0));

        let ddl = &structure.ddl;
        assert!(
            ddl.starts_with(r#"db.createCollection("people", {"validator": {"name": {"$type": "string"}}"#),
            "{ddl}"
        );
        assert!(ddl.contains(r#"db.people.createIndex({"email": 1}, {"name": "email_1", "unique": true, "partialFilterExpression": {"email": {"$exists": true}}});"#), "{ddl}");
        assert!(!ddl.contains("_id_"), "{ddl}");

        let view = table_structure(&connection, PASSWORD, &table("dbforge_structure", "adults")).await.unwrap();
        assert_eq!(view.kind, ObjectKind::View);
        assert!(view.indexes.is_empty() && view.row_estimate.is_none());
        assert_eq!(view.ddl, r#"db.createView("adults", "people", [{"$match": {"age": {"$gte": 18}}}]);"#);
        assert_eq!(view.columns.len(), 4);
    }
}
