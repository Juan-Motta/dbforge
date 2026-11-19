//! The commands the palette offers, as the former web UI's `App.tsx` lists them: a
//! new query and the ER diagram of what is selected, when there is one,
//! then a new connection, the three themes and the settings. Pure, so the
//! list is tested without a window; the window runs each one.

use dbforge_core::connections::model::{Connection, Engine};

use crate::connections::engine::{is_file, is_relational};
use crate::i18n::PaletteMessages;
use crate::sql::Dialect;
use crate::theme::ThemePreference;

/// What a command does, for the window to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    /// A query console on the selection (a Redis database's key browser).
    NewQuery,
    /// The ER diagram of this schema of the selected connection.
    ShowDiagram {
        database: String,
        schema: String,
    },
    NewConnection,
    Theme(ThemePreference),
    OpenSettings,
}

/// One command as the palette lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub label: &'static str,
    /// A faint note after the label: where it acts.
    pub detail: Option<String>,
    pub action: CommandAction,
}

/// The database a diagram shows when only a connection is selected: a
/// file's own schema (`main`), else the connection's database; `None` when it has none.
fn default_database(connection: &Connection) -> Option<String> {
    let database = if is_file(connection.engine) {
        Dialect::of(connection.engine).and_then(Dialect::default_schema).unwrap_or_default().to_owned()
    } else {
        connection.database.clone()
    };
    Some(database).filter(|database| !database.is_empty())
}

/// The schema a diagram shows when only a database is selected: the
/// engine's default (`public`, `dbo`), or the database itself (MySQL).
fn default_schema(engine: Engine, database: &str) -> String {
    Dialect::of(engine).and_then(Dialect::default_schema).unwrap_or(database).to_owned()
}

/// The diagram of the selected schema, or of the default one when only a
/// connection or a database is selected; `None` for engines without one.
pub fn diagram_target(connection: &Connection, path: &[String]) -> Option<(String, String)> {
    if !is_relational(connection.engine) {
        return None;
    }
    let database = path.first().cloned().or_else(|| default_database(connection))?;
    let schema = path.get(1).cloned().unwrap_or_else(|| default_schema(connection.engine, &database));
    Some((database, schema))
}

/// The palette's commands while `selection` (a connection, and where in it) is selected.
pub fn commands(selection: Option<(&Connection, &[String])>, m: &PaletteMessages) -> Vec<Command> {
    let mut commands = Vec::new();
    if let Some((connection, path)) = selection {
        commands.push(Command {
            label: m.new_query,
            detail: Some(connection.name.clone()),
            action: CommandAction::NewQuery,
        });
        if let Some((database, schema)) = diagram_target(connection, path) {
            let detail = if database == schema { database.clone() } else { format!("{database}.{schema}") };
            commands.push(Command {
                label: m.show_diagram,
                detail: Some(detail),
                action: CommandAction::ShowDiagram { database, schema },
            });
        }
    }
    let plain = |label, action| Command { label, detail: None, action };
    commands.extend([
        plain(m.new_connection, CommandAction::NewConnection),
        plain(m.theme_dark, CommandAction::Theme(ThemePreference::Dark)),
        plain(m.theme_light, CommandAction::Theme(ThemePreference::Light)),
        plain(m.theme_system, CommandAction::Theme(ThemePreference::System)),
        plain(m.open_settings, CommandAction::OpenSettings),
    ]);
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn connection(engine: Engine, database: &str) -> Connection {
        serde_json::from_value(serde_json::json!({
            "id": "c", "name": "local", "engine": engine, "environment": null, "host": "h", "port": 1,
            "database": database, "username": "u",
        }))
        .expect("a connection")
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    fn m() -> &'static PaletteMessages {
        &Language::En.messages().palette
    }

    #[test]
    fn offers_the_app_s_commands_without_a_selection() {
        let labels: Vec<&str> = commands(None, m()).iter().map(|command| command.label).collect();
        assert_eq!(labels, ["New connection", "Theme: Dark", "Theme: Light", "Theme: Match system", "Open settings"]);
    }

    #[test]
    fn offers_a_new_query_and_the_selected_schema_s_diagram_first() {
        let pg = connection(Engine::Postgres, "shop");
        let listed = commands(Some((&pg, &path(&["sales", "billing"]))), m());
        assert_eq!(
            listed[0],
            Command { label: "New query", detail: Some("local".into()), action: CommandAction::NewQuery }
        );
        assert_eq!(
            listed[1],
            Command {
                label: "Show ER diagram",
                detail: Some("sales.billing".into()),
                action: CommandAction::ShowDiagram { database: "sales".into(), schema: "billing".into() }
            }
        );
        assert_eq!(listed.len(), 7);
    }

    #[test]
    fn falls_back_to_the_default_database_and_schema() {
        assert_eq!(diagram_target(&connection(Engine::Postgres, "shop"), &[]), Some(("shop".into(), "public".into())));
        assert_eq!(diagram_target(&connection(Engine::Mysql, "app"), &[]), Some(("app".into(), "app".into())));
        assert_eq!(
            diagram_target(&connection(Engine::Sqlserver, "x"), &path(&["sales"])),
            Some(("sales".into(), "dbo".into()))
        );
        assert_eq!(
            diagram_target(&connection(Engine::Sqlite, "/tmp/a.db"), &[]),
            Some(("main".into(), "main".into())),
            "a file's database is its own schema"
        );
        assert_eq!(diagram_target(&connection(Engine::Postgres, ""), &[]), None, "no database to draw");
        assert_eq!(diagram_target(&connection(Engine::Mongodb, "shop"), &[]), None, "documents have no diagram");
        let mysql = connection(Engine::Mysql, "app");
        assert_eq!(commands(Some((&mysql, &[])), m())[1].detail.as_deref(), Some("app"), "one name when they match");
    }

    #[test]
    fn a_key_browser_still_gets_its_new_query() {
        let redis = connection(Engine::Redis, "0");
        let listed = commands(Some((&redis, &[])), m());
        assert_eq!(listed[0].action, CommandAction::NewQuery);
        assert_eq!(listed[1].action, CommandAction::NewConnection, "and no diagram");
    }
}
