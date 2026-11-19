//! The Passwords section: where connection passwords are kept, the system
//! keychain or the encrypted file, as the former web UI's `Passwords`. Picking the
//! other one asks the core to move every saved secret there
//! ([`commands::set_credential_storage`]), exactly as the former web UI did; the
//! window only shows the core's progress and outcome. No secret ever
//! reaches this view: it reads the storage's name and the file's path.

use dbforge_core::Event;
use dbforge_core::connections::commands;
use dbforge_core::credentials::{Storage, StorageInfo, SwitchProgress};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};

use super::{STORAGES, progress_text, should_switch, storage_label};
use crate::backend::Backend;
use crate::events::EventHub;
use crate::i18n::Translate;
use crate::theme::{ActiveTheme, MONO};
use crate::ui::{icon, spinner};

/// Where passwords are kept now, as the core says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    storage: Storage,
    file_path: String,
}

impl From<StorageInfo> for Kept {
    fn from(info: StorageInfo) -> Self {
        Self { storage: info.storage(), file_path: info.file_path().to_owned() }
    }
}

/// How the last move ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Moved(Storage),
    Failed(String),
}

pub struct PasswordsSection {
    kept: Option<Kept>,
    moving: bool,
    progress: Option<SwitchProgress>,
    outcome: Option<Outcome>,
    /// The core's progress events, heard only while a move runs.
    listening: Option<Subscription>,
}

impl PasswordsSection {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut section = Self { kept: None, moving: false, progress: None, outcome: None, listening: None };
        section.read_storage(cx);
        section
    }

    /// Reads where passwords are kept from the core.
    fn read_storage(&mut self, cx: &mut Context<Self>) {
        let reading =
            Backend::global(cx).spawn_blocking(cx, |core| Kept::from(commands::credential_storage(core.connections())));
        cx.spawn(async move |this, cx| {
            let kept = reading.await;
            let _ = this.update(cx, |this, cx| {
                this.kept = Some(kept);
                cx.notify();
            });
        })
        .detach();
    }

    /// Moves every saved password to `to`, through the core, showing its progress.
    fn choose(&mut self, to: Storage, cx: &mut Context<Self>) {
        if !should_switch(self.kept.as_ref().map(|kept| kept.storage), self.moving, to) {
            return;
        }
        self.moving = true;
        self.progress = None;
        self.outcome = None;
        self.listening = Some(cx.subscribe(&EventHub::global(cx), |this, _, event: &Event, cx| {
            if let Event::CredentialProgress(progress) = event
                && this.moving
            {
                this.progress = Some(*progress);
                cx.notify();
            }
        }));
        let switching = Backend::global(cx).spawn(cx, move |core| async move {
            commands::set_credential_storage(core.connections().clone(), core.events().clone(), to)
                .await
                .map(Kept::from)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let switched = switching.await;
            let _ = this.update(cx, |this, cx| {
                match switched {
                    Ok(kept) => {
                        this.kept = Some(kept);
                        this.outcome = Some(Outcome::Moved(to));
                    }
                    Err(message) => {
                        this.outcome = Some(Outcome::Failed(message));
                        // A partly finished switch may still have changed the choice.
                        this.read_storage(cx);
                    }
                }
                this.listening = None;
                this.moving = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for PasswordsSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        let current = self.kept.as_ref().map(|kept| kept.storage);
        let pickable = current.is_some() && !self.moving;
        let options = STORAGES.map(|storage| {
            let checked = current == Some(storage);
            let help = if storage == Storage::File { m.file_help } else { m.keychain_help };
            div()
                .id(match storage {
                    Storage::Keychain => "storage-keychain",
                    Storage::File => "storage-file",
                })
                .flex()
                .items_start()
                .gap(px(12.))
                .p(px(12.))
                .rounded(px(12.))
                .border_1()
                .border_color(if checked { theme.accent } else { theme.line })
                .when(checked, |option| option.bg(theme.accent.opacity(0.08)))
                .when(!pickable, |option| option.opacity(0.6))
                .when(pickable && !checked, |option| {
                    option
                        .cursor_pointer()
                        .hover(|option| option.bg(theme.sel))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose(storage, cx)))
                })
                // The radio mark.
                .child(
                    div()
                        .mt(px(2.))
                        .size(px(16.))
                        .flex_none()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(if checked { theme.accent } else { theme.text3 })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(checked, |mark| mark.child(div().size(px(8.)).rounded(px(4.)).bg(theme.accent))),
                )
                .child(icon(if storage == Storage::File { "lock" } else { "key" }, theme.text2, 16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(storage_label(storage, m)),
                        )
                        .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(theme.text3).child(help)),
                )
        });
        let callout = if self.moving {
            Some((
                theme.text2,
                Some(spinner("storage-moving", theme.text2, 14.).into_any_element()),
                progress_text(self.progress, m),
                // The keychain can stop to ask for each password; say so, or the switch looks stuck.
                (current == Some(Storage::Keychain)).then_some(m.system_may_ask.to_owned()),
            ))
        } else {
            match &self.outcome {
                Some(Outcome::Failed(error)) => Some((
                    theme.err,
                    Some(icon("warning", theme.err, 14.).into_any_element()),
                    m.move_failed.to_owned(),
                    Some(error.clone()),
                )),
                Some(Outcome::Moved(storage)) => Some((
                    theme.ok,
                    Some(icon("check", theme.ok, 14.).into_any_element()),
                    (m.moved_to)(storage_label(*storage, m)),
                    None,
                )),
                None => None,
            }
        };
        let title = |text: &'static str| {
            div().mb(px(8.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.text2).child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(div().child(title(m.storage)).child(div().flex().flex_col().gap(px(8.)).children(options)))
            .child(
                div().child(title(m.file_location)).child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .text_color(theme.text2)
                        .child(div().mt(px(2.)).child(icon("lock", theme.text3, 14.)))
                        // The whole path, wrapped where it must, so it can be read; a space keeps its line while it loads.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .min_h(px(17.))
                                .font_family(MONO)
                                .text_size(px(12.))
                                .line_height(px(17.))
                                .child(self.kept.as_ref().map(|kept| kept.file_path.clone()).unwrap_or_default()),
                        ),
                ),
            )
            // Room for the callout is kept, so one appearing never moves the section.
            .child(div().min_h(px(64.)).children(callout.map(|(tint, mark, headline, detail)| {
                div()
                    .p(px(12.))
                    .rounded(px(12.))
                    .bg(tint.opacity(0.1))
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .children(mark.map(|mark| div().mt(px(1.)).child(mark)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(tint)
                                    .child(headline),
                            )
                            .children(
                                detail.map(|detail| div().text_size(px(12.5)).text_color(theme.text2).child(detail)),
                            ),
                    )
            })))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup, wait_until};

    #[gpui::test]
    fn shows_where_passwords_are_kept_and_moves_nothing_when_the_same_place_is_picked(cx: &mut TestAppContext) {
        let dir = setup(cx);
        let (section, cx) = cx.add_window_view(|_, cx| PasswordsSection::new(cx));
        wait_until(cx, "the storage to be read", |cx| section.read_with(cx, |section, _| section.kept.is_some()));
        section.read_with(cx, |section, _| {
            let kept = section.kept.as_ref().expect("read");
            // The test core keeps its secrets in the encrypted file, never the keychain.
            assert_eq!(kept.storage, Storage::File);
            assert!(kept.file_path.starts_with(&dir.path().display().to_string()), "{}", kept.file_path);
        });
        draw(cx);
        section.update(cx, |section, cx| section.choose(Storage::File, cx));
        section.read_with(cx, |section, _| {
            assert!(!section.moving && section.outcome.is_none() && section.listening.is_none(), "nothing to move");
        });
    }
}
