//! The structure view of a container. Cosmos DB declares no columns, so the
//! properties come from a sample of items, each with the types found and how
//! often it appears. The indexes are the indexing policy's paths, composite
//! and spatial indexes and unique keys, plus the one Cosmos DB always keeps:
//! `id` is unique within a partition key value. The script is the container's
//! definition as JSON, as the REST API and the Azure tools take it.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::client::{Client, Target};
use super::query::{Items, Select};
use super::{SYSTEM_PROPERTIES, key_properties, ordered_columns, type_name, usage_value};
use crate::connections::model::Connection;
use crate::drivers::{ColumnInfo, IndexInfo, ObjectKind, RowFilter, TableRef, TableStructure};
use crate::error::Result;

/// Items read to find the properties.
const SAMPLE: usize = 1000;

pub async fn table_structure(
    connection: &Connection,
    password: Option<&str>,
    table: &TableRef,
) -> Result<TableStructure> {
    let client = Client::new(connection, password)?;
    let reply = client
        .get(&Target::container(&table.database, &table.name), &[("x-ms-documentdb-populatequotainfo", "true")])
        .await?;
    let usage = |name: &str| reply.header("x-ms-resource-usage").and_then(|usage| usage_value(usage, name));
    let (count, kilobytes) = (usage("documentsCount"), usage("documentsSize"));
    let container = reply.body;
    let database = client.get(&Target::database(&table.database), &[]).await?.body;
    let offers = client.list(&Target::offers(), "Offers").await?;

    let mut sample = Vec::new();
    let all = Select::of(&RowFilter::default());
    let mut items = Items::open(&client, &table.database, &table.name, &all, SAMPLE as u32).await?;
    while sample.len() < SAMPLE {
        match items.next().await? {
            Some(item) => sample.push(item),
            None => break,
        }
    }

    let keys = key_paths(&container);
    Ok(TableStructure {
        kind: ObjectKind::Table,
        row_estimate: count,
        data_bytes: kilobytes.unwrap_or_default() * 1024,
        index_bytes: 0,
        comment: Some(summary(&container, &keys, &throughput(&offers, &container, &database))),
        columns: properties(&sample, &key_properties(&container)),
        indexes: indexes(&container, &keys),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        ddl: definition(&container),
    })
}

fn key_paths(container: &Value) -> Vec<String> {
    strings(container.pointer("/partitionKey/paths"))
}

/// The container in one line: `Partition key /customer · 400 RU/s · Items expire after 3600 s`.
fn summary(container: &Value, keys: &[String], throughput: &Option<String>) -> String {
    let mut parts = vec![format!("Partition key {}", keys.join(", "))];
    parts.extend(throughput.clone());
    match container.get("defaultTtl").and_then(Value::as_i64) {
        Some(-1) => parts.push("Items expire when they say so (ttl)".into()),
        Some(seconds) => parts.push(format!("Items expire after {seconds} s")),
        None => {}
    }
    parts.join(" · ")
}

/// The throughput provisioned on the container, or shared from its database.
/// Serverless accounts have none, and say nothing.
fn throughput(offers: &[Value], container: &Value, database: &Value) -> Option<String> {
    let offer_of = |resource: &Value| {
        let rid = resource.get("_rid").and_then(Value::as_str)?;
        offers.iter().find(|offer| offer.get("offerResourceId").and_then(Value::as_str) == Some(rid))
    };
    let describe = |offer: &Value| {
        let content = offer.get("content")?;
        match content.pointer("/offerAutopilotSettings/maxThroughput").and_then(Value::as_i64) {
            Some(max) => Some(format!("Autoscale up to {max} RU/s")),
            None => content.get("offerThroughput").and_then(Value::as_i64).map(|ru| format!("{ru} RU/s")),
        }
    };
    match (offer_of(container), offer_of(database)) {
        (Some(offer), _) => describe(offer),
        (None, Some(offer)) => describe(offer).map(|shared| format!("Shares the database's {shared}")),
        (None, None) => None,
    }
}

/// Every top-level property of `sample`: its types (`number | string` when
/// they differ), and whether some items lack it or hold `null`.
fn properties(sample: &[Value], keys: &[String]) -> Vec<ColumnInfo> {
    ordered_columns(sample.iter(), keys, BTreeSet::new())
        .into_iter()
        .map(|column| {
            let values: Vec<&Value> = sample.iter().filter_map(|item| item.get(&column.name)).collect();
            let types: BTreeSet<&str> = values.iter().filter(|v| !v.is_null()).map(|v| type_name(v)).collect();
            let present = values.len();
            let mut notes = Vec::new();
            if keys.contains(&column.name) {
                notes.push("Partition key".to_string());
            }
            if present < sample.len() {
                let share = (present * 100 / sample.len()).max(1);
                notes.push(format!("In {share}% of {} sampled items", sample.len()));
            }
            ColumnInfo {
                nullable: present < sample.len() || values.iter().any(|v| v.is_null()),
                data_type: if types.is_empty() {
                    "null".into()
                } else {
                    types.into_iter().collect::<Vec<_>>().join(" | ")
                },
                default: None,
                comment: (!notes.is_empty()).then(|| notes.join(" · ")),
                name: column.name,
            }
        })
        .collect()
}

/// The indexing policy as indexes, after the key Cosmos DB keeps unique.
fn indexes(container: &Value, keys: &[String]) -> Vec<IndexInfo> {
    let index = |name: String, method: &str, columns: Vec<String>| IndexInfo {
        name,
        method: method.into(),
        unique: false,
        primary: false,
        columns,
        condition: None,
        bytes: 0,
    };
    let mut all = vec![IndexInfo {
        unique: true,
        primary: true,
        ..index("id".into(), "key", keys.iter().cloned().chain(["/id".to_string()]).collect())
    }];
    let policy = container.get("indexingPolicy").cloned().unwrap_or_default();
    if policy.get("indexingMode").and_then(Value::as_str) == Some("none") {
        return all;
    }
    let paths = |key: &str| -> Vec<String> {
        policy.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|p| text(p.get("path"))).collect()
    };
    let excluded = paths("excludedPaths");
    for path in paths("includedPaths") {
        let mut included = index(path.clone(), "range", vec![path]);
        if !excluded.is_empty() {
            included.condition = Some(format!("Except {}", excluded.join(", ")));
        }
        all.push(included);
    }
    for (number, composite) in arrays(policy.get("compositeIndexes")).iter().enumerate() {
        let columns = composite
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| {
                let path = text(part.get("path"))?;
                Some(match text(part.get("order")).as_deref() {
                    Some("descending") => format!("{path} DESC"),
                    _ => path,
                })
            })
            .collect();
        all.push(index(format!("composite {}", number + 1), "composite", columns));
    }
    for spatial in arrays(policy.get("spatialIndexes")) {
        let path = text(spatial.get("path")).unwrap_or_default();
        let types = strings(spatial.get("types")).join(", ");
        all.push(index(path.clone(), "spatial", vec![format!("{path} ({types})")]));
    }
    for (number, unique) in arrays(container.pointer("/uniqueKeyPolicy/uniqueKeys")).iter().enumerate() {
        all.push(IndexInfo {
            unique: true,
            ..index(format!("unique key {}", number + 1), "unique", strings(unique.get("paths")))
        });
    }
    all
}

/// The container's definition without what the service adds to it.
fn definition(container: &Value) -> String {
    let fields: Map<String, Value> = container
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| {
            !key.starts_with('_')
                && !SYSTEM_PROPERTIES.contains(&key.as_str())
                && !matches!(key.as_str(), "IndexVersion" | "internalIndexingProperties")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    serde_json::to_string_pretty(&Value::Object(fields)).unwrap_or_default()
}

fn text(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(String::from)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    arrays(value).iter().filter_map(Value::as_str).map(String::from).collect()
}

fn arrays(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{KEY, database, table};
    use super::*;

    fn container() -> Value {
        json!({
            "id": "orders",
            "_rid": "abc==",
            "_self": "dbs/x/colls/y/",
            "IndexVersion": 2,
            "partitionKey": {"kind": "Hash", "paths": ["/customer"]},
            "defaultTtl": 3600,
            "indexingPolicy": {
                "indexingMode": "consistent",
                "includedPaths": [{"path": "/*"}],
                "excludedPaths": [{"path": "/\"_etag\"/?"}],
                "compositeIndexes": [[{"path": "/customer", "order": "ascending"}, {"path": "/total", "order": "descending"}]],
                "spatialIndexes": [{"path": "/where/?", "types": ["Point", "Polygon"]}],
            },
            "uniqueKeyPolicy": {"uniqueKeys": [{"paths": ["/email"]}]},
        })
    }

    #[test]
    fn reads_the_indexing_policy_as_indexes() {
        let described: Vec<_> = indexes(&container(), &["/customer".into()])
            .into_iter()
            .map(|i| (i.name, i.method, i.unique, i.primary, i.columns, i.condition))
            .collect();
        let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            described,
            [
                ("id".into(), "key".into(), true, true, strings(&["/customer", "/id"]), None),
                ("/*".into(), "range".into(), false, false, strings(&["/*"]), Some("Except /\"_etag\"/?".into())),
                ("composite 1".into(), "composite".into(), false, false, strings(&["/customer", "/total DESC"]), None),
                ("/where/?".into(), "spatial".into(), false, false, strings(&["/where/? (Point, Polygon)"]), None),
                ("unique key 1".into(), "unique".into(), true, false, strings(&["/email"]), None),
            ]
        );
    }

    #[test]
    fn sums_up_the_key_throughput_and_ttl() {
        let offers = [
            json!({"offerResourceId": "db==", "content": {"offerThroughput": 400}}),
            json!({"offerResourceId": "abc==", "content": {"offerThroughput": 1000, "offerAutopilotSettings": {"maxThroughput": 4000}}}),
        ];
        let database = json!({"_rid": "db=="});
        let own = throughput(&offers, &container(), &database);
        assert_eq!(
            summary(&container(), &["/customer".into()], &own),
            "Partition key /customer · Autoscale up to 4000 RU/s · Items expire after 3600 s"
        );
        let shared = throughput(&offers, &json!({"_rid": "other=="}), &database);
        assert_eq!(shared.as_deref(), Some("Shares the database's 400 RU/s"));
        assert_eq!(throughput(&[], &container(), &database), None);
    }

    #[test]
    fn the_definition_leaves_out_what_the_service_adds() {
        let definition: Value = serde_json::from_str(&definition(&container())).unwrap();
        let keys: Vec<_> = definition.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["id", "partitionKey", "defaultTtl", "indexingPolicy", "uniqueKeyPolicy"]);
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn reads_properties_from_a_sample() {
        let items = vec![
            json!({"id": "1", "customer": "a", "total": 5, "note": "x"}),
            json!({"id": "2", "customer": "b", "total": "n/a"}),
        ];
        let connection = database("dbforge_structure", &[("orders", "/customer", items)]).await;
        let structure = table_structure(&connection, KEY, &table("dbforge_structure", "orders")).await.unwrap();
        let columns: Vec<_> = structure
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable, c.comment.as_deref()))
            .collect();
        assert_eq!(
            columns,
            [
                ("id", "string", false, None),
                ("customer", "string", false, Some("Partition key")),
                ("total", "number | string", false, None),
                ("note", "string", true, Some("In 50% of 2 sampled items")),
            ]
        );
        assert_eq!(structure.comment.as_deref(), Some("Partition key /customer"));
        assert_eq!(structure.indexes[0].columns, ["/customer", "/id"]);
        assert!(structure.ddl.contains("\"partitionKey\""));
    }
}
