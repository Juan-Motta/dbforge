//! `dbforge-release`: the release scripts' signing tool.
//!
//! ```text
//! dbforge-release sign <file> <version>
//! dbforge-release verify <file> <version> [<public-key-file>]
//! dbforge-release public-key
//! ```
//!
//! `sign` writes `<file>.sig` for the updater. The key comes from the
//! environment, never the command line: `UPDATER_PRIVATE_KEY` holds its text
//! (the base64 `tauri signer generate` wrote) or the path of its file, and
//! `UPDATER_PRIVATE_KEY_PASSWORD` its password (unset or empty: none).
//! `verify` checks `<file>.sig` against the app's public key
//! (`packaging/updater.pub` unless given) and the version, as the app does.
//! `public-key` prints the key's public half, to compare with the app's.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use dbforge_release::signing::{self, UpdaterKey};

const USAGE: &str = "usage:
  dbforge-release sign <file> <version>
  dbforge-release verify <file> <version> [<public-key-file>]
  dbforge-release public-key";

const KEY_VARIABLE: &str = "UPDATER_PRIVATE_KEY";
const PASSWORD_VARIABLE: &str = "UPDATER_PRIVATE_KEY_PASSWORD";
const PUBLIC_KEY_FILE: &str = "packaging/updater.pub";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["sign", file, version] => sign(Path::new(file), version),
        ["verify", file, version] => verify(Path::new(file), version, Path::new(PUBLIC_KEY_FILE)),
        ["verify", file, version, public_key] => verify(Path::new(file), version, Path::new(public_key)),
        ["public-key"] => public_key(),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dbforge-release: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The key from the environment: its text, or the path of its file.
fn load_key() -> Result<UpdaterKey, String> {
    let value = std::env::var(KEY_VARIABLE).map_err(|_| format!("{KEY_VARIABLE} is not set"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{KEY_VARIABLE} is empty"));
    }
    // A key's text is base64, which holds no path separator; a path does.
    let looks_like_path = value.contains('/') && Path::new(value).is_file();
    let text = if looks_like_path {
        std::fs::read_to_string(value).map_err(|error| format!("the key file can't be read: {error}"))?
    } else {
        value.to_owned()
    };
    let password = std::env::var(PASSWORD_VARIABLE).unwrap_or_default();
    UpdaterKey::decode(&text, &password).map_err(|error| error.to_string())
}

fn signature_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

fn sign(file: &Path, version: &str) -> Result<(), String> {
    let key = load_key()?;
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no file name", file.display()))?;
    let data = File::open(file).map_err(|error| format!("{} can't be read: {error}", file.display()))?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_secs();
    let signature =
        signing::sign(&key, BufReader::new(data), name, version, timestamp).map_err(|error| error.to_string())?;
    let target = signature_path(file);
    std::fs::write(&target, signature).map_err(|error| format!("{} can't be written: {error}", target.display()))?;
    eprintln!("Signed {} for {version}", file.display());
    Ok(())
}

fn verify(file: &Path, version: &str, public_key_file: &Path) -> Result<(), String> {
    let public_key = std::fs::read_to_string(public_key_file)
        .map_err(|error| format!("{} can't be read: {error}", public_key_file.display()))?;
    let target = signature_path(file);
    let signature =
        std::fs::read_to_string(&target).map_err(|error| format!("{} can't be read: {error}", target.display()))?;
    let data = File::open(file).map_err(|error| format!("{} can't be read: {error}", file.display()))?;
    signing::verify(&public_key, BufReader::new(data), &signature, version)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    eprintln!("Verified {} for {version}", file.display());
    Ok(())
}

fn public_key() -> Result<(), String> {
    println!("{}", load_key()?.public_key().map_err(|error| error.to_string())?);
    Ok(())
}
