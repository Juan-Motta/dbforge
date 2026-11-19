//! A Mac app update: `DBForge-<version>.app.tar.gz` holds `DBForge.app/…`.
//! As Tauri's updater, it is unpacked into a folder on the bundle's own
//! volume (the system temporary folder when it is there, else beside the
//! bundle), then exchanged with the running bundle in one step
//! (`renamex_np` with `RENAME_SWAP`), so the install path is never empty;
//! where the file system can't swap, two renames with the old bundle put
//! back on failure. A bundle whose folder the user can't write (an
//! administrator's /Applications) is swapped through `osascript … with
//! administrator privileges`, which asks for a password.

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;

/// Replaces `bundle` (`…/DBForge.app`) with the bundle in `archive`.
pub fn install(archive: &[u8], bundle: &Path) -> io::Result<()> {
    let root = staging_root(bundle)?;
    let backup_dir = tempfile::Builder::new().prefix("dbforge-current-app").tempdir_in(&root)?;
    let staged = tempfile::Builder::new().prefix("dbforge-updated-app").tempdir_in(&root)?;
    unpack(archive, staged.path())?;
    // The staged folder becomes the bundle: readable and runnable by every user, as an installed app is.
    fs::set_permissions(staged.path(), fs::Permissions::from_mode(0o755))?;

    let backup = backup_dir.path().join("current_app");
    let moved = match swap(bundle, staged.path()) {
        Err(error) if error.kind() != io::ErrorKind::PermissionDenied => replace(bundle, staged.path(), &backup),
        other => other,
    };
    match moved {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            privileged_swap(bundle, staged.path(), &backup)?;
        }
        Err(error) => return Err(error),
    }
    // A new modification time tells Launch Services the bundle changed.
    let _ = std::process::Command::new("touch").arg(bundle).status();
    // After a swap the previous bundle sits in `staged`, removed with it.
    Ok(())
}

/// A folder on `bundle`'s volume to stage in, so renames stay on one file system.
fn staging_root(bundle: &Path) -> io::Result<PathBuf> {
    let device = fs::symlink_metadata(bundle)?.dev();
    [Some(std::env::temp_dir()), bundle.parent().map(Path::to_path_buf)]
        .into_iter()
        .flatten()
        .find(|dir| dir.metadata().is_ok_and(|metadata| metadata.dev() == device))
        .ok_or_else(|| io::Error::other("no folder on the app's volume to unpack the update in"))
}

/// Unpacks `archive` into `into`, without its first folder (`DBForge.app/`).
/// Only plain files and folders are taken: an entry reaching outside (`..`,
/// an absolute path) or any link is refused, so nothing can be written
/// through a link to outside `into`. Modes keep only the permission bits a
/// bundle needs (runnable or not), never setuid, setgid or sticky.
pub fn unpack(archive: &[u8], into: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(GzDecoder::new(archive));
    let mut unpacked = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let refused = || io::Error::new(io::ErrorKind::InvalidData, format!("the update holds {}", path.display()));
        if path.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir)) {
            return Err(refused());
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(refused());
        }
        let mode = if kind.is_dir() || entry.header().mode()? & 0o111 != 0 { 0o755 } else { 0o644 };
        let inner: PathBuf = path.components().skip(1).collect();
        if inner.as_os_str().is_empty() {
            continue;
        }
        let target = into.join(&inner);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
        unpacked += 1;
    }
    if unpacked == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the update holds no app"));
    }
    Ok(())
}

/// Exchanges `target` and `staged` atomically (APFS); other file systems refuse.
#[cfg(target_os = "macos")]
fn swap(target: &Path, staged: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(staged.as_os_str().as_bytes())?;
    let to = CString::new(target.as_os_str().as_bytes())?;
    // SAFETY: both are NUL-terminated strings that outlive the call, which only reads them.
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(not(target_os = "macos"))]
fn swap(_target: &Path, _staged: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no atomic swap here"))
}

/// Moves `staged` to `target`, keeping `target`'s previous content in
/// `backup` until then, and putting it back when the move fails.
fn replace(target: &Path, staged: &Path, backup: &Path) -> io::Result<()> {
    fs::rename(target, backup)?;
    if let Err(error) = fs::rename(staged, target) {
        let _ = fs::rename(backup, target);
        return Err(error);
    }
    Ok(())
}

/// The swap (or the two moves) as an administrator, as Tauri's updater did.
#[cfg(target_os = "macos")]
fn privileged_swap(target: &Path, staged: &Path, backup: &Path) -> io::Result<()> {
    let script = format!(
        "do shell script {} with administrator privileges",
        applescript_string(&privileged_command(target, staged, backup))
    );
    let status = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "the administrator's permission wasn't given"))
    }
}

#[cfg(not(target_os = "macos"))]
fn privileged_swap(_target: &Path, _staged: &Path, _backup: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::PermissionDenied, "the app's folder can't be written"))
}

/// JavaScript for Automation calling `renamex_np` (`2`: `RENAME_SWAP`), for a
/// shell that has no command swapping two paths.
#[cfg(target_os = "macos")]
const SWAP_JXA: &str = r#"function run(argv) {
  ObjC.import("stdio");
  return $.renamex_np(argv[0], argv[1], 2) === 0 ? "swapped" : "failed";
}"#;

/// The privileged shell command: the swap, or where the volume can't swap,
/// the moves, the previous app deleted only once the new one is in place.
#[cfg(target_os = "macos")]
fn privileged_command(target: &Path, staged: &Path, backup: &Path) -> String {
    let swap = sh_quote(SWAP_JXA);
    let (target, staged, backup) = (sh_quote_path(target), sh_quote_path(staged), sh_quote_path(backup));
    format!(
        "case \"$(/usr/bin/osascript -l JavaScript -e {swap} {staged} {target} 2>/dev/null)\" in swapped) rm -rf {staged};; failed) mv -f {target} {backup} && {{ mv -f {staged} {target} || {{ mv -f {backup} {target}; exit 1; }}; }} && rm -rf {backup};; *) exit 1;; esac"
    )
}

#[cfg(target_os = "macos")]
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(target_os = "macos")]
fn sh_quote_path(path: &Path) -> String {
    sh_quote(&path.to_string_lossy())
}

#[cfg(target_os = "macos")]
fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    /// A `.app.tar.gz` as the release packs it (`tar -czf … -C <dir> DBForge.app`).
    fn archive(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (path, data, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// `tar::Builder` refuses to write `..`; an archive made elsewhere may hold it.
    fn archive_with_raw_path(path: &str) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_size(1);
        header.set_mode(0o644);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(b"x");
        bytes.resize(bytes.len() + 511, 0);
        bytes.extend_from_slice(&[0; 1024]);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn fake_bundle(dir: &Path, executable: &[u8]) -> PathBuf {
        let bundle = dir.join("DBForge.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS/DBForge"), executable).unwrap();
        fs::write(bundle.join("Contents/Info.plist"), b"old plist").unwrap();
        bundle
    }

    #[test]
    fn swaps_the_bundle_for_the_archive_s() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = fake_bundle(dir.path(), b"old");
        let update = archive(&[
            ("DBForge.app/Contents/MacOS/DBForge", b"new", 0o755),
            ("DBForge.app/Contents/Info.plist", b"new plist", 0o644),
        ]);
        install(&update, &bundle).unwrap();
        assert_eq!(fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"new");
        assert_eq!(fs::read(bundle.join("Contents/Info.plist")).unwrap(), b"new plist");
        let mode = fs::metadata(bundle.join("Contents/MacOS/DBForge")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the executable stays runnable");
        assert_eq!(fs::metadata(&bundle).unwrap().permissions().mode() & 0o777, 0o755);
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, ["DBForge.app"], "nothing staged is left beside it");
    }

    #[test]
    fn leaves_the_bundle_alone_when_the_archive_reaches_outside() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = fake_bundle(dir.path(), b"old");
        let error = install(&archive_with_raw_path("DBForge.app/../../escape"), &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"old");
        assert!(!dir.path().join("escape").exists());
    }

    /// An archive holding `DBForge.app/Contents/link -> <target>`, then `link/pwned`.
    fn archive_through_a_link(target: &Path) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_mode(0o777);
        builder.append_link(&mut link, "DBForge.app/Contents/link", target).unwrap();
        let mut file = tar::Header::new_gnu();
        file.set_size(1);
        file.set_mode(0o644);
        file.set_cksum();
        builder.append_data(&mut file, "DBForge.app/Contents/link/pwned", &b"x"[..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn refuses_links_so_nothing_is_written_outside_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let bundle = fake_bundle(dir.path(), b"old");
        let error = install(&archive_through_a_link(&outside), &bundle).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!outside.join("pwned").exists());
        assert_eq!(fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"old");
    }

    #[test]
    fn drops_special_mode_bits_but_keeps_the_executable_runnable() {
        let dir = tempfile::tempdir().unwrap();
        let update = archive(&[
            ("DBForge.app/Contents/MacOS/DBForge", b"new", 0o4777),
            ("DBForge.app/Contents/Info.plist", b"plist", 0o666),
        ]);
        unpack(&update, dir.path()).unwrap();
        let mode = |path: &str| fs::metadata(dir.path().join(path)).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode("Contents/MacOS/DBForge"), 0o755);
        assert_eq!(mode("Contents/Info.plist"), 0o644);
    }

    #[test]
    fn leaves_the_bundle_alone_when_the_archive_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = fake_bundle(dir.path(), b"old");
        assert!(install(b"not a tar.gz", &bundle).is_err());
        assert!(install(&archive(&[]), &bundle).is_err());
        assert_eq!(fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"old");
    }

    #[test]
    fn puts_the_previous_content_back_when_the_move_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = fake_bundle(dir.path(), b"old");
        let backup = dir.path().join("backup");
        let missing = dir.path().join("missing");
        assert!(replace(&target, &missing, &backup).is_err());
        assert_eq!(fs::read(target.join("Contents/MacOS/DBForge")).unwrap(), b"old");
    }
}
