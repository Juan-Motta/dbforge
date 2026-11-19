//! Drawing Settings › Backup, as the former web UI's `BackupSettings.tsx`: Export
//! writes the configuration where the OS's save dialog says (passwords and
//! the MCP token only when ticked: the core writes them in plain text, in a
//! file only its owner can read); Import reads a file, says what it holds
//! and what it would add or update here, and applies the parts ticked. The
//! core reads and writes the file; this section never sees a secret: the
//! preview only counts passwords and says whether a token is there.

use chrono::{Local, TimeZone};
use dbforge_core::backup::commands::{export_backup, import_backup, read_backup};
use dbforge_core::backup::{Parts, Preview};
use gpui::{
    AnyElement, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use super::{backup_file_name, current_preferences, known_preferences};
use crate::backend::Backend;
use crate::dump::ExportFolder;
use crate::i18n::{BackupMessages, Locale, Translate};
use crate::preferences::Preferences;
use crate::theme::{ActiveTheme, ThemePreference};
use crate::ui::callout::{Callout, Tone, callout_space};
use crate::ui::file_prompt::{documents_dir, pick_file, pick_new_path};
use crate::ui::modal::{form_section, note};
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::{button, enabled, ghost_button, icon, primary_button};

/// What the section tells the settings, which tell the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupEvent {
    /// Connections and tags were imported: the explorer and the tags read them again.
    ConnectionsChanged,
}

/// How the last export or import went, as data: it reads in the language
/// shown now, which an import may change.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Exported {
        connections: usize,
        passwords: usize,
        path: String,
    },
    /// What importing did; the MCP settings the core applied (with any token) are never kept here.
    Imported {
        added: usize,
        updated: usize,
        without_password: Vec<String>,
        missing_tools: Vec<String>,
    },
    Failed(String),
}

/// The parts of a backup to apply, as ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ticked {
    preferences: bool,
    connections: bool,
    mcp: bool,
    tool_paths: bool,
}

pub struct BackupSection {
    /// Passwords and the MCP token go in the file: on by default, as in the former web UI.
    secrets: bool,
    busy: bool,
    outcome: Option<Outcome>,
    /// A file picked to import, and what it holds.
    picked: Option<(String, Preview)>,
    parts: Ticked,
}

impl EventEmitter<BackupEvent> for BackupSection {}

impl BackupSection {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            secrets: true,
            busy: false,
            outcome: None,
            picked: None,
            parts: Ticked { preferences: true, connections: true, mcp: true, tool_paths: true },
        }
    }

    fn begin(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.outcome = None;
        cx.notify();
    }

    fn end(&mut self, outcome: Option<Outcome>, cx: &mut Context<Self>) {
        self.busy = false;
        self.outcome = outcome;
        cx.notify();
    }

    /// Asks where, then writes the configuration there.
    fn export(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.begin(cx);
        let preferences = current_preferences(
            ThemePreference::saved(Preferences::global(cx)),
            cx.global::<Locale>().0,
            ExportFolder::chosen(cx).as_deref(),
        );
        let secrets = self.secrets;
        let picked = pick_new_path(&documents_dir(), &backup_file_name(Local::now().date_naive()), cx);
        cx.spawn(async move |this, cx| {
            let Some(path) = picked.await else {
                let _ = this.update(cx, |this, cx| this.end(None, cx));
                return;
            };
            let shown = path.display().to_string();
            let Ok(writing) = this.update(cx, |_, cx| {
                Backend::global(cx).spawn_blocking(cx, move |core| export_backup(core, path, preferences, secrets))
            }) else {
                return;
            };
            let written = writing.await;
            let _ = this.update(cx, |this, cx| {
                this.end(
                    Some(match written {
                        Ok(done) => {
                            Outcome::Exported { connections: done.connections, passwords: done.passwords, path: shown }
                        }
                        Err(error) => Outcome::Failed(error.to_string()),
                    }),
                    cx,
                )
            });
        })
        .detach();
    }

    /// Asks for a backup, then shows what it holds.
    fn pick(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.begin(cx);
        let picked = pick_file(cx);
        cx.spawn(async move |this, cx| {
            let Some(path) = picked.await else {
                let _ = this.update(cx, |this, cx| this.end(None, cx));
                return;
            };
            let shown = path.display().to_string();
            let Ok(reading) =
                this.update(cx, |_, cx| Backend::global(cx).spawn_blocking(cx, move |core| read_backup(core, path)))
            else {
                return;
            };
            let read = reading.await;
            let _ = this.update(cx, |this, cx| match read {
                Ok(preview) => {
                    this.picked = Some((shown, preview));
                    this.end(None, cx);
                }
                Err(error) => this.end(Some(Outcome::Failed(error.to_string())), cx),
            });
        })
        .detach();
    }

    /// Applies the parts ticked: the core merges connections, tags, the MCP
    /// settings and tool paths; the window applies the preferences.
    fn import(&mut self, cx: &mut Context<Self>) {
        let Some((path, preview)) = self.picked.clone() else { return };
        if self.busy {
            return;
        }
        self.begin(cx);
        let ticked = self.parts;
        let parts = Parts { connections: ticked.connections, mcp: ticked.mcp, tool_paths: ticked.tool_paths };
        let importing =
            Backend::global(cx).spawn(cx, move |core| async move { import_backup(&core, path.into(), parts).await });
        cx.spawn(async move |this, cx| {
            let imported = importing.await;
            let _ = this.update(cx, |this, cx| match imported {
                Ok(done) => {
                    if ticked.preferences {
                        let known = known_preferences(&preview.preferences);
                        if let Some(theme) = known.theme {
                            theme.save(cx);
                        }
                        if let Some(language) = known.language {
                            language.save(cx);
                        }
                        if let Some(folder) = known.export_folder {
                            ExportFolder::choose(Some(folder), cx);
                        }
                    }
                    if ticked.connections {
                        cx.emit(BackupEvent::ConnectionsChanged);
                    }
                    this.picked = None;
                    let outcome = Outcome::Imported {
                        added: done.added,
                        updated: done.updated,
                        without_password: done.without_password,
                        missing_tools: done.missing_tools,
                    };
                    this.end(Some(outcome), cx);
                }
                Err(error) => this.end(Some(Outcome::Failed(error.to_string())), cx),
            });
        })
        .detach();
    }

    fn render_outcome(&self, m: &BackupMessages) -> AnyElement {
        match &self.outcome {
            None => callout_space().into_any_element(),
            Some(Outcome::Exported { connections, passwords, path }) => {
                Callout::new("backup-outcome", Tone::Done, (m.exported)(*connections, *passwords))
                    .detail(path.clone())
                    .into_any_element()
            }
            Some(Outcome::Failed(error)) => {
                Callout::new("backup-outcome", Tone::Failed, m.failed).detail(error.clone()).into_any_element()
            }
            Some(Outcome::Imported { added, updated, without_password, missing_tools }) => {
                let details: Vec<String> = [
                    (!without_password.is_empty()).then(|| (m.need_password)(&without_password.join(", "))),
                    (!missing_tools.is_empty()).then(|| (m.missing_tools)(&missing_tools.join(", "))),
                ]
                .into_iter()
                .flatten()
                .collect();
                Callout::new("backup-outcome", Tone::Done, (m.imported)(*added, *updated))
                    .detail_opt((!details.is_empty()).then(|| details.join(" ")))
                    .into_any_element()
            }
        }
    }

    fn render_preview(&self, path: &str, preview: &Preview, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let m = &cx.messages().backup;
        let known = known_preferences(&preview.preferences);
        let when = Local
            .timestamp_millis_opt(preview.exported_at as i64)
            .single()
            .map(|at| at.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let parts = self.parts;
        let connections = preview.new_connections.len() + preview.updated_connections.len();
        let tick =
            |id: &'static str, title: String, detail: String, checked: bool, disabled: bool, flip: fn(&mut Ticked)| {
                OptionItem::new(id, Mark::Check, title, checked)
                    .description(detail)
                    .disabled(disabled)
                    .on_select(cx.listener(move |this, _, _, cx| {
                        flip(&mut this.parts);
                        cx.notify();
                    }))
                    .into_any_element()
            };
        let rows = vec![
            tick(
                "backup-preferences",
                m.part_preferences.into(),
                (m.part_preferences_detail)(
                    known.theme.map_or("—", ThemePreference::as_str),
                    known.language.map_or("—", |language| language.as_str()),
                    known.export_folder.as_deref(),
                ),
                parts.preferences,
                false,
                |parts| parts.preferences = !parts.preferences,
            )
            .into_any_element(),
            tick(
                "backup-connections",
                (m.part_connections)(preview.new_connections.len(), preview.updated_connections.len()),
                (m.part_connections_detail)(preview.passwords, preview.tags),
                parts.connections,
                connections == 0,
                |parts| parts.connections = !parts.connections,
            ),
            tick(
                "backup-mcp",
                m.part_mcp.into(),
                match &preview.mcp {
                    Some(mcp) => (m.part_mcp_detail)(mcp.enabled, mcp.port, preview.mcp_token),
                    None => m.not_in_file.into(),
                },
                parts.mcp && preview.mcp.is_some(),
                preview.mcp.is_none(),
                |parts| parts.mcp = !parts.mcp,
            ),
            tick(
                "backup-tools",
                m.part_tools.into(),
                if preview.tool_paths > 0 { (m.part_tools_detail)(preview.tool_paths) } else { m.not_in_file.into() },
                parts.tool_paths && preview.tool_paths > 0,
                preview.tool_paths == 0,
                |parts| parts.tool_paths = !parts.tool_paths,
            ),
        ];
        let busy = self.busy;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(theme.text2)
                    .child(div().mt(px(1.)).child(icon("folder", theme.text3, 14.)))
                    .child(div().flex_1().min_w_0().child((m.from_file)(path, &when, &preview.app_version))),
            )
            .child(option_list(&theme, rows))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(enabled(ghost_button(&theme, m.cancel), !busy).id("backup-cancel").when(!busy, |cancel| {
                        cancel.on_click(cx.listener(|this, _, _, cx| {
                            this.picked = None;
                            cx.notify();
                        }))
                    }))
                    .child(
                        enabled(primary_button(&theme, m.import_button), !busy)
                            .id("backup-import")
                            .when(!busy, |import| import.on_click(cx.listener(|this, _, _, cx| this.import(cx)))),
                    ),
            )
            .into_any_element()
    }
}

impl Render for BackupSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().backup;
        let busy = self.busy;
        let secrets = self.secrets;
        let export = form_section(&theme, m.export_title)
            .child(option_list(
                &theme,
                vec![
                    OptionItem::new("backup-secrets", Mark::Check, m.include_secrets, secrets)
                        .icon("key")
                        .description(m.include_secrets_description)
                        .disabled(busy)
                        .on_select(cx.listener(move |this, _, _, cx| {
                            this.secrets = !secrets;
                            cx.notify();
                        }))
                        .into_any_element(),
                ],
            ))
            .child(
                div().flex().child(
                    enabled(button(&theme, Some("export"), m.export_button), !busy)
                        .id("backup-export")
                        .when(!busy, |export| export.on_click(cx.listener(|this, _, _, cx| this.export(cx)))),
                ),
            );
        let picked = self.picked.clone();
        let import = form_section(&theme, m.import_title).child(note(&theme, m.import_help)).child(match &picked {
            Some((path, preview)) => self.render_preview(path, preview, cx),
            None => div()
                .flex()
                .child(
                    enabled(button(&theme, Some("upload"), m.choose_file), !busy)
                        .id("backup-choose")
                        .when(!busy, |choose| choose.on_click(cx.listener(|this, _, _, cx| this.pick(cx)))),
                )
                .into_any_element(),
        });
        div().py(px(14.)).flex().flex_col().gap(px(22.)).child(export).child(import).child(self.render_outcome(m))
    }
}
