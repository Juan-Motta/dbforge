//! The import dialog, as the former web UI's `ImportDialog.tsx`: a file picked with
//! the OS's dialog, its format (guessed from its name) and how it is read,
//! its first rows, and the column each field goes into; then its rows are
//! added, counted as they go, and the import can be stopped. On a
//! connection whose tag asks first (PROD) the first press of Import only
//! warns, naming the tag and the table; the second imports. The box under
//! the form says how it goes and how it ended, its room kept so nothing
//! jumps. Enter presses Import; Escape stops and closes.

use std::path::PathBuf;
use std::time::Instant;

use dbforge_core::Event;
use dbforge_core::drivers::RowFilter;
use dbforge_core::explorer::fetch_rows;
use dbforge_core::import::{ImportRequest, Preview, ReadOptions, import_table, preview_import};
use gpui::{
    AnyElement, App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::{ImportFormat, ImportTarget, format_of_path, map_by_name, offered_columns, repeated_columns};
use crate::backend::Backend;
use crate::connections::engine::is_relational;
use crate::events::EventHub;
use crate::i18n::{ImportMessages, Translate, thousands};
use crate::tags::confirming_tag_of;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::callout::{Callout, Tone, callout_space, elapsed, tick_while};
use crate::ui::field::{field, label, text_box};
use crate::ui::file_prompt::pick_file;
use crate::ui::modal::{form_section, modal, modal_body, modal_footer, modal_status, note, path_tail};
use crate::ui::sheet::sheet;
use crate::ui::{Select, SelectEvent, SelectOption, TextInput, TextInputEvent, button, enabled, icon, primary_button};

actions!(import_dialog, [Submit, Dismiss]);

const CONTEXT: &str = "ImportDialog";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("enter", Submit, Some(CONTEXT)), KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// Rows the preview shows.
const PREVIEW_ROWS: usize = 5;
/// A preview column's width.
const PREVIEW_COLUMN: f32 = 140.;

/// The formats, in the order the dialog shows them, with their icons.
const FORMATS: [(ImportFormat, &str, &str); 3] =
    [(ImportFormat::Csv, "CSV", "table"), (ImportFormat::Tsv, "TSV", "column"), (ImportFormat::Json, "JSON", "code")];

/// What the dialog tells its owner, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportEvent {
    /// Rows went in: whatever shows the table's rows reads them again.
    Imported,
}

/// How the import goes, and how it ended.
#[derive(Debug, Clone, PartialEq)]
enum Status {
    Idle,
    /// A connection whose tag asks first: the next press imports.
    Confirming,
    Working {
        rows: u64,
        started: Instant,
    },
    Done(u64),
    Stopped(u64),
    Failed(String),
}

/// The running import: whether the user stopped it, and the last count it reported.
struct Job {
    id: String,
    stopped: bool,
    rows: u64,
}

/// The column each field goes into, as the user chose it for these fields.
struct Chosen {
    fields: Vec<String>,
    mapping: Vec<Option<String>>,
}

pub struct ImportDialog {
    target: ImportTarget,
    /// SQL engines import in one transaction; document databases a batch at a time.
    transactional: bool,
    path: Option<String>,
    format: ImportFormat,
    header: Entity<Select<bool>>,
    null_text: Entity<TextInput>,
    delimiter: Entity<Select<char>>,
    /// The file's fields and first rows as last read, or why they couldn't be.
    preview: Option<Result<Preview, String>>,
    /// The file is being read again; what was read stays shown until it is.
    reading: bool,
    request: u64,
    /// The columns a value can go into; `None` while they are read.
    table_columns: Option<Vec<String>>,
    chosen: Option<Chosen>,
    /// A dropdown per field, for the fields and columns they were made for.
    pickers: Vec<Entity<Select<Option<String>>>>,
    pickers_for: (Vec<String>, Vec<String>),
    picker_events: Vec<Subscription>,
    status: Status,
    job: Option<Job>,
    focus: FocusHandle,
    listening: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ImportDialog {}
impl EventEmitter<ImportEvent> for ImportDialog {}

impl Focusable for ImportDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ImportDialog {
    pub fn new(target: ImportTarget, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = &cx.messages().import;
        let defaults = ReadOptions::default();
        let header = cx.new(|cx| {
            let options = vec![SelectOption::new(true, m.header_names), SelectOption::new(false, m.header_data)];
            Select::new(options, Some(defaults.header), cx)
        });
        let null_text = cx.new(|cx| TextInput::new(m.null_text_placeholder, cx));
        let delimiter = cx.new(|cx| {
            let options = vec![
                SelectOption::new(',', m.comma),
                SelectOption::new(';', m.semicolon),
                SelectOption::new('|', m.pipe),
            ];
            Select::new(options, Some(defaults.delimiter), cx)
        });
        let subscriptions = vec![
            cx.subscribe(&header, |this, _, _: &SelectEvent<bool>, cx| this.options_changed(cx)),
            cx.subscribe(&delimiter, |this, _, _: &SelectEvent<char>, cx| this.options_changed(cx)),
            cx.subscribe_in(&null_text, window, |this, _, event: &TextInputEvent, window, cx| match event {
                TextInputEvent::Changed => this.options_changed(cx),
                TextInputEvent::Submitted => this.start(window, cx),
                TextInputEvent::Cancelled => this.close(cx),
                TextInputEvent::Pasted(_) => {}
            }),
        ];
        let transactional = is_relational(target.connection.engine);
        let mut dialog = Self {
            target,
            transactional,
            path: None,
            format: ImportFormat::Csv,
            header,
            null_text,
            delimiter,
            preview: None,
            reading: false,
            request: 0,
            table_columns: None,
            chosen: None,
            pickers: Vec::new(),
            pickers_for: (Vec::new(), Vec::new()),
            picker_events: Vec::new(),
            status: Status::Idle,
            job: None,
            focus: cx.focus_handle(),
            listening: None,
            _subscriptions: subscriptions,
        };
        dialog.read_table_columns(cx);
        dialog
    }

    /// Takes the keyboard, so Enter and Escape reach it.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    /// The columns a value can be imported into: all but the ones the database
    /// computes, from one row read for them. Without them no column can be
    /// offered; a document database still takes the file's fields.
    fn read_table_columns(&mut self, cx: &mut Context<Self>) {
        let (connection, table) = (self.target.connection.id.clone(), self.target.table.clone());
        let reading = Backend::global(cx).spawn(cx, move |core| async move {
            fetch_rows(core.connections(), core.statement_log(), connection, table, Some(RowFilter::default()), 0, 1)
                .await
        });
        cx.spawn(async move |this, cx| {
            let page = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.table_columns = Some(match page {
                    Ok(page) => page.columns.into_iter().filter(|c| !c.generated).map(|c| c.name).collect(),
                    Err(_) => Vec::new(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn read_options(&self, cx: &App) -> ReadOptions {
        let defaults = ReadOptions::default();
        ReadOptions {
            header: self.header.read(cx).value().copied().unwrap_or(defaults.header),
            null_text: self.null_text.read(cx).text().to_owned(),
            delimiter: self.delimiter.read(cx).value().copied().unwrap_or(defaults.delimiter),
        }
    }

    fn options_changed(&mut self, cx: &mut Context<Self>) {
        self.unconfirm();
        self.read_file(cx);
    }

    /// A change to what would be imported asks again on a connection that asks first.
    fn unconfirm(&mut self) {
        if self.status == Status::Confirming {
            self.status = Status::Idle;
        }
    }

    /// Reads the file's fields and first rows again, for the file, format and
    /// options now; an older reply that comes late is dropped.
    fn read_file(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return };
        self.request += 1;
        let request = self.request;
        self.reading = true;
        let (format, options) = (self.format, self.read_options(cx));
        let reading =
            Backend::global(cx).spawn_blocking(cx, move |_| preview_import(PathBuf::from(path), format, Some(options)));
        cx.spawn(async move |this, cx| {
            let preview = reading.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                this.reading = false;
                this.preview = Some(preview.map_err(|error| error.to_string()));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Development only: picks the file `DBFORGE_PICK` names, as Choose… would.
    #[cfg(debug_assertions)]
    pub(crate) fn pick_for_development(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick(window, cx);
    }

    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = pick_file(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let path = path.to_string_lossy().into_owned();
                this.format = format_of_path(&path);
                this.path = Some(path);
                this.preview = None;
                this.status = Status::Idle;
                this.read_file(cx);
                this.focus(window);
            });
        })
        .detach();
    }

    fn set_format(&mut self, format: ImportFormat, cx: &mut Context<Self>) {
        if self.format != format {
            self.format = format;
            self.unconfirm();
            self.read_file(cx);
        }
    }

    /// The file's fields, once read.
    fn fields(&self) -> &[String] {
        match &self.preview {
            Some(Ok(preview)) => &preview.fields,
            _ => &[],
        }
    }

    /// The columns offered, once the table's are known.
    fn columns(&self) -> Option<Vec<String>> {
        let table = self.table_columns.as_ref()?;
        Some(offered_columns(table, self.fields(), self.transactional))
    }

    /// Chosen by the user for these fields; until then, by name.
    fn mapping(&self) -> Vec<Option<String>> {
        let fields = self.fields();
        match (&self.chosen, self.columns()) {
            (Some(chosen), _) if chosen.fields == fields => chosen.mapping.clone(),
            (_, Some(columns)) => map_by_name(fields, &columns),
            (_, None) => Vec::new(),
        }
    }

    fn working(&self) -> bool {
        matches!(self.status, Status::Working { .. })
    }

    fn read(&self) -> bool {
        !self.reading && matches!(self.preview, Some(Ok(_)))
    }

    /// The file is read, the columns known, at least one field mapped, and no column taken twice.
    pub(crate) fn ready(&self, _cx: &App) -> bool {
        let mapping = self.mapping();
        !self.working()
            && self.read()
            && self.columns().is_some()
            && mapping.iter().any(Option::is_some)
            && repeated_columns(&mapping).is_empty()
    }

    fn set_column(&mut self, field: usize, column: Option<String>, cx: &mut Context<Self>) {
        let mut mapping = self.mapping();
        if let Some(slot) = mapping.get_mut(field) {
            *slot = column;
        }
        self.chosen = Some(Chosen { fields: self.fields().to_vec(), mapping });
        self.unconfirm();
        cx.notify();
    }

    /// Makes a dropdown per field when the fields or the columns change, and
    /// shows the mapping in them.
    fn sync_pickers(&mut self, cx: &mut Context<Self>) {
        let Some(columns) = self.columns() else { return };
        let fields = self.fields().to_vec();
        let mapping = self.mapping();
        if self.pickers_for != (fields.clone(), columns.clone()) {
            let leave_out = cx.messages().import.leave_out;
            let options: Vec<SelectOption<Option<String>>> = std::iter::once(SelectOption::new(None, leave_out))
                .chain(columns.iter().map(|column| SelectOption::new(Some(column.clone()), column.clone())))
                .collect();
            self.pickers = fields.iter().map(|_| cx.new(|cx| Select::new(options.clone(), None, cx))).collect();
            self.picker_events = self
                .pickers
                .iter()
                .enumerate()
                .map(|(index, picker)| {
                    cx.subscribe(picker, move |this, _, SelectEvent(column): &SelectEvent<Option<String>>, cx| {
                        this.set_column(index, column.clone(), cx);
                    })
                })
                .collect();
            self.pickers_for = (fields, columns);
        }
        let working = self.working();
        for (picker, column) in self.pickers.iter().zip(mapping) {
            picker.update(cx, |picker, cx| {
                picker.set_value(Some(column), cx);
                picker.set_disabled(working, cx);
            });
        }
    }

    /// Enter or Import: on a connection that asks first, the first press
    /// warns and the second imports; elsewhere it imports at once.
    pub(crate) fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ready(cx) {
            return;
        }
        if confirming_tag_of(cx, &self.target.connection.id).is_some() && self.status != Status::Confirming {
            self.status = Status::Confirming;
            cx.notify();
            return;
        }
        let (Some(path), Some(Ok(preview))) = (self.path.clone(), &self.preview) else { return };
        let job_id = uuid::Uuid::new_v4().to_string();
        let request = ImportRequest {
            job_id: job_id.clone(),
            connection_id: self.target.connection.id.clone(),
            table: self.target.table.clone(),
            path: PathBuf::from(path),
            format: self.format,
            options: Some(self.read_options(cx)),
            fields: preview.fields.clone(),
            columns: self.mapping(),
        };
        self.job = Some(Job { id: job_id.clone(), stopped: false, rows: 0 });
        self.status = Status::Working { rows: 0, started: Instant::now() };
        self.set_busy(true, window, cx);
        tick_while(cx, Self::working);
        self.listening = Some(cx.subscribe(&EventHub::global(cx), move |this, _, event: &Event, cx| {
            if let Event::ImportProgress { job_id: id, rows } = event
                && *id == job_id
            {
                if let Some(job) = &mut this.job {
                    job.rows = *rows;
                }
                if let Status::Working { rows: imported, .. } = &mut this.status {
                    *imported = *rows;
                }
                cx.notify();
            }
        }));
        let importing = Backend::global(cx).spawn(cx, move |core| async move {
            let (events, store, log, jobs) =
                (core.events().as_ref(), core.connections(), core.statement_log(), core.jobs());
            import_table(events, store, log, jobs, request).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let imported = importing.await;
            let _ = this.update_in(cx, |this, window, cx| this.finish(imported.map_err(|e| e.to_string()), window, cx));
        })
        .detach();
    }

    fn finish(&mut self, imported: Result<u64, String>, window: &mut Window, cx: &mut Context<Self>) {
        let (stopped, rows) = self.job.as_ref().map_or((false, 0), |job| (job.stopped, job.rows));
        self.status = match imported {
            Ok(rows) => {
                cx.emit(ImportEvent::Imported);
                Status::Done(rows)
            }
            Err(_) if stopped => {
                if !self.transactional && rows > 0 {
                    cx.emit(ImportEvent::Imported);
                }
                Status::Stopped(rows)
            }
            Err(error) => {
                // A document database keeps the rows in before the error; the error says how many.
                if !self.transactional {
                    cx.emit(ImportEvent::Imported);
                }
                Status::Failed(error)
            }
        };
        self.job = None;
        self.listening = None;
        self.set_busy(false, window, cx);
    }

    /// The form takes no changes while an import runs.
    fn set_busy(&mut self, busy: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.header.update(cx, |select, cx| select.set_disabled(busy, cx));
        self.delimiter.update(cx, |select, cx| select.set_disabled(busy, cx));
        self.null_text.update(cx, |input, cx| input.set_disabled(busy, window, cx));
        if !busy {
            self.focus(window);
        }
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.job.as_mut().filter(|job| !job.stopped) else { return };
        job.stopped = true;
        Backend::global(cx).core().jobs().cancel(&job.id);
    }

    /// Escape or Cancel: stops a running import, then closes.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        cx.emit(DismissEvent);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Done(_)) {
            cx.emit(DismissEvent);
        } else {
            self.start(window, cx);
        }
    }

    fn relation(&self) -> String {
        format!("{}.{}", self.target.table.schema, self.target.table.name)
    }

    fn render_file(&self, theme: &Theme, m: &ImportMessages, cx: &mut Context<Self>) -> AnyElement {
        let working = self.working();
        form_section(theme, m.file)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("folder", theme.text3, 15.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.5))
                            .font_family(MONO)
                            .text_color(if self.path.is_some() { theme.text } else { theme.text3 })
                            .child(self.path.as_deref().map_or_else(|| m.no_file.into(), |path| path_tail(path, 64))),
                    )
                    .child(
                        enabled(button(theme, None, m.choose), !working).id("import-choose").when(!working, |choose| {
                            choose.on_click(cx.listener(|this, _, window, cx| this.pick(window, cx)))
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_formats(&self, theme: &Theme, m: &ImportMessages, cx: &mut Context<Self>) -> AnyElement {
        let working = self.working();
        let tiles = FORMATS.iter().enumerate().map(|(index, (format, name, icon_name))| {
            let (format, on) = (*format, self.format == *format);
            let text = match format {
                ImportFormat::Csv => m.csv_description,
                ImportFormat::Tsv => m.tsv_description,
                ImportFormat::Json => m.json_description,
            };
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
                            .child(icon(icon_name, if on { theme.accent } else { theme.text2 }, 15.)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(*name))
                            .child(div().text_size(px(12.)).text_color(theme.text3).truncate().child(text)),
                    ),
                !working,
            )
            .id(("import-format", index))
            .when(!working, |tile| tile.on_click(cx.listener(move |this, _, _, cx| this.set_format(format, cx))))
        });
        form_section(theme, m.format).child(div().grid().grid_cols(3).gap(px(8.)).children(tiles)).into_any_element()
    }

    fn render_options(&self, theme: &Theme, m: &ImportMessages, window: &Window, cx: &App) -> AnyElement {
        let options = match self.format {
            ImportFormat::Json => div().min_h(px(54.)).flex().items_center().child(note(theme, m.json_note)),
            format => div()
                .min_h(px(54.))
                .grid()
                .grid_cols(if format == ImportFormat::Csv { 3 } else { 2 })
                .gap(px(12.))
                .child(field(label(theme, "import-header", m.header, None), self.header.clone(), None))
                .child(field(
                    label(theme, "import-null", m.null_text, None),
                    text_box(&self.null_text, false, true, window, cx),
                    None,
                ))
                .when(format == ImportFormat::Csv, |row| {
                    row.child(field(label(theme, "import-delimiter", m.delimiter, None), self.delimiter.clone(), None))
                }),
        };
        form_section(theme, m.options).child(options).into_any_element()
    }

    fn render_preview(&self, theme: &Theme, m: &ImportMessages) -> AnyElement {
        let body = match (&self.path, &self.preview) {
            (Some(_), Some(Ok(preview))) if !preview.rows.is_empty() => {
                let width = px(PREVIEW_COLUMN * preview.fields.len() as f32);
                let cell = |text: Option<&str>, tint| {
                    div()
                        .w(px(PREVIEW_COLUMN))
                        .flex_none()
                        .px(px(10.))
                        .truncate()
                        .text_color(tint)
                        .child(text.map_or_else(|| "NULL".to_owned(), crate::ui::sheet::one_line))
                };
                let header = div()
                    .h(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.line)
                    .font_weight(FontWeight::SEMIBOLD)
                    .children(preview.fields.iter().map(|name| cell(Some(name), theme.text2)));
                let rows =
                    div().children(preview.rows.iter().take(PREVIEW_ROWS).map(|row| {
                        div().h(px(24.)).flex().items_center().children(row.iter().map(|value| {
                            cell(value.as_deref(), if value.is_some() { theme.text } else { theme.text3 })
                        }))
                    }));
                sheet("import-preview", width, header, rows).into_any_element()
            }
            (None, _) => div().p(px(12.)).text_color(theme.text3).child(m.preview_empty).into_any_element(),
            (Some(_), None) => div().p(px(12.)).text_color(theme.text3).child(m.reading_file).into_any_element(),
            (Some(_), Some(Err(error))) => {
                div().p(px(12.)).text_color(theme.err).child(error.clone()).into_any_element()
            }
            (Some(_), Some(Ok(_))) => div().p(px(12.)).text_color(theme.text3).child(m.no_rows).into_any_element(),
        };
        form_section(theme, m.preview)
            .child(
                div()
                    .h(px(158.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme.line)
                    .overflow_hidden()
                    .font_family(MONO)
                    .text_size(px(12.))
                    .when(self.reading, |preview| preview.opacity(0.6))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_mapping(&self, theme: &Theme, m: &ImportMessages) -> Option<AnyElement> {
        if !matches!(self.preview, Some(Ok(_))) {
            return None;
        }
        let section = form_section(theme, m.mapping);
        let Some(_) = self.columns() else {
            return Some(section.child(note(theme, m.loading_columns)).into_any_element());
        };
        let mapping = self.mapping();
        let repeated = repeated_columns(&mapping);
        let rows = self.fields().iter().zip(&self.pickers).map(|(field, picker)| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div().w(px(200.)).flex_none().truncate().font_family(MONO).text_size(px(12.5)).child(field.clone()),
                )
                .child(icon("right", theme.text3, 13.))
                .child(div().flex_1().min_w_0().child(picker.clone()))
        });
        let error = if !repeated.is_empty() {
            Some((m.repeated)(&repeated.join(", ")))
        } else if !mapping.iter().any(Option::is_some) {
            Some(m.nothing_mapped.to_owned())
        } else {
            None
        };
        Some(
            section
                .child(div().flex().flex_col().gap(px(6.)).children(rows))
                .children(error.map(|error| div().text_size(px(12.)).text_color(theme.err).child(error)))
                .into_any_element(),
        )
    }

    fn render_status(&self, m: &ImportMessages, cx: &App) -> AnyElement {
        let count = |rows: u64| thousands(rows);
        match &self.status {
            Status::Idle => callout_space().into_any_element(),
            Status::Confirming => {
                let tag = confirming_tag_of(cx, &self.target.connection.id).unwrap_or_default();
                Callout::new("import-status", Tone::Warning, (m.confirm)(&tag))
                    .detail((m.confirm_detail)(&self.relation()))
                    .into_any_element()
            }
            Status::Working { rows, started } => Callout::new("import-status", Tone::Progress, m.importing)
                .aside(elapsed(*started))
                .detail((m.rows_imported)(&count(*rows)))
                .into_any_element(),
            Status::Done(rows) => {
                Callout::new("import-status", Tone::Done, (m.imported)(&count(*rows))).into_any_element()
            }
            Status::Stopped(rows) => Callout::new("import-status", Tone::Stopped, m.stopped)
                .detail(if self.transactional || *rows == 0 {
                    m.stopped_nothing.to_owned()
                } else {
                    (m.stopped_some)(&count(*rows))
                })
                .into_any_element(),
            Status::Failed(error) => Callout::new("import-status", Tone::Failed, m.failed)
                .detail(if self.transactional { format!("{error} {}", m.failed_nothing) } else { error.clone() })
                .mono(true)
                .into_any_element(),
        }
    }

    fn render_footer(&self, theme: &Theme, m: &ImportMessages, cx: &mut Context<Self>) -> AnyElement {
        let footer = modal_footer(theme);
        if matches!(self.status, Status::Done(_)) {
            return footer
                .child(
                    primary_button(theme, m.done)
                        .id("import-done")
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                )
                .into_any_element();
        }
        let (working, ready) = (self.working(), self.ready(cx));
        let leading = if working {
            button(theme, Some("stop"), m.stop)
                .id("import-stop")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            button(theme, None, m.cancel)
                .id("import-cancel")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
        };
        let tag = confirming_tag_of(cx, &self.target.connection.id);
        let label = match (&self.status, tag) {
            (Status::Working { .. }, _) => m.importing.to_owned(),
            (Status::Confirming, Some(tag)) => (m.import_into)(&tag),
            _ => m.import.to_owned(),
        };
        let primary = enabled(primary_button(theme, &label), ready)
            .id("import-run")
            .when(ready, |run| run.on_click(cx.listener(|this, _, window, cx| this.start(window, cx))));
        footer.child(leading).child(primary).into_any_element()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_pickers(cx);
        let theme = *cx.theme();
        let m = &cx.messages().import;
        let source = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .child(icon("table", theme.text3, 15.))
            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(self.relation()));
        let body = modal_body("import-body")
            .child(source)
            .child(self.render_file(&theme, m, cx))
            .child(self.render_formats(&theme, m, cx))
            .child(self.render_options(&theme, m, window, cx))
            .child(self.render_preview(&theme, m))
            .children(self.render_mapping(&theme, m))
            .child(note(&theme, if self.transactional { m.transaction } else { m.batches }));
        modal(&theme, 680., m.title)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &Submit, window, cx| this.submit(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close(cx)))
            .child(body)
            .child(modal_status(self.render_status(m, cx)))
            .child(self.render_footer(&theme, m, cx))
    }
}

#[cfg(test)]
mod tests {
    use dbforge_core::connections::model::{Connection, ConnectionInput};
    use dbforge_core::drivers::TableRef;
    use dbforge_core::query::{QueryScope, run_query};
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::test_support::{draw, setup, wait_until};

    /// A SQLite file in `dir` with a table `people (id, name)`, saved as a
    /// connection tagged `environment` (the default tags: `production` asks first).
    fn sqlite(dir: &std::path::Path, environment: &str, cx: &mut VisualTestContext) -> Connection {
        let file = dir.join("shop.db");
        // An empty file is an empty SQLite database; the driver opens, never creates.
        std::fs::File::create(&file).expect("the database file");
        let connection = cx.update(|_, cx| {
            let input: ConnectionInput = serde_json::from_value(serde_json::json!({
                "name": "Shop", "engine": "sqlite", "environment": environment, "host": "", "port": 0,
                "database": file.display().to_string(), "username": "",
            }))
            .expect("an input");
            Backend::global(cx).core().connections().save(input.into_parts().expect("valid")).expect("saved")
        });
        let core = cx.update(|_, cx| Backend::global(cx).core().clone());
        let sql = "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT)".to_owned();
        let (store, sessions, history, log) =
            (core.connections(), core.query_sessions(), core.history(), core.statement_log());
        let outcome = crate::backend::runtime()
            .block_on(run_query(
                store,
                sessions,
                history,
                log,
                "test".into(),
                connection.id.clone(),
                QueryScope::default(),
                sql,
            ))
            .expect("the table");
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        connection
    }

    fn open_tagged<'a>(
        dir: &tempfile::TempDir,
        environment: &str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<ImportDialog>, &'a mut VisualTestContext) {
        let (_, scratch) = cx.add_window_view(|_, _| gpui::Empty);
        let connection = sqlite(dir.path(), environment, scratch);
        let table = TableRef { database: "main".into(), schema: "main".into(), name: "people".into() };
        cx.add_window_view(move |window, cx| {
            ImportDialog::new(ImportTarget { connection, table, rows: None }, window, cx)
        })
    }

    fn load(dialog: &Entity<ImportDialog>, file: &std::path::Path, cx: &mut VisualTestContext) {
        let path = file.display().to_string();
        dialog.update(cx, |dialog, cx| {
            dialog.path = Some(path);
            dialog.read_file(cx);
        });
        wait_until(cx, "the preview and the columns", |cx| {
            dialog.read_with(cx, |dialog, _| dialog.read() && dialog.table_columns.is_some())
        });
        draw(cx);
    }

    #[gpui::test]
    fn asks_twice_on_a_connection_whose_tag_asks_first(cx: &mut TestAppContext) {
        let dir = setup(cx);
        let (dialog, vcx) = open_tagged(&dir, "production", cx);
        let file = dir.path().join("people.csv");
        std::fs::write(&file, "id,name\n1,Ann\n2,Bob\n").unwrap();
        load(&dialog, &file, vcx);
        dialog.update_in(vcx, |dialog, window, cx| dialog.start(window, cx));
        dialog.read_with(vcx, |dialog, _| assert_eq!(dialog.status, Status::Confirming, "the first press only asks"));
        draw(vcx);
        dialog.update_in(vcx, |dialog, window, cx| dialog.start(window, cx));
        wait_until(vcx, "the import", |cx| dialog.read_with(cx, |dialog, _| !dialog.working()));
        dialog.read_with(vcx, |dialog, _| assert_eq!(dialog.status, Status::Done(2)));
    }

    #[gpui::test]
    fn a_change_after_the_warning_asks_again(cx: &mut TestAppContext) {
        let dir = setup(cx);
        let (dialog, vcx) = open_tagged(&dir, "production", cx);
        let file = dir.path().join("people.csv");
        std::fs::write(&file, "id,name\n1,Ann\n").unwrap();
        load(&dialog, &file, vcx);
        dialog.update_in(vcx, |dialog, window, cx| {
            dialog.start(window, cx);
            assert_eq!(dialog.status, Status::Confirming);
            dialog.set_format(ImportFormat::Tsv, cx);
            assert_eq!(dialog.status, Status::Idle);
        });
    }

    #[gpui::test]
    fn imports_at_once_where_the_tag_does_not_ask(cx: &mut TestAppContext) {
        let dir = setup(cx);
        let (dialog, vcx) = open_tagged(&dir, "development", cx);
        let file = dir.path().join("people.csv");
        std::fs::write(&file, "ID,Name\n1,Ann\n").unwrap();
        load(&dialog, &file, vcx);
        dialog.read_with(vcx, |dialog, _| {
            assert_eq!(dialog.mapping(), vec![Some("id".to_owned()), Some("name".to_owned())], "mapped ignoring case");
        });
        dialog.update_in(vcx, |dialog, window, cx| dialog.start(window, cx));
        wait_until(vcx, "the import", |cx| dialog.read_with(cx, |dialog, _| !dialog.working()));
        dialog.read_with(vcx, |dialog, _| assert_eq!(dialog.status, Status::Done(1)));
    }
}
