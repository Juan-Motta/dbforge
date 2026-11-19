//! Which of `latest.json`'s downloads this copy installs, named as Tauri's
//! updater named them so one manifest serves both: `{os}-{arch}`, and
//! before it `{os}-{arch}-{installer}` for the kind of package that is
//! running (`linux-x86_64-deb` is a .deb, `linux-x86_64-appimage` an AppImage).

/// The kind of package this copy was installed from, which decides how an
/// update replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// A macOS app bundle (from the DMG): the update is a `.app.tar.gz`.
    App,
    /// A Linux AppImage: the update is the new AppImage.
    AppImage,
    /// A Debian package: the update is the new `.deb`, installed with dpkg.
    Deb,
    /// The Windows NSIS installer: the update is the new setup, run passive.
    Nsis,
}

impl Installer {
    /// Its name in the manifest's targets.
    pub fn name(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::AppImage => "appimage",
            Self::Deb => "deb",
            Self::Nsis => "nsis",
        }
    }
}

/// The OS, architecture and package of this copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
    pub installer: Installer,
}

/// The manifest's name for Rust's `target_os`.
pub fn os_name(os: &str) -> Option<&'static str> {
    match os {
        "macos" => Some("darwin"),
        "linux" => Some("linux"),
        "windows" => Some("windows"),
        _ => None,
    }
}

/// The manifest's name for Rust's `target_arch`.
pub fn arch_name(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        "x86" => Some("i686"),
        "arm" => Some("armv7"),
        "riscv64" => Some("riscv64"),
        _ => None,
    }
}

/// The package a copy on `os` (the manifest's name) runs from. On Linux
/// the AppImage runtime sets `APPIMAGE` to the file it runs from; a copy
/// without it was installed from the .deb.
pub fn installer_for(os: &str, appimage: bool) -> Option<Installer> {
    match os {
        "darwin" => Some(Installer::App),
        "linux" if appimage => Some(Installer::AppImage),
        "linux" => Some(Installer::Deb),
        "windows" => Some(Installer::Nsis),
        _ => None,
    }
}

impl Target {
    /// This copy's, or `None` on a system DBForge publishes no update for.
    pub fn current() -> Option<Self> {
        let os = os_name(std::env::consts::OS)?;
        let arch = arch_name(std::env::consts::ARCH)?;
        let appimage = std::env::var_os("APPIMAGE").is_some_and(|path| !path.is_empty());
        Some(Self { os, arch, installer: installer_for(os, appimage)? })
    }

    /// The manifest's entries to look for, most specific first.
    pub fn keys(&self) -> Vec<String> {
        let (os, arch) = (self.os, self.arch);
        vec![format!("{os}-{arch}-{}", self.installer.name()), format!("{os}-{arch}")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_systems_as_the_manifest_does() {
        assert_eq!(os_name("macos"), Some("darwin"));
        assert_eq!(os_name("linux"), Some("linux"));
        assert_eq!(os_name("windows"), Some("windows"));
        assert_eq!(os_name("freebsd"), None);
        assert_eq!(arch_name("x86_64"), Some("x86_64"));
        assert_eq!(arch_name("aarch64"), Some("aarch64"));
        assert_eq!(arch_name("x86"), Some("i686"));
    }

    #[test]
    fn tells_an_appimage_from_a_deb_and_names_the_others_package() {
        assert_eq!(installer_for("linux", true), Some(Installer::AppImage));
        assert_eq!(installer_for("linux", false), Some(Installer::Deb));
        assert_eq!(installer_for("darwin", false), Some(Installer::App));
        assert_eq!(installer_for("windows", false), Some(Installer::Nsis));
    }

    #[test]
    fn looks_for_the_package_s_entry_then_the_platform_s() {
        let deb = Target { os: "linux", arch: "x86_64", installer: Installer::Deb };
        assert_eq!(deb.keys(), ["linux-x86_64-deb", "linux-x86_64"]);
        let mac = Target { os: "darwin", arch: "aarch64", installer: Installer::App };
        assert_eq!(mac.keys(), ["darwin-aarch64-app", "darwin-aarch64"]);
        let windows = Target { os: "windows", arch: "x86_64", installer: Installer::Nsis };
        assert_eq!(windows.keys(), ["windows-x86_64-nsis", "windows-x86_64"]);
    }
}
