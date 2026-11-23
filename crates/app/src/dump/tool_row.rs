//! Which client tool runs (pg_dump, mysql, mongorestore…), as the former web UI's `ToolRow.tsx`: its version
//! and path, a way to choose another when the one found isn't right, and
//! back to the one found. The core finds the tool and checks a chosen one
//! before keeping it (`tools.json`).

use crate::ui::Ellipsis;
use dbforge_core::dump::{ToolInfo, dump_tool, set_dump_tool};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::backend::Backend;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::file_prompt::pick_file;
use crate::ui::{button, enabled, ghost_button, icon};

/// The tool that will run, as the row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    path: String,
    version: String,
    custom: bool,
}

impl From<ToolInfo> for Found {
    fn from(tool: ToolInfo) -> Self {
        Self { path: tool.path().to_owned(), version: tool.version().to_owned(), custom: tool.custom() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Loading,
    Found(Found),
    /// Why none can run.
    Missing(String),
}

pub struct ToolRow {
    name: &'static str,
    state: State,
    /// A path that couldn't be used: said once, next to the tool still in use.
    refused: Option<String>,
    disabled: bool,
}

impl ToolRow {
    /// The row of `name`, one of the tools [`dbforge_core::dump::dump_tools`] names.
    pub fn new(name: &'static str, cx: &mut Context<Self>) -> Self {
        let finding = Backend::global(cx)
            .spawn(cx, move |core| async move { dump_tool(core.tool_paths(), name.to_owned()).await });
        cx.spawn(async move |this, cx| {
            let found = finding.await;
            let _ = this.update(cx, |this, cx| {
                this.state = match found {
                    Ok(tool) => State::Found(tool.into()),
                    Err(error) => State::Missing(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
        Self { name, state: State::Loading, refused: None, disabled: false }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Its buttons take no clicks while a run uses the tool.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }

    /// Runs `path` from now on once it proves to be the tool; `None` goes back to the one found.
    fn change(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let name = self.name;
        let changing = Backend::global(cx)
            .spawn(cx, move |core| async move { set_dump_tool(core.tool_paths(), name.to_owned(), path).await });
        cx.spawn(async move |this, cx| {
            let changed = changing.await;
            let _ = this.update(cx, |this, cx| {
                match changed {
                    Ok(tool) => {
                        this.state = State::Found(tool.into());
                        this.refused = None;
                    }
                    Err(error) => this.refused = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        let picked = pick_file(cx);
        cx.spawn(async move |this, cx| {
            let Some(path) = picked.await else { return };
            let _ = this.update(cx, |this, cx| this.change(Some(path.to_string_lossy().into_owned()), cx));
        })
        .detach();
    }
}

impl Render for ToolRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().dump;
        let text = match &self.state {
            State::Found(tool) => div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("{} {}", self.name, tool.version))
                        .when(tool.custom, |name| {
                            name.child(
                                div()
                                    .px(px(6.))
                                    .rounded(px(6.))
                                    .bg(theme.sel)
                                    .text_size(px(11.))
                                    .text_color(theme.text3)
                                    .child(m.tool_chosen),
                            )
                        }),
                )
                .child(
                    div()
                        .ellipsis()
                        .font_family(MONO)
                        .text_size(px(11.5))
                        .text_color(theme.text3)
                        .child(tool.path.clone()),
                ),
            State::Missing(error) => div().text_size(px(12.5)).text_color(theme.err).child(error.clone()),
            State::Loading => div().text_size(px(12.5)).text_color(theme.text3).child((m.tool_looking)(self.name)),
        };
        let custom = matches!(&self.state, State::Found(tool) if tool.custom);
        let can_choose = !self.disabled && self.state != State::Loading;
        let choose_label =
            if matches!(self.state, State::Missing(_)) { (m.tool_choose)(self.name) } else { m.change.into() };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(10.))
            .py(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme.line)
            .child(icon("code", theme.text3, 14.))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(text).children(
                self.refused.clone().map(|refused| div().text_size(px(12.)).text_color(theme.err).child(refused)),
            ))
            .when(custom, |row| {
                row.child(
                    enabled(ghost_button(&theme, m.tool_use_found), !self.disabled)
                        .id(SharedString::from(format!("tool-use-found-{}", self.name)))
                        .when(!self.disabled, |back| {
                            back.on_click(cx.listener(|this, _, _, cx| this.change(None, cx)))
                        }),
                )
            })
            .child(
                enabled(button(&theme, None, &choose_label), can_choose)
                    .id(SharedString::from(format!("tool-choose-{}", self.name)))
                    .when(can_choose, |choose| choose.on_click(cx.listener(|this, _, _, cx| this.choose(cx)))),
            )
    }
}
