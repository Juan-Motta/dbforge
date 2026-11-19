//! The export dialog, as the former web UI's `ExportDialog.tsx`: where the rows
//! come from (and, for a filtered table, whether only the rows the filter
//! keeps), the format, its options, the columns, then the export to a file
//! the OS's save dialog names, or (for rows already shown) to the
//! clipboard. A table streams from the server, its rows counted as they
//! go, and can be stopped; the box under the form says how it goes and how
//! it ended, its room kept so nothing jumps. Enter exports; Escape stops
//! and closes.

use std::path::PathBuf;
use std::time::Instant;

use dbforge_core::Event;
use dbforge_core::explorer::table_structure;
use dbforge_core::export::{export_rows, export_table, format_rows};
use dbforge_core::files::reveal_file;
use gpui::{
    AnyElement, App, AppContext, BorrowAppContext, ClipboardItem, Context, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::{
    Columns, ExportChoice, ExportSource, FORMATS, Format, FormatOptions, JsonStyle, ROWS_PER_INSERT, columns_summary,
    default_file_name, description, export_options, read_choice, toggle_column, write_choice,
};
use crate::backend::Backend;
use crate::events::EventHub;
use crate::i18n::{ExportMessages, Translate, thousands};
use crate::preferences::{Preferences, keys};
use crate::table::filter::is_filtered;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::callout::{Callout, Tone, callout_space, elapsed, tick_while};
use crate::ui::field::{field, label, text_box};
use crate::ui::file_prompt::{documents_dir, pick_new_path};
use crate::ui::modal::{form_section, modal, modal_body, modal_footer, modal_status, note};
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::{
    Checkbox, Select, SelectEvent, SelectOption, TextInput, TextInputEvent, button, enabled, icon, primary_button,
};

actions!(export_dialog, [Submit, Dismiss]);

const CONTEXT: &str = "ExportDialog";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Submit, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// How the export goes, and how it ended.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    Idle,
    Working { rows: u64, started: Instant },
    Saved { rows: u64, path: String },
    Copied,
    Stopped,
    Failed(String),
}

/// A filtered table's rows: those the filter keeps, or every row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Filtered,
    All,
}

/// The table export running now, and whether the user stopped it.
struct Job {
    id: String,
    stopped: bool,
}

pub struct ExportDialog {
    source: ExportSource,
    choice: ExportChoice,
    status: Status,
    scope: Scope,
    columns: Columns,
    /// The columns the user kept; every one until some are left out.
    chosen: Option<Vec<String>>,
    picking_columns: bool,
    job: Option<Job>,
    header: Entity<Select<bool>>,
    null_text: Entity<TextInput>,
    json_style: Entity<Select<JsonStyle>>,
    rows_per_insert: Entity<Select<u32>>,
    focus: FocusHandle,
    listening: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ExportDialog {}

impl Focusable for ExportDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ExportDialog {
    pub fn new(source: ExportSource, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().export;
        let choice = read_choice(Preferences::global(cx).get(keys::EXPORT));
        let options = &choice.options;
        let header = cx.new(|cx| {
            let options = vec![SelectOption::new(true, m.header_include), SelectOption::new(false, m.header_leave_out)];
            Select::new(options, Some(choice.options.header), cx)
        });
        let null_text = cx.new(|cx| {
            let mut input = TextInput::new(m.null_text_placeholder, cx);
            input.set_text(&options.null_text, cx);
            input
        });
        let json_style = cx.new(|cx| {
            let options = vec![
                SelectOption::new(JsonStyle::Array, m.json_array),
                SelectOption::new(JsonStyle::Lines, m.json_lines),
            ];
            Select::new(options, Some(choice.options.json_style), cx)
        });
        let rows_per_insert = cx.new(|cx| {
            let options = ROWS_PER_INSERT
                .iter()
                .map(|count| SelectOption::new(*count, (m.rows_per_insert_option)(*count)))
                .collect();
            Select::new(options, Some(choice.options.rows_per_insert), cx)
        });
        let subscriptions = vec![
            cx.subscribe(&header, |this, _, SelectEvent(header): &SelectEvent<bool>, cx| {
                this.set_options(FormatOptions { header: *header, ..this.choice.options.clone() }, cx);
            }),
            cx.subscribe(&json_style, |this, _, SelectEvent(style): &SelectEvent<JsonStyle>, cx| {
                this.set_options(FormatOptions { json_style: *style, ..this.choice.options.clone() }, cx);
            }),
            cx.subscribe(&rows_per_insert, |this, _, SelectEvent(count): &SelectEvent<u32>, cx| {
                this.set_options(FormatOptions { rows_per_insert: *count, ..this.choice.options.clone() }, cx);
            }),
            cx.subscribe_in(&null_text, window, |this, input, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => {
                    let null_text = input.read(cx).text().to_owned();
                    this.set_options(FormatOptions { null_text, ..this.choice.options.clone() }, cx);
                }
                TextInputEvent::Submitted => this.submit(window, cx),
                TextInputEvent::Cancelled => this.close(cx),
                TextInputEvent::Pasted(_) => {}
            }),
        ];
        let columns = match &source {
            ExportSource::Rows { columns, .. } => {
                Columns::Known(columns.iter().map(|column| column.name.clone()).collect())
            }
            ExportSource::Table { .. } => Columns::Loading,
        };
        let mut dialog = Self {
            source,
            choice,
            status: Status::Idle,
            scope: Scope::Filtered,
            columns,
            chosen: None,
            picking_columns: false,
            job: None,
            header,
            null_text,
            json_style,
            rows_per_insert,
            focus: cx.focus_handle(),
            listening: None,
            _subscriptions: subscriptions,
        };
        dialog.read_columns(cx);
        dialog
    }

    /// Takes the keyboard, so Enter and Escape reach it.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    /// A table's column names, from its structure; unknown when it can't be read.
    fn read_columns(&mut self, cx: &mut Context<Self>) {
        let ExportSource::Table { connection_id, table, .. } = &self.source else { return };
        let (connection, table) = (connection_id.clone(), table.clone());
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            table_structure(core.connections(), core.statement_log(), connection, table).await
        });
        cx.spawn(async move |this, cx| {
            let structure = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.columns = match structure {
                    Ok(structure) => Columns::Known(structure.columns.into_iter().map(|column| column.name).collect()),
                    Err(_) => Columns::Unavailable,
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn name(&self) -> &str {
        match &self.source {
            ExportSource::Table { table, .. } => &table.name,
            ExportSource::Rows { name, .. } => name,
        }
    }

    fn filtered(&self) -> bool {
        matches!(&self.source, ExportSource::Table { filter, .. } if is_filtered(filter))
    }

    fn working(&self) -> bool {
        matches!(self.status, Status::Working { .. })
    }

    /// The chosen columns: every known one until some are left out.
    fn chosen_columns(&self) -> Vec<String> {
        self.chosen.clone().unwrap_or_else(|| self.columns.known().to_vec())
    }

    /// Development only: whether Export… can be pressed (the columns read first).
    #[cfg(debug_assertions)]
    pub(crate) fn ready_for_development(&self, _cx: &App) -> bool {
        self.ready() && self.columns != Columns::Loading
    }

    /// Nothing runs and, once the columns are known, at least one is chosen.
    fn ready(&self) -> bool {
        !self.working() && (!matches!(self.columns, Columns::Known(_)) || !self.chosen_columns().is_empty())
    }

    fn set_format(&mut self, format: Format, cx: &mut Context<Self>) {
        self.choice.format = format;
        self.status = Status::Idle;
        cx.notify();
    }

    fn set_options(&mut self, options: FormatOptions, cx: &mut Context<Self>) {
        self.choice.options = options;
        self.status = Status::Idle;
        cx.notify();
    }

    /// The form takes no changes while an export runs.
    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.header.update(cx, |select, cx| select.set_disabled(busy, cx));
        self.json_style.update(cx, |select, cx| select.set_disabled(busy, cx));
        self.rows_per_insert.update(cx, |select, cx| select.set_disabled(busy, cx));
        self.null_text.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        if !busy {
            self.focus(window);
        }
        cx.notify();
    }

    fn sent(&self) -> dbforge_core::export::ExportOptions {
        let chosen = self.chosen_columns();
        let all = match &self.columns {
            Columns::Known(all) => all.clone(),
            Columns::Loading | Columns::Unavailable => chosen.clone(),
        };
        export_options(&self.choice.options, &chosen, &all)
    }

    /// Enter: the footer's main button.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Saved { .. }) {
            cx.emit(DismissEvent);
        } else {
            self.export_to_file(window, cx);
        }
    }

    /// Asks where to save, then writes the rows there.
    pub(crate) fn export_to_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready() {
            return;
        }
        let picked = pick_new_path(&documents_dir(), &default_file_name(self.name(), self.choice.format), cx);
        cx.spawn_in(window, async move |this, cx| {
            // Cancelled: the dialog stays as it was.
            let Some(path) = picked.await else { return };
            let _ = this.update_in(cx, |this, window, cx| this.write(path, window, cx));
        })
        .detach();
    }

    fn write(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready() {
            return;
        }
        self.status = Status::Working { rows: 0, started: Instant::now() };
        self.set_busy(true, window, cx);
        tick_while(cx, Self::working);
        let (format, options, shown) = (self.choice.format, Some(self.sent()), path.display().to_string());
        let writing = match self.source.clone() {
            ExportSource::Rows { name, columns, rows } => Backend::global(cx).spawn(cx, move |_| async move {
                export_rows(path, format, options, name, columns, rows.to_vec()).await
            }),
            ExportSource::Table { connection_id, table, filter } => {
                let job_id = uuid::Uuid::new_v4().to_string();
                self.job = Some(Job { id: job_id.clone(), stopped: false });
                self.listen(job_id.clone(), cx);
                let filter = (self.filtered() && self.scope == Scope::Filtered).then_some(filter);
                Backend::global(cx).spawn(cx, move |core| async move {
                    let (events, store, log, jobs) =
                        (core.events().as_ref(), core.connections(), core.statement_log(), core.jobs());
                    export_table(events, store, log, jobs, job_id, connection_id, table, filter, format, options, path)
                        .await
                })
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let written = writing.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let stopped = this.job.as_ref().is_some_and(|job| job.stopped);
                this.status = match written {
                    Ok(rows) => {
                        this.keep_choice(cx);
                        Status::Saved { rows, path: shown }
                    }
                    Err(_) if stopped => Status::Stopped,
                    Err(error) => Status::Failed(error.to_string()),
                };
                this.job = None;
                this.listening = None;
                this.set_busy(false, window, cx);
            });
        })
        .detach();
    }

    /// Follows the rows job `job_id` has written.
    fn listen(&mut self, job_id: String, cx: &mut Context<Self>) {
        self.listening = Some(cx.subscribe(&EventHub::global(cx), move |this, _, event: &Event, cx| {
            if let Event::ExportProgress { job_id: id, rows } = event
                && *id == job_id
                && let Status::Working { rows: written, .. } = &mut this.status
            {
                *written = *rows;
                cx.notify();
            }
        }));
    }

    /// The rows shown, in the format, on the clipboard.
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ExportSource::Rows { name, columns, rows } = self.source.clone() else { return };
        if !self.ready() {
            return;
        }
        self.status = Status::Working { rows: 0, started: Instant::now() };
        self.set_busy(true, window, cx);
        let (format, options) = (self.choice.format, Some(self.sent()));
        let formatting =
            Backend::global(cx).spawn_blocking(cx, move |_| format_rows(format, options, name, columns, rows.to_vec()));
        cx.spawn_in(window, async move |this, cx| {
            let text = formatting.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.status = match text {
                    Ok(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.keep_choice(cx);
                        Status::Copied
                    }
                    Err(error) => Status::Failed(error.to_string()),
                };
                this.set_busy(false, window, cx);
            });
        })
        .detach();
    }

    /// The format and options that worked, offered first next time.
    fn keep_choice(&self, cx: &mut Context<Self>) {
        let kept = write_choice(&self.choice);
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::EXPORT, Some(kept)));
    }

    /// Stops the running table export; it then ends as stopped.
    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.job.as_mut().filter(|job| !job.stopped) else { return };
        job.stopped = true;
        Backend::global(cx).core().jobs().cancel(&job.id);
    }

    /// Escape or Cancel: stops a running export, then closes.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        cx.emit(DismissEvent);
    }

    fn reveal(&mut self, path: String, cx: &mut Context<Self>) {
        let revealing = Backend::global(cx).spawn_blocking(cx, move |_| reveal_file(PathBuf::from(path)));
        cx.spawn(async move |this, cx| {
            if let Err(error) = revealing.await {
                let _ = this.update(cx, |this, cx| {
                    this.status = Status::Failed(error.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn render_source(&self, theme: &Theme, m: &ExportMessages) -> AnyElement {
        let (icon_name, strong, muted) = match &self.source {
            ExportSource::Table { table, .. } => (
                "table",
                format!("{}.{}", table.schema, table.name),
                (!self.filtered()).then(|| format!("· {}", m.every_row)),
            ),
            ExportSource::Rows { rows, .. } => (
                "code",
                m.query_result.to_owned(),
                Some(format!("· {}", (m.rows_shown)(&thousands(rows.len() as u64)))),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(icon(icon_name, theme.text3, 15.))
            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(strong))
            .children(muted.map(|muted| div().flex_none().text_color(theme.text3).child(muted)))
            .into_any_element()
    }

    fn render_scope(&self, theme: &Theme, m: &ExportMessages, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.filtered() {
            return None;
        }
        let working = self.working();
        let row = |scope: Scope, icon_name: &'static str, title: &'static str, text: &'static str| {
            OptionItem::new(("export-scope", scope as usize), Mark::Radio, title, self.scope == scope)
                .icon(icon_name)
                .description(text)
                .disabled(working)
                .on_select(cx.listener(move |this, _, _, cx| {
                    this.scope = scope;
                    cx.notify();
                }))
                .into_any_element()
        };
        let rows = vec![
            row(Scope::Filtered, "search", m.scope_filtered, m.scope_filtered_description),
            row(Scope::All, "table", m.scope_all, m.scope_all_description),
        ];
        Some(form_section(theme, m.scope).child(option_list(theme, rows)).into_any_element())
    }

    fn render_formats(&self, theme: &Theme, m: &ExportMessages, cx: &mut Context<Self>) -> AnyElement {
        let working = self.working();
        let tiles = FORMATS.iter().enumerate().map(|(index, info)| {
            let format = info.format;
            let on = self.choice.format == format;
            enabled(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .min_w_0()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if on { theme.accent } else { theme.line })
                    .when(on, |tile| tile.bg(theme.accent.opacity(0.1)))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .bg(theme.sel)
                            .child(icon(info.icon, if on { theme.accent } else { theme.text2 }, 15.)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(5.))
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(info.label)
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_family(MONO)
                                            .text_color(theme.text3)
                                            .child(format!(".{}", info.extension)),
                                    ),
                            )
                            .child(div().text_size(px(12.)).text_color(theme.text3).truncate().child(description(
                                format,
                                self.choice.options.json_style,
                                m,
                            ))),
                    ),
                !working,
            )
            .id(("export-format", index))
            .when(!working, |tile| tile.on_click(cx.listener(move |this, _, _, cx| this.set_format(format, cx))))
        });
        form_section(theme, m.format).child(div().grid().grid_cols(3).gap(px(8.)).children(tiles)).into_any_element()
    }

    fn render_options(&self, theme: &Theme, m: &ExportMessages, window: &Window, cx: &App) -> AnyElement {
        let row = || div().min_h(px(54.)).grid().grid_cols(2).gap(px(12.));
        let options = match self.choice.format {
            Format::Csv | Format::Tsv => row()
                .child(field(label(theme, "export-header", m.header, None), self.header.clone(), None))
                .child(field(
                    label(theme, "export-null", m.null_text, None),
                    text_box(&self.null_text, false, true, window, cx),
                    None,
                )),
            Format::Json => {
                row().child(field(label(theme, "export-json", m.json_layout, None), self.json_style.clone(), None))
            }
            Format::Sql => row().child(field(
                label(theme, "export-rows-per-insert", m.rows_per_insert, None),
                self.rows_per_insert.clone(),
                None,
            )),
            Format::Markdown => div().min_h(px(54.)).flex().items_center().child(note(theme, m.no_options)),
        };
        form_section(theme, m.options).child(options).into_any_element()
    }

    fn render_columns(&self, theme: &Theme, m: &ExportMessages, cx: &mut Context<Self>) -> AnyElement {
        let working = self.working();
        let chosen = self.chosen_columns();
        let known = matches!(self.columns, Columns::Known(_));
        let none_chosen = known && chosen.is_empty();
        let link = |id: &'static str, text: &'static str| {
            enabled(div().text_size(px(12.5)).text_color(theme.accent), !working).id(id).child(text)
        };
        let mut summary = div().flex().items_center().gap(px(10.)).child(icon("column", theme.text3, 15.)).child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.5))
                .text_color(if none_chosen { theme.err } else { theme.text2 })
                .child(columns_summary(&self.columns, &chosen, m)),
        );
        if self.picking_columns {
            let all = self.columns.known().to_vec();
            summary = summary
                .child(link("export-columns-all", m.select_all).when(!working, |link| {
                    link.on_click(cx.listener(move |this, _, _, cx| {
                        this.chosen = Some(all.clone());
                        cx.notify();
                    }))
                }))
                .child(link("export-columns-none", m.select_none).when(!working, |link| {
                    link.on_click(cx.listener(|this, _, _, cx| {
                        this.chosen = Some(Vec::new());
                        cx.notify();
                    }))
                }));
        }
        if known {
            let text = if self.picking_columns { m.hide_columns } else { m.choose_columns };
            summary = summary.child(button(theme, None, text).id("export-columns-toggle").cursor_pointer().on_click(
                cx.listener(|this, _, _, cx| {
                    this.picking_columns = !this.picking_columns;
                    cx.notify();
                }),
            ));
        }
        let list = (self.picking_columns && known).then(|| {
            let known = self.columns.known().to_vec();
            div()
                .id("export-column-list")
                .max_h(px(168.))
                .overflow_y_scroll()
                .px(px(12.))
                .py(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(theme.line)
                .child(div().grid().grid_cols(3).gap_x(px(12.)).gap_y(px(4.)).children(known.iter().enumerate().map(
                    |(index, name)| {
                        let (name, all) = (name.clone(), known.clone());
                        // A query result may have two columns of one name.
                        Checkbox::new(("export-column", index), name.clone(), chosen.contains(&name))
                            .disabled(working)
                            .on_change(cx.listener(move |this, on: &bool, _, cx| {
                                let chosen = this.chosen_columns();
                                this.chosen = Some(toggle_column(&all, &chosen, &name, *on));
                                cx.notify();
                            }))
                    },
                )))
        });
        form_section(theme, m.columns).child(summary).children(list).into_any_element()
    }

    fn render_status(&self, m: &ExportMessages) -> AnyElement {
        let streams = matches!(self.source, ExportSource::Table { .. });
        match &self.status {
            Status::Idle => callout_space().into_any_element(),
            // Rows already in the window are written at once; a table's are counted as they go.
            Status::Working { rows, started } => Callout::new("export-status", Tone::Progress, m.exporting)
                .aside(elapsed(*started))
                .detail(if streams { (m.rows_written)(&thousands(*rows)) } else { " ".into() })
                .into_any_element(),
            Status::Saved { rows, path } => {
                Callout::new("export-status", Tone::Done, (m.exported_rows)(&thousands(*rows)))
                    .detail(path.clone())
                    .mono(true)
                    .into_any_element()
            }
            Status::Copied => Callout::new("export-status", Tone::Done, m.copied).into_any_element(),
            Status::Stopped => {
                Callout::new("export-status", Tone::Stopped, m.stopped).detail(m.stopped_detail).into_any_element()
            }
            Status::Failed(error) => Callout::new("export-status", Tone::Failed, m.failed)
                .detail(error.clone())
                .mono(true)
                .into_any_element(),
        }
    }

    fn render_footer(&self, theme: &Theme, m: &ExportMessages, cx: &mut Context<Self>) -> AnyElement {
        let footer = modal_footer(theme);
        if let Status::Saved { path, .. } = &self.status {
            let path = path.clone();
            return footer
                .child(
                    button(theme, Some("folder"), (m.reveal)(cfg!(target_os = "macos")))
                        .id("export-reveal")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.reveal(path.clone(), cx))),
                )
                .child(
                    primary_button(theme, m.done)
                        .id("export-done")
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                )
                .into_any_element();
        }
        let (working, ready) = (self.working(), self.ready());
        let streams = matches!(self.source, ExportSource::Table { .. });
        let leading = if working && streams {
            button(theme, Some("stop"), m.stop)
                .id("export-stop")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            button(theme, None, m.cancel)
                .id("export-cancel")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
        };
        let copy = (!streams).then(|| {
            enabled(button(theme, Some("duplicate"), m.copy_to_clipboard), ready)
                .id("export-copy")
                .when(ready, |copy| copy.on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))))
        });
        let primary = enabled(primary_button(theme, if working { m.exporting } else { m.export }), ready)
            .id("export-run")
            .when(ready, |run| run.on_click(cx.listener(|this, _, window, cx| this.export_to_file(window, cx))));
        footer.child(leading).children(copy).child(primary).into_any_element()
    }
}

impl Render for ExportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().export;
        let body = modal_body("export-body")
            .child(self.render_source(&theme, m))
            .children(self.render_scope(&theme, m, cx))
            .child(self.render_formats(&theme, m, cx))
            .child(self.render_options(&theme, m, window, cx))
            .child(self.render_columns(&theme, m, cx));
        modal(&theme, 640., m.title)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, window, cx| this.submit(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
            .child(body)
            .child(modal_status(self.render_status(m)))
            .child(self.render_footer(&theme, m, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dbforge_core::drivers::Column;
    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup, wait_until};

    fn column(name: &str) -> Column {
        Column { name: name.into(), data_type: "text".into(), primary_key: false, generated: false, default: None }
    }

    fn rows_source() -> ExportSource {
        let rows = vec![vec![Some("1".into()), Some("Ann".into())], vec![Some("2".into()), None]];
        ExportSource::Rows {
            name: "query_result".into(),
            columns: vec![column("id"), column("name")],
            rows: Arc::new(rows),
        }
    }

    #[gpui::test]
    fn exports_rows_to_the_file_chosen_and_keeps_the_choice(cx: &mut TestAppContext) {
        let dir = setup(cx);
        let (dialog, cx) = cx.add_window_view(|window, cx| ExportDialog::new(rows_source(), window, cx));
        draw(cx);
        let path = dir.path().join("out.csv");
        dialog.update_in(cx, |dialog, window, cx| {
            dialog.chosen = Some(vec!["name".into()]);
            dialog.write(path.clone(), window, cx);
        });
        wait_until(cx, "the export", |cx| dialog.read_with(cx, |dialog, _| !dialog.working()));
        draw(cx);
        dialog.read_with(cx, |dialog, _| {
            assert_eq!(dialog.status, Status::Saved { rows: 2, path: path.display().to_string() });
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name\nAnn\n\n");
        cx.update(|_, cx| {
            assert_eq!(
                Preferences::global(cx).get(keys::EXPORT),
                Some(write_choice(&ExportChoice::default()).as_str()),
                "the choice that worked is kept"
            );
        });
    }

    #[gpui::test]
    fn needs_a_column_once_they_are_known(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (dialog, cx) = cx.add_window_view(|window, cx| ExportDialog::new(rows_source(), window, cx));
        dialog.update(cx, |dialog, cx| {
            assert!(dialog.ready());
            dialog.chosen = Some(Vec::new());
            assert!(!dialog.ready(), "no column chosen");
            dialog.columns = Columns::Unavailable;
            assert!(dialog.ready(), "unknown columns: every one is exported");
            dialog.set_format(Format::Markdown, cx);
        });
        draw(cx);
    }
}
