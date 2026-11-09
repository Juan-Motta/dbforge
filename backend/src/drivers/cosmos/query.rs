//! Reading a container's items with a query, one page at a time.
//!
//! A container is split into partition key ranges, and the gateway only runs
//! a query with ORDER BY on one of them at a time. So every query runs range
//! by range: without ORDER BY the ranges' items follow one another; with it
//! each range is asked for its items with their sort values (the rewrite the
//! SDKs use) and the ranges are merged in order.

use std::cmp::Ordering;
use std::collections::VecDeque;

use reqwest::Method;
use serde_json::{Value, json};

use super::client::{Client, Reply, Target};
use crate::drivers::RowFilter;
use crate::error::Result;

/// The query a table view asks for: a WHERE condition and ORDER BY terms on `c`.
#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    filter: String,
    order: Vec<OrderTerm>,
}

#[derive(Debug, Clone, PartialEq)]
struct OrderTerm {
    expression: String,
    descending: bool,
}

impl Select {
    /// The filter and sort typed in the table view: `c.total > 100` and `c.total DESC`.
    /// The server checks the expressions themselves.
    pub fn of(filter: &RowFilter) -> Self {
        let order = split_top_level(&filter.order, ',')
            .into_iter()
            .map(|term| {
                let (expression, descending) = match term.rsplit_once(char::is_whitespace) {
                    Some((expression, word)) if word.eq_ignore_ascii_case("desc") => (expression, true),
                    Some((expression, word)) if word.eq_ignore_ascii_case("asc") => (expression, false),
                    _ => (term.as_str(), false),
                };
                OrderTerm { expression: expression.trim().to_string(), descending }
            })
            .collect();
        Self { filter: filter.filter.trim().to_string(), order }
    }

    /// The same items in no particular order, which costs the least to read.
    pub fn unordered(&self) -> Self {
        Self { filter: self.filter.clone(), order: Vec::new() }
    }

    /// The text each range runs: plain, or rewritten to return the sort values
    /// (`orderByItems`) beside the item (`payload`) so ranges can be merged.
    fn text(&self) -> String {
        let condition = if self.filter.is_empty() { String::new() } else { format!(" WHERE {}", self.filter) };
        if self.order.is_empty() {
            return format!("SELECT * FROM c{condition}");
        }
        let items: Vec<String> = self.order.iter().map(|t| format!("{{\"item\": {}}}", t.expression)).collect();
        let order: Vec<String> = self
            .order
            .iter()
            .map(|t| format!("{} {}", t.expression, if t.descending { "DESC" } else { "ASC" }))
            .collect();
        format!(
            "SELECT c._rid, [{}] AS orderByItems, c AS payload FROM c{condition} ORDER BY {}",
            items.join(", "),
            order.join(", ")
        )
    }
}

/// Splits text at the `separator`s outside quotes, brackets and parentheses,
/// dropping empty parts.
pub fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ch if ch == separator && depth == 0 => {
                    parts.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            },
        }
        current.push(ch);
    }
    parts.push(current);
    parts.into_iter().map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect()
}

/// One partition key range being read.
struct Range {
    id: String,
    continuation: Option<String>,
    buffer: VecDeque<Value>,
    done: bool,
}

/// The items of a query, read on demand range by range.
pub struct Items<'a> {
    client: &'a Client,
    target: Target,
    query: Value,
    ordered: Option<Vec<bool>>,
    page_size: String,
    ranges: Vec<Range>,
    current: usize,
}

impl<'a> Items<'a> {
    pub async fn open(
        client: &'a Client,
        database: &str,
        container: &str,
        select: &Select,
        page_size: u32,
    ) -> Result<Self> {
        let mut ranges = client.list(&Target::partition_key_ranges(database, container), "PartitionKeyRanges").await?;
        // In key order, so items without ORDER BY come in the same order every time.
        ranges.sort_by(|a, b| text_of(a, "minInclusive").cmp(text_of(b, "minInclusive")));
        let ranges = ranges
            .iter()
            .map(|range| Range {
                id: text_of(range, "id").to_string(),
                continuation: None,
                buffer: VecDeque::new(),
                done: false,
            })
            .collect();
        let ordered = (!select.order.is_empty()).then(|| select.order.iter().map(|term| term.descending).collect());
        Ok(Self {
            client,
            target: Target::items(database, container),
            query: json!({ "query": select.text(), "parameters": [] }),
            ordered,
            page_size: page_size.to_string(),
            ranges,
            current: 0,
        })
    }

    /// The next item, or `None` once every range is read.
    pub async fn next(&mut self) -> Result<Option<Value>> {
        match self.ordered.clone() {
            None => {
                while self.current < self.ranges.len() {
                    self.fill(self.current).await?;
                    if let Some(item) = self.ranges[self.current].buffer.pop_front() {
                        return Ok(Some(item));
                    }
                    self.current += 1;
                }
                Ok(None)
            }
            Some(descending) => {
                for index in 0..self.ranges.len() {
                    self.fill(index).await?;
                }
                let heads = self.ranges.iter().map(|range| range.buffer.front());
                let Some(index) = first_in_order(heads, &descending) else {
                    return Ok(None);
                };
                let row = self.ranges[index].buffer.pop_front().expect("chosen range has an item");
                Ok(row.get("payload").cloned())
            }
        }
    }

    /// Reads the range's next page when its buffer is empty and it has more.
    async fn fill(&mut self, index: usize) -> Result<()> {
        while self.ranges[index].buffer.is_empty() && !self.ranges[index].done {
            let range = &self.ranges[index];
            let mut headers = vec![
                ("x-ms-documentdb-isquery", "true"),
                ("x-ms-documentdb-partitionkeyrangeid", range.id.as_str()),
                ("x-ms-max-item-count", self.page_size.as_str()),
            ];
            if let Some(token) = &range.continuation {
                headers.push(("x-ms-continuation", token.as_str()));
            }
            let reply = self.client.send(Method::POST, &self.target, &headers, Some(&self.query)).await?;
            let continuation = reply.header("x-ms-continuation").map(String::from);
            let range = &mut self.ranges[index];
            range.buffer.extend(documents(&reply).iter().cloned());
            range.done = continuation.is_none();
            range.continuation = continuation;
        }
        Ok(())
    }
}

/// A query the gateway runs over the whole container, read page by page. The
/// gateway refuses what needs merging across partition key ranges (ORDER BY,
/// aggregates, …) on a container that has several; its message says so.
pub struct Feed<'a> {
    client: &'a Client,
    target: Target,
    query: Value,
    headers: Vec<(&'static str, &'static str)>,
    continuation: Option<String>,
    done: bool,
}

impl<'a> Feed<'a> {
    pub fn new(
        client: &'a Client,
        database: &str,
        container: &str,
        text: &str,
        headers: &[(&'static str, &'static str)],
    ) -> Self {
        Self {
            client,
            target: Target::items(database, container),
            query: json!({ "query": text, "parameters": [] }),
            headers: headers.to_vec(),
            continuation: None,
            done: false,
        }
    }

    /// Values for the query's `@name` parameters.
    pub fn with_parameters(mut self, parameters: &[(&str, Value)]) -> Self {
        let parameters: Vec<Value> =
            parameters.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect();
        self.query["parameters"] = Value::Array(parameters);
        self
    }

    /// The next page's reply, or `None` once the query is read to the end.
    pub async fn next(&mut self) -> Result<Option<Reply>> {
        if self.done {
            return Ok(None);
        }
        let mut headers =
            vec![("x-ms-documentdb-isquery", "true"), ("x-ms-documentdb-query-enablecrosspartition", "true")];
        headers.extend(self.headers.iter().copied());
        if let Some(token) = &self.continuation {
            headers.push(("x-ms-continuation", token.as_str()));
        }
        let reply = self.client.send(Method::POST, &self.target, &headers, Some(&self.query)).await?;
        self.continuation = reply.header("x-ms-continuation").map(String::from);
        self.done = self.continuation.is_none();
        Ok(Some(reply))
    }
}

/// The items of a query reply.
pub fn documents(reply: &Reply) -> &[Value] {
    reply.body.get("Documents").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn text_of<'v>(value: &'v Value, key: &str) -> &'v str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The range whose next item sorts first, by its `orderByItems`.
fn first_in_order<'v>(heads: impl Iterator<Item = Option<&'v Value>>, descending: &[bool]) -> Option<usize> {
    let mut best: Option<(usize, &Value)> = None;
    for (index, head) in heads.enumerate() {
        let Some(head) = head else { continue };
        let better = match best {
            None => true,
            Some((_, current)) => compare_rows(head, current, descending) == Ordering::Less,
        };
        if better {
            best = Some((index, head));
        }
    }
    best.map(|(index, _)| index)
}

fn compare_rows(a: &Value, b: &Value, descending: &[bool]) -> Ordering {
    let items = |row: &Value| row.get("orderByItems").and_then(Value::as_array).cloned().unwrap_or_default();
    let (a, b) = (items(a), items(b));
    for (index, descending) in descending.iter().enumerate() {
        // A term the item lacks comes back as `{}`: undefined, which sorts first.
        let value = |items: &[Value]| items.get(index).and_then(|item| item.get("item")).cloned();
        let ordering = compare_values(value(&a).as_ref(), value(&b).as_ref());
        let ordering = if *descending { ordering.reverse() } else { ordering };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// Cosmos DB's order across types: undefined, null, booleans, numbers,
/// strings, arrays, objects.
fn compare_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    let rank = |value: Option<&Value>| match value {
        None => 0,
        Some(Value::Null) => 1,
        Some(Value::Bool(_)) => 2,
        Some(Value::Number(_)) => 3,
        Some(Value::String(_)) => 4,
        Some(Value::Array(_)) => 5,
        Some(Value::Object(_)) => 6,
    };
    match (a, b) {
        (Some(Value::Bool(a)), Some(Value::Bool(b))) => a.cmp(b),
        (Some(Value::Number(a)), Some(Value::Number(b))) => {
            a.as_f64().unwrap_or_default().total_cmp(&b.as_f64().unwrap_or_default())
        }
        (Some(Value::String(a)), Some(Value::String(b))) => a.cmp(b),
        _ => rank(a).cmp(&rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(filter: &str, order: &str) -> Select {
        Select::of(&RowFilter { filter: filter.into(), order: order.into() })
    }

    #[test]
    fn writes_the_query_each_range_runs() {
        assert_eq!(select("", "").text(), "SELECT * FROM c");
        assert_eq!(select("c.total > 1", "").text(), "SELECT * FROM c WHERE c.total > 1");
        assert_eq!(
            select("c.total > 1", "c.total desc, c[\"a,b\"], ARRAY_LENGTH(c.tags) ASC").text(),
            "SELECT c._rid, [{\"item\": c.total}, {\"item\": c[\"a,b\"]}, {\"item\": ARRAY_LENGTH(c.tags)}] \
             AS orderByItems, c AS payload FROM c WHERE c.total > 1 \
             ORDER BY c.total DESC, c[\"a,b\"] ASC, ARRAY_LENGTH(c.tags) ASC"
        );
    }

    #[test]
    fn splits_outside_quotes_and_brackets() {
        assert_eq!(
            split_top_level("SELECT ';' FROM c; SELECT [1;2] FROM d ;; ", ';'),
            ["SELECT ';' FROM c", "SELECT [1;2] FROM d"]
        );
        assert_eq!(split_top_level(r#"c["a\"b,c"], f(x, y)"#, ','), [r#"c["a\"b,c"]"#, "f(x, y)"]);
    }

    #[test]
    fn merges_ranges_in_cosmos_order() {
        let row = |values: Vec<Value>| json!({ "orderByItems": values.into_iter().map(|v| if v == json!("undefined") { json!({}) } else { json!({ "item": v }) }).collect::<Vec<_>>() });
        let a = row(vec![json!(5), json!("x")]);
        let b = row(vec![json!(5), json!("a")]);
        let c = row(vec![json!("undefined"), json!("z")]);
        let d = row(vec![json!(null), json!("z")]);
        let e = row(vec![json!(true), json!("z")]);
        let heads = [Some(&a), None, Some(&b), Some(&c), Some(&d), Some(&e)];
        // Ascending: undefined first.
        assert_eq!(first_in_order(heads.iter().copied(), &[false, false]), Some(3));
        // Descending on the first term: the number beats null, bool and undefined;
        // ties go to the second term, ascending.
        assert_eq!(first_in_order(heads.iter().copied(), &[true, false]), Some(2));
        assert_eq!(first_in_order([None, None].into_iter(), &[false]), None);
        assert_eq!(compare_values(Some(&json!(2.5)), Some(&json!(10))), Ordering::Less);
        assert_eq!(compare_values(Some(&json!("b")), Some(&json!([1]))), Ordering::Less);
    }
}
