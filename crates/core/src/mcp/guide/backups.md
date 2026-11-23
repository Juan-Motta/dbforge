# Backups and restores

`export_database` backs up a whole database into a new file in DBForge's exports folder; `import_database` restores one of those exports into a database; `list_exports` lists the ones you can restore. Supported engines: **PostgreSQL, MySQL, MariaDB, MongoDB and SQLite**. SQL Server, Cosmos DB and Redis databases can't be backed up or restored this way.

You never name a path: DBForge names every file and only reads files from its exports folder. If the user hasn't chosen one, the tools answer so; the user picks it in DBForge's Settings › Export and import.

## Approvals

**Every backup and every restore waits for the user to approve it in DBForge**, whatever the connection's tag: a backup copies every row out of the database, a restore changes its data. A decline or no answer within 5 minutes runs nothing. If the connection changed in DBForge while waiting, nothing runs either. Once approved, the run goes on to its end even if you disconnect, and the call returns when it ends.

## `list_exports`

Optional `connectionId`: only the exports that can be restored into that connection. You see only exports DBForge itself made from connections you may use; other files in the folder aren't listed and can't be restored. Each export has:

- `name`: what `import_database` takes as `file`;
- `kind`: `sql` (a SQL script: PostgreSQL, MySQL, MariaDB or SQLite), `archive` (a gzipped MongoDB archive) or `sqliteFile` (a copy of a SQLite file, which opens as a connection and can't be restored);
- `restoresInto`, `bytes`, `modified` (RFC 3339, UTC).

## `export_database`

Needs `read` access or more. Takes `connectionId`, optional `database` (the connection's own when left out; not for SQLite, whose connection is one file) and optional `options`. Returns `file`, `bytes` and `elapsedMs`.

The file is named after the database and the local minute: `shop-2026-11-22-1430.sql`, `shop-2026-11-22-1430.archive.gz`, or for SQLite the file's name, e.g. `app-2026-11-22-1430.sql` (or `.db`). A second backup of the same database within the same minute is refused; try again a minute later.

### Options

Each option is for the engines named; one given for another engine is refused. Left out, an option takes its default, which backs up everything.

| Option | Engines | Default | Meaning |
| --- | --- | --- | --- |
| `content` | PostgreSQL, MySQL, MariaDB | `all` | `all`, `structure` or `data` |
| `clean` | PostgreSQL | `false` | drop each object before creating it |
| `keepOwners` | PostgreSQL | `false` | keep owners and privileges (needs the same roles where it's restored) |
| `addDropTable` | MySQL, MariaDB | `false` | `DROP TABLE IF EXISTS` before each table |
| `singleTransaction` | MySQL, MariaDB | `true` | read InnoDB tables in one consistent snapshot |
| `routines` | MySQL, MariaDB | `true` | include procedures, functions, triggers and events |
| `collections` | MongoDB | all | only these collections |
| `format` | SQLite | `script` | `script` (restorable SQL) or `file` (a copy of the database file) |

PostgreSQL, MySQL, MariaDB and MongoDB backups run the engine's client tools (pg_dump, mysqldump or mariadb-dump, mongodump) installed on the user's computer; SQLite needs none.

## `import_database`

Needs `write` access. Takes:

- `connectionId`;
- `file`: an export's `name` from `list_exports`, without a folder. Its kind must match the connection: `.sql` for PostgreSQL, MySQL, MariaDB and SQLite, `.archive.gz` for MongoDB. An export restores only into the engine it was made from (a MySQL or MariaDB export into MySQL or MariaDB): a PostgreSQL `.sql` into a SQLite connection, say, is refused, and `list_exports` with a `connectionId` lists only those that fit;
- `database`: the database to restore into, created when it doesn't exist. Needed, except for SQLite, which takes none;
- `dropExisting`: MongoDB only, default `false`.

Returns `file`, `database`, `created` (the database was created for the restore), `elapsedMs` and `skippedExtensions`. Objects already in the target may be replaced. **A failed restore drops a database it created.**

### What a restore does, per engine

- **PostgreSQL** (psql): the whole file runs in one transaction and stops at the first error, so a failed restore changes nothing. Extensions the file creates that the server doesn't have are left out and listed in `skippedExtensions`.
- **MySQL, MariaDB** (mysql or mariadb): a file that switches to or creates another database (`USE`, `CREATE DATABASE`, `connect`, …) is refused, since it wouldn't restore into the target; export it again without `--databases`. DBForge's own exports don't name their database. The restore stops at the first error, but tables already created stay (MySQL commits DDL on its own).
- **MongoDB** (mongorestore): the archive's database is renamed to the target. It stops at the first error, so a collection that already holds a document with the same `_id` stops it there; set `dropExisting: true` to drop each collection the archive holds before restoring it.
- **SQLite**: restores a SQL script into the connection's own file, in one transaction rolled back on failure. A script that begins or commits transactions of its own is refused. A `.db` file copy isn't restored: the user opens it as a connection.
