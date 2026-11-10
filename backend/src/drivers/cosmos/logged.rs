//! How each request the client sends reads in the statement log, and what it
//! returned. [`Client::send`](super::client::Client::send) records every
//! request through [`record`], so nothing reaches the server unrecorded.
//!
//! A query is its Cosmos DB SQL, with its `@parameters` written in as
//! literals, so it can be copied into the console and run:
//! `SELECT * FROM c WHERE c.id = "42"`. A request for a query's plan is the
//! query after `EXPLAIN`. Anything else is one line in a single form,
//!
//! `OPERATION link [PARTITION key] [json]`
//!
//! the operation (`READ`, `LIST`, `CREATE`, `REPLACE`, `DELETE`), the link of
//! the resource as Cosmos DB names it (`/` being the account), the partition
//! key the request is for, and the JSON it sends:
//!
//! - `READ dbs/shop/colls/orders`, `LIST dbs/shop/colls`, `LIST offers`
//! - `CREATE dbs/shop`, the new resource's link; its JSON follows when it holds
//!   more than the id: `CREATE dbs/shop/colls/orders/docs/42 PARTITION ["eu"] {"id":"42",…}`
//! - `REPLACE dbs/shop/colls/orders/docs/42 PARTITION ["eu"] {"id":"42",…}`
//! - `DELETE dbs/shop/colls/orders/docs/42 PARTITION ["eu"]`
//!
//! A query read page by page is one entry per page: every page is a round trip
//! of its own, with its own time, documents and failure. The text is the same
//! query on every page, so each entry stays runnable; the continuation token a
//! page is asked with means nothing outside the request, so it isn't shown.
//! A throttled request sent again is still one entry, its time included.
//!
//! Only what a request is about is written: never the account key, the
//! signature, continuation tokens or any other header.

use std::future::Future;

use reqwest::Method;
use serde_json::Value;

use super::client::{Reply, Target, is_query};
use crate::error::Result;
use crate::statement_log::Recording;

/// The header naming the partition an item request is for, as a JSON array.
const PARTITION_KEY: &str = "x-ms-documentdb-partitionkey";
/// The header that turns a query into a request for its plan.
const PLAN_REQUEST: &str = "x-ms-cosmos-is-query-plan-request";

/// Sends a request with `sent` and records it: its text, how long it took,
/// the documents it returned or changed, or the server's message.
pub(super) async fn record(request: Request<'_>, sent: impl Future<Output = Result<Reply>>) -> Result<Reply> {
    let mut recording = Recording::start(request.database(), || request.text());
    let result = sent.await;
    match &result {
        Ok(reply) => request.count(&mut recording, reply),
        Err(error) => recording.fail(error),
    }
    result
}

/// What a request does, read from its verb, its target and its headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Query,
    Plan,
    Read,
    List,
    Create,
    Replace,
    Delete,
    /// A verb the driver doesn't send today, shown as itself.
    Other,
}

/// A request as the client is about to send it.
pub(super) struct Request<'a> {
    method: &'a Method,
    target: &'a Target,
    headers: &'a [(&'a str, &'a str)],
    body: Option<&'a Value>,
}

impl<'a> Request<'a> {
    pub(super) fn new(
        method: &'a Method,
        target: &'a Target,
        headers: &'a [(&'a str, &'a str)],
        body: Option<&'a Value>,
    ) -> Self {
        Self { method, target, headers, body }
    }

    fn operation(&self) -> Operation {
        let feed = matches!(self.target, Target::Feed { .. });
        if is_query(self.headers) {
            return if self.header(PLAN_REQUEST).is_some() { Operation::Plan } else { Operation::Query };
        }
        match self.method {
            method if *method == Method::GET => {
                if feed {
                    Operation::List
                } else {
                    Operation::Read
                }
            }
            method if *method == Method::POST && feed => Operation::Create,
            method if *method == Method::PUT && !feed => Operation::Replace,
            method if *method == Method::DELETE && !feed => Operation::Delete,
            _ => Operation::Other,
        }
    }

    /// The database the request is on; a new database is on itself.
    fn database(&self) -> Option<&'a str> {
        let creates_database = matches!(self.target, Target::Feed { kind: "dbs", .. });
        self.target.database_name().or_else(|| creates_database.then(|| self.id()).flatten())
    }

    /// The request as the statement log shows it; see the module's docs.
    fn text(&self) -> String {
        let operation = self.operation();
        let verb = match operation {
            Operation::Query => return self.sql(),
            Operation::Plan => return format!("EXPLAIN {}", self.sql()),
            Operation::Read => "READ",
            Operation::List => "LIST",
            Operation::Create => "CREATE",
            Operation::Replace => "REPLACE",
            Operation::Delete => "DELETE",
            Operation::Other => self.method.as_str(),
        };
        let link = self.target.link();
        let mut text = match (operation, self.id()) {
            (Operation::Create, Some(id)) => format!("{verb} {link}/{id}"),
            _ if link.is_empty() => format!("{verb} /"),
            _ => format!("{verb} {link}"),
        };
        if let Some(key) = self.header(PARTITION_KEY) {
            text.push_str(" PARTITION ");
            text.push_str(key);
        }
        if let Some(body) = self.body.filter(|body| operation != Operation::Create || !only_an_id(body)) {
            text.push(' ');
            text.push_str(&body.to_string());
        }
        text
    }

    /// Counts what the reply holds: the documents of a query page or a list,
    /// the one resource read, or the one written.
    fn count(&self, recording: &mut Recording, reply: &Reply) {
        match self.operation() {
            Operation::Query | Operation::List => recording.rows(returned(&reply.body)),
            Operation::Read => recording.row(),
            Operation::Create | Operation::Replace | Operation::Delete => recording.affected(1),
            Operation::Plan | Operation::Other => {}
        }
    }

    /// The query's text with its parameters written in.
    fn sql(&self) -> String {
        let Some(body) = self.body else { return String::new() };
        match body.get("query").and_then(Value::as_str) {
            Some(query) => {
                let parameters = body.get("parameters").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
                written_in(query, parameters)
            }
            None => body.to_string(),
        }
    }

    fn header(&self, name: &str) -> Option<&'a str> {
        self.headers.iter().find(|(header, _)| header.eq_ignore_ascii_case(name)).map(|(_, value)| *value)
    }

    /// The id of the resource the request sends.
    fn id(&self) -> Option<&'a str> {
        self.body?.get("id")?.as_str()
    }
}

/// A body that only names the resource, such as a new database's.
fn only_an_id(body: &Value) -> bool {
    body.as_object().is_some_and(|fields| fields.keys().all(|key| key == "id"))
}

/// The documents or resources a page holds: its `_count`, which every feed
/// and query reply carries, or else its `Documents`.
fn returned(body: &Value) -> u64 {
    body.get("_count").and_then(Value::as_u64).unwrap_or_else(|| {
        body.get("Documents").and_then(Value::as_array).map_or(0, |documents| documents.len() as u64)
    })
}

/// `sql` with each `@name` outside string literals replaced by its value from
/// `parameters` (`[{"name": "@name", "value": …}]`). A JSON value is already a
/// Cosmos DB SQL literal: `"text"`, `5`, `true`, `null`, `[1]`, `{"a": 1}`.
fn written_in(sql: &str, parameters: &[Value]) -> String {
    let value_of = |name: &str| {
        parameters
            .iter()
            .find(|parameter| parameter.get("name").and_then(Value::as_str) == Some(name))
            .map(|parameter| parameter.get("value").cloned().unwrap_or(Value::Null).to_string())
    };
    let mut written = String::with_capacity(sql.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = sql.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            written.push(ch);
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                written.push(ch);
            }
            '@' => {
                let mut end = start + 1;
                while let Some(&(at, next)) = chars.peek() {
                    if !(next.is_alphanumeric() || next == '_') {
                        break;
                    }
                    end = at + next.len_utf8();
                    chars.next();
                }
                let name = &sql[start..end];
                written.push_str(&value_of(name).unwrap_or_else(|| name.to_string()));
            }
            _ => written.push(ch),
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(method: Method, target: &Target, headers: &[(&str, &str)], body: Option<&Value>) -> String {
        Request::new(&method, target, headers, body).text()
    }

    const QUERY: [(&str, &str); 1] = [("x-ms-documentdb-isquery", "true")];

    #[test]
    fn queries_have_their_parameters_written_in() {
        let body = json!({
            "query": "SELECT * FROM c WHERE c.status = @status AND c.total > @min AND c.id = @id \
                      AND c.tag = '@status' AND c[\"@min\"] = @missing AND c.x = @status_2",
            "parameters": [
                { "name": "@status", "value": "it's \"paid\"" },
                { "name": "@min", "value": 10.5 },
                { "name": "@id", "value": null },
                { "name": "@status_2", "value": ["a", { "b": true }] },
            ],
        });
        let target = Target::items("shop", "orders");
        assert_eq!(
            text(Method::POST, &target, &QUERY, Some(&body)),
            r#"SELECT * FROM c WHERE c.status = "it's \"paid\"" AND c.total > 10.5 AND c.id = null AND c.tag = '@status' AND c["@min"] = @missing AND c.x = ["a",{"b":true}]"#
        );
        let plain = json!({ "query": "SELECT VALUE COUNT(1) FROM c", "parameters": [] });
        assert_eq!(text(Method::POST, &target, &QUERY, Some(&plain)), "SELECT VALUE COUNT(1) FROM c");
        let plan = [QUERY[0], (PLAN_REQUEST, "True")];
        assert_eq!(text(Method::POST, &target, &plan, Some(&plain)), "EXPLAIN SELECT VALUE COUNT(1) FROM c");
    }

    #[test]
    fn other_requests_name_the_operation_and_the_resource() {
        let key = [(PARTITION_KEY, r#"["eu"]"#), ("if-match", "\"etag\"")];
        let item = json!({ "id": "42", "total": 5 });
        assert_eq!(text(Method::GET, &Target::account(), &[], None), "READ /");
        assert_eq!(text(Method::GET, &Target::container("shop", "orders"), &[], None), "READ dbs/shop/colls/orders");
        assert_eq!(text(Method::GET, &Target::databases(), &[], None), "LIST dbs");
        assert_eq!(text(Method::GET, &Target::containers("shop"), &[], None), "LIST dbs/shop/colls");
        assert_eq!(text(Method::GET, &Target::offers(), &[], None), "LIST offers");
        assert_eq!(
            text(Method::GET, &Target::partition_key_ranges("shop", "orders"), &[], None),
            "LIST dbs/shop/colls/orders/pkranges"
        );
        assert_eq!(text(Method::POST, &Target::databases(), &[], Some(&json!({ "id": "shop" }))), "CREATE dbs/shop");
        let container = json!({ "id": "orders", "partitionKey": { "paths": ["/region"] } });
        assert_eq!(
            text(Method::POST, &Target::containers("shop"), &[], Some(&container)),
            r#"CREATE dbs/shop/colls/orders {"id":"orders","partitionKey":{"paths":["/region"]}}"#
        );
        assert_eq!(
            text(Method::POST, &Target::items("shop", "orders"), &key[..1], Some(&item)),
            r#"CREATE dbs/shop/colls/orders/docs/42 PARTITION ["eu"] {"id":"42","total":5}"#
        );
        assert_eq!(
            text(Method::PUT, &Target::item("shop", "orders", "42"), &key, Some(&item)),
            r#"REPLACE dbs/shop/colls/orders/docs/42 PARTITION ["eu"] {"id":"42","total":5}"#
        );
        assert_eq!(
            text(Method::DELETE, &Target::item("shop", "orders", "42"), &key, None),
            r#"DELETE dbs/shop/colls/orders/docs/42 PARTITION ["eu"]"#
        );
        assert_eq!(text(Method::DELETE, &Target::database("shop"), &[], None), "DELETE dbs/shop");
        assert_eq!(text(Method::PATCH, &Target::item("shop", "o", "1"), &[], None), "PATCH dbs/shop/colls/o/docs/1");
    }

    #[test]
    fn requests_are_on_the_database_they_name() {
        let database = |method: Method, target: &Target, body: Option<&Value>| {
            Request::new(&method, target, &[], body).database().map(String::from)
        };
        assert_eq!(database(Method::GET, &Target::item("shop", "orders", "42"), None).as_deref(), Some("shop"));
        assert_eq!(database(Method::GET, &Target::containers("shop"), None).as_deref(), Some("shop"));
        assert_eq!(database(Method::GET, &Target::databases(), None), None);
        assert_eq!(database(Method::GET, &Target::account(), None), None);
        let new = json!({ "id": "fresh" });
        assert_eq!(database(Method::POST, &Target::databases(), Some(&new)).as_deref(), Some("fresh"));
    }

    #[test]
    fn counts_documents_returned_or_written() {
        assert_eq!(returned(&json!({ "Documents": [{}, {}], "_count": 2 })), 2);
        assert_eq!(returned(&json!({ "Documents": [{}] })), 1);
        assert_eq!(returned(&Value::Null), 0);
    }
}
