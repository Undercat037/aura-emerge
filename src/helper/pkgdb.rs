//! Helper-side libalpm: package db writes, runs as root.
//! No `crate::` imports: std, alpm, alpm-utils and siblings only.
//! Alpm is !Send and caches the db, so a fresh handle per request.
//!
//! Hooks: libalpm runs every `*.hook` under the configured HookDirs during
//! `trans_commit` (system dirs from pacman.conf + `/etc/portage/hooks`).
//! Event lines on the wire:
//!   `pkg start|done <name>`
//!   `hook start|done pre|post`
//!   `hook run <pos>/<total> <name> [<desc…>]`

use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::mpsc;

use alpm::{Alpm, PackageReason, PrepareData, SigLevel, TransFlag};
use alpm_utils::alpm_with_conf;
use alpm_utils::config::Config;

use super::stage::Stage;
use super::validate::{self, FileOpts, RemoveMode};

/// HookDirs that must always be registered. `pacman-conf` / the
/// `pacmanconf` crate often expand a commented-out `HookDir` in
/// `/etc/pacman.conf` to only `/etc/pacman.d/hooks/` (or nothing) and
/// **omit** `/usr/share/libalpm/hooks/`, where package-provided hooks
/// live (`60-depmod.hook`, `90-dracut-install.hook`, `70-dkms-*.hook`, …).
/// Empty / wrong hookdirs ⇒ silent installs and a broken boot after a
/// kernel upgrade. Missing directories are fine: libalpm skips them.
const REQUIRED_HOOK_DIRS: &[&str] = &[
    "/usr/share/libalpm/hooks/",
    "/etc/pacman.d/hooks/",
    "/etc/portage/hooks/",
];

fn fail(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::Other, msg.into())
}

/// Root handle from /etc/pacman.conf.
/// Keeps pacman.conf SigLevel so package/db signatures are verified
/// (user-side `alpm_db::open` turns them off to avoid gpg "unsafe
/// ownership" on `/etc/pacman.d/gnupg`).
fn open() -> io::Result<Alpm> {
    let conf = Config::new().map_err(|e| fail(format!("pacman.conf: {}", e)))?;
    let mut alpm = alpm_with_conf(&conf).map_err(|e| fail(format!("libalpm: {}", e)))?;
    ensure_hook_dirs(&mut alpm);
    Ok(alpm)
}

/// Register every required HookDir if not already present (idempotent).
fn ensure_hook_dirs(alpm: &mut Alpm) {
    let existing: Vec<String> = alpm.hookdirs().iter().map(|s| s.to_string()).collect();
    for dir in REQUIRED_HOOK_DIRS {
        let already = existing
            .iter()
            .any(|e| e.trim_end_matches('/') == dir.trim_end_matches('/'));
        if !already {
            let _ = alpm.add_hookdir(*dir);
        }
    }
}

/// New-side package name of an install/upgrade step (removes are skipped
/// for the `pkg start/done` progress line; removals still fire hooks).
fn op_name(op: alpm::PackageOperation) -> Option<String> {
    use alpm::PackageOperation as P;
    match op {
        P::Install(p) | P::Upgrade(p, _) | P::Reinstall(p, _) | P::Downgrade(p, _) => {
            Some(p.name().to_string())
        }
        P::Remove(_) => None,
    }
}

/// Wire libalpm package + hook events onto `tx` (non-blocking best-effort).
/// Must be called **before** `trans_init` / `trans_commit`.
///
/// The frontend owns the terminal (Jobs footer + `>>>` lines); the helper
/// only ships events on the protocol channel so the Jobs line can stay
/// pinned at the bottom via `progress::note`.
fn wire_events(alpm: &mut Alpm, tx: mpsc::Sender<String>) {
    alpm.set_event_cb((), move |ev, _| match ev.event() {
        alpm::Event::PackageOperationStart(e) => {
            if let Some(n) = op_name(e.operation()) {
                let _ = tx.send(format!("pkg start {}", n));
            }
        }
        alpm::Event::PackageOperationDone(e) => {
            if let Some(n) = op_name(e.operation()) {
                let _ = tx.send(format!("pkg done {}", n));
            }
        }
        alpm::Event::HookStart(e) => {
            let when = match e.when() {
                alpm::HookWhen::PreTransaction => "pre",
                alpm::HookWhen::PostTransaction => "post",
            };
            let _ = tx.send(format!("hook start {}", when));
        }
        alpm::Event::HookDone(e) => {
            let when = match e.when() {
                alpm::HookWhen::PreTransaction => "pre",
                alpm::HookWhen::PostTransaction => "post",
            };
            let _ = tx.send(format!("hook done {}", when));
        }
        alpm::Event::HookRunStart(e) => {
            let name = e.name();
            let pos = e.position();
            let total = e.total();
            let line = match e.desc().filter(|d| !d.is_empty()) {
                Some(d) => format!("hook run {}/{} {} {}", pos, total, name, d),
                None => format!("hook run {}/{} {}", pos, total, name),
            };
            let _ = tx.send(line);
        }
        _ => {}
    });
}

/// Run `work` on a fresh Alpm handle in a worker thread; forward events live.
/// Alpm is !Send, so the handle is created inside the worker (same pattern as
/// `sync` / `sysupgrade`).
fn with_live_events(
    emit: &mut dyn FnMut(&str),
    work: impl FnOnce(mpsc::Sender<String>) -> io::Result<()> + Send,
) -> io::Result<()> {
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<String>();
        let worker = s.spawn(move || work(tx));
        for ev in rx {
            emit(&ev);
        }
        worker
            .join()
            .unwrap_or_else(|_| Err(fail("alpm worker panicked")))
    })
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

/// Libalpm flags for each mode (same as the pacman options in `RemoveMode`).
fn flags_for(mode: RemoveMode) -> TransFlag {
    match mode {
        RemoveMode::Plain => TransFlag::NONE,
        RemoveMode::Unmerge => TransFlag::NO_SAVE | TransFlag::NO_DEPS | TransFlag::NO_DEP_VERSION,
        RemoveMode::Prune => TransFlag::RECURSE | TransFlag::NO_SAVE,
    }
}

/// `pacman -R*` as root. Hooks and scriptlets run as usual.
/// `emit` gets `pkg`/`hook` event lines live (same format as install/sysupgrade).
pub(crate) fn remove(
    mode: RemoveMode,
    names: &[String],
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let names = names.to_vec();
    with_live_events(emit, move |tx| {
        let mut alpm = open()?;
        remove_in_with(&mut alpm, mode, &names, tx)
    })
}

/// Same, on a given handle. All-or-nothing: an unknown name aborts
/// before anything is queued, like pacman's "target not found".
/// Events are drained after commit (for tests on a fixture handle).
pub(crate) fn remove_in(
    alpm: &mut Alpm,
    mode: RemoveMode,
    names: &[String],
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let (tx, rx) = mpsc::channel::<String>();
    let res = remove_in_with(alpm, mode, names, tx);
    for ev in rx.try_iter() {
        emit(&ev);
    }
    res
}

fn remove_in_with(
    alpm: &mut Alpm,
    mode: RemoveMode,
    names: &[String],
    tx: mpsc::Sender<String>,
) -> io::Result<()> {
    let mut bare = Vec::with_capacity(names.len());
    for n in names {
        let a = validate::atom(n).map_err(|r| fail(format!("bad name: {:?}", r)))?;
        bare.push(a.name);
    }
    if bare.is_empty() {
        return Err(fail("no packages"));
    }

    wire_events(alpm, tx);
    alpm.trans_init(flags_for(mode))
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = run_remove(alpm, &bare);
    let _ = alpm.trans_release();
    res
}

fn run_remove(alpm: &mut Alpm, names: &[&str]) -> io::Result<()> {
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| alpm.localdb().pkg(*n).is_err())
        .collect();
    if !missing.is_empty() {
        return Err(fail(format!("not installed: {}", missing.join(" "))));
    }
    for &name in names {
        let pkg = alpm
            .localdb()
            .pkg(name)
            .map_err(|e| fail(format!("{}: {}", name, e)))?;
        alpm.trans_remove_pkg(pkg)
            .map_err(|e| fail(format!("{}: {}", name, e)))?;
    }
    alpm.trans_prepare().map_err(|e| fail(prepare_msg(&e)))?;
    alpm.trans_commit()
        .map_err(|e| fail(format!("commit: {}", e)))
}

/// `pacman -Sy`; `force` = `-Syy` (download even if up to date).
/// `emit` gets `sync <repo> <updated|uptodate|failed>` per db, live, in
/// completion order (libalpm downloads in parallel).
///
/// libalpm blocks in `update()`, so it runs on a worker thread (the
/// handle is created there: Alpm is !Send) and events cross a channel
/// to the caller's thread, which owns `emit`.
pub(crate) fn sync(force: bool, emit: &mut dyn FnMut(&str)) -> io::Result<()> {
    use std::sync::mpsc;

    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<String>();
        let worker = s.spawn(move || {
            let mut alpm = open()?;
            sync_in_with(&mut alpm, force, &tx)
        });
        // Ends when the worker is done and its sender is dropped.
        for ev in rx {
            emit(&ev);
        }
        worker
            .join()
            .unwrap_or_else(|_| Err(fail("sync: worker panicked")))
    })
}

/// Same, on a given handle. Takes the db lock like pacman does.
/// The "already up to date" flag from libalpm is dropped on purpose.
pub(crate) fn sync_in(alpm: &mut Alpm, force: bool) -> io::Result<()> {
    let (tx, _rx) = std::sync::mpsc::channel();
    sync_in_with(alpm, force, &tx)
}

pub(crate) fn sync_in_with(
    alpm: &mut Alpm,
    force: bool,
    tx: &std::sync::mpsc::Sender<String>,
) -> io::Result<()> {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    // dbs that already reported an outcome
    let seen: Rc<RefCell<HashSet<String>>> = Rc::default();
    let sink = Rc::clone(&seen);
    let cb_tx = tx.clone();
    alpm.set_dl_cb((), move |file, ev, _| {
        if let alpm::DownloadEvent::Completed(c) = ev.event() {
            if let Some(db) = file.strip_suffix(".db") {
                let state = match c.result {
                    alpm::DownloadResult::Success => "updated",
                    alpm::DownloadResult::UpToDate => "uptodate",
                    alpm::DownloadResult::Failed => "failed",
                };
                sink.borrow_mut().insert(db.to_string());
                let _ = cb_tx.send(format!("sync {} {}", db, state));
            }
        }
    });

    alpm.trans_init(TransFlag::NONE)
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = alpm
        .syncdbs_mut()
        .update(force)
        .map(|_| ())
        .map_err(|e| fail(format!("sync: {}", e)));
    let _ = alpm.trans_release();

    // A db with no event counts as unchanged (failed if the update failed).
    let seen = seen.borrow();
    for db in alpm.syncdbs() {
        if !seen.contains(db.name()) {
            let state = if res.is_err() { "failed" } else { "uptodate" };
            let _ = tx.send(format!("sync {} {}", db.name(), state));
        }
    }
    res
}

/// `pacman -Su`: upgrade every installed package that has a newer
/// version in a sync db. `ignore` is `--ignore` / package.mask holdback
/// (bare names). Empty transaction is success (nothing to do).
/// `emit` gets `pkg start <name>` / `pkg done <name>` per installed or
/// upgraded package, live (worker thread + channel, as in `sync`).
pub(crate) fn sysupgrade(ignore: &[String], emit: &mut dyn FnMut(&str)) -> io::Result<()> {
    use std::sync::mpsc;

    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel::<String>();
        let worker = s.spawn(move || {
            let mut alpm = open()?;
            sysupgrade_in_with(&mut alpm, ignore, &tx)
        });
        for ev in rx {
            emit(&ev);
        }
        worker
            .join()
            .unwrap_or_else(|_| Err(fail("sysupgrade: worker panicked")))
    })
}

pub(crate) fn sysupgrade_in(alpm: &mut Alpm, ignore: &[String]) -> io::Result<()> {
    let (tx, _rx) = std::sync::mpsc::channel();
    sysupgrade_in_with(alpm, ignore, &tx)
}

fn sysupgrade_in_with(
    alpm: &mut Alpm,
    ignore: &[String],
    tx: &mpsc::Sender<String>,
) -> io::Result<()> {
    wire_events(alpm, tx.clone());
    for n in ignore {
        let a = validate::atom(n).map_err(|r| fail(format!("bad ignore: {:?}", r)))?;
        let _ = alpm.add_ignorepkg(a.name);
    }
    alpm.trans_init(TransFlag::NONE)
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = (|| {
        // false = no downgrade (same as pacman -Su without -d).
        alpm.sync_sysupgrade(false)
            .map_err(|e| fail(format!("sysupgrade: {}", e)))?;
        // Real transaction size, so the client can tell "nothing to do"
        // from "plan and libalpm disagree".
        let _ = tx.send(format!("plan {}", alpm.trans_add().iter().count()));
        prepare_and_commit(alpm)
    })();
    let _ = alpm.trans_release();
    res
}

/// `pacman -S`: install from the sync dbs by `[repo/]name`.
/// `emit` gets `pkg`/`hook` event lines live.
pub(crate) fn install(
    names: &[String],
    needed: bool,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let names = names.to_vec();
    with_live_events(emit, move |tx| {
        let mut alpm = open()?;
        install_in_with(&mut alpm, &names, needed, tx)
    })
}

/// Same, on a given handle. All-or-nothing: an unknown target aborts
/// before anything is queued. Deps are resolved by libalpm; they get
/// the `asdeps` reason, the named targets stay explicit.
/// Events are drained after commit (for tests on a fixture handle).
pub(crate) fn install_in(
    alpm: &mut Alpm,
    names: &[String],
    needed: bool,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let (tx, rx) = mpsc::channel::<String>();
    let res = install_in_with(alpm, names, needed, tx);
    for ev in rx.try_iter() {
        emit(&ev);
    }
    res
}

fn install_in_with(
    alpm: &mut Alpm,
    names: &[String],
    needed: bool,
    tx: mpsc::Sender<String>,
) -> io::Result<()> {
    let mut targets = Vec::with_capacity(names.len());
    for n in names {
        let a = validate::atom(n).map_err(|r| fail(format!("bad name: {:?}", r)))?;
        targets.push((a.repo, a.name));
    }
    if targets.is_empty() {
        return Err(fail("no packages"));
    }

    // package.mask is a hard stop, including transitive names the client
    // asked for by hand. libalpm-resolved deps are not checked here yet
    // (would need a prepare-pass to know them); explicit targets are.
    super::pkgmask::refuse_masked(&targets).map_err(|e| fail(e.to_string()))?;

    let mut flags = TransFlag::NONE;
    if needed {
        flags |= TransFlag::NEEDED;
    }
    wire_events(alpm, tx);
    alpm.trans_init(flags)
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = run_install(alpm, &targets);
    let _ = alpm.trans_release();
    res
}

/// First sync db (in pacman.conf order) that has `name`; `repo` pins one.
/// Exact names only, no providers yet.
fn find_sync<'a>(alpm: &'a Alpm, repo: Option<&str>, name: &str) -> Option<&'a alpm::Package> {
    alpm.syncdbs()
        .iter()
        .filter(|d| repo.map_or(true, |r| d.name() == r))
        .find_map(|d| d.pkg(name).ok())
}

fn run_install(alpm: &mut Alpm, targets: &[(Option<&str>, &str)]) -> io::Result<()> {
    let missing: Vec<String> = targets
        .iter()
        .filter(|(r, n)| find_sync(alpm, *r, n).is_none())
        .map(|(r, n)| match r {
            Some(r) => format!("{}/{}", r, n),
            None => n.to_string(),
        })
        .collect();
    if !missing.is_empty() {
        return Err(fail(format!("target not found: {}", missing.join(" "))));
    }
    for &(repo, name) in targets {
        let pkg = find_sync(alpm, repo, name).ok_or_else(|| fail(format!("{}: gone", name)))?;
        alpm.trans_add_pkg(pkg)
            .map_err(|e| fail(format!("{}: {}", name, e)))?;
    }
    prepare_and_commit(alpm)
}

/// `pacman -U`. `specs` are `<sha256> <abs path>` lines. The files are
/// copied into a root-private dir and hash-checked *before* libalpm
/// sees them (closes the audit -> install TOCTOU); the stage dir is
/// removed when this returns.
/// `emit` gets `pkg`/`hook` event lines live.
pub(crate) fn install_files(
    opts: FileOpts,
    specs: &[String],
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let mut stage = Stage::create()?;
    let paths = stage.copy_specs(specs)?;
    with_live_events(emit, move |tx| {
        let mut alpm = open()?;
        let level = alpm.local_file_siglevel();
        install_files_in_with(&mut alpm, &paths, level, opts, tx)
    })
}

/// Same, on a given handle and already-staged files.
/// Events are drained after commit (for tests on a fixture handle).
pub(crate) fn install_files_in(
    alpm: &mut Alpm,
    files: &[PathBuf],
    level: SigLevel,
    opts: FileOpts,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let (tx, rx) = mpsc::channel::<String>();
    let res = install_files_in_with(alpm, files, level, opts, tx);
    for ev in rx.try_iter() {
        emit(&ev);
    }
    res
}

fn install_files_in_with(
    alpm: &mut Alpm,
    files: &[PathBuf],
    level: SigLevel,
    opts: FileOpts,
    tx: mpsc::Sender<String>,
) -> io::Result<()> {
    if files.is_empty() {
        return Err(fail("no packages"));
    }
    let mut flags = TransFlag::NONE;
    if opts.needed {
        flags |= TransFlag::NEEDED;
    }
    if opts.asdeps {
        flags |= TransFlag::ALL_DEPS;
    }
    wire_events(alpm, tx);
    alpm.trans_init(flags)
        .map_err(|e| fail(format!("db lock: {}", e)))?;
    let res = run_install_files(alpm, files, level);
    let _ = alpm.trans_release();
    res
}

fn run_install_files(alpm: &mut Alpm, files: &[PathBuf], level: SigLevel) -> io::Result<()> {
    for f in files {
        // Staged names are "<n>-<original>"; show the original.
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let shown = name.split_once('-').map_or(name, |(_, rest)| rest);
        let path = f.to_str().ok_or_else(|| fail("non-utf8 path"))?;
        let pkg = alpm
            .pkg_load(path, true, level)
            .map_err(|e| fail(format!("{}: {}", shown, e)))?;
        alpm.trans_add_pkg(pkg)
            .map_err(|e| fail(format!("{}: {}", shown, e)))?;
    }
    prepare_and_commit(alpm)
}

fn prepare_and_commit(alpm: &mut Alpm) -> io::Result<()> {
    // `--needed` can leave nothing to do; that is success, as in pacman.
    if alpm.trans_add().iter().next().is_none() {
        return Ok(());
    }
    alpm.trans_prepare().map_err(|e| fail(prepare_msg(&e)))?;
    alpm.trans_commit()
        .map_err(|e| fail(format!("commit: {}", e)))
}

/// Short one-liner for a failed prepare; the wire caps it anyway.
fn prepare_msg(e: &alpm::PrepareError) -> String {
    if let Some(PrepareData::UnsatisfiedDeps(list)) = e.data() {
        let top: Vec<String> = list
            .iter()
            .take(4)
            .map(|d| format!("{} needs {}", d.target(), d.depend()))
            .collect();
        if !top.is_empty() {
            return format!("unsatisfied: {}", top.join(", "));
        }
    }
    format!("prepare: {}", e)
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

        /// Adds an installed package that owns `files` (root-relative,
        /// created on disk) and, optionally, depends on / was pulled in as dep.
        fn add(&self, name: &str, ver: &str, files: &[&str], deps: &[&str], asdep: bool) {
            let dir = self.root.join("db/local").join(format!("{}-{}", name, ver));
            fs::create_dir_all(&dir).unwrap();
            let mut desc = format!("%NAME%\n{}\n\n%VERSION%\n{}\n\n", name, ver);
            if asdep {
                desc.push_str("%REASON%\n1\n\n");
            }
            if !deps.is_empty() {
                desc.push_str(&format!("%DEPENDS%\n{}\n\n", deps.join("\n")));
            }
            fs::write(dir.join("desc"), desc).unwrap();
            let mut list = String::from("%FILES%\n");
            for f in files {
                let p = self.root.join(f);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(&p, "x").unwrap();
                list.push_str(f);
                list.push('\n');
            }
            fs::write(dir.join("files"), list).unwrap();
        }

        fn installed(&self, name: &str) -> bool {
            self.handle().localdb().pkg(name).is_ok()
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

    #[test]
    fn flags_match_pacman_options() {
        assert_eq!(flags_for(RemoveMode::Plain), TransFlag::NONE);
        let u = flags_for(RemoveMode::Unmerge);
        assert!(u.contains(TransFlag::NO_SAVE | TransFlag::NO_DEPS | TransFlag::NO_DEP_VERSION));
        assert!(!u.contains(TransFlag::RECURSE));
        let p = flags_for(RemoveMode::Prune);
        assert!(p.contains(TransFlag::RECURSE | TransFlag::NO_SAVE));
        assert!(!p.contains(TransFlag::NO_DEPS));
    }

    #[test]
    fn remove_deletes_files_and_db_entry() {
        let fx = Fixture::new("rm", &[]);
        fx.add("foo", "1.0-1", &["usr/bin/foo"], &[], false);
        remove_in(
            &mut fx.handle(),
            RemoveMode::Plain,
            &names(&["foo"]),
            &mut |_| {},
        )
        .unwrap();
        assert!(!fx.installed("foo"));
        assert!(!fx.root.join("usr/bin/foo").exists());
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn unknown_name_aborts_before_removing_anything() {
        let fx = Fixture::new("rmmiss", &[]);
        fx.add("foo", "1.0-1", &["usr/bin/foo"], &[], false);
        let err = remove_in(
            &mut fx.handle(),
            RemoveMode::Plain,
            &names(&["foo", "nope"]),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "not installed: nope");
        assert!(fx.installed("foo"));
        assert!(fx.root.join("usr/bin/foo").exists());
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn plain_refuses_needed_package_unmerge_does_not() {
        let fx = Fixture::new("rmdep", &[]);
        fx.add("lib", "1-1", &["usr/lib/lib"], &[], false);
        fx.add("app", "1-1", &["usr/bin/app"], &["lib"], false);

        let err = remove_in(
            &mut fx.handle(),
            RemoveMode::Plain,
            &names(&["lib"]),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(err.starts_with("unsatisfied:"), "{}", err);
        assert!(fx.installed("lib"));

        remove_in(
            &mut fx.handle(),
            RemoveMode::Unmerge,
            &names(&["lib"]),
            &mut |_| {},
        )
        .unwrap();
        assert!(!fx.installed("lib"));
        assert!(fx.installed("app"));
    }

    #[test]
    fn prune_takes_unneeded_deps_but_not_explicit_ones() {
        let fx = Fixture::new("rmprune", &[]);
        fx.add("dep", "1-1", &["usr/lib/dep"], &[], true);
        fx.add("keep", "1-1", &["usr/lib/keep"], &[], false);
        fx.add("app", "1-1", &["usr/bin/app"], &["dep", "keep"], false);
        remove_in(
            &mut fx.handle(),
            RemoveMode::Prune,
            &names(&["app"]),
            &mut |_| {},
        )
        .unwrap();
        assert!(!fx.installed("app"));
        assert!(!fx.installed("dep"));
        assert!(fx.installed("keep"));
    }

    #[test]
    fn remove_rejects_bad_names_and_empty_list() {
        let fx = Fixture::new("rmbad", &[]);
        fx.add("foo", "1.0-1", &["usr/bin/foo"], &[], false);
        assert!(remove_in(
            &mut fx.handle(),
            RemoveMode::Plain,
            &names(&["--nodeps"]),
            &mut |_| {}
        )
        .is_err());
        assert!(remove_in(&mut fx.handle(), RemoveMode::Plain, &[], &mut |_| {}).is_err());
        assert!(fx.installed("foo"));
    }

    #[test]
    fn sync_pulls_a_file_repo() {
        let fx = Fixture::new("sync", &[]);
        fs::create_dir_all(fx.root.join("db/sync")).unwrap();
        // Tiny repo db served over file://; needs `tar` (always on Arch).
        let repo = fx.root.join("repo");
        let entry = repo.join("src/foo-1.0-1");
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("desc"), "%NAME%\nfoo\n\n%VERSION%\n1.0-1\n\n").unwrap();
        let st = std::process::Command::new("tar")
            .arg("-czf")
            .arg(repo.join("test.db"))
            .arg("-C")
            .arg(repo.join("src"))
            .arg("foo-1.0-1")
            .status()
            .unwrap();
        assert!(st.success());

        let url = format!("file://{}", repo.display());
        let handle = || {
            let mut h = fx.handle();
            h.register_syncdb_mut("test", SigLevel::NONE)
                .unwrap()
                .add_server(url.clone())
                .unwrap();
            h
        };
        sync_in(&mut handle(), false).unwrap();
        assert!(fx.root.join("db/sync/test.db").exists());
        assert!(!fx.root.join("db/db.lck").exists());
        // New handle: proves the db really landed on disk.
        assert!(handle().syncdbs().iter().any(|d| d.pkg("foo").is_ok()));
        // -Syy path works too.
        sync_in(&mut handle(), true).unwrap();
    }

    #[test]
    fn sync_with_held_lock_is_an_error() {
        let fx = Fixture::new("synclock", &[]);
        fs::write(fx.root.join("db/db.lck"), "").unwrap();
        let err = sync_in(&mut fx.handle(), false).unwrap_err();
        assert!(err.to_string().starts_with("db lock:"));
    }

    /// Local `file://` repo "test" with `foo-1.0-1` (owns usr/bin/foo).
    /// Needs `tar`, `sha256sum`, `md5sum` (coreutils/Arch base).
    fn make_repo(fx: &Fixture) -> String {
        use std::process::Command;
        let run = |cmd: &str| -> String {
            let o = Command::new("sh").arg("-c").arg(cmd).output().unwrap();
            assert!(o.status.success(), "{}", cmd);
            String::from_utf8(o.stdout).unwrap()
        };
        let repo = fx.root.join("repo");
        let src = fx.root.join("pkgsrc");
        let dbsrc = fx.root.join("dbsrc/foo-1.0-1");
        fs::create_dir_all(src.join("usr/bin")).unwrap();
        fs::create_dir_all(&dbsrc).unwrap();
        fs::write(src.join("usr/bin/foo"), "x").unwrap();
        fs::write(
            src.join(".PKGINFO"),
            "pkgname = foo\npkgver = 1.0-1\npkgdesc = t\nsize = 1\narch = any\n",
        )
        .unwrap();
        let file = "foo-1.0-1-any.pkg.tar.gz";
        let pkg = repo.join(file);
        fs::create_dir_all(&repo).unwrap();
        run(&format!(
            "tar -czf {} -C {} .PKGINFO usr",
            pkg.display(),
            src.display()
        ));
        let sum = |tool: &str| {
            run(&format!("{} {}", tool, pkg.display()))
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        };
        let size = fs::metadata(&pkg).unwrap().len();
        fs::write(
            dbsrc.join("desc"),
            format!(
                "%FILENAME%\n{}\n\n%NAME%\nfoo\n\n%VERSION%\n1.0-1\n\n%ARCH%\nany\n\n\
                 %CSIZE%\n{}\n\n%ISIZE%\n1\n\n%MD5SUM%\n{}\n\n%SHA256SUM%\n{}\n\n",
                file,
                size,
                sum("md5sum"),
                sum("sha256sum")
            ),
        )
        .unwrap();
        run(&format!(
            "tar -czf {} -C {} foo-1.0-1",
            repo.join("test.db").display(),
            fx.root.join("dbsrc").display()
        ));
        format!("file://{}", repo.display())
    }

    /// Handle with repo "test" registered and a private cache dir.
    fn repo_handle(fx: &Fixture, url: &str) -> Alpm {
        let cache = fx.root.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let mut h = fx.handle();
        h.add_cachedir(cache.to_str().unwrap().to_string()).unwrap();
        h.register_syncdb_mut("test", SigLevel::NONE)
            .unwrap()
            .add_server(url.to_string())
            .unwrap();
        h
    }

    #[test]
    fn install_unknown_target_aborts_and_unlocks() {
        let fx = Fixture::new("inmiss", &[]);
        let err = install_in(&mut fx.handle(), &names(&["nope"]), false, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert_eq!(err, "target not found: nope");
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn install_rejects_bad_names_and_empty_list() {
        let fx = Fixture::new("inbad", &[]);
        assert!(install_in(
            &mut fx.handle(),
            &names(&["--noconfirm"]),
            false,
            &mut |_| {}
        )
        .is_err());
        assert!(install_in(&mut fx.handle(), &[], false, &mut |_| {}).is_err());
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn install_with_held_lock_is_an_error() {
        let fx = Fixture::new("inlock", &[]);
        fs::write(fx.root.join("db/db.lck"), "").unwrap();
        let err = install_in(&mut fx.handle(), &names(&["foo"]), false, &mut |_| {}).unwrap_err();
        assert!(err.to_string().starts_with("db lock:"));
    }

    #[test]
    fn install_pinned_to_the_wrong_repo_is_not_found() {
        let fx = Fixture::new("inrepo", &[]);
        let url = make_repo(&fx);
        sync_in(&mut repo_handle(&fx, &url), false).unwrap();
        let err = install_in(
            &mut repo_handle(&fx, &url),
            &names(&["other/foo"]),
            false,
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "target not found: other/foo");
        assert!(!fx.installed("foo"));
    }

    #[test]
    fn install_from_file_repo() {
        let fx = Fixture::new("inok", &[]);
        let url = make_repo(&fx);
        sync_in(&mut repo_handle(&fx, &url), false).unwrap();
        // `test/foo` and bare `foo` both resolve.
        install_in(
            &mut repo_handle(&fx, &url),
            &names(&["test/foo"]),
            false,
            &mut |_| {},
        )
        .unwrap();
        assert!(fx.installed("foo"));
        assert!(fx.root.join("usr/bin/foo").exists());
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn install_local_file() {
        let fx = Fixture::new("localpkg", &[]);
        make_repo(&fx);
        let file = fx.root.join("repo/foo-1.0-1-any.pkg.tar.gz");
        install_files_in(
            &mut fx.handle(),
            &[file],
            SigLevel::NONE,
            FileOpts::default(),
            &mut |_| {},
        )
        .unwrap();
        assert!(fx.installed("foo"));
        assert!(fx.root.join("usr/bin/foo").exists());
        assert_eq!(reason(&fx, "foo"), PackageReason::Explicit);
        assert!(!fx.root.join("db/db.lck").exists());
    }

    #[test]
    fn install_local_garbage_is_an_error_and_unlocks() {
        let fx = Fixture::new("localbad", &[]);
        // Named like a staged copy: "<n>-<original>".
        let junk = fx.root.join("1-junk.pkg");
        fs::write(&junk, "not a package").unwrap();
        let err = install_files_in(
            &mut fx.handle(),
            &[junk],
            SigLevel::NONE,
            FileOpts::default(),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("junk.pkg:"), "{}", err);
        assert!(!fx.root.join("db/db.lck").exists());
        assert!(install_files_in(
            &mut fx.handle(),
            &[],
            SigLevel::NONE,
            FileOpts::default(),
            &mut |_| {},
        )
        .is_err());
    }

    #[test]
    fn install_local_asdeps_and_needed() {
        let fx = Fixture::new("localopts", &[]);
        make_repo(&fx);
        let file = fx.root.join("repo/foo-1.0-1-any.pkg.tar.gz");
        let opts = FileOpts {
            needed: true,
            asdeps: true,
        };
        install_files_in(
            &mut fx.handle(),
            &[file.clone()],
            SigLevel::NONE,
            opts,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(reason(&fx, "foo"), PackageReason::Depend);
        // Same version again with --needed: nothing to do, still success.
        install_files_in(&mut fx.handle(), &[file], SigLevel::NONE, opts, &mut |_| {}).unwrap();
        assert!(fx.installed("foo"));
        assert!(!fx.root.join("db/db.lck").exists());
    }
}
