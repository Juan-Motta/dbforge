//! Settings › Export and import, as the former web UI's `ExportSettings.tsx` and
//! `ExportFolderUsage.tsx`: where database exports go (and back to the
//! default), how much the exports there take and deleting them (the first
//! press asks, with the count and size; the second deletes, only files
//! DBForge named), and which pg_dump and psql run.

use dbforge_core::dump::{ExportFiles, clear_export_folder, export_folder_usage};
use gpui::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::{ExportFolder, ToolRow};
use crate::backend::Backend;
use crate::i18n::{DumpMessages, Translate};
use crate::preferences::Preferences;
use crate::structure::format::format_bytes;
use crate::theme::{ActiveTheme, MONO, Theme};
use crate::ui::file_prompt::pick_folder;
use crate::ui::modal::{form_section, note};
use crate::ui::{button, enabled, ghost_button, icon, primary_button};

/// How much the exports in the folder take, and deleting them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Usage {
    Measuring,
    Shown { files: ExportFiles, note: Option<String> },
    Confirming { files: ExportFiles },
    Failed(String),
}

pub struct ExportSettings {
    tools: [Entity<ToolRow>; 2],
    usage: Usage,
    /// The folder the usage is of; a new one is measured afresh.
    measured: Option<String>,
    request: u64,
    _subscriptions: Vec<Subscription>,
}

impl ExportSettings {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tools = [cx.new(|cx| ToolRow::new("pg_dump", cx)), cx.new(|cx| ToolRow::new("psql", cx))];
        let subscriptions = vec![cx.observe_global::<Preferences>(|this, cx| this.measure(false, cx))];
        let mut settings =
            Self { tools, usage: Usage::Measuring, measured: None, request: 0, _subscriptions: subscriptions };
        settings.measure(false, cx);
        settings
    }

    /// Measures the folder when it changed, or `again`.
    fn measure(&mut self, again: bool, cx: &mut Context<Self>) {
        let folder = ExportFolder::folder(cx);
        if !again && folder == self.measured {
            return;
        }
        self.measured = folder.clone();
        self.request += 1;
        let request = self.request;
        self.usage = Usage::Measuring;
        let Some(folder) = folder else { return };
        let measuring = Backend::global(cx).spawn_blocking(cx, move |_| export_folder_usage(folder));
        cx.spawn(async move |this, cx| {
            let measured = measuring.await;
            let _ = this.update(cx, |this, cx| {
                if this.request == request {
                    this.usage = match measured {
                        Ok(files) => Usage::Shown { files, note: None },
                        Err(error) => Usage::Failed(error.to_string()),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Deletes the exports in the folder, leaving its other files, and says what went.
    fn clear(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.measured.clone() else { return };
        self.request += 1;
        let request = self.request;
        let clearing = Backend::global(cx).spawn_blocking(cx, move |_| {
            let removed = clear_export_folder(folder.clone())?;
            Ok::<_, dbforge_core::AppError>((removed, export_folder_usage(folder)?))
        });
        cx.spawn(async move |this, cx| {
            let cleared = clearing.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                let m = &cx.messages().dump;
                this.usage = match cleared {
                    Ok((removed, files)) => Usage::Shown {
                        files,
                        note: Some((m.cleared)(removed.count, &format_bytes(removed.bytes as i64))),
                    },
                    Err(error) => Usage::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        let picked = pick_folder(cx);
        cx.spawn(async move |_, cx| {
            let Some(folder) = picked.await else { return };
            let _ = cx.update(|cx| ExportFolder::choose(Some(folder.to_string_lossy().into_owned()), cx));
        })
        .detach();
    }

    fn render_usage(&self, theme: &Theme, m: &DumpMessages, cx: &mut Context<Self>) -> AnyElement {
        let size = |files: &ExportFiles| format_bytes(files.bytes as i64);
        let confirming = matches!(self.usage, Usage::Confirming { .. });
        let text = match &self.usage {
            Usage::Measuring => m.measuring.to_owned(),
            Usage::Failed(error) => error.clone(),
            Usage::Confirming { files } => (m.confirm_clear)(files.count, &size(files)),
            Usage::Shown { files, note } => note.clone().unwrap_or_else(|| (m.export_files)(files.count, &size(files))),
        };
        let row = div()
            .min_h(px(46.))
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(6.))
            .py(px(6.))
            .rounded(px(10.))
            .border_1()
            .map(|row| {
                if confirming {
                    row.border_color(theme.warn.opacity(0.45)).bg(theme.warn.opacity(0.1))
                } else {
                    row.border_color(theme.line)
                }
            })
            .child(icon(
                if confirming { "warning" } else { "database" },
                if confirming { theme.warn } else { theme.text3 },
                14.,
            ))
            .child(div().flex_1().min_w_0().text_size(px(12.5)).text_color(theme.text2).child(text));
        match &self.usage {
            Usage::Confirming { files } => {
                let files = *files;
                row.child(ghost_button(theme, m.cancel).id("exports-cancel").cursor_pointer().on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.usage = Usage::Shown { files, note: None };
                        cx.notify();
                    },
                )))
                .child(
                    primary_button(theme, m.delete_exports)
                        .id("exports-delete")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                )
                .into_any_element()
            }
            usage => {
                let files = match usage {
                    Usage::Shown { files, .. } if files.count > 0 => Some(*files),
                    _ => None,
                };
                row.child(enabled(button(theme, None, m.clear_folder), files.is_some()).id("exports-clear").when_some(
                    files,
                    |clear, files| {
                        clear.on_click(cx.listener(move |this, _, _, cx| {
                            this.usage = Usage::Confirming { files };
                            cx.notify();
                        }))
                    },
                ))
                .into_any_element()
            }
        }
    }
}

impl Render for ExportSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let folder = ExportFolder::folder(cx);
        let custom = ExportFolder::chosen(cx).is_some();
        let folder_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(icon("folder", theme.text3, 15.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .text_size(px(12.5))
                    .child(folder.clone().unwrap_or_else(|| m.looking_for_folder.into())),
            )
            .when(custom, |row| {
                row.child(
                    ghost_button(&theme, m.use_default_folder)
                        .id("exports-default")
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| ExportFolder::choose(None, cx))),
                )
            })
            .child(
                button(&theme, None, m.change)
                    .id("exports-change")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.pick(cx))),
            );
        let usage = folder.is_some().then(|| self.render_usage(&theme, m, cx));
        div()
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                form_section(&theme, m.default_folder)
                    .child(folder_row)
                    .child(note(&theme, m.default_folder_help))
                    .children(usage)
                    .child(note(&theme, m.export_files_help)),
            )
            .child(
                form_section(&theme, m.client_tools)
                    .child(self.tools[0].clone())
                    .child(self.tools[1].clone())
                    .child(note(&theme, m.client_tools_help)),
            )
    }
}
