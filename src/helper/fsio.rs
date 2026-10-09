//! Root-side file writes. Std + libc only, no `crate::` imports.
//!
//! Everything goes through a directory fd: the dir is opened with

use std::ffi::CString;
use std::fs::File;
use std::io::{self, ErrorKind, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

use super::validate::{self, Reject, Target, MAX_BATCH};

/// New files are 0644 whatever the umask (client reads world/log).
const FILE_MODE: libc::mode_t = 0o644;
/// Files and dirs must be owned by root in production.
const ROOT: u32 = 0;

fn bad(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, msg.to_string())
}

fn denied(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, msg.to_string())
}

fn rejected(r: Reject) -> io::Error {
    bad(&format!("rejected: {:?}", r))
}

fn cstr(s: &str) -> io::Result<CString> {
    CString::new(s).map_err(|_| bad("NUL in path"))
}

fn fstat(fd: i32) -> io::Result<libc::stat> {
    // SAFETY: zeroed stat is a valid out-buffer for fstat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st)
}

fn split(path: &Path) -> io::Result<(&Path, &str)> {
    let dir = path.parent().ok_or_else(|| bad("no parent dir"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| bad("bad file name"))?;
    Ok((dir, name))
}

/// Opens the parent dir; must be ours and not writable by others.
fn open_dir(dir: &Path, uid: u32) -> io::Result<OwnedFd> {
    let c = cstr(dir.to_str().ok_or_else(|| bad("non-utf8 path"))?)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: valid C string; fd is wrapped right away.
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let st = fstat(fd.as_raw_fd())?;
    if st.st_uid != uid || st.st_mode & (libc::S_IWGRP | libc::S_IWOTH) != 0 {
        return Err(denied("unsafe directory"));
    }
    Ok(fd)
}

/// `openat` relative to `dir`, never following a final symlink.
fn openat_file(dir: &OwnedFd, name: &str, flags: i32, mode: libc::mode_t) -> io::Result<File> {
    let c = cstr(name)?;
    let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: valid dirfd and C string; fd is wrapped right away.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags, mode as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn fchmod(f: &File) -> io::Result<()> {
    if unsafe { libc::fchmod(f.as_raw_fd(), FILE_MODE) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn unlink(dir: &OwnedFd, name: &str) {
    if let Ok(c) = cstr(name) {
        unsafe { libc::unlinkat(dir.as_raw_fd(), c.as_ptr(), 0) };
    }
}

/// Regular file, ours, single link, not group/world-writable.
fn check_regular(f: &File, uid: u32) -> io::Result<()> {
    let st = fstat(f.as_raw_fd())?;
    if st.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(denied("not a regular file"));
    }
    if st.st_uid != uid {
        return Err(denied("wrong owner"));
    }
    if st.st_nlink != 1 {
        return Err(denied("hardlinked file"));
    }
    if st.st_mode & (libc::S_IWGRP | libc::S_IWOTH) != 0 {
        return Err(denied("group/world-writable file"));
    }
    Ok(())
}

/// Existing file, or a new one with exactly `FILE_MODE`.
fn open_or_create(dir: &OwnedFd, name: &str, flags: i32) -> io::Result<File> {
    match openat_file(dir, name, flags, 0) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let f = openat_file(dir, name, flags | libc::O_CREAT | libc::O_EXCL, FILE_MODE)?;
            fchmod(&f)?;
            Ok(f)
        }
        other => other,
    }
}

fn append_in(path: &Path, line: &str, uid: u32) -> io::Result<()> {
    validate::line(line).map_err(rejected)?;
    let (dir, name) = split(path)?;
    let d = open_dir(dir, uid)?;
    let mut f = open_or_create(&d, name, libc::O_WRONLY | libc::O_APPEND)?;
    check_regular(&f, uid)?;
    // One write(): O_APPEND keeps concurrent lines whole.
    f.write_all(format!("{}\n", line).as_bytes())?;
    f.sync_data()
}

fn replace_in(path: &Path, lines: &[String], uid: u32) -> io::Result<()> {
    if lines.len() > MAX_BATCH {
        return Err(rejected(Reject::TooMany));
    }
    let mut body = String::new();
    for l in lines {
        body.push_str(validate::line(l).map_err(rejected)?);
        body.push('\n');
    }
    let (dir, name) = split(path)?;
    let d = open_dir(dir, uid)?;

    // Refuse to replace anything odd (symlink, hardlink, foreign owner).
    match openat_file(&d, name, libc::O_RDONLY, 0) {
        Ok(f) => check_regular(&f, uid)?,
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    // Temp file next to the target, so rename() stays atomic.
    let tmp = format!(".{}.ae-tmp", name);
    unlink(&d, &tmp);
    let res = (|| -> io::Result<()> {
        let mut f = openat_file(
            &d,
            &tmp,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            FILE_MODE,
        )?;
        fchmod(&f)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        let (from, to) = (cstr(&tmp)?, cstr(name)?);
        // SAFETY: valid dirfd and C strings.
        if unsafe { libc::renameat(d.as_raw_fd(), from.as_ptr(), d.as_raw_fd(), to.as_ptr()) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if res.is_err() {
        unlink(&d, &tmp);
        return res;
    }
    // Make the rename itself durable.
    if unsafe { libc::fsync(d.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Appends one line to a whitelisted file.
pub(crate) fn append(target: Target, line: &str) -> io::Result<()> {
    append_in(Path::new(target.path()), line, ROOT)
}

/// Rewrites a whitelisted file atomically. Not for the log (append-only).
pub(crate) fn replace(target: Target, lines: &[String]) -> io::Result<()> {
    if target == Target::EmergeLog {
        return Err(bad("log is append-only"));
    }
    replace_in(Path::new(target.path()), lines, ROOT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn euid() -> u32 {
        unsafe { libc::geteuid() }
    }

    /// Scratch dir, removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "ae-fsio-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::DirBuilder::new().mode(0o755).create(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            Tmp(p)
        }

        fn at(&self, n: &str) -> PathBuf {
            self.0.join(n)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn append_creates_0644_and_appends() {
        let t = Tmp::new();
        let p = t.at("emerge.log");
        append_in(&p, "one", euid()).unwrap();
        append_in(&p, "two", euid()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "one\ntwo\n");
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn append_refuses_symlink() {
        let t = Tmp::new();
        let real = t.at("real");
        fs::write(&real, "keep\n").unwrap();
        symlink(&real, t.at("world")).unwrap();
        assert!(append_in(&t.at("world"), "x", euid()).is_err());
        // dangling link must not get created through either
        symlink(t.at("nowhere"), t.at("dangling")).unwrap();
        assert!(append_in(&t.at("dangling"), "x", euid()).is_err());
        assert!(!t.at("nowhere").exists());
        assert_eq!(fs::read_to_string(&real).unwrap(), "keep\n");
    }

    #[test]
    fn append_refuses_hardlink_and_loose_perms() {
        let t = Tmp::new();
        let p = t.at("world");
        append_in(&p, "a", euid()).unwrap();
        fs::hard_link(&p, t.at("other")).unwrap();
        assert!(append_in(&p, "b", euid()).is_err());
        fs::remove_file(t.at("other")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(append_in(&p, "b", euid()).is_err());
    }

    #[test]
    fn refuses_bad_dir_and_wrong_owner() {
        let t = Tmp::new();
        let p = t.at("world");
        // pretend root must own it: we don't, so it's refused
        assert!(append_in(&p, "x", euid() + 1).is_err());
        fs::set_permissions(&t.0, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(append_in(&p, "x", euid()).is_err());
        assert!(!p.exists());
    }

    #[test]
    fn append_validates_line() {
        let t = Tmp::new();
        let p = t.at("world");
        assert!(append_in(&p, "a\nb", euid()).is_err());
        assert!(append_in(&p, "a\x1b[2J", euid()).is_err());
        assert!(append_in(&p, "", euid()).is_err());
        assert!(!p.exists());
    }

    #[test]
    fn replace_is_atomic_and_leaves_no_tmp() {
        let t = Tmp::new();
        let p = t.at("world");
        fs::write(&p, "old\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
        // stale tmp from a crashed run must not matter
        fs::write(t.at(".world.ae-tmp"), "junk").unwrap();
        replace_in(&p, &["nano".into(), "extra/vim".into()], euid()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "nano\nextra/vim\n");
        assert!(!t.at(".world.ae-tmp").exists());
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o644
        );
        // empty list = empty file (last package removed)
        replace_in(&p, &[], euid()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "");
    }

    #[test]
    fn replace_refuses_symlink_and_bad_lines() {
        let t = Tmp::new();
        let real = t.at("real");
        fs::write(&real, "keep\n").unwrap();
        symlink(&real, t.at("world")).unwrap();
        assert!(replace_in(&t.at("world"), &["x".into()], euid()).is_err());
        assert_eq!(fs::read_to_string(&real).unwrap(), "keep\n");

        let p = t.at("w2");
        assert!(replace_in(&p, &["a\nb".into()], euid()).is_err());
        assert!(!p.exists());
        assert!(!t.at(".w2.ae-tmp").exists());
    }

    #[test]
    fn log_is_append_only() {
        assert!(replace(Target::EmergeLog, &["x".into()]).is_err());
    }
}
