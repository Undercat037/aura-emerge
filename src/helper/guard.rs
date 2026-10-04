//! Helper process guard: runs first, before anything is read or parsed.
//! Std + libc only, no `crate::` imports.
//!
//! Fails closed: any doubt is an error and the helper must exit.
//! The channel arrives on stdin (requests) and stdout (responses),
//! because sudo closes every fd >= 3. `harden()` moves it to fd 3/4,
//! points stdin at /dev/null and stdout at stderr, so a stray print or
//! child process can never touch the protocol stream.

use std::fmt;
use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Only this binary may run as the helper.
pub(crate) const EXPECTED_EXE: &str = "/usr/bin/emerge";
pub(crate) const FD_IN: i32 = 3;
pub(crate) const FD_OUT: i32 = 4;

const SAFE_PATH: &str = "/usr/bin";
/// prctl() is variadic and wants full-width args.
const ZERO: libc::c_ulong = 0;

#[derive(Debug)]
pub(crate) struct GuardError(String);

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(msg: impl Into<String>) -> GuardError {
    GuardError(msg.into())
}

fn os_err(what: &str) -> GuardError {
    err(format!("{}: {}", what, io::Error::last_os_error()))
}

/// Call first thing in the helper path of `main()`.
pub(crate) fn harden() -> Result<(), GuardError> {
    // SAFETY: plain getters/setters below, no pointers except where noted.
    let ppid = unsafe { libc::getppid() };

    check_ids(unsafe { libc::getuid() }, unsafe { libc::geteuid() })?;
    check_exe_in(Path::new("/proc/self/exe"), Path::new(EXPECTED_EXE), 0)?;

    ensure_std_fds()?;
    // A terminal here means someone ran the helper by hand.
    if !is_channel(0) || !is_channel(1) {
        return Err(err("stdin/stdout must be pipes/sockets"));
    }
    clean_env();

    unsafe { libc::umask(0o077) };
    std::env::set_current_dir("/").map_err(|e| err(format!("chdir /: {}", e)))?;
    // No core dumps / ptrace-by-attach of a root process.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, ZERO, ZERO, ZERO, ZERO) } != 0 {
        return Err(os_err("prctl(DUMPABLE)"));
    }
    let no_core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &no_core) } != 0 {
        return Err(os_err("setrlimit(CORE)"));
    }

    close_fds_from(3);
    move_channel()?;

    // Belt and braces: the real "client died" signal is EOF on fd 3.
    let sigterm = libc::SIGTERM as libc::c_ulong;
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, sigterm, ZERO, ZERO, ZERO) } != 0 {
        return Err(os_err("prctl(PDEATHSIG)"));
    }
    // Parent may have died before the signal was armed.
    if unsafe { libc::getppid() } != ppid {
        return Err(err("parent exited during startup"));
    }
    Ok(())
}

/// stdin/stdout -> fd 3/4 (close-on-exec), then stdin = /dev/null and
/// stdout = stderr. Call after everything >= 3 is closed.
fn move_channel() -> Result<(), GuardError> {
    // SAFETY: plain fd syscalls on fds we own; valid C string.
    unsafe {
        if libc::dup2(0, FD_IN) != FD_IN || libc::dup2(1, FD_OUT) != FD_OUT {
            return Err(os_err("dup2(channel)"));
        }
        for fd in [FD_IN, FD_OUT] {
            if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) != 0 {
                return Err(os_err("fcntl(CLOEXEC)"));
            }
        }
        let null = std::ffi::CString::new("/dev/null").unwrap();
        let nfd = libc::open(null.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
        if nfd < 0 || libc::dup2(nfd, 0) != 0 {
            return Err(os_err("stdin -> /dev/null"));
        }
        libc::close(nfd);
        if libc::dup2(2, 1) != 1 {
            return Err(os_err("stdout -> stderr"));
        }
    }
    Ok(())
}

/// Real and effective uid must both be root (sudo/doas/pkexec do that).
fn check_ids(ruid: u32, euid: u32) -> Result<(), GuardError> {
    if ruid != 0 || euid != 0 {
        return Err(err("must run as root"));
    }
    Ok(())
}

/// `link` (/proc/self/exe) must point at `expected`, and the binary
/// must be a regular file owned by `uid`, not group/world-writable.
/// A " (deleted)" exe (replaced mid-run) fails the path compare.
fn check_exe_in(link: &Path, expected: &Path, uid: u32) -> Result<(), GuardError> {
    let actual = std::fs::read_link(link).map_err(|e| err(format!("readlink exe: {}", e)))?;
    if actual != expected {
        return Err(err(format!(
            "unexpected binary {} (want {})",
            actual.display(),
            expected.display()
        )));
    }
    let md = File::open(link)
        .and_then(|f| f.metadata())
        .map_err(|e| err(format!("stat exe: {}", e)))?;
    if !md.is_file() || md.uid() != uid || md.mode() & 0o022 != 0 {
        return Err(err("binary not root-owned or is writable by others"));
    }
    Ok(())
}

/// If 0/1/2 are closed, later open() would reuse them. Plug with /dev/null.
fn ensure_std_fds() -> Result<(), GuardError> {
    for fd in 0..3 {
        // SAFETY: F_GETFD on an arbitrary fd is harmless.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
            let null = std::ffi::CString::new("/dev/null").unwrap();
            let got = unsafe { libc::open(null.as_ptr(), libc::O_RDWR) };
            if got != fd {
                return Err(err("cannot reopen std fds"));
            }
        }
    }
    Ok(())
}

/// Drops everything inherited; nothing from the user's env is trusted.
fn clean_env() {
    let keys: Vec<_> = std::env::vars_os().map(|(k, _)| k).collect();
    for k in keys {
        std::env::remove_var(k);
    }
    std::env::set_var("PATH", SAFE_PATH);
    std::env::set_var("LANG", "C");
}

/// Closes every fd >= `min`.
fn close_fds_from(min: libc::c_uint) {
    // SAFETY: plain syscall, no pointers.
    let r = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            min,
            libc::c_uint::MAX,
            0 as libc::c_uint,
        )
    };
    if r == 0 {
        return;
    }
    // Kernel without close_range: brute force.
    let open_max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let max: i32 = if open_max < 0 {
        1024
    } else {
        open_max.min(65536) as i32
    };
    for fd in (min as i32)..max {
        unsafe { libc::close(fd) };
    }
}

fn is_channel(fd: i32) -> bool {
    // SAFETY: zeroed stat is a valid out-buffer.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return false;
    }
    let kind = st.st_mode & libc::S_IFMT;
    kind == libc::S_IFIFO || kind == libc::S_IFSOCK
}

// `clean_env`, `umask`, `close_fds_from` etc. change process-wide state
// and would break parallel tests, so only the pure checks are tested.
#[cfg(test)]
mod tests {
    use super::*;

    fn euid() -> u32 {
        unsafe { libc::geteuid() }
    }

    #[test]
    fn ids_must_both_be_root() {
        assert!(check_ids(0, 0).is_ok());
        assert!(check_ids(1000, 0).is_err());
        assert!(check_ids(0, 1000).is_err());
        assert!(check_ids(1000, 1000).is_err());
    }

    #[test]
    fn exe_checks() {
        let link = Path::new("/proc/self/exe");
        let me = std::fs::read_link(link).unwrap();
        let mode = std::fs::metadata(&me).unwrap().mode();

        // right path: ok only if our test binary isn't group/world-writable
        let ok = check_exe_in(link, &me, euid()).is_ok();
        assert_eq!(ok, mode & 0o022 == 0);

        // wrong path / wrong owner always fail
        assert!(check_exe_in(link, Path::new(EXPECTED_EXE), euid()).is_err());
        assert!(check_exe_in(link, &me, euid() + 1).is_err());
        assert!(check_exe_in(Path::new("/nonexistent"), &me, euid()).is_err());
    }

    #[test]
    fn channel_detection() {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        assert!(is_channel(fds[0]) && is_channel(fds[1]));
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
        let f = File::open("/proc/self/exe").unwrap();
        assert!(!is_channel(std::os::fd::AsRawFd::as_raw_fd(&f)));
        assert!(!is_channel(-1));
    }
}
