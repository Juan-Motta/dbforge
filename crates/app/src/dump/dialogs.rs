//! A database's two dialogs, as the former web UI's `DumpDialogs.tsx`.
//!
//! Export writes the whole database with pg_dump to a .sql file in the
//! export folder (or one picked for this export only): what it holds,
//! whether it drops objects first or keeps owners, and the file's name. A
//! file already there is replaced only on a second press, after saying so.
//!
//! Import restores a .sql file with psql into a database, creating it when
//! it doesn't exist, in one transaction. On a connection whose tag asks
//! first (PROD) the first press only warns, naming the connection and the
//! tag; the second restores. Changing the file or the database asks again.
//!
//! Both show the tool's last line and the time while they run, can be
//! stopped, and say how they ended. Enter presses the main button; Escape
//! stops a run and closes.

use crate::ui::Ellipsis;
use std::path::Path;

use chrono::Local;
use dbforge_core::connections::model::Connection;
use dbforge_core::dump::{Content, DumpOptions, ExportRequest, ImportRequest, export_database, import_database};
use dbforge_core::explorer::list_objects;
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::job::DumpJob;
use super::{ExportFolder, Outcome, ToolRow, contents, ended, export_file_name, file_name_of, join_path, target_note};
use crate::backend::Backend;
use crate::i18n::{DumpMessages, Translate};
use crate::query::plan::format_ms;
use crate::structure::format::format_bytes;
use crate::tags::confirming_tag_of;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::callout::{Tone, tick_while};
use crate::ui::field::{field, help, label, text_box};
use crate::ui::file_prompt::{pick_file, pick_folder};
use crate::ui::modal::{form_section, modal, modal_body, modal_footer, modal_status, note, path_tail};
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::{TextInput, TextInputEvent, button, enabled, icon, primary_button};

actions!(dump_dialog, [Submit, Dismiss]);

const CONTEXT: &str = "DumpDialog";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Submit, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What a dump dialog tells its owner, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpEvent {
    /// A restore finished: the explorer shows the database, created or changed.
    Imported,
}

/// The source line: the database and its connection.
fn source(theme: &Theme, database: &str, connection: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(13.))
        .child(icon("database", theme.text3, 15.))
        .child(div().font_weight(FontWeight::SEMIBOLD).ellipsis().child(database.to_owned()))
        .child(div().flex_none().text_color(theme.text3).child(format!("· {connection}")))
        .into_any_element()
}

/// A path on one line, in monospace, its start cut when it is long.
fn path_line(theme: &Theme, text: String, muted: bool) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .ellipsis()
        .font_family(MONO)
        .text_size(px(12.5))
        .text_color(if muted { theme.text3 } else { theme.text })
        .child(path_tail(&text, 56))
}

pub struct ExportDatabaseDialog {
    connection: Connection,
    database: String,
    content: Content,
    clean: bool,
    keep_owners: bool,
    /// A folder picked for this export only; the settings keep the default.
    folder: Option<String>,
    file_name: Entity<TextInput>,
    /// A file already at the path: the first press said so, the next replaces it.
    replacing: bool,
    outcome: Option<Outcome>,
    job: DumpJob,
    tool: Entity<ToolRow>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ExportDatabaseDialog {}

impl Focusable for ExportDatabaseDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ExportDatabaseDialog {
    pub fn new(connection: Connection, database: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let file_name = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&export_file_name(&database, Local::now().naive_local()), cx);
            input
        });
        let subscriptions =
            vec![cx.subscribe_in(&file_name, window, |this, _, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {
                    this.replacing = false;
                    cx.notify();
                }
                TextInputEvent::Submitted => this.start(window, cx),
                TextInputEvent::Cancelled => this.close(cx),
                TextInputEvent::Pasted(_) => {}
            })];
        Self {
            connection,
            database,
            content: Content::All,
            clean: false,
            keep_owners: false,
            folder: None,
            file_name,
            replacing: false,
            outcome: None,
            job: DumpJob::default(),
            tool: cx.new(|cx| ToolRow::new("pg_dump", cx)),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn job(this: &mut Self) -> &mut DumpJob {
        &mut this.job
    }

    fn target(&self, cx: &App) -> Option<String> {
        self.folder.clone().or_else(|| ExportFolder::folder(cx))
    }

    fn path(&self, cx: &App) -> Option<String> {
        let name = self.file_name.read(cx).text().trim().to_owned();
        let folder = self.target(cx)?;
        (!name.is_empty()).then(|| join_path(&folder, &name))
    }

    /// The first press on a file already there says so; the next replaces it.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path(cx) else { return };
        if self.job.running() {
            return;
        }
        if self.replacing {
            self.replacing = false;
            return self.run(path, window, cx);
        }
        let checking = Backend::global(cx).spawn_blocking(cx, {
            let path = path.clone();
            move |_| Path::new(&path).exists()
        });
        cx.spawn_in(window, async move |this, cx| {
            let exists = checking.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if exists {
                    this.replacing = true;
                    cx.notify();
                } else {
                    this.run(path, window, cx);
                }
            });
        })
        .detach();
    }

    fn run(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.outcome = None;
        let job_id = self.job.start(Self::job, cx);
        self.set_busy(true, window, cx);
        tick_while(cx, |this: &Self| this.job.running());
        let request = ExportRequest {
            job_id,
            connection_id: self.connection.id.clone(),
            database: self.database.clone(),
            path: path.clone(),
            options: DumpOptions { content: self.content, clean: self.clean, keep_owners: self.keep_owners },
        };
        let exporting = Backend::global(cx).spawn(cx, move |core| async move {
            let (events, store, jobs, tools, log) =
                (core.events().as_ref(), core.connections(), core.jobs(), core.tool_paths(), core.statement_log());
            export_database(events, store, jobs, tools, log, request).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let exported = exporting.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let m = &cx.messages().dump;
                this.outcome = Some(match exported {
                    Ok(done) => Outcome {
                        tone: Tone::Done,
                        title: (m.exported)(
                            &this.database,
                            &format_bytes(done.bytes as i64),
                            &format_ms(done.elapsed_ms as f64),
                        ),
                        detail: Some(path),
                        mono: true,
                    },
                    Err(error) => ended(&error.to_string(), m.export_stopped, m.export_failed),
                });
                this.job.end();
                this.set_busy(false, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.file_name.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        self.tool.update(cx, |tool, cx| tool.set_disabled(busy, cx));
        if !busy {
            self.focus(window);
        }
        cx.notify();
    }

    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let picked = pick_folder(cx);
        cx.spawn(async move |this, cx| {
            let Some(folder) = picked.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.folder = Some(folder.to_string_lossy().into_owned());
                this.replacing = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.job.cancel(cx);
        cx.emit(DismissEvent);
    }

    fn render_content(&self, theme: &Theme, m: &DumpMessages, cx: &mut Context<Self>) -> AnyElement {
        let running = self.job.running();
        let rows = contents(m)
            .into_iter()
            .enumerate()
            .map(|(index, (content, title, text, icon_name))| {
                OptionItem::new(("dump-content", index), Mark::Radio, title, self.content == content)
                    .icon(icon_name)
                    .description(text)
                    .disabled(running)
                    .on_select(cx.listener(move |this, _, _, cx| {
                        this.content = content;
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        form_section(theme, m.content).child(option_list(theme, rows)).into_any_element()
    }

    fn render_options(&self, theme: &Theme, m: &DumpMessages, cx: &mut Context<Self>) -> AnyElement {
        let running = self.job.running();
        let data_only = self.content == Content::Data;
        let (clean, keep_owners) = (self.clean, self.keep_owners);
        let rows = vec![
            OptionItem::new("dump-clean", Mark::Check, m.clean, clean && !data_only)
                .description(m.clean_description)
                .disabled(running || data_only)
                .on_select(cx.listener(move |this, _, _, cx| {
                    this.clean = !clean;
                    cx.notify();
                }))
                .into_any_element(),
            OptionItem::new("dump-owners", Mark::Check, m.keep_owners, keep_owners)
                .description(m.keep_owners_description)
                .disabled(running)
                .on_select(cx.listener(move |this, _, _, cx| {
                    this.keep_owners = !keep_owners;
                    cx.notify();
                }))
                .into_any_element(),
        ];
        form_section(theme, m.options).child(option_list(theme, rows)).into_any_element()
    }
}

impl Render for ExportDatabaseDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let running = self.job.running();
        let target = self.target(cx);
        let path = self.path(cx);
        let name = self.file_name.read(cx).text().trim().to_owned();
        let folder_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(icon("folder", theme.text3, 15.))
            .child(path_line(&theme, target.clone().unwrap_or_else(|| m.looking_for_folder.into()), target.is_none()))
            .child(
                enabled(button(&theme, None, m.change), !running)
                    .id("dump-change-folder")
                    .when(!running, |change| change.on_click(cx.listener(|this, _, _, cx| this.pick_folder(cx)))),
            );
        let save_to = form_section(&theme, m.save_to).child(folder_row).child(field(
            label(&theme, "dump-file-name", m.file_name, None),
            text_box(&self.file_name, false, true, window, cx),
            Some(help(&theme, m.folder_help, None)),
        ));
        let replacing = self.replacing.then(|| Outcome {
            tone: Tone::Warning,
            title: (m.file_exists)(&name),
            detail: Some(m.file_exists_detail.into()),
            mono: false,
        });
        let status = self.job.status((m.exporting_into)(&self.database), replacing.as_ref().or(self.outcome.as_ref()));
        let body = modal_body("dump-export-body")
            .child(source(&theme, &self.database, &self.connection.name))
            .child(self.render_content(&theme, m, cx))
            .child(self.render_options(&theme, m, cx))
            .child(save_to)
            .child(self.tool.clone());
        let done = matches!(&self.outcome, Some(outcome) if outcome.tone == Tone::Done);
        let leading = if running {
            button(&theme, Some("stop"), m.stop)
                .id("dump-stop")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.job.cancel(cx)))
        } else {
            button(&theme, None, if done { m.done } else { m.cancel })
                .id("dump-cancel")
                .cursor_pointer()
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
        };
        let ready = !running && path.is_some();
        let label = if running {
            m.exporting
        } else if self.replacing {
            m.replace
        } else {
            m.export
        };
        let primary = enabled(primary_button(&theme, label), ready)
            .id("dump-export")
            .when(ready, |run| run.on_click(cx.listener(|this, _, window, cx| this.start(window, cx))));
        modal(&theme, 600., (m.export_title)(&self.database))
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, window, cx| this.start(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
            .child(body)
            .child(modal_status(status))
            .child(modal_footer(&theme).child(leading).child(primary))
    }
}

pub struct ImportDatabaseDialog {
    connection: Connection,
    path: Option<String>,
    /// The database the file goes into: the one the menu was opened on unless another is typed.
    target: Entity<TextInput>,
    /// The server's databases, read again after each import (which may have created one).
    databases: Option<Vec<String>>,
    /// On a connection whose tag asks first: the first press warned, the next restores.
    confirming: bool,
    outcome: Option<Outcome>,
    job: DumpJob,
    tool: Entity<ToolRow>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ImportDatabaseDialog {}
impl EventEmitter<DumpEvent> for ImportDatabaseDialog {}

impl Focusable for ImportDatabaseDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ImportDatabaseDialog {
    pub fn new(connection: Connection, database: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let target = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&database, cx);
            input
        });
        let subscriptions =
            vec![cx.subscribe_in(&target, window, |this, _, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {
                    this.outcome = None;
                    this.confirming = false;
                    cx.notify();
                }
                TextInputEvent::Submitted => this.start(window, cx),
                TextInputEvent::Cancelled => this.close(cx),
                TextInputEvent::Pasted(_) => {}
            })];
        let mut dialog = Self {
            connection,
            path: None,
            target,
            databases: None,
            confirming: false,
            outcome: None,
            job: DumpJob::default(),
            tool: cx.new(|cx| ToolRow::new("psql", cx)),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        dialog.list_databases(cx);
        dialog
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn job(this: &mut Self) -> &mut DumpJob {
        &mut this.job
    }

    fn list_databases(&mut self, cx: &mut Context<Self>) {
        let connection = self.connection.id.clone();
        let listing = Backend::global(cx).spawn(cx, move |core| async move {
            list_objects(core.connections(), core.statement_log(), connection, Vec::new()).await
        });
        cx.spawn(async move |this, cx| {
            // Unread, the note says the database is restored into or created.
            let Ok(objects) = listing.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.databases = Some(objects.into_iter().map(|object| object.name).collect());
                cx.notify();
            });
        })
        .detach();
    }

    fn name(&self, cx: &App) -> String {
        self.target.read(cx).text().trim().to_owned()
    }

    /// Development only: whether Import can be pressed.
    #[cfg(debug_assertions)]
    pub(crate) fn ready_for_development(&self, cx: &App) -> bool {
        self.ready(cx)
    }

    fn ready(&self, cx: &App) -> bool {
        !self.job.running() && self.path.is_some() && !self.name(cx).is_empty()
    }

    pub(crate) fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = pick_file(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.path = Some(path.to_string_lossy().into_owned());
                this.outcome = None;
                this.confirming = false;
                this.focus(window);
                cx.notify();
            });
        })
        .detach();
    }

    /// Enter or Import: on a connection whose tag asks first, the first press
    /// only warns; the next restores. Elsewhere it restores at once.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        if confirming_tag_of(cx, &self.connection.id).is_some() && !self.confirming {
            self.confirming = true;
            cx.notify();
            return;
        }
        let Some(path) = self.path.clone() else { return };
        self.confirming = false;
        self.outcome = None;
        let name = self.name(cx);
        let job_id = self.job.start(Self::job, cx);
        self.set_busy(true, window, cx);
        tick_while(cx, |this: &Self| this.job.running());
        let request = ImportRequest {
            job_id,
            connection_id: self.connection.id.clone(),
            database: name.clone(),
            path: path.clone(),
        };
        let importing = Backend::global(cx).spawn(cx, move |core| async move {
            let (events, store, jobs, tools, log) =
                (core.events().as_ref(), core.connections(), core.jobs(), core.tool_paths(), core.statement_log());
            import_database(events, store, jobs, tools, log, request).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let imported = importing.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let m = &cx.messages().dump;
                this.outcome = Some(match imported {
                    Ok(done) => {
                        let time = format_ms(done.elapsed_ms as f64);
                        let title = if done.created {
                            (m.created_and_restored)(&name, &time)
                        } else {
                            (m.restored)(&name, &time)
                        };
                        // What the import left out matters more than the file's name.
                        let (detail, mono) = if done.skipped_extensions.is_empty() {
                            (file_name_of(&path).to_owned(), true)
                        } else {
                            ((m.skipped_extensions)(&done.skipped_extensions.join(", ")), false)
                        };
                        this.list_databases(cx);
                        cx.emit(DumpEvent::Imported);
                        Outcome { tone: Tone::Done, title, detail: Some(detail), mono }
                    }
                    Err(error) => ended(&error.to_string(), m.import_stopped, m.import_failed),
                });
                this.job.end();
                this.set_busy(false, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.target.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        self.tool.update(cx, |tool, cx| tool.set_disabled(busy, cx));
        if !busy {
            self.focus(window);
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.job.cancel(cx);
        cx.emit(DismissEvent);
    }
}

impl Render for ImportDatabaseDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let running = self.job.running();
        let name = self.name(cx);
        let tag = confirming_tag_of(cx, &self.connection.id);
        let file = form_section(&theme, m.file).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(path_line(
                    &theme,
                    self.path.clone().unwrap_or_else(|| m.file_placeholder.into()),
                    self.path.is_none(),
                ))
                .child(
                    enabled(button(&theme, None, m.choose), !running).id("dump-choose-file").when(!running, |choose| {
                        choose.on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)))
                    }),
                ),
        );
        let target = field(
            label(&theme, "dump-target", (m.database_on)(&self.connection.name), None),
            text_box(&self.target, false, true, window, cx),
            Some(help(&theme, target_note(&name, self.databases.as_deref(), m), None)),
        );
        // One paragraph, wrapping in the dialog's width; `\!` is what psql runs shell commands with.
        let safety = div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().mt(px(2.)).child(icon("info", theme.text3, 13.)))
            .child(note(&theme, format!("{}\\!{}", m.import_note_before, m.import_note_after)).flex_1().min_w_0());
        let confirming = (self.confirming && tag.is_some()).then(|| Outcome {
            tone: Tone::Warning,
            title: (m.confirm_import)(&self.connection.name, tag.as_deref().unwrap_or_default()),
            detail: Some((m.confirm_import_detail)(tag.as_deref().unwrap_or_default())),
            mono: false,
        });
        let status = self.job.status((m.importing_into)(&name), confirming.as_ref().or(self.outcome.as_ref()));
        let body = modal_body("dump-import-body").child(file).child(target).child(safety).child(self.tool.clone());
        let done = matches!(&self.outcome, Some(outcome) if outcome.tone == Tone::Done);
        let leading = if running {
            button(&theme, Some("stop"), m.stop)
                .id("dump-stop")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.job.cancel(cx)))
        } else {
            button(&theme, None, if done { m.done } else { m.cancel })
                .id("dump-cancel")
                .cursor_pointer()
                .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
        };
        let ready = self.ready(cx);
        let label = match (running, self.confirming, &tag) {
            (true, _, _) => m.importing.to_owned(),
            (false, true, Some(tag)) => (m.import_into)(tag),
            _ => m.import.to_owned(),
        };
        let primary = enabled(primary_button(&theme, &label), ready)
            .id("dump-import")
            .when(ready, |run| run.on_click(cx.listener(|this, _, window, cx| this.start(window, cx))));
        modal(&theme, 600., m.import_title)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, window, cx| this.start(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
            .child(body)
            .child(modal_status(status))
            .child(modal_footer(&theme).child(leading).child(primary))
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::ConnectionInput;
    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup};

    fn postgres(environment: &str, cx: &mut TestAppContext) -> Connection {
        cx.update(|cx| {
            let input: ConnectionInput = serde_json::from_value(serde_json::json!({
                "name": "Shop", "engine": "postgres", "environment": environment, "host": "127.0.0.1",
                "port": 1, "database": "shop", "username": "nobody",
            }))
            .expect("an input");
            Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved")
        })
    }

    #[gpui::test]
    fn a_restore_on_a_connection_whose_tag_asks_first_only_warns_on_the_first_press(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let connection = postgres("production", cx);
        let (dialog, cx) =
            cx.add_window_view(|window, cx| ImportDatabaseDialog::new(connection, "shop".into(), window, cx));
        dialog.update_in(cx, |dialog, window, cx| {
            dialog.start(window, cx);
            assert!(!dialog.confirming && !dialog.job.running(), "no file chosen: nothing happens");
            dialog.path = Some("/nowhere/shop.sql".into());
            dialog.start(window, cx);
            assert!(dialog.confirming, "the first press only warns");
            assert!(!dialog.job.running(), "and runs nothing");
            dialog.target.update(cx, |input, cx| input.insert("_copy", cx));
        });
        cx.run_until_parked();
        dialog.read_with(cx, |dialog, _| assert!(!dialog.confirming, "another database asks again"));
        draw(cx);
    }

    #[gpui::test]
    fn an_export_names_its_file_after_the_database_and_the_minute(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let connection = postgres("development", cx);
        let (dialog, cx) =
            cx.add_window_view(|window, cx| ExportDatabaseDialog::new(connection, "shop".into(), window, cx));
        dialog.read_with(cx, |dialog, cx| {
            let name = dialog.file_name.read(cx).text().to_owned();
            assert!(name.starts_with("shop-") && name.ends_with(".sql"), "{name}");
        });
        draw(cx);
    }
}
