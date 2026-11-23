# Engines

What changes from one engine to another. `list_connections` gives each connection's engine and language.

## PostgreSQL

- Language: PostgreSQL SQL. `list_objects`: `[]` databases, `[db]` schemas, `[db, schema]` tables and views.
- `query`: one statement, in a read-only transaction that is rolled back; stopped by the server after a minute. Not prevented by the server: functions that act outside the tables (`pg_terminate_backend`, `dblink`, `COPY … TO` a file for superusers); don't call them from `query`.
- `execute`: a script of several statements runs as one transaction: if one fails, the ones before it are rolled back too.
- Backups: pg_dump and psql. See the `backups` section.

## MySQL and MariaDB

- Language: MySQL SQL. A database is also its schema: `list_objects` `[]` lists databases, `[db]` their tables and views; `describe_table` takes the database as `schema` too.
- `query`: one statement, in a read-only transaction that is rolled back. Statements MySQL can't prepare are refused.
- `execute`: a script of several statements, each committing on its own unless you write `START TRANSACTION … COMMIT`. DDL always commits.
- Backups: mysqldump and mysql (MariaDB's mariadb-dump and mariadb). A restore refuses files that switch to or create another database.

## SQLite

- A connection is one database file (its `database` is the file's path); host, port and user don't apply. `list_objects` `[]` lists the file's tables directly; in `describe_table`, `database` and `schema` are both `main`.
- `query`: the file is opened read-only; one statement starting with a reading word (`SELECT`, `WITH`, …); `PRAGMA` is refused.
- `execute`: runs every statement of the script.
- Backups: built in, no tool needed. `format: "script"` (default, restorable) or `"file"` (a copy). A restore runs the script into the connection's own file; it takes no `database`.

## SQL Server

- Language: T-SQL. `list_objects`: `[]` databases, `[db]` schemas, `[db, schema]` tables and views.
- `query`: one `SELECT`, optionally after `WITH`. Words that write or start another statement are refused anywhere outside strings, names and comments (`INTO`, `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `EXEC`, `SET`, `BEGIN`, `COMMIT`, …), as are `OPENQUERY`, `OPENROWSET` and `OPENDATASOURCE`. It runs in a transaction that is rolled back.
- `execute`: a script of batches separated by `GO` lines, as SQL Server's tools run it.
- No backups or restores through DBForge.

## MongoDB

- Language: a subset of mongosh. Call methods on a collection, `db.orders.find(…)` or `db.getCollection("orders").find(…)`, or on `db`: `db.runCommand({…})`, `db.getCollectionNames()`. Arguments are literals as mongosh writes them: unquoted keys, either quote style, `/regex/i`, `ObjectId("…")`, `ISODate("…")`, `new Date(…)`, `NumberLong(…)`, `NumberInt(…)`, `NumberDecimal("…")`. Variables and functions aren't supported. `list_objects`: `[]` databases, `[db]` collections.
- `query`: `find` and `findOne` (with `.sort()`, `.projection()`, `.skip()`, `.limit()`), `aggregate` (no `$out` or `$merge` anywhere in the pipeline), `countDocuments`, `count`, `estimatedDocumentCount`, `distinct`, `db.getCollectionNames()`, and `db.runCommand` for reading commands only. One statement.
- `execute`: also `insertOne`, `insertMany`, `updateOne`, `updateMany`, `replaceOne`, `deleteOne`, `deleteMany`, and `use <database>`. A script is read whole first, so a typo anywhere means nothing runs.
- Backups: mongodump and mongorestore, into a `.archive.gz`. A restore stops at the first duplicate `_id` unless `dropExisting` is set.

## Azure Cosmos DB

- Language: Cosmos DB SQL, on **one container**: give `container` on every `query` and `execute`. The name after `FROM` is only an alias for that container: `SELECT * FROM c WHERE c.status = "paid"`. `list_objects`: `[]` databases, `[db]` containers.
- Cosmos DB SQL only reads: `query` takes one query; `execute` takes several separated by `;`, and can't change items either.
- No backups or restores through DBForge.

## Redis

- Language: redis-cli commands, e.g. `HGETALL user:1`. `database` is the database number (`0`, `1`, …); the connection's own when left out. `list_objects` `[]` lists the numbered databases.
- `query`: one command per call, only ones the server flags `readonly` and not `admin`, blocking (e.g. `XREAD`) or pub/sub. `EVAL_RO` and `FCALL_RO` run their scripts read-only.
- `execute`: one command per line. A line the server refuses returns its error, and the next lines still run.
- No backups or restores through DBForge.
