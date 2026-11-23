//! A database's two dialogs, as the former web UI's `DumpDialogs.tsx`.
//!
//! Export writes the whole database to a file in the export folder (or one
//! picked for this export only), with what its engine offers: PostgreSQL what
//! it holds, whether it drops objects first or keeps owners; MySQL and
//! MariaDB what it holds, DROP TABLE statements, one consistent snapshot and
//! routines; MongoDB the whole database or chosen collections; SQLite a SQL
//! script or a copy of the file. The file is named with the engine's
//! extension. A file already there is replaced only on a second press, after
//! saying so.
//!
//! Import restores a file into a database, creating it when it doesn't
//! exist (MongoDB can drop the archive's collections first), or for SQLite
//! into the connection's own file or a new one. On a connection whose tag
//! asks first (PROD) the first press only warns, naming the connection and
//! the tag; the second restores. Changing the file or the target asks again.
//!
//! Each engine's sections are fixed, so changing an option never resizes the
//! dialog. Both show the tool's last line and the time while they run, can
//! be stopped, and say how they ended. Enter presses the main button; Escape
//! stops a run and closes.

use crate::ui::Ellipsis;
use std::path::Path;

use chrono::Local;
use dbforge_core::connections::model::Connection;
use dbforge_core::dump::{
    Content, DumpEngine, ExportRequest, ImportRequest, MysqlOptions, RestoreOptions, SqliteFormat, export_database,
    import_database,
};
use dbforge_core::explorer::list_objects;
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::choices::{Collections, ExportChoices, sqlite_name, sqlite_target, swap_extension};
use super::job::DumpJob;
use super::{
    ExportFolder, Outcome, ToolRow, contents, ended, export_file_name, file_name_of, file_placeholder, join_path,
    target_note,
};
use crate::backend::Backend;
use crate::i18n::{DumpMessages, Translate};
use crate::query::plan::format_ms;
use crate::structure::format::format_bytes;
use crate::tags::confirming_tag_of;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::callout::{Tone, tick_while};
use crate::ui::checkbox::Checkbox;
use crate::ui::field::{field, help, label, text_box};
use crate::ui::file_prompt::{documents_dir, pick_file, pick_folder, pick_new_path};
use crate::ui::modal::{form_section, modal, modal_body, modal_footer, modal_status, note, path_tail};
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::segmented::segmented;
use crate::ui::{TextInput, TextInputEvent, any, button, enabled, icon, primary_button, segment_face};

actions!(dump_dialog, [Submit, Dismiss]);

const CONTEXT: &str = "DumpDialog";

/// How tall MongoDB's collection list is, whatever it holds, so choosing never resizes the dialog.
const COLLECTION_LIST_HEIGHT: f32 = 132.;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Submit, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What a dump dialog tells its owner, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpEvent {
    /// A restore finished: the explorer shows the database, created or changed.
    Imported,
}

/// How `connection`'s databases are dumped. The menus open these dialogs
/// only where the engine has dumps; elsewhere the core refuses the run.
fn engine_of(connection: &Connection) -> DumpEngine {
    DumpEngine::of(connection.engine).unwrap_or(DumpEngine::Postgres)
}

/// The row of the tool at `index` of `engine`'s (export's first), or none when it needs none (SQLite).
fn tool_row<T>(engine: DumpEngine, index: usize, cx: &mut Context<T>) -> Option<Entity<ToolRow>> {
    engine.tools().get(index).map(|&name| cx.new(|cx| ToolRow::new(name, cx)))
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

/// One choice of a few, as segments, `chosen` raised; each picks its value
/// with `pick` unless `disabled`. Its note below stays one line high.
fn choice<V: Copy + PartialEq + 'static, T: 'static>(
    theme: &Theme,
    id: &'static str,
    items: &[(V, &'static str)],
    chosen: V,
    disabled: bool,
    pick: fn(&mut T, V, &mut Context<T>),
    cx: &mut Context<T>,
) -> Div {
    let segments = items
        .iter()
        .enumerate()
        .map(|(index, &(value, text))| {
            any(segment_face(theme, None, text, value == chosen).id((id, index)).when(!disabled, |segment| {
                segment.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| pick(this, value, cx)))
            }))
        })
        .collect();
    let selected = items.iter().position(|(value, _)| *value == chosen);
    div().flex().when(disabled, |row| row.opacity(0.7)).child(segmented(theme, id, selected, segments))
}

/// One of mysqldump's options as a checkbox: what it sets in [`MysqlOptions`].
struct MysqlFlag {
    id: &'static str,
    title: &'static str,
    help: &'static str,
    checked: bool,
    disabled: bool,
    set: fn(&mut MysqlOptions, bool),
}

/// A note that keeps one line's room, so a shorter one never moves what follows.
fn one_line_note(theme: &Theme, text: impl Into<gpui::SharedString>) -> Div {
    note(theme, text).min_h(px(17.))
}

pub struct ExportDatabaseDialog {
    connection: Connection,
    /// The database's name; a SQLite connection's file's, without its extension.
    database: String,
    choices: ExportChoices,
    /// MongoDB: the database's collections to choose from.
    collections: Listing,
    /// A folder picked for this export only; the settings keep the default.
    folder: Option<String>,
    file_name: Entity<TextInput>,
    /// A file already at the path: the first press said so, the next replaces it.
    replacing: bool,
    outcome: Option<Outcome>,
    job: DumpJob,
    /// The tool that writes the file; SQLite needs none.
    tool: Option<Entity<ToolRow>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// A list read from the server for a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Listing {
    Loading,
    Listed(Vec<String>),
    Failed(String),
}

impl EventEmitter<DismissEvent> for ExportDatabaseDialog {}

impl Focusable for ExportDatabaseDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ExportDatabaseDialog {
    pub fn new(connection: Connection, database: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = engine_of(&connection);
        // A SQLite connection is one database: its file.
        let database = if engine == DumpEngine::Sqlite { sqlite_name(&connection.database) } else { database };
        let choices = ExportChoices::new(engine);
        let file_name = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&export_file_name(&database, Local::now().naive_local(), choices.extension()), cx);
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
        let mut dialog = Self {
            connection,
            database,
            choices,
            collections: Listing::Loading,
            folder: None,
            file_name,
            replacing: false,
            outcome: None,
            job: DumpJob::default(),
            tool: tool_row(engine, 0, cx),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        if engine == DumpEngine::Mongodb {
            dialog.list_collections(cx);
        }
        dialog
    }

    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn job(this: &mut Self) -> &mut DumpJob {
        &mut this.job
    }

    fn list_collections(&mut self, cx: &mut Context<Self>) {
        let (connection, database) = (self.connection.id.clone(), self.database.clone());
        let listing = Backend::global(cx).spawn(cx, move |core| async move {
            list_objects(core.connections(), core.statement_log(), connection, vec![database]).await
        });
        cx.spawn(async move |this, cx| {
            let listed = listing.await;
            let _ = this.update(cx, |this, cx| {
                this.collections = match listed {
                    Ok(objects) => Listing::Listed(objects.into_iter().map(|object| object.name).collect()),
                    Err(error) => Listing::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn target(&self, cx: &App) -> Option<String> {
        self.folder.clone().or_else(|| ExportFolder::folder(cx))
    }

    fn path(&self, cx: &App) -> Option<String> {
        let name = self.file_name.read(cx).text().trim().to_owned();
        let folder = self.target(cx)?;
        (!name.is_empty()).then(|| join_path(&folder, &name))
    }

    fn ready(&self, cx: &App) -> bool {
        !self.job.running() && self.choices.ready() && self.path(cx).is_some()
    }

    /// Changes the choices; a file name ending with the old extension takes the new one.
    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut ExportChoices)) {
        let before = self.choices.extension();
        change(&mut self.choices);
        let after = self.choices.extension();
        if before != after {
            let name = self.file_name.read(cx).text().to_owned();
            let renamed = swap_extension(&name, before, after);
            if renamed != name {
                self.file_name.update(cx, |input, cx| input.set_text(&renamed, cx));
            }
            self.replacing = false;
        }
        cx.notify();
    }

    /// What the core is asked to export, as `job_id`, to `path`.
    fn request(&self, job_id: String, path: String) -> ExportRequest {
        ExportRequest {
            job_id,
            connection_id: self.connection.id.clone(),
            database: self.database.clone(),
            path,
            options: self.choices.options(),
        }
    }

    /// The first press on a file already there says so; the next replaces it.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        let Some(path) = self.path(cx) else { return };
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
        let request = self.request(job_id, path.clone());
        let exporting = Backend::global(cx).spawn(cx, move |core| async move {
            let (events, store, jobs, tools, log) =
                (core.events().as_ref(), core.connections(), core.jobs(), core.tool_paths(), core.statement_log());
            export_database(events, store, jobs, tools, log, core.export_records(), request).await
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
        if let Some(tool) = &self.tool {
            tool.update(cx, |tool, cx| tool.set_disabled(busy, cx));
        }
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

    /// The sections of the engine's options, fixed for the engine.
    fn render_choices(&self, theme: &Theme, m: &DumpMessages, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match &self.choices {
            ExportChoices::Postgres(options) => vec![
                self.render_postgres_content(theme, m, options.content, cx),
                self.render_postgres_options(theme, m, options.content, options.clean, options.keep_owners, cx),
            ],
            ExportChoices::Mysql(options) => self.render_mysql(theme, m, options, cx),
            ExportChoices::Mongodb(collections) => vec![self.render_collections(theme, m, collections, cx)],
            ExportChoices::Sqlite(format) => {
                let items = [(SqliteFormat::Script, m.sqlite_script), (SqliteFormat::File, m.sqlite_file)];
                let pick: fn(&mut Self, SqliteFormat, &mut Context<Self>) =
                    |this, format, cx| this.change(cx, |choices| *choices = ExportChoices::Sqlite(format));
                let description = match format {
                    SqliteFormat::Script => m.sqlite_script_description,
                    SqliteFormat::File => m.sqlite_file_description,
                };
                vec![
                    form_section(theme, m.format)
                        .child(choice(theme, "dump-sqlite-format", &items, *format, self.job.running(), pick, cx))
                        .child(one_line_note(theme, description))
                        .into_any_element(),
                ]
            }
        }
    }

    /// MySQL and MariaDB: what the file holds, then mysqldump's options.
    fn render_mysql(
        &self,
        theme: &Theme,
        m: &DumpMessages,
        options: &MysqlOptions,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let running = self.job.running();
        let content = options.content;
        let items: Vec<(Content, &'static str)> =
            contents(m).iter().map(|(content, title, _, _)| (*content, *title)).collect();
        let description = match content {
            Content::All => m.all_description,
            Content::Structure => m.mysql_structure_description,
            Content::Data => m.data_description,
        };
        let pick: fn(&mut Self, Content, &mut Context<Self>) =
            |this, content, cx| this.change_mysql(cx, |options| options.content = content);
        let content_section = form_section(theme, m.content)
            .child(choice(theme, "dump-mysql-content", &items, content, running, pick, cx))
            .child(one_line_note(theme, description));
        // Data only creates no tables, so there are none to drop first.
        let data_only = content == Content::Data;
        let flags = [
            MysqlFlag {
                id: "dump-add-drop-table",
                title: m.add_drop_table,
                help: m.add_drop_table_description,
                checked: options.add_drop_table && !data_only,
                disabled: running || data_only,
                set: |options, on| options.add_drop_table = on,
            },
            MysqlFlag {
                id: "dump-single-transaction",
                title: m.single_transaction,
                help: m.single_transaction_description,
                checked: options.single_transaction,
                disabled: running,
                set: |options, on| options.single_transaction = on,
            },
            MysqlFlag {
                id: "dump-routines",
                title: m.routines,
                help: m.routines_description,
                checked: options.routines,
                disabled: running,
                set: |options, on| options.routines = on,
            },
        ];
        let rows = flags.into_iter().map(|flag| {
            let set = flag.set;
            Checkbox::new(flag.id, flag.title, flag.checked).help(flag.help, None).disabled(flag.disabled).on_change(
                cx.listener(move |this, on: &bool, _, cx| {
                    let on = *on;
                    this.change_mysql(cx, |options| set(options, on));
                }),
            )
        });
        let options_section = form_section(theme, m.options).child(div().flex().flex_col().gap(px(10.)).children(rows));
        vec![content_section.into_any_element(), options_section.into_any_element()]
    }

    /// Changes MySQL's options, when the export is MySQL's.
    fn change_mysql(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut MysqlOptions)) {
        self.change(cx, |choices| {
            if let ExportChoices::Mysql(options) = choices {
                change(options);
            }
        });
    }

    fn render_postgres_content(
        &self,
        theme: &Theme,
        m: &DumpMessages,
        chosen: Content,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.job.running();
        let rows = contents(m)
            .into_iter()
            .enumerate()
            .map(|(index, (content, title, text, icon_name))| {
                OptionItem::new(("dump-content", index), Mark::Radio, title, chosen == content)
                    .icon(icon_name)
                    .description(text)
                    .disabled(running)
                    .on_select(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |choices| {
                            if let ExportChoices::Postgres(options) = choices {
                                options.content = content;
                            }
                        })
                    }))
                    .into_any_element()
            })
            .collect();
        form_section(theme, m.content).child(option_list(theme, rows)).into_any_element()
    }

    fn render_postgres_options(
        &self,
        theme: &Theme,
        m: &DumpMessages,
        content: Content,
        clean: bool,
        keep_owners: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.job.running();
        let data_only = content == Content::Data;
        let rows = vec![
            OptionItem::new("dump-clean", Mark::Check, m.clean, clean && !data_only)
                .description(m.clean_description)
                .disabled(running || data_only)
                .on_select(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |choices| {
                        if let ExportChoices::Postgres(options) = choices {
                            options.clean = !clean;
                        }
                    })
                }))
                .into_any_element(),
            OptionItem::new("dump-owners", Mark::Check, m.keep_owners, keep_owners)
                .description(m.keep_owners_description)
                .disabled(running)
                .on_select(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |choices| {
                        if let ExportChoices::Postgres(options) = choices {
                            options.keep_owners = !keep_owners;
                        }
                    })
                }))
                .into_any_element(),
        ];
        form_section(theme, m.options).child(option_list(theme, rows)).into_any_element()
    }

    /// MongoDB: the whole database, or the collections ticked in a list of fixed height.
    fn render_collections(
        &self,
        theme: &Theme,
        m: &DumpMessages,
        collections: &Collections,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.job.running();
        let whole = collections.whole;
        let pick: fn(&mut Self, bool, &mut Context<Self>) = |this, whole, cx| {
            this.change(cx, |choices| {
                if let ExportChoices::Mongodb(collections) = choices {
                    collections.whole = whole;
                }
            })
        };
        let items = [(true, m.whole_database), (false, m.chosen_collections)];
        let list = div()
            .id("dump-collection-list")
            .h(px(COLLECTION_LIST_HEIGHT))
            .overflow_y_scroll()
            .px(px(12.))
            .py(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme.line);
        let list = match &self.collections {
            Listing::Loading => list.child(note(theme, m.loading_collections)),
            Listing::Failed(error) => list.child(note(theme, error.clone()).text_color(theme.err)),
            Listing::Listed(names) if names.is_empty() => list.child(note(theme, m.no_collections)),
            Listing::Listed(names) => list.child(div().grid().grid_cols(2).gap_x(px(12.)).gap_y(px(4.)).children(
                names.iter().enumerate().map(|(index, name)| {
                    let checked = whole || collections.chosen.contains(name);
                    let name = name.clone();
                    Checkbox::new(("dump-collection", index), name.clone(), checked)
                        .icon("table")
                        .disabled(running || whole)
                        .on_change(cx.listener(move |this, on: &bool, _, cx| {
                            let on = *on;
                            this.change(cx, |choices| {
                                if let ExportChoices::Mongodb(collections) = choices {
                                    collections.toggle(&name, on);
                                }
                            })
                        }))
                }),
            )),
        };
        let summary = if whole {
            m.whole_database_note.to_owned()
        } else {
            (m.chosen_collections_note)(collections.chosen.len())
        };
        form_section(theme, m.collections)
            .child(choice(theme, "dump-collections", &items, whole, running, pick, cx))
            .child(list)
            .child(one_line_note(theme, summary))
            .into_any_element()
    }
}

impl Render for ExportDatabaseDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let running = self.job.running();
        let target = self.target(cx);
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
            .children(self.render_choices(&theme, m, cx))
            .child(save_to)
            .children(self.tool.clone());
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
    engine: DumpEngine,
    path: Option<String>,
    /// The database the file goes into: the one the menu was opened on unless another is typed.
    target: Entity<TextInput>,
    /// The server's databases, read again after each import (which may have created one).
    databases: Option<Vec<String>>,
    /// MongoDB: drops the archive's collections before restoring them.
    drop_existing: bool,
    /// SQLite: restores into a new file (`new_file`, once picked) instead of the connection's.
    into_new_file: bool,
    new_file: Option<String>,
    /// On a connection whose tag asks first: the first press warned, the next restores.
    confirming: bool,
    outcome: Option<Outcome>,
    job: DumpJob,
    /// The tool that restores; SQLite needs none.
    tool: Option<Entity<ToolRow>>,
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
        let engine = engine_of(&connection);
        let target = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&database, cx);
            input
        });
        let subscriptions =
            vec![cx.subscribe_in(&target, window, |this, _, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => this.target_changed(cx),
                TextInputEvent::Submitted => this.start(window, cx),
                TextInputEvent::Cancelled => this.close(cx),
                TextInputEvent::Pasted(_) => {}
            })];
        let mut dialog = Self {
            connection,
            engine,
            path: None,
            target,
            databases: None,
            drop_existing: false,
            into_new_file: false,
            new_file: None,
            confirming: false,
            outcome: None,
            job: DumpJob::default(),
            tool: tool_row(engine, 1, cx),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        if engine != DumpEngine::Sqlite {
            dialog.list_databases(cx);
        }
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

    /// Another target: the last outcome no longer applies, and a tag asks again.
    fn target_changed(&mut self, cx: &mut Context<Self>) {
        self.outcome = None;
        self.confirming = false;
        cx.notify();
    }

    /// The typed database's name.
    fn name(&self, cx: &App) -> String {
        self.target.read(cx).text().trim().to_owned()
    }

    /// Where the file goes: the typed database, or for SQLite the file restored into.
    fn database(&self, cx: &App) -> Option<String> {
        if self.engine == DumpEngine::Sqlite {
            sqlite_target(&self.connection.database, self.into_new_file, self.new_file.as_deref())
        } else {
            Some(self.name(cx)).filter(|name| !name.is_empty())
        }
    }

    /// The target as the dialog's lines name it: a SQLite file by its name.
    fn shown(&self, database: &str) -> String {
        if self.engine == DumpEngine::Sqlite { file_name_of(database).to_owned() } else { database.to_owned() }
    }

    /// Development only: whether Import can be pressed.
    #[cfg(debug_assertions)]
    pub(crate) fn ready_for_development(&self, cx: &App) -> bool {
        self.ready(cx)
    }

    fn ready(&self, cx: &App) -> bool {
        !self.job.running() && self.path.is_some() && self.database(cx).is_some()
    }

    /// What the core is asked to restore, as `job_id`; `None` until a file and a target are chosen.
    fn request(&self, job_id: String, cx: &App) -> Option<ImportRequest> {
        Some(ImportRequest {
            job_id,
            connection_id: self.connection.id.clone(),
            database: self.database(cx)?,
            path: self.path.clone()?,
            options: RestoreOptions { drop_existing: self.engine == DumpEngine::Mongodb && self.drop_existing },
        })
    }

    pub(crate) fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = pick_file(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.path = Some(path.to_string_lossy().into_owned());
                this.focus(window);
                this.target_changed(cx);
            });
        })
        .detach();
    }

    /// SQLite: where the new file goes, starting beside the connection's.
    fn choose_new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let own = Path::new(&self.connection.database);
        let folder = own.parent().filter(|folder| folder.is_dir()).map_or_else(documents_dir, Path::to_path_buf);
        let suggested = format!("{}-restored.db", sqlite_name(&self.connection.database));
        let picked = pick_new_path(&folder, &suggested, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.new_file = Some(path.to_string_lossy().into_owned());
                this.focus(window);
                this.target_changed(cx);
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
        let Some(request) = self.request(String::new(), cx) else { return };
        self.confirming = false;
        self.outcome = None;
        let (name, path) = (self.shown(&request.database), request.path.clone());
        let job_id = self.job.start(Self::job, cx);
        let request = ImportRequest { job_id, ..request };
        self.set_busy(true, window, cx);
        tick_while(cx, |this: &Self| this.job.running());
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
                        if this.engine != DumpEngine::Sqlite {
                            this.list_databases(cx);
                        }
                        cx.emit(DumpEvent::Imported);
                        Outcome { tone: Tone::Done, title, detail: Some(detail), mono }
                    }
                    Err(error) => {
                        // MySQL's and MongoDB's restores aren't one transaction: what ran stays.
                        let (stopped, failed) = match this.engine {
                            DumpEngine::Mysql | DumpEngine::Mongodb => {
                                (m.import_stopped_partial, m.import_failed_partial)
                            }
                            DumpEngine::Postgres | DumpEngine::Sqlite => (m.import_stopped, m.import_failed),
                        };
                        ended(&error.to_string(), stopped, failed)
                    }
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
        if let Some(tool) = &self.tool {
            tool.update(cx, |tool, cx| tool.set_disabled(busy, cx));
        }
        if !busy {
            self.focus(window);
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.job.cancel(cx);
        cx.emit(DismissEvent);
    }

    /// Where the file goes: a database typed on the server, or SQLite's file.
    fn render_target(
        &self,
        theme: &Theme,
        m: &DumpMessages,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = self.job.running();
        if self.engine == DumpEngine::Sqlite {
            let pick: fn(&mut Self, bool, &mut Context<Self>) = |this, into_new_file, cx| {
                this.into_new_file = into_new_file;
                this.target_changed(cx);
            };
            let items = [(false, m.into_this_file), (true, m.into_new_file)];
            let (shown, muted) = match (self.into_new_file, &self.new_file) {
                (false, _) => (self.connection.database.clone(), false),
                (true, Some(file)) => (file.clone(), false),
                (true, None) => (m.new_file_placeholder.to_owned(), true),
            };
            let can_choose = self.into_new_file && !running;
            let file_row = div().flex().items_center().gap(px(10.)).child(path_line(theme, shown, muted)).child(
                enabled(button(theme, None, m.choose), can_choose)
                    .id("dump-choose-new-file")
                    .when(can_choose, |choose| {
                        choose.on_click(cx.listener(|this, _, window, cx| this.choose_new_file(window, cx)))
                    }),
            );
            let text = if self.into_new_file { m.into_new_file_note } else { m.into_this_file_note };
            return form_section(theme, m.restore_into)
                .child(choice(theme, "dump-sqlite-target", &items, self.into_new_file, running, pick, cx))
                .child(file_row)
                .child(one_line_note(theme, text))
                .into_any_element();
        }
        let name = self.name(cx);
        let target = field(
            label(theme, "dump-target", (m.database_on)(&self.connection.name), None),
            text_box(&self.target, false, true, window, cx),
            Some(help(theme, target_note(&name, self.databases.as_deref(), m), None)),
        );
        if self.engine != DumpEngine::Mongodb {
            return target.into_any_element();
        }
        let drop_existing = Checkbox::new("dump-drop-existing", m.drop_existing, self.drop_existing)
            .help(m.drop_existing_description, None)
            .disabled(running)
            .on_change(cx.listener(|this, on: &bool, _, cx| {
                this.drop_existing = *on;
                this.target_changed(cx);
            }));
        div().flex().flex_col().gap(px(14.)).child(target).child(drop_existing).into_any_element()
    }

    /// What restoring can do, and to trust the file: psql's runs shell commands with `\!`.
    fn render_safety(&self, theme: &Theme, m: &DumpMessages) -> Div {
        let text = match self.engine {
            DumpEngine::Postgres => format!("{}\\!{}", m.import_note_before, m.import_note_after),
            DumpEngine::Mysql => m.import_note_mysql.to_owned(),
            DumpEngine::Mongodb => m.import_note_mongodb.to_owned(),
            DumpEngine::Sqlite => m.import_note_sqlite.to_owned(),
        };
        // One paragraph, wrapping in the dialog's width.
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().mt(px(2.)).child(icon("info", theme.text3, 13.)))
            .child(note(theme, text).flex_1().min_w_0())
    }
}

impl Render for ImportDatabaseDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let running = self.job.running();
        let shown = self.database(cx).map(|database| self.shown(&database)).unwrap_or_default();
        let tag = confirming_tag_of(cx, &self.connection.id);
        let file = form_section(&theme, m.file).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(path_line(
                    &theme,
                    self.path.clone().unwrap_or_else(|| file_placeholder(self.engine, m).into()),
                    self.path.is_none(),
                ))
                .child(
                    enabled(button(&theme, None, m.choose), !running).id("dump-choose-file").when(!running, |choose| {
                        choose.on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)))
                    }),
                ),
        );
        let confirming = (self.confirming && tag.is_some()).then(|| Outcome {
            tone: Tone::Warning,
            title: (m.confirm_import)(&self.connection.name, tag.as_deref().unwrap_or_default()),
            detail: Some((m.confirm_import_detail)(tag.as_deref().unwrap_or_default())),
            mono: false,
        });
        let status = self.job.status((m.importing_into)(&shown), confirming.as_ref().or(self.outcome.as_ref()));
        let body = modal_body("dump-import-body")
            .child(file)
            .child(self.render_target(&theme, m, window, cx))
            .child(self.render_safety(&theme, m))
            .children(self.tool.clone());
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
mod tests;
