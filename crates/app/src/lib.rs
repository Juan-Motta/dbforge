//! DBForge's window, drawn with GPUI over `dbforge-core`. The shell lays out
//! the explorer, toolbar, tabs and main area; features fill them in. Views
//! reach the core through [`backend::Backend`], hear its events through
//! [`events::EventHub`], and share the theme, language and preferences as
//! GPUI globals.

pub mod app;
pub mod assets;
pub mod backend;
pub mod connections;
pub mod events;
pub mod i18n;
pub mod preferences;
pub mod shell;
pub mod theme;
pub mod ui;

pub use app::run;
