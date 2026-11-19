//! Where an update stands, as the former web UI's `useUpdates.ts`: an automatic
//! check says something only when there is an update; a check the user asks
//! for (the menu, Settings › About) always gets an answer.

use std::sync::Arc;

use semver::Version;

use super::manifest::{ManifestError, Release};
use super::target::{Installer, Target};

/// A newer version, ready to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// The version the manifest announced, as written.
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub url: String,
    pub signature: String,
    /// How it replaces this copy.
    pub installer: Installer,
}

/// The update `release` offers this copy (version `current`, on `target`),
/// or `None` when it is not newer. A release that is not newer is no error
/// even when it has nothing for this target, as for Tauri's updater.
pub fn update_from(release: Release, current: &Version, target: &Target) -> Result<Option<Update>, ManifestError> {
    if !release.is_newer_than(current) {
        return Ok(None);
    }
    let platform = release.platform(&target.keys())?.clone();
    Ok(Some(Update {
        version: release.announced,
        current_version: current.to_string(),
        notes: release.notes,
        url: platform.url,
        signature: platform.signature,
        installer: target.installer,
    }))
}

/// What the install is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Bytes so far, of how many when the server said.
    Downloading { downloaded: u64, total: Option<u64> },
    /// Checking the signature and putting the new version in place.
    Installing,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    /// Nothing to show.
    #[default]
    Idle,
    /// A check the user asked for, running; an automatic one shows nothing.
    Checking,
    Available(Arc<Update>),
    UpToDate,
    Installing {
        update: Arc<Update>,
        stage: Stage,
    },
    /// What went wrong; with the update when it was its download or install.
    Failed {
        error: String,
        update: Option<Arc<Update>>,
    },
}

impl Phase {
    /// After a check: `manual` when the user asked for it.
    pub fn checked(manual: bool, result: Result<Option<Update>, String>) -> Self {
        match result {
            Ok(Some(update)) => Self::Available(Arc::new(update)),
            Ok(None) if manual => Self::UpToDate,
            // An automatic check that fails (offline, say) stays quiet; the next one tries again.
            Err(error) if manual => Self::Failed { error, update: None },
            Ok(None) | Err(_) => Self::Idle,
        }
    }

    /// While checking or installing, nothing else starts.
    pub fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Installing { .. })
    }

    /// Whether the update dialog shows.
    pub fn shown(&self) -> bool {
        *self != Self::Idle
    }

    /// The update the dialog's Install (or Try Again) would install.
    pub fn installable(&self) -> Option<&Arc<Update>> {
        match self {
            Self::Available(update) | Self::Failed { update: Some(update), .. } => Some(update),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        let json = format!(
            r#"{{ "version": "{version}", "notes": "n", "platforms": {{
                "linux-x86_64": {{ "url": "https://e.com/a.AppImage", "signature": "appimage" }},
                "linux-x86_64-deb": {{ "url": "https://e.com/a.deb", "signature": "deb" }} }} }}"#
        );
        Release::parse(json.as_bytes()).unwrap()
    }

    fn target(installer: Installer) -> Target {
        Target { os: "linux", arch: "x86_64", installer }
    }

    fn update(version: &str) -> Update {
        update_from(release(version), &Version::new(0, 3, 5), &target(Installer::Deb)).unwrap().unwrap()
    }

    #[test]
    fn offers_a_newer_release_s_download_for_this_package() {
        let deb = update("0.4.0");
        assert_eq!((deb.version.as_str(), deb.current_version.as_str()), ("0.4.0", "0.3.5"));
        assert_eq!((deb.url.as_str(), deb.signature.as_str()), ("https://e.com/a.deb", "deb"));
        assert_eq!(deb.notes.as_deref(), Some("n"));
        let appimage =
            update_from(release("0.4.0"), &Version::new(0, 3, 5), &target(Installer::AppImage)).unwrap().unwrap();
        assert_eq!(appimage.signature, "appimage", "falls back to the platform's entry");
    }

    #[test]
    fn offers_nothing_when_the_release_is_not_newer_even_without_this_target() {
        let mac = Target { os: "darwin", arch: "aarch64", installer: Installer::App };
        assert_eq!(update_from(release("0.3.5"), &Version::new(0, 3, 5), &mac), Ok(None));
        assert!(matches!(
            update_from(release("0.4.0"), &Version::new(0, 3, 5), &mac),
            Err(ManifestError::NoPlatform(_))
        ));
    }

    #[test]
    fn answers_a_check_the_user_asked_for_and_keeps_automatic_ones_quiet() {
        assert_eq!(Phase::checked(true, Ok(None)), Phase::UpToDate);
        assert_eq!(Phase::checked(false, Ok(None)), Phase::Idle);
        assert_eq!(
            Phase::checked(true, Err("offline".into())),
            Phase::Failed { error: "offline".into(), update: None }
        );
        assert_eq!(Phase::checked(false, Err("offline".into())), Phase::Idle);
        assert_eq!(Phase::checked(false, Ok(Some(update("0.4.0")))), Phase::Available(Arc::new(update("0.4.0"))));
    }

    #[test]
    fn starts_nothing_while_checking_or_installing() {
        let update = Arc::new(update("0.4.0"));
        assert!(Phase::Checking.busy());
        assert!(Phase::Installing { update: update.clone(), stage: Stage::Installing }.busy());
        assert!(!Phase::Available(update.clone()).busy());
        assert!(!Phase::Idle.shown());
        assert!(Phase::UpToDate.shown());
    }

    #[test]
    fn installs_the_update_offered_or_the_one_that_failed() {
        let update = Arc::new(update("0.4.0"));
        assert_eq!(Phase::Available(update.clone()).installable(), Some(&update));
        let failed = Phase::Failed { error: "x".into(), update: Some(update.clone()) };
        assert_eq!(failed.installable(), Some(&update));
        assert_eq!(Phase::Failed { error: "x".into(), update: None }.installable(), None);
        assert_eq!(Phase::UpToDate.installable(), None);
    }
}
