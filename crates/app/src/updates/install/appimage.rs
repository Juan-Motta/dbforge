//! An AppImage update: the new AppImage replaces the file the running one
//! was started from (`$APPIMAGE`). It is written beside it first and
//! renamed over it, so the path always holds a whole AppImage, with the
//! previous file's permissions (it stays executable).

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// An AppImage is an ELF executable.
pub fn is_elf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
}

/// Replaces the AppImage at `path` with `bytes`.
pub fn install(bytes: &[u8], path: &Path) -> io::Result<()> {
    if !is_elf(bytes) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the update is not an AppImage"));
    }
    let permissions = fs::metadata(path)?.permissions();
    let dir = path.parent().ok_or_else(|| io::Error::other("the AppImage has no folder"))?;
    let mut staged = tempfile::Builder::new().prefix(".dbforge-update").tempfile_in(dir)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    fs::set_permissions(staged.path(), permissions)?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn replaces_the_appimage_keeping_it_executable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DBForge.AppImage");
        fs::write(&path, b"\x7fELF old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        install(b"\x7fELF new", &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"\x7fELF new");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "nothing staged is left");
    }

    #[test]
    fn refuses_what_is_not_an_appimage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("DBForge.AppImage");
        fs::write(&path, b"\x7fELF old").unwrap();
        assert_eq!(install(b"<html>", &path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), b"\x7fELF old");
    }
}
