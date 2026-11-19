//! The texts of the window's own pieces: the toolbar's connection details,
//! the explorer's Copy menu, the notices at the bottom of the window, the
//! line showing a failure in full, and the macOS menu bar.

/// The shell's texts in one language.
pub struct ShellMessages {
    // Connection details, from the toolbar's connection
    pub reachable: &'static str,
    pub not_reachable: &'static str,
    pub checking: &'static str,
    pub server: &'static str,
    pub file: &'static str,
    pub address: &'static str,
    pub database: &'static str,
    pub ssl: &'static str,
    pub ssl_disable: &'static str,
    pub ssl_prefer: &'static str,
    pub ssl_require: &'static str,
    pub ssl_verify_ca: &'static str,
    pub ssl_verify_full: &'static str,
    pub ssh: &'static str,
    /// The user in an address that names none: the server's default.
    pub default_user: &'static str,
    // A table's or view's Copy menu
    pub copy_menu: &'static str,
    pub copy_name: &'static str,
    pub copy_find: &'static str,
    pub copy_count: &'static str,
    pub copy_select: &'static str,
    pub copy_insert: &'static str,
    pub copy_create: &'static str,
    pub copy_definition: &'static str,
    // Notices
    pub copied: &'static str,
    // A failure on one line, opened in full
    pub show_error: &'static str,
    pub copy: &'static str,
    /// The macOS menu bar.
    pub menu: AppMenuMessages,
}

/// The macOS menu bar's menus and items.
pub struct AppMenuMessages {
    pub about: &'static str,
    pub services: &'static str,
    pub hide: &'static str,
    pub hide_others: &'static str,
    pub show_all: &'static str,
    pub quit: &'static str,
    pub edit: &'static str,
    pub undo: &'static str,
    pub redo: &'static str,
    pub cut: &'static str,
    pub copy: &'static str,
    pub paste: &'static str,
    pub select_all: &'static str,
    pub view: &'static str,
    pub full_screen: &'static str,
    pub window: &'static str,
    pub minimize: &'static str,
    pub zoom: &'static str,
}
