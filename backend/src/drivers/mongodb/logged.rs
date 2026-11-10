//! A MongoDB client whose every operation is recorded in the statement log,
//! written as the mongosh call that does the same, such as
//! `db.getCollection("orders").find({"status": "paid"}).sort({"_id": 1}).limit(51)`.
//!
//! The driver holds a [`LoggedClient`] in place of the official driver's
//! client, which it can't reach: databases and collections are reached
//! through [`LoggedDatabase`] and [`LoggedCollection`], reads return their
//! documents as [`Documents`], and writes are described as a [`Write`], so
//! the call sent and the text recorded come from the same value.

use std::future::IntoFuture;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::{Stream, TryStreamExt};
use mongodb::action::Action;
use mongodb::bson::{Bson, Document, doc};
use mongodb::error::Result;
use mongodb::options::{ClientOptions, UpdateModifications};
use mongodb::results::CollectionSpecification;
use mongodb::{Client, ClientSession, Collection, Cursor, Database};

use super::{literal, object_literal};
use crate::error::mongodb_message;
use crate::statement_log::Recording;

/// A MongoDB client whose operations are recorded.
pub struct LoggedClient {
    client: Client,
}

impl LoggedClient {
    /// Builds the client. Nothing is sent until the first operation, so nothing is recorded.
    pub fn with_options(options: ClientOptions) -> Result<Self> {
        Ok(Self { client: Client::with_options(options)? })
    }

    pub fn database(&self, name: &str) -> LoggedDatabase {
        LoggedDatabase { database: self.client.database(name), comment: None }
    }

    /// The database named in the connection string, if any.
    pub fn default_database(&self) -> Option<String> {
        self.client.default_database().map(|database| database.name().to_owned())
    }

    pub async fn list_database_names(&self) -> Result<Vec<String>> {
        // What the driver sends: the names alone, run on `admin`.
        let recording =
            Recording::start(Some("admin"), || run_command_text(&doc! { "listDatabases": 1, "nameOnly": true }));
        finish(recording, self.client.list_database_names(), |recording, names| recording.rows(names.len() as u64))
            .await
    }

    /// Closes the connections once every [`Documents`] of this client is dropped.
    pub async fn shutdown(self) {
        self.client.shutdown().await;
    }

    /// The driver's own client, for tests to set up and inspect their data unrecorded.
    #[cfg(test)]
    pub fn unlogged(&self) -> &Client {
        &self.client
    }
}

/// A database whose operations are recorded.
#[derive(Clone)]
pub struct LoggedDatabase {
    database: Database,
    /// Sent with every operation and never shown: the console's tag, which
    /// lets a cancel find its operations among the server's.
    comment: Option<Bson>,
}

impl LoggedDatabase {
    /// The same database, with `comment` sent along with every operation.
    pub fn tagged(self, comment: &str) -> Self {
        Self { comment: Some(Bson::String(comment.to_owned())), ..self }
    }

    pub fn collection(&self, name: &str) -> LoggedCollection {
        LoggedCollection { collection: self.database.collection(name), comment: self.comment.clone() }
    }

    fn start(&self, text: impl FnOnce() -> String) -> Recording {
        Recording::start(Some(self.database.name()), text)
    }

    pub async fn run_command(&self, command: Document) -> Result<Document> {
        let recording = self.start(|| run_command_text(&command));
        let mut command = command;
        if let Some(comment) = &self.comment
            && !command.contains_key("comment")
        {
            command.insert("comment", comment.clone());
        }
        finish(recording, self.database.run_command(command), |recording, _| recording.row()).await
    }

    /// A pipeline that starts with a stage such as `$currentOp`, which reads no collection.
    pub async fn aggregate(&self, pipeline: impl IntoIterator<Item = Document>) -> Result<Documents> {
        let pipeline: Vec<Document> = pipeline.into_iter().collect();
        let recording = self.start(|| call("db", "aggregate", &[array_text(&pipeline)]));
        let work = self.database.aggregate(pipeline).optional(self.comment.clone(), |action, c| action.comment(c));
        Documents::open(recording, work).await
    }

    pub async fn list_collections(&self) -> Result<Vec<CollectionSpecification>> {
        let recording = self.start(|| call("db", "getCollectionInfos", &[]));
        let work = async { self.database.list_collections().await?.try_collect::<Vec<_>>().await };
        finish(recording, work, |recording, specs| recording.rows(specs.len() as u64)).await
    }

    pub async fn list_collection_names(&self) -> Result<Vec<String>> {
        let recording = self.start(|| call("db", "getCollectionNames", &[]));
        finish(recording, self.database.list_collection_names(), |recording, names| recording.rows(names.len() as u64))
            .await
    }

    pub async fn create_collection(&self, name: &str) -> Result<()> {
        let recording = self.start(|| call("db", "createCollection", &[string_text(name)]));
        finish(recording, self.database.create_collection(name), |_, _| {}).await
    }

    /// A session to run writes in one transaction. Starting one sends nothing.
    pub async fn start_session(&self) -> Result<LoggedSession> {
        let session = self.database.client().start_session().await?;
        Ok(LoggedSession { session, database: self.database.name().to_owned() })
    }
}

/// A collection whose operations are recorded.
pub struct LoggedCollection {
    collection: Collection<Document>,
    comment: Option<Bson>,
}

impl LoggedCollection {
    fn start(&self, text: impl FnOnce(&str) -> String) -> Recording {
        let name = self.collection.name();
        Recording::start(Some(&self.collection.namespace().db), || text(&collection_text(name)))
    }

    pub async fn find(&self, find: Find) -> Result<Documents> {
        let recording = self.start(|target| find.text(target));
        let Find { filter, projection, sort, skip, limit } = find;
        let work = self
            .collection
            .find(filter)
            .optional(projection, |action, projection| action.projection(projection))
            .optional(sort, |action, sort| action.sort(sort))
            .optional(skip, |action, skip| action.skip(skip))
            .optional(limit, |action, limit| action.limit(limit))
            .optional(self.comment.clone(), |action, comment| action.comment(comment));
        Documents::open(recording, work).await
    }

    pub async fn aggregate(&self, pipeline: impl IntoIterator<Item = Document>) -> Result<Documents> {
        let pipeline: Vec<Document> = pipeline.into_iter().collect();
        let recording = self.start(|target| call(target, "aggregate", &[array_text(&pipeline)]));
        let work = self.collection.aggregate(pipeline).optional(self.comment.clone(), |action, c| action.comment(c));
        Documents::open(recording, work).await
    }

    pub async fn count_documents(&self, filter: Document) -> Result<u64> {
        let recording = self.start(|target| call(target, "countDocuments", &[document_text(&filter)]));
        let work =
            self.collection.count_documents(filter).optional(self.comment.clone(), |action, c| action.comment(c));
        finish(recording, work, |recording, _| recording.row()).await
    }

    /// The count kept in the collection's metadata: instant, but not exact.
    pub async fn estimated_document_count(&self) -> Result<u64> {
        let recording = self.start(|target| call(target, "estimatedDocumentCount", &[]));
        let work =
            self.collection.estimated_document_count().optional(self.comment.clone(), |action, c| action.comment(c));
        finish(recording, work, |recording, _| recording.row()).await
    }

    pub async fn distinct(&self, field: &str, filter: Document) -> Result<Vec<Bson>> {
        let recording = self.start(|target| call(target, "distinct", &[string_text(field), document_text(&filter)]));
        let work =
            self.collection.distinct(field, filter).optional(self.comment.clone(), |action, c| action.comment(c));
        finish(recording, work, |recording, values| recording.rows(values.len() as u64)).await
    }

    pub async fn write(&self, write: &Write) -> Result<Written> {
        let recording = self.start(|target| write.text_on(target));
        finish(recording, send(&self.collection, self.comment.clone(), None, write), Written::tally).await
    }
}

/// A client session, for writes that must apply together or not at all.
pub struct LoggedSession {
    session: ClientSession,
    database: String,
}

impl LoggedSession {
    fn start(&self, method: &str) -> Recording {
        Recording::start(Some(&self.database), || call("session", method, &[]))
    }

    /// Recorded although the driver sends it with the first write, so the log
    /// shows where the transaction starts.
    pub async fn start_transaction(&mut self) -> Result<()> {
        let recording = self.start("startTransaction");
        finish(recording, self.session.start_transaction(), |_, _| {}).await
    }

    pub async fn commit_transaction(&mut self) -> Result<()> {
        let recording = self.start("commitTransaction");
        finish(recording, self.session.commit_transaction(), |_, _| {}).await
    }

    pub async fn abort_transaction(&mut self) -> Result<()> {
        let recording = self.start("abortTransaction");
        finish(recording, self.session.abort_transaction(), |_, _| {}).await
    }

    /// Sends `write` to `collection` in this session.
    pub async fn write(&mut self, collection: &LoggedCollection, write: &Write) -> Result<Written> {
        let recording = collection.start(|target| write.text_on(target));
        let work = send(&collection.collection, collection.comment.clone(), Some(&mut self.session), write);
        finish(recording, work, Written::tally).await
    }
}

/// Awaits `work`, noting in `recording` what came back with `tally`, or why it failed.
async fn finish<T>(
    mut recording: Recording,
    work: impl IntoFuture<Output = Result<T>>,
    tally: impl FnOnce(&mut Recording, &T),
) -> Result<T> {
    let result = work.await;
    match &result {
        Ok(value) => tally(&mut recording, value),
        Err(error) => recording.fail(mongodb_message(error)),
    }
    result
}

/// The documents a find or an aggregate returns, read as a stream. Recorded,
/// with the documents read, when dropped: a cursor read only in part (the
/// first document, a page) is recorded too.
pub struct Documents {
    cursor: Cursor<Document>,
    recording: Recording,
}

impl Documents {
    async fn open(mut recording: Recording, work: impl IntoFuture<Output = Result<Cursor<Document>>>) -> Result<Self> {
        match work.await {
            Ok(cursor) => Ok(Self { cursor, recording }),
            Err(error) => {
                recording.fail(mongodb_message(&error));
                Err(error)
            }
        }
    }
}

impl Stream for Documents {
    type Item = Result<Document>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let next = Pin::new(&mut this.cursor).poll_next(cx);
        match &next {
            Poll::Ready(Some(Ok(_))) => this.recording.row(),
            Poll::Ready(Some(Err(error))) => this.recording.fail(mongodb_message(error)),
            _ => {}
        }
        next
    }
}

/// A find with the modifiers mongosh chains on it.
#[derive(Debug, Clone, Default)]
pub struct Find {
    filter: Document,
    projection: Option<Document>,
    sort: Option<Document>,
    skip: Option<u64>,
    limit: Option<i64>,
}

impl Find {
    pub fn new(filter: Document) -> Self {
        Self { filter, ..Self::default() }
    }

    pub fn projection(self, projection: Document) -> Self {
        Self { projection: Some(projection), ..self }
    }

    pub fn sort(self, sort: Document) -> Self {
        Self { sort: Some(sort), ..self }
    }

    pub fn skip(self, skip: u64) -> Self {
        Self { skip: Some(skip), ..self }
    }

    pub fn limit(self, limit: i64) -> Self {
        Self { limit: Some(limit), ..self }
    }

    /// `target.find(filter, projection).sort(…).skip(…).limit(…)`, with only the parts set.
    fn text(&self, target: &str) -> String {
        let mut args = vec![document_text(&self.filter)];
        args.extend(self.projection.as_ref().map(document_text));
        let mut text = call(target, "find", &args);
        if let Some(sort) = &self.sort {
            text = call(&text, "sort", &[document_text(sort)]);
        }
        if let Some(skip) = self.skip {
            text = call(&text, "skip", &[skip.to_string()]);
        }
        if let Some(limit) = self.limit {
            text = call(&text, "limit", &[limit.to_string()]);
        }
        text
    }
}

/// A change to a collection's documents.
#[derive(Debug, Clone)]
pub enum Write {
    InsertOne(Document),
    InsertMany(Vec<Document>),
    UpdateOne(Update),
    UpdateMany(Update),
    ReplaceOne { filter: Document, replacement: Document, upsert: bool },
    DeleteOne(Document),
    DeleteMany(Document),
}

/// What `updateOne` and `updateMany` take.
#[derive(Debug, Clone)]
pub struct Update {
    pub filter: Document,
    pub update: UpdateModifications,
    pub upsert: bool,
}

impl Write {
    /// The documents it changes, for every write but an insert.
    pub fn filter(&self) -> Option<&Document> {
        match self {
            Write::InsertOne(_) | Write::InsertMany(_) => None,
            Write::UpdateOne(update) | Write::UpdateMany(update) => Some(&update.filter),
            Write::ReplaceOne { filter, .. } | Write::DeleteOne(filter) | Write::DeleteMany(filter) => Some(filter),
        }
    }

    /// The write as mongosh runs it on collection `collection`.
    pub fn text(&self, collection: &str) -> String {
        self.text_on(&collection_text(collection))
    }

    fn text_on(&self, target: &str) -> String {
        // mongosh's own option document, written only when it says something.
        let upserting = |upsert: bool| upsert.then(|| document_text(&doc! { "upsert": true }));
        let (method, args) = match self {
            Write::InsertOne(document) => ("insertOne", vec![document_text(document)]),
            Write::InsertMany(documents) => ("insertMany", vec![array_text(documents)]),
            Write::UpdateOne(update) | Write::UpdateMany(update) => {
                let method = if matches!(self, Write::UpdateOne(_)) { "updateOne" } else { "updateMany" };
                let mut args = vec![document_text(&update.filter), update_text(&update.update)];
                args.extend(upserting(update.upsert));
                (method, args)
            }
            Write::ReplaceOne { filter, replacement, upsert } => {
                let mut args = vec![document_text(filter), document_text(replacement)];
                args.extend(upserting(*upsert));
                ("replaceOne", args)
            }
            Write::DeleteOne(filter) => ("deleteOne", vec![document_text(filter)]),
            Write::DeleteMany(filter) => ("deleteMany", vec![document_text(filter)]),
        };
        call(target, method, &args)
    }
}

/// What a write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    /// Documents its filter found, or documents inserted.
    pub matched: u64,
    /// Documents inserted, changed (counting one upserted) or deleted.
    pub affected: u64,
}

impl Written {
    fn tally(recording: &mut Recording, written: &Self) {
        recording.affected(written.affected);
    }
}

/// Sends `write`, in `session` when there is one.
async fn send(
    collection: &Collection<Document>,
    comment: Option<Bson>,
    session: Option<&mut ClientSession>,
    write: &Write,
) -> Result<Written> {
    let updated = |result: mongodb::results::UpdateResult| Written {
        matched: result.matched_count,
        affected: result.modified_count + u64::from(result.upserted_id.is_some()),
    };
    let deleted = |result: mongodb::results::DeleteResult| Written {
        matched: result.deleted_count,
        affected: result.deleted_count,
    };
    Ok(match write {
        Write::InsertOne(document) => {
            collection
                .insert_one(document)
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?;
            Written { matched: 1, affected: 1 }
        }
        Write::InsertMany(documents) => {
            let inserted = collection
                .insert_many(documents)
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?
                .inserted_ids
                .len() as u64;
            Written { matched: inserted, affected: inserted }
        }
        Write::UpdateOne(Update { filter, update, upsert }) => updated(
            collection
                .update_one(filter.clone(), update.clone())
                .upsert(*upsert)
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?,
        ),
        Write::UpdateMany(Update { filter, update, upsert }) => updated(
            collection
                .update_many(filter.clone(), update.clone())
                .upsert(*upsert)
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?,
        ),
        Write::ReplaceOne { filter, replacement, upsert } => updated(
            collection
                .replace_one(filter.clone(), replacement)
                .upsert(*upsert)
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?,
        ),
        Write::DeleteOne(filter) => deleted(
            collection
                .delete_one(filter.clone())
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?,
        ),
        Write::DeleteMany(filter) => deleted(
            collection
                .delete_many(filter.clone())
                .optional(comment, |action, comment| action.comment(comment))
                .optional(session, |action, session| action.session(session))
                .await?,
        ),
    })
}

/// `db.getCollection("name")`, which reaches any collection, whatever its name.
fn collection_text(name: &str) -> String {
    call("db", "getCollection", &[string_text(name)])
}

/// `target.method(args…)`.
fn call(target: &str, method: &str, args: &[String]) -> String {
    format!("{target}.{method}({})", args.join(", "))
}

fn string_text(text: &str) -> String {
    literal(&Bson::String(text.to_owned()))
}

fn document_text(document: &Document) -> String {
    object_literal(document.iter().map(|(key, value)| (key.as_str(), literal(value))))
}

fn array_text(documents: &[Document]) -> String {
    format!("[{}]", documents.iter().map(document_text).collect::<Vec<_>>().join(", "))
}

fn update_text(update: &UpdateModifications) -> String {
    match update {
        UpdateModifications::Document(document) => document_text(document),
        UpdateModifications::Pipeline(stages) => array_text(stages),
        // A kind added by a later driver: Extended JSON says it exactly.
        other => mongodb::bson::to_bson(other).map(|value| literal(&value)).unwrap_or_default(),
    }
}

/// `db.runCommand(command)`, with a password (`createUser`, `updateUser`)
/// asked for by mongosh instead of written into the log.
fn run_command_text(command: &Document) -> String {
    let fields = command.iter().map(|(key, value)| {
        let value = if key == "pwd" { "passwordPrompt()".to_owned() } else { literal(value) };
        (key.as_str(), value)
    });
    call("db", "runCommand", &[object_literal(fields)])
}

#[cfg(test)]
mod tests {
    use mongodb::bson::oid::ObjectId;

    use super::*;

    fn id() -> ObjectId {
        ObjectId::parse_str("65a0f0f0f0f0f0f0f0f0f0f0").unwrap()
    }

    #[test]
    fn finds_show_only_the_modifiers_they_have() {
        let orders = collection_text("orders");
        assert_eq!(Find::new(doc! {}).text(&orders), r#"db.getCollection("orders").find({})"#);
        let page = Find::new(doc! { "status": "paid" }).sort(doc! { "_id": 1 }).skip(50).limit(51);
        assert_eq!(
            page.text(&orders),
            r#"db.getCollection("orders").find({"status": "paid"}).sort({"_id": 1}).skip(50).limit(51)"#
        );
        let projected = Find::new(doc! {}).limit(1).projection(doc! { "total": 1 });
        assert_eq!(projected.text(&orders), r#"db.getCollection("orders").find({}, {"total": 1}).limit(1)"#);
    }

    #[test]
    fn collections_are_reached_by_name_whatever_it_holds() {
        assert_eq!(collection_text(r#"a.b "c""#), r#"db.getCollection("a.b \"c\"")"#);
    }

    #[test]
    fn writes_show_their_values_written_in() {
        let filter = doc! { "_id": id() };
        let update = |upsert| Update { filter: filter.clone(), update: doc! { "$set": { "n": 2_i64 } }.into(), upsert };
        let cases = [
            (Write::InsertOne(doc! { "a": 1 }), r#"insertOne({"a": 1})"#),
            (Write::InsertMany(vec![doc! { "_id": 7 }, doc! {}]), r#"insertMany([{"_id": 7}, {}])"#),
            (
                Write::UpdateOne(update(false)),
                r#"updateOne({"_id": ObjectId("65a0f0f0f0f0f0f0f0f0f0f0")}, {"$set": {"n": NumberLong(2)}})"#,
            ),
            (
                Write::UpdateMany(update(true)),
                r#"updateMany({"_id": ObjectId("65a0f0f0f0f0f0f0f0f0f0f0")}, {"$set": {"n": NumberLong(2)}}, {"upsert": true})"#,
            ),
            (
                Write::UpdateMany(Update {
                    filter: doc! {},
                    update: UpdateModifications::Pipeline(vec![doc! { "$set": { "x": "$y" } }]),
                    upsert: false,
                }),
                r#"updateMany({}, [{"$set": {"x": "$y"}}])"#,
            ),
            (
                Write::ReplaceOne {
                    filter: doc! { "_id": 2 },
                    replacement: doc! { "_id": 2, "s": "it's" },
                    upsert: true,
                },
                r#"replaceOne({"_id": 2}, {"_id": 2, "s": "it's"}, {"upsert": true})"#,
            ),
            (Write::DeleteOne(doc! { "_id": "3" }), r#"deleteOne({"_id": "3"})"#),
            (Write::DeleteMany(doc! { "n": { "$lt": 1.5 } }), r#"deleteMany({"n": {"$lt": 1.5}})"#),
        ];
        for (write, call) in cases {
            assert_eq!(write.text("orders"), format!(r#"db.getCollection("orders").{call}"#));
        }
        assert_eq!(Write::DeleteOne(filter.clone()).filter(), Some(&filter));
        assert_eq!(Write::InsertOne(doc! {}).filter(), None);
    }

    #[test]
    fn reads_and_commands_read_as_mongosh_calls() {
        let orders = collection_text("orders");
        let pipeline = [doc! { "$match": { "a": 1 } }, doc! { "$limit": 1 }];
        assert_eq!(
            call(&orders, "aggregate", &[array_text(&pipeline)]),
            r#"db.getCollection("orders").aggregate([{"$match": {"a": 1}}, {"$limit": 1}])"#
        );
        assert_eq!(
            call(&orders, "distinct", &[string_text("customer"), document_text(&doc! {})]),
            r#"db.getCollection("orders").distinct("customer", {})"#
        );
        assert_eq!(run_command_text(&doc! { "listCollections": 1 }), r#"db.runCommand({"listCollections": 1})"#);
    }

    #[test]
    fn passwords_are_never_written_into_the_log() {
        let command = doc! { "createUser": "ann", "pwd": "it's secret", "roles": [] };
        assert_eq!(
            run_command_text(&command),
            r#"db.runCommand({"createUser": "ann", "pwd": passwordPrompt(), "roles": []})"#
        );
    }
}
