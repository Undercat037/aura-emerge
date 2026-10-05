//! Root-private staging of user files for `-U`. Std + libc only,
//! no `crate::` imports.
//!
//! Copy, hash while copying, compare with the sha256 the client sent
//! (what passed the audit). libalpm only ever sees the root-owned copy,
//! so nothing done to the original after the audit matters (TOCTOU).

use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use super::sha256::{self, Sha256};
use super::validate;

/// Root-only parent of all stage dirs.
pub(crate) const STAGE_BASE: &str = "/var/cache/aura-emerge";
/// Sanity cap per file.
const MAX_FILE: u64 = 8 << 30;
const NAME_MAX: usize = 255;

fn bad(msg: String) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, msg)
}

fn denied(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, msg.to_string())
}

/// One private dir (0700) for this run; removed on drop.
pub(crate) struct Stage {
    dir: PathBuf,
    n: usize,
}

impl Stage {
    pub(crate) fn create() -> io::Result<Self> {
        Self::create_in(Path::new(STAGE_BASE), 0)
    }

    /// `base` must be a real dir owned by `uid` with no group/other access.
    pub(crate) fn create_in(base: &Path, uid: u32) -> io::Result<Self> {
        match DirBuilder::new().mode(0o700).create(base) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let md = fs::symlink_metadata(base)?;
        if !md.is_dir() {
            return Err(denied("stage base is not a plain directory"));
        }
        if md.uid() != uid || md.mode() & 0o077 != 0 {
            return Err(denied("stage base must be private and ours"));
        }
        let mut tpl = format!(
            "{}/stage.XXXXXX\0",
            base.to_str().ok_or_else(|| bad("non-utf8 path".into()))?
        )
        .into_bytes();
        // SAFETY: tpl is NUL-terminated and writable; mkdtemp makes a 0700 dir.
        let p = unsafe { libc::mkdtemp(tpl.as_mut_ptr() as *mut libc::c_char) };
        if p.is_null() {
            return Err(io::Error::last_os_error());
        }
        tpl.pop();
        Ok(Stage {
            dir: PathBuf::from(OsString::from_vec(tpl)),
            n: 0,
        })
    }

    /// Copies `src` in and checks it against `want_sha`; returns the copy.
    pub(crate) fn copy(&mut self, src: &Path, want_sha: &str) -> io::Result<PathBuf> {
        let base = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| bad("bad file name".into()))?;
        if base.len() + 8 > NAME_MAX {
            return Err(bad("file name too long".into()));
        }
        self.n += 1;
        let dest = self.dir.join(format!("{}-{}", self.n, base));
        let got = copy_file(src, &dest)?;
        if got != want_sha {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("sha256 mismatch: {}", base),
            ));
        }
        // A signature checks itself against the keyring: copied as is,
        // never trusted, never required.
        let (ssig, dsig) = (with_sig(src), with_sig(&dest));
        if copy_file(&ssig, &dsig).is_err() {
            let _ = fs::remove_file(&dsig);
        }
        Ok(dest)
    }

    /// All `-U` specs of one request, in order.
    pub(crate) fn copy_specs(&mut self, specs: &[String]) -> io::Result<Vec<PathBuf>> {
        let parsed = validate::file_specs(specs).map_err(|r| bad(format!("rejected: {:?}", r)))?;
        parsed
            .iter()
            .map(|f| self.copy(Path::new(f.path), f.sha256))
            .collect()
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn with_sig(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".sig");
    PathBuf::from(s)
}

/// No symlink, regular file only, never overwrites. Returns the sha256
/// of the bytes actually written.
fn copy_file(src: &Path, dest: &Path) -> io::Result<String> {
    let mut inp = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(src)?;
    let md = inp.metadata()?;
    if !md.is_file() {
        return Err(bad("not a regular file".into()));
    }
    if md.len() > MAX_FILE {
        return Err(bad("file too big".into()));
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dest)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let n = match inp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        total += n as u64;
        if total > MAX_FILE {
            return Err(bad("file too big".into()));
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    Ok(sha256::hex(&h.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("ae-stage-{}-{}", std::process::id(), name));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self, name: &str, body: &str) -> PathBuf {
            let p = self.0.join(name);
            fs::write(&p, body).unwrap();
            p
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn me() -> u32 {
        // SAFETY: plain getter.
        unsafe { libc::geteuid() }
    }

    fn sha(s: &str) -> String {
        let mut h = Sha256::new();
        h.update(s.as_bytes());
        sha256::hex(&h.finish())
    }

    #[test]
    fn copies_verifies_and_is_private() {
        let t = Tmp::new("ok");
        let src = t.file("a.pkg", "hello");
        let mut st = Stage::create_in(&t.0.join("base"), me()).unwrap();
        let dest = st.copy(&src, &sha("hello")).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello");
        assert_eq!(fs::metadata(&dest).unwrap().mode() & 0o777, 0o600);
        assert!(dest.starts_with(&st.dir));
        assert_ne!(dest, src);
    }

    #[test]
    fn wrong_sha_is_refused() {
        let t = Tmp::new("sha");
        let src = t.file("a.pkg", "hello");
        let mut st = Stage::create_in(&t.0.join("base"), me()).unwrap();
        let err = st.copy(&src, &sha("something else")).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().starts_with("sha256 mismatch"));
    }

    #[test]
    fn symlink_and_non_regular_are_refused() {
        let t = Tmp::new("kind");
        let src = t.file("a.pkg", "hello");
        let link = t.0.join("link.pkg");
        symlink(&src, &link).unwrap();
        let mut st = Stage::create_in(&t.0.join("base"), me()).unwrap();
        assert!(st.copy(&link, &sha("hello")).is_err());
        assert!(st.copy(&t.0, &sha("hello")).is_err()); // a directory
    }

    #[test]
    fn signature_is_copied_when_present() {
        let t = Tmp::new("sig");
        let src = t.file("a.pkg", "hello");
        t.file("a.pkg.sig", "SIG");
        let mut st = Stage::create_in(&t.0.join("base"), me()).unwrap();
        let dest = st.copy(&src, &sha("hello")).unwrap();
        assert_eq!(fs::read_to_string(with_sig(&dest)).unwrap(), "SIG");
        // and absent is fine
        let src2 = t.file("b.pkg", "x");
        let d2 = st.copy(&src2, &sha("x")).unwrap();
        assert!(!with_sig(&d2).exists());
    }

    #[test]
    fn drop_removes_the_stage_dir() {
        let t = Tmp::new("drop");
        let src = t.file("a.pkg", "hello");
        let base = t.0.join("base");
        {
            let mut st = Stage::create_in(&base, me()).unwrap();
            st.copy(&src, &sha("hello")).unwrap();
            assert_eq!(fs::read_dir(&base).unwrap().count(), 1);
        }
        assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
    }

    #[test]
    fn base_must_be_private_and_ours() {
        let t = Tmp::new("base");
        let loose = t.0.join("loose");
        fs::create_dir(&loose).unwrap();
        fs::set_permissions(&loose, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Stage::create_in(&loose, me()).is_err());

        let mine = t.0.join("mine");
        assert!(Stage::create_in(&mine, me() + 1).is_err()); // wrong owner
        let link = t.0.join("link");
        symlink(&mine, &link).unwrap();
        assert!(Stage::create_in(&link, me()).is_err());
    }

    #[test]
    fn copy_specs_validates_first() {
        let t = Tmp::new("specs");
        let src = t.file("a.pkg", "hello");
        let mut st = Stage::create_in(&t.0.join("base"), me()).unwrap();
        let err = st.copy_specs(&["nonsense".to_string()]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let good = validate::spec(&sha("hello"), src.to_str().unwrap()).unwrap();
        assert_eq!(st.copy_specs(&[good]).unwrap().len(), 1);
    }
}
