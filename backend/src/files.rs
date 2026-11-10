//! Files DBForge keeps: written whole or not at all, and settings kept in
//! memory with the JSON file they live in.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Result;

/// Who may read a file written with [`write_atomic`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Access {
    /// As the system's defaults allow.
    Shared,
    /// Its owner only: passwords and keys.
    Private,
}

/// Writes `bytes` to `path` through a temporary file beside it, synced to disk
/// before it takes the file's place: a crash leaves the old file or the new
/// one, never half of one. A private file is owner-only from the moment it
/// exists, and a temporary file left by a crash is tightened too.
pub fn write_atomic(path: &Path, bytes: &[u8], access: Access) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = temporary(path);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if access == Access::Private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    #[cfg(unix)]
    if access == Access::Private {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// `name.json.tmp` for `name.json`: the name with `.tmp` added, so it never
/// stands for another file of the same stem.
fn temporary(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

/// A value kept in memory and in a JSON file. A change reaches memory only
/// once the file holds it, so what the app shows is always what is saved.
pub struct JsonFile<T> {
    path: PathBuf,
    value: Mutex<T>,
}

impl<T: Serialize + DeserializeOwned + Clone> JsonFile<T> {
    /// The value saved at `path`, or `default` until there is a file.
    pub fn load(path: PathBuf, default: impl FnOnce() -> T) -> Result<Self> {
        let value = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(error) if error.kind() == ErrorKind::NotFound => default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, value: Mutex::new(value) })
    }

    pub fn get(&self) -> T {
        self.lock().clone()
    }

    /// What `read` makes of the value, without copying it.
    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        read(&self.lock())
    }

    /// Applies `change` to a copy of the value and saves it; the value changes
    /// only once saved. Changes are made one at a time. When `change` fails,
    /// nothing is written.
    pub fn update<R>(&self, change: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        let mut current = self.lock();
        let mut next = current.clone();
        let result = change(&mut next)?;
        write_atomic(&self.path, serde_json::to_string_pretty(&next)?.as_bytes(), Access::Shared)?;
        *current = next;
        Ok(result)
    }

    fn lock(&self) -> MutexGuard<'_, T> {
        // A poisoned lock only means another thread panicked; the value is still whole.
        self.value.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;

    #[test]
    fn keeps_a_value_in_its_file_and_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("list.json");
        let file = JsonFile::load(path.clone(), || vec![1]).unwrap();
        assert_eq!(file.get(), [1], "the default until there is a file");
        file.update(|list| {
            list.push(2);
            Ok(())
        })
        .unwrap();
        assert_eq!(JsonFile::<Vec<i32>>::load(path.clone(), Vec::new).unwrap().get(), [1, 2]);
        assert!(!temporary(&path).exists());
    }

    #[test]
    fn a_change_that_fails_or_cannot_be_saved_leaves_the_value_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let file = JsonFile::load(dir.path().join("list.json"), || vec![1]).unwrap();
        let refused = file.update(|list| {
            list.push(2);
            Err::<(), _>(AppError::Invalid("no".into()))
        });
        assert!(refused.is_err());
        assert_eq!(file.get(), [1]);

        // A file where its folder should be: the write fails, and memory keeps the old value.
        let blocked = dir.path().join("blocked");
        let file = JsonFile::load(blocked.join("list.json"), || vec![1]).unwrap();
        fs::write(&blocked, "").unwrap();
        let saved = file.update(|list| {
            list.push(2);
            Ok(())
        });
        assert!(saved.is_err());
        assert_eq!(file.get(), [1]);
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("list.json");
        fs::write(&path, "[1,").unwrap();
        assert!(matches!(JsonFile::<Vec<i32>>::load(path, Vec::new), Err(AppError::Format(_))));
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_its_owners_only_even_over_a_looser_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.json");
        // A temporary file left by a crash, readable by anyone.
        fs::write(temporary(&path), "old").unwrap();
        fs::set_permissions(temporary(&path), fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, b"{}", Access::Private).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        // Two files of the same stem (credentials.bin, credentials.key) each have their own temporary file.
        assert_ne!(temporary(&dir.path().join("a.bin")), temporary(&dir.path().join("a.key")));
        // A file written beside one of the same stem leaves that one alone.
        let other = dir.path().join("backup.tmp");
        fs::write(&other, "mine").unwrap();
        write_atomic(&path, b"[]", Access::Private).unwrap();
        assert_eq!(fs::read_to_string(other).unwrap(), "mine");
    }
}
