//! The updater's signatures, byte-compatible with what `tauri signer sign`
//! wrote for DBForge 0.3.x, so every installed copy, the Tauri ones
//! included, accepts the updates this signs:
//!
//! - The key is the updater's minisign secret key file, base64-encoded
//!   (`tauri signer generate` wrote it so), and encrypted with a password,
//!   empty when it has none.
//! - A signature file (`<update>.sig`, and the `signature` of `latest.json`)
//!   is the base64 of a minisign signature file: a prehashed Ed25519
//!   signature whose trusted comment reads
//!   `timestamp:<unix seconds>\tfile:<file name>\tversion:<version>`. The
//!   updater accepts an update only when that version is the one
//!   `latest.json` announces (`requireSignedVersion`).
//! - The public key, in `packaging/updater.pub`, is the base64 of the
//!   minisign public key file.

use std::fmt;
use std::io::{Cursor, Read, Seek};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use minisign::{PublicKey, PublicKeyBox, SecretKey, SecretKeyBox, SignatureBox};

/// The signature file's untrusted comment, which nothing verifies.
pub const UNTRUSTED_COMMENT: &str = "signature from the DBForge updater key";

/// Why a key couldn't be read, or a file signed or verified. It never holds
/// the key or its password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The updater's secret key, decrypted, with its public half.
pub struct UpdaterKey {
    secret: SecretKey,
    public: PublicKey,
}

impl UpdaterKey {
    /// The key as `tauri signer generate` wrote it: `encoded` is the
    /// base64 of a minisign secret key file, and `password` the one it was
    /// encrypted with (empty when it was made without one).
    pub fn decode(encoded: &str, password: &str) -> Result<Self, Error> {
        let text = base64_text(encoded).map_err(|reason| Error::new(format!("the updater key is not {reason}")))?;
        let secret_box =
            SecretKeyBox::from_string(&text).map_err(|_| Error::new("the updater key is not a minisign secret key"))?;
        // A wrong password fails the key's checksum; the password itself is never reported.
        let secret = secret_box
            .into_secret_key(Some(password.to_owned()))
            .map_err(|_| Error::new("the updater key can't be decrypted: check its password"))?;
        let public = PublicKey::from_secret_key(&secret)
            .map_err(|_| Error::new("the updater key's public half can't be derived"))?;
        Ok(Self { secret, public })
    }

    /// The public key as the app keeps it (`packaging/updater.pub`): the
    /// base64 of the minisign public key file.
    pub fn public_key(&self) -> Result<String, Error> {
        let public_box = self.public.to_box().map_err(|_| Error::new("the public key can't be written"))?;
        Ok(STANDARD.encode(public_box.into_string()))
    }
}

/// The signed trusted comment: when, which file, and for which version.
pub fn trusted_comment(timestamp: u64, file_name: &str, version: &str) -> String {
    format!("timestamp:{timestamp}\tfile:{file_name}\tversion:{version}")
}

/// Signs `data`, the update file named `file_name` of DBForge `version`;
/// returns the signature file's contents (base64, with no line break).
pub fn sign(
    key: &UpdaterKey,
    data: impl Read,
    file_name: &str,
    version: &str,
    timestamp: u64,
) -> Result<String, Error> {
    if version.trim().is_empty() {
        return Err(Error::new("the version to sign for is empty"));
    }
    let comment = trusted_comment(timestamp, file_name, version);
    let signature = minisign::sign(Some(&key.public), &key.secret, data, Some(&comment), Some(UNTRUSTED_COMMENT))
        .map_err(|error| Error::new(format!("the update can't be signed: {error}")))?;
    Ok(STANDARD.encode(signature.into_string()))
}

/// Checks that `signature` (a signature file's contents) signs `data` with
/// `public_key` (`packaging/updater.pub`'s contents) for `version`, as the
/// app's updater does before installing.
pub fn verify(public_key: &str, data: impl Read + Seek, signature: &str, version: &str) -> Result<(), Error> {
    let public_text =
        base64_text(public_key).map_err(|reason| Error::new(format!("the public key is not {reason}")))?;
    let public = PublicKeyBox::from_string(&public_text)
        .and_then(PublicKeyBox::into_public_key)
        .map_err(|_| Error::new("the public key is not a minisign public key"))?;
    let signature_text =
        base64_text(signature).map_err(|reason| Error::new(format!("the signature is not {reason}")))?;
    let signature = SignatureBox::from_string(&signature_text)
        .map_err(|_| Error::new("the signature is not a minisign signature"))?;
    minisign::verify(&public, &signature, data, true, false, true)
        .map_err(|_| Error::new("the signature doesn't match the file and the public key"))?;
    let comment = signature.trusted_comment().map_err(|_| Error::new("the signature has no trusted comment"))?;
    match signed_version(&comment) {
        Some(signed) if signed == version => Ok(()),
        Some(signed) => Err(Error::new(format!("the signature is for version {signed}, not {version}"))),
        None => Err(Error::new("the signature doesn't name the version it was made for")),
    }
}

/// The `version:` field of a trusted comment.
pub fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment.split('\t').find_map(|field| field.strip_prefix("version:"))
}

/// `encoded`'s text: base64 of UTF-8, surrounding whitespace ignored.
fn base64_text(encoded: &str) -> Result<String, &'static str> {
    let bytes = STANDARD.decode(encoded.trim()).map_err(|_| "base64")?;
    String::from_utf8(bytes).map_err(|_| "base64 of text")
}

/// `data` read whole, for callers that sign or verify bytes in memory.
pub fn reader(data: &[u8]) -> Cursor<&[u8]> {
    Cursor::new(data)
}

#[cfg(test)]
mod tests {
    use minisign::KeyPair;

    use super::*;

    /// A throwaway key made for the test, written as `tauri signer generate` writes one.
    fn throwaway_key(password: &str) -> String {
        let pair = KeyPair::generate_encrypted_keypair(Some(password.to_owned())).unwrap();
        STANDARD.encode(pair.sk.to_box(None).unwrap().into_string())
    }

    #[test]
    fn decodes_a_password_protected_key_and_refuses_a_wrong_password() {
        let encoded = throwaway_key("throwaway");
        assert!(UpdaterKey::decode(&encoded, "throwaway").is_ok());
        let wrong = UpdaterKey::decode(&encoded, "other").err().unwrap();
        assert!(wrong.to_string().contains("password"), "{wrong}");
        assert!(!wrong.to_string().contains("other"), "the password never shows: {wrong}");
    }

    #[test]
    fn decodes_a_key_made_without_a_password() {
        let encoded = throwaway_key("");
        assert!(UpdaterKey::decode(&encoded, "").is_ok());
    }

    #[test]
    fn refuses_what_is_not_a_key() {
        assert!(UpdaterKey::decode("not base64!", "").is_err());
        assert!(UpdaterKey::decode(&STANDARD.encode("untrusted comment: x\nnope\n"), "").is_err());
    }

    #[test]
    fn writes_the_trusted_comment_tauri_signer_wrote() {
        assert_eq!(
            trusted_comment(1_795_098_801, "DBForge-1.2.3.app.tar.gz", "1.2.3"),
            "timestamp:1795098801\tfile:DBForge-1.2.3.app.tar.gz\tversion:1.2.3"
        );
        assert_eq!(signed_version("timestamp:1\tfile:a\tversion:1.2.3"), Some("1.2.3"));
        assert_eq!(signed_version("timestamp:1\tfile:a"), None);
    }

    /// The format the updaters verify: base64 of a minisign signature file,
    /// prehashed, which minisign-verify (the Tauri updater's and the app's
    /// verifier) accepts with the base64-decoded public key.
    #[test]
    fn signs_in_the_format_the_updaters_verify() {
        let key = UpdaterKey::decode(&throwaway_key("throwaway"), "throwaway").unwrap();
        let data = b"an update".as_slice();
        let signature = sign(&key, reader(data), "DBForge-1.2.3-amd64.AppImage", "1.2.3", 1_795_098_801).unwrap();
        assert!(!signature.contains('\n'), "one base64 line, as tauri signer wrote");

        let text = base64_text(&signature).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], format!("untrusted comment: {UNTRUSTED_COMMENT}"));
        assert!(lines[1].starts_with("RU"), "a prehashed (ED) signature: {}", lines[1]);
        assert_eq!(lines[2], "trusted comment: timestamp:1795098801\tfile:DBForge-1.2.3-amd64.AppImage\tversion:1.2.3");

        let public = minisign_verify::PublicKey::decode(&base64_text(&key.public_key().unwrap()).unwrap()).unwrap();
        let decoded = minisign_verify::Signature::decode(&text).unwrap();
        public.verify(data, &decoded, true).unwrap();
        assert!(public.verify(b"another update", &decoded, true).is_err());
    }

    #[test]
    fn verifies_the_file_the_key_and_the_version() {
        let key = UpdaterKey::decode(&throwaway_key(""), "").unwrap();
        let public = key.public_key().unwrap();
        let signature = sign(&key, reader(b"setup"), "DBForge-1.2.3-x64-setup.exe", "1.2.3", 1).unwrap();
        assert_eq!(verify(&public, reader(b"setup"), &signature, "1.2.3"), Ok(()));
        assert!(verify(&public, reader(b"tampered"), &signature, "1.2.3").is_err());
        let other = UpdaterKey::decode(&throwaway_key(""), "").unwrap().public_key().unwrap();
        assert!(verify(&other, reader(b"setup"), &signature, "1.2.3").is_err());
        let mismatch = verify(&public, reader(b"setup"), &signature, "1.2.4").unwrap_err();
        assert!(mismatch.to_string().contains("1.2.3"), "{mismatch}");
    }

    #[test]
    fn refuses_to_sign_for_no_version() {
        let key = UpdaterKey::decode(&throwaway_key(""), "").unwrap();
        assert!(sign(&key, reader(b"x"), "x", " ", 1).is_err());
    }
}
