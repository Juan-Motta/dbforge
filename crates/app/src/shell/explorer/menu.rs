//! A node's right-click menu: the commands it offers. Only commands whose
//! behaviour exists are listed; features that open tabs or dialogs add theirs.
//! A connection's own commands follow the web UI's order: a new query first,
//! then copy its URL, duplicate, edit, delete. A table's or view's start
//! with opening its rows, its structure, then a new query, as the web UI's
//! do; a schema's (or a database that is its own schema) offer its ER
//! diagram after the new query. A MongoDB collection's copies also offer its
//! mongosh `find()` and `countDocuments()`. A connection's server objects
//! it can create follow its new query under a "New" heading; a database
//! that can be dropped offers it last. A table's or view's export, and a
//! table's import of a file's rows, follow its new query; a PostgreSQL
//! database's export to a .sql file and import of one come before its drop.

use dbforge_core::connections::model::Engine;

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
    /// Puts the mongosh that reads a collection's first documents on the clipboard.
    CopyFind,
    /// Puts the mongosh that counts a collection's documents on the clipboard.
    CopyCount,
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
            Self::CopyFind => m.copy_find,
            Self::CopyCount => m.copy_count,
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

    /// The heading it is listed under: the server objects a connection creates are under "New".
    pub fn group(self, m: &Messages) -> Option<&'static str> {
        matches!(self, Self::NewObject(_)).then_some(m.admin.new_menu)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::OpenData => "table",
            Self::ViewStructure => "column",
            Self::ShowDiagram => "diagram",
            Self::NewQuery => "code",
            Self::Refresh | Self::RefreshConnection => "refresh",
            Self::CopyName | Self::CopyFind | Self::CopyCount => "copy",
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
        NodeKind::Table | NodeKind::View if node.engine == Engine::Mongodb => {
            vec![Refresh, RefreshConnection, CopyName, CopyFind, CopyCount]
        }
        NodeKind::Table | NodeKind::View => vec![Refresh, RefreshConnection, CopyName],
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
            [OpenData, ViewStructure, NewQuery, ExportTable, ImportRows, Refresh, RefreshConnection, CopyName]
        );
        let cosmos = Node { engine: Engine::Cosmosdb, ..sql_node(NodeKind::Table) };
        assert_eq!(
            commands(&cosmos),
            [OpenData, ViewStructure, NewQuery, ExportTable, ImportRows, Refresh, RefreshConnection, CopyName]
        );
        // A view can't be imported into.
        assert_eq!(
            commands(&sql_node(NodeKind::View)),
            [OpenData, ViewStructure, NewQuery, ExportTable, Refresh, RefreshConnection, CopyName]
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
                CopyName,
                CopyFind,
                CopyCount
            ]
        );
        assert_eq!(CopyFind.label(Language::En.messages()), "Copy find() query");
        assert_eq!(CopyCount.label(Language::Es.messages()), "Copiar consulta countDocuments()");
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
