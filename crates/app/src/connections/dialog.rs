//! The connection dialog: creates a connection or edits a saved one, with
//! its General, SSL and SSH sections, tests what is typed off the UI thread,
//! and saves it through the core, which keeps the passwords where the user
//! chose. A saved password is never read back: an empty field keeps it.
//! The sections are drawn in `general.rs`, `ssl.rs` and `ssh.rs`.

use std::collections::HashMap;
use std::time::Duration;

use dbforge_core::connections::commands;
use dbforge_core::connections::model::{AgentAccess, Connection, Engine};
use dbforge_core::connections::tags::Tag;
use dbforge_core::connections::transport::{SshAuth, SslMode};
use dbforge_core::credentials::Storage;
use gpui::{
    AnyElement, App, AppContext, ClipboardItem, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::engine::{ENGINES, is_account, is_relational};
use super::form::{
    ConnectionForm, Section, Submission, TextField, can_submit, can_test, client_cert_incomplete, empty_form,
    form_from_connection, sections_of, ssl_modes_of, submission, to_input, tunnel_on, uses_srv, with_changes,
    with_engine,
};
use super::url::parse_connection_url;
use crate::backend::Backend;
use crate::i18n::{ConnectionMessages, Translate};
use crate::preferences::Preferences;
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{
    FileField, SectionTabs, Select, SelectEvent, SelectOption, Swatch, Tab, TextInput, TextInputEvent, button, enabled,
    ghost_button, glass, icon, primary_button, spinner,
};
use dbforge_core::connections::model::ConnectionInput;

actions!(connection_dialog, [Submit, Close, FocusNext, FocusPrevious]);

const CONTEXT: &str = "ConnectionDialog";

/// One size for every engine, so switching never resizes the dialog; the form scrolls.
const WIDTH: f32 = 640.;
const HEIGHT: f32 = 640.;
/// Room kept between the dialog and the window's edges.
const MARGIN: f32 = 16.;

/// Enter saves, Escape closes, Tab moves between the fields.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("enter", Submit, context),
        KeyBinding::new("escape", Close, context),
        KeyBinding::new("tab", FocusNext, context),
        KeyBinding::new("shift-tab", FocusPrevious, context),
    ]
}

/// What the dialog tells the window, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone)]
pub enum ConnectionDialogEvent {
    /// The connection was saved: created, or changed.
    Saved(Connection),
}

/// Where a connection test is.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TestState {
    Idle,
    Testing,
    Passed(String),
    Failed(String),
}

/// The fields drawn as file pickers rather than text fields.
const FILE_FIELDS: [TextField; 5] =
    [TextField::Database, TextField::CaFile, TextField::CertFile, TextField::KeyFile, TextField::SshKeyFile];

pub struct ConnectionDialog {
    /// The saved connection being edited; `None` for a new one.
    pub(super) editing: Option<Connection>,
    pub(super) form: ConnectionForm,
    pub(super) section: Section,
    pub(super) test: TestState,
    /// Bumped on every change, so a test that ends after the form changed is dropped.
    revision: u64,
    pub(super) saving: bool,
    pub(super) save_error: Option<String>,
    /// What a pasted URL held that the form can't; `None` while nothing was pasted.
    pub(super) from_url: Option<Vec<String>>,
    /// Save was pressed before agent access was chosen.
    pub(super) access_asked: bool,
    /// Where passwords are kept; `None` while it loads.
    pub(super) storage: Option<Storage>,
    /// The connection URL was just copied.
    pub(super) copied: bool,
    pub(super) texts: HashMap<TextField, Entity<TextInput>>,
    pub(super) files: HashMap<TextField, Entity<FileField>>,
    pub(super) engine: Entity<Select<Engine>>,
    pub(super) tag: Entity<Select<Option<String>>>,
    pub(super) agent: Entity<Select<AgentAccess>>,
    pub(super) ssl_mode: Entity<Select<SslMode>>,
    pub(super) ssh_auth: Entity<Select<SshAuth>>,
    tags: Vec<Tag>,
    focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ConnectionDialog {}
impl EventEmitter<ConnectionDialogEvent> for ConnectionDialog {}

impl Focusable for ConnectionDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConnectionDialog {
    /// The dialog for `connection`, or for a new connection; the name field takes the keyboard.
    pub fn new(connection: Option<Connection>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().connection;
        let form = match &connection {
            Some(connection) => form_from_connection(connection, m.extra_hosts),
            None => empty_form(),
        };
        let mut subscriptions = Vec::new();

        let mut texts = HashMap::new();
        for field in
            TextField::ALL.into_iter().filter(|field| !FILE_FIELDS.contains(field) || *field == TextField::Database)
        {
            let input = cx.new(|cx| {
                let input = TextInput::new("", cx);
                let input = if field.is_secret() { input.masked() } else { input };
                // A pasted connection URL fills in the fields instead of landing in Host.
                if field == TextField::Host { input.intercept_paste() } else { input }
            });
            subscriptions.push(cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
                this.on_text(field, input.read(cx).text().to_owned(), event, window, cx)
            }));
            texts.insert(field, input);
        }
        let mut files = HashMap::new();
        for field in FILE_FIELDS {
            let file = cx.new(|cx| FileField::new("", m.browse, cx));
            subscriptions.push(cx.subscribe_in(&file, window, move |this, file, event, window, cx| {
                this.on_text(field, file.read(cx).text(cx).to_owned(), event, window, cx)
            }));
            files.insert(field, file);
        }
        // A URL typed rather than pasted fills in the fields when Host is left.
        let host = texts[&TextField::Host].focus_handle(cx);
        subscriptions.push(cx.on_blur(&host, window, |this, window, cx| {
            let host = this.form.host.clone();
            this.fill_from_url(&host, window, cx);
        }));

        let engine = cx.new(|cx| Select::new(Vec::new(), Some(form.engine), cx));
        subscriptions.push(cx.subscribe_in(&engine, window, |this, _, SelectEvent(engine), window, cx| {
            let engine = *engine;
            this.from_url = None;
            this.change(|form| *form = with_engine(form.clone(), engine), window, cx);
        }));
        let tag = cx.new(|cx| Select::new(Vec::new(), Some(form.environment.clone()), cx));
        subscriptions.push(cx.subscribe_in(&tag, window, |this, _, SelectEvent(tag), window, cx| {
            let tag = tag.clone();
            this.change(|form| form.environment = tag, window, cx);
        }));
        let agent = cx.new(|cx| Select::new(Vec::new(), form.agent_access, cx));
        subscriptions.push(cx.subscribe_in(&agent, window, |this, _, SelectEvent(access), window, cx| {
            let access = *access;
            this.change(|form| form.agent_access = Some(access), window, cx);
        }));
        let ssl_mode = cx.new(|cx| Select::new(Vec::new(), Some(form.ssl.mode), cx));
        subscriptions.push(cx.subscribe_in(&ssl_mode, window, |this, _, SelectEvent(mode), window, cx| {
            let mode = *mode;
            this.change(|form| form.ssl.mode = mode, window, cx);
        }));
        let ssh_auth = cx.new(|cx| Select::new(Vec::new(), Some(form.ssh.auth), cx));
        subscriptions.push(cx.subscribe_in(&ssh_auth, window, |this, _, SelectEvent(auth), window, cx| {
            let auth = *auth;
            this.change(|form| form.ssh.auth = auth, window, cx);
        }));
        // The language may change while the dialog is open.
        subscriptions.push(cx.observe_global_in::<Preferences>(window, |this, window, cx| this.sync(window, cx)));

        let core = Backend::global(cx).core();
        let tags = core.tags().list();
        let mut dialog = Self {
            editing: connection,
            form,
            section: Section::General,
            test: TestState::Idle,
            revision: 0,
            saving: false,
            save_error: None,
            from_url: None,
            access_asked: false,
            storage: None,
            copied: false,
            texts,
            files,
            engine,
            tag,
            agent,
            ssl_mode,
            ssh_auth,
            tags,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        dialog.sync(window, cx);
        dialog.load_storage(cx);
        dialog
    }

    /// Takes the keyboard: the name field, as the user starts there.
    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.texts[&TextField::Name].focus_handle(cx));
    }

    pub(super) fn messages(cx: &App) -> &'static ConnectionMessages {
        &cx.messages().connection
    }

    /// Reads where passwords are kept, for the line under the password.
    fn load_storage(&mut self, cx: &mut Context<Self>) {
        let reading = Backend::global(cx).spawn_blocking(cx, |core| {
            let info = commands::credential_storage(core.connections());
            serde_json::to_value(info)
                .ok()
                .and_then(|info| serde_json::from_value::<Storage>(info["storage"].clone()).ok())
        });
        cx.spawn(async move |this, cx| {
            let storage = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.storage = storage;
                cx.notify();
            });
        })
        .detach();
    }

    /// Changes the form; a test result no longer applies to it.
    pub(super) fn change(
        &mut self,
        change: impl FnOnce(&mut ConnectionForm),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        change(&mut self.form);
        self.revision += 1;
        self.test = TestState::Idle;
        self.sync(window, cx);
        cx.notify();
    }

    fn on_text(
        &mut self,
        field: TextField,
        text: String,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => self.change(|form| field.set(form, &text), window, cx),
            TextInputEvent::Submitted => self.submit(window, cx),
            TextInputEvent::Cancelled => self.close(cx),
            TextInputEvent::Pasted(pasted) => {
                if !self.fill_from_url(pasted, window, cx) {
                    let pasted = pasted.clone();
                    self.texts[&field].update(cx, |input, cx| input.insert(&pasted, cx));
                }
            }
        }
    }

    /// Splits a connection URL into the fields; false when `text` isn't one.
    fn fill_from_url(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(parsed) = parse_connection_url(text, Self::messages(cx).extra_hosts) else { return false };
        self.from_url = Some(parsed.ignored);
        self.change(|form| *form = with_changes(form.clone(), parsed.changes), window, cx);
        true
    }

    /// Puts the form's values, and what they allow, into the fields.
    pub(super) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = Self::messages(cx);
        let form = &self.form;
        let editing = self.editing.is_some();
        let srv = uses_srv(form);
        let tunnel = form.ssh.enabled;
        let agent_auth = form.ssh.auth == SshAuth::Agent;
        for (&field, input) in &self.texts {
            let value = field.value(form).to_owned();
            let disabled = match field {
                TextField::Port => srv,
                TextField::SshHost | TextField::SshPort | TextField::SshUser => !tunnel,
                TextField::SshSecret => !tunnel || agent_auth,
                _ => false,
            };
            let placeholder: SharedString = match field {
                TextField::Host if is_account(form.engine) => m.uri_placeholder.into(),
                TextField::Host if form.engine == Engine::Mongodb => m.mongo_host_hint.into(),
                TextField::Host if form.engine == Engine::Redis => m.redis_host_hint.into(),
                TextField::Port if srv => m.port_from_dns.into(),
                TextField::Username if form.engine == Engine::Redis => "default".into(),
                TextField::Password if editing => m.unchanged.into(),
                TextField::Password if is_account(form.engine) => m.key_placeholder.into(),
                TextField::SshHost => "bastion.example.com".into(),
                TextField::SshSecret if agent_auth => m.ssh_no_secret.into(),
                TextField::SshSecret if editing => m.unchanged.into(),
                _ => SharedString::default(),
            };
            input.update(cx, |input, cx| {
                input.set_text(&value, cx);
                input.set_placeholder(placeholder, cx);
                input.set_disabled(disabled, window, cx);
            });
        }
        let encrypts = form.ssl.mode.encrypts();
        let incomplete = client_cert_incomplete(form);
        for (&field, file) in &self.files {
            let value = field.value(form).to_owned();
            let (disabled, invalid, placeholder) = match field {
                TextField::Database => (false, false, m.database_file_placeholder),
                TextField::CaFile => (!encrypts, false, m.no_file),
                TextField::CertFile => (!encrypts, incomplete && form.ssl.cert_file.trim().is_empty(), m.no_file),
                TextField::KeyFile => (!encrypts, incomplete && form.ssl.key_file.trim().is_empty(), m.no_file),
                _ => {
                    let key = form.ssh.auth == SshAuth::Key;
                    (!tunnel || !key, tunnel && key && form.ssh.key_file.trim().is_empty(), m.ssh_key_file_placeholder)
                }
            };
            file.update(cx, |file, cx| {
                file.set_text(&value, cx);
                file.set_placeholder(placeholder, cx);
                file.set_browse_label(m.browse, cx);
                file.set_disabled(disabled, window, cx);
                file.set_invalid(invalid, cx);
            });
        }

        let engines = ENGINES
            .into_iter()
            .map(|engine| {
                SelectOption::new(engine, engine.label())
                    .group(if is_relational(engine) { m.relational } else { m.nosql })
                    .swatch(Swatch::Color(cx.theme().engine(engine)))
            })
            .collect();
        let engine = form.engine;
        self.engine.update(cx, |select, cx| {
            select.set_options(engines, cx);
            select.set_value(Some(engine), cx);
        });

        let theme = *cx.theme();
        let mut tags = vec![SelectOption::new(None, m.no_tag).swatch(Swatch::Empty)];
        tags.extend(self.tags.iter().map(|tag| {
            SelectOption::new(Some(tag.id.clone()), tag.name.clone()).swatch(Swatch::Color(theme.tag(tag.color)))
        }));
        // A tag deleted since the connection was saved shows as none.
        let environment = form.environment.clone().filter(|id| self.tags.iter().any(|tag| &tag.id == id));
        self.tag.update(cx, |select, cx| {
            select.set_options(tags, cx);
            select.set_value(Some(environment), cx);
        });

        let access = form.agent_access;
        let asked = self.access_asked && access.is_none();
        self.agent.update(cx, |select, cx| {
            select.set_options(
                vec![
                    SelectOption::new(AgentAccess::None, m.agent_none),
                    SelectOption::new(AgentAccess::Read, m.agent_read),
                    SelectOption::new(AgentAccess::Write, m.agent_write),
                ],
                cx,
            );
            select.set_placeholder(m.agent_choose, cx);
            select.set_value(access, cx);
            select.set_invalid(asked, cx);
        });

        let tunneled = tunnel_on(form);
        let modes = ssl_modes_of(form)
            .into_iter()
            .map(|mode| {
                let blocked = tunneled && !super::engine::ssl_mode_works_through_ssh(form.engine, mode);
                let label = super::ssl::mode_label(mode, m);
                let label = if blocked { format!("{label}{}", m.not_through_ssh) } else { label.to_owned() };
                SelectOption::new(mode, label)
            })
            .collect();
        let mode = form.ssl.mode;
        self.ssl_mode.update(cx, |select, cx| {
            select.set_options(modes, cx);
            select.set_value(Some(mode), cx);
        });

        let auth = form.ssh.auth;
        self.ssh_auth.update(cx, |select, cx| {
            select.set_options(
                vec![
                    SelectOption::new(SshAuth::Password, m.ssh_auth_password),
                    SelectOption::new(SshAuth::Key, m.ssh_auth_key),
                    SelectOption::new(SshAuth::Agent, m.ssh_auth_agent),
                ],
                cx,
            );
            select.set_value(Some(auth), cx);
            select.set_disabled(!tunnel, cx);
        });
        cx.notify();
    }

    /// The section shown: the chosen one, or General when the engine has no such section.
    pub(super) fn shown_section(&self) -> Section {
        if sections_of(self.form.engine).contains(&self.section) { self.section } else { Section::General }
    }

    fn input(&self) -> ConnectionInput {
        to_input(&self.form, self.editing.as_ref().map(|connection| connection.id.as_str()))
    }

    /// Tries the typed values off the UI thread; the result shows in the footer.
    fn run_test(&mut self, cx: &mut Context<Self>) {
        if self.test == TestState::Testing || !can_test(&self.form) {
            return;
        }
        self.test = TestState::Testing;
        let revision = self.revision;
        let input = self.input();
        let testing = Backend::global(cx).spawn(cx, move |core| async move {
            commands::test_connection(core.connections(), core.statement_log(), input)
                .await
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = testing.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                let connected = Self::messages(cx).connected;
                this.test = match result {
                    Ok(version) => TestState::Passed(connected(&version)),
                    Err(message) => TestState::Failed(message),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Enter or Save: saves, or shows what is still missing.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        match submission(&self.form) {
            Submission::Blocked => {}
            Submission::AskAgentAccess => {
                // The field sits at the end of General, often out of view.
                self.section = Section::General;
                self.access_asked = true;
                self.sync(window, cx);
                self.scroll.scroll_to_bottom();
                self.agent.read(cx).focus(window);
            }
            Submission::Save => self.save(cx),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.saving = true;
        self.save_error = None;
        let input = self.input();
        let saving = Backend::global(cx).spawn(cx, move |core| async move {
            commands::save_connection(core.connections(), input).await.map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(connection) => {
                    cx.emit(ConnectionDialogEvent::Saved(connection));
                    cx.emit(DismissEvent);
                }
                Err(message) => {
                    this.save_error = Some(message);
                    this.saving = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// Puts the connection URL on the clipboard; the button says so for a moment.
    pub(super) fn copy_url(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.copied = true;
        cx.notify();
        let timer = cx.background_executor().timer(Duration::from_millis(1500));
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Development: shows `section` at start, for screenshots of each.
    #[cfg(debug_assertions)]
    pub fn show_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        cx.notify();
    }

    /// Development: opens the engine list at start, once the dialog was drawn.
    #[cfg(debug_assertions)]
    pub fn open_engines(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        // Two frames: the list drops from where the dropdown was drawn.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| engine.update(cx, |select, cx| select.open(window, cx)));
        });
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let m = Self::messages(cx);
        let title = if self.editing.is_some() { m.edit_connection } else { m.new_connection };
        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(24.))
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .id("close")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(14.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.sel))
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                    .child(icon("close", theme.text2, 14.)),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let m = Self::messages(cx);
        let sections = sections_of(self.form.engine);
        let shown = self.shown_section();
        let tabs = sections
            .iter()
            .map(|section| match section {
                Section::General => Tab { label: m.general.into(), active: false, title: None },
                Section::Ssl => Tab {
                    label: m.ssl_section.into(),
                    active: self.form.ssl.mode.encrypts(),
                    title: Some(m.ssl_on.into()),
                },
                Section::Ssh => {
                    Tab { label: m.ssh_section.into(), active: self.form.ssh.enabled, title: Some(m.ssh_on.into()) }
                }
            })
            .collect();
        let selected = sections.iter().position(|section| *section == shown).unwrap_or(0);
        div().flex_none().px(px(18.)).border_t_1().border_color(cx.theme().line).child(SectionTabs::new(
            "connection-sections",
            tabs,
            selected,
            cx.listener(move |this, index: &usize, _, cx| {
                if let Some(&section) = sections.get(*index) {
                    this.section = section;
                    cx.notify();
                }
            }),
        ))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = Self::messages(cx);
        let testing = self.test == TestState::Testing;
        let testable = !testing && can_test(&self.form);
        let savable = !self.saving && can_submit(&self.form);
        // One line, always there, so a result never moves the buttons.
        let result: Option<AnyElement> = match (&self.test, &self.save_error) {
            (_, Some(error)) | (TestState::Failed(error), None) => Some(
                div()
                    .id("result")
                    .min_w_0()
                    .truncate()
                    .text_color(theme.err)
                    .tooltip(Tooltip::text(error.clone()))
                    .child(error.clone())
                    .into_any_element(),
            ),
            (TestState::Passed(message), None) => Some(
                div()
                    .h(px(22.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .rounded(px(11.))
                    .bg(theme.ok.opacity(0.16))
                    .text_color(theme.ok)
                    .font_weight(FontWeight::MEDIUM)
                    .min_w_0()
                    .truncate()
                    .child(message.clone())
                    .into_any_element(),
            ),
            (TestState::Testing, None) => Some(spinner("testing", theme.text3, 13.).into_any_element()),
            (TestState::Idle, None) => None,
        };
        div()
            .h(px(60.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(24.))
            .pr(px(20.))
            .border_t_1()
            .border_color(theme.line)
            .child(
                enabled(button(&theme, Some("pulse"), if testing { m.testing } else { m.test_connection }), testable)
                    .id("test")
                    .when(testable, |test| test.on_click(cx.listener(|this, _, _, cx| this.run_test(cx)))),
            )
            .child(div().flex_1().min_w_0().flex().items_center().text_size(px(12.)).children(result))
            .child(
                ghost_button(&theme, m.cancel)
                    .id("cancel")
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme.text))
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
            .child(
                enabled(primary_button(&theme, if self.saving { m.saving } else { m.save }), savable)
                    .id("save")
                    .when(savable, |save| save.on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))),
            )
    }
}

impl Render for ConnectionDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let viewport = window.viewport_size();
        let height = HEIGHT.min(f32::from(viewport.height) - 2. * MARGIN);
        let width = WIDTH.min(f32::from(viewport.width) - 2. * MARGIN);
        let panel = match self.shown_section() {
            Section::General => self.render_general(window, cx).into_any_element(),
            Section::Ssl => self.render_ssl(window, cx).into_any_element(),
            Section::Ssh => self.render_ssh(window, cx).into_any_element(),
        };
        glass(&theme)
            .bg(theme.card)
            .rounded(px(22.))
            .overflow_hidden()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, window, cx| this.submit(window, cx)))
            .on_action(cx.listener(|this, _: &Close, _, cx| this.close(cx)))
            .on_action(cx.listener(|_, _: &FocusNext, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, _| window.focus_prev()))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .text_color(theme.text)
            .child(self.render_header(cx))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .id("connection-form")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .border_t_1()
                    .border_color(theme.line)
                    .child(div().px(px(24.)).py(px(20.)).flex().flex_col().gap(px(16.)).child(panel)),
            )
            .child(self.render_footer(cx))
    }
}
