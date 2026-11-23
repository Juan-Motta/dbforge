//! An agent wants to change data on a connection whose tag asks first, as
//! the former web UI's `ApprovalDialog.tsx`: what it will run, where, and who
//! asks. Or it wants to create, update or delete a saved query: what the
//! query is, or what changes (each field before and after, and the SQL).
//! Decline has the focus, so Enter or Space pressed by mistake can't
//! approve; Tab moves between the two buttons, and Enter or Space presses
//! the one focused. Escape and the close button decline. Approve and run is
//! the only way an agent's change runs from here. Every backup and restore an
//! agent asks for shows here too: the database, the connection, the export's
//! file and the options, and for a restore a warning that it changes data.
//! So does every change an agent asks to make to the saved connections: the
//! connection's settings (before and after, for an edit), and for a create or
//! an edit the secrets the connection signs in with, for the user to type
//! (masked, and on an edit empty keeps the saved ones), plus a new
//! connection's agent access. What is typed goes with the answer to the core's
//! waiting tool only; the agent never sees it. Enter and Space press a button
//! only while one has the keyboard, so typing a secret never answers.

use dbforge_core::connections::model::{AgentAccess, Connection, Engine};
use dbforge_core::connections::transport::{SshAuth, SslMode};
use dbforge_core::dump::{Content, DumpOptions, SqliteFormat};
use dbforge_core::mcp::{
    ApprovalRequest, ApprovalSecrets, ApprovalSubject, ConnectionChange, ConnectionField, DraftScope, DumpAction,
    DumpApproval, SavedQueryChange, SavedQueryDraft, SavedQueryField,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, actions, div, px,
};

use super::Approvals;
use crate::i18n::{Messages, Translate};
use crate::sql::Syntax;
use crate::theme::{ActiveTheme, Theme};
use crate::ui::code_editor::LINE_HEIGHT;
use crate::ui::field::{Tooltip, label as field_label, text_box};
use crate::ui::{
    CodeEditor, TextInput, TextInputEvent, button, enabled, glass, icon, icon_button, pill, primary_button,
    segment_face, segmented,
};

actions!(approval_dialog, [Decline, Press, SwitchButton, SwitchBack]);

const CONTEXT: &str = "ApprovalDialog";
/// The buttons' row: Enter and Space press the focused button only there,
/// so a space typed in a secret's field is just a space.
const BUTTONS: &str = "ApprovalButtons";

/// The statement's box grows with it up to this height, then scrolls.
const STATEMENT_MAX_HEIGHT: f32 = 280.;

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("escape", Decline, context),
        KeyBinding::new("enter", Press, Some(BUTTONS)),
        KeyBinding::new("space", Press, Some(BUTTONS)),
        KeyBinding::new("tab", SwitchButton, context),
        KeyBinding::new("shift-tab", SwitchBack, context),
    ]
}

/// What the user types approving a connection's change; nothing for other requests.
#[derive(Default)]
struct ConnectionInputs {
    /// Its password (an account's key), unless it has none (a SQLite file).
    password: Option<Entity<TextInput>>,
    /// Its SSH password or key passphrase, when its tunnel signs in with one.
    ssh_secret: Option<Entity<TextInput>>,
    /// A new connection's agent access: Read only until the user changes it.
    access: Option<AgentAccess>,
    /// The fields Approve waits for: an edit pointing the connection
    /// elsewhere carries no saved secret there, so it must be typed again.
    password_required: bool,
    ssh_required: bool,
}

impl ConnectionInputs {
    fn fields(&self) -> impl Iterator<Item = &Entity<TextInput>> {
        [&self.password, &self.ssh_secret].into_iter().flatten()
    }

    /// Whether every field that must be typed is.
    fn complete(&self, cx: &App) -> bool {
        let typed =
            |field: &Option<Entity<TextInput>>| field.as_ref().is_some_and(|field| !field.read(cx).text().is_empty());
        (!self.password_required || typed(&self.password)) && (!self.ssh_required || typed(&self.ssh_secret))
    }
}

pub struct ApprovalDialog {
    request: ApprovalRequest,
    /// The statement; for a saved query, its SQL (the new SQL of an update).
    statement: Entity<CodeEditor>,
    /// An update's SQL as it is now, when the update changes it.
    statement_before: Option<Entity<CodeEditor>>,
    decline: FocusHandle,
    approve: FocusHandle,
    inputs: ConnectionInputs,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ApprovalDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.decline.clone()
    }
}

impl ApprovalDialog {
    /// `request` on a connection of `engine` (its statement highlighted in
    /// that engine's language; PostgreSQL's when the connection is gone). A
    /// saved query's SQL is highlighted in the language it was written for.
    pub fn new(request: ApprovalRequest, engine: Option<Engine>, cx: &mut Context<Self>) -> Self {
        let (engine, text, before) = match &request.subject {
            ApprovalSubject::Statement { statement, .. } => (engine.unwrap_or(Engine::Postgres), statement, None),
            // No statement: the dialog lists the backup's or restore's details instead.
            ApprovalSubject::Dump(dump) => (dump.engine, &String::new(), None),
            // No statement either: the connection's settings are listed.
            ApprovalSubject::Connection(change) => (change.connection().engine, &String::new(), None),
            ApprovalSubject::SavedQuery(change) => match change.as_ref() {
                SavedQueryChange::Create { query } | SavedQueryChange::Delete { query } => {
                    (query.engine, &query.sql, None)
                }
                SavedQueryChange::Update { before, after } => {
                    (after.engine, &after.sql, change.changes_sql().then_some(before))
                }
            },
        };
        let statement = read_only_editor(text, engine, cx);
        let statement_before = before.map(|before| read_only_editor(&before.sql, before.engine, cx));
        let (inputs, subscriptions) = match &request.subject {
            ApprovalSubject::Connection(change) => connection_inputs(change, cx),
            _ => (ConnectionInputs::default(), Vec::new()),
        };
        Self {
            request,
            statement,
            statement_before,
            decline: cx.focus_handle(),
            approve: cx.focus_handle(),
            inputs,
            _subscriptions: subscriptions,
        }
    }

    pub fn id(&self) -> &str {
        &self.request.id
    }

    /// Gives Decline the keyboard.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.decline);
    }

    /// Answers, with what the user typed when approving a connection's
    /// change; the fields are emptied either way, so nothing typed stays.
    fn answer(&self, approved: bool, cx: &mut Context<Self>) {
        let id = self.request.id.clone();
        let secrets = approved.then(|| self.typed(cx)).flatten();
        for field in self.inputs.fields() {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        Approvals::global(cx).update(cx, |approvals, cx| approvals.answer(&id, approved, secrets, cx));
    }

    /// What the user typed for a connection's change; nothing for any other request.
    fn typed(&self, cx: &App) -> Option<ApprovalSecrets> {
        let ApprovalSubject::Connection(_) = &self.request.subject else { return None };
        let text = |field: &Option<Entity<TextInput>>| {
            field.as_ref().map(|field| field.read(cx).text().to_owned()).filter(|text| !text.is_empty())
        };
        Some(ApprovalSecrets {
            password: text(&self.inputs.password),
            ssh_secret: text(&self.inputs.ssh_secret),
            agent_access: self.inputs.access,
        })
    }

    fn set_access(&mut self, access: AgentAccess, cx: &mut Context<Self>) {
        if self.inputs.access.is_some() {
            self.inputs.access = Some(access);
            cx.notify();
        }
    }

    pub fn decline(&mut self, cx: &mut Context<Self>) {
        self.answer(false, cx);
    }

    /// Runs the agent's change: Approve and run, pressed. Does nothing while a
    /// secret that must be typed isn't.
    pub fn approve(&mut self, cx: &mut Context<Self>) {
        if self.inputs.complete(cx) {
            self.answer(true, cx);
        }
    }

    /// Enter or Space: the focused button. Approves only while Approve has
    /// the keyboard; anything else declines, as Decline has it by default.
    fn press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.approve.is_focused(window) {
            true => self.approve(cx),
            false => self.answer(false, cx),
        }
    }

    /// Tab (`forward`) or Shift-Tab: the next or previous of the secrets'
    /// fields and the two buttons, round again at the ends.
    fn switch_button(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut order: Vec<FocusHandle> = self.inputs.fields().map(|field| field.focus_handle(cx)).collect();
        order.extend([self.decline.clone(), self.approve.clone()]);
        let count = order.len();
        // Nothing of the dialog's focused: as if Decline were, where the keyboard starts.
        let at = order.iter().position(|handle| handle.is_focused(window)).unwrap_or(count - 2);
        let next = if forward { (at + 1) % count } else { (at + count - 1) % count };
        window.focus(&order[next]);
        cx.notify();
    }

    fn render_who(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &cx.messages().mcp;
        let agent = self.request.agent.as_deref().unwrap_or(m.an_agent);
        let (who, tag) = match &self.request.subject {
            ApprovalSubject::Statement { connection_name, tag, .. } => {
                ((m.approval_who)(agent, connection_name), tag.as_deref())
            }
            ApprovalSubject::SavedQuery(change) => {
                let who = match change.as_ref() {
                    SavedQueryChange::Create { .. } => m.saved_create_who,
                    SavedQueryChange::Update { .. } => m.saved_update_who,
                    SavedQueryChange::Delete { .. } => m.saved_delete_who,
                };
                (who(agent, change.name()), None)
            }
            ApprovalSubject::Connection(change) => {
                let who = match change.as_ref() {
                    ConnectionChange::Create { .. } => m.conn_create_who,
                    ConnectionChange::Update { .. } => m.conn_update_who,
                    ConnectionChange::Delete { .. } => m.conn_delete_who,
                };
                (who(agent, &change.connection().name), change.tag())
            }
            ApprovalSubject::Dump(dump) => {
                let who = match &dump.action {
                    DumpAction::Export { .. } => (m.dump_export_who)(agent, &dump.database, &dump.connection_name),
                    DumpAction::Import { .. } => {
                        (m.dump_import_who)(agent, &dump.file, &dump.database, &dump.connection_name)
                    }
                };
                (who, dump.tag.as_deref())
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(icon("warning", theme.warn, 15.))
            .child(div().min_w_0().child(who))
            .children(tag.map(|tag| pill(tag, theme.err).flex_none()))
    }

    /// Where a statement runs, or what a saved query is or what changes in it.
    fn render_details(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let messages = cx.messages();
        let small = || div().text_size(px(12.)).text_color(theme.text3);
        match &self.request.subject {
            ApprovalSubject::Connection(change) => {
                let rows = connection_rows(change, messages).into_iter().map(|row| {
                    let value = match row.before {
                        Some(before) => format!("{before}  →  {}", row.value),
                        None => row.value,
                    };
                    div()
                        .debug_selector(move || row.selector.to_owned())
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().w(px(130.)).flex_none().text_color(theme.text3).child(row.label))
                        .child(div().min_w_0().flex_1().child(value))
                });
                div().flex().flex_col().gap(px(4.)).children(rows)
            }
            ApprovalSubject::Dump(dump) => {
                let row = |selector: &'static str, label: &'static str, value: String| {
                    div()
                        .debug_selector(move || selector.to_owned())
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().w(px(110.)).flex_none().text_color(theme.text3).child(label))
                        .child(div().min_w_0().flex_1().child(value))
                };
                let m = &messages.mcp;
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(row("approval-dump-file", m.dump_file, (m.dump_in_exports_folder)(&dump.file)))
                            .children(
                                dump_options(&dump.action, messages)
                                    .map(|options| row("approval-dump-options", m.dump_options, options)),
                            ),
                    )
                    .children(restore_warning(dump, messages).map(|warning| {
                        div()
                            .debug_selector(|| "approval-dump-warning".into())
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .text_color(theme.warn)
                            .child(icon("warning", theme.warn, 13.))
                            .child(div().min_w_0().flex_1().child(warning))
                    }))
            }
            ApprovalSubject::Statement { database, .. } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(database.as_deref().map(|database| small().child((messages.mcp.in_database)(database))))
                .child(sql_box(&self.statement, theme, cx)),
            ApprovalSubject::SavedQuery(change) => {
                let rows = detail_rows(change, messages).into_iter().map(|row| {
                    let value = match row.before {
                        Some(before) => format!("{before}  →  {}", row.value),
                        None => row.value,
                    };
                    div()
                        .debug_selector(move || row.selector.to_owned())
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().w(px(110.)).flex_none().text_color(theme.text3).child(row.label))
                        .child(div().min_w_0().flex_1().child(value))
                });
                let sql = match (change.as_ref(), &self.statement_before) {
                    (SavedQueryChange::Update { .. }, Some(before)) => div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(small().child(messages.mcp.sql_before))
                        .child(sql_box(before, theme, cx).debug_selector(|| "approval-sql-before".into()))
                        .child(small().child(messages.mcp.sql_after))
                        .child(sql_box(&self.statement, theme, cx).debug_selector(|| "approval-sql-after".into())),
                    (SavedQueryChange::Update { .. }, None) => small().child(messages.mcp.sql_unchanged),
                    _ => div().child(sql_box(&self.statement, theme, cx)),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().flex_col().gap(px(4.)).children(rows))
                    .child(sql)
            }
        }
    }
}

impl ApprovalDialog {
    /// A connection's secrets to type and a new one's agent access; nothing for other requests.
    fn render_inputs(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let ApprovalSubject::Connection(change) = &self.request.subject else { return None };
        let c = &cx.messages().connection;
        let secrets = change.secrets();
        let password_label =
            if secrets.password && change.connection().engine == Engine::Cosmosdb { c.key } else { c.password };
        let ssh_label = if secrets.ssh == Some(SshAuth::Key) { c.ssh_passphrase } else { c.ssh_password };
        let secret = |selector: &'static str, title: &'static str, field: &Option<Entity<TextInput>>| {
            field.as_ref().map(|field| {
                div()
                    .debug_selector(move || selector.to_owned())
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .child(field_label(theme, selector, title, None))
                    .child(text_box(field, false, false, window, cx))
            })
        };
        let access = self.inputs.access.map(|access| {
            let choice = |value: AgentAccess, id: &'static str, text: &'static str| {
                segment_face(theme, None, text, access == value)
                    .debug_selector(move || id.to_owned())
                    .id(id)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_access(value, cx)))
                    .into_any_element()
            };
            let selected = match access {
                AgentAccess::None => 0,
                AgentAccess::Read => 1,
                AgentAccess::Write => 2,
            };
            div()
                .debug_selector(|| "approval-access".into())
                .flex()
                .flex_col()
                .child(field_label(theme, "approval-access-label", c.agent_access, Some(c.agent_access_info)))
                .child(div().flex().child(segmented(
                    theme,
                    "approval-access-choice",
                    Some(selected),
                    vec![
                        choice(AgentAccess::None, "approval-access-none", c.agent_none),
                        choice(AgentAccess::Read, "approval-access-read", c.agent_read),
                        choice(AgentAccess::Write, "approval-access-write", c.agent_write),
                    ],
                )))
        });
        let fields = [
            secret("approval-secret-password", password_label, &self.inputs.password),
            secret("approval-secret-ssh", ssh_label, &self.inputs.ssh_secret),
        ];
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().flex().gap(px(12.)).children(fields.into_iter().flatten()))
                .children(access),
        )
    }
}

/// The fields a connection's change asks the user for, and what the dialog hears from them.
fn connection_inputs(
    change: &ConnectionChange,
    cx: &mut Context<ApprovalDialog>,
) -> (ConnectionInputs, Vec<Subscription>) {
    let m = &cx.messages().mcp;
    let secrets = change.secrets();
    // Empty keeps the saved one on an edit, unless the edit drops it.
    let placeholder = |required: bool, dropped: bool| match change {
        _ if required => m.conn_retype_secret,
        ConnectionChange::Update { .. } if !dropped => m.conn_keep_secret,
        _ => m.conn_optional,
    };
    let mut subscriptions = Vec::new();
    let mut field = |wanted: bool, placeholder: &'static str, cx: &mut Context<ApprovalDialog>| {
        wanted.then(|| {
            let field = cx.new(|cx| TextInput::new(placeholder, cx).masked());
            subscriptions.push(cx.subscribe(&field, |this, _, event: &TextInputEvent, cx| match event {
                // Escape declines, as it does anywhere in the dialog.
                TextInputEvent::Cancelled => this.decline(cx),
                // Enter in a field approves nothing: only the Approve button does.
                _ => cx.notify(),
            }));
            field
        })
    };
    let password_placeholder = placeholder(secrets.password_required(), secrets.password_dropped);
    let password = field(secrets.password, password_placeholder, cx);
    let ssh_placeholder = placeholder(secrets.ssh_required(), secrets.ssh_dropped);
    let ssh_secret = field(secrets.ssh.is_some(), ssh_placeholder, cx);
    let access = matches!(change, ConnectionChange::Create { .. }).then_some(AgentAccess::Read);
    let (password_required, ssh_required) = (secrets.password_required(), secrets.ssh_required());
    (ConnectionInputs { password, ssh_secret, access, password_required, ssh_required }, subscriptions)
}

/// The fields a new or deleted connection lists: where it is and how it is reached.
fn listed_fields(connection: &Connection) -> Vec<ConnectionField> {
    use ConnectionField as F;
    let mut fields = vec![F::Engine];
    if connection.engine == Engine::Sqlite {
        fields.push(F::Database);
        return fields;
    }
    fields.push(F::Host);
    if !connection.srv {
        fields.push(F::Port);
    }
    if !connection.database.is_empty() {
        fields.push(F::Database);
    }
    if !connection.username.is_empty() {
        fields.push(F::Username);
    }
    if connection.srv {
        fields.push(F::Srv);
    }
    if connection.engine.ssl_support().modes.len() > 1 {
        fields.push(F::SslMode);
    }
    let ssl = &connection.ssl;
    for (field, path) in [(F::CaFile, &ssl.ca_file), (F::CertFile, &ssl.cert_file), (F::KeyFile, &ssl.key_file)] {
        if !path.is_empty() {
            fields.push(field);
        }
    }
    if connection.ssh.enabled {
        fields.extend([F::Ssh, F::SshHost, F::SshPort, F::SshUser, F::SshAuth]);
        if connection.ssh.auth == SshAuth::Key {
            fields.push(F::SshKeyFile);
        }
    }
    fields
}

/// What the dialog lists about a connection's change: a new or deleted
/// connection's settings; the fields an edit changes, before and after.
fn connection_rows(change: &ConnectionChange, m: &Messages) -> Vec<DetailRow> {
    let row = |field, connection: &Connection, before: Option<&Connection>| DetailRow {
        selector: connection_selector(field),
        label: connection_label(field, connection, m),
        before: before.map(|before| connection_value(field, before, m)),
        value: connection_value(field, connection, m),
    };
    match change {
        ConnectionChange::Create { connection } | ConnectionChange::Delete { connection, .. } => {
            listed_fields(connection).into_iter().map(|field| row(field, connection, None)).collect()
        }
        ConnectionChange::Update { before, after, .. } => {
            change.changed_fields().into_iter().map(|field| row(field, after, Some(before))).collect()
        }
    }
}

fn connection_selector(field: ConnectionField) -> &'static str {
    use ConnectionField as F;
    match field {
        F::Name => "approval-conn-name",
        F::Engine => "approval-conn-engine",
        F::Host => "approval-conn-host",
        F::Port => "approval-conn-port",
        F::Database => "approval-conn-database",
        F::Username => "approval-conn-username",
        F::Srv => "approval-conn-srv",
        F::SslMode => "approval-conn-ssl-mode",
        F::CaFile => "approval-conn-ca-file",
        F::CertFile => "approval-conn-cert-file",
        F::KeyFile => "approval-conn-key-file",
        F::Ssh => "approval-conn-ssh",
        F::SshHost => "approval-conn-ssh-host",
        F::SshPort => "approval-conn-ssh-port",
        F::SshUser => "approval-conn-ssh-user",
        F::SshAuth => "approval-conn-ssh-auth",
        F::SshKeyFile => "approval-conn-ssh-key-file",
    }
}

fn connection_label(field: ConnectionField, connection: &Connection, m: &Messages) -> &'static str {
    use ConnectionField as F;
    let c = &m.connection;
    match field {
        F::Name => c.name,
        F::Engine => c.engine,
        F::Host => c.host,
        F::Port => c.port,
        F::Database if connection.engine == Engine::Sqlite => c.database_file,
        F::Database => c.database,
        F::Username => c.user,
        F::Srv => c.srv,
        F::SslMode => c.ssl_mode,
        F::CaFile => c.ca_file,
        F::CertFile => c.client_cert,
        F::KeyFile => c.client_key,
        F::Ssh => c.use_ssh,
        F::SshHost => c.ssh_host,
        F::SshPort => m.mcp.conn_ssh_port,
        F::SshUser => c.ssh_user,
        F::SshAuth => c.ssh_auth,
        F::SshKeyFile => c.ssh_key_file,
    }
}

fn connection_value(field: ConnectionField, connection: &Connection, m: &Messages) -> String {
    use ConnectionField as F;
    let c = &m.connection;
    let text = |value: &str| if value.is_empty() { "—".to_owned() } else { value.to_owned() };
    let file = |path: &str| if path.is_empty() { c.no_file.to_owned() } else { path.to_owned() };
    let yes_no = |on: bool| if on { m.mcp.conn_yes } else { m.mcp.conn_no }.to_owned();
    let (ssl, ssh) = (&connection.ssl, &connection.ssh);
    match field {
        F::Name => connection.name.clone(),
        F::Engine => connection.engine.label().to_owned(),
        F::Host => text(&connection.host),
        F::Port => connection.port.to_string(),
        F::Database => text(&connection.database),
        F::Username => text(&connection.username),
        F::Srv => yes_no(connection.srv),
        F::SslMode => match ssl.mode {
            SslMode::Disable => c.ssl_disable,
            SslMode::Prefer => c.ssl_prefer,
            SslMode::Require => c.ssl_require,
            SslMode::VerifyCa => c.ssl_verify_ca,
            SslMode::VerifyFull => c.ssl_verify_full,
        }
        .to_owned(),
        F::CaFile => file(&ssl.ca_file),
        F::CertFile => file(&ssl.cert_file),
        F::KeyFile => file(&ssl.key_file),
        F::Ssh => yes_no(ssh.enabled),
        F::SshHost => text(&ssh.host),
        F::SshPort => ssh.port.to_string(),
        F::SshUser => text(&ssh.username),
        F::SshAuth => match ssh.auth {
            SshAuth::Password => c.ssh_auth_password,
            SshAuth::Key => c.ssh_auth_key,
            SshAuth::Agent => c.ssh_auth_agent,
        }
        .to_owned(),
        F::SshKeyFile => file(&ssh.key_file),
    }
}

/// A backup's options or a restore's, in a line; `None` when a restore has none.
fn dump_options(action: &DumpAction, m: &Messages) -> Option<String> {
    let d = &m.dump;
    let content = |content: Content| match content {
        Content::All => d.all_label,
        Content::Structure => d.structure_label,
        Content::Data => d.data_label,
    };
    let parts: Vec<String> = match action {
        DumpAction::Export { options } => match options {
            DumpOptions::Postgres(options) => [
                Some(content(options.content)),
                options.clean.then_some(d.clean),
                options.keep_owners.then_some(d.keep_owners),
            ]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect(),
            DumpOptions::Mysql(options) => [
                Some(content(options.content)),
                options.add_drop_table.then_some(d.add_drop_table),
                options.single_transaction.then_some(d.single_transaction),
                options.routines.then_some(d.routines),
            ]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect(),
            DumpOptions::Mongodb(options) if options.collections.is_empty() => vec![d.whole_database.to_owned()],
            DumpOptions::Mongodb(options) => vec![(m.mcp.dump_collections)(&options.collections.join(", "))],
            DumpOptions::Sqlite(options) => vec![
                match options.format {
                    SqliteFormat::Script => d.sqlite_script,
                    SqliteFormat::File => d.sqlite_file,
                }
                .to_owned(),
            ],
        },
        DumpAction::Import { drop_existing, .. } => {
            drop_existing.then(|| d.drop_existing.to_owned()).into_iter().collect()
        }
    };
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What a restore warns of: it changes data, and objects already there may be replaced.
fn restore_warning(dump: &DumpApproval, m: &Messages) -> Option<String> {
    let DumpAction::Import { target_exists, .. } = &dump.action else { return None };
    let warning = match target_exists {
        Some(true) => m.mcp.dump_restore_existing,
        Some(false) => m.mcp.dump_restore_new,
        None => m.mcp.dump_restore_unknown,
    };
    Some(warning(&dump.database))
}

/// A read-only editor showing `text` in `engine`'s language.
fn read_only_editor(text: &str, engine: Engine, cx: &mut Context<ApprovalDialog>) -> Entity<CodeEditor> {
    cx.new(|cx| {
        let mut editor = CodeEditor::new("", Syntax::console(engine), cx);
        editor.show_text(text, cx);
        editor.set_read_only(true, cx);
        editor
    })
}

/// `editor`'s box, growing with its text up to [`STATEMENT_MAX_HEIGHT`], then scrolling.
fn sql_box(editor: &Entity<CodeEditor>, theme: &Theme, cx: &App) -> gpui::Div {
    div()
        .h(px(sql_height(editor.read(cx).text())))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.line)
        .overflow_hidden()
        .flex()
        .child(editor.clone())
}

fn sql_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1) as f32;
    (lines * LINE_HEIGHT + 2. * crate::ui::code_editor::PADDING_TOP).min(STATEMENT_MAX_HEIGHT)
}

/// A line of a saved query's details: a field, and its value (before and after, for an update).
#[derive(Debug, PartialEq)]
struct DetailRow {
    /// What tests find it by.
    selector: &'static str,
    label: &'static str,
    before: Option<String>,
    value: String,
}

/// What the dialog lists about `change`: the new or deleted query's scope,
/// folder and description; the fields an update changes, before and after.
fn detail_rows(change: &SavedQueryChange, m: &Messages) -> Vec<DetailRow> {
    let row = |field, draft: &SavedQueryDraft, before: Option<&SavedQueryDraft>| DetailRow {
        selector: selector(field),
        label: label(field, m),
        before: before.map(|before| field_value(field, before, m)),
        value: field_value(field, draft, m),
    };
    match change {
        SavedQueryChange::Create { query } | SavedQueryChange::Delete { query } => {
            [SavedQueryField::Scope, SavedQueryField::Folder, SavedQueryField::Description]
                .into_iter()
                .filter(|field| *field != SavedQueryField::Description || query.description.is_some())
                .map(|field| row(field, query, None))
                .collect()
        }
        SavedQueryChange::Update { before, after } => {
            change.changed_fields().into_iter().map(|field| row(field, after, Some(before))).collect()
        }
    }
}

fn selector(field: SavedQueryField) -> &'static str {
    match field {
        SavedQueryField::Name => "approval-field-name",
        SavedQueryField::Folder => "approval-field-folder",
        SavedQueryField::Scope => "approval-field-scope",
        SavedQueryField::Description => "approval-field-description",
    }
}

fn label(field: SavedQueryField, m: &Messages) -> &'static str {
    match field {
        SavedQueryField::Name => m.saved.name,
        SavedQueryField::Folder => m.saved.folder,
        SavedQueryField::Scope => m.saved.available_on,
        SavedQueryField::Description => m.saved.description,
    }
}

fn field_value(field: SavedQueryField, draft: &SavedQueryDraft, m: &Messages) -> String {
    match field {
        SavedQueryField::Name => draft.name.clone(),
        SavedQueryField::Folder => draft.folder.clone().unwrap_or_else(|| m.saved.no_folder.to_owned()),
        SavedQueryField::Description => draft.description.clone().unwrap_or_else(|| m.mcp.no_description.to_owned()),
        SavedQueryField::Scope => match &draft.scope {
            DraftScope::Connection { connection_name, .. } => connection_name.clone(),
            DraftScope::Engine => (m.saved.any_engine_connection)(draft.engine.label()),
        },
    }
}

/// The ring around a button that shows it has the keyboard; its room is
/// kept while it doesn't, so moving the focus moves nothing.
fn focus_ring(button: impl IntoElement, theme: &Theme, focused: bool) -> gpui::Div {
    div()
        .p(px(1.))
        .rounded(px(19.))
        .border_2()
        .border_color(if focused { theme.accent } else { gpui::transparent_black() })
        .child(button)
}

impl Render for ApprovalDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().mcp;
        let (title, note, approve) = match &self.request.subject {
            ApprovalSubject::SavedQuery(_) => (m.saved_approval_title, m.saved_approval_note, m.approve_change),
            ApprovalSubject::Connection(change) => match change.as_ref() {
                ConnectionChange::Delete { .. } => (m.conn_approval_title, m.conn_delete_note, m.approve_change),
                _ => (m.conn_approval_title, m.conn_approval_note, m.approve_change),
            },
            ApprovalSubject::Dump(dump) => match dump.action {
                DumpAction::Export { .. } => (m.dump_export_title, m.dump_export_note, m.approve_change),
                DumpAction::Import { .. } => (m.dump_import_title, m.dump_import_note, m.approve_change),
            },
            ApprovalSubject::Statement { .. } => (m.approval_title, m.approval_note, m.approve),
        };
        let decline_focused = self.decline.is_focused(window);
        let approve_focused = self.approve.is_focused(window);
        let ready = self.inputs.complete(cx);
        glass(&theme)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Decline, _, cx| this.decline(cx)))
            .on_action(cx.listener(|this, _: &Press, window, cx| this.press(window, cx)))
            .on_action(cx.listener(|this, _: &SwitchButton, window, cx| this.switch_button(true, window, cx)))
            .on_action(cx.listener(|this, _: &SwitchBack, window, cx| this.switch_button(false, window, cx)))
            .bg(theme.dialog)
            .w(px(640.))
            .max_w_full()
            .rounded(px(22.))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(22.))
                    .pt(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        icon_button(&theme, "approval-close", "close", true, false)
                            .tooltip(Tooltip::text(m.close))
                            .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                    ),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(14.))
                    .pb(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(self.render_who(&theme, cx))
                    .child(self.render_details(&theme, cx))
                    .children(self.render_inputs(&theme, window, cx))
                    .child(div().text_size(px(12.)).text_color(theme.text3).child(note)),
            )
            .child(
                div()
                    .key_context(BUTTONS)
                    .px(px(22.))
                    .pt(px(12.))
                    .pb(px(18.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(focus_ring(
                        button(&theme, None, m.decline)
                            .id("approval-decline")
                            .track_focus(&self.decline)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.decline(cx))),
                        &theme,
                        decline_focused,
                    ))
                    .child(focus_ring(
                        enabled(primary_button(&theme, approve), ready)
                            .id("approval-approve")
                            .track_focus(&self.approve)
                            .when(ready, |approve| approve.on_click(cx.listener(|this, _, _, cx| this.approve(cx)))),
                        &theme,
                        approve_focused,
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{TestAppContext, VisualTestContext};
    use tokio::task::JoinHandle;

    use super::*;
    use crate::backend::{Backend, runtime};
    use crate::events::{ChannelEvents, EventHub, channel};
    use crate::test_support::{draw, setup};

    fn request(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            agent: None,
            subject: ApprovalSubject::Statement {
                connection_id: "gone".into(),
                connection_name: "Shop".into(),
                tag: Some("PROD".into()),
                database: Some("shop".into()),
                statement: "DELETE FROM orders\nWHERE id = 1".into(),
            },
        }
    }

    /// The core asks, as the MCP server does for an agent's change, and the
    /// dialog shows the request with Decline focused. The task ends with the answer the agent gets.
    fn asked(cx: &mut TestAppContext) -> (JoinHandle<bool>, Entity<ApprovalDialog>, &mut VisualTestContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let events: Arc<ChannelEvents> = cx.update(|cx| {
            let (events, received) = channel();
            EventHub::start(received, cx);
            Approvals::start(cx);
            Arc::new(events)
        });
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        let asking = runtime().spawn(async move { core.mcp_approvals().ask(events.as_ref(), request("a")).await });
        let shown = loop {
            cx.run_until_parked();
            if let Some(shown) = cx.update(|cx| Approvals::global(cx).read(cx).current().cloned()) {
                break shown;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(shown, None, cx));
        dialog.update_in(cx, |dialog, window, _| dialog.focus(window));
        draw(cx);
        (asking, dialog, cx)
    }

    fn answer(asking: JoinHandle<bool>, cx: &mut VisualTestContext) -> bool {
        cx.run_until_parked();
        assert!(cx.update(|_, cx| Approvals::global(cx).read(cx).current().is_none()), "answered requests leave");
        runtime().block_on(asking).expect("the request's answer")
    }

    #[gpui::test]
    fn enter_pressed_by_mistake_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        assert!(!asking.is_finished(), "nothing runs before the user answers");
        cx.simulate_keystrokes("enter");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn space_on_decline_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        cx.simulate_keystrokes("space");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn escape_declines(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, _dialog, cx) = asked(cx);
        cx.simulate_keystrokes("escape");
        assert!(!answer(asking, cx));
    }

    #[gpui::test]
    fn enter_approves_only_once_tab_moved_the_keyboard_to_approve(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (asking, dialog, cx) = asked(cx);
        cx.simulate_keystrokes("tab");
        assert!(dialog.update_in(cx, |dialog, window, _| dialog.approve.is_focused(window)));
        cx.simulate_keystrokes("tab");
        assert!(dialog.update_in(cx, |dialog, window, _| dialog.decline.is_focused(window)), "back to Decline");
        cx.simulate_keystrokes("shift-tab enter");
        assert!(answer(asking, cx));
    }

    fn draft(name: &str, sql: &str) -> SavedQueryDraft {
        SavedQueryDraft {
            name: name.into(),
            sql: sql.into(),
            description: Some("By status".into()),
            folder: Some("Ops".into()),
            scope: DraftScope::Connection { connection_id: "shop".into(), connection_name: "Shop".into() },
            engine: Engine::Postgres,
        }
    }

    fn update() -> SavedQueryChange {
        let before = draft("Orders", "select 1");
        let after = SavedQueryDraft {
            name: "Paid orders".into(),
            sql: "select *\nfrom orders\nwhere status = :status".into(),
            folder: None,
            scope: DraftScope::Engine,
            ..before.clone()
        };
        SavedQueryChange::Update { before, after }
    }

    #[test]
    fn an_update_lists_each_changed_field_before_and_after() {
        let m = crate::i18n::Language::En.messages();
        let rows = detail_rows(&update(), m);
        let shown: Vec<(&str, Option<&str>, &str)> =
            rows.iter().map(|row| (row.label, row.before.as_deref(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("Name", Some("Orders"), "Paid orders"),
                ("Folder", Some("Ops"), "No folder"),
                ("Available on", Some("Shop"), "Any PostgreSQL connection"),
            ]
        );
    }

    #[test]
    fn a_new_or_deleted_query_lists_where_it_is_and_what_it_says() {
        let m = crate::i18n::Language::En.messages();
        let rows = detail_rows(&SavedQueryChange::Create { query: draft("Orders", "select 1") }, m);
        let shown: Vec<(&str, Option<&str>, &str)> =
            rows.iter().map(|row| (row.label, row.before.as_deref(), row.value.as_str())).collect();
        assert_eq!(
            shown,
            [("Available on", None, "Shop"), ("Folder", None, "Ops"), ("Description", None, "By status")]
        );
        let plain = SavedQueryDraft { description: None, folder: None, ..draft("Orders", "select 1") };
        let rows = detail_rows(&SavedQueryChange::Delete { query: plain }, m);
        let labels: Vec<&str> = rows.iter().map(|row| row.label).collect();
        assert_eq!(labels, ["Available on", "Folder"], "no description, no line for it");
    }

    #[gpui::test]
    fn shows_a_saved_query_update_with_its_sql_before_and_after(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let request = ApprovalRequest {
            id: "a".into(),
            agent: Some("codex".into()),
            subject: ApprovalSubject::SavedQuery(Box::new(update())),
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(request, None, cx));
        draw(cx);

        for shown in ["approval-field-name", "approval-field-folder", "approval-field-scope"] {
            assert!(cx.debug_bounds(shown).is_some(), "{shown} shows");
        }
        assert!(cx.debug_bounds("approval-field-description").is_none(), "an unchanged field isn't listed");
        assert!(cx.debug_bounds("approval-sql-before").is_some() && cx.debug_bounds("approval-sql-after").is_some());
        dialog.update(cx, |dialog, cx| {
            assert_eq!(dialog.statement.read(cx).text(), "select *\nfrom orders\nwhere status = :status");
            let before = dialog.statement_before.as_ref().expect("the SQL before");
            assert_eq!(before.read(cx).text(), "select 1");
        });
    }

    #[gpui::test]
    fn an_update_that_keeps_the_sql_shows_it_unchanged(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let before = draft("Orders", "select 1");
        let after = SavedQueryDraft { name: "Renamed".into(), ..before.clone() };
        let request = ApprovalRequest {
            id: "a".into(),
            agent: None,
            subject: ApprovalSubject::SavedQuery(Box::new(SavedQueryChange::Update { before, after })),
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(request, None, cx));
        draw(cx);
        assert!(cx.debug_bounds("approval-field-name").is_some());
        assert!(cx.debug_bounds("approval-sql-before").is_none() && cx.debug_bounds("approval-sql-after").is_none());
        dialog.update(cx, |dialog, _| assert!(dialog.statement_before.is_none()));
    }

    fn dump(action: DumpAction) -> ApprovalRequest {
        ApprovalRequest {
            id: "d".into(),
            agent: Some("codex".into()),
            subject: ApprovalSubject::Dump(Box::new(DumpApproval {
                connection_id: "shop".into(),
                connection_name: "Shop".into(),
                tag: Some("PROD".into()),
                engine: Engine::Mysql,
                database: "shop".into(),
                file: "shop-2026-11-22-1030.sql".into(),
                action,
            })),
        }
    }

    fn mysql_backup() -> DumpAction {
        DumpAction::Export { options: DumpOptions::defaults(dbforge_core::dump::DumpEngine::Mysql) }
    }

    #[test]
    fn a_backup_lists_its_options_and_a_restore_warns_it_changes_data() {
        let m = crate::i18n::Language::En.messages();
        assert_eq!(
            dump_options(&mysql_backup(), m).as_deref(),
            Some(
                "Structure and data · Consistent snapshot (single transaction, InnoDB) · Routines, triggers and events"
            )
        );
        let mongodb = DumpAction::Export {
            options: DumpOptions::Mongodb(dbforge_core::dump::MongodbOptions {
                collections: vec!["a".into(), "b".into()],
            }),
        };
        assert_eq!(dump_options(&mongodb, m).as_deref(), Some("Collections: a, b"));
        let restore = |target_exists| DumpAction::Import { drop_existing: false, target_exists };
        assert_eq!(dump_options(&restore(None), m), None);
        let ApprovalSubject::Dump(existing) = dump(restore(Some(true))).subject else { unreachable!() };
        assert_eq!(
            restore_warning(&existing, m).as_deref(),
            Some("This changes data in shop, which already exists: objects in it may be replaced.")
        );
        let ApprovalSubject::Dump(backup) = dump(mysql_backup()).subject else { unreachable!() };
        assert_eq!(restore_warning(&backup, m), None, "a backup changes nothing");
        let es = crate::i18n::Language::Es.messages();
        assert_eq!(
            restore_warning(&existing, es).as_deref(),
            Some("Esto cambia datos en shop, que ya existe: los objetos que tiene pueden reemplazarse.")
        );
    }

    /// The dialog for `request`, drawn with Decline focused; the text it shows.
    fn shown_dump(
        request: ApprovalRequest,
        cx: &mut TestAppContext,
    ) -> (Entity<ApprovalDialog>, &mut VisualTestContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(request, None, cx));
        dialog.update_in(cx, |dialog, window, _| dialog.focus(window));
        draw(cx);
        assert!(
            dialog.update_in(cx, |dialog, window, _| dialog.decline.is_focused(window)),
            "Decline has the keyboard"
        );
        (dialog, cx)
    }

    #[gpui::test]
    fn shows_an_agent_s_backup_with_its_file_and_options(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (_dialog, cx) = shown_dump(dump(mysql_backup()), cx);
        assert!(cx.debug_bounds("approval-dump-file").is_some());
        assert!(cx.debug_bounds("approval-dump-options").is_some());
        assert!(cx.debug_bounds("approval-dump-warning").is_none(), "a backup changes no data");
    }

    #[gpui::test]
    fn shows_an_agent_s_restore_with_its_warning(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let restore = DumpAction::Import { drop_existing: false, target_exists: Some(true) };
        let (_dialog, cx) = shown_dump(dump(restore), cx);
        assert!(cx.debug_bounds("approval-dump-file").is_some());
        assert!(cx.debug_bounds("approval-dump-options").is_none(), "no options to list");
        assert!(cx.debug_bounds("approval-dump-warning").is_some());
        cx.simulate_keystrokes("enter");
    }

    fn shop() -> Connection {
        Connection {
            id: "shop".into(),
            name: "Shop".into(),
            engine: Engine::Postgres,
            environment: None,
            host: "db.example.com".into(),
            port: 5432,
            database: "shop".into(),
            username: "app".into(),
            ssl: dbforge_core::connections::transport::Ssl { mode: SslMode::Require, ..Default::default() },
            ssh: dbforge_core::connections::transport::SshTunnel {
                enabled: true,
                host: "bastion".into(),
                port: 22,
                username: "jump".into(),
                auth: SshAuth::Key,
                key_file: "/keys/id".into(),
            },
            srv: false,
            agent_access: AgentAccess::Read,
        }
    }

    fn connection_request(change: ConnectionChange) -> ApprovalRequest {
        ApprovalRequest {
            id: "c".into(),
            agent: Some("codex".into()),
            subject: ApprovalSubject::Connection(Box::new(change)),
        }
    }

    fn edit() -> ConnectionChange {
        let before = shop();
        let after = Connection { name: "Shop EU".into(), port: 6543, ..before.clone() };
        ConnectionChange::Update { before, after, tag: Some("PROD".into()) }
    }

    #[test]
    fn a_connection_edit_lists_each_changed_field_before_and_after() {
        let m = crate::i18n::Language::En.messages();
        let rows = connection_rows(&edit(), m);
        let shown: Vec<(&str, Option<&str>, &str)> =
            rows.iter().map(|row| (row.label, row.before.as_deref(), row.value.as_str())).collect();
        assert_eq!(shown, [("Name", Some("Shop"), "Shop EU"), ("Port", Some("5432"), "6543")]);

        let created = connection_rows(&ConnectionChange::Create { connection: shop() }, m);
        let labels: Vec<&str> = created.iter().map(|row| row.selector).collect();
        assert_eq!(
            labels,
            [
                "approval-conn-engine",
                "approval-conn-host",
                "approval-conn-port",
                "approval-conn-database",
                "approval-conn-username",
                "approval-conn-ssl-mode",
                "approval-conn-ssh",
                "approval-conn-ssh-host",
                "approval-conn-ssh-port",
                "approval-conn-ssh-user",
                "approval-conn-ssh-auth",
                "approval-conn-ssh-key-file",
            ]
        );
    }

    #[gpui::test]
    fn shows_a_new_connection_with_its_secrets_and_agent_access(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let request = connection_request(ConnectionChange::Create { connection: shop() });
        let (dialog, cx) = shown_dump(request, cx);
        for shown in [
            "approval-conn-host",
            "approval-secret-password",
            "approval-secret-ssh",
            "approval-access",
            "approval-access-read",
        ] {
            assert!(cx.debug_bounds(shown).is_some(), "{shown} shows");
        }
        dialog.update(cx, |dialog, _| assert_eq!(dialog.inputs.access, Some(AgentAccess::Read), "Read only first"));
    }

    #[gpui::test]
    fn shows_an_edit_before_and_after_and_a_delete_without_secrets(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (dialog, cx) = shown_dump(connection_request(edit()), cx);
        assert!(cx.debug_bounds("approval-conn-name").is_some() && cx.debug_bounds("approval-conn-port").is_some());
        assert!(cx.debug_bounds("approval-conn-host").is_none(), "an unchanged field isn't listed");
        assert!(cx.debug_bounds("approval-secret-password").is_some(), "a new password may be typed");
        assert!(cx.debug_bounds("approval-access").is_none(), "an edit keeps the agent access");
        dialog.update(cx, |dialog, cx| {
            let password = dialog.inputs.password.as_ref().unwrap().read(cx);
            assert!(password.text().is_empty());
        });

        let delete = connection_request(ConnectionChange::Delete { connection: shop(), tag: None });
        let (_dialog, cx) = shown_dump(delete, &mut cx.cx);
        assert!(cx.debug_bounds("approval-conn-host").is_some());
        assert!(cx.debug_bounds("approval-secret-password").is_none() && cx.debug_bounds("approval-access").is_none());
    }

    #[gpui::test]
    fn approving_sends_what_the_user_typed_to_the_waiting_tool_only(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        cx.update(|cx| {
            cx.bind_keys(key_bindings());
            cx.bind_keys(crate::ui::text_input::key_bindings());
        });
        let events: Arc<ChannelEvents> = cx.update(|cx| {
            let (events, received) = channel();
            EventHub::start(received, cx);
            Approvals::start(cx);
            Arc::new(events)
        });
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        let new = Connection { ssh: Default::default(), ..shop() };
        let request = connection_request(ConnectionChange::Create { connection: new });
        let asking =
            runtime().spawn(async move { core.mcp_approvals().ask_for_answer(events.as_ref(), request).await });
        let shown = loop {
            cx.run_until_parked();
            if let Some(shown) = cx.update(|cx| Approvals::global(cx).read(cx).current().cloned()) {
                break shown;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(shown, None, cx));
        dialog.update_in(cx, |dialog, window, _| dialog.focus(window));
        draw(cx);
        assert!(cx.debug_bounds("approval-secret-ssh").is_none(), "no tunnel, no SSH secret");

        // Shift-Tab from Decline reaches the password; a space typed there is just a space.
        cx.simulate_keystrokes("shift-tab");
        cx.simulate_input("typed");
        cx.simulate_keystrokes("space");
        cx.simulate_input("pw");
        let write = cx.debug_bounds("approval-access-write").expect("Read and write");
        cx.simulate_click(write.center(), gpui::Modifiers::none());
        assert!(!asking.is_finished(), "typing answers nothing");
        // Password → Decline → Approve.
        cx.simulate_keystrokes("tab tab enter");
        cx.run_until_parked();
        let answer = runtime().block_on(asking).expect("the request's answer");
        assert!(answer.approved);
        let mut secrets = answer.secrets.expect("what the user typed");
        assert_eq!(secrets.take_password().as_deref(), Some("typed pw"));
        assert_eq!(secrets.take_ssh_secret(), None);
        assert_eq!(secrets.agent_access, Some(AgentAccess::Write));
        dialog.update(cx, |dialog, cx| {
            assert!(dialog.inputs.password.as_ref().unwrap().read(cx).text().is_empty(), "nothing typed stays");
        });
    }

    #[gpui::test]
    fn an_edit_pointing_elsewhere_approves_only_with_the_password_typed_again(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        cx.update(|cx| {
            cx.bind_keys(key_bindings());
            cx.bind_keys(crate::ui::text_input::key_bindings());
        });
        let events: Arc<ChannelEvents> = cx.update(|cx| {
            let (events, received) = channel();
            EventHub::start(received, cx);
            Approvals::start(cx);
            Arc::new(events)
        });
        let core = cx.update(|cx| Backend::global(cx).core().clone());
        // The port changes: the saved password isn't carried to the new place.
        let request = connection_request(edit());
        let asking =
            runtime().spawn(async move { core.mcp_approvals().ask_for_answer(events.as_ref(), request).await });
        let shown = loop {
            cx.run_until_parked();
            if let Some(shown) = cx.update(|cx| Approvals::global(cx).read(cx).current().cloned()) {
                break shown;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let (dialog, cx) = cx.add_window_view(|_, cx| ApprovalDialog::new(shown, None, cx));
        dialog.update_in(cx, |dialog, window, _| dialog.focus(window));
        draw(cx);
        dialog.update(cx, |dialog, _| {
            assert!(dialog.inputs.password_required, "the password must be typed again");
            assert!(!dialog.inputs.ssh_required, "a key may have no passphrase");
        });

        // Decline → Approve, pressed with nothing typed: nothing answered.
        cx.simulate_keystrokes("tab enter");
        dialog.update(cx, |dialog, cx| dialog.approve(cx));
        cx.run_until_parked();
        assert!(!asking.is_finished(), "Approve waits for the password");
        assert!(cx.update(|_, cx| Approvals::global(cx).read(cx).current().is_some()), "still asked");

        // Approve → Decline → the SSH passphrase → the password.
        cx.simulate_keystrokes("shift-tab shift-tab shift-tab");
        cx.simulate_input("new pw");
        dialog.update(cx, |dialog, cx| dialog.approve(cx));
        cx.run_until_parked();
        let answer = runtime().block_on(asking).expect("the request's answer");
        assert!(answer.approved);
        let mut secrets = answer.secrets.expect("what the user typed");
        assert_eq!(secrets.take_password().as_deref(), Some("new pw"));
        assert_eq!(secrets.take_ssh_secret(), None);
    }

    #[gpui::test]
    fn an_edit_staying_in_place_approves_with_nothing_typed(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let before = shop();
        let after = Connection { name: "Shop EU".into(), ..before.clone() };
        let rename = connection_request(ConnectionChange::Update { before, after, tag: None });
        let (dialog, cx) = shown_dump(rename, cx);
        dialog.update(cx, |dialog, cx| {
            assert!(!dialog.inputs.password_required && !dialog.inputs.ssh_required);
            assert!(dialog.inputs.complete(cx), "empty keeps the saved secrets");
        });
    }
}
