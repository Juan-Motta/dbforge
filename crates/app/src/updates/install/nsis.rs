//! A Windows update: the new `DBForge-<version>-x64-setup.exe`, run as
//! Tauri's updater ran it, so either installs it the same: passive (`/P`,
//! a progress window and no questions), as an update (`/UPDATE`: shortcuts
//! and settings kept), then restarting DBForge (`/R`) with the arguments it
//! was started with (`/ARGS …`). The app quits right after starting it, so
//! the setup can replace `DBForge.exe` (it closes a copy still running).

/// A Windows executable starts with `MZ`.
pub fn is_exe(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
}

/// The setup's arguments, `current` being DBForge's own (without the executable).
pub fn arguments(current: &[String]) -> Vec<String> {
    let mut arguments: Vec<String> = ["/P", "/UPDATE", "/R", "/ARGS"].map(str::to_owned).to_vec();
    arguments.extend(current.iter().cloned());
    arguments
}

/// Saves the setup `bytes` of DBForge `version` and starts it.
#[cfg(windows)]
pub fn run(bytes: &[u8], version: &str) -> Result<(), String> {
    if !is_exe(bytes) {
        return Err("The update is not a Windows installer".into());
    }
    // Kept: the setup runs after DBForge has quit, and Windows cleans the temporary folder.
    let dir = tempfile::Builder::new()
        .prefix(&format!("DBForge-{version}-updater-"))
        .tempdir()
        .map_err(|error| format!("The update couldn't be saved: {error}"))?
        .keep();
    let setup = dir.join(format!("DBForge-{version}-x64-setup.exe"));
    std::fs::write(&setup, bytes).map_err(|error| format!("The update couldn't be saved: {error}"))?;
    let current: Vec<String> = std::env::args().skip(1).collect();
    std::process::Command::new(&setup)
        .args(arguments(&current))
        .spawn()
        .map_err(|error| format!("The installer couldn't start: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_the_setup_passive_as_an_update_that_restarts_dbforge() {
        assert_eq!(arguments(&[]), ["/P", "/UPDATE", "/R", "/ARGS"]);
        assert_eq!(arguments(&["--flag".into()]), ["/P", "/UPDATE", "/R", "/ARGS", "--flag"]);
        assert!(is_exe(b"MZ\x90\x00"));
        assert!(!is_exe(b"\x7fELF"));
    }
}
