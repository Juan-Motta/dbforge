//! A node's right-click menu: the commands it offers. Only commands whose
//! behaviour exists are listed; features that open tabs or dialogs add theirs.
//! A connection's own commands follow the former web UI's order: a new query first,
//! then copy its URL, duplicate, edit, delete. A table's or view's start
//! with opening its rows, its structure, then a new query, as the former web UI's
//! do; a schema's (or a database that is its own schema) offer its ER
//! diagram after the new query. A table's or view's copies are last, under
//! a "Copy" heading as the former web UI's Copy submenu: its name, then a
//! MongoDB collection's mongosh `find()` and `countDocuments()`, or a
//! relational table's SELECT and INSERT naming every column and its CREATE
//! statement (a view's definition). A connection's server objects
//! it can create follow its new query under a "New" heading; a database
//! that can be dropped offers it last. A table's or view's export, and a
//! table's import of a file's rows, follow its new query; a PostgreSQL
//! database's export to a .sql file and import of one come before its drop.

use dbforge_core::connections::model::Engine;
use dbforge_core::drivers::TableStructure;

use super::tree::{Node, NodeKind};
use crate::admin::NewObjectKind;
use crate::admin::new_object::{drops_databases, menu_icon_of, menu_label_of, new_objects};
use crate::connections::engine::is_relational;
use crate::dump::dumps;
use crate::i18n::Messages;
use crate::workspace::has_query_console;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCommand {
    /// Opens a table's or view's rows in a tab.
    OpenData,
    /// Opens a table's or view's tab on its structure's columns.
    ViewStructure,
    /// Opens the ER diagram of a schema, or of a database that is its own schema.
    ShowDiagram,
    /// Opens a query console on the node's database (a table's starts with its first rows).
    NewQuery,
    /// Lists the node's part of the tree again.
    Refresh,
    /// Lists the node's whole connection again, e.g. once a database was dropped.
    RefreshConnection,
    /// Puts the node's name on the clipboard.
    CopyName,
    /// A table's or view's Copy menu: puts what it names on the clipboard.
    Copy(CopyKind),
    /// Puts the connection's URL, without its password, on the clipboard.
    CopyConnectionUrl,
    /// Saves a copy of the connection, its secrets included.
    DuplicateConnection,
    /// Opens the connection dialog on it.
    EditConnection,
    /// Asks before deleting it.
    DeleteConnection,
    /// Creates a database, role or user on the connection's server.
    NewObject(NewObjectKind),
    /// Drops the database from its server, once its name is typed again.
    DropDatabase,
    /// Exports a table's or view's rows.
    ExportTable,
    /// Imports a file's rows into a table (a view can't take them).
    ImportRows,
    /// Writes the database to a .sql file with pg_dump.
    ExportDatabase,
    /// Restores a .sql file into the database with psql.
    ImportSqlFile,
}

/// What a table's or view's Copy menu puts on the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Name,
    /// The mongosh that reads a collection's first documents.
    Find,
    /// The mongosh that counts a collection's documents.
    Count,
    /// A SELECT naming every column.
    Select,
    /// An INSERT naming every column, NULL in each value.
    Insert,
    /// A table's CREATE statement.
    Create,
    /// A view's definition.
    Definition,
}

impl CopyKind {
    /// It is written from the table's columns or DDL, read from its database first.
    pub fn needs_structure(self) -> bool {
        matches!(self, Self::Select | Self::Insert | Self::Create | Self::Definition)
    }
}

/// What `kind` copies of `node` (a table or view), given its `structure`
/// when [`CopyKind::needs_structure`]; `None` when it can't be written.
pub fn copied_text(kind: CopyKind, node: &Node, structure: Option<&TableStructure>) -> Option<String> {
    use crate::sql::snippets::{count_query, insert_statement, preview_query, select_statement, table_ref};
    let table = table_ref(&node.path)?;
    let columns = || -> Option<(crate::sql::Dialect, Vec<String>)> {
        let names = structure?.columns.iter().map(|column| column.name.clone()).collect();
        Some((crate::sql::Dialect::of(node.engine)?, names))
    };
    match kind {
        CopyKind::Name => Some(node.name.clone()),
        CopyKind::Find => preview_query(node.engine, &table),
        CopyKind::Count => Some(count_query(&table)),
        CopyKind::Select => columns().map(|(dialect, names)| select_statement(dialect, &table, &names)),
        CopyKind::Insert => columns().map(|(dialect, names)| insert_statement(dialect, &table, &names)),
        CopyKind::Create | CopyKind::Definition => structure.map(|structure| structure.ddl.clone()),
    }
}

/// A table's or view's Copy menu, in the former web UI's order.
fn copy_kinds(node: &Node) -> Vec<CopyKind> {
    let mut kinds = vec![CopyKind::Name];
    if node.engine == Engine::Mongodb {
        kinds.extend([CopyKind::Find, CopyKind::Count]);
    }
    if is_relational(node.engine) {
        let ddl = if node.kind == NodeKind::View { CopyKind::Definition } else { CopyKind::Create };
        kinds.extend([CopyKind::Select, CopyKind::Insert, ddl]);
    }
    kinds
}

impl NodeCommand {
    pub fn label(self, m: &Messages) -> &'static str {
        match self {
            Self::OpenData => m.open_data,
            Self::ViewStructure => m.view_structure,
            Self::ShowDiagram => m.show_diagram,
            Self::NewQuery => m.new_query,
            Self::Refresh => m.refresh,
            Self::RefreshConnection => m.refresh_connection,
            Self::CopyName => m.copy_name,
            Self::Copy(kind) => match kind {
                CopyKind::Name => m.shell.copy_name,
                CopyKind::Find => m.shell.copy_find,
                CopyKind::Count => m.shell.copy_count,
                CopyKind::Select => m.shell.copy_select,
                CopyKind::Insert => m.shell.copy_insert,
                CopyKind::Create => m.shell.copy_create,
                CopyKind::Definition => m.shell.copy_definition,
            },
            Self::CopyConnectionUrl => m.copy_connection_url,
            Self::DuplicateConnection => m.duplicate_connection,
            Self::EditConnection => m.edit_connection,
            Self::DeleteConnection => m.delete_connection,
            Self::NewObject(kind) => menu_label_of(kind, &m.admin),
            Self::DropDatabase => m.admin.drop_menu,
            Self::ExportTable => m.export_menu,
            Self::ImportRows => m.import_menu,
            Self::ExportDatabase => m.export_database,
            Self::ImportSqlFile => m.import_sql_file,
        }
    }

    /// The heading it is listed under: the server objects a connection
    /// creates are under "New", a table's or view's copies under "Copy".
    pub fn group(self, m: &Messages) -> Option<&'static str> {
        match self {
            Self::NewObject(_) => Some(m.admin.new_menu),
            Self::Copy(_) => Some(m.shell.copy_menu),
            _ => None,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::OpenData => "table",
            Self::ViewStructure => "column",
            Self::ShowDiagram => "diagram",
            Self::NewQuery => "code",
            Self::Refresh | Self::RefreshConnection => "refresh",
            Self::CopyName | Self::Copy(_) => "copy",
            Self::CopyConnectionUrl => "external",
            Self::DuplicateConnection => "duplicate",
            Self::EditConnection => "edit",
            Self::DeleteConnection | Self::DropDatabase => "trash",
            Self::NewObject(kind) => menu_icon_of(kind),
            Self::ExportTable | Self::ExportDatabase => "export",
            Self::ImportRows | Self::ImportSqlFile => "upload",
        }
    }
}

/// The commands `node`'s menu offers, in order. A new query comes first
/// wherever the engine has a console.
pub fn commands(node: &Node) -> Vec<NodeCommand> {
    use NodeCommand::*;
    let query = has_query_console(node.engine) && node.kind != NodeKind::Folder;
    let mut commands =
        if matches!(node.kind, NodeKind::Table | NodeKind::View) { vec![OpenData, ViewStructure] } else { Vec::new() };
    if query {
        commands.push(NewQuery);
    }
    match node.kind {
        NodeKind::Table => commands.extend([ExportTable, ImportRows]),
        NodeKind::View => commands.push(ExportTable),
        _ => {}
    }
    if diagram_target(node).is_some() {
        commands.push(ShowDiagram);
    }
    if node.kind == NodeKind::Connection {
        commands.extend(new_objects(node.engine).iter().map(|kind| NewObject(*kind)));
    }
    commands.extend(match node.kind {
        // A connection's own refresh is its whole connection already.
        NodeKind::Connection => {
            vec![Refresh, CopyName, CopyConnectionUrl, DuplicateConnection, EditConnection, DeleteConnection]
        }
        NodeKind::Folder => vec![Refresh, RefreshConnection],
        NodeKind::Database if dumps(node.engine) || drops_databases(node.engine) => {
            let mut database = vec![Refresh, RefreshConnection, CopyName];
            if dumps(node.engine) {
                database.extend([ExportDatabase, ImportSqlFile]);
            }
            if drops_databases(node.engine) {
                database.push(DropDatabase);
            }
            database
        }
        NodeKind::Database | NodeKind::Schema => vec![Refresh, RefreshConnection, CopyName],
        NodeKind::Table | NodeKind::View => {
            let mut table = vec![Refresh, RefreshConnection];
            table.extend(copy_kinds(node).into_iter().map(Copy));
            table
        }
    });
    commands
}

/// The database and schema a node's ER diagram shows: a schema's, or a
/// database's that is its own schema (MySQL); `None` when it has none.
pub fn diagram_target(node: &Node) -> Option<(String, String)> {
    if !is_relational(node.engine) {
        return None;
    }
    match (node.kind, node.path.as_slice()) {
        (NodeKind::Schema, [database, schema, ..]) => Some((database.clone(), schema.clone())),
        // Where the database is its own default schema: MySQL's and MariaDB's.
        (NodeKind::Database, [database]) => {
            let default_schema = crate::sql::Dialect::of(node.engine)?.default_schema().unwrap_or(database);
            (default_schema == database).then(|| (database.clone(), database.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn node(kind: NodeKind) -> Node {
        Node { engine: Engine::Redis, ..sql_node(kind) }
    }

    fn sql_node(kind: NodeKind) -> Node {
        Node {
            id: "id".into(),
            name: "orders".into(),
            kind,
            meta: None,
            connection_id: "pg".into(),
            engine: Engine::Postgres,
            path: Vec::new(),
            expandable: false,
            loading: false,
            error: None,
        }
    }

    #[test]
    fn offers_refresh_everywhere_and_the_connection_s_refresh_below_it() {
        use NodeCommand::*;
        assert_eq!(
            commands(&node(NodeKind::Connection)),
            [
                NewObject(NewObjectKind::User),
                Refresh,
                CopyName,
                CopyConnectionUrl,
                DuplicateConnection,
                EditConnection,
                DeleteConnection
            ]
        );
        assert_eq!(commands(&node(NodeKind::Folder)), [Refresh, RefreshConnection]);
        for kind in [NodeKind::Database, NodeKind::Schema] {
            assert_eq!(commands(&node(kind)), [Refresh, RefreshConnection, CopyName], "{kind:?}");
        }
    }

    #[test]
    fn offers_a_new_query_first_where_the_engine_has_a_sql_console() {
        use NodeCommand::*;
        assert_eq!(commands(&sql_node(NodeKind::Connection))[..2], [NewQuery, NewObject(NewObjectKind::Database)]);
        assert_eq!(commands(&sql_node(NodeKind::Schema)), [NewQuery, Refresh, RefreshConnection, CopyName]);
        assert_eq!(
            commands(&sql_node(NodeKind::Database)),
            [NewQuery, Refresh, RefreshConnection, CopyName, ExportDatabase, ImportSqlFile, DropDatabase]
        );
        assert_eq!(commands(&sql_node(NodeKind::Folder)), [Refresh, RefreshConnection]);
        for engine in [Engine::Mongodb, Engine::Cosmosdb] {
            let database = Node { engine, ..sql_node(NodeKind::Database) };
            assert_eq!(commands(&database)[0], NewQuery, "{engine:?}");
        }
        assert!(!commands(&node(NodeKind::Database)).contains(&NewQuery), "Redis has no console");
        assert_eq!(NodeCommand::NewQuery.label(Language::Es.messages()), "Nueva consulta");
    }

    #[test]
    fn opens_a_table_s_or_view_s_rows_first_on_every_engine() {
        use NodeCommand::*;
        assert_eq!(
            commands(&sql_node(NodeKind::Table)),
            [
                OpenData,
                ViewStructure,
                NewQuery,
                ExportTable,
                ImportRows,
                Refresh,
                RefreshConnection,
                Copy(CopyKind::Name),
                Copy(CopyKind::Select),
                Copy(CopyKind::Insert),
                Copy(CopyKind::Create)
            ]
        );
        let cosmos = Node { engine: Engine::Cosmosdb, ..sql_node(NodeKind::Table) };
        assert_eq!(
            commands(&cosmos),
            [
                OpenData,
                ViewStructure,
                NewQuery,
                ExportTable,
                ImportRows,
                Refresh,
                RefreshConnection,
                Copy(CopyKind::Name)
            ]
        );
        // A view can't be imported into, and its DDL is its definition.
        assert_eq!(
            commands(&sql_node(NodeKind::View)),
            [
                OpenData,
                ViewStructure,
                NewQuery,
                ExportTable,
                Refresh,
                RefreshConnection,
                Copy(CopyKind::Name),
                Copy(CopyKind::Select),
                Copy(CopyKind::Insert),
                Copy(CopyKind::Definition)
            ]
        );
        assert_eq!(OpenData.label(Language::En.messages()), "Open data");
        assert_eq!(OpenData.label(Language::Es.messages()), "Abrir datos");
        assert_eq!(ViewStructure.label(Language::En.messages()), "View structure");
        assert_eq!(ViewStructure.label(Language::Es.messages()), "Ver estructura");
    }

    fn at(kind: NodeKind, engine: Engine, path: &[&str]) -> Node {
        Node { engine, path: path.iter().map(|part| part.to_string()).collect(), ..sql_node(kind) }
    }

    #[test]
    fn offers_a_schema_s_er_diagram_after_the_new_query() {
        use NodeCommand::*;
        let schema = at(NodeKind::Schema, Engine::Postgres, &["shop", "public"]);
        assert_eq!(commands(&schema), [NewQuery, ShowDiagram, Refresh, RefreshConnection, CopyName]);
        assert_eq!(diagram_target(&schema), Some(("shop".into(), "public".into())));
        // A MySQL database is its own schema; a PostgreSQL one has schemas inside.
        let mysql = at(NodeKind::Database, Engine::Mysql, &["app"]);
        assert_eq!(commands(&mysql)[..2], [NewQuery, ShowDiagram]);
        assert_eq!(commands(&mysql).last(), Some(&DropDatabase));
        assert_eq!(diagram_target(&mysql), Some(("app".into(), "app".into())));
        assert_eq!(diagram_target(&at(NodeKind::Database, Engine::Postgres, &["shop"])), None);
        assert_eq!(
            diagram_target(&at(NodeKind::Schema, Engine::Sqlserver, &["sales", "dbo"])),
            Some(("sales".into(), "dbo".into()))
        );
        // Documents and keys have no relations to draw.
        assert_eq!(diagram_target(&at(NodeKind::Database, Engine::Mongodb, &["catalog"])), None);
        assert_eq!(diagram_target(&at(NodeKind::Table, Engine::Postgres, &["shop", "public", "orders"])), None);
        assert_eq!(ShowDiagram.label(Language::En.messages()), "Show ER diagram");
        assert_eq!(ShowDiagram.label(Language::Es.messages()), "Mostrar diagrama ER");
    }

    #[test]
    fn names_each_command_in_the_language_in_use() {
        assert_eq!(NodeCommand::CopyName.label(Language::En.messages()), "Copy name");
        assert_eq!(NodeCommand::RefreshConnection.label(Language::Es.messages()), "Actualizar conexión");
        assert_eq!(NodeCommand::DuplicateConnection.label(Language::En.messages()), "Duplicate connection");
        assert_eq!(NodeCommand::DeleteConnection.label(Language::Es.messages()), "Eliminar conexión");
    }

    #[test]
    fn copies_a_collection_s_find_and_count_in_mongosh() {
        use NodeCommand::*;
        let collection = at(NodeKind::Table, Engine::Mongodb, &["shop", "orders"]);
        assert_eq!(
            commands(&collection),
            [
                OpenData,
                ViewStructure,
                NewQuery,
                ExportTable,
                ImportRows,
                Refresh,
                RefreshConnection,
                Copy(CopyKind::Name),
                Copy(CopyKind::Find),
                Copy(CopyKind::Count)
            ]
        );
        assert_eq!(Copy(CopyKind::Find).label(Language::En.messages()), "find() query");
        assert_eq!(Copy(CopyKind::Count).label(Language::Es.messages()), "Consulta countDocuments()");
    }

    #[test]
    fn lists_a_table_s_copies_under_copy_named_as_the_former_web_ui() {
        use NodeCommand::*;
        let (en, es) = (Language::En.messages(), Language::Es.messages());
        let labels = |node: &Node, m: &Messages| -> Vec<(Option<&str>, &str)> {
            commands(node).into_iter().filter(|c| matches!(c, Copy(_))).map(|c| (c.group(m), c.label(m))).collect()
        };
        let table = at(NodeKind::Table, Engine::Postgres, &["shop", "public", "users"]);
        assert_eq!(
            labels(&table, en),
            [
                (Some("Copy"), "Name"),
                (Some("Copy"), "SELECT statement"),
                (Some("Copy"), "INSERT statement"),
                (Some("Copy"), "CREATE statement")
            ]
        );
        let view = at(NodeKind::View, Engine::Mysql, &["shop", "v"]);
        assert_eq!(labels(&view, es).last(), Some(&(Some("Copiar"), "Definición")));
        assert_eq!(labels(&view, es)[0], (Some("Copiar"), "Nombre"));
        // A connection's, a database's and a schema's name stay a single "Copy name".
        assert_eq!(CopyName.group(en), None);
        assert_eq!(CopyName.label(en), "Copy name");
    }

    fn structure(columns: &[&str], ddl: &str) -> TableStructure {
        use dbforge_core::drivers::{ColumnInfo, ObjectKind};
        let column = |name: &&str| ColumnInfo {
            name: (*name).into(),
            data_type: "text".into(),
            nullable: true,
            default: None,
            comment: None,
            options: Vec::new(),
        };
        TableStructure {
            kind: ObjectKind::Table,
            row_estimate: None,
            data_bytes: 0,
            index_bytes: 0,
            comment: None,
            columns: columns.iter().map(column).collect(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            triggers: Vec::new(),
            ddl: ddl.into(),
        }
    }

    #[test]
    fn copies_a_table_s_name_select_insert_and_create_as_its_engine_writes_them() {
        let users = at(NodeKind::Table, Engine::Postgres, &["shop", "public", "users"]);
        let users = Node { name: "users".into(), ..users };
        let read = structure(&["id", "name"], "CREATE TABLE users (…);");
        assert_eq!(copied_text(CopyKind::Name, &users, None).as_deref(), Some("users"));
        assert_eq!(
            copied_text(CopyKind::Insert, &users, Some(&read)).as_deref(),
            Some("INSERT INTO \"public\".\"users\" (\"id\", \"name\")\nVALUES (NULL, NULL);")
        );
        assert_eq!(
            copied_text(CopyKind::Select, &users, Some(&read)).as_deref(),
            Some("SELECT\n  \"id\",\n  \"name\"\nFROM \"public\".\"users\";")
        );
        assert_eq!(copied_text(CopyKind::Create, &users, Some(&read)).as_deref(), Some("CREATE TABLE users (…);"));
        assert_eq!(copied_text(CopyKind::Select, &users, None), None, "it waits for the columns");
        let mysql = at(NodeKind::Table, Engine::Mysql, &["shop", "orders"]);
        assert_eq!(
            copied_text(CopyKind::Select, &mysql, Some(&structure(&["id"], ""))).as_deref(),
            Some("SELECT\n  `id`\nFROM `orders`;")
        );
        let collection = at(NodeKind::Table, Engine::Mongodb, &["shop", "orders"]);
        assert_eq!(
            copied_text(CopyKind::Count, &collection, None).as_deref(),
            Some("db.getCollection(\"orders\").countDocuments({})")
        );
        assert!(CopyKind::Insert.needs_structure() && !CopyKind::Find.needs_structure());
    }

    #[test]
    fn offers_the_server_objects_a_connection_creates_under_new_and_drops_where_the_engine_can() {
        use NodeCommand::*;
        let m = Language::En.messages();
        let pg = sql_node(NodeKind::Connection);
        assert_eq!(
            commands(&pg),
            [
                NewQuery,
                NewObject(NewObjectKind::Database),
                NewObject(NewObjectKind::Role),
                NewObject(NewObjectKind::User),
                Refresh,
                CopyName,
                CopyConnectionUrl,
                DuplicateConnection,
                EditConnection,
                DeleteConnection
            ]
        );
        assert_eq!(commands(&node(NodeKind::Connection))[0], NewObject(NewObjectKind::User), "Redis creates ACL users");
        let sqlite = Node { engine: Engine::Sqlite, ..sql_node(NodeKind::Connection) };
        assert!(!commands(&sqlite).iter().any(|command| matches!(command, NewObject(_))), "a file has no server");
        assert!(!commands(&node(NodeKind::Database)).contains(&DropDatabase), "Redis databases can't be dropped");
        assert_eq!(NewObject(NewObjectKind::Role).label(m), "Role");
        assert_eq!(NewObject(NewObjectKind::Role).group(m), Some("New"));
        assert_eq!(DropDatabase.label(m), "Drop database…");
        assert_eq!(DropDatabase.label(Language::Es.messages()), "Eliminar base de datos…");
        assert_eq!(Refresh.group(m), None);
    }

    #[test]
    fn offers_a_postgresql_database_s_export_and_import_before_its_drop_and_names_them() {
        use NodeCommand::*;
        let mysql = at(NodeKind::Database, Engine::Mysql, &["app"]);
        assert!(!commands(&mysql).contains(&ExportDatabase), "only PostgreSQL has pg_dump and psql");
        let m = Language::En.messages();
        assert_eq!(ExportTable.label(m), "Export…");
        assert_eq!(ImportRows.label(m), "Import rows…");
        assert_eq!(ExportDatabase.label(m), "Export database…");
        assert_eq!(ImportSqlFile.label(Language::Es.messages()), "Importar archivo SQL…");
        assert_eq!(ExportTable.icon(), "export");
        assert_eq!(ImportRows.icon(), "upload");
    }
}
