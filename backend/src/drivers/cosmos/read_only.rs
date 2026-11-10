//! One Cosmos DB SQL query, for callers that must not change data.
//!
//! The service enforces it: its SQL is a query language only, sent to the
//! query endpoint, which reads and never writes. Writes go through other
//! endpoints this never calls. The query runs on the given container, as the
//! console runs it, and only one query is taken at a time.

use super::open_session;
use super::query::split_top_level;
use crate::connections::model::Connection;
use crate::drivers::{QueryOutcome, with_timeout};
use crate::error::{AppError, Result};

/// Runs one query on `container` of `database` (the connection's own when `None`).
pub async fn run_read_only(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    container: Option<&str>,
    statement: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    if split_top_level(statement, ';').len() > 1 {
        return Err(AppError::Invalid("Read-only queries run one statement at a time.".into()));
    }
    let mut session = with_timeout(open_session(connection, password, database, container)).await?;
    session.run(statement, row_limit).await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests::{KEY, database};
    use super::*;

    #[tokio::test]
    #[ignore = "needs the Cosmos DB emulator on port 58081"]
    async fn read_only_runs_one_query_on_the_container() {
        let items = (0..3).map(|i| json!({"id": i.to_string(), "customer": "c", "total": i})).collect();
        let connection = database("dbforge_read_only", &[("orders", "/customer", items)]).await;
        let outcome = run_read_only(&connection, KEY, None, Some("orders"), "SELECT c.id FROM c ORDER BY c.total", 2)
            .await
            .unwrap();
        let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(result.rows, [[Some("0".into())], [Some("1".into())]]);
        assert!(result.truncated);

        let script = run_read_only(&connection, KEY, None, Some("orders"), "SELECT 1; SELECT 2", 10).await;
        assert_eq!(script.unwrap_err().to_string(), "Read-only queries run one statement at a time.");
        let unpicked = run_read_only(&connection, KEY, None, None, "SELECT * FROM c", 10).await;
        assert_eq!(unpicked.unwrap_err().to_string(), "Pick the container to query.");
        // Not a query: the service refuses it, and nothing changes.
        let write = run_read_only(&connection, KEY, None, Some("orders"), "DELETE FROM c", 10).await;
        assert!(write.is_err());
        let count = run_read_only(&connection, KEY, None, Some("orders"), "SELECT VALUE COUNT(1) FROM c", 10);
        assert_eq!(count.await.unwrap().results[0].rows, [[Some("3".into())]]);
    }
}
