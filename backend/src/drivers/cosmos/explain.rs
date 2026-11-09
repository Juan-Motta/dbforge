//! Query plans, in the JSON shape PostgreSQL's `EXPLAIN (FORMAT JSON)` has, so
//! one plan view serves every engine. Explain asks the gateway for the query's
//! plan without running it: which partition key ranges it reads, then its
//! grouping, aggregates, DISTINCT, ORDER BY and paging. Analyze runs the query,
//! reading every page (queries never write), and shows where its time went
//! with the metrics the service reports: index lookup, document load and
//! execution, the indexes it used and the request units it cost.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::client::{Client, Target};
use super::query::{Feed, split_top_level};
use super::session::{console_database, container_for};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// The query features this client can be told about; the gateway plans for them.
const QUERY_FEATURES: &str = "Aggregate, CompositeAggregate, Distinct, GroupBy, MultipleAggregates, \
                              MultipleOrderBy, OffsetAndLimit, OrderBy, Top, NonValueAggregate, DCount";

pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    script: &str,
    analyze: bool,
) -> Result<Value> {
    let queries = split_top_level(script, ';');
    let [query] = queries.as_slice() else {
        return Err(AppError::Invalid("Explain one query at a time.".into()));
    };
    let database = console_database(connection, database)?;
    let client = Client::new(connection, password)?;
    let container = container_for(&client, &database, query).await?;
    if analyze {
        let metrics = run_with_metrics(&client, &database, &container, query).await?;
        Ok(analyzed(&container, &metrics))
    } else {
        let headers = [
            ("x-ms-documentdb-isquery", "true"),
            ("x-ms-cosmos-is-query-plan-request", "True"),
            ("x-ms-cosmos-supported-query-features", QUERY_FEATURES),
            ("x-ms-cosmos-query-version", "1.4"),
        ];
        let body = json!({ "query": query, "parameters": [] });
        let plan = client.send(Method::POST, &Target::items(&database, &container), &headers, Some(&body)).await?;
        Ok(json!([{ "Plan": planned(&container, &plan.body) }]))
    }
}

/// The plan's steps, the read of the container at the bottom.
fn planned(container: &str, plan: &Value) -> Value {
    let ranges = plan.get("queryRanges").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut node = json!({
        "Node Type": "Scan",
        "Relation Name": container,
        "Detail": ranges_read(&ranges),
    });
    let info = plan.get("queryInfo").cloned().unwrap_or_default();
    let strings = |key: &str| -> Vec<String> {
        info.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), String::from)).collect()
            })
            .unwrap_or_default()
    };
    let mut wrap = |fields: Value| {
        let mut fields = fields;
        fields["Plans"] = json!([node.take()]);
        node = fields;
    };
    let groups = strings("groupByExpressions");
    if !groups.is_empty() {
        wrap(json!({ "Node Type": "Group By", "Group Key": groups }));
    }
    let aggregates = strings("aggregates");
    if !aggregates.is_empty() {
        wrap(json!({ "Node Type": "Aggregate", "Detail": aggregates.join(", ") }));
    }
    if let Some(distinct) = info.get("distinctType").and_then(Value::as_str).filter(|d| *d != "None") {
        wrap(json!({ "Node Type": "Distinct", "Detail": distinct }));
    }
    let order = strings("orderByExpressions");
    if !order.is_empty() {
        let directions = strings("orderBy");
        let keys: Vec<String> = order
            .iter()
            .enumerate()
            .map(|(i, key)| match directions.get(i).map(String::as_str) {
                Some("Descending") => format!("{key} DESC"),
                _ => key.clone(),
            })
            .collect();
        wrap(json!({ "Node Type": "Sort", "Sort Key": keys }));
    }
    let number = |key: &str| info.get(key).and_then(Value::as_u64);
    if number("offset").is_some() || number("limit").is_some() {
        let detail = format!("OFFSET {} LIMIT {}", number("offset").unwrap_or(0), number("limit").unwrap_or(0));
        wrap(json!({ "Node Type": "Limit", "Detail": detail }));
    }
    if let Some(top) = number("top") {
        wrap(json!({ "Node Type": "Top", "Detail": format!("TOP {top}") }));
    }
    node
}

/// How much of the container the query reads, from the ranges it was planned on.
fn ranges_read(ranges: &[Value]) -> String {
    let bound = |range: &Value, key: &str| range.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    match ranges {
        [range]
            if bound(range, "min") == bound(range, "max")
                && range.get("isMaxInclusive").and_then(Value::as_bool) == Some(true) =>
        {
            "One partition key value".into()
        }
        [range] if bound(range, "min").is_empty() && bound(range, "max") == "FF" => "Every partition".into(),
        _ => format!("{} partition key ranges", ranges.len()),
    }
}

/// What a run reported, added up over its pages.
#[derive(Debug, Default)]
struct Metrics {
    /// `x-ms-documentdb-query-metrics`, summed.
    values: BTreeMap<String, f64>,
    /// Request units the run cost.
    charge: f64,
    pages: u64,
    /// `x-ms-cosmos-index-utilization`, which comes with the first page.
    indexes: Option<Value>,
}

impl Metrics {
    fn get(&self, key: &str) -> f64 {
        self.values.get(key).copied().unwrap_or_default()
    }
}

async fn run_with_metrics(client: &Client, database: &str, container: &str, query: &str) -> Result<Metrics> {
    let headers = [("x-ms-documentdb-populatequerymetrics", "true"), ("x-ms-cosmos-populateindexmetrics", "true")];
    let mut feed = Feed::new(client, database, container, query, &headers);
    let mut metrics = Metrics::default();
    while let Some(reply) = feed.next().await? {
        metrics.pages += 1;
        metrics.charge += reply.header("x-ms-request-charge").and_then(|c| c.parse::<f64>().ok()).unwrap_or_default();
        for (key, value) in reply.header("x-ms-documentdb-query-metrics").map(parse_metrics).unwrap_or_default() {
            *metrics.values.entry(key).or_default() += value;
        }
        if metrics.indexes.is_none() {
            metrics.indexes = reply.header("x-ms-cosmos-index-utilization").and_then(parse_index_use);
        }
    }
    Ok(metrics)
}

/// `totalExecutionTimeInMs=4.54;retrievedDocumentCount=4;…`
fn parse_metrics(text: &str) -> Vec<(String, f64)> {
    text.split(';')
        .filter_map(|pair| pair.split_once('='))
        .filter_map(|(key, value)| Some((key.trim().to_string(), value.trim().parse().ok()?)))
        .collect()
}

/// The index report, which Azure sends as Base64 and the emulator as plain JSON.
fn parse_index_use(text: &str) -> Option<Value> {
    serde_json::from_str(text)
        .ok()
        .or_else(|| BASE64.decode(text).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()))
}

/// The run as nested steps, each one's time including the steps below it.
fn analyzed(container: &str, metrics: &Metrics) -> Value {
    let lookup = metrics.get("indexLookupTimeInMs");
    let load = lookup + metrics.get("documentLoadTimeInMs");
    let execution = load
        + metrics.get("VMExecutionTimeInMs")
        + metrics.get("systemFunctionExecuteTimeInMs")
        + metrics.get("userFunctionExecuteTimeInMs");
    let total = metrics.get("totalExecutionTimeInMs").max(execution);
    let output = metrics.get("outputDocumentCount");
    let retrieved = metrics.get("retrievedDocumentCount");

    let mut index = actual("Index Lookup", lookup, retrieved);
    let used = metrics.indexes.as_ref().map(|i| indexes(i, "Utilized")).unwrap_or_default();
    let potential = metrics.indexes.as_ref().map(|i| indexes(i, "Potential")).unwrap_or_default();
    index.insert("Relation Name".into(), container.into());
    if !used.is_empty() {
        index.insert("Detail".into(), format!("using {}", used.join(", ")).into());
    }
    if !potential.is_empty() {
        index.insert("Potential Indexes".into(), potential.into());
    }
    index.insert("Index Hit Ratio".into(), metrics.get("indexUtilizationRatio").into());

    let mut documents = actual("Document Load", load, retrieved);
    documents.insert("Retrieved Bytes".into(), metrics.get("retrievedDocumentSize").into());
    documents.insert("Plans".into(), json!([index]));

    let mut runtime = actual("Execution", execution, output);
    runtime.insert("Plans".into(), json!([documents]));

    let mut root = actual("Query", total, output);
    root.insert("Request Charge (RU)".into(), ((metrics.charge * 100.0).round() / 100.0).into());
    root.insert("Pages".into(), metrics.pages.into());
    root.insert("Output Bytes".into(), metrics.get("outputDocumentSize").into());
    root.insert("Plans".into(), json!([runtime]));

    let planning = metrics.get("queryCompileTimeInMs")
        + metrics.get("queryLogicalPlanBuildTimeInMs")
        + metrics.get("queryPhysicalPlanBuildTimeInMs")
        + metrics.get("queryOptimizationTimeInMs");
    json!([{ "Plan": root, "Planning Time": planning, "Execution Time": total }])
}

fn actual(name: &str, milliseconds: f64, rows: f64) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("Node Type".into(), name.into());
    fields.insert("Actual Total Time".into(), milliseconds.into());
    fields.insert("Actual Rows".into(), rows.into());
    fields.insert("Actual Loops".into(), 1.into());
    fields
}

/// The `Utilized` or `Potential` indexes of the report: `/total/?` for a
/// single path, `(/customer ASC, /total DESC)` for a composite one.
fn indexes(report: &Value, kind: &str) -> Vec<String> {
    let list = |key: &str| report.get(format!("{kind}{key}")).and_then(Value::as_array).cloned().unwrap_or_default();
    let single = list("SingleIndexes")
        .iter()
        .filter_map(|index| index.get("IndexSpec").and_then(Value::as_str).map(String::from))
        .collect::<Vec<_>>();
    let composites = list("CompositeIndexes");
    let composite = composites.iter().filter_map(|index| {
        let specs = index.get("IndexSpecs").and_then(Value::as_array)?;
        Some(format!("({})", specs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")))
    });
    single.into_iter().chain(composite).collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{KEY, database};
    use super::*;

    #[test]
    fn plans_read_bottom_up_from_the_scan() {
        let plan = json!({
            "queryInfo": {
                "distinctType": "None", "top": 10, "offset": null, "limit": null,
                "orderBy": ["Descending"], "orderByExpressions": ["c.total"],
                "groupByExpressions": [], "aggregates": [],
            },
            "queryRanges": [{"min": "", "max": "FF", "isMinInclusive": true, "isMaxInclusive": false}],
        });
        let top = planned("orders", &plan);
        assert_eq!(top["Node Type"], "Top");
        assert_eq!(top["Detail"], "TOP 10");
        let sort = &top["Plans"][0];
        assert_eq!(sort["Sort Key"], json!(["c.total DESC"]));
        let scan = &sort["Plans"][0];
        assert_eq!((scan["Node Type"].as_str(), scan["Detail"].as_str()), (Some("Scan"), Some("Every partition")));
        assert!(scan.get("Plans").is_none());

        let grouped = json!({
            "queryInfo": {"distinctType": "Ordered", "groupByExpressions": ["c.customer"], "aggregates": ["Sum"],
                          "offset": 5, "limit": 10},
            "queryRanges": [{"min": "05C1", "max": "05C1", "isMinInclusive": true, "isMaxInclusive": true}],
        });
        let limit = planned("orders", &grouped);
        assert_eq!(limit["Detail"], "OFFSET 5 LIMIT 10");
        let distinct = &limit["Plans"][0];
        assert_eq!(distinct["Node Type"], "Distinct");
        let aggregate = &distinct["Plans"][0];
        assert_eq!(aggregate["Detail"], "Sum");
        assert_eq!(aggregate["Plans"][0]["Group Key"], json!(["c.customer"]));
        assert_eq!(aggregate["Plans"][0]["Plans"][0]["Detail"], "One partition key value");
    }

    #[test]
    fn analyze_nests_the_metrics_with_inclusive_times() {
        let report = r#"{"UtilizedSingleIndexes":[{"FilterExpression":"","IndexSpec":"/total/?"}],"PotentialSingleIndexes":[],"UtilizedCompositeIndexes":[],"PotentialCompositeIndexes":[{"IndexSpecs":["/customer ASC","/total DESC"]}]}"#;
        let mut metrics = Metrics {
            values: parse_metrics("totalExecutionTimeInMs=10;indexLookupTimeInMs=1;documentLoadTimeInMs=2;VMExecutionTimeInMs=3;retrievedDocumentCount=40;outputDocumentCount=4;queryCompileTimeInMs=0.5").into_iter().collect(),
            charge: 2.856,
            pages: 2,
            indexes: parse_index_use(&BASE64.encode(report)),
        };
        let plan = analyzed("orders", &metrics);
        assert_eq!(plan[0]["Execution Time"], 10.0);
        assert_eq!(plan[0]["Planning Time"], 0.5);
        let root = &plan[0]["Plan"];
        assert_eq!((root["Actual Rows"].as_f64(), root["Request Charge (RU)"].as_f64()), (Some(4.0), Some(2.86)));
        let documents = &root["Plans"][0]["Plans"][0];
        assert_eq!(
            (documents["Actual Total Time"].as_f64(), documents["Actual Rows"].as_f64()),
            (Some(3.0), Some(40.0))
        );
        let index = &documents["Plans"][0];
        assert_eq!(index["Detail"], "using /total/?");
        assert_eq!(index["Potential Indexes"], json!(["(/customer ASC, /total DESC)"]));

        // The emulator sends the report as plain JSON.
        metrics.indexes = parse_index_use(report);
        assert!(metrics.indexes.is_some());
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn explains_and_analyzes_a_query() {
        let items = (0..4).map(|i| json!({"id": i.to_string(), "customer": "c", "total": i})).collect();
        let connection = database("dbforge_explain", &[("orders", "/customer", items)]).await;
        let query = "SELECT * FROM orders o WHERE o.total > 1 ORDER BY o.total DESC";

        let plan = explain(&connection, KEY, None, query, false).await.unwrap();
        assert!(plan[0]["Plan"].get("Actual Total Time").is_none());

        let run = explain(&connection, KEY, None, query, true).await.unwrap();
        let root = &run[0]["Plan"];
        assert_eq!(root["Node Type"], "Query");
        assert_eq!(root["Actual Rows"], 2.0);
        assert!(root["Request Charge (RU)"].as_f64().unwrap() > 0.0);

        let error = explain(&connection, KEY, None, "SELECT 1 FROM orders; SELECT 2 FROM orders", false).await;
        assert_eq!(error.unwrap_err().to_string(), "Explain one query at a time.");
    }
}
