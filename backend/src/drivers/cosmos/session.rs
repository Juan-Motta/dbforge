//! The query console: Cosmos DB SQL, one query per statement, separated by `;`.
//! A query reads one container, and in Cosmos DB SQL the name after FROM is
//! only an alias for it, so the console takes that name as the container:
//! `SELECT * FROM orders o WHERE o.total > 100` reads `orders` as written.

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::Notify;

use super::client::{Client, Target};
use super::query::{Feed, documents, split_top_level};
use super::{containers, row, shared_type, text, typed};
use crate::connections::model::Connection;
use crate::drivers::{Column, QueryOutcome, StatementResult};
use crate::error::{AppError, Result};

pub struct Session {
    client: Client,
    database: String,
    /// Wakes a running query to stop reading it.
    cancel: Arc<Notify>,
}

/// A console on `database`, or on the connection's own.
pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    let database = console_database(connection, database)?;
    let client = Client::new(connection, password)?;
    // Proves the database is there before the first query.
    client.get(&Target::database(&database), &[]).await?;
    Ok(Session { client, database, cancel: Arc::new(Notify::new()) })
}

impl Session {
    pub fn canceller(&self) -> Arc<Notify> {
        self.cancel.clone()
    }

    /// Runs every query in order. Each query's container is found first, so a
    /// wrong name anywhere means nothing runs. Only `row_limit` items are read.
    pub async fn run(&mut self, script: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        // Registered before the queries start, so a cancel can't slip in between.
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();

        let queries = split_top_level(script, ';');
        if queries.is_empty() {
            return Err(AppError::Invalid("Nothing to run.".into()));
        }
        let run = async {
            let mut targets = Vec::new();
            for query in &queries {
                targets.push(container_for(&self.client, &self.database, query).await?);
            }
            let mut results = Vec::new();
            for (query, container) in queries.iter().zip(&targets) {
                results.push(read(&self.client, &self.database, container, query, row_limit).await?);
            }
            Ok::<_, AppError>(results)
        };
        let results = tokio::select! {
            results = run => results?,
            () = &mut cancelled => return Err(AppError::Invalid("Query cancelled.".into())),
        };
        Ok(QueryOutcome { results, elapsed_ms: started.elapsed().as_millis() as u64 })
    }

    /// Every request opens its own connection, so a session never goes stale.
    pub async fn is_alive(&mut self) -> bool {
        true
    }
}

/// The console's database: the one picked, or the connection's own.
pub(super) fn console_database(connection: &Connection, database: Option<&str>) -> Result<String> {
    database
        .filter(|d| !d.is_empty())
        .or(Some(connection.database.as_str()).filter(|d| !d.is_empty()))
        .map(String::from)
        .ok_or_else(|| AppError::Invalid("Select a database for the console.".into()))
}

/// The container `query` names after FROM, once it is known to exist.
pub(super) async fn container_for(client: &Client, database: &str, query: &str) -> Result<String> {
    let container = container_of(query).ok_or_else(|| {
        AppError::Invalid("Name the container after FROM, like SELECT * FROM orders o WHERE o.total > 100.".into())
    })?;
    if !containers(client, database).await?.contains(&container) {
        return Err(AppError::Invalid(format!(
            "There's no container {container} in {database}. The name after FROM is the container to read."
        )));
    }
    Ok(container)
}

/// Reads the query's items up to `row_limit`; the rest are never fetched.
async fn read(
    client: &Client,
    database: &str,
    container: &str,
    query: &str,
    row_limit: usize,
) -> Result<StatementResult> {
    let mut feed = Feed::new(client, database, container, query, &[]);
    let mut items = Vec::new();
    let mut truncated = false;
    while let Some(reply) = feed.next().await? {
        items.extend(documents(&reply).iter().cloned());
        if items.len() > row_limit {
            items.truncate(row_limit);
            truncated = true;
            break;
        }
    }
    Ok(result_of(&items, truncated))
}

/// Objects show their properties as columns, every one the query returned;
/// anything else (`SELECT VALUE …`, or nothing) is one `value` column.
fn result_of(items: &[Value], truncated: bool) -> StatementResult {
    // No items still shows a grid, empty, rather than a statement that returns none.
    let objects = !items.is_empty() && items.iter().all(Value::is_object);
    let columns = if objects {
        // In the order they first appear, the service's own included: the query asked for them.
        let mut names: Vec<&String> = Vec::new();
        for name in items.iter().filter_map(Value::as_object).flat_map(|item| item.keys()) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let columns = names.into_iter().map(|name| column(name, String::new())).collect();
        typed(columns, items)
    } else {
        vec![column("value", shared_type(items.iter()))]
    };
    let rows: Vec<_> = if objects {
        items.iter().map(|item| row(item, &columns)).collect()
    } else {
        items.iter().map(|item| vec![text(item)]).collect()
    };
    StatementResult { rows_affected: rows.len() as u64, columns, rows, truncated }
}

fn column(name: &str, data_type: String) -> Column {
    Column { name: name.into(), data_type, primary_key: false, generated: false, default: None }
}

/// The container a query reads: the name after its first FROM outside
/// quotes and brackets. `FROM x IN c.list` iterates a property, not a container.
pub(super) fn container_of(query: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let chars: Vec<char> = query.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if let Some(q) = quote {
            if ch == '\\' {
                index += 1;
            } else if ch == q {
                quote = None;
            }
        } else {
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ if depth == 0 && word_at(&chars, index).eq_ignore_ascii_case("from") => {
                    let mut words = chars[index + 4..].iter().collect::<String>();
                    words = words.trim_start().to_string();
                    let name: String = words.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    let rest = words[name.len()..].trim_start();
                    let iterates = rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("in ");
                    return (!name.is_empty() && !iterates).then_some(name);
                }
                _ => {}
            }
        }
        index += 1;
    }
    None
}

/// The word starting at `index`, when a word starts there.
fn word_at(chars: &[char], index: usize) -> String {
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    if index > 0 && is_word(&chars[index - 1]) {
        return String::new();
    }
    chars[index..].iter().take_while(|c| is_word(c)).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{KEY, database};
    use super::*;

    #[test]
    fn finds_the_container_after_from() {
        assert_eq!(container_of("SELECT * FROM orders o WHERE o.total > 1").as_deref(), Some("orders"));
        assert_eq!(container_of("select value count(1) from c").as_deref(), Some("c"));
        assert_eq!(
            container_of("SELECT 'from x' AS f, (SELECT VALUE 1 FROM t IN o.tags) FROM orders o").as_deref(),
            Some("orders")
        );
        assert_eq!(container_of("SELECT o.fromage FROM\n  big_orders").as_deref(), Some("big_orders"));
        assert_eq!(container_of("SELECT * FROM t IN c.tags"), None);
        assert_eq!(container_of("SELECT 1"), None);
    }

    #[test]
    fn results_show_objects_as_columns_and_values_as_one() {
        let objects = result_of(&[json!({"customer": "a", "n": 2}), json!({"customer": "b", "_ts": 5})], false);
        let names: Vec<_> = objects.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["customer", "n", "_ts"]);
        assert_eq!(objects.rows[1], [Some("b".into()), None, Some("5".into())]);

        let empty = result_of(&[], false);
        assert_eq!((empty.columns.len(), empty.rows.len()), (1, 0));

        let values = result_of(&[json!(3), json!({"a": 1})], true);
        assert_eq!(values.columns[0].name, "value");
        assert_eq!(values.columns[0].data_type, "mixed");
        assert_eq!(values.rows, [[Some("3".into())], [Some(r#"{"a":1}"#.into())]]);
        assert!(values.truncated);
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn runs_queries_on_the_container_after_from() {
        let items =
            (0..5).map(|i| json!({"id": i.to_string(), "customer": format!("c{}", i % 2), "total": i})).collect();
        let connection = database("dbforge_console", &[("orders", "/customer", items)]).await;
        let mut session = open_session(&connection, KEY, None).await.unwrap();

        let outcome = session
            .run("SELECT o.id, o.total FROM orders o WHERE o.total > 1 ORDER BY o.total DESC; SELECT VALUE COUNT(1) FROM orders", 2)
            .await
            .unwrap();
        let first = &outcome.results[0];
        assert_eq!(first.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "total"]);
        assert_eq!(first.rows, [[Some("4".into()), Some("4".into())], [Some("3".into()), Some("3".into())]]);
        assert!(first.truncated);
        assert_eq!(outcome.results[1].rows, [[Some("5".into())]]);

        let error = session.run("SELECT * FROM orders; SELECT * FROM nope", 10).await.unwrap_err();
        assert!(error.to_string().starts_with("There's no container nope in dbforge_console"), "{error}");
        let error = session.run("SELECT * FROM orders o WHERE o.nope(", 10).await.unwrap_err();
        assert!(error.to_string().contains("Syntax error"), "{error}");

        let missing = open_session(&connection, KEY, Some("dbforge_nope")).await.err().unwrap();
        assert!(missing.to_string().contains("not found"), "{missing}");
    }

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn cancel_stops_the_running_query_and_the_session_stays_usable() {
        let items = (0..3).map(|i| json!({"id": i.to_string(), "customer": "c"})).collect();
        let connection = database("dbforge_cancel", &[("orders", "/customer", items)]).await;
        let mut session = open_session(&connection, KEY, None).await.unwrap();
        let canceller = session.canceller();
        // Harmless while idle, and not remembered for the next run.
        canceller.notify_waiters();
        assert_eq!(session.run("SELECT * FROM orders", 10).await.unwrap().results[0].rows.len(), 3);

        let waker = canceller.clone();
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            waker.notify_waiters();
        });
        // Many queries, so the cancel lands while they run.
        let script = vec!["SELECT * FROM orders"; 200].join(";");
        let error = session.run(&script, 10).await.unwrap_err();
        cancel.await.unwrap();
        assert_eq!(error.to_string(), "Query cancelled.");
        assert!(session.is_alive().await);
        assert_eq!(session.run("SELECT VALUE COUNT(1) FROM orders", 10).await.unwrap().results[0].rows.len(), 1);
    }
}
