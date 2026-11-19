//! A statement's plan as the console shows it, read from the JSON the core
//! returns (`EXPLAIN (FORMAT JSON[, ANALYZE])`, or each engine's plan in that
//! shape): its nodes flattened depth-first, each with what it works on, its
//! own time or cost (its children's left out), its share of the whole and
//! the warnings worth a look. Pure, so it is tested without a window.

use serde_json::{Map, Value};

use crate::i18n::{PlanMessages, thousands};

/// One node of a plan, flattened in display order (depth-first).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanNode {
    pub depth: usize,
    pub node_type: String,
    /// What the node works on: table, index, join or filter condition, sort key…
    pub detail: String,
    /// `None` when the engine didn't estimate this node (MySQL sorts, temporary tables).
    pub estimated_rows: Option<f64>,
    /// Rows produced over all loops; `None` without ANALYZE.
    pub actual_rows: Option<f64>,
    /// Time spent in this node alone, children excluded; `None` without ANALYZE.
    pub self_ms: Option<f64>,
    /// Planner cost of this node alone, children excluded.
    pub self_cost: f64,
    /// Share of the whole plan: of time when analyzed, else of cost. 0–100.
    pub share: u32,
    pub warnings: Vec<String>,
    /// Every scalar field the server reported, for the detail pane.
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    /// Rows the statement returned; `None` without ANALYZE.
    pub rows: Option<f64>,
    pub nodes: Vec<PlanNode>,
}

impl Plan {
    /// The costliest node: what the user usually came for.
    pub fn costliest(&self) -> usize {
        let mut best = 0;
        for (index, node) in self.nodes.iter().enumerate() {
            if node.share > self.nodes[best].share {
                best = index;
            }
        }
        best
    }

    pub fn warning_count(&self) -> usize {
        self.nodes.iter().map(|node| node.warnings.len()).sum()
    }
}

/// Reads the plan the core returned; the texts it writes (warnings,
/// details) are in the language of `m`. A plan in another shape is an error.
pub fn parse_plan(json: &Value, m: &PlanMessages) -> Result<Plan, String> {
    let top = match json {
        Value::Array(items) => items.first(),
        other => Some(other),
    }
    .and_then(Value::as_object);
    let (Some(top), Some(root)) = (top, top.and_then(|top| top.get("Plan")).and_then(Value::as_object)) else {
        return Err("The server returned a plan in a shape DBForge can't read.".into());
    };
    let analyzed = root.contains_key("Actual Total Time");

    let mut nodes = Vec::new();
    walk(root, 0, 1., analyzed, m, &mut nodes);

    // Shares follow measured time; a plan too fast to time (SQL Server counts
    // whole milliseconds) shares by estimated cost instead.
    let timed = analyzed && nodes.iter().any(|node| node.self_ms.unwrap_or(0.) > 0.);
    let weight = |node: &PlanNode| if timed { node.self_ms.unwrap_or(0.) } else { node.self_cost };
    let total: f64 = nodes.iter().map(weight).sum();
    let shares: Vec<u32> =
        nodes.iter().map(|node| if total > 0. { (weight(node) / total * 100.).round() as u32 } else { 0 }).collect();
    for (node, share) in nodes.iter_mut().zip(shares) {
        node.share = share;
    }

    Ok(Plan {
        analyzed,
        planning_ms: top.get("Planning Time").and_then(Value::as_f64),
        execution_ms: top.get("Execution Time").and_then(Value::as_f64),
        rows: if analyzed { nodes[0].actual_rows } else { None },
        nodes,
    })
}

fn walk(
    raw: &Map<String, Value>,
    depth: usize,
    processes: f64,
    analyzed: bool,
    m: &PlanMessages,
    nodes: &mut Vec<PlanNode>,
) {
    nodes.push(to_node(raw, depth, analyzed, processes, m));
    let child_processes = parallel_processes(raw).unwrap_or(processes);
    for child in children(raw) {
        walk(child, depth + 1, child_processes, analyzed, m, nodes);
    }
}

fn children(raw: &Map<String, Value>) -> impl Iterator<Item = &Map<String, Value>> {
    raw.get("Plans").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_object)
}

/// Below a Gather, each process (the leader and every worker) runs the
/// subtree at the same time, and their loops are added together. How many
/// there were.
fn parallel_processes(raw: &Map<String, Value>) -> Option<f64> {
    let node_type = raw.get("Node Type").and_then(Value::as_str);
    if !matches!(node_type, Some("Gather" | "Gather Merge")) {
        return None;
    }
    Some(num(raw, "Workers Launched", num(raw, "Workers Planned", 0.)) + 1.)
}

fn to_node(raw: &Map<String, Value>, depth: usize, analyzed: bool, processes: f64, m: &PlanMessages) -> PlanNode {
    let child_processes = parallel_processes(raw).unwrap_or(processes);
    let loops = num(raw, "Actual Loops", 1.);
    // "Actual Total Time" and "Actual Rows" are averages per loop. Loops of
    // parallel processes overlap in time, so their total is split among them.
    let total_ms = |node: &Map<String, Value>, in_parallel: f64| {
        num(node, "Actual Total Time", 0.) * num(node, "Actual Loops", 1.) / in_parallel
    };
    let estimated_rows = raw.get("Plan Rows").and_then(Value::as_f64);
    let actual_per_loop = num(raw, "Actual Rows", 0.);
    let children_ms: f64 = children(raw).map(|child| total_ms(child, child_processes)).sum();
    let children_cost: f64 = children(raw).map(|child| num(child, "Total Cost", 0.)).sum();
    let node_type = match raw.get("Node Type") {
        Some(value) => js_string(value),
        None => "undefined".into(),
    };

    let mut node = PlanNode {
        depth,
        node_type,
        detail: describe(raw, m),
        estimated_rows,
        actual_rows: analyzed.then_some(actual_per_loop * loops),
        self_ms: analyzed.then(|| (total_ms(raw, processes) - children_ms).max(0.)),
        self_cost: (num(raw, "Total Cost", 0.) - children_cost).max(0.),
        share: 0,
        warnings: Vec::new(),
        fields: fields_of(raw),
    };

    if analyzed {
        if let Some(estimated) = estimated_rows {
            let (low, high) = (estimated.min(actual_per_loop), estimated.max(actual_per_loop));
            if high >= 100. && high >= low.max(1.) * 10. {
                node.warnings.push((m.stale_statistics)(&rounded(estimated), &rounded(actual_per_loop)));
            }
        }
        let removed = num(raw, "Rows Removed by Filter", 0.) * loops;
        if node.node_type == "Seq Scan" && removed > 1000. && removed > node.actual_rows.unwrap_or(0.) * 10. {
            node.warnings.push((m.discarded_rows)(&rounded(removed)));
        }
    }
    node
}

/// Scalar fields first, then lists joined; child plans are shown as their own nodes.
fn fields_of(raw: &Map<String, Value>) -> Vec<(String, String)> {
    let entries = || raw.iter().filter(|(key, value)| key.as_str() != "Plans" && !value.is_null());
    let scalars = entries().filter(|(_, value)| !value.is_object() && !value.is_array());
    let lists = entries().filter(|(_, value)| value.is_array());
    scalars.chain(lists).map(|(key, value)| (key.clone(), js_string(value))).collect()
}

/// The parts of a node worth a glance, joined: `orders o · Filter: (total > 100)`.
fn describe(raw: &Map<String, Value>, m: &PlanMessages) -> String {
    let text = |key: &str| raw.get(key).and_then(Value::as_str);
    let mut parts: Vec<String> = Vec::new();
    // MySQL plans come with their description already written out.
    if let Some(detail) = text("Detail") {
        parts.push(detail.to_owned());
    }
    if let Some(relation) = text("Relation Name") {
        match text("Alias") {
            Some(alias) if !alias.is_empty() && alias != relation => parts.push(format!("{relation} {alias}")),
            _ => parts.push(relation.to_owned()),
        }
    }
    if let Some(index) = text("Index Name") {
        parts.push((m.using_index)(index));
    }
    for key in ["Hash Cond", "Merge Cond", "Join Filter", "Index Cond", "Recheck Cond", "Filter"] {
        if let Some(condition) = text(key) {
            parts.push(format!("{key}: {condition}"));
        }
    }
    for key in ["Sort Key", "Group Key"] {
        if let Some(value @ Value::Array(_)) = raw.get(key) {
            parts.push(format!("{key}: {}", js_string(value)));
        }
    }
    parts.join(" · ")
}

/// A number field, or `fallback` when it is missing or not a number.
fn num(raw: &Map<String, Value>, key: &str, fallback: f64) -> f64 {
    raw.get(key).and_then(Value::as_f64).unwrap_or(fallback)
}

/// A value as the former web UI wrote it (JavaScript's `String`): a list's items
/// joined with commas, `null` inside one as nothing.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(n), _, _) => n.to_string(),
            (_, Some(n), _) => n.to_string(),
            (_, _, Some(n)) => n.to_string(),
            _ => number.to_string(),
        },
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(", "),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `n` rounded to a whole number, with commas between thousands.
fn rounded(n: f64) -> String {
    let n = n.round();
    if n < 0. { format!("-{}", thousands((-n) as u64)) } else { thousands(n as u64) }
}

/// `n` as the former web UI wrote counts (`toLocaleString('en-US')`): commas
/// between thousands, up to three decimals.
pub fn format_count(n: f64) -> String {
    let negative = n < 0.;
    let thousandths = (n.abs() * 1000.).round() as u64;
    let (whole, fraction) = (thousandths / 1000, thousandths % 1000);
    let mut text = thousands(whole);
    if fraction > 0 {
        let decimals = format!("{fraction:03}");
        text.push('.');
        text.push_str(decimals.trim_end_matches('0'));
    }
    if negative && thousandths > 0 { format!("-{text}") } else { text }
}

/// 0.40 ms, 24.1 ms, 1.2 s
pub fn format_ms(ms: f64) -> String {
    if ms >= 1000. {
        format!("{:.1} s", ms / 1000.)
    } else if ms < 10. {
        format!("{ms:.2} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::i18n::Language;

    fn en() -> &'static PlanMessages {
        &Language::En.messages().query.plan
    }

    fn parse(json: Value) -> Plan {
        parse_plan(&json, en()).expect("a plan")
    }

    /// Shaped like PostgreSQL's EXPLAIN (ANALYZE, FORMAT JSON) output.
    fn analyzed() -> Value {
        json!([{
            "Plan": {
                "Node Type": "Hash Join",
                "Total Cost": 300,
                "Plan Rows": 500,
                "Actual Total Time": 30,
                "Actual Rows": 480,
                "Actual Loops": 1,
                "Hash Cond": "(o.customer_id = c.id)",
                "Plans": [
                    {
                        "Node Type": "Seq Scan",
                        "Relation Name": "orders",
                        "Alias": "o",
                        "Total Cost": 200,
                        "Plan Rows": 10,
                        "Actual Total Time": 24,
                        "Actual Rows": 480,
                        "Actual Loops": 1,
                        "Filter": "(total > 100)",
                        "Rows Removed by Filter": 50000
                    },
                    {
                        "Node Type": "Index Scan",
                        "Relation Name": "customers",
                        "Alias": "customers",
                        "Index Name": "customers_pkey",
                        "Total Cost": 40,
                        "Plan Rows": 1,
                        "Actual Total Time": 0.002,
                        "Actual Rows": 1,
                        "Actual Loops": 480,
                        "Sort Key": ["a", "b"]
                    }
                ]
            },
            "Planning Time": 0.9,
            "Execution Time": 30.5
        }])
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn flattens_the_tree_depth_first() {
        let plan = parse(analyzed());
        let shape: Vec<(usize, &str)> = plan.nodes.iter().map(|n| (n.depth, n.node_type.as_str())).collect();
        assert_eq!(shape, [(0, "Hash Join"), (1, "Seq Scan"), (1, "Index Scan")]);
    }

    #[test]
    fn reads_timings_and_the_rows_returned() {
        let plan = parse(analyzed());
        assert!(plan.analyzed);
        assert_eq!((plan.planning_ms, plan.execution_ms, plan.rows), (Some(0.9), Some(30.5), Some(480.)));
    }

    #[test]
    fn counts_rows_and_time_over_every_loop_without_the_children() {
        let plan = parse(analyzed());
        let [join, scan, index] = &plan.nodes[..] else { panic!("three nodes") };
        assert_eq!(index.actual_rows, Some(480.));
        assert!(close(index.self_ms.unwrap(), 0.96));
        assert_eq!(scan.self_ms, Some(24.));
        assert!(close(join.self_ms.unwrap(), 30. - 24. - 0.96));
    }

    #[test]
    fn gives_each_node_its_share_of_the_total() {
        let plan = parse(analyzed());
        assert_eq!(plan.nodes.iter().map(|n| n.share).collect::<Vec<_>>(), [17, 80, 3]);
        assert_eq!(plan.costliest(), 1);
    }

    #[test]
    fn describes_what_each_node_works_on() {
        let plan = parse(analyzed());
        let details: Vec<&str> = plan.nodes.iter().map(|n| n.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "Hash Cond: (o.customer_id = c.id)",
                "orders o · Filter: (total > 100)",
                "customers · using customers_pkey · Sort Key: a, b"
            ]
        );
    }

    #[test]
    fn warns_about_bad_estimates_and_wasteful_scans() {
        let plan = parse(analyzed());
        assert_eq!(
            plan.nodes[1].warnings,
            [
                "Estimated 10 rows but got 480; statistics may be stale.",
                "Reads 50,000 rows only to discard them; an index on the filter may help."
            ]
        );
        assert!(plan.nodes[0].warnings.is_empty());
        assert_eq!(plan.warning_count(), 2);
    }

    #[test]
    fn writes_its_warnings_and_details_in_the_language_it_is_given() {
        let spanish = parse_plan(&analyzed(), &Language::Es.messages().query.plan).expect("a plan");
        assert_eq!(
            spanish.nodes[1].warnings[0],
            "Se estimaron 10 filas pero hubo 480; las estadísticas pueden estar desactualizadas."
        );
        assert_eq!(spanish.nodes[2].detail, "customers · usando customers_pkey · Sort Key: a, b");
    }

    #[test]
    fn keeps_every_scalar_and_list_field_for_the_detail_pane() {
        let plan = parse(analyzed());
        let fields = &plan.nodes[2].fields;
        assert!(fields.contains(&("Index Name".into(), "customers_pkey".into())));
        assert!(fields.contains(&("Sort Key".into(), "a, b".into())));
        assert_eq!(fields[0], ("Node Type".into(), "Index Scan".into()), "in the server's order, lists last");
        assert_eq!(fields.last(), Some(&("Sort Key".into(), "a, b".into())));
        assert!(fields.contains(&("Actual Total Time".into(), "0.002".into())));
    }

    #[test]
    fn does_not_add_up_the_time_of_workers_that_ran_in_parallel() {
        // From a real plan: a parallel Seq Scan in 3 processes (leader + 2 workers),
        // 232.5 ms each, inside a 252 ms query. Summing the loops gave 697 ms.
        let parallel = parse(json!([{
            "Plan": {
                "Node Type": "Gather",
                "Total Cost": 29000,
                "Plan Rows": 1,
                "Actual Total Time": 240,
                "Actual Rows": 9,
                "Actual Loops": 1,
                "Workers Launched": 2,
                "Plans": [{
                    "Node Type": "Seq Scan",
                    "Parallel Aware": true,
                    "Total Cost": 28204,
                    "Plan Rows": 1,
                    "Actual Total Time": 232.5,
                    "Actual Rows": 3,
                    "Actual Loops": 3,
                    "Rows Removed by Filter": 323327
                }]
            },
            "Execution Time": 252.3
        }]));
        let [gather, scan] = &parallel.nodes[..] else { panic!("two nodes") };
        assert!(close(scan.self_ms.unwrap(), 232.5));
        assert!(close(gather.self_ms.unwrap(), 7.5));
        assert_eq!(scan.actual_rows, Some(9.));
        assert_eq!(scan.warnings, ["Reads 969,981 rows only to discard them; an index on the filter may help."]);
    }

    #[test]
    fn falls_back_to_cost_shares_without_analyze() {
        let estimate = parse(json!([{
            "Plan": {
                "Node Type": "Limit",
                "Total Cost": 100,
                "Plan Rows": 10,
                "Plans": [{ "Node Type": "Seq Scan", "Total Cost": 75, "Plan Rows": 10 }]
            }
        }]));
        assert!(!estimate.analyzed);
        assert_eq!((estimate.execution_ms, estimate.rows), (None, None));
        let nodes: Vec<_> = estimate.nodes.iter().map(|n| (n.actual_rows, n.self_ms, n.share)).collect();
        assert_eq!(nodes, [(None, None, 25), (None, None, 75)]);
    }

    #[test]
    fn shares_by_cost_when_an_analyzed_plan_ran_too_fast_to_time() {
        // SQL Server reports whole milliseconds, so small queries measure 0 everywhere.
        let fast = parse(json!([{
            "Plan": {
                "Node Type": "Compute Scalar",
                "Total Cost": 100,
                "Actual Total Time": 0,
                "Actual Rows": 3,
                "Actual Loops": 1,
                "Plans": [{
                    "Node Type": "Clustered Index Scan",
                    "Total Cost": 75,
                    "Actual Total Time": 0,
                    "Actual Rows": 3,
                    "Actual Loops": 1
                }]
            }
        }]));
        assert!(fast.analyzed);
        let nodes: Vec<_> = fast.nodes.iter().map(|n| (n.actual_rows, n.self_ms, n.share)).collect();
        assert_eq!(nodes, [(Some(3.), Some(0.), 25), (Some(3.), Some(0.), 75)]);
    }

    #[test]
    fn shows_the_detail_mysql_writes_out() {
        let plan =
            parse(json!([{ "Plan": { "Node Type": "Filter", "Detail": "(o.total > 100)", "Plan Rows": 1.33 } }]));
        assert_eq!(plan.nodes[0].node_type, "Filter");
        assert_eq!(plan.nodes[0].detail, "(o.total > 100)");
        assert_eq!(plan.nodes[0].estimated_rows, Some(1.33));
        assert_eq!(plan.planning_ms, None);
    }

    #[test]
    fn leaves_nodes_without_an_estimate_unestimated_and_unwarned() {
        let plan = parse(json!([{
            "Plan": { "Node Type": "Sort", "Actual Total Time": 5, "Actual Rows": 50000, "Actual Loops": 1 }
        }]));
        assert_eq!(plan.nodes[0].estimated_rows, None);
        assert!(plan.nodes[0].warnings.is_empty());
    }

    #[test]
    fn reads_sqlite_s_plan_tree() {
        // As the core writes SQLite's EXPLAIN QUERY PLAN.
        let plan = parse(json!([{ "Plan": {
            "Node Type": "Query",
            "Plans": [
                { "Node Type": "SCAN", "Detail": "o" },
                { "Node Type": "SEARCH", "Detail": "c USING INTEGER PRIMARY KEY (rowid=?)" }
            ]
        } }]));
        assert!(!plan.analyzed);
        let nodes: Vec<(usize, &str, &str)> =
            plan.nodes.iter().map(|n| (n.depth, n.node_type.as_str(), n.detail.as_str())).collect();
        assert_eq!(nodes, [(0, "Query", ""), (1, "SCAN", "o"), (1, "SEARCH", "c USING INTEGER PRIMARY KEY (rowid=?)")]);
        assert_eq!(plan.nodes.iter().map(|n| n.share).collect::<Vec<_>>(), [0, 0, 0], "no costs, no shares");
    }

    #[test]
    fn refuses_a_plan_in_another_shape() {
        assert!(parse_plan(&json!({ "rows": [] }), en()).is_err());
        assert!(parse_plan(&json!([]), en()).is_err());
    }

    #[test]
    fn writes_times_and_counts_as_the_web_ui_did() {
        assert_eq!(format_ms(0.4), "0.40 ms");
        assert_eq!(format_ms(24.12), "24.1 ms");
        assert_eq!(format_ms(1234.), "1.2 s");
        assert_eq!(format_count(48_306.), "48,306");
        assert_eq!(format_count(1.33), "1.33");
        assert_eq!(format_count(0.), "0");
    }
}
