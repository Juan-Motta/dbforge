//! Query consoles: a client that stays open and runs scripts read by
//! [`shell::parse`](super::shell::parse). `use name` switches the session's
//! database for later runs, as in mongosh.
//!
//! Every operation carries the session's tag as its `comment`, so [`cancel`]
//! can find it among the server's running operations and stop it with `killOp`.

use std::time::Instant;

use futures_util::TryStreamExt;
use mongodb::bson::oid::ObjectId;
use mongodb::bson::{Bson, Document, doc};
use mongodb::error::{CommandError, ErrorKind, WriteError, WriteFailure};
use mongodb::options::UpdateModifications;

use super::logged::{Documents, Find, LoggedClient, LoggedCollection, LoggedDatabase, Update, Write};
use super::shell::{self, Method, Statement};
use super::{columns_of, connect, default_database, row, text};
use crate::connections::model::Connection;
use crate::drivers::{QueryOutcome, StatementResult};
use crate::error::{AppError, Result};

/// The server's code for an operation stopped by `killOp`.
const INTERRUPTED: i32 = 11601;

pub struct Session {
    client: LoggedClient,
    database: String,
    tag: String,
}

/// mongosh's database when none is named.
const DEFAULT_DATABASE: &str = "test";

pub async fn open_session(connection: &Connection, password: Option<&str>, database: Option<&str>) -> Result<Session> {
    let client = connect(connection, password).await?;
    client.database("admin").run_command(doc! { "ping": 1 }).await?;
    let database = database
        .filter(|d| !d.is_empty())
        .map(String::from)
        .or_else(|| default_database(connection, &client))
        .unwrap_or_else(|| DEFAULT_DATABASE.into());
    Ok(Session { client, database, tag: format!("dbforge-console-{}", ObjectId::new().to_hex()) })
}

impl Session {
    pub fn tag(&self) -> String {
        self.tag.clone()
    }

    /// Runs every statement in order. The whole script is read first, so a
    /// typo anywhere means nothing runs.
    pub async fn run(&mut self, script: &str, row_limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let statements = shell::parse(script).map_err(AppError::Invalid)?;
        if statements.is_empty() {
            return Err(AppError::Invalid("Nothing to run.".into()));
        }
        let mut results = Vec::new();
        for statement in statements {
            results.push(self.execute(statement, row_limit).await?);
        }
        Ok(QueryOutcome { results, elapsed_ms: started.elapsed().as_millis() as u64 })
    }

    /// Runs one parsed statement.
    pub(super) async fn execute(&mut self, statement: Statement, row_limit: usize) -> Result<StatementResult> {
        match statement {
            Statement::Use(name) => {
                self.database = name;
                Ok(StatementResult::default())
            }
            Statement::Call { collection, method, chain } => {
                let database = self.client.database(&self.database).tagged(&self.tag);
                let call = Call { row_limit };
                let outcome = match collection {
                    Some(name) => call.collection(database.collection(&name), method, chain).await,
                    None => call.database(database, method, chain).await,
                };
                outcome.map_err(|error| match error {
                    AppError::MongoDb(error) if interrupted(&error) => AppError::Invalid("Query cancelled.".into()),
                    other => other,
                })
            }
        }
    }

    pub(super) async fn close(self) {
        self.client.shutdown().await;
    }

    pub async fn is_alive(&mut self) -> bool {
        self.client.database("admin").run_command(doc! { "ping": 1 }).await.is_ok()
    }
}

/// Stops the operations tagged `tag`. Harmless when there are none.
pub async fn cancel(connection: &Connection, password: Option<&str>, tag: &str) -> Result<()> {
    let client = connect(connection, password).await?;
    let admin = client.database("admin");
    let pipeline = [
        doc! { "$currentOp": {} },
        // A cursor's later batches (getMore) keep the comment of the find that opened it.
        doc! { "$match": { "$or": [{ "command.comment": tag }, { "cursor.originatingCommand.comment": tag }] } },
        doc! { "$project": { "opid": 1 } },
    ];
    let operations: Vec<Document> = admin.aggregate(pipeline).await?.try_collect().await?;
    for operation in operations {
        if let Some(id) = operation.get("opid") {
            admin.run_command(doc! { "killOp": 1, "op": id.clone() }).await?;
        }
    }
    client.shutdown().await;
    Ok(())
}

fn interrupted(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        ErrorKind::Command(CommandError { code, .. }) => *code == INTERRUPTED,
        ErrorKind::Write(WriteFailure::WriteError(WriteError { code, .. })) => *code == INTERRUPTED,
        _ => false,
    }
}

/// What every operation of one run shares.
struct Call {
    row_limit: usize,
}

impl Call {
    async fn collection(
        &self,
        collection: LoggedCollection,
        method: Method,
        chain: Vec<Method>,
    ) -> Result<StatementResult> {
        let name = method.name.as_str();
        if name != "find" && name != "aggregate" {
            no_chain(name, &chain)?;
        }
        let args = Args { method: name, values: &method.args };
        match name {
            "find" | "findOne" => {
                args.at_most(2)?;
                let mut find = Find::new(args.document(0)?);
                if let Some(projection) = args.optional_document(1)? {
                    find = find.projection(projection);
                }
                if name == "findOne" {
                    find = find.limit(1);
                }
                for modifier in chain {
                    let args = Args { method: &modifier.name, values: &modifier.args };
                    find = match modifier.name.as_str() {
                        "sort" => find.sort(args.required_document(0)?),
                        "projection" => find.projection(args.required_document(0)?),
                        "skip" => find.skip(args.count(0)?.unsigned_abs()),
                        // A negative limit means the same in mongosh: at most that many.
                        "limit" => find.limit(args.count(0)?.abs()),
                        "toArray" | "pretty" => find,
                        other => return Err(unsupported_after(other, name)),
                    };
                }
                self.documents(collection.find(find).await?).await
            }
            "aggregate" => {
                args.at_most(1)?;
                let pipeline = args.stages()?;
                for modifier in &chain {
                    if !matches!(modifier.name.as_str(), "toArray" | "pretty") {
                        return Err(unsupported_after(&modifier.name, name));
                    }
                }
                self.documents(collection.aggregate(pipeline).await?).await
            }
            "countDocuments" | "count" => {
                args.at_most(1)?;
                Ok(count_result(collection.count_documents(args.document(0)?).await?))
            }
            "estimatedDocumentCount" => {
                args.at_most(0)?;
                Ok(count_result(collection.estimated_document_count().await?))
            }
            "distinct" => {
                args.at_most(2)?;
                let Some(Bson::String(field)) = args.values.first() else {
                    return Err(AppError::Invalid("distinct takes a field name.".into()));
                };
                let values = collection.distinct(field, args.document(1)?).await?;
                let documents = values.into_iter().map(|value| doc! { field.as_str(): value }).collect();
                Ok(self.rows(documents, false))
            }
            "insertOne" => {
                args.at_most(1)?;
                self.write(&collection, Write::InsertOne(args.required_document(0)?)).await
            }
            "insertMany" => {
                args.at_most(1)?;
                let Some(Bson::Array(items)) = args.values.first() else {
                    return Err(AppError::Invalid("insertMany takes an array of documents.".into()));
                };
                let documents = items
                    .iter()
                    .map(|item| match item {
                        Bson::Document(document) => Ok(document.clone()),
                        _ => Err(AppError::Invalid("insertMany takes an array of documents.".into())),
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.write(&collection, Write::InsertMany(documents)).await
            }
            "updateOne" | "updateMany" | "replaceOne" => {
                args.at_most(3)?;
                let filter = args.required_document(0)?;
                let upsert = upsert(&args)?;
                let write = if name == "replaceOne" {
                    Write::ReplaceOne { filter, replacement: args.required_document(1)?, upsert }
                } else {
                    let update = match args.values.get(1) {
                        Some(Bson::Document(update)) => UpdateModifications::Document(update.clone()),
                        Some(Bson::Array(stages)) => UpdateModifications::Pipeline(
                            stages.iter().filter_map(|s| s.as_document().cloned()).collect(),
                        ),
                        _ => return Err(AppError::Invalid(format!("{name} takes a filter and an update."))),
                    };
                    let update = Update { filter, update, upsert };
                    if name == "updateOne" { Write::UpdateOne(update) } else { Write::UpdateMany(update) }
                };
                self.write(&collection, write).await
            }
            "deleteOne" | "deleteMany" => {
                args.at_most(1)?;
                let filter = args.required_document(0)?;
                let write = if name == "deleteOne" { Write::DeleteOne(filter) } else { Write::DeleteMany(filter) };
                self.write(&collection, write).await
            }
            other => Err(AppError::Invalid(format!(
                "`{other}` isn't supported. Use find, findOne, aggregate, countDocuments, estimatedDocumentCount, \
                 distinct, insertOne, insertMany, updateOne, updateMany, replaceOne, deleteOne or deleteMany."
            ))),
        }
    }

    async fn database(&self, database: LoggedDatabase, method: Method, chain: Vec<Method>) -> Result<StatementResult> {
        no_chain(&method.name, &chain)?;
        let args = Args { method: &method.name, values: &method.args };
        match method.name.as_str() {
            "runCommand" => {
                args.at_most(1)?;
                let reply = database.run_command(args.required_document(0)?).await?;
                Ok(self.rows(vec![reply], false))
            }
            "getCollectionNames" => {
                args.at_most(0)?;
                let names = database.list_collection_names().await?;
                Ok(self.rows(names.into_iter().map(|name| doc! { "name": name }).collect(), false))
            }
            other => Err(AppError::Invalid(format!(
                "`db.{other}()` isn't supported. Use db.<collection>.<method>(…), db.runCommand(…) or \
                 db.getCollectionNames()."
            ))),
        }
    }

    /// Writes and reports the documents it changed.
    async fn write(&self, collection: &LoggedCollection, write: Write) -> Result<StatementResult> {
        Ok(affected(collection.write(&write).await?.affected))
    }

    /// Reads the cursor until the row limit; the rest is dropped.
    async fn documents(&self, mut cursor: Documents) -> Result<StatementResult> {
        let mut documents = Vec::new();
        while let Some(document) = cursor.try_next().await? {
            if documents.len() == self.row_limit {
                return Ok(self.rows(documents, true));
            }
            documents.push(document);
        }
        Ok(self.rows(documents, false))
    }

    fn rows(&self, documents: Vec<Document>, truncated: bool) -> StatementResult {
        let mut columns = columns_of(&documents);
        // Results such as `$group` stages without `_id`, or `distinct`, have no ids to show.
        if !documents.is_empty() && !documents.iter().any(|d| d.contains_key("_id")) {
            columns.retain(|column| column.name != "_id");
        }
        let rows: Vec<_> = documents.iter().map(|document| row(document, &columns)).collect();
        StatementResult { rows_affected: rows.len() as u64, columns, rows, truncated }
    }
}

fn count_result(count: u64) -> StatementResult {
    let column = crate::drivers::Column {
        name: "count".into(),
        data_type: "long".into(),
        primary_key: false,
        generated: false,
        default: None,
    };
    StatementResult {
        columns: vec![column],
        rows: vec![vec![text(&Bson::Int64(count as i64))]],
        truncated: false,
        rows_affected: 1,
    }
}

fn affected(rows: u64) -> StatementResult {
    StatementResult { rows_affected: rows, ..StatementResult::default() }
}

fn no_chain(method: &str, chain: &[Method]) -> Result<()> {
    match chain.first() {
        Some(next) => Err(unsupported_after(&next.name, method)),
        None => Ok(()),
    }
}

pub(super) fn unsupported_after(modifier: &str, method: &str) -> AppError {
    AppError::Invalid(format!("`.{modifier}()` isn't supported after {method}()."))
}

/// `{upsert: true}`, the only update option read; any other is refused rather
/// than ignored.
pub(super) fn upsert(args: &Args) -> Result<bool> {
    let Some(options) = args.optional_document(2)? else { return Ok(false) };
    let mut upsert = false;
    for (key, value) in options {
        match (key.as_str(), value) {
            ("upsert", Bson::Boolean(on)) => upsert = on,
            (other, _) => {
                return Err(AppError::Invalid(format!("Option `{other}` of {} isn't supported.", args.method)));
            }
        }
    }
    Ok(upsert)
}

/// A method's arguments, read with errors that name the method.
pub(super) struct Args<'a> {
    pub method: &'a str,
    pub values: &'a [Bson],
}

impl Args<'_> {
    pub fn at_most(&self, count: usize) -> Result<()> {
        if self.values.len() > count {
            return Err(AppError::Invalid(format!(
                "{} takes at most {count} argument{}.",
                self.method,
                if count == 1 { "" } else { "s" }
            )));
        }
        Ok(())
    }

    pub fn optional_document(&self, index: usize) -> Result<Option<Document>> {
        match self.values.get(index) {
            None => Ok(None),
            Some(Bson::Document(document)) => Ok(Some(document.clone())),
            Some(_) => Err(AppError::Invalid(format!("Argument {} of {} must be a document.", index + 1, self.method))),
        }
    }

    /// A document, `{}` when it is left out.
    pub fn document(&self, index: usize) -> Result<Document> {
        Ok(self.optional_document(index)?.unwrap_or_default())
    }

    pub fn required_document(&self, index: usize) -> Result<Document> {
        self.optional_document(index)?
            .ok_or_else(|| AppError::Invalid(format!("{} takes a document as argument {}.", self.method, index + 1)))
    }

    /// The pipeline of `aggregate([...])`.
    pub fn stages(&self) -> Result<Vec<Document>> {
        let Some(Bson::Array(stages)) = self.values.first() else {
            return Err(AppError::Invalid(format!("{} takes an array of stages.", self.method)));
        };
        stages
            .iter()
            .map(|stage| match stage {
                Bson::Document(stage) => Ok(stage.clone()),
                _ => Err(AppError::Invalid("Each aggregate stage must be a document.".into())),
            })
            .collect()
    }

    pub fn count(&self, index: usize) -> Result<i64> {
        match self.values.get(index) {
            Some(Bson::Int32(n)) => Ok(i64::from(*n)),
            Some(Bson::Int64(n)) => Ok(*n),
            Some(Bson::Double(n)) if n.fract() == 0.0 => Ok(*n as i64),
            _ => Err(AppError::Invalid(format!("{} takes a whole number.", self.method))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::tests::{PASSWORD, database};
    use super::*;

    fn cells(result: &StatementResult) -> Vec<Vec<Option<&str>>> {
        result.rows.iter().map(|row| row.iter().map(Option::as_deref).collect()).collect()
    }

    fn names(result: &StatementResult) -> Vec<&str> {
        result.columns.iter().map(|c| c.name.as_str()).collect()
    }

    async fn shop(name: &str) -> Connection {
        let orders = (1..=5)
            .map(|id| doc! { "_id": id, "customer": if id % 2 == 0 { "ann" } else { "bob" }, "total": id * 50 })
            .collect();
        database(name, &[("orders", orders)]).await
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn runs_queries_in_mongosh_syntax() {
        let connection = shop("dbforge_console_read").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let outcome = session
            .run(
                "db.orders.find({total: {$gt: 100}}, {total: 1}).sort({total: -1}).limit(2)
                 db.orders.countDocuments({customer: 'bob'})
                 db.orders.distinct('customer')
                 db.orders.aggregate([{$group: {_id: '$customer', sum: {$sum: '$total'}}}, {$sort: {_id: 1}}])
                 db.orders.aggregate([{$project: {_id: 0, total: 1}}, {$limit: 1}])",
                100,
            )
            .await
            .unwrap();
        let [find, count, distinct, group, project] = outcome.results.as_slice() else { panic!("{outcome:?}") };
        assert_eq!(names(find), ["_id", "total"]);
        assert_eq!(cells(find), [[Some("5"), Some("250")], [Some("4"), Some("200")]]);
        assert_eq!(cells(count), [[Some("3")]]);
        assert_eq!((names(distinct), cells(distinct)), (vec!["customer"], vec![vec![Some("ann")], vec![Some("bob")]]));
        assert_eq!(cells(group), [[Some("ann"), Some("300")], [Some("bob"), Some("450")]]);
        // Without `_id` in the results, no empty `_id` column is shown.
        assert_eq!(names(project), ["total"]);

        let limited = session.run("db.orders.find()", 2).await.unwrap();
        assert_eq!((limited.results[0].rows.len(), limited.results[0].truncated), (2, true));
        let empty = session.run("db.orders.find({total: -1})", 2).await.unwrap();
        assert_eq!((names(&empty.results[0]), empty.results[0].rows.len()), (vec!["_id"], 0));
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn records_each_statement_as_mongosh_runs_it() {
        use crate::statement_log::store::tests::Collected;
        use crate::statement_log::{self, Origin, StatementLog};

        let connection = shop("dbforge_console_log").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let log = StatementLog::new(50, Collected::default());
        let script = "db.orders.find({customer: 'bob'}).sort({_id: -1}).limit(2)
                      db.orders.countDocuments()
                      db.orders.updateMany({customer: 'ann'}, {$inc: {total: 1}})
                      db.runCommand({ping: 1})
                      db.orders.insertOne({_id: 1})";

        let error = statement_log::scoped(&log, "mongo", Origin::Console, session.run(script, 1)).await.unwrap_err();
        assert!(error.to_string().contains("duplicate key"), "{error}");

        let entries = log.entries();
        let recorded: Vec<_> = entries.iter().map(|e| (e.text.as_str(), e.rows, e.error.is_some())).collect();
        assert_eq!(
            recorded,
            [
                // One document shown, and the one more read to tell the result was cut.
                (r#"db.getCollection("orders").find({"customer": "bob"}).sort({"_id": -1}).limit(2)"#, Some(2), false),
                (r#"db.getCollection("orders").countDocuments({})"#, Some(1), false),
                (
                    r#"db.getCollection("orders").updateMany({"customer": "ann"}, {"$inc": {"total": 1}})"#,
                    Some(2),
                    false
                ),
                // The session's tag goes along as the command's comment, but isn't shown.
                (r#"db.runCommand({"ping": 1})"#, Some(1), false),
                (r#"db.getCollection("orders").insertOne({"_id": 1})"#, None, true),
            ]
        );
        assert!(entries.iter().all(|e| e.database.as_deref() == Some("dbforge_console_log")));
        assert!(entries[4].error.as_deref().unwrap().contains("duplicate key"));
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn writes_report_the_documents_they_changed() {
        let connection = shop("dbforge_console_write").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let outcome = session
            .run(
                "db.orders.insertOne({_id: 6, customer: 'cy', total: 10});
                 db.orders.insertMany([{_id: 7}, {_id: 8}]);
                 db.orders.updateMany({customer: 'bob'}, {$inc: {total: 1}});
                 db.orders.updateOne({_id: 99}, {$set: {total: 1}}, {upsert: true});
                 db.orders.replaceOne({_id: 7}, {customer: 'dee'});
                 db.orders.deleteMany({total: {$lt: 60}});",
                100,
            )
            .await
            .unwrap();
        let affected: Vec<u64> = outcome.results.iter().map(|r| r.rows_affected).collect();
        // Deleted: 6 (10), 99 (1) and 1 (51); 7 and 8 have no total, so $lt skips them.
        assert_eq!(affected, [1, 2, 3, 1, 1, 3]);
        assert!(outcome.results.iter().all(|r| r.columns.is_empty()));

        let error = session.run("db.orders.updateOne({_id: 2}, {$set: {a: 1}}, {multi: true})", 100).await.unwrap_err();
        assert_eq!(error.to_string(), "Option `multi` of updateOne isn't supported.");
        let error = session.run("db.orders.insertOne({_id: 2})", 100).await.unwrap_err();
        assert!(error.to_string().contains("duplicate key"), "{error}");
        // A typo anywhere means nothing runs.
        let error = session.run("db.orders.deleteMany({})\ndb.orders.find({", 100).await.unwrap_err();
        assert!(error.to_string().starts_with("Expected a field name (line 2"), "{error}");
        let count = session.run("db.orders.countDocuments()", 100).await.unwrap();
        assert_eq!(cells(&count.results[0]), [[Some("6")]]);
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn use_switches_the_database_for_later_runs() {
        shop("dbforge_console_use").await;
        let connection = super::super::tests::local();
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        session.run("use dbforge_console_use", 100).await.unwrap();
        let outcome = session.run("db.orders.estimatedDocumentCount()\ndb.getCollectionNames()", 100).await.unwrap();
        assert_eq!(cells(&outcome.results[0]), [[Some("5")]]);
        assert_eq!(cells(&outcome.results[1]), [[Some("orders")]]);
        let reply = session.run("db.runCommand({ping: 1})", 100).await.unwrap();
        assert_eq!(names(&reply.results[0]), ["ok"]);
    }

    #[tokio::test]
    #[ignore = "needs a local MongoDB on port 57017"]
    async fn cancel_stops_the_running_operation() {
        let connection = shop("dbforge_console_cancel").await;
        let mut session = open_session(&connection, PASSWORD, None).await.unwrap();
        let tag = session.tag();
        let canceller = {
            let connection = connection.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(700)).await;
                cancel(&connection, PASSWORD, &tag).await
            })
        };
        let started = Instant::now();
        let error = session.run("db.orders.find({$where: 'sleep(2000) || true'})", 100).await.unwrap_err();
        assert_eq!(error.to_string(), "Query cancelled.");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        canceller.await.unwrap().unwrap();
        // The session keeps working.
        assert_eq!(session.run("db.orders.countDocuments()", 100).await.unwrap().results.len(), 1);
    }
}
