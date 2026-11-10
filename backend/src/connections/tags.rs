//! The tags a connection can carry (its environment): PROD, STAGE, TEST and
//! DEV by default, renamed, recolored, removed or added in the settings.

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

/// The longest name a badge has room for.
const MAX_NAME: usize = 16;

/// A palette that reads in both themes; the interface owns the actual colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagColor {
    Red,
    Orange,
    Amber,
    Green,
    Teal,
    Blue,
    Violet,
    Pink,
    Gray,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    /// What connections store; it never changes when the tag is renamed.
    pub id: String,
    pub name: String,
    pub color: TagColor,
    /// Changing data on a connection with this tag asks first (production).
    pub confirm: bool,
}

/// The defaults keep the ids environments had before tags, so saved
/// connections keep theirs.
fn defaults() -> Vec<Tag> {
    let tag = |id: &str, name: &str, color, confirm| Tag { id: id.into(), name: name.into(), color, confirm };
    vec![
        tag("production", "PROD", TagColor::Red, true),
        tag("staging", "STAGE", TagColor::Amber, false),
        tag("test", "TEST", TagColor::Violet, false),
        tag("development", "DEV", TagColor::Blue, false),
    ]
}

pub struct TagStore {
    path: PathBuf,
    tags: Mutex<Vec<Tag>>,
}

impl TagStore {
    /// The tags saved at `path`; the defaults until the user changes them.
    pub fn load(path: PathBuf) -> Result<Self> {
        let tags = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(error) if error.kind() == ErrorKind::NotFound => defaults(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, tags: Mutex::new(tags) })
    }

    pub fn list(&self) -> Vec<Tag> {
        self.lock().clone()
    }

    /// Replaces the tags with `tags`, in that order, after checking them.
    pub fn replace(&self, tags: Vec<Tag>) -> Result<()> {
        let tags = check(tags)?;
        let mut current = self.lock();
        self.persist(&tags)?;
        *current = tags;
        Ok(())
    }

    /// Through a temporary file synced to disk, so a crash never leaves half a file.
    fn persist(&self, tags: &[Tag]) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(serde_json::to_string_pretty(tags)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Tag>> {
        self.tags.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Trimmed names, each present, short enough and unique (ignoring case), and unique ids.
fn check(tags: Vec<Tag>) -> Result<Vec<Tag>> {
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    tags.into_iter()
        .map(|tag| {
            let name = tag.name.trim().to_string();
            if name.is_empty() {
                return Err(AppError::Invalid("Every tag needs a name.".into()));
            }
            if name.chars().count() > MAX_NAME {
                return Err(AppError::Invalid(format!("Tag names are at most {MAX_NAME} characters.")));
            }
            if !names.insert(name.to_lowercase()) {
                return Err(AppError::Invalid(format!("There is already a tag named {name}.")));
            }
            if tag.id.is_empty() || !ids.insert(tag.id.clone()) {
                return Err(AppError::Invalid("Every tag needs its own id.".into()));
            }
            Ok(Tag { name, ..tag })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tags.json");
        (dir, path)
    }

    #[test]
    fn starts_with_prod_stage_test_and_dev() {
        let (_dir, path) = store();
        let tags = TagStore::load(path).unwrap().list();
        let names: Vec<_> = tags.iter().map(|t| (t.id.as_str(), t.name.as_str(), t.confirm)).collect();
        assert_eq!(
            names,
            [
                ("production", "PROD", true),
                ("staging", "STAGE", false),
                ("test", "TEST", false),
                ("development", "DEV", false)
            ]
        );
    }

    #[test]
    fn saves_added_renamed_and_removed_tags_in_order() {
        let (_dir, path) = store();
        let tags = TagStore::load(path.clone()).unwrap();
        let mut next = tags.list();
        next.retain(|t| t.id != "test");
        next[0].name = "  Live ".into();
        next.push(Tag { id: "uat".into(), name: "UAT".into(), color: TagColor::Teal, confirm: true });

        tags.replace(next).unwrap();
        let reloaded = TagStore::load(path).unwrap().list();
        let names: Vec<_> = reloaded.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Live", "STAGE", "DEV", "UAT"]);
        assert_eq!(reloaded[3].color, TagColor::Teal);
    }

    #[test]
    fn refuses_empty_long_or_repeated_names_and_changes_nothing() {
        let (_dir, path) = store();
        let tags = TagStore::load(path.clone()).unwrap();
        let with = |name: &str| {
            let mut next = tags.list();
            next[1].name = name.into();
            tags.replace(next).unwrap_err().to_string()
        };
        assert_eq!(with(" "), "Every tag needs a name.");
        assert_eq!(with("a much too long tag name"), "Tag names are at most 16 characters.");
        assert_eq!(with("prod"), "There is already a tag named prod.");
        let mut twice = tags.list();
        twice[1].id = "production".into();
        assert_eq!(tags.replace(twice).unwrap_err().to_string(), "Every tag needs its own id.");
        assert_eq!(tags.list(), defaults());
        assert!(!path.exists(), "a refused change was saved");
    }
}
