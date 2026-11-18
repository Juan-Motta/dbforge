//! A node's right-click menu: the commands it offers. Only commands whose
//! behaviour exists are listed; features that open tabs or dialogs add theirs.
//! A connection's own commands follow the web UI's order: a new query first,
//! then copy its URL, duplicate, edit, delete.

use super::tree::{Node, NodeKind};
use crate::i18n::Messages;
use crate::workspace::has_sql_console;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCommand {
    /// Opens a query console on the node's database (a table's starts with its first rows).
    NewQuery,
    /// Lists the node's part of the tree again.
    Refresh,
    /// Lists the node's whole connection again, e.g. once a database was dropped.
    RefreshConnection,
    /// Puts the node's name on the clipboard.
    CopyName,
    /// Puts the connection's URL, without its password, on the clipboard.
    CopyConnectionUrl,
    /// Saves a copy of the connection, its secrets included.
    DuplicateConnection,
    /// Opens the connection dialog on it.
    EditConnection,
    /// Asks before deleting it.
    DeleteConnection,
}

impl NodeCommand {
    pub fn label(self, m: &Messages) -> &'static str {
        match self {
            Self::NewQuery => m.new_query,
            Self::Refresh => m.refresh,
            Self::RefreshConnection => m.refresh_connection,
            Self::CopyName => m.copy_name,
            Self::CopyConnectionUrl => m.copy_connection_url,
            Self::DuplicateConnection => m.duplicate_connection,
            Self::EditConnection => m.edit_connection,
            Self::DeleteConnection => m.delete_connection,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::NewQuery => "code",
            Self::Refresh | Self::RefreshConnection => "refresh",
            Self::CopyName => "copy",
            Self::CopyConnectionUrl => "external",
            Self::DuplicateConnection => "duplicate",
            Self::EditConnection => "edit",
            Self::DeleteConnection => "trash",
        }
    }
}

/// The commands `node`'s menu offers, in order. A new query comes first
/// wherever the engine has a SQL console.
pub fn commands(node: &Node) -> Vec<NodeCommand> {
    use NodeCommand::*;
    let query = has_sql_console(node.engine) && node.kind != NodeKind::Folder;
    let mut commands = if query { vec![NewQuery] } else { Vec::new() };
    commands.extend(match node.kind {
        // A connection's own refresh is its whole connection already.
        NodeKind::Connection => {
            vec![Refresh, CopyName, CopyConnectionUrl, DuplicateConnection, EditConnection, DeleteConnection]
        }
        NodeKind::Folder => vec![Refresh, RefreshConnection],
        NodeKind::Database | NodeKind::Schema | NodeKind::Table | NodeKind::View => {
            vec![Refresh, RefreshConnection, CopyName]
        }
    });
    commands
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::Engine;

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
            [Refresh, CopyName, CopyConnectionUrl, DuplicateConnection, EditConnection, DeleteConnection]
        );
        assert_eq!(commands(&node(NodeKind::Folder)), [Refresh, RefreshConnection]);
        for kind in [NodeKind::Database, NodeKind::Schema, NodeKind::Table, NodeKind::View] {
            assert_eq!(commands(&node(kind)), [Refresh, RefreshConnection, CopyName], "{kind:?}");
        }
    }

    #[test]
    fn offers_a_new_query_first_where_the_engine_has_a_sql_console() {
        use NodeCommand::*;
        assert_eq!(commands(&sql_node(NodeKind::Connection))[..2], [NewQuery, Refresh]);
        for kind in [NodeKind::Database, NodeKind::Schema, NodeKind::Table, NodeKind::View] {
            assert_eq!(commands(&sql_node(kind)), [NewQuery, Refresh, RefreshConnection, CopyName], "{kind:?}");
        }
        assert_eq!(commands(&sql_node(NodeKind::Folder)), [Refresh, RefreshConnection]);
        let mongo = Node { engine: Engine::Mongodb, ..sql_node(NodeKind::Database) };
        assert!(!commands(&mongo).contains(&NewQuery), "MongoDB's console comes later");
        assert_eq!(NodeCommand::NewQuery.label(Language::Es.messages()), "Nueva consulta");
    }

    #[test]
    fn names_each_command_in_the_language_in_use() {
        assert_eq!(NodeCommand::CopyName.label(Language::En.messages()), "Copy name");
        assert_eq!(NodeCommand::RefreshConnection.label(Language::Es.messages()), "Actualizar conexión");
        assert_eq!(NodeCommand::DuplicateConnection.label(Language::En.messages()), "Duplicate connection");
        assert_eq!(NodeCommand::DeleteConnection.label(Language::Es.messages()), "Eliminar conexión");
    }
}
