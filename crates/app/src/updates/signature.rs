//! The update's signature check, as Tauri's updater made it (so both
//! accept exactly the same updates): `signature` is the base64 of a
//! minisign signature file, `public_key` the base64 of a minisign public key
//! file, and the signature's trusted comment, which the signature covers,
//! names the version it was made for (`…\tversion:0.4.0`). That version
//! must be the one `latest.json` announced, since the manifest itself is
//! not signed: otherwise an old, genuinely signed update could be served as
//! a new version.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};

use super::manifest::parse_version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    /// The public key or the signature isn't minisign's, base64-encoded.
    Unreadable(&'static str),
    /// The signature doesn't sign this download with this key.
    Mismatch,
    /// The signature names no version, and one is required.
    NoVersion,
    /// The signature was made for another version than the one announced.
    OtherVersion { signed: String, announced: String },
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(what) => write!(f, "The update's {what} can't be read"),
            Self::Mismatch => f.write_str("The update's signature is not valid"),
            Self::NoVersion => f.write_str("The update's signature doesn't name its version"),
            Self::OtherVersion { signed, announced } => {
                write!(f, "The update was signed for version {signed}, not {announced}")
            }
        }
    }
}

fn base64_text(encoded: &str, what: &'static str) -> Result<String, SignatureError> {
    let bytes = STANDARD.decode(encoded.trim()).map_err(|_| SignatureError::Unreadable(what))?;
    String::from_utf8(bytes).map_err(|_| SignatureError::Unreadable(what))
}

/// Checks `data`, the downloaded update, against `signature` and
/// `public_key`, then the signed version against `announced`. With
/// `require_signed_version` (DBForge's setting) a signature naming no
/// version is refused.
pub fn verify(
    data: &[u8],
    signature: &str,
    public_key: &str,
    announced: &str,
    require_signed_version: bool,
) -> Result<(), SignatureError> {
    let public_key = PublicKey::decode(&base64_text(public_key, "public key")?)
        .map_err(|_| SignatureError::Unreadable("public key"))?;
    let signature = Signature::decode(&base64_text(signature, "signature")?)
        .map_err(|_| SignatureError::Unreadable("signature"))?;
    public_key.verify(data, &signature, true).map_err(|_| SignatureError::Mismatch)?;
    // Only now is the trusted comment trustworthy: the check above covered it.
    verify_signed_version(signature.trusted_comment(), announced, require_signed_version)
}

fn verify_signed_version(comment: &str, announced: &str, required: bool) -> Result<(), SignatureError> {
    let Some(signed) = signed_version(comment) else {
        return if required { Err(SignatureError::NoVersion) } else { Ok(()) };
    };
    // As semver, so `v1.2.3` and `1.2.3` match; literally when either isn't semver.
    let matches = match (parse_version(signed), parse_version(announced)) {
        (Ok(signed), Ok(announced)) => signed == announced,
        _ => signed == announced,
    };
    if matches {
        Ok(())
    } else {
        Err(SignatureError::OtherVersion { signed: signed.to_owned(), announced: announced.to_owned() })
    }
}

/// The `version:` field of a trusted comment (`timestamp:…\tfile:…\tversion:…`).
fn signed_version(comment: &str) -> Option<&str> {
    comment.split('\t').find_map(|field| field.strip_prefix("version:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with `tauri signer` (tauri-cli 2.12.4) and a throwaway key, so the
    // check is proven against the signatures DBForge 0.3.x was published with.
    const PUBLIC_KEY: &str = include_str!("fixtures/throwaway.key.pub");
    const PAYLOAD: &[u8] = include_bytes!("fixtures/payload.bin");
    /// `tauri signer sign --app-version 1.2.3 payload.bin`.
    const SIGNATURE: &str = include_str!("fixtures/payload.bin.sig");
    /// `tauri signer sign payload.bin`, which names no version.
    const SIGNATURE_WITHOUT_VERSION: &str = include_str!("fixtures/payload.bin.noversion.sig");
    /// DBForge's own public key, which signed every release.
    const DBFORGE_PUBLIC_KEY: &str = include_str!("../../../../packaging/updater.pub");

    #[test]
    fn accepts_a_signature_tauri_signer_made_for_the_announced_version() {
        assert_eq!(verify(PAYLOAD, SIGNATURE, PUBLIC_KEY, "1.2.3", true), Ok(()));
        assert_eq!(verify(PAYLOAD, SIGNATURE, PUBLIC_KEY, "v1.2.3", true), Ok(()), "a tag's v is the same version");
    }

    #[test]
    fn refuses_a_changed_download() {
        let mut changed = PAYLOAD.to_vec();
        changed[0] ^= 1;
        assert_eq!(verify(&changed, SIGNATURE, PUBLIC_KEY, "1.2.3", true), Err(SignatureError::Mismatch));
    }

    #[test]
    fn refuses_another_key_s_signature() {
        assert_eq!(verify(PAYLOAD, SIGNATURE, DBFORGE_PUBLIC_KEY, "1.2.3", true), Err(SignatureError::Mismatch));
    }

    #[test]
    fn refuses_a_signature_made_for_another_version() {
        assert_eq!(
            verify(PAYLOAD, SIGNATURE, PUBLIC_KEY, "1.2.4", true),
            Err(SignatureError::OtherVersion { signed: "1.2.3".into(), announced: "1.2.4".into() })
        );
    }

    #[test]
    fn requires_a_signed_version_only_when_asked() {
        assert_eq!(
            verify(PAYLOAD, SIGNATURE_WITHOUT_VERSION, PUBLIC_KEY, "1.2.3", true),
            Err(SignatureError::NoVersion)
        );
        assert_eq!(verify(PAYLOAD, SIGNATURE_WITHOUT_VERSION, PUBLIC_KEY, "1.2.3", false), Ok(()));
    }

    #[test]
    fn refuses_what_isn_t_a_signature_or_a_key() {
        assert_eq!(
            verify(PAYLOAD, "not base64!", PUBLIC_KEY, "1.2.3", true),
            Err(SignatureError::Unreadable("signature"))
        );
        assert_eq!(
            verify(PAYLOAD, SIGNATURE, "bm90IGEga2V5", "1.2.3", true),
            Err(SignatureError::Unreadable("public key"))
        );
    }
}
