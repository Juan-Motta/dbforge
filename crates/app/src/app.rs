//! Starting the app: the runtime, the core, the preferences, then the window.

use std::path::PathBuf;
use std::sync::Arc;

use dbforge_core::{Core, Dirs};
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    actions, point, px, size,
};

use crate::assets::Assets;
use crate::backend::{Backend, runtime};
use crate::events::{self, EventHub};
use crate::i18n::Locale;
use crate::i18n::Translate;
use crate::mcp::Approvals;
use crate::preferences::{PreferenceFile, Preferences};
use crate::shell::{self, Shell};
use crate::theme::Theme;
use crate::updates::{CheckForUpdates, Updates};

actions!(dbforge, [Quit]);

/// For development: keep the config and data in `$DBFORGE_DIR/config` and
/// `$DBFORGE_DIR/data` instead of the installed app's directories.
const DIR_VARIABLE: &str = "DBFORGE_DIR";

/// The smallest window the explorer, toolbar and main area fit in.
const MIN_SIZE: (f32, f32) = (900., 560.);

/// Opens DBForge's window and runs until the user quits; the exit code.
pub fn run() -> i32 {
    let Some(dirs) = dirs() else {
        eprintln!("DBForge: this system has no config directory");
        return 1;
    };
    let (events, received) = events::channel();
    // The core starts its tunnels and MCP server on the runtime it loads in.
    let core = {
        let _runtime = runtime().enter();
        Core::load(dirs.clone(), Arc::new(events))
    };
    let core = match core {
        Ok(core) => core,
        Err(error) => {
            eprintln!("DBForge could not start: {error}");
            return 1;
        }
    };
    let preferences = match PreferenceFile::load(dirs.preferences_file()) {
        Ok(file) => Preferences::new(file),
        Err(error) => {
            eprintln!("DBForge could not read its preferences: {error}");
            return 1;
        }
    };

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        cx.set_global(Backend::new(core));
        cx.set_global(preferences);
        EventHub::start(received, cx);
        // Before the hub forwards anything, so an agent's request sent while the window opens waits for it.
        Approvals::start(cx);
        Updates::start(cx);
        // Theme and language before the first frame, so it never flickers.
        Theme::sync(cx.window_appearance().into(), cx);
        Locale::sync(cx);
        bind_keys(cx);
        open_window(cx);
        cx.activate(true);
    });
    0
}

/// The installed app's directories, or the development ones (`DBFORGE_DIR`).
fn dirs() -> Option<Dirs> {
    match std::env::var_os(DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            Some(Dirs { config: dir.join("config"), data: dir.join("data") })
        }
        None => Dirs::default_for_app(),
    }
}

fn bind_keys(cx: &mut App) {
    let quit = if cfg!(target_os = "macos") { "cmd-q" } else { "ctrl-q" };
    cx.bind_keys([KeyBinding::new(quit, Quit, None)]);
    cx.bind_keys(shell::key_bindings());
    cx.bind_keys(shell::set_aside::key_bindings());
    cx.bind_keys(crate::palette::key_bindings());
    cx.bind_keys(crate::settings::key_bindings());
    cx.bind_keys(shell::explorer::key_bindings());
    cx.bind_keys(crate::ui::text_input::key_bindings());
    cx.bind_keys(crate::ui::code_editor::key_bindings());
    cx.bind_keys(crate::grid::key_bindings());
    cx.bind_keys(crate::query::key_bindings());
    cx.bind_keys(crate::table::key_bindings());
    cx.bind_keys(crate::ui::menu::key_bindings());
    cx.bind_keys(crate::ui::select::key_bindings());
    cx.bind_keys(crate::connections::key_bindings());
    cx.bind_keys(crate::keyspace::key_bindings());
    cx.bind_keys(crate::export::key_bindings());
    cx.bind_keys(crate::import::key_bindings());
    cx.bind_keys(crate::dump::key_bindings());
    cx.bind_keys(crate::mcp::key_bindings());
    cx.bind_keys(crate::updates::dialog_key_bindings());
    cx.on_action(|_: &Quit, cx| cx.quit());
    set_menus(cx);
    // Closing the window quits, as the Tauri app did.
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    // A change still waiting to be written is saved before the process ends.
    cx.on_app_quit(|cx| {
        Preferences::global(cx).flush();
        async {}
    })
    .detach();
}

/// The app menu (on macOS the menu bar's DBForge menu): Check for Updates…
/// and Quit, in the language in use. Set again when the language changes.
pub fn set_menus(cx: &mut App) {
    let check = cx.messages().settings_dialog.check_now;
    cx.set_menus(vec![Menu {
        name: "DBForge".into(),
        items: vec![
            MenuItem::action(check, CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Quit DBForge", Quit),
        ],
    }]);
}

fn open_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(880.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(titlebar()),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(dbforge_core::IDENTIFIER.into()),
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| cx.new(|cx| Shell::new(window, cx)));
    if let Err(error) = opened {
        eprintln!("DBForge could not open its window: {error}");
        cx.quit();
    }
}

/// On macOS the title bar is transparent and the traffic lights sit in the
/// explorer's top corner; elsewhere the system draws its own title bar.
fn titlebar() -> TitlebarOptions {
    let title = Some("DBForge".into());
    if cfg!(target_os = "macos") {
        TitlebarOptions { title, appears_transparent: true, traffic_light_position: Some(point(px(26.), px(26.))) }
    } else {
        TitlebarOptions { title, appears_transparent: false, traffic_light_position: None }
    }
}
