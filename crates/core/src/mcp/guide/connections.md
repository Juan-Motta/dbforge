# Connections

You can read a connection's settings without its secrets, and create, change or delete connections once the user approves. You never send or see a password: the user types secrets in DBForge's approval dialog.

## `connection_details`

Takes `connectionId`. Returns `id`, `name`, `engine` (the id `save_connection` takes), `engineName`, `host`, `port`, `database` (for SQLite, the file's path), `username`, `ssl` (`mode`, `caFile`, `certFile`, `keyFile`), `ssh` (`enabled`, `host`, `port`, `username`, `auth`, `keyFile`), `srv`, `tag` and `access` (`none`, `read` or `write`). Never a password, SSH password, passphrase or key.

## `save_connection`

Without `id`, creates a connection; with `id`, changes one you may use, keeping every field you leave out.

| Field | Notes |
| --- | --- |
| `name` | Needed to create. |
| `engine` | Needed to create: `postgres`, `mysql`, `mariadb`, `sqlite`, `sqlserver`, `mongodb`, `redis` or `cosmosdb`. |
| `host` | The server's host (a Cosmos DB account's); not for SQLite. |
| `port` | The engine's usual port unless given. Changing the engine moves to the new engine's port unless you give one. |
| `database` | The database to open; for SQLite, the database file's path (needed). |
| `username` | |
| `ssl` | `{mode, caFile, certFile, keyFile}`; `mode` is `disable`, `prefer`, `require`, `verify-ca` or `verify-full`. A new connection starts at `prefer` where the engine has it. Files are paths on the user's computer, in PEM. |
| `ssh` | `{enabled, host, port, username, auth, keyFile}`; `port` 22 unless given; `auth` is `password`, `key` (with `keyFile`) or `agent` (the system's SSH agent). |
| `srv` | MongoDB: `host` names a DNS seed list (`mongodb+srv://`). |

What you can't send, and the call refuses:

- **Secrets**: `password`, `sshPassword`, `sshSecret`, `passphrase`. The user types them in the approval dialog. On a change, typing nothing keeps the saved ones.
- **What agents may do, and the tag**: `agentAccess`, `tag`, `environment`. The user chooses a new connection's access in the dialog (Read only unless they change it); a change keeps both.

**Pointing a connection elsewhere** (changing its `engine`, `host`, `port`, `srv`, or its SSH tunnel's `enabled`, `host`, `port` or `username`) doesn't carry the saved password to the new place: the user must type it again when approving, or nothing is saved. The same goes for the SSH password or passphrase when the tunnel moves or changes how it signs in. A change that changes nothing returns the connection without asking.

Returns the connection's details, as `connection_details` does.

## `delete_connection`

Takes `id`. Deletes the connection with its saved password.

## Approvals and refusals

- Every create, change and delete waits for the user's approval in DBForge. A decline or no answer within 5 minutes changes nothing; tell the user rather than retrying.
- If the connection changed or was deleted in DBForge while waiting, nothing is changed: call `connection_details` and decide again.
- A connection with a transaction open in one of DBForge's consoles can't be changed or deleted (`A transaction is open on …`). Ask the user to commit or roll it back in DBForge, then try again.
