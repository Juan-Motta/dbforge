//! The texts of the settings dialog and of its list of tags.

use dbforge_core::connections::tags::TagColor;

/// The settings dialog's texts in one language.
pub struct SettingsMessages {
    pub title: &'static str,
    pub close: &'static str,
    pub appearance: &'static str,
    pub appearance_intro: &'static str,
    pub theme: &'static str,
    pub dark: &'static str,
    pub light: &'static str,
    pub system: &'static str,
    pub language: &'static str,
    pub language_intro: &'static str,
    pub interface_language: &'static str,
    pub interface_language_help: &'static str,
    pub tags: &'static str,
    pub exports: &'static str,
    pub exports_intro: &'static str,
    pub agents: &'static str,
    pub agents_intro: &'static str,
    pub agent_access: &'static str,
    pub agent_access_intro: &'static str,
    pub passwords: &'static str,
    pub passwords_intro: &'static str,
    pub backup: &'static str,
    pub backup_intro: &'static str,
    pub storage: &'static str,
    pub keychain: &'static str,
    pub file: &'static str,
    pub keychain_help: &'static str,
    pub file_help: &'static str,
    pub file_location: &'static str,
    pub moved_to: fn(&str) -> String,
    pub move_failed: &'static str,
    pub moving: &'static str,
    pub moving_one: fn(usize, usize) -> String,
    pub removing_one: fn(usize, usize) -> String,
    pub system_may_ask: &'static str,
    pub about: &'static str,
    pub tagline: &'static str,
    pub version: &'static str,
    pub check_automatically: &'static str,
    pub check_automatically_description: &'static str,
    /// Settings › About's button, and the app menu's item.
    pub check_now: &'static str,
}

/// The settings' list of tags in one language.
pub struct TagsMessages {
    pub tags: &'static str,
    pub tags_intro: &'static str,
    pub color: fn(&str) -> String,
    pub color_name: fn(TagColor) -> &'static str,
    pub confirm: &'static str,
    pub confirm_help: &'static str,
    pub remove: fn(&str) -> String,
    pub add: &'static str,
    pub new_name: fn(usize) -> String,
}
