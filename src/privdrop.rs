//! Started as real root (`sudo emerge ...`, `doas`, `pkexec`, `su -`, a
//! root shell)? Become the invoking user before anything else runs, so
//! the frontend (resolver, AUR/ABS builds, bwrap sandbox, caches under
//! `$HOME`) behaves exactly as if it had been started by that user.
//! Everything that needs root keeps going through the helper, which the
//! caller starts *before* the drop (no second privilege prompt).
//!
//! Std + libc only, no `crate::` imports.

use std::ffi::{CStr, CString, OsString};
use std::fmt;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Who we turn into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Account {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) name: String,
    pub(crate) home: PathBuf,
    pub(crate) shell: String,
}

#[derive(Debug)]
pub(crate) enum DropError {
    /// Nothing says who started us.
    NoInvoker,
    /// An account was named, but it cannot be used.
    BadAccount(String),
    Sys(&'static str, io::Error),
    /// The drop "worked" but root is still reachable. Never continue.
    StillRoot,
}

impl fmt::Display for DropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DropError::NoInvoker => f.write_str(
                "started as root and cannot tell which user to continue as \
                 (no AE_USER, SUDO_UID, DOAS_USER, PKEXEC_UID, no audit login uid). \
                 Run it through sudo/doas as your normal user, or set AE_USER=<name>",
            ),
            DropError::BadAccount(m) => write!(f, "cannot continue as that user: {}", m),
            DropError::Sys(what, e) => write!(f, "{}: {}", what, e),
            DropError::StillRoot => f.write_str("privilege drop failed: root is still reachable"),
        }
    }
}

/// A way of naming the invoking user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Who {
    Uid(u32),
    Name(String),
}

/// Audit login uid when nobody has logged in (the kernel's `(uid_t)-1`).
const LOGINUID_UNSET: u32 = u32::MAX;

fn parse_uid(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

/// A usable uid: not root, not "unset".
fn real_uid(u: u32) -> Option<Who> {
    (u != 0 && u != LOGINUID_UNSET).then_some(Who::Uid(u))
}

/// Invoking user, best source first. Pure: `env` and the `loginuid`
/// file contents are passed in.
///
/// 1. `AE_USER`: explicit, wins over everything (name or uid)
/// 2. `SUDO_UID`
/// 3. `DOAS_USER`
/// 4. `PKEXEC_UID`
/// 5. `/proc/self/loginuid`: set by PAM at login and kept across
///    `su`/`sudo -i`, so a plain root shell still finds its owner
///
/// A candidate that names root (or garbage) is skipped, not trusted:
/// the whole point is to *not* stay root.
pub(crate) fn who_from(
    env: &dyn Fn(&str) -> Option<String>,
    loginuid: Option<&str>,
) -> Option<Who> {
    let named = |v: String| -> Option<Who> {
        let v = v.trim().to_string();
        if v.is_empty() {
            return None;
        }
        match parse_uid(&v) {
            Some(u) => real_uid(u),
            None if v == "root" => None,
            None => Some(Who::Name(v)),
        }
    };
    let numeric = |v: String| parse_uid(&v).and_then(real_uid);

    env("AE_USER")
        .and_then(named)
        .or_else(|| env("SUDO_UID").and_then(numeric))
        .or_else(|| env("DOAS_USER").and_then(named))
        .or_else(|| env("PKEXEC_UID").and_then(numeric))
        .or_else(|| loginuid.and_then(parse_uid).and_then(real_uid))
}

/// getpwuid_r / getpwnam_r. `Ok(None)` = no such account.
fn passwd_lookup(who: &Who) -> io::Result<Option<Account>> {
    let cname = match who {
        Who::Name(n) => Some(
            CString::new(n.as_str())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in user name"))?,
        ),
        Who::Uid(_) => None,
    };
    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: zeroed passwd is a valid out-buffer; pointers stay valid
        // for the call; `buf` outlives every use of `pwd`'s strings below.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut res: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            match (who, &cname) {
                (Who::Uid(u), _) => libc::getpwuid_r(
                    *u,
                    &mut pwd,
                    buf.as_mut_ptr() as *mut libc::c_char,
                    buf.len(),
                    &mut res,
                ),
                (Who::Name(_), Some(c)) => libc::getpwnam_r(
                    c.as_ptr(),
                    &mut pwd,
                    buf.as_mut_ptr() as *mut libc::c_char,
                    buf.len(),
                    &mut res,
                ),
                (Who::Name(_), None) => unreachable!(),
            }
        };
        if rc == libc::ERANGE && buf.len() < (1 << 20) {
            let n = buf.len() * 2;
            buf.resize(n, 0);
            continue;
        }
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        if res.is_null() {
            return Ok(None);
        }
        // SAFETY: on success the pw_* strings point into `buf`.
        let text = |p: *const libc::c_char| -> String {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        return Ok(Some(Account {
            uid: pwd.pw_uid,
            gid: pwd.pw_gid,
            name: text(pwd.pw_name),
            home: PathBuf::from(text(pwd.pw_dir)),
            shell: text(pwd.pw_shell),
        }));
    }
}

/// Accepts an account only if continuing as it makes sense.
fn accept(acc: Account) -> Result<Account, DropError> {
    if acc.uid == 0 {
        return Err(DropError::BadAccount("it is root".into()));
    }
    if acc.name.is_empty() {
        return Err(DropError::BadAccount(format!(
            "uid {} has no name",
            acc.uid
        )));
    }
    if !acc.home.is_absolute() {
        return Err(DropError::BadAccount(format!(
            "{} has no usable home directory ({:?})",
            acc.name, acc.home
        )));
    }
    Ok(acc)
}

/// Looks at the real environment and decides who to become.
/// Changes nothing.
pub(crate) fn resolve() -> Result<Account, DropError> {
    let loginuid = std::fs::read_to_string("/proc/self/loginuid").ok();
    let who =
        who_from(&|k| std::env::var(k).ok(), loginuid.as_deref()).ok_or(DropError::NoInvoker)?;
    match passwd_lookup(&who).map_err(|e| DropError::Sys("passwd lookup", e))? {
        Some(acc) => accept(acc),
        None => Err(DropError::BadAccount(format!(
            "{:?} is not in the passwd database",
            who
        ))),
    }
}

// ── environment ─────────────────────────────────────────────────────────────

/// Variables that survive: terminal, locale, proxies, our own knobs.
const KEEP: &[&str] = &[
    "PATH",
    "TERM",
    "COLORTERM",
    "LANG",
    "LANGUAGE",
    "TZ",
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "EDITOR",
    "VISUAL",
    "PAGER",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
];
const KEEP_PREFIX: &[&str] = &["LC_", "AE_", "EMERGE_"];

const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/bin";

/// Whether a root-side variable is carried over. Everything else (root's
/// `HOME`, `XDG_*`, `SUDO_*`, `MAIL`, `LD_*`, ...) is dropped.
pub(crate) fn keep_var(key: &str) -> bool {
    KEEP.contains(&key) || KEEP_PREFIX.iter().any(|p| key.starts_with(p))
}

/// The environment the new user starts with: kept variables plus
/// `HOME`/`USER`/`LOGNAME`/`SHELL` from passwd.
pub(crate) fn user_env(
    acc: &Account,
    old: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = old
        .into_iter()
        .filter(|(k, _)| k.to_str().is_some_and(keep_var))
        .collect();
    fn set(out: &mut Vec<(OsString, OsString)>, k: &str, v: OsString) {
        out.retain(|(ek, _)| ek != k);
        out.push((OsString::from(k), v));
    }
    set(&mut out, "HOME", acc.home.clone().into_os_string());
    set(&mut out, "USER", OsString::from(&acc.name));
    set(&mut out, "LOGNAME", OsString::from(&acc.name));
    if !acc.shell.is_empty() {
        set(&mut out, "SHELL", OsString::from(&acc.shell));
    }
    if !out.iter().any(|(k, v)| k == "PATH" && !v.is_empty()) {
        set(&mut out, "PATH", OsString::from(DEFAULT_PATH));
    }
    out
}

fn apply_env(acc: &Account) {
    let old: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let new = user_env(acc, old.clone());
    for (k, _) in &old {
        // SAFETY: privilege-drop path is single-threaded; 2024 edition marks
        // set_var/remove_var unsafe (data-race if concurrent readers exist).
        unsafe { std::env::remove_var(k) };
    }
    for (k, v) in new {
        // SAFETY: same as above — no other threads yet.
        unsafe { std::env::set_var(k, v) };
    }
}

/// The directory we were started in may be unreadable for the new user
/// (`sudo emerge` from /root). Fall back to $HOME, then `/`.
fn fix_cwd(home: &Path) {
    let usable = |p: &Path| -> bool {
        CString::new(p.as_os_str().as_bytes()).is_ok_and(|c| {
            // SAFETY: valid C string.
            unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::X_OK) == 0 }
        })
    };
    if let Ok(cwd) = std::env::current_dir()
        && usable(&cwd)
    {
        return;
    }
    if usable(home) && std::env::set_current_dir(home).is_ok() {
        return;
    }
    let _ = std::env::set_current_dir("/");
}

// ── the drop itself ─────────────────────────────────────────────────────────

fn sys(what: &'static str) -> DropError {
    DropError::Sys(what, io::Error::last_os_error())
}

/// Irreversible: groups, gid, uid (real+effective+saved), environment,
/// umask, cwd. Call while single-threaded and before reading any config.
///
/// Any failure leaves the process in an unknown state: the caller must
/// exit, not carry on.
pub(crate) fn become_user(acc: &Account) -> Result<(), DropError> {
    let cname = CString::new(acc.name.as_str())
        .map_err(|_| DropError::BadAccount("NUL in user name".into()))?;

    // SAFETY: plain credential syscalls. Order matters: supplementary
    // groups and gid while we are still allowed to, uid last.
    unsafe {
        if libc::initgroups(cname.as_ptr(), acc.gid) != 0 {
            return Err(sys("initgroups"));
        }
        if libc::setresgid(acc.gid, acc.gid, acc.gid) != 0 {
            return Err(sys("setresgid"));
        }
        if libc::setresuid(acc.uid, acc.uid, acc.uid) != 0 {
            return Err(sys("setresuid"));
        }
    }
    verify_dropped(acc)?;

    // The kernel marks a process that changed uid as non-dumpable, which
    // leaves /proc/self/* root-owned and breaks tools that look there
    // (bwrap, ps-like helpers). We are an ordinary user process now.
    // SAFETY: prctl with integer args only.
    unsafe {
        libc::prctl(
            libc::PR_SET_DUMPABLE,
            1 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        );
        libc::umask(0o022);
    }
    apply_env(acc);
    fix_cwd(&acc.home);
    Ok(())
}

/// Reads every id back and proves root cannot be regained.
fn verify_dropped(acc: &Account) -> Result<(), DropError> {
    let (mut ru, mut eu, mut su) = (0 as libc::uid_t, 0 as libc::uid_t, 0 as libc::uid_t);
    let (mut rg, mut eg, mut sg) = (0 as libc::gid_t, 0 as libc::gid_t, 0 as libc::gid_t);
    // SAFETY: out-pointers are valid locals.
    unsafe {
        if libc::getresuid(&mut ru, &mut eu, &mut su) != 0 {
            return Err(sys("getresuid"));
        }
        if libc::getresgid(&mut rg, &mut eg, &mut sg) != 0 {
            return Err(sys("getresgid"));
        }
    }
    if [ru, eu, su] != [acc.uid; 3] || [rg, eg, sg] != [acc.gid; 3] {
        return Err(DropError::StillRoot);
    }
    // SAFETY: setuid(0) must now fail; if it succeeds we are root again.
    if unsafe { libc::setuid(0) } == 0 {
        return Err(DropError::StillRoot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let m: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |k| m.get(k).map(|v| v.to_string())
    }

    #[test]
    fn sudo_uid_is_found() {
        let e = env_of(&[("SUDO_UID", "1000")]);
        assert_eq!(who_from(&e, None), Some(Who::Uid(1000)));
    }

    #[test]
    fn explicit_ae_user_beats_sudo() {
        let e = env_of(&[("AE_USER", "deltacat"), ("SUDO_UID", "1000")]);
        assert_eq!(who_from(&e, None), Some(Who::Name("deltacat".into())));
        let e = env_of(&[("AE_USER", "1234"), ("SUDO_UID", "1000")]);
        assert_eq!(who_from(&e, None), Some(Who::Uid(1234)));
    }

    #[test]
    fn doas_and_pkexec() {
        let e = env_of(&[("DOAS_USER", "bob")]);
        assert_eq!(who_from(&e, None), Some(Who::Name("bob".into())));
        let e = env_of(&[("PKEXEC_UID", "1001")]);
        assert_eq!(who_from(&e, None), Some(Who::Uid(1001)));
    }

    #[test]
    fn root_and_garbage_are_skipped_not_trusted() {
        // sudo run by root: SUDO_UID=0 -> next source
        let e = env_of(&[
            ("SUDO_UID", "0"),
            ("DOAS_USER", "root"),
            ("AE_USER", "root"),
        ]);
        assert_eq!(who_from(&e, Some("1000\n")), Some(Who::Uid(1000)));
        let e = env_of(&[
            ("SUDO_UID", "-1"),
            ("PKEXEC_UID", "abc"),
            ("SUDO_USER", "x"),
        ]);
        assert_eq!(who_from(&e, None), None);
        let e = env_of(&[]);
        assert_eq!(who_from(&e, Some("0")), None);
        assert_eq!(who_from(&e, Some("4294967295")), None);
        assert_eq!(who_from(&e, Some("")), None);
    }

    #[test]
    fn loginuid_is_the_last_resort() {
        let e = env_of(&[]);
        assert_eq!(who_from(&e, Some("1000")), Some(Who::Uid(1000)));
    }

    #[test]
    fn env_whitelist() {
        assert!(keep_var("TERM") && keep_var("LC_ALL") && keep_var("AE_DEBUG"));
        assert!(keep_var("EMERGE_DEFAULT_OPTS") && keep_var("https_proxy"));
        for bad in [
            "HOME",
            "USER",
            "XDG_CACHE_HOME",
            "XDG_RUNTIME_DIR",
            "SUDO_USER",
            "LD_PRELOAD",
            "MAIL",
            "GNUPGHOME",
            "BASH_ENV",
            "IFS",
        ] {
            assert!(!keep_var(bad), "{} must not survive", bad);
        }
    }

    #[test]
    fn user_env_rebuilds_identity() {
        let acc = Account {
            uid: 1000,
            gid: 1000,
            name: "deltacat".into(),
            home: "/home/deltacat".into(),
            shell: "/bin/zsh".into(),
        };
        let old: Vec<(OsString, OsString)> = [
            ("HOME", "/root"),
            ("XDG_CACHE_HOME", "/root/.cache"),
            ("LD_PRELOAD", "/tmp/x.so"),
            ("TERM", "xterm-256color"),
            ("LANG", "uk_UA.UTF-8"),
        ]
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
        let new: HashMap<String, String> = user_env(&acc, old)
            .into_iter()
            .map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
            .collect();
        assert_eq!(new["HOME"], "/home/deltacat");
        assert_eq!(new["USER"], "deltacat");
        assert_eq!(new["LOGNAME"], "deltacat");
        assert_eq!(new["SHELL"], "/bin/zsh");
        assert_eq!(new["TERM"], "xterm-256color");
        assert_eq!(new["LANG"], "uk_UA.UTF-8");
        assert_eq!(new["PATH"], DEFAULT_PATH);
        assert!(!new.contains_key("XDG_CACHE_HOME"));
        assert!(!new.contains_key("LD_PRELOAD"));
    }

    #[test]
    fn accept_refuses_root_and_homeless() {
        let ok = Account {
            uid: 1000,
            gid: 1000,
            name: "u".into(),
            home: "/home/u".into(),
            shell: String::new(),
        };
        assert!(accept(ok.clone()).is_ok());
        assert!(
            accept(Account {
                uid: 0,
                ..ok.clone()
            })
            .is_err()
        );
        assert!(
            accept(Account {
                name: String::new(),
                ..ok.clone()
            })
            .is_err()
        );
        assert!(
            accept(Account {
                home: "relative".into(),
                ..ok.clone()
            })
            .is_err()
        );
        assert!(
            accept(Account {
                home: "".into(),
                ..ok
            })
            .is_err()
        );
    }

    #[test]
    fn passwd_lookup_finds_root_and_misses_nobody_like_names() {
        let root = passwd_lookup(&Who::Uid(0)).unwrap().unwrap();
        assert_eq!(root.name, "root");
        assert_eq!(
            passwd_lookup(&Who::Name("root".into()))
                .unwrap()
                .unwrap()
                .uid,
            0
        );
        assert!(
            passwd_lookup(&Who::Name("no-such-user-ae-test".into()))
                .unwrap()
                .is_none()
        );
    }
}
