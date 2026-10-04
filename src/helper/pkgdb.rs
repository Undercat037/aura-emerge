//! Helper-side libalpm: package db writes, runs as root.
//! No `crate::` imports: std, alpm, alpm-utils and siblings only.
//! Alpm is !Send and caches the db, so a fresh handle per request.

use std::io::{self, ErrorKind};

use alpm::{Alpm, PackageReason, TransFlag};
use alpm_utils::alpm_with_conf;
use alpm_utils::config::Config;

use super::validate;

fn fail(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::Other, msg.into())
}

/// Root handle from /etc/pacman.conf.
fn open() -> io::Result<Alpm> {
    let conf = Config::new().map_err(|e| fail(format!("pacman.conf: {}", e)))?;
    alpm_with_conf(&conf).map_err(|e| fail(format!("libalpm: {}", e)))
}

/// `pacman -D --asexplicit` / `--asdeps`.
pub(crate) fn set_reason(explicit: bool, names: &[String]) -> io::Result<()> {
    let mut alpm = open()?;
    set_reason_in(&mut alpm, explicit, names)
}

/// Same, on a given handle (tests use a temp db).
/// Takes the db lock like pacman does. Every name is tried; the error
/// lists the ones that were not installed or failed.
pub(crate) fn set_reason_in(alpm: &mut Alpm, explicit: bool, names: &[String]) -> io::Result<()> {
    let reason = if explicit {
        PackageReason::Explicit
    } else {
        PackageReason::Depend
    };
    // Validate everything before taking the lock or touching the db.
    let mut bare = Vec::with_capacity(names.len());
    for n in names {
        let a = validate::atom(n).map_err(|r| fail(format!("bad name: {:?}", r)))?;
        bare.push(a.name);
    }
    if bare.is_empty() {
        return Err(fail("no packages"));
    }

    alpm.trans_init(TransFlag::NONE)
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = apply(alpm, reason, &bare);
    let _ = alpm.trans_release();
    res
}

fn apply(alpm: &Alpm, reason: PackageReason, names: &[&str]) -> io::Result<()> {
    let local = alpm.localdb();
    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for &name in names {
        match local.pkg(name) {
            Err(_) => missing.push(name),
            Ok(pkg) => {
                if let Err(e) = pkg.set_reason(reason) {
                    failed.push(format!("{} ({})", name, e));
                }
            }
        }
    }
    if missing.is_empty() && failed.is_empty() {
        return Ok(());
    }
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("not installed: {}", missing.join(" ")));
    }
    if !failed.is_empty() {
        parts.push(format!("failed: {}", failed.join(" ")));
    }
    Err(fail(parts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Temp root with an empty-but-valid local db and `pkgs` installed.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str, pkgs: &[(&str, &str)]) -> Fixture {
            let root =
                std::env::temp_dir().join(format!("ae-pkgdb-{}-{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let local = root.join("db/local");
            fs::create_dir_all(&local).unwrap();
            fs::write(local.join("ALPM_DB_VERSION"), "9\n").unwrap();
            for (name, ver) in pkgs {
                let dir = local.join(format!("{}-{}", name, ver));
                fs::create_dir_all(&dir).unwrap();
                fs::write(
                    dir.join("desc"),
                    format!("%NAME%\n{}\n\n%VERSION%\n{}\n\n", name, ver),
                )
                .unwrap();
            }
            Fixture { root }
        }

        fn handle(&self) -> Alpm {
            let r = self.root.to_str().unwrap();
            let db = self.root.join("db");
            Alpm::new(r, db.to_str().unwrap()).unwrap()
        }

        fn desc(&self, dir: &str) -> String {
            fs::read_to_string(self.root.join("db/local").join(dir).join("desc")).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(Path::new(&self.root));
        }
    }

    fn reason(fx: &Fixture, name: &str) -> PackageReason {
        // New handle: proves the change reached the disk.
        let h = fx.handle();
        let r = h.localdb().pkg(name).unwrap().reason();
        r
    }

    fn names(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn asdeps_then_asexplicit_roundtrip() {
        let fx = Fixture::new("rt", &[("foo", "1.0-1")]);
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);

        set_reason_in(&mut fx.handle(), false, &names(&["foo"])).unwrap();
        assert_eq!(reason(&fx, "foo"), PackageReason::Depend);
        assert!(fx.desc("foo-1.0-1").contains("%REASON%\n1"));

        set_reason_in(&mut fx.handle(), true, &names(&["foo"])).unwrap();
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);
    }

    #[test]
    fn repo_prefix_is_ignored() {
        let fx = Fixture::new("repo", &[("foo", "1.0-1")]);
        set_reason_in(&mut fx.handle(), false, &names(&["extra/foo"])).unwrap();
        assert_eq!(reason(&fx, "foo"), PackageReason::Depend);
    }

    #[test]
    fn missing_is_reported_but_others_still_done() {
        let fx = Fixture::new("miss", &[("foo", "1.0-1"), ("bar", "2-1")]);
        let err = set_reason_in(&mut fx.handle(), false, &names(&["foo", "nope", "bar"]))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "not installed: nope");
        assert_eq!(reason(&fx, "foo"), PackageReason::Depend);
        assert_eq!(reason(&fx, "bar"), PackageReason::Depend);
    }

    #[test]
    fn bad_name_touches_nothing() {
        let fx = Fixture::new("bad", &[("foo", "1.0-1")]);
        let err = set_reason_in(&mut fx.handle(), false, &names(&["foo", "--noconfirm"]));
        assert!(err.is_err());
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);
    }

    #[test]
    fn empty_list_is_an_error() {
        let fx = Fixture::new("empty", &[]);
        assert!(set_reason_in(&mut fx.handle(), true, &[]).is_err());
    }

    #[test]
    fn lock_is_released_after() {
        let fx = Fixture::new("lock", &[("foo", "1.0-1")]);
        set_reason_in(&mut fx.handle(), false, &names(&["foo"])).unwrap();
        assert!(!fx.root.join("db/db.lck").exists());
        // Failing run must release it too.
        let _ = set_reason_in(&mut fx.handle(), false, &names(&["nope"]));
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn held_lock_is_an_error() {
        let fx = Fixture::new("held", &[("foo", "1.0-1")]);
        fs::write(fx.root.join("db/db.lck"), "").unwrap();
        let err = set_reason_in(&mut fx.handle(), false, &names(&["foo"])).unwrap_err();
        assert!(err.to_string().starts_with("db lock:"));
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);
    }
}
