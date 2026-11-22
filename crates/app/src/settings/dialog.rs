//! Drawing the settings dialog: a fixed size, so switching sections never
//! resizes it; the section list on the left; the chosen section's heading,
//! intro and body on the right. Escape or the close button closes it.

use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, actions, div, prelude::FluentBuilder, px,
};

use super::{PasswordsSection, Section};
use crate::backup::{BackupEvent, BackupSection};
use crate::dump::ExportSettings;
use crate::i18n::{Language, Locale, Translate};
use crate::mcp::{AccessSection, McpSection, McpSectionEvent};
use crate::preferences::Preferences;
use crate::tags::TagsSection;
use crate::tags::section::TagsEvent;
use crate::theme::{ActiveTheme, Appearance, Theme, ThemePreference};
use crate::ui::field::Tooltip;
use crate::ui::option_list::{Mark, OptionItem, option_list};
use crate::ui::{button, choice, choice_group, glass, icon, icon_button};
use crate::updates::Updates;

actions!(settings_dialog, [Dismiss]);

const CONTEXT: &str = "SettingsDialog";

/// The dialog's size, the same for every section; a smaller window shrinks it to fit.
const WIDTH: f32 = 1000.;
const HEIGHT: f32 = 760.;

/// The themes, in the order the former web UI shows them.
const THEMES: [ThemePreference; 3] = [ThemePreference::Dark, ThemePreference::Light, ThemePreference::System];

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(CONTEXT))]
}

/// What the dialog tells the window, besides closing ([`DismissEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEvent {
    /// The tags were saved: connections show their new names and colours,
    /// and lost a deleted tag when `removed`.
    TagsSaved { removed: bool },
    /// A backup brought connections and tags in, or an agent access was
    /// saved: the explorer reads them again.
    ConnectionsChanged,
}

pub struct SettingsDialog {
    section: Section,
    tags: Entity<TagsSection>,
    passwords: Entity<PasswordsSection>,
    exports: Entity<ExportSettings>,
    backup: Entity<BackupSection>,
    /// Agents (MCP) while it is shown, with what it tells the dialog: the
    /// token is read only when the user goes there.
    agents: Option<(Entity<McpSection>, Subscription)>,
    /// Agent access while it is shown, read again each time.
    access: Option<(Entity<AccessSection>, Subscription)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SettingsDialog {}
impl EventEmitter<SettingsEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsDialog {
    pub fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tags = cx.new(|cx| TagsSection::new(window, cx));
        let passwords = cx.new(PasswordsSection::new);
        let exports = cx.new(ExportSettings::new);
        let backup = cx.new(BackupSection::new);
        let subscriptions = vec![
            cx.subscribe(&tags, |_, _, TagsEvent::Saved { removed }, cx| {
                cx.emit(SettingsEvent::TagsSaved { removed: *removed });
            }),
            cx.subscribe_in(&backup, window, |this, _, BackupEvent::ConnectionsChanged, window, cx| {
                // The tags may have come with them.
                this.tags.update(cx, |tags, cx| tags.reload(window, cx));
                cx.emit(SettingsEvent::ConnectionsChanged);
            }),
        ];
        let section =
            if Section::available().any(|available| available == section) { section } else { Section::Appearance };
        let mut dialog = Self {
            section: Section::Appearance,
            tags,
            passwords,
            exports,
            backup,
            agents: None,
            access: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        dialog.show_section_in(section, window, cx);
        dialog
    }

    /// Takes the keyboard, so Escape reaches it.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    pub fn show_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.show_section_in(section, window, cx);
    }

    fn show_section_in(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if !Section::available().any(|available| available == section) {
            return;
        }
        // As the former web UI mounts it: read again each time it is shown, the token hidden.
        if section != Section::Agents {
            self.agents = None;
        } else if self.agents.is_none() {
            let agents = cx.new(|cx| McpSection::new(window, cx));
            // An agent access saved there shows in the explorer too.
            let changed = cx.subscribe(&agents, |_, _, McpSectionEvent::ConnectionsChanged, cx| {
                cx.emit(SettingsEvent::ConnectionsChanged);
            });
            self.agents = Some((agents, changed));
        }
        if section != Section::AgentAccess {
            self.access = None;
        } else if self.access.is_none() {
            let access = cx.new(AccessSection::new);
            // An access saved there shows in the explorer too.
            let changed = cx.subscribe(&access, |_, _, McpSectionEvent::ConnectionsChanged, cx| {
                cx.emit(SettingsEvent::ConnectionsChanged);
            });
            self.access = Some((access, changed));
        }
        self.section = section;
        cx.notify();
    }

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        div()
            .w(px(180.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(12.))
            .border_r_1()
            .border_color(theme.line)
            .children(Section::available().map(|section| {
                let current = section == self.section;
                div()
                    .id(section.name())
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if current { theme.text } else { theme.text2 })
                    .cursor_pointer()
                    .when(current, |item| item.bg(theme.accent.opacity(0.2)))
                    .when(!current, |item| item.hover(|item| item.bg(theme.sel)))
                    .on_click(cx.listener(move |this, _, window, cx| this.show_section(section, window, cx)))
                    .child(section.label(m))
            }))
    }

    fn render_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        let (heading, intro, body): (&str, Option<&str>, gpui::AnyElement) = match self.section {
            Section::Appearance => {
                (m.appearance, Some(m.appearance_intro), self.render_appearance(cx).into_any_element())
            }
            Section::Language => (m.language, Some(m.language_intro), self.render_language(cx).into_any_element()),
            Section::Tags => {
                let t = &cx.messages().tags;
                (t.tags, Some(t.tags_intro), self.tags.clone().into_any_element())
            }
            Section::Passwords => (m.passwords, Some(m.passwords_intro), self.passwords.clone().into_any_element()),
            Section::Exports => (m.exports, Some(m.exports_intro), self.exports.clone().into_any_element()),
            Section::Backup => (m.backup, Some(m.backup_intro), self.backup.clone().into_any_element()),
            Section::About => (m.about, None, self.render_about(cx).into_any_element()),
            Section::Agents => (
                m.agents,
                Some(m.agents_intro),
                self.agents
                    .as_ref()
                    .map_or_else(|| div().into_any_element(), |(agents, _)| agents.clone().into_any_element()),
            ),
            Section::AgentAccess => (
                m.agent_access,
                Some(m.agent_access_intro),
                self.access
                    .as_ref()
                    .map_or_else(|| div().into_any_element(), |(access, _)| access.clone().into_any_element()),
            ),
        };
        div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .px(px(28.))
            .py(px(22.))
            .overflow_y_scroll()
            .child(div().mb(px(4.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(heading))
            .children(intro.map(|intro| div().mb(px(14.)).text_size(px(13.)).text_color(theme.text2).child(intro)))
            .child(body)
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        let chosen = ThemePreference::saved(Preferences::global(cx));
        div()
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(m.theme))
            .child(div().flex().gap(px(12.)).children(THEMES.map(|preference| {
                let checked = preference == chosen;
                let label = match preference {
                    ThemePreference::Dark => m.dark,
                    ThemePreference::Light => m.light,
                    ThemePreference::System => m.system,
                };
                div()
                    .id(preference.as_str())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(if checked { theme.text } else { theme.text2 })
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| preference.save(cx)))
                    .child(preview(&theme, preference, checked))
                    .child(label)
            })))
    }

    fn render_language(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        let current = cx.global::<Locale>().0;
        div()
            .py(px(14.))
            .flex()
            .items_center()
            .gap(px(24.))
            .border_b_1()
            .border_color(theme.line)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(m.interface_language))
                    .child(div().text_size(px(12.5)).text_color(theme.text3).child(m.interface_language_help)),
            )
            .child(choice_group(&theme).children(Language::ALL.map(|(language, name)| {
                choice(&theme, language.as_str(), name, language == current)
                    .on_click(cx.listener(move |_, _, _, cx| language.save(cx)))
            })))
    }

    fn render_about(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pt(px(14.))
                    .pb(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .border_b_1()
                    .border_color(theme.line)
                    .child(
                        div()
                            .size(px(44.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .bg(theme.accent)
                            .child(icon("database", gpui::white(), 22.)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("DBForge"))
                            .child(div().text_size(px(12.5)).text_color(theme.text3).child(m.tagline)),
                    ),
            )
            .child(
                div()
                    .py(px(14.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.line)
                    .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(m.version))
                    .child(div().text_size(px(13.)).text_color(theme.text2).child(dbforge_core::VERSION)),
            )
            .child(self.render_updates(cx))
    }

    /// Below the version, as the former web UI: the automatic check, then a check now.
    fn render_updates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        let automatic = Updates::automatic(cx);
        div()
            .mt(px(18.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(option_list(
                &theme,
                vec![
                    OptionItem::new("updates-automatic", Mark::Check, m.check_automatically, automatic)
                        .description(m.check_automatically_description)
                        .on_select(cx.listener(move |_, _, _, cx| {
                            Updates::set_automatic(!automatic, cx);
                            cx.notify();
                        }))
                        .into_any_element(),
                ],
            ))
            .child(div().flex().child(
                button(&theme, Some("refresh"), m.check_now).id("updates-check").cursor_pointer().on_click(
                    cx.listener(|_, _, _, cx| {
                        Updates::global(cx).update(cx, |updates, cx| updates.check(true, cx));
                    }),
                ),
            ))
    }
}

/// A theme's thumbnail: a sidebar, an accent line and a text line in its
/// colours; "Match system" shows dark and light side by side.
fn preview(theme: &Theme, preference: ThemePreference, checked: bool) -> impl IntoElement {
    let half = |appearance: Appearance| {
        let colours = Theme::new(appearance);
        let line = |tint: Hsla, width: f32| div().h(px(5.)).w(px(width)).rounded(px(3.)).bg(tint);
        div().flex_1().h_full().flex().bg(colours.wall_top).child(div().w(px(22.)).h_full().bg(colours.glass)).child(
            div()
                .flex_1()
                .p(px(10.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(line(colours.accent, 36.))
                .child(line(colours.text3, 52.)),
        )
    };
    let halves = match preference {
        ThemePreference::Dark => vec![half(Appearance::Dark)],
        ThemePreference::Light => vec![half(Appearance::Light)],
        ThemePreference::System => vec![half(Appearance::Dark), half(Appearance::Light)],
    };
    div()
        .w(px(150.))
        .h(px(92.))
        .flex()
        .rounded(px(12.))
        .overflow_hidden()
        .border_2()
        .border_color(if checked { theme.accent } else { theme.line })
        .children(halves)
}

impl Render for SettingsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let m = &cx.messages().settings_dialog;
        glass(&theme)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(DismissEvent)))
            .bg(theme.card)
            .w(px(WIDTH))
            .h(px(HEIGHT))
            .max_w_full()
            .max_h_full()
            .rounded(px(22.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(52.))
                    .flex_none()
                    .px(px(20.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.line)
                    .child(div().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(m.title))
                    .child(
                        icon_button(&theme, "settings-close", "close", true, false)
                            .tooltip(Tooltip::text(m.close))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    ),
            )
            .child(div().flex_1().min_h_0().flex().child(self.render_nav(cx)).child(self.render_section(cx)))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::{draw, setup};

    #[gpui::test]
    fn draws_every_section_it_offers_and_applies_a_theme_and_a_language_at_once(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let (dialog, cx) = cx.add_window_view(|window, cx| SettingsDialog::new(Section::Agents, window, cx));
        dialog.read_with(cx, |dialog, _| assert_eq!(dialog.section, Section::Agents));
        for section in Section::available() {
            dialog.update_in(cx, |dialog, window, cx| dialog.show_section(section, window, cx));
            cx.run_until_parked();
            draw(cx);
        }
        cx.update(|_, cx| {
            ThemePreference::Light.save(cx);
            Language::Es.save(cx);
            assert_eq!(ThemePreference::saved(Preferences::global(cx)), ThemePreference::Light);
            assert_eq!(Preferences::global(cx).get(crate::preferences::keys::LANGUAGE), Some("es"));
        });
    }
}
