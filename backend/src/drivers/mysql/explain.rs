//! MySQL query plans. MySQL prints plans as an indented text tree
//! (`EXPLAIN FORMAT=TREE`, `EXPLAIN ANALYZE`); this turns that tree into the
//! same JSON shape PostgreSQL's `EXPLAIN (FORMAT JSON)` has, so one plan view
//! serves both engines.

use serde_json::{Map, Value, json};
use sqlx::{AssertSqlSafe, Connection as _, Row};

use super::{connect, with_timeout};
use crate::connections::model::Connection;
use crate::error::{AppError, Result};

/// The plan of one statement. With `analyze` the statement really runs, so it
/// is only allowed for reads: in MySQL a rollback can't undo changes to MyISAM
/// tables and some statements commit on their own. It still runs inside a
/// rolled-back transaction on its own connection. A prepared statement accepts
/// a single statement only, so a script is rejected rather than half-run.
pub async fn explain(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    sql: &str,
    analyze: bool,
) -> Result<Value> {
    let statement = sql.trim().trim_end_matches(';');
    if analyze && !is_read(statement) {
        return Err(AppError::Invalid(
            "On MySQL, Analyze only runs SELECT statements: other statements could commit or change \
             tables a rollback can't undo. Turn Analyze off to see the estimated plan."
                .into(),
        ));
    }
    let mut conn = with_timeout(connect(connection, password, database)).await?;
    let mode = if analyze { "ANALYZE" } else { "FORMAT=TREE" };
    let mut transaction = conn.begin().await?;
    let tree = sqlx::query(AssertSqlSafe(format!("EXPLAIN {mode} {statement}"))).fetch_one(&mut *transaction).await;
    let text = match tree {
        Ok(row) => String::from_utf8_lossy(&row.try_get_unchecked::<Vec<u8>, _>(0)?).into_owned(),
        Err(error) => {
            transaction.rollback().await?;
            return Err(error.into());
        }
    };
    // Single-table UPDATE and DELETE have no tree; the classic table plan still works.
    let plan = if text.contains("<not executable by iterator executor>") {
        let rows = sqlx::query(AssertSqlSafe(format!("EXPLAIN {statement}"))).fetch_all(&mut *transaction).await;
        rows.map(|rows| table_to_plan(&rows))
    } else {
        Ok(tree_to_plan(&text))
    };
    transaction.rollback().await?;
    conn.close().await?;
    plan.map_err(Into::into)
}

/// Builds a plan from classic `EXPLAIN` rows: one node per table access.
fn table_to_plan(rows: &[sqlx::mysql::MySqlRow]) -> Value {
    let text = |row: &sqlx::mysql::MySqlRow, column: &str| -> Option<String> {
        let bytes: Option<Vec<u8>> = row.try_get_unchecked(column).ok().flatten();
        bytes.map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    let nodes: Vec<Value> = rows
        .iter()
        .map(|row| {
            let access = text(row, "type").unwrap_or_default();
            let node_type = match access.as_str() {
                "ALL" => "Table scan",
                "index" => "Index scan",
                "range" => "Index range scan",
                "ref" | "eq_ref" | "const" | "system" | "ref_or_null" => "Index lookup",
                "" => "Step",
                other => other,
            };
            let parts: Vec<String> =
                [text(row, "table"), text(row, "key").map(|key| format!("using {key}")), text(row, "Extra")]
                    .into_iter()
                    .flatten()
                    .collect();
            json!({
                "Node Type": node_type,
                "Detail": parts.join(" · "),
                // A number column: the prepared statement's binary protocol doesn't send it as text.
                "Plan Rows": row.try_get_unchecked::<Option<u64>, _>("rows").ok().flatten().unwrap_or(0),
                "Statement": text(row, "select_type"),
            })
        })
        .collect();
    let root = match <[Value; 1]>::try_from(nodes) {
        Ok([node]) => node,
        Err(nodes) => json!({ "Node Type": "Plan", "Plans": nodes }),
    };
    json!([{ "Plan": root }])
}

/// Whether a statement only reads: its first keyword, past comments and
/// parentheses, is SELECT, WITH, TABLE or VALUES.
fn is_read(sql: &str) -> bool {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
        if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        } else if rest.starts_with("--") || rest.starts_with('#') {
            rest = rest.split_once('\n').map_or("", |(_, tail)| tail);
        } else {
            break;
        }
    }
    let keyword: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    matches!(keyword.to_uppercase().as_str(), "SELECT" | "WITH" | "TABLE" | "VALUES")
}

/// One line of the tree: `-> description  (cost=… rows=…) (actual time=a..b rows=r loops=l)`.
struct Line {
    depth: usize,
    node: Map<String, Value>,
}

/// Builds `[{ "Plan": {...}, "Execution Time": … }]` from MySQL's text tree.
fn tree_to_plan(text: &str) -> Value {
    let mut lines: Vec<Line> = Vec::new();
    for raw in text.lines() {
        let Some(position) = raw.find("-> ") else {
            // A wrapped description continues the previous node.
            if let Some(last) = lines.last_mut()
                && let Some(Value::String(detail)) = last.node.get_mut("Detail")
            {
                detail.push(' ');
                detail.push_str(raw.trim());
            }
            continue;
        };
        lines.push(Line { depth: position / 4, node: parse_line(&raw[position + 3..]) });
    }

    // Folds the flat, indented lines into nested "Plans".
    fn take_children(lines: &mut std::iter::Peekable<std::vec::IntoIter<Line>>, depth: usize) -> Vec<Value> {
        let mut children = Vec::new();
        while lines.peek().is_some_and(|line| line.depth == depth) {
            let mut line = lines.next().expect("peeked");
            let grandchildren = take_children(lines, depth + 1);
            if !grandchildren.is_empty() {
                line.node.insert("Plans".into(), Value::Array(grandchildren));
            }
            children.push(Value::Object(line.node));
        }
        children
    }
    let mut iter = lines.into_iter().peekable();
    let mut roots = take_children(&mut iter, 0);
    let root = if roots.len() == 1 { roots.remove(0) } else { json!({ "Node Type": "Plan", "Plans": roots }) };

    let mut top = Map::new();
    // MySQL reports no separate execution time; the root's last row is when it finished.
    if let (Some(time), Some(loops)) = (root.get("Actual Total Time"), root.get("Actual Loops")) {
        let total = time.as_f64().unwrap_or(0.0) * loops.as_f64().unwrap_or(1.0);
        top.insert("Execution Time".into(), json!(total));
    }
    top.insert("Plan".into(), root);
    Value::Array(vec![Value::Object(top)])
}

fn parse_line(text: &str) -> Map<String, Value> {
    let (description, measures) = match text.find("  (") {
        Some(i) => (&text[..i], &text[i..]),
        None => (text.trim_end(), ""),
    };
    let (node_type, detail) = split_description(description);
    let mut node = Map::new();
    node.insert("Node Type".into(), json!(node_type));
    if let Some(detail) = detail {
        node.insert("Detail".into(), json!(detail));
    }
    if let Some(cost) = between(measures, "(cost=", " ") {
        // `cost=a..b` on some nodes; the total is the last number.
        node.insert("Total Cost".into(), json!(number(cost.rsplit("..").next().unwrap_or(cost))));
    }
    // Only the cost part holds an estimate; MySQL doesn't estimate some nodes (sorts,
    // temporary tables), and their only `rows=` is the measured one.
    if let Some(estimate) = measures.split("(actual").next().filter(|part| part.contains("(cost="))
        && let Some(rows) = between(estimate, "rows=", ")")
    {
        node.insert("Plan Rows".into(), json!(number(rows)));
    }

    if measures.contains("(never executed)") {
        node.insert("Actual Total Time".into(), json!(0.0));
        node.insert("Actual Rows".into(), json!(0.0));
        node.insert("Actual Loops".into(), json!(0));
    } else if let Some(actual) = measures.split("(actual time=").nth(1) {
        let (time, rest) = actual.split_once(' ').unwrap_or((actual, ""));
        let (first, last) = time.split_once("..").unwrap_or((time, time));
        node.insert("Actual Startup Time".into(), json!(number(first)));
        node.insert("Actual Total Time".into(), json!(number(last)));
        node.insert("Actual Rows".into(), json!(between(rest, "rows=", " ").map_or(0.0, number)));
        node.insert("Actual Loops".into(), json!(between(rest, "loops=", ")").map_or(1.0, number)));
    }
    node
}

/// `Table scan on o` → ("Table scan", "o"); `Filter: (a > 1)` → ("Filter", "(a > 1)").
fn split_description(description: &str) -> (&str, Option<&str>) {
    let colon = description.find(": ").map(|i| (i, 2));
    let on = description.find(" on ").map(|i| (i, 4));
    match [colon, on].into_iter().flatten().min() {
        Some((i, width)) => (&description[..i], Some(&description[i + width..])),
        None => (description, None),
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let rest = &text[from..];
    Some(rest.find(end).map_or(rest, |to| &rest[..to]))
}

fn number(text: &str) -> f64 {
    text.trim().parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{PASSWORD, local, run};
    use super::*;

    // Real output of MySQL 8.4's EXPLAIN ANALYZE.
    const ANALYZED: &str = "\
-> Table scan on <temporary>  (actual time=1.01..1.01 rows=2 loops=1)
    -> Aggregate using temporary table  (actual time=1.01..1.01 rows=2 loops=1)
        -> Nested loop inner join  (cost=1.12 rows=1.33) (actual time=0.444..0.447 rows=2 loops=1)
            -> Filter: ((o.total > 100) and (o.cid is not null))  (cost=0.65 rows=1.33) (actual time=0.04..0.0419 rows=2 loops=1)
                -> Table scan on o  (cost=0.65 rows=4) (actual time=0.0216..0.0236 rows=4 loops=1)
            -> Single-row index lookup on c using PRIMARY (id=o.cid)  (cost=0.325 rows=1) (actual time=0.00335..0.0034 rows=1 loops=2)
";

    #[test]
    fn turns_the_text_tree_into_nested_plans() {
        let plan = tree_to_plan(ANALYZED);
        let root = &plan[0]["Plan"];
        assert_eq!(root["Node Type"], "Table scan");
        assert_eq!(root["Detail"], "<temporary>");
        assert_eq!(plan[0]["Execution Time"], 1.01);

        let join = &root["Plans"][0]["Plans"][0];
        assert_eq!(join["Node Type"], "Nested loop inner join");
        assert_eq!(join["Total Cost"], 1.12);
        assert_eq!(join["Plan Rows"], 1.33);

        let [filter, lookup] = join["Plans"].as_array().unwrap().as_slice() else { panic!("{join}") };
        assert_eq!(filter["Node Type"], "Filter");
        assert_eq!(filter["Detail"], "((o.total > 100) and (o.cid is not null))");
        assert_eq!(filter["Plans"][0]["Detail"], "o");
        assert_eq!(lookup["Node Type"], "Single-row index lookup");
        assert_eq!(lookup["Detail"], "c using PRIMARY (id=o.cid)");
        assert_eq!((lookup["Actual Rows"].as_f64(), lookup["Actual Loops"].as_f64()), (Some(1.0), Some(2.0)));
        assert_eq!(lookup["Actual Total Time"], 0.0034);
    }

    #[test]
    fn nodes_mysql_does_not_estimate_have_no_estimate() {
        let plan = tree_to_plan(ANALYZED);
        let root = &plan[0]["Plan"];
        assert!(root.get("Plan Rows").is_none(), "{root}");
        assert_eq!(root["Actual Rows"], 2.0);
    }

    #[test]
    fn estimated_plans_have_no_actual_values() {
        let plan = tree_to_plan("-> Table scan on o  (cost=0.65 rows=4)\n");
        assert!(plan[0]["Plan"].get("Actual Total Time").is_none());
        assert!(plan[0].get("Execution Time").is_none());
    }

    #[test]
    fn recognizes_reads_past_comments_and_parentheses() {
        for read in ["SELECT 1", "  /* why */ select 1", "-- note\n(SELECT 1)", "WITH x AS (SELECT 1) SELECT * FROM x"]
        {
            assert!(is_read(read), "{read}");
        }
        for write in ["UPDATE t SET a = 1", "/* SELECT */ DELETE FROM t", "INSERT INTO t SELECT 1"] {
            assert!(!is_read(write), "{write}");
        }
    }

    #[tokio::test]
    #[ignore = "needs a local MySQL on port 53306"]
    async fn explains_reads_and_refuses_to_analyze_writes() {
        let mut conn = run("DROP DATABASE IF EXISTS dbforge_mexplain;
             CREATE DATABASE dbforge_mexplain;
             CREATE TABLE dbforge_mexplain.items (id int PRIMARY KEY, qty int);
             INSERT INTO dbforge_mexplain.items VALUES (1, 1), (2, 2);")
        .await;

        let analyzed =
            explain(&local(), PASSWORD, Some("dbforge_mexplain"), "SELECT * FROM items WHERE qty > 1;", true)
                .await
                .unwrap();
        assert!(analyzed[0]["Plan"]["Actual Rows"].is_number(), "{analyzed}");

        // A single-table DELETE has no tree plan in MySQL; the classic one is used.
        let estimated = explain(&local(), PASSWORD, Some("dbforge_mexplain"), "DELETE FROM items WHERE qty > 1", false)
            .await
            .unwrap();
        assert_eq!(estimated[0]["Plan"]["Node Type"], "Table scan", "{estimated}");
        assert_eq!(estimated[0]["Plan"]["Plan Rows"], 2, "{estimated}");
        assert!(estimated[0]["Plan"]["Detail"].as_str().unwrap().starts_with("items"), "{estimated}");
        assert!(estimated[0]["Plan"].get("Actual Total Time").is_none(), "{estimated}");

        let error = explain(&local(), PASSWORD, Some("dbforge_mexplain"), "DELETE FROM items", true).await.unwrap_err();
        assert!(error.to_string().contains("only runs SELECT"), "{error}");

        let error = explain(&local(), PASSWORD, Some("dbforge_mexplain"), "SELECT 1; DROP TABLE items", false)
            .await
            .unwrap_err();
        assert!(error.to_string().to_lowercase().contains("syntax"), "{error}");
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM dbforge_mexplain.items").fetch_one(&mut conn).await.unwrap();
        assert_eq!(count, 2, "nothing was deleted or dropped");
    }
}
