//! Putting a downloaded (and verified) update in place of this copy, the
//! way Tauri's updater did, so an update installs the same whether a 0.3.x
//! copy or this one downloaded it:
//!
//! - macOS: the `.app.tar.gz` holds `DBForge.app`; it is unpacked beside
//!   the running bundle and swapped with it in one step ([`bundle`]), asking
//!   for an administrator only when the bundle's folder isn't writable.
//! - Linux AppImage: the new AppImage replaces the file the AppImage runtime
//!   runs from (`$APPIMAGE`), keeping its permissions ([`appimage`]).
//! - Linux .deb: `dpkg -i` through `pkexec`, which asks for an
//!   administrator's password ([`deb`]).
//! - Windows: the NSIS setup runs passive (`/P /UPDATE /R`), and the app
//!   quits so it can replace the executable and start the new version
//!   ([`nsis`]).

use std::path::{Path, PathBuf};

use super::target::Installer;

#[cfg(unix)]
pub mod appimage;
#[cfg(unix)]
pub mod bundle;
pub mod deb;
pub mod nsis;

/// What starts the new version once the update is in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relaunch {
    /// Start this executable, then quit.
    Start(PathBuf),
    /// The installer that was started takes over (and starts DBForge); quit now.
    Quit,
}

/// Where this copy is installed, for the update to replace: the `.app`
/// bundle on macOS, the AppImage file, or the running executable (a .deb's
/// `/usr/bin/DBForge`, the Windows install's `DBForge.exe`).
pub fn install_place(installer: Installer, executable: &Path, appimage: Option<&Path>) -> Result<PathBuf, String> {
    match installer {
        Installer::App => bundle_of(executable)
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("DBForge isn't running from an app bundle ({})", executable.display())),
        Installer::AppImage => {
            appimage.map(Path::to_path_buf).ok_or_else(|| "DBForge isn't running from an AppImage".to_owned())
        }
        Installer::Deb | Installer::Nsis => Ok(executable.to_path_buf()),
    }
}

/// The `.app` bundle holding `executable` (`…/DBForge.app/Contents/MacOS/DBForge`).
pub fn bundle_of(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|extension| extension == "app");
    is_bundle.then_some(bundle)
}

/// The executable inside `bundle` named `name`, as the swapped bundle holds it.
pub fn bundle_executable(bundle: &Path, name: &std::ffi::OsStr) -> PathBuf {
    bundle.join("Contents").join("MacOS").join(name)
}

/// Installs `bytes`, an update of kind `installer`, over `place` (see
/// [`install_place`]); what starts the new version. Blocking: call it off
/// the UI thread.
pub fn install(installer: Installer, bytes: &[u8], place: &Path, version: &str) -> Result<Relaunch, String> {
    match installer {
        #[cfg(target_os = "macos")]
        Installer::App => {
            bundle::install(bytes, place)
                .map_err(|error| format!("The new version couldn't be put in place: {error}"))?;
            let name = place_executable_name(place)?;
            Ok(Relaunch::Start(bundle_executable(place, &name)))
        }
        #[cfg(unix)]
        Installer::AppImage => {
            appimage::install(bytes, place)
                .map_err(|error| format!("The new AppImage couldn't be written: {error}"))?;
            Ok(Relaunch::Start(place.to_path_buf()))
        }
        #[cfg(target_os = "linux")]
        Installer::Deb => {
            deb::install(bytes, version)?;
            Ok(Relaunch::Start(place.to_path_buf()))
        }
        #[cfg(windows)]
        Installer::Nsis => {
            nsis::run(bytes, version)?;
            Ok(Relaunch::Quit)
        }
        #[allow(unreachable_patterns)]
        _ => {
            let _ = (bytes, place, version);
            Err(format!("A {} update can't be installed on this system", installer.name()))
        }
    }
}

/// The executable's name the running bundle's `Info.plist` points at: the
/// running executable's own name.
#[cfg(target_os = "macos")]
fn place_executable_name(_bundle: &Path) -> Result<std::ffi::OsString, String> {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.file_name().map(ToOwned::to_owned))
        .ok_or_else(|| "DBForge's executable can't be found".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bundle_a_mac_app_runs_from() {
        let executable = Path::new("/Applications/DBForge.app/Contents/MacOS/DBForge");
        assert_eq!(bundle_of(executable), Some(Path::new("/Applications/DBForge.app")));
        assert_eq!(bundle_of(Path::new("/Users/me/dbforge/target/debug/dbforge")), None);
        assert_eq!(install_place(Installer::App, executable, None), Ok(PathBuf::from("/Applications/DBForge.app")));
        assert!(install_place(Installer::App, Path::new("/usr/local/bin/dbforge"), None).is_err());
        assert_eq!(
            bundle_executable(Path::new("/Applications/DBForge.app"), "DBForge".as_ref()),
            PathBuf::from("/Applications/DBForge.app/Contents/MacOS/DBForge")
        );
    }

    #[test]
    fn replaces_the_appimage_it_runs_from_and_the_executable_otherwise() {
        let executable = Path::new("/tmp/.mount_DBForgeX/usr/bin/DBForge");
        let appimage = Path::new("/home/me/Apps/DBForge.AppImage");
        assert_eq!(install_place(Installer::AppImage, executable, Some(appimage)), Ok(appimage.to_path_buf()));
        assert!(install_place(Installer::AppImage, executable, None).is_err());
        let deb = Path::new("/usr/bin/DBForge");
        assert_eq!(install_place(Installer::Deb, deb, None), Ok(deb.to_path_buf()));
    }
}
