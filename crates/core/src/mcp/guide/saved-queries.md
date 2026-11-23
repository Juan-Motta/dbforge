# Saved queries

The user keeps reusable queries in DBForge's library. You can list and run them, and create, change or delete them once the user approves.

## Scopes

A saved query runs either on **one connection** (`scope: "connection"`) or on **any connection of an engine** (`scope: "engine"`). You see a saved query only when it can run on a connection you may use; any other reads as unknown.

## `list_saved_queries`

Optional `connectionId`: only the queries that run on that connection. Each query has `id`, `name`, `description`, `folder`, `scope`, `engine`, `connections` (the ones you may run it on), `sql` and `parameters` (its `:name` parameters, in order of first use).

## `run_saved_query`

- `id`: from `list_saved_queries`.
- `connectionId`: where to run it, one of its `connections`. Needed for a query saved for any connection of an engine; a query saved for one connection runs there.
- `values`: one value per parameter, by name without the colon.
- `database`, `container` (Cosmos DB), `limit`: as for `query`.
- `allowChanges`: left out, the query runs as `query` does (one statement, read-only; a change is refused). Set it to `true` for a query that changes data: it then runs as `execute` does, only on a connection whose access is `write`, approved by the user when the connection asks before changes.

### Parameters and values

A parameter is `:name` in the SQL (letters, digits, underscores), outside strings, quoted names, comments and PostgreSQL casts (`::int` is not a parameter). Only SQL engines have parameters. Each value is `{"kind": …, "value": "…"}`, written into the SQL as a literal of the connection's dialect:

| `kind` | `value` | Written as |
| --- | --- | --- |
| `text` | any text | quoted and escaped (`N'…'` on SQL Server) |
| `number` | e.g. `42`, `-3.5` | as is; anything else is refused |
| `date` | `YYYY-MM-DD` | `'2026-11-22'` |
| `dateTime` | `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS` | `'2026-11-22 14:30:00'` |
| `boolean` | `true`, `false`, `1` or `0` | `true`/`false`; `1`/`0` on SQL Server and SQLite |
| `null` | ignored, may be left out | `NULL` |

Example, for `SELECT * FROM orders WHERE status = :status AND total > :min AND placed_at >= :since`:

```json
{
  "id": "…",
  "values": {
    "status": {"kind": "text", "value": "paid"},
    "min": {"kind": "number", "value": "100"},
    "since": {"kind": "date", "value": "2026-01-01"}
  }
}
```

A missing or invalid value is refused before anything runs, naming the parameter (e.g. `Enter a value for :status.`).

## `save_query`

Creates a saved query, or with `id` changes one.

- To create: `name`, `sql` and `scope` are needed; `description` and `folder` are optional.
- `scope` is `{"connectionId": "…"}` for one connection, or `{"engine": "PostgreSQL"}` for any connection of an engine (the engine's name as `list_connections` gives it, or its id such as `postgres`), an engine of a connection you may use.
- To change: give `id` and only what changes; what you leave out stays. An empty `description` removes it; an empty `folder` moves the query to the top level. A change that changes nothing returns the query without asking the user.
- Write the SQL in the language of its connections, with `:name` placeholders for the values that change between runs.

## `delete_saved_query`

Takes `id`. Deletes the saved query.

## Approvals

Every `save_query` and `delete_saved_query` waits for the user to approve it in DBForge, where they see the query before and after. A decline or no answer within 5 minutes changes nothing; tell the user rather than retrying.

If the user changed or deleted that saved query in DBForge while your request waited, nothing is changed and the answer says so (`The saved query … changed while waiting for approval, so nothing was changed.`). Call `list_saved_queries`, look at the query as it is now, and ask again only if your change still makes sense.
