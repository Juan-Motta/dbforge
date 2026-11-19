//! The settings dialog, as the web UI's `SettingsDialog.tsx`: a list of
//! sections on the left and the chosen one on the right, applied as soon as
//! something is picked. Appearance (the theme), Language, Tags, Passwords
//! (where credentials are kept) and About live here; Export and import,
//! Agents (MCP) and Backup join from their own features ([`Section::available`]).

mod dialog;
mod passwords;

use dbforge_core::credentials::{Phase, Storage, SwitchProgress};

pub use dialog::{SettingsDialog, SettingsEvent, key_bindings};
pub use passwords::PasswordsSection;

use crate::i18n::SettingsMessages;

/// A section of the dialog, in the order the web UI lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Appearance,
    Language,
    Tags,
    Exports,
    Agents,
    Passwords,
    Backup,
    About,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Appearance,
        Section::Language,
        Section::Tags,
        Section::Exports,
        Section::Agents,
        Section::Passwords,
        Section::Backup,
        Section::About,
    ];

    /// The sections this app draws, in order. Export and import and Backup
    /// come with their features (T10), Agents with the MCP settings (T11):
    /// each joins the list once its body exists.
    pub fn available() -> impl Iterator<Item = Section> {
        Self::ALL.into_iter().filter(|section| !matches!(section, Section::Exports | Section::Agents | Section::Backup))
    }

    /// `DBFORGE_OPEN=settings#<name>`: the section named so.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Language => "language",
            Self::Tags => "tags",
            Self::Exports => "exports",
            Self::Agents => "agents",
            Self::Passwords => "passwords",
            Self::Backup => "backup",
            Self::About => "about",
        }
    }

    pub fn label(self, m: &SettingsMessages) -> &'static str {
        match self {
            Self::Appearance => m.appearance,
            Self::Language => m.language,
            Self::Tags => m.tags,
            Self::Exports => m.exports,
            Self::Agents => m.agents,
            Self::Passwords => m.passwords,
            Self::Backup => m.backup,
            Self::About => m.about,
        }
    }
}

/// The places passwords can be kept, in the order the settings offer them.
pub const STORAGES: [Storage; 2] = [Storage::Keychain, Storage::File];

/// A storage's name as the settings show it.
pub fn storage_label(storage: Storage, m: &SettingsMessages) -> &'static str {
    match storage {
        Storage::Keychain => m.keychain,
        Storage::File => m.file,
    }
}

/// Whether picking `to` moves the passwords: only when the current storage
/// is known, differs, and no move is running.
pub fn should_switch(current: Option<Storage>, moving: bool, to: Storage) -> bool {
    !moving && current.is_some_and(|current| current != to)
}

/// What a running move says: the password it is on, or that it started.
pub fn progress_text(progress: Option<SwitchProgress>, m: &SettingsMessages) -> String {
    match progress {
        None => m.moving.to_owned(),
        Some(SwitchProgress { phase: Phase::Moving, current, total }) => (m.moving_one)(current, total),
        Some(SwitchProgress { phase: Phase::Removing, current, total }) => (m.removing_one)(current, total),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn lists_the_sections_in_the_web_ui_s_order_and_reads_their_names() {
        let names: Vec<&str> = Section::ALL.iter().map(|section| section.name()).collect();
        assert_eq!(names, ["appearance", "language", "tags", "exports", "agents", "passwords", "backup", "about"]);
        for section in Section::ALL {
            assert_eq!(Section::parse(section.name()), Some(section));
        }
        assert_eq!(Section::parse("nope"), None);
        let shown: Vec<Section> = Section::available().collect();
        assert_eq!(shown, [Section::Appearance, Section::Language, Section::Tags, Section::Passwords, Section::About]);
    }

    #[test]
    fn moves_the_passwords_only_to_another_known_storage_and_one_move_at_a_time() {
        assert!(should_switch(Some(Storage::Keychain), false, Storage::File));
        assert!(!should_switch(Some(Storage::File), false, Storage::File), "already there");
        assert!(!should_switch(None, false, Storage::File), "not read yet");
        assert!(!should_switch(Some(Storage::Keychain), true, Storage::File), "a move is running");
    }

    #[test]
    fn says_which_password_a_move_is_on() {
        let m = &Language::En.messages().settings_dialog;
        assert_eq!(progress_text(None, m), "Moving your passwords…");
        let at = |phase, current| Some(SwitchProgress { phase, current, total: 3 });
        assert_eq!(progress_text(at(Phase::Moving, 2), m), "Moving password 2 of 3…");
        assert_eq!(progress_text(at(Phase::Removing, 1), m), "Removing the old copy 1 of 3…");
        assert_eq!((m.moved_to)(storage_label(Storage::File, m)), "Your passwords are now in the encrypted file");
        let es = &Language::Es.messages().settings_dialog;
        assert_eq!(
            (es.moved_to)(storage_label(Storage::Keychain, es)),
            "Tus contraseñas ahora están en el llavero del sistema"
        );
    }
}
