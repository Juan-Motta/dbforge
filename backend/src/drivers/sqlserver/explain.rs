//! SQL Server query plans. With `SHOWPLAN_XML` on, the server returns the
//! estimated plan of a batch as XML instead of running it; with `STATISTICS XML`
//! on, it runs the batch and returns the plan with what each operator really
//! did. This turns the plan's operator tree into the JSON shape PostgreSQL's
//! `EXPLAIN (FORMAT JSON[, ANALYZE])` has, so one plan view serves every engine.

use roxmltree::{Document, Node};
use serde_json::{Map, Value, json};

use futures_util::TryStreamExt;
use tiberius::QueryItem;

use super::{Client, connect, with_timeout};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// The column `STATISTICS XML` returns the actual plan in.
const SHOWPLAN_COLUMN: &str = "Microsoft SQL Server 2005 XML Showplan";

/// The plan of one statement. Without `analyze` nothing runs: with
/// `SHOWPLAN_XML` on, the server only compiles what it is sent.
///
/// With `analyze` the statement really runs, on its own connection and inside a
/// transaction that is always rolled back. A batch can hold several statements,
/// so the estimated plan is read first and the statement runs only when it is
/// exactly one that can't end the transaction (no `COMMIT`, no procedure that
/// might commit on its own).
pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    analyze: bool,
) -> Result<Value> {
    let mut client = with_timeout(connect(connection, password, database)).await?;
    let estimated = estimated_plan(&mut client, sql).await;
    let result = match estimated {
        Ok(xml) if analyze => match statement(&Document::parse(&xml).map_err(unreadable)?) {
            Ok(_) => actual_plan(&mut client, sql).await,
            Err(error) => Err(error),
        },
        other => other,
    };
    client.close().await?;
    parse(&result?)
}

async fn estimated_plan(client: &mut Client, sql: &str) -> Result<String> {
    // SET SHOWPLAN_XML must be alone in its batch.
    client.simple_query("SET SHOWPLAN_XML ON").await?.into_results().await?;
    let rows = client.simple_query(sql).await?.into_first_result().await;
    client.simple_query("SET SHOWPLAN_XML OFF").await?.into_results().await?;
    Ok(rows?.first().and_then(|row| row.get::<&str, _>(0).map(String::from)).unwrap_or_default())
}

/// Runs `sql` in a transaction that is rolled back, keeping only the plan
/// `STATISTICS XML` adds after the statement's own results.
async fn actual_plan(client: &mut Client, sql: &str) -> Result<String> {
    client.simple_query("BEGIN TRANSACTION").await?.into_results().await?;
    client.simple_query("SET STATISTICS XML ON").await?.into_results().await?;
    let run = async {
        let mut stream = client.simple_query(sql).await?;
        let mut plan = None;
        let mut is_plan = false;
        while let Some(item) = stream.try_next().await? {
            match item {
                QueryItem::Metadata(meta) => {
                    is_plan = meta.columns().first().is_some_and(|c| c.name() == SHOWPLAN_COLUMN);
                }
                // The statement's own rows are read and dropped.
                QueryItem::Row(row) if is_plan => plan = row.get::<&str, _>(0).map(String::from),
                QueryItem::Row(_) => {}
            }
        }
        Ok::<_, AppError>(plan.unwrap_or_default())
    }
    .await;
    client.simple_query("SET STATISTICS XML OFF").await?.into_results().await?;
    client.simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION").await?.into_results().await?;
    run
}

fn unreadable(error: roxmltree::Error) -> AppError {
    AppError::Invalid(format!("Unreadable plan: {error}"))
}

/// The plan's one statement; refused when there are several, or when it is one
/// that could end the transaction an analyzed run is rolled back in.
fn statement<'a, 'input>(document: &'a Document<'input>) -> Result<Node<'a, 'input>> {
    let statements: Vec<Node> = document
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name().starts_with("Stmt") && n.attribute("StatementText").is_some())
        .collect();
    let [statement] = statements.as_slice() else {
        return Err(AppError::Invalid("Explain one statement at a time.".into()));
    };
    let kind = statement.attribute("StatementType").unwrap_or_default();
    if kind.contains("TRANSACTION") || kind.starts_with("EXECUTE") {
        return Err(AppError::Invalid(format!(
            "Analyze runs the statement in a transaction it rolls back, so it can't run {kind} statements."
        )));
    }
    Ok(*statement)
}

fn parse(xml: &str) -> Result<Value> {
    let document = Document::parse(xml).map_err(unreadable)?;
    let statement = statement(&document).or_else(|error| {
        // Transaction statements have a plan to look at; only running them is refused.
        let simple: Vec<Node> = document
            .descendants()
            .filter(|n| n.has_tag_name("StmtSimple") && n.attribute("StatementText").is_some())
            .collect();
        match simple.as_slice() {
            [one] => Ok(*one),
            _ => Err(error),
        }
    })?;
    let root = statement
        .descendants()
        .find(|n| n.has_tag_name("RelOp"))
        // DDL and SET statements have a statement but no operators.
        .ok_or_else(|| AppError::Invalid("This statement has no plan to show.".into()))?;
    let mut plan = node(root);
    if statement.descendants().any(|n| n.has_tag_name("RunTimeInformation")) {
        fill_actuals(&mut plan);
    }
    let mut top = Map::new();
    top.insert("Plan".into(), plan);
    let elapsed =
        statement.descendants().find(|n| n.has_tag_name("QueryTimeStats")).and_then(|n| n.attribute("ElapsedTime"));
    if let Some(ms) = elapsed.and_then(|v| v.parse::<f64>().ok()) {
        top.insert("Execution Time".into(), ms.into());
    }
    Ok(json!([top]))
}

/// SQL Server doesn't count operators that only pass rows on (`Compute Scalar`
/// and the like); in an analyzed plan they take the counts of the operator that
/// feeds them, so every operator, the root included, has actual figures.
fn fill_actuals(node: &mut Value) {
    let Some(fields) = node.as_object_mut() else { return };
    if let Some(Value::Array(children)) = fields.get_mut("Plans") {
        children.iter_mut().for_each(fill_actuals);
    }
    if fields.contains_key("Actual Rows") {
        return;
    }
    let source = fields.get("Plans").and_then(|plans| plans.get(0)).cloned();
    for key in ["Actual Rows", "Actual Total Time", "Actual Loops"] {
        let value = source.as_ref().and_then(|child| child.get(key)).cloned().unwrap_or_else(|| 0.0.into());
        fields.insert(key.into(), value);
    }
}

/// One operator and, under `Plans`, the operators that feed it.
fn node(op: Node) -> Value {
    let physical = op.attribute("PhysicalOp").unwrap_or("?");
    let logical = op.attribute("LogicalOp").unwrap_or(physical);
    let mut fields = Map::new();
    // Written the way SSMS labels operators: `Hash Match (Inner Join)`.
    let node_type = if logical == physical { physical.to_string() } else { format!("{physical} ({logical})") };
    fields.insert("Node Type".into(), node_type.into());
    let detail = detail(op);
    if !detail.is_empty() {
        fields.insert("Detail".into(), detail.into());
    }
    for (name, attribute) in [
        ("Plan Rows", "EstimateRows"),
        ("Total Cost", "EstimatedTotalSubtreeCost"),
        ("Estimated I/O", "EstimateIO"),
        ("Estimated CPU", "EstimateCPU"),
        ("Average Row Size", "AvgRowSize"),
    ] {
        if let Some(number) = op.attribute(attribute).and_then(|v| v.parse::<f64>().ok()) {
            fields.insert(name.into(), number.into());
        }
    }
    // An analyzed plan: rows and time per execution, as PostgreSQL reports per loop.
    let counters: Vec<Node> = op
        .children()
        .filter(|n| n.has_tag_name("RunTimeInformation"))
        .flat_map(|n| n.children().filter(|c| c.has_tag_name("RunTimeCountersPerThread")))
        .collect();
    if !counters.is_empty() {
        let sum = |name: &str| counters.iter().filter_map(|c| c.attribute(name)?.parse::<f64>().ok()).sum::<f64>();
        // Threads of a parallel operator run at the same time: its time is the slowest one's.
        let elapsed =
            counters.iter().filter_map(|c| c.attribute("ActualElapsedms")?.parse::<f64>().ok()).fold(0.0, f64::max);
        let loops = sum("ActualExecutions");
        let per_loop = |total: f64| if loops > 0.0 { total / loops } else { 0.0 };
        fields.insert("Actual Rows".into(), per_loop(sum("ActualRows")).into());
        fields.insert("Actual Total Time".into(), per_loop(elapsed).into());
        fields.insert("Actual Loops".into(), loops.into());
    }
    if op.attribute("Parallel").is_some_and(|p| p == "1" || p == "true") {
        fields.insert("Parallel".into(), true.into());
    }
    let children: Vec<Value> = own(op).filter(|n| n.has_tag_name("RelOp")).map(node).collect();
    if !children.is_empty() {
        fields.insert("Plans".into(), children.into());
    }
    Value::Object(fields)
}

/// Descendants of `op` that belong to it: the walk stops at nested operators,
/// which are yielded but not entered.
fn own<'a, 'input>(op: Node<'a, 'input>) -> impl Iterator<Item = Node<'a, 'input>> {
    let mut stack: Vec<Node> = op.children().filter(Node::is_element).collect();
    stack.reverse();
    std::iter::from_fn(move || {
        let next = stack.pop()?;
        if !next.has_tag_name("RelOp") {
            stack.extend(next.children().filter(Node::is_element).collect::<Vec<_>>().into_iter().rev());
        }
        Some(next)
    })
}

/// What the operator works on and filters by: `[dbo].[orders] [o] using [ix] · Seek: …`.
fn detail(op: Node) -> String {
    let mut parts = Vec::new();
    let object = own(op).find(|n| n.has_tag_name("Object") && n.attribute("Table").is_some());
    if let Some(object) = object {
        let mut name =
            [object.attribute("Schema"), object.attribute("Table")].into_iter().flatten().collect::<Vec<_>>().join(".");
        if let Some(alias) = object.attribute("Alias") {
            name.push_str(&format!(" {alias}"));
        }
        parts.push(name);
        if let Some(index) = object.attribute("Index") {
            parts.push(format!("using {index}"));
        }
    }
    for (label, tag) in [("Seek", "SeekPredicates"), ("Predicate", "Predicate"), ("Residual", "ProbeResidual")] {
        let conditions: Vec<&str> = own(op)
            .filter(|n| n.has_tag_name(tag))
            .filter_map(|n| n.descendants().find(|d| d.has_tag_name("ScalarOperator"))?.attribute("ScalarString"))
            .collect();
        if !conditions.is_empty() {
            parts.push(format!("{label}: {}", conditions.join(" AND ")));
        }
    }
    let sort_keys: Vec<String> = own(op)
        .filter(|n| n.has_tag_name("OrderByColumn"))
        .filter_map(|n| {
            let column = n.descendants().find(|d| d.has_tag_name("ColumnReference"))?.attribute("Column")?;
            let descending = n.attribute("Ascending") == Some("0") || n.attribute("Ascending") == Some("false");
            Some(format!("{column}{}", if descending { " DESC" } else { "" }))
        })
        .collect();
    if !sort_keys.is_empty() {
        parts.push(format!("Sort Key: {}", sort_keys.join(", ")));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::super::fetch_rows;
    use super::super::tests::{PASSWORD, database, table};
    use super::*;

    const SHOP: &str = "
        CREATE TABLE customers (id int PRIMARY KEY, country char(2));
        CREATE TABLE orders (id int PRIMARY KEY, customer_id int NOT NULL, total decimal(10,2));
        CREATE INDEX ix_customer ON orders (customer_id);
        INSERT INTO customers VALUES (1, 'US');
        INSERT INTO orders VALUES (1, 1, 5);";

    fn types(plan: &Value) -> Vec<String> {
        let mut found = vec![plan["Node Type"].as_str().unwrap().to_string()];
        for child in plan["Plans"].as_array().into_iter().flatten() {
            found.extend(types(child));
        }
        found
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn shows_the_estimated_plan_as_a_tree() {
        let connection = database("dbforge_explain", SHOP).await;
        let sql = "SELECT c.country, COUNT(*) FROM orders o JOIN customers c ON c.id = o.customer_id
                   WHERE o.total > 10 GROUP BY c.country ORDER BY c.country";
        let plan = explain(&connection, PASSWORD, None, sql, false).await.unwrap();
        let root = &plan[0]["Plan"];
        assert!(root["Total Cost"].as_f64().unwrap() > 0.0, "{root}");
        let all = types(root);
        assert!(all.iter().any(|t| t.contains("Join")), "{all:?}");
        let text = root.to_string();
        assert!(text.contains("[dbo].[orders] [o]") && text.contains("Predicate: "), "{text}");

        // Nothing runs: the delete is only compiled.
        explain(&connection, PASSWORD, None, "DELETE FROM orders", false).await.unwrap();
        let orders = fetch_rows(
            &connection,
            PASSWORD,
            &table("dbforge_explain", "orders"),
            &crate::drivers::RowFilter::default(),
            0,
            10,
        )
        .await
        .unwrap();
        assert_eq!(orders.rows.len(), 1);

        let error = explain(&connection, PASSWORD, None, "SELECT 1 FROM orders; SELECT 2 FROM orders", false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Explain one statement at a time.");
    }

    #[tokio::test]
    #[ignore = "needs a local SQL Server on port 51433"]
    async fn analyze_runs_the_statement_and_rolls_it_back() {
        let connection = database("dbforge_analyze", SHOP).await;
        let sql =
            "SELECT c.country, COUNT(*) FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.country";
        let plan = explain(&connection, PASSWORD, None, sql, true).await.unwrap();
        let root = &plan[0]["Plan"];
        assert_eq!(root["Actual Rows"], 1.0, "{root}");
        assert!(root["Actual Loops"].as_f64().unwrap() >= 1.0 && root["Actual Total Time"].is_number(), "{root}");
        assert!(plan[0]["Execution Time"].is_number(), "{plan}");

        // The delete really runs (one row), then is rolled back.
        let plan = explain(&connection, PASSWORD, None, "DELETE FROM orders", true).await.unwrap();
        assert_eq!(plan[0]["Plan"]["Actual Rows"], 1.0, "{plan}");
        let orders = fetch_rows(
            &connection,
            PASSWORD,
            &table("dbforge_analyze", "orders"),
            &crate::drivers::RowFilter::default(),
            0,
            10,
        )
        .await
        .unwrap();
        assert_eq!(orders.rows.len(), 1);

        // Nothing runs that could end the transaction it is rolled back in.
        for (sql, message) in [
            ("DELETE FROM orders; COMMIT", "Explain one statement at a time."),
            ("IF 1 = 1 DELETE FROM orders", "Explain one statement at a time."),
            (
                "COMMIT",
                "Analyze runs the statement in a transaction it rolls back, so it can't run COMMIT TRANSACTION statements.",
            ),
        ] {
            let error = explain(&connection, PASSWORD, None, sql, true).await.unwrap_err();
            assert_eq!(error.to_string(), message, "{sql}");
        }
        let orders = fetch_rows(
            &connection,
            PASSWORD,
            &table("dbforge_analyze", "orders"),
            &crate::drivers::RowFilter::default(),
            0,
            10,
        )
        .await
        .unwrap();
        assert_eq!(orders.rows.len(), 1);
    }

    #[test]
    fn operators_keep_their_own_details_and_children() {
        let xml = r#"<ShowPlanXML><BatchSequence><Batch><Statements>
            <StmtSimple StatementText="SELECT ..."><QueryPlan>
              <RelOp PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="3" EstimatedTotalSubtreeCost="0.5">
                <RunTimeInformation>
                  <RunTimeCountersPerThread Thread="1" ActualRows="4" ActualElapsedms="7" ActualExecutions="1"/>
                  <RunTimeCountersPerThread Thread="2" ActualRows="2" ActualElapsedms="9" ActualExecutions="1"/>
                </RunTimeInformation>
                <NestedLoops>
                  <RelOp PhysicalOp="Index Seek" LogicalOp="Index Seek" EstimateRows="1" EstimatedTotalSubtreeCost="0.1">
                    <IndexScan><Object Schema="[dbo]" Table="[t]" Index="[ix]" Alias="[x]"/>
                      <SeekPredicates><SeekPredicateNew><SeekKeys><Prefix>
                        <RangeExpressions><ScalarOperator ScalarString="[x].[id]=(1)"/></RangeExpressions>
                      </Prefix></SeekKeys></SeekPredicateNew></SeekPredicates>
                    </IndexScan>
                  </RelOp>
                  <RelOp PhysicalOp="Sort" LogicalOp="Sort" EstimateRows="3" EstimatedTotalSubtreeCost="0.2">
                    <Sort><OrderBy><OrderByColumn Ascending="false"><ColumnReference Column="total"/></OrderByColumn></OrderBy></Sort>
                  </RelOp>
                </NestedLoops>
              </RelOp>
            </QueryPlan></StmtSimple></Statements></Batch></BatchSequence></ShowPlanXML>"#;
        let plan = parse(xml).unwrap();
        let root = &plan[0]["Plan"];
        assert_eq!(root["Node Type"], "Nested Loops (Inner Join)");
        // The seek's table is not the join's.
        assert!(root.get("Detail").is_none(), "{root}");
        assert_eq!(root["Plans"][0]["Detail"], "[dbo].[t] [x] · using [ix] · Seek: [x].[id]=(1)");
        assert_eq!(root["Plans"][1]["Detail"], "Sort Key: total DESC");
        assert_eq!((root["Plan Rows"].as_f64(), root["Total Cost"].as_f64()), (Some(3.0), Some(0.5)));
        // Two threads ran once each: 6 rows in all, 3 per execution, in the slower thread's 9 ms.
        assert_eq!(
            (root["Actual Rows"].as_f64(), root["Actual Loops"].as_f64(), root["Actual Total Time"].as_f64()),
            (Some(3.0), Some(2.0), Some(4.5))
        );
        // Operators SQL Server doesn't count take their input's figures.
        assert_eq!(root["Plans"][0]["Actual Rows"], 0.0, "{root}");
    }
}
