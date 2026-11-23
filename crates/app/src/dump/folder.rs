//! Where database exports go: the folder chosen in Settings, kept under the
//! former web UI's `dbforge.exportFolder` preference, else `DBForge` in the user's
//! Documents.

use gpui::{App, BorrowAppContext};

use super::default_export_folder;
use crate::backend::Backend;
use crate::preferences::{Preferences, keys};

pub struct ExportFolder;

impl ExportFolder {
    /// The folder the user chose; `None` while the default is used.
    pub fn chosen(cx: &App) -> Option<String> {
        Preferences::global(cx).get(keys::EXPORT_FOLDER).filter(|folder| !folder.is_empty()).map(str::to_owned)
    }

    /// `DBForge` in the user's Documents; `None` when the OS has no Documents folder.
    pub fn fallback() -> Option<String> {
        dirs::document_dir().map(|documents| default_export_folder(&documents.to_string_lossy()))
    }

    /// Where exports go now.
    pub fn folder(cx: &App) -> Option<String> {
        Self::chosen(cx).or_else(Self::fallback)
    }

    /// Tells the core where exports go now and whenever the preferences
    /// change, so agents' backups (MCP) go where the app's do.
    pub fn keep_core_in_sync(cx: &mut App) {
        Self::tell_core(cx);
        cx.observe_global::<Preferences>(Self::tell_core).detach();
    }

    fn tell_core(cx: &mut App) {
        let folder = Self::folder(cx).map(Into::into);
        Backend::global(cx).core().export_folder().set(folder);
    }

    /// Keeps `folder` as where exports go; `None` goes back to the default.
    pub fn choose(folder: Option<String>, cx: &mut App) {
        cx.update_global::<Preferences, _>(|preferences, _| preferences.set(keys::EXPORT_FOLDER, folder));
    }
}
