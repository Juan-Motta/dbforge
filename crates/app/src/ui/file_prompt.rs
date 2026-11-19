//! The OS's own file dialogs, on every platform GPUI runs on: a file to
//! open, a folder, and where to save a new file. Each resolves to `None`
//! when the user cancels (or the platform has no dialog).
//!
//! Development builds take `DBFORGE_PICK` instead of asking, so a dialog
//! can be run without a person at the keyboard: the path itself, or for a
//! new file, the suggested name inside it when it is a folder.

use std::future::Future;
use std::path::{Path, PathBuf};

use gpui::{App, PathPromptOptions};

/// What a development build answers instead of asking.
fn preset() -> Option<PathBuf> {
    if cfg!(debug_assertions) { std::env::var_os("DBFORGE_PICK").map(PathBuf::from) } else { None }
}

fn pick(options: PathPromptOptions, cx: &App) -> impl Future<Output = Option<PathBuf>> + 'static {
    let preset = preset();
    let prompt = preset.is_none().then(|| cx.prompt_for_paths(options));
    async move {
        if preset.is_some() {
            return preset;
        }
        let Ok(Ok(Some(paths))) = prompt?.await else { return None };
        paths.into_iter().next()
    }
}

/// One existing file.
pub fn pick_file(cx: &App) -> impl Future<Output = Option<PathBuf>> + 'static {
    pick(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None }, cx)
}

/// One folder.
pub fn pick_folder(cx: &App) -> impl Future<Output = Option<PathBuf>> + 'static {
    pick(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None }, cx)
}

/// Where to save a new file, starting in `directory` with `name` suggested.
pub fn pick_new_path(directory: &Path, name: &str, cx: &App) -> impl Future<Output = Option<PathBuf>> + 'static {
    let preset = preset().map(|path| if path.is_dir() { path.join(name) } else { path });
    let prompt = preset.is_none().then(|| cx.prompt_for_new_path(directory, Some(name)));
    async move {
        if preset.is_some() {
            return preset;
        }
        let Ok(Ok(path)) = prompt?.await else { return None };
        path
    }
}

/// Where save dialogs start: the user's Documents, else their home.
pub fn documents_dir() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}
