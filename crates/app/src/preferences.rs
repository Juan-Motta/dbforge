//! The window's preferences: a string map under the same keys the former web UI
//! kept in `localStorage`, saved whole in `preferences.json` in the config
//! directory. Memory changes at once; the file is written off the UI thread,
//! in order, and once more as the app quits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use dbforge_core::files::JsonFile;
use gpui::{App, Global};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::backend::runtime;

/// The keys, as the former web UI named them, so a backup or a later version reads them alike.
pub mod keys {
    /// `system`, `light` or `dark`.
    pub const THEME: &str = "dbforge.theme";
    /// `en` or `es`.
    pub const LANGUAGE: &str = "dbforge.language";
    /// The folder database exports go to.
    pub const EXPORT_FOLDER: &str = "dbforge.exportFolder";
    /// The last table export's format and options, as JSON.
    pub const EXPORT: &str = "dbforge.export";
    /// The last update announced and when: `<version>@<milliseconds since the epoch>`.
    pub const UPDATES_ANNOUNCED: &str = "dbforge.updates.announced";
    /// Whether updates are checked by themselves: `true` or `false`.
    pub const UPDATES_AUTOMATIC: &str = "dbforge.updates.automatic";
    /// Panel sizes in pixels, and whether the statement console is open.
    pub const EXPLORER_WIDTH: &str = "dbforge.explorerWidth";
    pub const INSPECTOR_WIDTH: &str = "dbforge.inspectorWidth";
    pub const STATEMENTS_HEIGHT: &str = "dbforge.statementsHeight";
    pub const STATEMENTS_OPEN: &str = "dbforge.statementsOpen";
}

type Values = BTreeMap<String, String>;

/// The preferences file and the newest change written to it, so a change
/// written late never replaces a newer one.
pub struct PreferenceFile {
    file: JsonFile<Values>,
    written: Mutex<u64>,
}

impl PreferenceFile {
    /// The preferences saved at `path`; none until there is a file. A file
    /// that can't be read is set aside ([`PreferenceFile::set_aside`]).
    pub fn load(path: PathBuf) -> dbforge_core::Result<Self> {
        Ok(Self { file: JsonFile::load(path, Values::new)?, written: Mutex::new(0) })
    }

    pub fn values(&self) -> Values {
        self.file.get()
    }

    /// Where an unreadable file was moved at start.
    pub fn set_aside(&self) -> Option<&Path> {
        self.file.set_aside()
    }

    /// Saves `values`, the `version`th change, unless a newer one is saved already.
    pub fn save(&self, version: u64, values: Values) -> dbforge_core::Result<()> {
        let mut written = self.written.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if version <= *written {
            return Ok(());
        }
        self.file.update(|saved| {
            *saved = values;
            Ok(())
        })?;
        *written = version;
        Ok(())
    }
}

/// The preferences as the window uses them, a GPUI global. Observe it
/// (`cx.observe_global::<Preferences>`) to follow changes.
pub struct Preferences {
    values: Values,
    version: u64,
    file: Arc<PreferenceFile>,
    saver: UnboundedSender<(u64, Values)>,
}

impl Global for Preferences {}

impl Preferences {
    /// Keeps `file`'s preferences and saves each change to it on the runtime.
    pub fn new(file: PreferenceFile) -> Self {
        let file = Arc::new(file);
        let (saver, mut changes) = unbounded_channel::<(u64, Values)>();
        let saving = file.clone();
        runtime().spawn(async move {
            while let Some(mut change) = changes.recv().await {
                // Only the newest of the changes waiting needs writing.
                while let Ok(newer) = changes.try_recv() {
                    change = newer;
                }
                let file = saving.clone();
                let saved = tokio::task::spawn_blocking(move || file.save(change.0, change.1)).await;
                if let Ok(Err(error)) = saved {
                    eprintln!("DBForge: could not save the preferences: {error}");
                }
            }
        });
        Self { values: file.values(), version: 0, file, saver }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Every preference, for a backup.
    pub fn all(&self) -> &Values {
        &self.values
    }

    /// Where an unreadable preferences file was moved at start.
    pub fn set_aside(&self) -> Option<&Path> {
        self.file.set_aside()
    }

    /// Sets `key` to `value` (`None` removes it) and saves the preferences.
    /// Change through `cx.update_global` so observers hear of it.
    pub fn set(&mut self, key: &str, value: Option<String>) {
        let changed = match value {
            Some(value) => self.values.insert(key.to_owned(), value.clone()) != Some(value),
            None => self.values.remove(key).is_some(),
        };
        if changed {
            self.version += 1;
            // Fails only if the runtime is gone; the quit flush still saves.
            let _ = self.saver.send((self.version, self.values.clone()));
        }
    }

    /// Writes what is not saved yet, now: for the app's last moment.
    pub fn flush(&self) {
        if let Err(error) = self.file.save(self.version, self.values.clone()) {
            eprintln!("DBForge: could not save the preferences: {error}");
        }
    }
}

/// Preferences kept in `dir`, for tests that draw views reading them.
#[cfg(test)]
pub fn test_preferences(cx: &mut App, dir: &Path) {
    let file = PreferenceFile::load(dir.join("preferences.json")).expect("a preferences file");
    cx.set_global(Preferences::new(file));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> Values {
        pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn keeps_the_web_ui_s_keys() {
        assert_eq!(
            [keys::THEME, keys::LANGUAGE, keys::EXPORT_FOLDER, keys::EXPORT],
            ["dbforge.theme", "dbforge.language", "dbforge.exportFolder", "dbforge.export"]
        );
        assert_eq!(keys::UPDATES_ANNOUNCED, "dbforge.updates.announced");
        assert_eq!(
            [keys::UPDATES_AUTOMATIC, keys::EXPLORER_WIDTH, keys::INSPECTOR_WIDTH],
            ["dbforge.updates.automatic", "dbforge.explorerWidth", "dbforge.inspectorWidth"]
        );
        assert_eq!(
            [keys::STATEMENTS_HEIGHT, keys::STATEMENTS_OPEN],
            ["dbforge.statementsHeight", "dbforge.statementsOpen"]
        );
    }

    #[test]
    fn saves_a_string_map_and_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let file = PreferenceFile::load(path.clone()).unwrap();
        assert!(file.values().is_empty(), "none until there is a file");
        file.save(1, values(&[(keys::THEME, "light"), (keys::LANGUAGE, "es")])).unwrap();
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "dbforge.theme": "light", "dbforge.language": "es" }));
        assert_eq!(
            PreferenceFile::load(path).unwrap().values(),
            values(&[(keys::THEME, "light"), (keys::LANGUAGE, "es")])
        );
    }

    #[test]
    fn a_change_written_late_never_replaces_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let file = PreferenceFile::load(path.clone()).unwrap();
        file.save(2, values(&[(keys::THEME, "dark")])).unwrap();
        file.save(1, values(&[(keys::THEME, "light")])).unwrap();
        assert_eq!(PreferenceFile::load(path).unwrap().values(), values(&[(keys::THEME, "dark")]));
    }

    #[test]
    fn an_unreadable_file_is_set_aside_and_none_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        std::fs::write(&path, "{\"dbforge.theme\": ").unwrap();
        let file = PreferenceFile::load(path).unwrap();
        assert!(file.values().is_empty());
        assert!(file.set_aside().is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_s_changes_reach_the_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let mut preferences = Preferences::new(PreferenceFile::load(path.clone()).unwrap());
        preferences.set(keys::THEME, Some("light".into()));
        preferences.set(keys::THEME, Some("system".into()));
        preferences.set(keys::EXPORT_FOLDER, Some("/tmp/out".into()));
        preferences.set(keys::EXPORT_FOLDER, None);
        assert_eq!(preferences.get(keys::THEME), Some("system"), "memory changes at once");
        let expected = values(&[(keys::THEME, "system")]);
        for _ in 0..100 {
            if PreferenceFile::load(path.clone()).unwrap().values() == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the file never held the last change: {:?}", PreferenceFile::load(path).unwrap().values());
    }
}
