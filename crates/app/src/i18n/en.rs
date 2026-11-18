use super::Messages;

pub(super) const MESSAGES: Messages = Messages {
    connections: "Connections",
    new_connection: "New connection",
    refresh: "Refresh",
    filter_placeholder: "Filter objects…",
    no_connections: "No connections yet.",
    collapse_all: "Collapse all",
    no_objects: "No objects",
    no_matches: |query| format!("No objects match “{query}”."),
    tables: "Tables",
    views: "Views",
    copy_name: "Copy name",
    refresh_connection: "Refresh connection",
    resize_explorer: "Resize the explorer",
    no_connection: "No connection selected",
    search_everywhere: "Search everywhere…",
    settings: "Settings",
    new_query: "New query",
    empty_before: "Double-click a table to see its rows, or press ",
    empty_after: ".",
    set_aside_title: "Some settings could not be read",
    set_aside_note: "These files were damaged or not in the expected format. DBForge kept them where they are, \
                     renamed, and started without them:",
    set_aside_ok: "OK",
};
