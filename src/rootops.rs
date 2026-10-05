//! Crate-side door to the root helper: one lazy connection per process
//! (a single sudo prompt), plus "pinning" of files before they go to
//! root.
//!
//! Pinning: hash a file *before* the audit, re-hash *after*; the helper
//! hashes its own root-owned copy and refuses on mismatch. So the bytes
//! that get installed are the bytes that were audited.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::helper::client::{Client, ClientError};
use crate::helper::sha256;
use crate::helper::validate::{self, FileOpts};

thread_local! {
    static HELPER: RefCell<Option<Client>> = const { RefCell::new(None) };
}

/// Runs `f` on the shared helper, starting it on first use. A dead
/// connection is dropped, so the next call starts a fresh helper.
pub(crate) fn with_helper<T>(
    f: impl FnOnce(&mut Client) -> Result<T, ClientError>,
) -> Result<T, ClientError> {
    HELPER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Client::start()?);
        }
        let r = match slot.as_mut() {
            Some(c) => f(c),
            None => Err(ClientError::Broken),
        };
        let dead = match &r {
            Err(ClientError::Broken) => true,
            Err(ClientError::Proto(e)) => e.is_fatal(),
            _ => false,
        };
        if dead {
            *slot = None;
        }
        r
    })
}

/// A file as it was when it got audited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pinned {
    pub(crate) path: String,
    pub(crate) sha256: String,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, msg.to_string())
}

/// Never follows a final symlink; regular files only.
fn hash_nofollow(path: &Path) -> io::Result<String> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(invalid("not a regular file"));
    }
    sha256::of_reader(&mut f)
}

/// Absolute path (parent canonicalized, file name kept) + sha256.
pub(crate) fn pin(files: &[String]) -> io::Result<Vec<Pinned>> {
    files
        .iter()
        .map(|t| {
            let p = Path::new(t);
            let name = p.file_name().ok_or_else(|| invalid("no file name"))?;
            let dir = match p.parent() {
                Some(d) if !d.as_os_str().is_empty() => d,
                _ => Path::new("."),
            };
            let abs = fs::canonicalize(dir)?.join(name);
            let sha256 = hash_nofollow(&abs)?;
            let path = abs
                .to_str()
                .ok_or_else(|| invalid("non-utf8 path"))?
                .to_string();
            Ok(Pinned { path, sha256 })
        })
        .collect()
}

/// True when every file still hashes to what was pinned.
pub(crate) fn unchanged(pins: &[Pinned]) -> bool {
    pins.iter()
        .all(|p| hash_nofollow(Path::new(&p.path)).map_or(false, |h| h == p.sha256))
}

pub(crate) fn specs(pins: &[Pinned]) -> Result<Vec<String>, String> {
    pins.iter()
        .map(|p| validate::spec(&p.sha256, &p.path).map_err(|r| format!("{}: {:?}", p.path, r)))
        .collect()
}

/// `pacman -U` through the root helper.
pub(crate) fn install_files(pins: &[Pinned], opts: FileOpts) -> Result<(), String> {
    let lines = specs(pins)?;
    with_helper(|c| c.install_files(opts, &lines)).map_err(|e| e.to_string())
}

/// `pacman -S` through the root helper (libalpm transaction).
pub(crate) fn install(names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    with_helper(|c| c.install(names)).map_err(|e| e.to_string())
}

/// Refresh sync dbs (`-Sy` / `-Syy` when force).
pub(crate) fn sync(force: bool) -> Result<(), String> {
    with_helper(|c| c.sync(force)).map_err(|e| e.to_string())
}

/// Official sysupgrade (`-Su`); `ignore` is holdback list.
pub(crate) fn sysupgrade(ignore: &[String]) -> Result<(), String> {
    with_helper(|c| c.sysupgrade(ignore)).map_err(|e| e.to_string())
}

/// Remove installed packages through the helper.
pub(crate) fn remove(
    mode: crate::helper::validate::RemoveMode,
    names: &[String],
) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    with_helper(|c| c.remove(mode, names)).map_err(|e| e.to_string())
}

/// `pacman -D --asexplicit` / `--asdeps`.
pub(crate) fn set_reason(explicit: bool, names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Ok(());
    }
    with_helper(|c| c.set_reason(explicit, names)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("ae-rootops-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn pin_hashes_and_unchanged_notices_edits() {
        let d = tmp("pin");
        let f = d.join("a.pkg.tar.zst");
        fs::write(&f, "abc").unwrap();
        let pins = pin(&[f.to_str().unwrap().to_string()]).unwrap();
        assert_eq!(pins[0].sha256, ABC);
        assert!(Path::new(&pins[0].path).is_absolute());
        assert!(unchanged(&pins));
        fs::write(&f, "abd").unwrap();
        assert!(!unchanged(&pins));
        fs::remove_file(&f).unwrap();
        assert!(!unchanged(&pins));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn pin_refuses_symlink_dir_and_missing() {
        let d = tmp("refuse");
        let f = d.join("a.pkg");
        fs::write(&f, "abc").unwrap();
        let l = d.join("l.pkg");
        std::os::unix::fs::symlink(&f, &l).unwrap();
        assert!(pin(&[l.to_str().unwrap().to_string()]).is_err());
        assert!(pin(&[d.to_str().unwrap().to_string()]).is_err());
        assert!(pin(&[d.join("nope.pkg").to_str().unwrap().to_string()]).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn specs_are_valid_wire_lines() {
        let d = tmp("specs");
        let f = d.join("a b.pkg");
        fs::write(&f, "abc").unwrap();
        let pins = pin(&[f.to_str().unwrap().to_string()]).unwrap();
        let lines = specs(&pins).unwrap();
        let parsed = validate::file_spec(&lines[0]).unwrap();
        assert_eq!(parsed.sha256, ABC);
        assert!(parsed.path.ends_with("/a b.pkg"));
        let _ = fs::remove_dir_all(&d);
    }
}
