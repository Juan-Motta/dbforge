//! A .deb update: the new package, installed with `dpkg -i` through
//! `pkexec`, which asks for an administrator's password, as Tauri's updater
//! did first. (Tauri then fell back to reading a password through zenity or
//! kdialog for `sudo`; DBForge never handles the password itself.)

/// A Debian package is an `ar` archive whose first member is `debian-binary`.
pub fn is_deb(bytes: &[u8]) -> bool {
    bytes.starts_with(b"!<arch>\ndebian-binary")
}

/// Installs the package `bytes` of DBForge `version`.
#[cfg(target_os = "linux")]
pub fn install(bytes: &[u8], version: &str) -> Result<(), String> {
    if !is_deb(bytes) {
        return Err("The update is not a Debian package".into());
    }
    let dir = tempfile::Builder::new()
        .prefix("dbforge-deb-update")
        .tempdir()
        .map_err(|error| format!("The update couldn't be saved: {error}"))?;
    let package = dir.path().join(format!("DBForge-{version}-amd64.deb"));
    std::fs::write(&package, bytes).map_err(|error| format!("The update couldn't be saved: {error}"))?;
    let status = std::process::Command::new("pkexec")
        .arg("dpkg")
        .arg("-i")
        .arg(&package)
        .status()
        .map_err(|error| format!("pkexec couldn't run dpkg: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("dpkg couldn't install the update ({status})")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_debian_package() {
        assert!(is_deb(b"!<arch>\ndebian-binary   1700000000  0     0     100644  4         `\n2.0\n"));
        assert!(!is_deb(b"!<arch>\nlibfoo.o"));
        assert!(!is_deb(b"\x7fELF"));
    }
}
