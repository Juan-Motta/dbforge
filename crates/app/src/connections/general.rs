//! The dialog's General section: engine and tag, name, where the server is
//! (a host and port, an account URI or a file), the database, who signs in,
//! the security options of MongoDB and Cosmos DB, the connection URL, and
//! what agents may do.

use dbforge_core::connections::model::Engine;
use dbforge_core::connections::transport::SslMode;
use dbforge_core::credentials::Storage;
use gpui::{
    AnyElement, App, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::dialog::ConnectionDialog;
use super::engine::{default_port, is_account, is_file};
use super::form::{TextField, can_test, is_valid_port, preview_connection, uses_srv};
use super::url::connection_url;
use crate::i18n::ConnectionMessages;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::field::{CONTROL_HEIGHT, Tooltip, field, help, label, text_box};
use crate::ui::{Checkbox, icon};

/// The port field's width beside the host.
const PORT_WIDTH: f32 = 110.;

/// Two fields side by side, each half the width.
pub(super) fn pair(left: impl IntoElement, right: impl IntoElement) -> Div {
    div().flex().gap(px(12.)).child(div().flex_1().min_w_0().child(left)).child(div().flex_1().min_w_0().child(right))
}

/// A field and a narrow one beside it: a host and its port.
pub(super) fn host_row(host: impl IntoElement, port: impl IntoElement) -> Div {
    div()
        .flex()
        .gap(px(12.))
        .child(div().flex_1().min_w_0().child(host))
        .child(div().w(px(PORT_WIDTH)).flex_none().child(port))
}

/// The database field in each engine's terms; none for Redis, Cosmos DB and SQLite.
fn database_field(engine: Engine, m: &ConnectionMessages) -> Option<(&'static str, &'static str)> {
    match engine {
        Engine::Redis | Engine::Cosmosdb | Engine::Sqlite => None,
        Engine::Postgres => Some((m.database, m.database_help)),
        Engine::Sqlserver => Some((m.default_database, m.sql_server_database_help)),
        Engine::Mongodb => Some((m.default_database, m.mongo_database_help)),
        Engine::Mysql | Engine::Mariadb => Some((m.default_database, m.optional_database_help)),
    }
}

fn host_info(engine: Engine, m: &ConnectionMessages) -> String {
    match engine {
        Engine::Mongodb => (m.host_paste_info)("mongodb://"),
        Engine::Redis => (m.host_paste_info)("redis://"),
        _ => m.host_info.to_owned(),
    }
}

fn user_info(engine: Engine, m: &ConnectionMessages) -> &'static str {
    match engine {
        Engine::Sqlserver => m.sql_server_user_info,
        Engine::Redis => m.redis_user_info,
        _ => m.user_info,
    }
}

/// Where the password (or account key) will be kept; a blank line while that loads.
fn storage_note(storage: Option<Storage>, account: bool, m: &ConnectionMessages) -> &'static str {
    match (storage, account) {
        (None, _) => "\u{a0}",
        (Some(Storage::File), true) => m.keys_stored_file,
        (Some(Storage::File), false) => m.passwords_stored_file,
        (Some(Storage::Keychain), true) => m.keys_stored,
        (Some(Storage::Keychain), false) => m.passwords_stored,
    }
}

impl ConnectionDialog {
    /// A text field of the form, labelled, in its frame.
    pub(super) fn text_field(
        &self,
        field_id: TextField,
        title: &str,
        info: Option<&str>,
        invalid: bool,
        window: &Window,
        cx: &App,
    ) -> Div {
        let theme = cx.theme();
        let mono = !matches!(field_id, TextField::Name);
        field(
            label(theme, SharedString::from(format!("info-{field_id:?}")), title.to_owned(), info),
            text_box(&self.texts[&field_id], invalid, mono, window, cx),
            None,
        )
    }

    /// A file field of the form, labelled, with its Browse… button.
    pub(super) fn file_field(&self, field_id: TextField, title: &str, info: &str, hint: Option<Div>, cx: &App) -> Div {
        field(
            label(cx.theme(), SharedString::from(format!("info-{field_id:?}")), title.to_owned(), Some(info)),
            div().h(px(CONTROL_HEIGHT)).child(self.files[&field_id].clone()),
            hint,
        )
    }

    pub(super) fn render_general(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let m = Self::messages(cx);
        let form = &self.form;
        let mut panel = div().flex().flex_col().gap(px(16.)).child(pair(
            field(label(&theme, "info-engine", m.engine, Some(m.engine_info)), self.engine.clone(), None),
            field(label(&theme, "info-tag", m.environment, Some(m.tag_info)), self.tag.clone(), None),
        ));
        panel = panel.child(self.text_field(TextField::Name, m.name, None, false, window, cx).child(help(
            &theme,
            m.name_help,
            None,
        )));
        if is_file(form.engine) {
            panel = panel.child(self.file_field(
                TextField::Database,
                m.database_file,
                m.file_info,
                Some(help(&theme, m.file_hint, None)),
                cx,
            ));
        } else {
            panel = self.render_server(panel, &theme, window, cx);
        }
        panel.child(field(
            label(&theme, "info-agent", m.agent_access, Some(m.agent_access_info)),
            self.agent.clone(),
            None,
        ))
    }

    /// Where a server or account is, who signs in, and its URL.
    fn render_server(&self, panel: Div, theme: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let m = Self::messages(cx);
        let form = &self.form;
        let account = is_account(form.engine);
        let srv = uses_srv(form);
        let location = if account {
            let invalid = !form.host.trim().is_empty() && !can_test(form);
            self.text_field(TextField::Host, m.account_uri, Some(m.uri_info), invalid, window, cx)
        } else {
            host_row(
                self.text_field(TextField::Host, m.host, Some(&host_info(form.engine, m)), false, window, cx),
                self.text_field(
                    TextField::Port,
                    m.port,
                    Some(&(m.port_info)(default_port(form.engine))),
                    !srv && !is_valid_port(&form.port),
                    window,
                    cx,
                ),
            )
        };
        let mut panel = panel.child(location);
        if let Some(ignored) = &self.from_url {
            let filled = if account { m.filled_from_connection_string } else { m.filled_from_url };
            panel =
                panel.child(
                    div().mt(px(-6.)).flex().flex_wrap().text_size(px(12.)).text_color(theme.text3).child(filled).when(
                        !ignored.is_empty(),
                        |hint| {
                            hint.child(div().text_color(theme.warn).child((m.not_carried_over)(&ignored.join(", "))))
                        },
                    ),
                );
        }
        if let Some((title, database_help)) = database_field(form.engine, m) {
            panel = panel.child(self.text_field(TextField::Database, title, None, false, window, cx).child(help(
                theme,
                database_help,
                None,
            )));
        }
        let password = self.text_field(
            TextField::Password,
            if account { m.key } else { m.password },
            Some(if account { m.key_info } else { m.password_info }),
            false,
            window,
            cx,
        );
        panel = if account {
            panel.child(password)
        } else {
            // Redis 6+ has ACL users; without ACLs the password alone is enough.
            let user_label = if form.engine == Engine::Redis { m.redis_user } else { m.user };
            let user =
                self.text_field(TextField::Username, user_label, Some(user_info(form.engine, m)), false, window, cx);
            panel.child(pair(user, password))
        };
        // The line is kept while the storage loads, so the form never moves.
        panel =
            panel.child(div().mt(px(-6.)).h(px(16.)).text_size(px(12.)).text_color(theme.text3).child(storage_note(
                self.storage,
                account,
                m,
            )));
        if form.engine == Engine::Mongodb || account {
            panel = panel.child(self.render_security(theme, cx));
        }
        panel.child(self.render_url_preview(theme, cx))
    }

    /// MongoDB's seed list, or an account's self-signed certificate.
    fn render_security(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let m = Self::messages(cx);
        let form = &self.form;
        let option: AnyElement = if form.engine == Engine::Mongodb {
            Checkbox::new("srv", m.srv, form.srv)
                .help(m.srv_help, None)
                .on_change(cx.listener(|this, checked: &bool, window, cx| {
                    let checked = *checked;
                    // A seed list is always reached over TLS.
                    this.change(
                        |form| {
                            form.srv = checked;
                            if checked && !form.ssl.mode.encrypts() {
                                form.ssl.mode = SslMode::VerifyFull;
                            }
                        },
                        window,
                        cx,
                    );
                }))
                .into_any_element()
        } else {
            // An https URI encrypts; this picks whether its certificate is checked.
            let https = form.host.trim().to_ascii_lowercase().starts_with("https:");
            Checkbox::new("self-signed", m.self_signed, form.ssl.mode == SslMode::Require)
                .help(m.self_signed_help, None)
                .disabled(!https)
                .on_change(cx.listener(|this, checked: &bool, window, cx| {
                    let mode = if *checked { SslMode::Require } else { SslMode::VerifyFull };
                    this.change(|form| form.ssl.mode = mode, window, cx);
                }))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .child(div().mb(px(8.)).text_size(px(12.)).text_color(theme.text2).child(m.security))
            .child(option)
    }

    /// Where the fields point, without the password, ready to copy. Until the
    /// fields could connect there is nothing to point at; the row stays so the
    /// form never moves.
    fn render_url_preview(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let m = Self::messages(cx);
        let url = can_test(&self.form).then(|| connection_url(&preview_connection(&self.form)));
        let copy_label = if self.copied { m.copied } else { m.copy_url };
        let copy = div()
            .id("copy-url")
            .size(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .tooltip(Tooltip::text(copy_label))
            .child(icon(if self.copied { "check" } else { "duplicate" }, theme.text2, 14.))
            .map(|copy| match url.clone() {
                Some(url) => copy
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.glass_hi))
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_url(url.clone(), cx))),
                None => copy.opacity(0.45),
            });
        field(
            label(theme, "info-url", m.connection_url, Some(m.connection_url_info)),
            div()
                .h(px(CONTROL_HEIGHT))
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(10.))
                .pr(px(3.))
                .rounded(px(10.))
                .border_1()
                .border_color(theme.line)
                .bg(theme.glass)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(px(12.))
                        .text_color(theme.text2)
                        .child(url.unwrap_or_else(|| "—".to_owned())),
                )
                .child(copy),
            None,
        )
    }
}
