//! The menu bar. On macOS it is the system's usual one, as the former Tauri
//! app's default menu was, in the language in use: the app's menu (About
//! DBForge, Check for Updates…, Services, Hide DBForge, Hide Others, Show
//! All, Quit), Edit (Undo, Redo, Cut, Copy, Paste, Select All, carried out
//! by whatever has the keyboard, see [`crate::ui::edit`]), View (full
//! screen) and Window (Minimize, Zoom). Elsewhere GPUI draws no menu bar,
//! so the app's menu keeps only Check for Updates… and Quit.

use gpui::{App, InteractiveElement, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions};

use crate::app::Quit;
use crate::i18n::Messages;
use crate::ui::edit;
use crate::updates::CheckForUpdates;

actions!(menu_bar, [About, Hide, HideOthers, ShowAll, Minimize, Zoom, ToggleFullScreen]);

/// The menu bar's shortcuts, as macOS's own: shown beside the items, and
/// answered where no control has its own (a text field's ⌘C is its own).
pub fn key_bindings() -> Vec<KeyBinding> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    vec![
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-z", edit::Undo, None),
        KeyBinding::new("cmd-shift-z", edit::Redo, None),
        KeyBinding::new("cmd-x", edit::Cut, None),
        KeyBinding::new("cmd-c", edit::Copy, None),
        KeyBinding::new("cmd-v", edit::Paste, None),
        KeyBinding::new("cmd-a", edit::SelectAll, None),
    ]
}

/// The menus, in `m`'s language; the full menu bar on macOS.
pub fn menus(m: &Messages) -> Vec<Menu> {
    if cfg!(target_os = "macos") { mac_menus(m) } else { app_menu_only(m) }
}

fn app_menu_only(m: &Messages) -> Vec<Menu> {
    vec![Menu {
        name: "DBForge".into(),
        items: vec![
            MenuItem::action(m.settings_dialog.check_now, CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action(m.shell.menu.quit, Quit),
        ],
    }]
}

fn mac_menus(m: &Messages) -> Vec<Menu> {
    let menu = &m.shell.menu;
    vec![
        Menu {
            name: "DBForge".into(),
            items: vec![
                MenuItem::action(menu.about, About),
                MenuItem::action(m.settings_dialog.check_now, CheckForUpdates),
                MenuItem::separator(),
                MenuItem::os_submenu(menu.services, SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(menu.hide, Hide),
                MenuItem::action(menu.hide_others, HideOthers),
                MenuItem::action(menu.show_all, ShowAll),
                MenuItem::separator(),
                MenuItem::action(menu.quit, Quit),
            ],
        },
        Menu {
            name: menu.edit.into(),
            items: vec![
                MenuItem::os_action(menu.undo, edit::Undo, OsAction::Undo),
                MenuItem::os_action(menu.redo, edit::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action(menu.cut, edit::Cut, OsAction::Cut),
                MenuItem::os_action(menu.copy, edit::Copy, OsAction::Copy),
                MenuItem::os_action(menu.paste, edit::Paste, OsAction::Paste),
                MenuItem::os_action(menu.select_all, edit::SelectAll, OsAction::SelectAll),
            ],
        },
        Menu { name: menu.view.into(), items: vec![MenuItem::action(menu.full_screen, ToggleFullScreen)] },
        // Named in the language in use: GPUI would make only a menu named "Window"
        // the one macOS lists the windows in, which a single window doesn't need.
        Menu {
            name: menu.window.into(),
            items: vec![MenuItem::action(menu.minimize, Minimize), MenuItem::action(menu.zoom, Zoom)],
        },
    ]
}

/// The app's own commands, answered with or without a window.
pub fn register(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
}

/// The window's commands, answered on `element`, the window's root; the
/// Edit commands go to whatever has the keyboard.
pub fn handle_window_commands<E: InteractiveElement>(element: E) -> E {
    edit::handle(element)
        .on_action(|_: &Minimize, window: &mut Window, _| window.minimize_window())
        .on_action(|_: &Zoom, window: &mut Window, _| window.zoom_window())
        .on_action(|_: &ToggleFullScreen, window: &mut Window, _| window.toggle_fullscreen())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    /// A menu as its labels, a separator as "-", a system menu as "[name]".
    fn outline(menus: &[Menu]) -> Vec<(String, Vec<String>)> {
        menus
            .iter()
            .map(|menu| {
                let items = menu
                    .items
                    .iter()
                    .map(|item| match item {
                        MenuItem::Separator => "-".to_owned(),
                        MenuItem::Action { name, .. } => name.to_string(),
                        MenuItem::Submenu(menu) => format!("{} ›", menu.name),
                        MenuItem::SystemMenu(menu) => format!("[{}]", menu.name),
                    })
                    .collect();
                (menu.name.to_string(), items)
            })
            .collect()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn the_mac_menu_bar_is_the_system_s_usual_one_with_check_for_updates() {
        let menus = outline(&mac_menus(Language::En.messages()));
        assert_eq!(
            menus,
            vec![
                (
                    "DBForge".to_owned(),
                    strings(&[
                        "About DBForge",
                        "Check for Updates…",
                        "-",
                        "[Services]",
                        "-",
                        "Hide DBForge",
                        "Hide Others",
                        "Show All",
                        "-",
                        "Quit DBForge"
                    ])
                ),
                ("Edit".to_owned(), strings(&["Undo", "Redo", "-", "Cut", "Copy", "Paste", "Select All"])),
                ("View".to_owned(), strings(&["Toggle Full Screen"])),
                ("Window".to_owned(), strings(&["Minimize", "Zoom"])),
            ]
        );
    }

    #[test]
    fn the_menu_bar_speaks_the_language_in_use_and_the_edit_items_are_the_system_s() {
        let menus = outline(&mac_menus(Language::Es.messages()));
        assert_eq!(menus[0].1[0], "Acerca de DBForge");
        assert_eq!(menus[0].1.last().map(String::as_str), Some("Salir de DBForge"));
        assert_eq!(menus[1].0, "Edición");
        assert_eq!(menus[1].1[3..], strings(&["Cortar", "Copiar", "Pegar", "Seleccionar todo"]));
        assert_eq!(menus[3], ("Ventana".to_owned(), strings(&["Minimizar", "Zoom"])));
        let edit = mac_menus(Language::En.messages()).swap_remove(1);
        let os_actions: Vec<Option<OsAction>> = edit
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { os_action, .. } => Some(*os_action),
                _ => None,
            })
            .collect();
        assert!(
            os_actions
                == [
                    Some(OsAction::Undo),
                    Some(OsAction::Redo),
                    Some(OsAction::Cut),
                    Some(OsAction::Copy),
                    Some(OsAction::Paste),
                    Some(OsAction::SelectAll)
                ],
            "the system's own Edit items, so its text fields (a save panel's) answer them too"
        );
    }

    #[test]
    fn elsewhere_the_app_menu_keeps_check_for_updates_and_quit() {
        assert_eq!(
            outline(&app_menu_only(Language::En.messages())),
            vec![("DBForge".to_owned(), strings(&["Check for Updates…", "-", "Quit DBForge"]))]
        );
    }
}
