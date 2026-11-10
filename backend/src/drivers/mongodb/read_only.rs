//! One mongosh statement that may only read, for callers that must not change data.
//!
//! MongoDB has no read-only session, so the statement is parsed with the
//! console's parser and only operations that can't write are sent:
//!
//! - on a collection: `find` and `findOne` (with the console's `sort`,
//!   `projection`, `skip` and `limit`), `aggregate`, `countDocuments`, `count`,
//!   `estimatedDocumentCount` and `distinct`;
//! - on the database: `getCollectionNames`, and `runCommand` for the commands
//!   in [`READ_COMMANDS`] only.
//!
//! An aggregation writes only through `$out` and `$merge`. The server takes
//! them only as the last stage of the top-level pipeline, but the whole
//! pipeline is searched, nested `$lookup`, `$facet` and `$unionWith` pipelines
//! included, so no future placement slips through. Anything else, an unknown
//! method included, is refused before anything is sent.
//!
//! Not prevented: server-side JavaScript in `$where`, `$function` or
//! `$accumulator` runs, but it can't write.

use std::time::Instant;

use mongodb::bson::{Bson, Document};

use super::shell::{self, Method, Statement};
use super::{open_session, with_timeout};
use crate::connections::model::Connection;
use crate::drivers::QueryOutcome;
use crate::error::{AppError, Result};

/// Collection methods that only read.
const READ_METHODS: [&str; 7] =
    ["find", "findOne", "aggregate", "countDocuments", "count", "estimatedDocumentCount", "distinct"];

/// Commands `db.runCommand` may run: each only reads. `find` and `aggregate`
/// are searched for write stages like `aggregate()`.
const READ_COMMANDS: [&str; 15] = [
    "aggregate",
    "buildInfo",
    "buildinfo",
    "collStats",
    "count",
    "dbStats",
    "distinct",
    "find",
    "hello",
    "isMaster",
    "ismaster",
    "listCollections",
    "listDatabases",
    "listIndexes",
    "ping",
];

/// The aggregation stages that write.
const WRITE_STAGES: [&str; 2] = ["$out", "$merge"];

/// Checks `statement`, then runs it on a new client that is closed afterwards.
pub async fn run_read_only(
    connection: &Connection,
    password: Option<&str>,
    database: Option<&str>,
    statement: &str,
    row_limit: usize,
) -> Result<QueryOutcome> {
    let statement = read_statement(statement)?;
    let started = Instant::now();
    let mut session = with_timeout(open_session(connection, password, database)).await?;
    let result = session.execute(statement, row_limit).await;
    session.close().await;
    Ok(QueryOutcome { results: vec![result?], elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// The one statement in `text`, once it is known to only read.
fn read_statement(text: &str) -> Result<Statement> {
    let mut statements = shell::parse(text).map_err(AppError::Invalid)?;
    let statement = match statements.len() {
        1 => statements.remove(0),
        0 => return Err(AppError::Invalid("Nothing to run.".into())),
        _ => return Err(AppError::Invalid("Read-only queries run one statement at a time.".into())),
    };
    match &statement {
        Statement::Use(_) => {
            return Err(AppError::Invalid(
                "`use` isn't available in a read-only query: name the database instead.".into(),
            ));
        }
        Statement::Call { collection: Some(_), method, .. } => check_collection_method(method)?,
        Statement::Call { collection: None, method, .. } => check_database_method(method)?,
    }
    Ok(statement)
}

fn check_collection_method(method: &Method) -> Result<()> {
    let name = method.name.as_str();
    if !READ_METHODS.contains(&name) {
        return Err(AppError::Invalid(format!(
            "`{name}` isn't allowed in a read-only query. Use {}.",
            READ_METHODS.join(", ")
        )));
    }
    if name == "aggregate" {
        no_write_stage(&method.args)?;
    }
    Ok(())
}

fn check_database_method(method: &Method) -> Result<()> {
    match method.name.as_str() {
        "getCollectionNames" => Ok(()),
        "runCommand" => {
            let Some(Bson::Document(command)) = method.args.first() else {
                return Err(AppError::Invalid("runCommand takes a command document.".into()));
            };
            // The server reads the command's name from its first field.
            let name = command.keys().next().map(String::as_str).unwrap_or_default();
            if !READ_COMMANDS.contains(&name) {
                return Err(AppError::Invalid(format!(
                    "The `{name}` command isn't allowed in a read-only query. Allowed commands: {}.",
                    READ_COMMANDS.join(", ")
                )));
            }
            no_write_stage(&method.args)
        }
        other => Err(AppError::Invalid(format!(
            "`db.{other}()` isn't allowed in a read-only query. Use db.<collection>.<method>(…), \
             db.getCollectionNames() or db.runCommand(…) with a read command."
        ))),
    }
}

/// Refuses `$out` and `$merge` anywhere in `values`.
fn no_write_stage(values: &[Bson]) -> Result<()> {
    match values.iter().find_map(write_stage) {
        Some(stage) => Err(AppError::Invalid(format!("`{stage}` writes, so it isn't allowed in a read-only query."))),
        None => Ok(()),
    }
}

fn write_stage(value: &Bson) -> Option<&str> {
    match value {
        Bson::Document(document) => document_write_stage(document),
        Bson::Array(items) => items.iter().find_map(write_stage),
        _ => None,
    }
}

fn document_write_stage(document: &Document) -> Option<&str> {
    document.iter().find_map(
        |(key, value)| {
            if WRITE_STAGES.contains(&key.as_str()) { Some(key.as_str()) } else { write_stage(value) }
        },
    )
}

#[cfg(test)]
mod tests {
    use mongodb::bson::doc;

    use super::super::tests::{PASSWORD, database, local};
    use super::*;

    fn refused(text: &str) -> String {
        read_statement(text).err().map(|error| error.to_string()).unwrap_or_else(|| panic!("allowed: {text}"))
    }

    #[test]
    fn allows_only_reads() {
        for allowed in [
            "db.orders.find({total: {$gt: 1}}, {total: 1}).sort({total: -1}).skip(1).limit(2)",
            "db.orders.findOne()",
            "db.orders.aggregate([{$match: {a: 1}}, {$lookup: {from: 'x', pipeline: [{$match: {}}], as: 'y'}}])",
            "db.orders.countDocuments({})",
            "db.orders.count()",
            "db.orders.estimatedDocumentCount()",
            "db.orders.distinct('customer');",
            "db.getCollectionNames()",
            "db.runCommand({listIndexes: 'orders'})",
            "db.runCommand({aggregate: 'orders', pipeline: [], cursor: {}})",
        ] {
            assert!(read_statement(allowed).is_ok(), "{allowed}");
        }
        for write in [
            "db.orders.insertOne({a: 1})",
            "db.orders.insertMany([{a: 1}])",
            "db.orders.updateOne({}, {$set: {a: 1}})",
            "db.orders.updateMany({}, {$set: {a: 1}})",
            "db.orders.replaceOne({}, {a: 1})",
            "db.orders.deleteOne({})",
            "db.orders.deleteMany({})",
            "db.orders.drop()",
            "db.orders.findOneAndUpdate({}, {$set: {a: 1}})",
        ] {
            assert!(refused(write).contains("isn't allowed in a read-only query"), "{write}");
        }
        assert_eq!(
            refused("db.orders.aggregate([{$match: {}}, {$out: 'copy'}])"),
            "`$out` writes, so it isn't allowed in a read-only query."
        );
        assert!(refused("db.orders.aggregate([{$facet: {a: [{$merge: {into: 'x'}}]}}])").contains("$merge"));
        assert!(refused("db.runCommand({aggregate: 'orders', pipeline: [{$out: 'x'}], cursor: {}})").contains("$out"));
        assert_eq!(
            refused("db.runCommand({dropDatabase: 1})"),
            format!(
                "The `dropDatabase` command isn't allowed in a read-only query. Allowed commands: {}.",
                READ_COMMANDS.join(", ")
            )
        );
        for other in [
            "db.runCommand({delete: 'orders', deletes: [{q: {}, limit: 0}]})",
            "db.runCommand({insert: 'o', documents: [{}]})",
            "db.runCommand({})",
            "db.runCommand()",
        ] {
            assert!(read_statement(other).is_err(), "{other}");
        }
        assert!(refused("db.orders.find()\ndb.orders.deleteMany({})").contains("one statement at a time"));
        assert!(refused("use admin").contains("`use`"));
        assert!(refused("db.dropDatabase()").contains("isn't allowed"));
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn read_only_reads_and_sends_no_write() {
        let orders = (1..=3).map(|id| doc! { "_id": id, "total": id * 10 }).collect();
        let connection = database("dbforge_read_only", &[("orders", orders)]).await;
        let outcome = run_read_only(&connection, PASSWORD, None, "db.orders.find({}, {total: 1}).sort({_id: 1})", 2)
            .await
            .unwrap();
        let [result] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(result.rows, [[Some("1".into()), Some("10".into())], [Some("2".into()), Some("20".into())]]);
        assert!(result.truncated);

        // Another database than the connection's, by name.
        let other =
            run_read_only(&local(), PASSWORD, Some("dbforge_read_only"), "db.orders.countDocuments()", 10).await;
        assert_eq!(other.unwrap().results[0].rows, [[Some("3".into())]]);

        for attempt in [
            "db.orders.deleteMany({})",
            "db.orders.insertOne({_id: 9})",
            "db.orders.updateMany({}, {$set: {total: 0}})",
            "db.orders.aggregate([{$out: 'copied'}])",
            "db.runCommand({drop: 'orders'})",
            "db.orders.find()\ndb.orders.deleteMany({})",
        ] {
            assert!(run_read_only(&connection, PASSWORD, None, attempt, 100).await.is_err(), "ran: {attempt}");
        }
        let sum = "db.orders.aggregate([{$group: {_id: 'all', n: {$sum: '$total'}}}])";
        let total = run_read_only(&connection, PASSWORD, None, sum, 10).await.unwrap();
        assert_eq!(total.results[0].rows, [[Some("all".into()), Some("60".into())]]);
        let names = run_read_only(&connection, PASSWORD, None, "db.getCollectionNames()", 10).await.unwrap();
        assert_eq!(names.results[0].rows, [[Some("orders".into())]]);
    }
}
