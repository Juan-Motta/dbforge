//! Query plans. The console's statement is sent to the server's `explain`
//! command, which plans it without running it; with Analyze the server also
//! runs the query part to count (verbosity `executionStats`), but explain never
//! applies writes. The plan is turned into the JSON shape PostgreSQL's
//! `EXPLAIN (FORMAT JSON)` has, so one plan view serves every engine.

use mongodb::bson::{Bson, Document, doc};
use serde_json::{Map, Value, json};

use super::session::{Args, unsupported_after, upsert};
use super::shell::{self, Method, Statement};
use super::{connect, default_database, literal};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    script: &str,
    analyze: bool,
) -> Result<Value> {
    let statements = shell::parse(script).map_err(AppError::Invalid)?;
    let [Statement::Call { collection: Some(collection), method, chain }] = statements.as_slice() else {
        return Err(AppError::Invalid("Explain one db.<collection>.<method>(…) statement at a time.".into()));
    };
    let command = command(collection, method, chain)?;

    let client = connect(connection, password).await?;
    let database = database
        .filter(|d| !d.is_empty())
        .map(String::from)
        .or_else(|| default_database(connection, &client))
        .unwrap_or_else(|| "test".into());
    let verbosity = if analyze { "executionStats" } else { "queryPlanner" };
    let reply = client.database(&database).run_command(doc! { "explain": command, "verbosity": verbosity }).await;
    client.shutdown().await;
    plan(&reply?, analyze)
}

/// The command `method` stands for, as the `explain` command takes it.
fn command(collection: &str, method: &Method, chain: &[Method]) -> Result<Document> {
    let name = method.name.as_str();
    let args = Args { method: name, values: &method.args };
    if !matches!(name, "find" | "findOne" | "aggregate")
        && let Some(next) = chain.first()
    {
        return Err(unsupported_after(&next.name, name));
    }
    Ok(match name {
        "find" | "findOne" => {
            args.at_most(2)?;
            let mut find = doc! { "find": collection, "filter": args.document(0)? };
            if let Some(projection) = args.optional_document(1)? {
                find.insert("projection", projection);
            }
            if name == "findOne" {
                find.insert("limit", 1_i64);
            }
            for modifier in chain {
                let args = Args { method: &modifier.name, values: &modifier.args };
                match modifier.name.as_str() {
                    "sort" => find.insert("sort", args.required_document(0)?),
                    "projection" => find.insert("projection", args.required_document(0)?),
                    "skip" => find.insert("skip", args.count(0)?.abs()),
                    "limit" => find.insert("limit", args.count(0)?.abs()),
                    "toArray" | "pretty" => None,
                    other => return Err(unsupported_after(other, name)),
                };
            }
            find
        }
        "aggregate" => {
            args.at_most(1)?;
            if let Some(next) = chain.iter().find(|m| !matches!(m.name.as_str(), "toArray" | "pretty")) {
                return Err(unsupported_after(&next.name, name));
            }
            doc! { "aggregate": collection, "pipeline": args.stages()?, "cursor": {} }
        }
        "countDocuments" | "count" => {
            args.at_most(1)?;
            doc! { "count": collection, "query": args.document(0)? }
        }
        "distinct" => {
            args.at_most(2)?;
            let Some(Bson::String(field)) = args.values.first() else {
                return Err(AppError::Invalid("distinct takes a field name.".into()));
            };
            doc! { "distinct": collection, "key": field, "query": args.document(1)? }
        }
        "updateOne" | "updateMany" | "replaceOne" => {
            args.at_most(3)?;
            let update = match args.values.get(1) {
                Some(update @ (Bson::Document(_) | Bson::Array(_))) => update.clone(),
                _ => return Err(AppError::Invalid(format!("{name} takes a filter and an update."))),
            };
            let change = doc! {
                "q": args.required_document(0)?, "u": update, "multi": name == "updateMany", "upsert": upsert(&args)?,
            };
            doc! { "update": collection, "updates": [change] }
        }
        "deleteOne" | "deleteMany" => {
            args.at_most(1)?;
            let limit = i32::from(name == "deleteOne");
            doc! { "delete": collection, "deletes": [{ "q": args.required_document(0)?, "limit": limit }] }
        }
        other => {
            return Err(AppError::Invalid(format!(
                "Explain doesn't support `{other}`. It plans find, findOne, aggregate, countDocuments, distinct, \
                 update, replace and delete statements."
            )));
        }
    })
}

/// The explain reply as `[{ "Plan": …, "Execution Time": … }]`.
fn plan(reply: &Document, analyze: bool) -> Result<Value> {
    // An aggregation the query engine can't run whole: a `$cursor` stage that
    // reads the collection, then each remaining stage on top of the previous one.
    let root = if let Ok(stages) = reply.get_array("stages") {
        let mut below: Option<Value> = None;
        for stage in stages.iter().filter_map(Bson::as_document) {
            let Some((name, spec)) = stage.iter().next() else { continue };
            let mut node = if name == "$cursor" {
                let cursor = spec.as_document().ok_or_else(no_plan)?;
                query_plan(cursor, analyze)?
            } else {
                let mut fields = Map::new();
                fields.insert("Node Type".into(), name.as_str().into());
                fields.insert("Detail".into(), literal(spec).into());
                actuals(&mut fields, stage, analyze);
                Value::Object(fields)
            };
            if let (Some(child), Value::Object(fields)) = (below.take(), &mut node) {
                fields.insert("Plans".into(), json!([child]));
            }
            below = Some(node);
        }
        below.ok_or_else(no_plan)?
    } else {
        query_plan(reply, analyze)?
    };

    let mut top = Map::new();
    top.insert("Plan".into(), root);
    if analyze && let Some(time) = execution_stats(reply).and_then(|s| number(s.get("executionTimeMillis"))) {
        top.insert("Execution Time".into(), time.into());
    }
    Ok(json!([top]))
}

/// The plan of a `queryPlanner` reply, with the counts of `executionStats`
/// when they were asked for.
fn query_plan(reply: &Document, analyze: bool) -> Result<Value> {
    let winning =
        reply.get_document("queryPlanner").and_then(|p| p.get_document("winningPlan")).map_err(|_| no_plan())?;
    let stats = execution_stats(reply);
    if let Ok(plan) = winning.get_document("queryPlan") {
        // The slot-based engine reports its own low-level stages when it runs;
        // the plan it was built from reads better, with the totals on its root.
        let mut root = stage(plan, false);
        if let (true, Some(stats), Value::Object(fields)) = (analyze, stats, &mut root) {
            fields.insert("Actual Rows".into(), number(stats.get("nReturned")).unwrap_or_default().into());
            fields.insert(
                "Actual Total Time".into(),
                number(stats.get("executionTimeMillis")).unwrap_or_default().into(),
            );
            fields.insert("Actual Loops".into(), 1.into());
        }
        return Ok(root);
    }
    match (analyze, stats.and_then(|s| s.get_document("executionStages").ok())) {
        (true, Some(stages)) => Ok(stage(stages, true)),
        _ => Ok(stage(winning, false)),
    }
}

fn execution_stats(reply: &Document) -> Option<&Document> {
    reply.get_document("executionStats").ok()
}

/// One plan stage and, under `Plans`, the stages that feed it.
fn stage(stage: &Document, analyze: bool) -> Value {
    let mut fields = Map::new();
    fields.insert("Node Type".into(), stage.get_str("stage").unwrap_or("?").into());
    let detail = detail(stage);
    if !detail.is_empty() {
        fields.insert("Detail".into(), detail.into());
    }
    actuals(&mut fields, stage, analyze);
    for (name, key) in [("Docs Examined", "docsExamined"), ("Keys Examined", "keysExamined")] {
        if let Some(count) = number(stage.get(key)) {
            fields.insert(name.into(), count.into());
        }
    }
    let mut children: Vec<&Document> = Vec::new();
    for key in ["inputStage", "outerStage", "innerStage", "thenStage", "elseStage"] {
        children.extend(stage.get_document(key).ok());
    }
    if let Ok(inputs) = stage.get_array("inputStages") {
        children.extend(inputs.iter().filter_map(Bson::as_document));
    }
    if !children.is_empty() {
        fields.insert("Plans".into(), children.into_iter().map(|child| self::stage(child, analyze)).collect());
    }
    Value::Object(fields)
}

fn actuals(fields: &mut Map<String, Value>, stage: &Document, analyze: bool) {
    if analyze {
        fields.insert("Actual Rows".into(), number(stage.get("nReturned")).unwrap_or_default().into());
        let time = number(stage.get("executionTimeMillisEstimate")).unwrap_or_default();
        fields.insert("Actual Total Time".into(), time.into());
        fields.insert("Actual Loops".into(), 1.into());
    }
}

/// What a stage works on: `using customer_id_1 {"customer_id": 1} · Filter: …`.
fn detail(stage: &Document) -> String {
    let mut parts = Vec::new();
    if let Ok(index) = stage.get_str("indexName") {
        let mut part = format!("using {index}");
        if let Ok(keys) = stage.get_document("keyPattern") {
            part.push_str(&format!(" {}", literal(&Bson::Document(keys.clone()))));
        }
        if stage.get_str("direction") == Ok("backward") {
            part.push_str(" backward");
        }
        parts.push(part);
    }
    for (label, key) in [("Filter", "filter"), ("Sort Key", "sortPattern"), ("Projection", "transformBy")] {
        if let Some(value) = stage.get(key).filter(|v| v.as_document().is_none_or(|d| !d.is_empty())) {
            parts.push(format!("{label}: {}", literal(value)));
        }
    }
    for (label, key) in [("Limit", "limitAmount"), ("Skip", "skipAmount")] {
        if let Some(amount) = number(stage.get(key)) {
            parts.push(format!("{label}: {amount}"));
        }
    }
    parts.join(" · ")
}

fn number(value: Option<&Bson>) -> Option<f64> {
    match value? {
        Bson::Int32(n) => Some(f64::from(*n)),
        Bson::Int64(n) => Some(*n as f64),
        Bson::Double(n) => Some(*n),
        _ => None,
    }
}

fn no_plan() -> AppError {
    AppError::Invalid("This statement has no plan to show.".into())
}

#[cfg(test)]
mod tests {
    use super::super::fetch_rows;
    use super::super::tests::{PASSWORD, database, table};
    use super::*;

    fn types(plan: &Value) -> Vec<String> {
        let mut found = vec![plan["Node Type"].as_str().unwrap().to_string()];
        for child in plan["Plans"].as_array().into_iter().flatten() {
            found.extend(types(child));
        }
        found
    }

    async fn orders(name: &str) -> Connection {
        let orders = (1..=20).map(|id| doc! { "_id": id, "customer": id % 4, "total": id * 10 }).collect();
        let connection = database(name, &[("orders", orders)]).await;
        let client = connect(&connection, PASSWORD).await.unwrap();
        let index = mongodb::IndexModel::builder().keys(doc! { "customer": 1 }).build();
        client.database(name).collection::<Document>("orders").create_index(index).await.unwrap();
        client.shutdown().await;
        connection
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn shows_the_winning_plan_and_its_counts() {
        let connection = orders("dbforge_explain").await;
        let script = "db.orders.find({customer: 1, total: {$gt: 50}}).sort({total: -1})";
        let plan = explain(&connection, PASSWORD, None, script, false).await.unwrap();
        let root = &plan[0]["Plan"];
        assert_eq!(types(root), ["SORT", "FETCH", "IXSCAN"], "{root}");
        assert_eq!(root["Detail"], r#"Sort Key: {"total": -1}"#);
        assert!(
            root["Plans"][0]["Plans"][0]["Detail"].as_str().unwrap().starts_with(r#"using customer_1 {"customer": 1}"#)
        );
        assert!(root.get("Actual Rows").is_none());

        let plan = explain(&connection, PASSWORD, None, script, true).await.unwrap();
        let root = &plan[0]["Plan"];
        assert_eq!(root["Actual Rows"], 3.0, "{root}");
        assert_eq!(root["Plans"][0]["Docs Examined"], 5.0, "{root}");
        assert!(plan[0]["Execution Time"].is_number());

        let plan = explain(&connection, PASSWORD, None, "db.orders.find()", false).await.unwrap();
        assert_eq!(types(&plan[0]["Plan"]), ["COLLSCAN"]);
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn aggregations_show_each_stage_and_writes_never_apply() {
        let connection = orders("dbforge_explain_agg").await;
        // `$addFields` after `$sort` stays outside the query engine.
        let script = "db.orders.aggregate([{$match: {customer: 1}}, {$sort: {total: -1}}, {$addFields: {x: 1}}])";
        let plan = explain(&connection, PASSWORD, None, script, true).await.unwrap();
        let root = &plan[0]["Plan"];
        assert_eq!(root["Node Type"], "$addFields", "{root}");
        assert_eq!(root["Actual Rows"], 5.0, "{root}");
        assert!(types(root).contains(&"IXSCAN".to_string()), "{root}");

        // Pushed down whole, `$group` runs in the slot-based engine.
        let script =
            "db.orders.aggregate([{$match: {customer: {$gt: 0}}}, {$group: {_id: '$customer', n: {$sum: 1}}}])";
        let plan = explain(&connection, PASSWORD, None, script, true).await.unwrap();
        assert_eq!(plan[0]["Plan"]["Node Type"], "GROUP", "{plan}");
        assert_eq!(plan[0]["Plan"]["Actual Rows"], 3.0, "{plan}");

        for script in ["db.orders.deleteMany({})", "db.orders.updateMany({}, {$set: {total: 0}})"] {
            explain(&connection, PASSWORD, None, script, true).await.unwrap();
        }
        let page = fetch_rows(&connection, PASSWORD, &table("dbforge_explain_agg", "orders"), 0, 100).await.unwrap();
        assert_eq!(page.rows.len(), 20);
        assert_eq!(page.rows[0][2].as_deref(), Some("10"));

        let error =
            explain(&connection, PASSWORD, None, "db.orders.find()\ndb.orders.find()", false).await.unwrap_err();
        assert_eq!(error.to_string(), "Explain one db.<collection>.<method>(…) statement at a time.");
        let error = explain(&connection, PASSWORD, None, "db.orders.insertOne({})", false).await.unwrap_err();
        assert!(error.to_string().starts_with("Explain doesn't support `insertOne`"), "{error}");
    }
}
