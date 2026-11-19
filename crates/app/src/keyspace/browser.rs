//! A Redis database in its tab, as the former web UI's `KeyBrowser.tsx`: its keys
//! on the left; the server's metrics, the selected key and the CLI on the
//! right. Choosing a key mounts its detail anew; a change of the keys (a
//! key added, renamed or deleted) scans again and reads the metrics again,
//! and a CLI command reads the key shown again. Dialogs go to the window as
//! a [`KeyBrowserEvent`].

use std::rc::Rc;

use dbforge_core::drivers::redis::{Overview, Text, database_number};
use dbforge_core::keyspace::{redis_create_key, redis_overview};
use gpui::{
    AnyView, App, AppContext, Context, DismissEvent, Entity, EventEmitter, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use super::cli_panel::{CliEvent, CliPanel};
use super::detail::{DialogKind, KeyDetail, KeyDetailEvent};
use super::dialog::{Action, FieldKind, FormDialog, FormField, Values};
use super::format::{FieldName, address_of, format_bytes, hit_rate, new_key, new_key_fields};
use super::keys::KINDS;
use super::list::{KeyAction, KeyList, KeyListEvent};
use crate::backend::Backend;
use crate::i18n::{Translate, thousands};
use crate::theme::ActiveTheme;
use crate::ui::field::Tooltip;
use crate::ui::{dot, icon_button};

/// What the browser asks of the window.
#[derive(Debug, Clone)]
pub enum KeyBrowserEvent {
    /// Show this dialog over the window, or close the one shown.
    Dialog(Option<AnyView>),
}

pub struct KeyBrowser {
    connection_id: String,
    db: i64,
    list: Entity<KeyList>,
    detail: Option<Entity<KeyDetail>>,
    cli: Entity<CliPanel>,
    overview: Option<Overview>,
    overview_error: Option<String>,
    /// The newest metrics read; an older one arriving late is dropped.
    overview_read: u64,
    dialog: Option<(Entity<FormDialog>, Subscription)>,
    /// A key just added, selected once its dialog closes.
    pending_selection: Option<String>,
    detail_events: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<KeyBrowserEvent> for KeyBrowser {}

impl KeyBrowser {
    /// The browser of `database` (`db0`) of `connection_id`, for the tab
    /// `tab_id`; its CLI's session is named after the tab.
    pub fn new(
        tab_id: &str,
        connection_id: String,
        database: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let db = database_number(database).unwrap_or(0);
        let connection = Backend::global(cx).core().connections().list().into_iter().find(|c| c.id == connection_id);
        let address = connection.map(|c| address_of(&c.host, c.port)).unwrap_or_default();
        let list = cx.new(|cx| KeyList::new(connection_id.clone(), db, window, cx));
        let cli = cx.new(|cx| {
            CliPanel::new(
                format!("{tab_id}:cli"),
                connection_id.clone(),
                db,
                format!("{address} · {database}"),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&list, window, Self::on_list),
            cx.subscribe(&cli, |this, _, _: &CliEvent, cx| {
                if let Some(detail) = &this.detail {
                    detail.update(cx, |detail, cx| detail.load(cx));
                }
                this.refresh_overview(cx);
            }),
        ];
        let mut browser = Self {
            connection_id,
            db,
            list,
            detail: None,
            cli,
            overview: None,
            overview_error: None,
            overview_read: 0,
            dialog: None,
            pending_selection: None,
            detail_events: None,
            _subscriptions: subscriptions,
        };
        browser.refresh_overview(cx);
        browser
    }

    pub fn list(&self) -> &Entity<KeyList> {
        &self.list
    }

    pub fn detail(&self) -> Option<&Entity<KeyDetail>> {
        self.detail.as_ref()
    }

    pub fn cli(&self) -> &Entity<CliPanel> {
        &self.cli
    }

    pub fn overview(&self) -> Option<&Overview> {
        self.overview.as_ref()
    }

    /// The CLI's session, closed with the tab.
    pub fn session_id(&self, cx: &App) -> String {
        self.cli.read(cx).session_id().to_owned()
    }

    fn on_list(&mut self, _: &Entity<KeyList>, event: &KeyListEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            KeyListEvent::Select(key) => {
                let same = self.detail.as_ref().is_some_and(|detail| detail.read(cx).key().text == key.name);
                if !same {
                    self.select(Some(Text { text: key.name.clone(), binary: key.binary }), None, window, cx);
                }
            }
            KeyListEvent::AddKey => self.open_add_key(window, cx),
            KeyListEvent::Action(key, action) => {
                self.select(Some(Text { text: key.name.clone(), binary: key.binary }), Some(*action), window, cx)
            }
        }
    }

    /// Shows `key` (none: the hint to pick one) in a detail of its own,
    /// which opens the dialog `action` asks for once the key is read.
    pub fn select(
        &mut self,
        key: Option<Text>,
        action: Option<KeyAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = key.as_ref().map(|key| key.text.clone());
        self.list.update(cx, |list, cx| list.set_selected(name, cx));
        self.detail_events = None;
        self.detail = key.map(|key| {
            let connection = self.connection_id.clone();
            let detail = cx.new(|cx| KeyDetail::new(connection, self.db, key, action, window, cx));
            self.detail_events = Some(cx.subscribe_in(&detail, window, Self::on_detail));
            detail
        });
        cx.notify();
    }

    fn on_detail(
        &mut self,
        detail: &Entity<KeyDetail>,
        event: &KeyDetailEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            KeyDetailEvent::KeysChanged(selected) => self.keys_changed(selected.clone(), window, cx),
            KeyDetailEvent::OpenDialog(kind) => {
                if let Some(dialog) = detail.update(cx, |detail, cx| detail.dialog(*kind, window, cx)) {
                    self.show_dialog(dialog, window, cx);
                }
            }
        }
    }

    /// Keys were added, renamed or deleted: scan again, show `selected`.
    fn keys_changed(&mut self, selected: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.select(selected.map(|name| Text::from(name.as_str())), None, window, cx);
        self.list.update(cx, |list, cx| list.reload(cx));
        self.refresh_overview(cx);
    }

    /// Opens one of the shown key's dialogs (development hooks).
    pub fn open_key_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.clone() else { return };
        if let Some(dialog) = detail.update(cx, |detail, cx| detail.dialog(kind, window, cx)) {
            self.show_dialog(dialog, window, cx);
        }
    }

    pub fn refresh_overview(&mut self, cx: &mut Context<Self>) {
        self.overview_read += 1;
        let read = self.overview_read;
        let (connection, db) = (self.connection_id.clone(), self.db);
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            redis_overview(core.connections(), core.statement_log(), connection, db).await
        });
        cx.spawn(async move |this, cx| {
            let overview = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.overview_read != read {
                    return;
                }
                match overview {
                    Ok(overview) => {
                        this.overview = Some(overview);
                        this.overview_error = None;
                    }
                    Err(error) => this.overview_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = &cx.messages().keyspace;
        let kinds = KINDS.iter().map(|kind| ((*kind).to_owned(), (*kind).into())).collect();
        let fields = vec![
            FormField::new(FieldName::Name, m.key_name, FieldKind::Text).placeholder("session:7f3a9c21e8"),
            FormField::new(FieldName::Kind, m.kind, FieldKind::Choice(kinds)).initial("string"),
            FormField::new(FieldName::Field, m.first_field, FieldKind::Text),
            FormField::new(FieldName::Value, m.value, FieldKind::Area),
            FormField::new(FieldName::Score, m.score, FieldKind::Number),
            FormField::new(FieldName::Ttl, m.expires_in_optional, FieldKind::Number),
        ];
        let this = cx.entity().downgrade();
        let (connection, db) = (self.connection_id.clone(), self.db);
        let create: Action = Rc::new(move |values: &Values, cx: &mut App| {
            let key = new_key(values, &cx.messages().keyspace)?;
            let name = key.name.clone();
            let connection = connection.clone();
            let creating = Backend::global(cx).spawn(cx, move |core| async move {
                redis_create_key(core.connections(), core.statement_log(), connection, db, key)
                    .await
                    .map_err(|error| error.to_string())
            });
            let this = this.clone();
            Ok(cx.spawn(async move |cx| {
                creating.await?;
                let _ = this.update(cx, |this, _| this.pending_selection = Some(name));
                Ok(())
            }))
        });
        let dialog =
            cx.new(|cx| FormDialog::new(m.add_key, m.add_key, fields, create, window, cx).layout(new_key_fields));
        self.show_dialog(dialog, window, cx);
    }

    fn show_dialog(&mut self, dialog: Entity<FormDialog>, window: &mut Window, cx: &mut Context<Self>) {
        dialog.read(cx).focus(window, cx);
        let closing = cx.subscribe_in(&dialog, window, |this, _, _: &DismissEvent, window, cx| {
            this.dialog = None;
            cx.emit(KeyBrowserEvent::Dialog(None));
            if let Some(name) = this.pending_selection.take() {
                this.keys_changed(Some(name), window, cx);
            }
        });
        cx.emit(KeyBrowserEvent::Dialog(Some(dialog.clone().into())));
        self.dialog = Some((dialog, closing));
    }

    /// The dialog shown, if any.
    pub fn dialog(&self) -> Option<&Entity<FormDialog>> {
        self.dialog.as_ref().map(|(dialog, _)| dialog)
    }

    fn render_metrics(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let metric = |label: &'static str, value: String, detail: Option<String>, ok: bool| {
            div()
                .flex_1()
                .min_w_0()
                .px(px(14.))
                .py(px(10.))
                .border_r_1()
                .border_color(theme.line)
                .child(div().text_size(px(11.)).text_color(theme.text3).font_weight(FontWeight::MEDIUM).child(label))
                .child(
                    div()
                        .mt(px(4.))
                        .flex()
                        .items_baseline()
                        .gap(px(4.))
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if ok { theme.ok } else { theme.text })
                        .child(div().truncate().child(value))
                        .children(detail.map(|detail| {
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(theme.text3)
                                .truncate()
                                .child(detail)
                        })),
                )
        };
        let count = |n: i64| thousands(n.max(0) as u64);
        let content = match &self.overview {
            Some(overview) => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(metric(m.keys, count(overview.keys), None, false))
                .child(metric(
                    m.memory,
                    format_bytes(overview.used_memory),
                    (overview.max_memory > 0).then(|| format!("/ {}", format_bytes(overview.max_memory))),
                    false,
                ))
                .child(metric(m.ops_per_sec, count(overview.ops_per_sec), None, false))
                .child(metric(m.hit_rate, hit_rate(overview.keyspace_hits, overview.keyspace_misses), None, true))
                .child(metric(m.clients, count(overview.clients), None, false)),
            None => div()
                .flex_1()
                .px(px(14.))
                .h(px(62.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(if self.overview_error.is_some() { theme.err } else { theme.text3 })
                .child(self.overview_error.clone().unwrap_or_else(|| m.reading_metrics.to_owned())),
        };
        div().flex().flex_none().items_center().border_b_1().border_color(theme.line).child(content).child(
            div().px(px(8.)).child(
                icon_button(&theme, "refresh-metrics", "refresh", true, false)
                    .tooltip(Tooltip::text(m.refresh_metrics))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_overview(cx))),
            ),
        )
    }
}

impl Render for KeyBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().keyspace;
        let main = match &self.detail {
            Some(detail) => div().flex_1().min_h_0().flex().flex_col().child(detail.clone()),
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(theme.text3)
                .child(m.select_key),
        };
        let footer = self.overview.as_ref().map(|overview| {
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .border_t_1()
                .border_color(theme.line)
                .text_size(px(11.5))
                .text_color(theme.text3)
                .child(dot(theme.ok))
                .child((m.connected_to)(&overview.version, &overview.mode))
                .child(div().flex_1())
                .child(format!(
                    "{} {} · {} {}",
                    m.evictions,
                    thousands(overview.evicted_keys.max(0) as u64),
                    m.expired,
                    thousands(overview.expired_keys.max(0) as u64)
                ))
        });
        div().id("key-browser").size_full().flex().child(self.list.clone()).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(self.render_metrics(cx))
                .child(main)
                .child(self.cli.clone())
                .children(footer),
        )
    }
}
