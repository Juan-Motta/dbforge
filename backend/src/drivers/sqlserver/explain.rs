//! SQL Server query plans. With `SHOWPLAN_XML` on, the server returns the
//! estimated plan of a batch as XML instead of running it; this turns the plan's
//! operator tree into the JSON shape PostgreSQL's `EXPLAIN (FORMAT JSON)` has, so
//! one plan view serves every engine.

use roxmltree::{Document, Node};
use serde_json::{Map, Value, json};

use super::{connect, with_timeout};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// The estimated plan of one statement. Nothing runs: with `SHOWPLAN_XML` on,
/// the server only compiles what it is sent.
pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    analyze: bool,
) -> Result<Value> {
    if analyze {
        return Err(AppError::Invalid(
            "Analyze isn't available for SQL Server yet. Turn it off to see the estimated plan.".into(),
        ));
    }
    let mut client = with_timeout(connect(connection, password, database)).await?;
    // SET SHOWPLAN_XML must be alone in its batch.
    client.simple_query("SET SHOWPLAN_XML ON").await?.into_results().await?;
    let rows = client.simple_query(sql).await?.into_first_result().await;
    client.close().await?;

    let xml = rows?.first().and_then(|row| row.get::<&str, _>(0).map(String::from)).unwrap_or_default();
    parse(&xml)
}

fn parse(xml: &str) -> Result<Value> {
    let document = Document::parse(xml).map_err(|e| AppError::Invalid(format!("Unreadable plan: {e}")))?;
    let statements: Vec<Node> = document
        .descendants()
        .filter(|n| n.has_tag_name("StmtSimple") && n.attribute("StatementText").is_some())
        .collect();
    let [statement] = statements.as_slice() else {
        return Err(AppError::Invalid("Explain one statement at a time.".into()));
    };
    let root = statement
        .descendants()
        .find(|n| n.has_tag_name("RelOp"))
        // DDL and SET statements have a statement but no operators.
        .ok_or_else(|| AppError::Invalid("This statement has no plan to show.".into()))?;
    Ok(json!([{ "Plan": node(root) }]))
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
        let orders = fetch_rows(&connection, PASSWORD, &table("dbforge_explain", "orders"), 0, 10).await.unwrap();
        assert_eq!(orders.rows.len(), 1);

        let error = explain(&connection, PASSWORD, None, "SELECT 1 FROM orders; SELECT 2 FROM orders", false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Explain one statement at a time.");
        let error = explain(&connection, PASSWORD, None, "SELECT 1", true).await.unwrap_err();
        assert!(error.to_string().starts_with("Analyze isn't available for SQL Server"), "{error}");
    }

    #[test]
    fn operators_keep_their_own_details_and_children() {
        let xml = r#"<ShowPlanXML><BatchSequence><Batch><Statements>
            <StmtSimple StatementText="SELECT ..."><QueryPlan>
              <RelOp PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="3" EstimatedTotalSubtreeCost="0.5">
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
    }
}
