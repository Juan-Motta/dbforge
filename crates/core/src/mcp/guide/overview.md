# DBForge for agents

DBForge is a database client. Its MCP server lets you query and change the databases the user saved in DBForge without ever handling their credentials: you name a connection by its id, and DBForge opens it with its own secrets.

## Golden rules

1. **Start with `list_connections`.** It lists the connections you may use, each with its `id` (what every other tool takes as `connectionId`), its engine, the language its statements are written in, its tag (e.g. PROD), your `access` and `asksBeforeChanges`. A connection the user didn't share with agents isn't listed, and every tool answers as if it didn't exist.
2. **Respect each connection's access.** `read`: `query`, `list_objects`, `describe_table`, `list_saved_queries`, `run_saved_query`, `connection_details` and `export_database`. `write`: also `execute`, `run_saved_query` with `allowChanges`, and `import_database`. Only the user changes a connection's access, in DBForge.
3. **The user sees every statement you run** in DBForge's statement console, marked as yours. Run what you would show them.
4. **Never ask for, send or repeat a password**, nor any other secret. When a connection needs one, the user types it in DBForge's approval dialog.
5. **Some changes wait for the user's approval in DBForge**:
   - `execute` (and `run_saved_query` with `allowChanges`) on a connection whose tag asks before changes (`asksBeforeChanges: true`);
   - every `save_query` and `delete_saved_query`;
   - every `export_database` and `import_database`;
   - every `save_connection` and `delete_connection`.

   The call waits until the user answers, for up to 5 minutes. **A decline or no answer means nothing ran or changed.** Don't retry it on your own: tell the user it was declined and ask what they want instead.
6. **Read the section you need before your first change**: `dbforge_guide` with `section`, or the `dbforge://guide/<section>` resource.

## Tools

| Tool | What it does |
| --- | --- |
| `list_connections` | The connections you may use. Call it first. |
| `list_objects` | Databases, schemas, tables and views at a path. |
| `describe_table` | A script that creates a table or view. |
| `query` | One statement that only reads. |
| `execute` | Statements that change data (write access). |
| `list_saved_queries`, `run_saved_query` | The user's saved queries, run with values for their `:name` parameters. |
| `save_query`, `delete_saved_query` | Create, change or delete a saved query (approved). |
| `list_exports`, `export_database`, `import_database` | Backups of whole databases and their restores (approved). |
| `connection_details`, `save_connection`, `delete_connection` | A connection's settings without secrets; create, change or delete one (approved). |
| `dbforge_guide` | This guide. Always available, needs no connection. |

## Reading errors

A tool that can't do what you asked returns an error result whose text says why and, usually, what to do next. Act on that text:

- `No connection with id …` / `No saved query with id …`: the id is wrong, or not shared with agents. Call `list_connections` or `list_saved_queries` again.
- `… is read-only for agents`: use `query`, or ask the user to give agents Read and write access to that connection in DBForge.
- `The user declined this … or didn't answer in time. Nothing ran.` (or `Nothing was changed.`): stop and tell the user.
- `… changed while waiting for approval` / `… was deleted while waiting for approval`: the user changed it in DBForge meanwhile, so nothing happened. Read it again and ask again if it still makes sense.
- `DBForge isn't open` / `DBForge's agent server is off`: ask the user to open DBForge, or to turn on "Let AI agents use DBForge" in Settings → Agents (MCP). This guide still works meanwhile.
- `Lost DBForge before it answered, so the call may or may not have run`: check whether it ran (read the data) before trying again.
- Errors from the database itself (syntax, constraints, permissions) come back as the server's message.
