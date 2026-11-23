# Queries: reading and changing data

## Finding your way

- `list_objects` takes `connectionId` and a `path` of names from the root: `[]` lists databases, `["shop"]` that database's schemas (or its tables, collections or containers, for engines without schemas), `["shop", "public"]` the tables and views of that schema. Each object has a `kind` (`database`, `schema`, `table`, `view`) and, when the server knows it, an approximate `rowEstimate`.
- `describe_table` takes `connectionId`, `database`, `schema` (for engines without schemas, the same as `database`) and `table`, and returns a script that creates it: columns, types, keys, indexes, foreign keys, triggers.

## `query` or `execute`

Both take `connectionId`, `statement` (in the connection's language, see `list_connections`), and optionally `database` (the connection's own when left out), `container` (Cosmos DB only) and `limit`.

- **`query`** runs **one statement that only reads**, on any connection you may use. DBForge keeps it from changing anything: a statement that would is refused or fails, and a script of several statements is refused. It stops after one minute.
- **`execute`** runs statements that change data (INSERT, UPDATE, DELETE, DDL, or the engine's equivalent), only on a connection whose `access` is `write`. On a connection with `asksBeforeChanges: true`, the user approves the statement in DBForge first; a declined one doesn't run. `execute` may take a script of several statements.

Use `query` whenever you only read: it works on read-only connections, needs no approval, and can't change data by mistake.

## How read-only is enforced

| Engine | How `query` is kept from writing |
| --- | --- |
| PostgreSQL | A read-only session, inside a transaction that is always rolled back; one statement only; the server stops it after a minute. |
| MySQL, MariaDB | `SET SESSION TRANSACTION READ ONLY`, inside a transaction that is always rolled back; one statement only. |
| SQLite | The file is opened read-only; only one statement that starts with a reading word; `PRAGMA` is refused. |
| SQL Server | One `SELECT` (optionally after `WITH`); words that write or start another statement (`INSERT`, `UPDATE`, `DELETE`, `INTO`, `EXEC`, `SET`, `BEGIN`, …) and `OPENQUERY`/`OPENROWSET`/`OPENDATASOURCE` are refused; it runs in a transaction that is rolled back. |
| MongoDB | Only `find`, `findOne`, `aggregate` (without `$out`/`$merge`), `countDocuments`, `count`, `estimatedDocumentCount`, `distinct`, `db.getCollectionNames()` and read-only `db.runCommand(…)`. |
| Cosmos DB | Cosmos DB SQL only reads. |
| Redis | Only commands the server flags `readonly`, and not `admin`, blocking or pub/sub ones. One command per call. |

## Results

`query` and `execute` return `results`, one per statement that ran, and `elapsedMs`. Each result has `columns`, `rows` (every value as text, `null` for NULL), `truncated` and `rowsAffected` (rows returned, or changed by a statement that returns none).

- `limit` is the most rows returned per result: 100 unless given, at most 1000. `truncated: true` means more rows came back and the rest were left out; narrow the statement (WHERE, LIMIT/TOP, a projection) rather than raising the limit.
- Values over 2000 characters are cut, ending in `…`. Select the part you need (e.g. a JSON path) instead of whole documents or blobs.

## Language per engine

| Engine | Statements are written in |
| --- | --- |
| PostgreSQL | PostgreSQL SQL |
| MySQL, MariaDB | MySQL SQL |
| SQLite | SQLite SQL |
| SQL Server | T-SQL; `execute` splits a script into batches on `GO` lines |
| MongoDB | mongosh, e.g. `db.getCollection("orders").find({ status: "paid" })` |
| Cosmos DB | Cosmos DB SQL on one container, e.g. `SELECT * FROM c WHERE c.status = "paid"`; give `container` |
| Redis | redis-cli commands, e.g. `HGETALL user:1`; `query` takes one, `execute` one per line |

See the `engines` section for each engine's details.
