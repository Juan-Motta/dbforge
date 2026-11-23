//! A temporary file holding what a client tool needs to sign in, so the
//! password never shows on its command line or in its environment, where other
//! users of the computer could read it.

use std::io::Write;
use std::path::Path;

use crate::error::Result;

/// A file in the system's temporary folder that only its owner can read,
/// deleted when dropped: after the tool ran, failed, or was cancelled alike.
pub struct SecretFile(tempfile::NamedTempFile);

impl SecretFile {
    /// A new file holding `contents`, its name ending in `suffix` (`.cnf`, `.yaml`).
    pub fn new(contents: &str, suffix: &str) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("dbforge-").suffix(suffix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o600));
        }
        let mut file = builder.tempfile()?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        Ok(Self(file))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_its_owner_reads_it_and_it_is_gone_once_dropped() {
        let file = SecretFile::new("password=secret\n", ".cnf").unwrap();
        let path = file.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "password=secret\n");
        assert!(
            path.starts_with(std::env::temp_dir().canonicalize().unwrap()) || path.starts_with(std::env::temp_dir())
        );
        assert!(path.display().to_string().ends_with(".cnf"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(file);
        assert!(!path.exists());
    }
}
