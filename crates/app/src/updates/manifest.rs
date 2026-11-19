//! `latest.json`, the manifest each release publishes and every installed
//! copy reads (the Tauri updater of 0.3.x as well as this one), in the
//! shape `scripts/make-update-manifest.sh` writes:
//!
//! ```json
//! { "version": "0.4.0", "notes": "## Features…", "pub_date": "2026-11-19T10:00:00Z",
//!   "platforms": { "darwin-aarch64": { "signature": "…", "url": "https://…" }, … } }
//! ```
//!
//! As Tauri's updater, it also reads the single-platform shape a server
//! answering per platform would send (`url` and `signature` at the top).

use std::collections::BTreeMap;

use semver::Version;
use serde::Deserialize;

/// Where one platform downloads the update, and the update's signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub url: String,
    pub signature: String,
}

/// The release `latest.json` announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// As written, so a signature naming it can be compared literally when it isn't semver.
    pub announced: String,
    pub notes: Option<String>,
    platforms: Platforms,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Platforms {
    /// A server that picked the platform itself.
    One(Platform),
    /// Each platform's, by updater target (`darwin-aarch64`, `linux-x86_64-deb`…).
    Each(BTreeMap<String, Platform>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// Not JSON, or not a manifest's.
    Invalid(String),
    /// The version isn't semver.
    Version(String),
    /// No entry for any of these targets.
    NoPlatform(Vec<String>),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "The update manifest can't be read: {reason}"),
            Self::Version(version) => write!(f, "The update manifest's version isn't valid: {version}"),
            Self::NoPlatform(targets) => {
                write!(f, "The update has no download for this system ({})", targets.join(", "))
            }
        }
    }
}

#[derive(Deserialize)]
struct RawPlatform {
    url: String,
    signature: String,
}

#[derive(Deserialize)]
struct RawRelease {
    version: String,
    notes: Option<String>,
    platforms: Option<BTreeMap<String, RawPlatform>>,
    url: Option<String>,
    signature: Option<String>,
}

/// A version as manifests write it: `1.2.3`, or `v1.2.3` as tags do.
pub fn parse_version(text: &str) -> Result<Version, semver::Error> {
    Version::parse(text.trim().trim_start_matches('v'))
}

fn platform(raw: RawPlatform) -> Result<Platform, ManifestError> {
    url::Url::parse(&raw.url).map_err(|error| ManifestError::Invalid(format!("{}: {error}", raw.url)))?;
    Ok(Platform { url: raw.url, signature: raw.signature })
}

impl Release {
    pub fn parse(json: &[u8]) -> Result<Self, ManifestError> {
        let raw: RawRelease =
            serde_json::from_slice(json).map_err(|error| ManifestError::Invalid(error.to_string()))?;
        let version = parse_version(&raw.version).map_err(|_| ManifestError::Version(raw.version.clone()))?;
        let platforms = match (raw.platforms, raw.url, raw.signature) {
            (Some(each), _, _) => Platforms::Each(
                each.into_iter()
                    .map(|(target, raw)| platform(raw).map(|platform| (target, platform)))
                    .collect::<Result<_, _>>()?,
            ),
            (None, Some(url), Some(signature)) => Platforms::One(platform(RawPlatform { url, signature })?),
            _ => return Err(ManifestError::Invalid("it has no platforms".into())),
        };
        Ok(Self { version, announced: raw.version, notes: raw.notes, platforms })
    }

    /// Whether it is newer than `current`, which is all the updater asks.
    pub fn is_newer_than(&self, current: &Version) -> bool {
        &self.version > current
    }

    /// The download for the first of `targets` the manifest lists (most
    /// specific first, see [`super::target::Target::keys`]).
    pub fn platform(&self, targets: &[String]) -> Result<&Platform, ManifestError> {
        match &self.platforms {
            Platforms::One(platform) => Ok(platform),
            Platforms::Each(each) => targets
                .iter()
                .find_map(|target| each.get(target))
                .ok_or_else(|| ManifestError::NoPlatform(targets.to_vec())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `scripts/make-update-manifest.sh` writes for a release.
    const MANIFEST: &str = r###"{
      "version": "0.4.0",
      "notes": "## Features\n\n- **app:** native window",
      "pub_date": "2026-11-19T10:00:00Z",
      "platforms": {
        "darwin-aarch64": { "signature": "mac", "url": "https://example.com/DBForge-0.4.0.app.tar.gz" },
        "darwin-x86_64": { "signature": "mac", "url": "https://example.com/DBForge-0.4.0.app.tar.gz" },
        "linux-x86_64": { "signature": "appimage", "url": "https://example.com/DBForge-0.4.0-amd64.AppImage" },
        "linux-x86_64-appimage": { "signature": "appimage", "url": "https://example.com/DBForge-0.4.0-amd64.AppImage" },
        "linux-x86_64-deb": { "signature": "deb", "url": "https://example.com/DBForge-0.4.0-amd64.deb" },
        "windows-x86_64": { "signature": "setup", "url": "https://example.com/DBForge-0.4.0-x64-setup.exe" },
        "windows-x86_64-nsis": { "signature": "setup", "url": "https://example.com/DBForge-0.4.0-x64-setup.exe" }
      }
    }"###;

    fn targets(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn reads_the_manifest_the_release_publishes() {
        let release = Release::parse(MANIFEST.as_bytes()).unwrap();
        assert_eq!(release.version, Version::new(0, 4, 0));
        assert_eq!(release.announced, "0.4.0");
        assert_eq!(release.notes.as_deref(), Some("## Features\n\n- **app:** native window"));
    }

    #[test]
    fn picks_the_installer_s_own_entry_before_the_platform_s() {
        let release = Release::parse(MANIFEST.as_bytes()).unwrap();
        let deb = release.platform(&targets(&["linux-x86_64-deb", "linux-x86_64"])).unwrap();
        assert_eq!(deb.signature, "deb");
        let mac = release.platform(&targets(&["darwin-aarch64-app", "darwin-aarch64"])).unwrap();
        assert_eq!(mac.url, "https://example.com/DBForge-0.4.0.app.tar.gz");
    }

    #[test]
    fn names_the_targets_it_looked_for_when_none_is_listed() {
        let release = Release::parse(MANIFEST.as_bytes()).unwrap();
        let wanted = targets(&["linux-aarch64-deb", "linux-aarch64"]);
        assert_eq!(release.platform(&wanted), Err(ManifestError::NoPlatform(wanted.clone())));
    }

    #[test]
    fn reads_a_server_s_single_platform_answer() {
        let json = r#"{ "version": "v1.0.0", "url": "https://example.com/u.tar.gz", "signature": "sig" }"#;
        let release = Release::parse(json.as_bytes()).unwrap();
        assert_eq!(release.version, Version::new(1, 0, 0), "a leading v is a tag's");
        assert_eq!(release.platform(&targets(&["anything"])).unwrap().signature, "sig");
    }

    #[test]
    fn refuses_a_manifest_it_can_t_use() {
        assert!(matches!(Release::parse(b"<html>"), Err(ManifestError::Invalid(_))));
        assert!(matches!(Release::parse(br#"{ "version": "1.0.0" }"#), Err(ManifestError::Invalid(_))));
        assert!(matches!(
            Release::parse(br#"{ "version": "one", "url": "https://e.com/x", "signature": "s" }"#),
            Err(ManifestError::Version(_))
        ));
        assert!(matches!(
            Release::parse(br#"{ "version": "1.0.0", "url": "not a url", "signature": "s" }"#),
            Err(ManifestError::Invalid(_))
        ));
    }

    #[test]
    fn updates_only_to_a_newer_version() {
        let release = |version: &str| {
            let json = format!(r#"{{ "version": "{version}", "url": "https://e.com/x", "signature": "s" }}"#);
            Release::parse(json.as_bytes()).unwrap()
        };
        let current = Version::new(0, 3, 5);
        assert!(release("0.4.0").is_newer_than(&current));
        assert!(release("0.3.6").is_newer_than(&current));
        assert!(!release("0.3.5").is_newer_than(&current));
        assert!(!release("0.3.4").is_newer_than(&current));
        assert!(!release("0.3.5-rc.1").is_newer_than(&current), "a pre-release comes before its version");
        assert!(release("0.4.0-rc.1").is_newer_than(&current));
    }
}
