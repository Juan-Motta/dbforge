//! A node's right-click menu: the commands it offers. Only commands whose
//! behaviour exists are listed; features that open tabs or dialogs add theirs.

use super::tree::{Node, NodeKind};
use crate::i18n::Messages;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCommand {
    /// Lists the node's part of the tree again.
    Refresh,
    /// Lists the node's whole connection again, e.g. once a database was dropped.
    RefreshConnection,
    /// Puts the node's name on the clipboard.
    CopyName,
}

impl NodeCommand {
    pub fn label(self, m: &Messages) -> &'static str {
        match self {
            Self::Refresh => m.refresh,
            Self::RefreshConnection => m.refresh_connection,
            Self::CopyName => m.copy_name,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Refresh | Self::RefreshConnection => "refresh",
            Self::CopyName => "copy",
        }
    }
}

/// The commands `node`'s menu offers, in order.
pub fn commands(node: &Node) -> Vec<NodeCommand> {
    use NodeCommand::*;
    match node.kind {
        // A connection's own refresh is its whole connection already.
        NodeKind::Connection => vec![Refresh, CopyName],
        NodeKind::Folder => vec![Refresh, RefreshConnection],
        NodeKind::Database | NodeKind::Schema | NodeKind::Table | NodeKind::View => {
            vec![Refresh, RefreshConnection, CopyName]
        }
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::Engine;

    use super::*;
    use crate::i18n::Language;

    fn node(kind: NodeKind) -> Node {
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
        assert_eq!(commands(&node(NodeKind::Connection)), [Refresh, CopyName]);
        assert_eq!(commands(&node(NodeKind::Folder)), [Refresh, RefreshConnection]);
        for kind in [NodeKind::Database, NodeKind::Schema, NodeKind::Table, NodeKind::View] {
            assert_eq!(commands(&node(kind)), [Refresh, RefreshConnection, CopyName], "{kind:?}");
        }
    }

    #[test]
    fn names_each_command_in_the_language_in_use() {
        assert_eq!(NodeCommand::CopyName.label(Language::En.messages()), "Copy name");
        assert_eq!(NodeCommand::RefreshConnection.label(Language::Es.messages()), "Actualizar conexión");
    }
}
