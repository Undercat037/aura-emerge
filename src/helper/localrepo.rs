//! Every package aura-emerge builds (AUR / ABS / local PKGBUILD) or takes
//! from an archive is, after its audit and install, *published* into a
//! root-owned repository under `/var/lib/aura-emerge/repos/<repo>/`:
//! ---
//!   ae-aur/   foo-1.0-1-x86_64.pkg.tar.zst, ae-aur.db -> ae-aur.db.tar
//!   ae-abs/   ...
//!   ae-local/ ...
//! ---
//! The db is a plain (uncompressed) tar of `<name>-<ver>/desc` entries,
//! which is what `repo-add` writes minus the compression; libarchive,
//! hence libalpm, detects the filter by itself. That keeps this module
//! free of any compression crate and of a `repo-add` (bash) dependency.
//!
//! A copy of the db lands in libalpm's sync dir too, so the repo shows
//! up for `pacman -Si/-Ss/-Qm` as soon as it has a `[ae-aur]` section in
//! pacman.conf (see `misc/pacman-ae-repos.conf`).
//!
//! Trust: the packages were audited and sha256-pinned by the client and
//! are copied through the root-private `Stage` (hash re-checked there)
//! before they are touched. The repo is root-owned, so nobody can swap a
//! package between publish and a later `pacman -Su`.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::sha256::{self, Sha256};
use super::stage::Stage;
use super::validate::{LOCAL_REPO_ROOT, LocalRepo};

/// libalpm's sync dir. A copy of each db is dropped here (when it exists).
pub(crate) const SYNC_DIR: &str = "/var/lib/pacman/sync";

const BSDTAR: &str = "/usr/bin/bsdtar";
/// `.PKGINFO` of any sane package is a few KiB.
const MAX_PKGINFO: usize = 1 << 20;
/// Sanity cap for a db we read back.
const MAX_DB: u64 = 256 << 20;
/// ustar name field.
const TAR_NAME_MAX: usize = 100;

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, msg.into())
}

fn denied(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, msg.to_string())
}

// ── .PKGINFO ──────────────────────────────────────────────────────────

/// What a package says about itself; only what a sync db entry needs.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PkgInfo {
    pub(crate) name: String,
    pub(crate) base: String,
    /// `[epoch:]pkgver-pkgrel`, as makepkg writes it.
    pub(crate) version: String,
    pub(crate) desc: String,
    pub(crate) url: String,
    pub(crate) arch: String,
    pub(crate) builddate: String,
    pub(crate) packager: String,
    /// Installed size in bytes.
    pub(crate) isize: String,
    pub(crate) license: Vec<String>,
    pub(crate) groups: Vec<String>,
    pub(crate) replaces: Vec<String>,
    pub(crate) conflicts: Vec<String>,
    pub(crate) provides: Vec<String>,
    pub(crate) depends: Vec<String>,
    pub(crate) optdepends: Vec<String>,
    pub(crate) makedepends: Vec<String>,
    pub(crate) checkdepends: Vec<String>,
}

fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 90
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
}

fn version_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 60
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+~:-".contains(c))
}

fn arch_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Free text from a package: one line, no controls, and never something
/// that could pass for a `%SECTION%` header of the desc format.
fn text_ok(s: &str) -> bool {
    !s.chars().any(|c| c.is_control()) && !s.starts_with('%')
}

/// Parses `.PKGINFO` (`key = value` lines, `#` comments).
pub(crate) fn parse_pkginfo(text: &str) -> io::Result<PkgInfo> {
    let mut p = PkgInfo::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if v.is_empty() {
            continue;
        }
        if !text_ok(v) {
            return Err(bad(format!(".PKGINFO: bad value for {}", k)));
        }
        match k {
            "pkgname" => {
                if !p.name.is_empty() {
                    return Err(bad(".PKGINFO: more than one pkgname"));
                }
                p.name = v.to_string();
            }
            "pkgbase" => p.base = v.to_string(),
            "pkgver" => p.version = v.to_string(),
            "pkgdesc" => p.desc = v.to_string(),
            "url" => p.url = v.to_string(),
            "arch" => p.arch = v.to_string(),
            "builddate" => p.builddate = v.to_string(),
            "packager" => p.packager = v.to_string(),
            "size" => p.isize = v.to_string(),
            "license" => p.license.push(v.to_string()),
            "group" => p.groups.push(v.to_string()),
            "replaces" => p.replaces.push(v.to_string()),
            "conflict" => p.conflicts.push(v.to_string()),
            "provides" => p.provides.push(v.to_string()),
            "depend" => p.depends.push(v.to_string()),
            "optdepend" => p.optdepends.push(v.to_string()),
            "makedepend" => p.makedepends.push(v.to_string()),
            "checkdepend" => p.checkdepends.push(v.to_string()),
            _ => {}
        }
    }
    if !name_ok(&p.name) {
        return Err(bad(".PKGINFO: missing or odd pkgname"));
    }
    if !version_ok(&p.version) {
        return Err(bad(".PKGINFO: missing or odd pkgver"));
    }
    if !arch_ok(&p.arch) {
        return Err(bad(".PKGINFO: missing or odd arch"));
    }
    if !p.base.is_empty() && !name_ok(&p.base) {
        return Err(bad(".PKGINFO: odd pkgbase"));
    }
    if !p.builddate.chars().all(|c| c.is_ascii_digit()) {
        p.builddate.clear();
    }
    if !p.isize.chars().all(|c| c.is_ascii_digit()) {
        p.isize.clear();
    }
    Ok(p)
}

/// `.PKGINFO` text of `file`, via bsdtar (the same libarchive libalpm
/// reads the package with). Output is capped.
fn read_pkginfo(file: &Path) -> io::Result<String> {
    let mut child = Command::new(BSDTAR)
        .arg("-xOf")
        .arg(file)
        .arg(".PKGINFO")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| bad("bsdtar: no stdout"))?;
    let mut buf = Vec::new();
    let n = (&mut out)
        .take(MAX_PKGINFO as u64 + 1)
        .read_to_end(&mut buf)?;
    if n > MAX_PKGINFO {
        let _ = child.kill();
        let _ = child.wait();
        return Err(bad(".PKGINFO is too big"));
    }
    drop(out);
    let st = child.wait()?;
    if !st.success() || buf.is_empty() {
        return Err(bad("not a package archive (no .PKGINFO)"));
    }
    String::from_utf8(buf).map_err(|_| bad(".PKGINFO is not utf-8"))
}

// ── sync db entry ("desc") ────────────────────────────────────────────

fn section(out: &mut String, key: &str, vals: &[String]) {
    if vals.is_empty() {
        return;
    }
    out.push('%');
    out.push_str(key);
    out.push_str("%\n");
    for v in vals {
        out.push_str(v);
        out.push('\n');
    }
    out.push('\n');
}

fn one(out: &mut String, key: &str, val: &str) {
    if !val.is_empty() {
        section(out, key, &[val.to_string()]);
    }
}

/// Contents of `<name>-<ver>/desc`, in repo-add's order. `csize` = file
/// size; md5 is left out on purpose (libalpm uses the sha256 when set).
pub(crate) fn make_desc(i: &PkgInfo, filename: &str, csize: u64, sha256: &str) -> String {
    let mut d = String::new();
    one(&mut d, "FILENAME", filename);
    one(&mut d, "NAME", &i.name);
    one(&mut d, "BASE", &i.base);
    one(&mut d, "VERSION", &i.version);
    one(&mut d, "DESC", &i.desc);
    section(&mut d, "GROUPS", &i.groups);
    one(&mut d, "CSIZE", &csize.to_string());
    one(&mut d, "ISIZE", &i.isize);
    one(&mut d, "SHA256SUM", sha256);
    one(&mut d, "URL", &i.url);
    section(&mut d, "LICENSE", &i.license);
    one(&mut d, "ARCH", &i.arch);
    one(&mut d, "BUILDDATE", &i.builddate);
    one(&mut d, "PACKAGER", &i.packager);
    section(&mut d, "REPLACES", &i.replaces);
    section(&mut d, "CONFLICTS", &i.conflicts);
    section(&mut d, "PROVIDES", &i.provides);
    section(&mut d, "DEPENDS", &i.depends);
    section(&mut d, "OPTDEPENDS", &i.optdepends);
    section(&mut d, "MAKEDEPENDS", &i.makedepends);
    section(&mut d, "CHECKDEPENDS", &i.checkdepends);
    d
}

/// First value line of `%KEY%` in a desc.
fn desc_field<'a>(desc: &'a str, key: &str) -> Option<&'a str> {
    let head = format!("%{}%", key);
    let mut it = desc.lines();
    while let Some(l) = it.next() {
        if l == head {
            return it.next().filter(|v| !v.is_empty());
        }
    }
    None
}

// ── the db: a plain ustar archive ─────────────────────────────────────

fn put_octal(dst: &mut [u8], v: u64) {
    // `width-1` digits + NUL, as ustar wants.
    let s = format!("{:0w$o}", v, w = dst.len() - 1);
    dst[..s.len()].copy_from_slice(s.as_bytes());
    dst[s.len()] = 0;
}

fn tar_header(name: &str, size: u64, dir: bool) -> io::Result<[u8; 512]> {
    if name.len() > TAR_NAME_MAX || name.is_empty() || !name.is_ascii() {
        return Err(bad(format!("tar name not usable: {}", name)));
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    put_octal(&mut h[100..108], if dir { 0o755 } else { 0o644 });
    put_octal(&mut h[108..116], 0);
    put_octal(&mut h[116..124], 0);
    put_octal(&mut h[124..136], size);
    put_octal(&mut h[136..148], 0);
    h[148..156].fill(b' ');
    h[156] = if dir { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[265..269].copy_from_slice(b"root");
    h[297..301].copy_from_slice(b"root");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    let s = format!("{:06o}\0 ", sum);
    h[148..156].copy_from_slice(s.as_bytes());
    Ok(h)
}

/// `entries`: `(<name>-<ver>, desc)`. Sorted by dir name.
pub(crate) fn tar_db(entries: &[(String, String)]) -> io::Result<Vec<u8>> {
    let mut sorted: Vec<&(String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (dir, desc) in sorted {
        out.extend_from_slice(&tar_header(&format!("{}/", dir), 0, true)?);
        out.extend_from_slice(&tar_header(
            &format!("{}/desc", dir),
            desc.len() as u64,
            false,
        )?);
        out.extend_from_slice(desc.as_bytes());
        let pad = (512 - desc.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out.extend(std::iter::repeat_n(0u8, 1024));
    Ok(out)
}

fn field_str(h: &[u8]) -> &[u8] {
    let end = h.iter().position(|&b| b == 0).unwrap_or(h.len());
    &h[..end]
}

fn parse_octal(h: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(field_str(h)).ok()?.trim();
    if s.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(s, 8).ok()
}

/// Reads back a db this module wrote: `(<name>-<ver>, desc)` pairs.
/// Anything it does not understand is an error (the caller starts over).
pub(crate) fn read_db(bytes: &[u8]) -> io::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 512 <= bytes.len() {
        let h = &bytes[pos..pos + 512];
        if h.iter().all(|&b| b == 0) {
            return Ok(out);
        }
        let name = std::str::from_utf8(field_str(&h[..100])).map_err(|_| bad("db: bad name"))?;
        let size = parse_octal(&h[124..136]).ok_or_else(|| bad("db: bad size"))? as usize;
        let kind = h[156];
        let body = pos + 512;
        let end = body
            .checked_add(size)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| bad("db: truncated"))?;
        if (kind == b'0' || kind == 0)
            && let Some(dir) = name.strip_suffix("/desc")
        {
            let text = std::str::from_utf8(&bytes[body..end]).map_err(|_| bad("db: bad desc"))?;
            out.push((dir.to_string(), text.to_string()));
        }
        pos = body + size.div_ceil(512) * 512;
    }
    Err(bad("db: no end marker"))
}

// ── on disk ───────────────────────────────────────────────────────────

/// A real directory, ours, not writable by others; created 0755.
/// (`umask` is 077 in the helper, so the mode is set explicitly.)
fn ensure_dir(p: &Path, uid: u32) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o755).create(p)?;
    let md = fs::symlink_metadata(p)?;
    if !md.is_dir() {
        return Err(denied("repo path is not a plain directory"));
    }
    if md.uid() != uid || md.mode() & 0o022 != 0 {
        return Err(denied("repo dir must be ours and not group/world-writable"));
    }
    fs::set_permissions(p, fs::Permissions::from_mode(0o755))
}

/// Exclusive lock on the repo for the life of the returned file.
fn lock_repo(dir: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(".lock"))?;
    // SAFETY: plain syscall on a descriptor we own.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(f)
}

/// Writes `bytes` to `dest` via a temp file in the same dir, 0644.
fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| bad("bad file name"))?;
    let tmp = dest.with_file_name(format!(".{}.tmp", name));
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
        fs::rename(&tmp, dest)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Copies a staged package into the repo; returns `(size, sha256)` of
/// what was written.
fn copy_package(src: &Path, dest: &Path) -> io::Result<(u64, String)> {
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| bad("bad file name"))?;
    let tmp = dest.with_file_name(format!(".{}.tmp", name));
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut inp = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(src)?;
        if !inp.metadata()?.is_file() {
            return Err(bad("not a regular file"));
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
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
            h.update(&buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
        fs::rename(&tmp, dest)?;
        Ok((total, sha256::hex(&h.finish())))
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// `.pkg.tar[.zst|.xz|…]` part of a file name, validated.
fn archive_ext(file_name: &str) -> io::Result<&str> {
    let i = file_name
        .find(".pkg.tar")
        .ok_or_else(|| bad("not a *.pkg.tar.* file"))?;
    let ext = &file_name[i..];
    if ext.len() > 24
        || !ext
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '.' || c.is_ascii_digit())
    {
        return Err(bad("odd package archive suffix"));
    }
    Ok(ext)
}

/// A bare file name that is a package archive: what a db may point at.
fn plain_package_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.contains('/')
        && s.len() <= 200
        && archive_ext(s).is_ok()
}

/// `<name>.db -> <name>.db.tar`, like repo-add leaves it.
fn link_db(dir: &Path, name: &str) -> io::Result<()> {
    let link = dir.join(format!("{}.db", name));
    match fs::symlink_metadata(&link) {
        Ok(md) if md.file_type().is_symlink() => Ok(()),
        Ok(_) => {
            fs::remove_file(&link)?;
            std::os::unix::fs::symlink(format!("{}.db.tar", name), &link)
        }
        Err(_) => std::os::unix::fs::symlink(format!("{}.db.tar", name), &link),
    }
}

/// Copy of the db in libalpm's sync dir, so the repo is visible without
/// a `pacman -Sy` (skipped when that dir does not exist).
fn sync_copy(sync: &Path, name: &str, db: &[u8]) -> io::Result<()> {
    if fs::metadata(sync).is_ok_and(|m| m.is_dir()) {
        write_atomic(&sync.join(format!("{}.db", name)), db)?;
    }
    Ok(())
}

/// An empty repo (no packages yet). pacman.conf may list all three
/// repos before each has seen a package, and `pacman -Sy` fails on a
/// repo whose db file is missing. Never touches an existing db.
fn ensure_empty_db(root: &Path, sync: &Path, uid: u32, repo: LocalRepo) -> io::Result<()> {
    let name = repo.name();
    let dir = root.join(name);
    ensure_dir(&dir, uid)?;
    let _lock = lock_repo(&dir)?;
    let tar = dir.join(format!("{}.db.tar", name));
    if fs::symlink_metadata(&tar).is_ok() {
        // Still refresh the sync-dir copy when missing (e.g. after a
        // pacman -Scc of /var/lib/pacman/sync).
        let sync_db = sync.join(format!("{}.db", name));
        if fs::metadata(sync).is_ok_and(|m| m.is_dir()) && fs::symlink_metadata(&sync_db).is_err() {
            if let Ok(bytes) = fs::read(&tar) {
                let _ = sync_copy(sync, name, &bytes);
            }
        }
        return Ok(());
    }
    let empty = tar_db(&[])?;
    write_atomic(&tar, &empty)?;
    link_db(&dir, name)?;
    sync_copy(sync, name, &empty)
}

/// Create empty dbs for every local repo that has none yet. Idempotent.
/// Call before any `pacman -Sy` / helper sync so file:// servers never
/// 404 on a missing `ae-*.db`. Events: `repo <name> empty` when created.
pub(crate) fn ensure_all_in(
    root: &Path,
    sync: &Path,
    uid: u32,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    ensure_dir(root, uid)?;
    for repo in LocalRepo::ALL {
        let name = repo.name();
        let tar = root.join(name).join(format!("{}.db.tar", name));
        let existed = fs::symlink_metadata(&tar).is_ok();
        ensure_empty_db(root, sync, uid, repo)?;
        if !existed {
            emit(&format!("repo {} empty", name));
        }
    }
    Ok(())
}

/// Production entry: root-owned repos under `LOCAL_REPO_ROOT`.
pub(crate) fn ensure_all(emit: &mut dyn FnMut(&str)) -> io::Result<()> {
    ensure_all_in(Path::new(LOCAL_REPO_ROOT), Path::new(SYNC_DIR), 0, emit)
}

/// One package ready to publish.
struct Item {
    staged: PathBuf,
    info: PkgInfo,
    ext: String,
}

/// Publishes `staged` (already hash-checked copies) into `repo`.
/// `root` is the repos root, `sync` libalpm's sync dir (skipped when it
/// does not exist). Events: `repo <name> add|drop <file>`, `repo <name> db <n>`.
pub(crate) fn add_in(
    root: &Path,
    sync: &Path,
    uid: u32,
    repo: LocalRepo,
    staged: &[PathBuf],
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    if staged.is_empty() {
        return Err(bad("no packages"));
    }
    let mut items = Vec::with_capacity(staged.len());
    for s in staged {
        let fname = s
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| bad("bad file name"))?;
        let ext = archive_ext(fname)?.to_string();
        let info = parse_pkginfo(&read_pkginfo(s)?)?;
        items.push(Item {
            staged: s.clone(),
            info,
            ext,
        });
    }
    publish(root, sync, uid, repo, items, emit)
}

fn publish(
    root: &Path,
    sync: &Path,
    uid: u32,
    repo: LocalRepo,
    items: Vec<Item>,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let name = repo.name();
    let dir = root.join(name);
    ensure_dir(root, uid)?;
    for other in LocalRepo::ALL {
        if other != repo
            && let Err(e) = ensure_empty_db(root, sync, uid, other)
        {
            emit(&format!("repo {} empty db: {}", other.name(), e));
        }
    }
    ensure_dir(&dir, uid)?;
    let _lock = lock_repo(&dir)?;

    let db_tar = dir.join(format!("{}.db.tar", name));
    // pkgname -> (<name>-<ver>, desc)
    let mut entries: BTreeMap<String, (String, String)> = BTreeMap::new();
    match fs::symlink_metadata(&db_tar) {
        Ok(md) if md.is_file() && md.len() <= MAX_DB => {
            let parsed = fs::read(&db_tar).and_then(|b| read_db(&b));
            match parsed {
                Ok(list) => {
                    for (d, desc) in list {
                        if let Some(n) = desc_field(&desc, "NAME") {
                            entries.insert(n.to_string(), (d, desc));
                        }
                    }
                }
                Err(e) => emit(&format!(
                    "repo {} db unreadable, starting over: {}",
                    name, e
                )),
            }
        }
        _ => {}
    }

    let mut superseded: Vec<String> = Vec::new();
    for it in &items {
        let i = &it.info;
        let file = format!("{}-{}-{}{}", i.name, i.version, i.arch, it.ext);
        if !plain_package_name(&file) {
            return Err(bad(format!("package file name not usable: {}", file)));
        }
        let key = format!("{}-{}", i.name, i.version);
        if key.len() + "/desc".len() > TAR_NAME_MAX {
            return Err(bad(format!("name + version too long for the db: {}", key)));
        }
        let (size, sha) = copy_package(&it.staged, &dir.join(&file))?;
        let desc = make_desc(i, &file, size, &sha);
        if let Some((_, old)) = entries.get(&i.name)
            && let Some(old_file) = desc_field(old, "FILENAME")
            && old_file != file
        {
            superseded.push(old_file.to_string());
        }
        entries.insert(i.name.clone(), (key, desc));
        emit(&format!("repo {} add {}", name, file));
        // Keep CacheDir in sync so a later `pacman -S ae-*/name` does not
        // hit a stale checksum from a previous -U / download.
        let _ = ensure_pkg_cache(repo, &i.name, &i.version);
    }

    let list: Vec<(String, String)> = entries.into_values().collect();
    write_atomic(&db_tar, &tar_db(&list)?)?;
    link_db(&dir, name)?;
    sync_copy(sync, name, &fs::read(&db_tar)?)?;

    // Drop the files an update replaced. Only bare names from our own
    // db, only regular files inside the repo dir.
    for old in superseded {
        if !plain_package_name(&old) {
            continue;
        }
        let p = dir.join(&old);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) && fs::remove_file(&p).is_ok() {
            emit(&format!("repo {} drop {}", name, old));
        }
    }
    emit(&format!("repo {} db {}", name, list.len()));
    Ok(())
}

/// Helper entry: stage + hash-check `specs`, then publish into `repo`.
pub(crate) fn add(repo: LocalRepo, specs: &[String], emit: &mut dyn FnMut(&str)) -> io::Result<()> {
    // Sibling empty dbs first so a half-configured pacman.conf never
    // trips over a missing file:// server mid-publish.
    let _ = ensure_all(emit);
    let mut stage = Stage::create()?;
    let staged = stage.copy_specs(specs)?;
    add_in(
        Path::new(LOCAL_REPO_ROOT),
        Path::new(SYNC_DIR),
        0,
        repo,
        &staged,
        emit,
    )
}

/// Pacman package cache. libalpm prefers this over `file://` when a file
/// of the same name exists — a stale copy with a different SHA256 is the
/// usual cause of `invalid or corrupted package (checksum)` on `-S`
/// from an `ae-*` repo.
const PKG_CACHE: &str = "/var/cache/pacman/pkg";

/// Make sure `CacheDir` has the same bytes as the archive in the local
/// repo (overwrite or create). No-op if the cache dir is missing.
/// Call before `-S ae-*/name` so "Installed From" is set and checksums match.
pub(crate) fn ensure_pkg_cache(repo: LocalRepo, name: &str, ver: &str) -> io::Result<()> {
    let cache = Path::new(PKG_CACHE);
    if !cache.is_dir() {
        return Ok(());
    }
    let src = match find_archive(Path::new(repo.dir()), name, ver) {
        Some(p) => p,
        None => return Ok(()),
    };
    let fname = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| bad("bad file name"))?;
    if !plain_package_name(fname) {
        return Err(bad("package file name not usable"));
    }
    let dest = cache.join(fname);
    if let Ok(true) = same_content(&src, &dest) {
        return Ok(());
    }
    let _ = fs::remove_file(&dest);
    let (_size, _sha) = copy_package(&src, &dest)?;
    let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o644));
    Ok(())
}

fn find_archive(dir: &Path, name: &str, ver: &str) -> Option<PathBuf> {
    let prefix = format!("{}-{}-", name, ver);
    let mut hits = Vec::new();
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let s = e.file_name().to_string_lossy().into_owned();
        if s.starts_with(&prefix) && s.contains(".pkg.tar") && !s.starts_with('.') {
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                hits.push(e.path());
            }
        }
    }
    if hits.len() == 1 { hits.pop() } else { None }
}

fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
    let ma = fs::metadata(a)?;
    let mb = match fs::metadata(b) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let ha = {
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(a)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            h.update(&buf[..n]);
        }
        sha256::hex(&h.finish())
    };
    let hb = {
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(b)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            h.update(&buf[..n]);
        }
        sha256::hex(&h.finish())
    };
    Ok(ha == hb)
}

/// Drop packages by bare name from every local repo that still carries them.
/// Used after unmerge and by `--localrepo-clean`.
pub(crate) fn remove_pkgs(names: &[String], emit: &mut dyn FnMut(&str)) -> io::Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    remove_pkgs_in(
        Path::new(LOCAL_REPO_ROOT),
        Path::new(SYNC_DIR),
        0,
        names,
        emit,
    )
}

pub(crate) fn remove_pkgs_in(
    root: &Path,
    sync: &Path,
    uid: u32,
    names: &[String],
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let want: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    for repo in LocalRepo::ALL {
        remove_from_repo(root, sync, uid, repo, &want, emit)?;
    }
    Ok(())
}

fn remove_from_repo(
    root: &Path,
    sync: &Path,
    uid: u32,
    repo: LocalRepo,
    want: &std::collections::HashSet<&str>,
    emit: &mut dyn FnMut(&str),
) -> io::Result<()> {
    let name = repo.name();
    let dir = root.join(name);
    if fs::symlink_metadata(&dir).is_err() {
        return Ok(());
    }
    ensure_dir(&dir, uid)?;
    let _lock = lock_repo(&dir)?;
    let db_tar = dir.join(format!("{}.db.tar", name));
    let mut entries: BTreeMap<String, (String, String)> = BTreeMap::new();
    match fs::symlink_metadata(&db_tar) {
        Ok(md) if md.is_file() && md.len() <= MAX_DB => {
            if let Ok(list) = fs::read(&db_tar).and_then(|b| read_db(&b)) {
                for (d, desc) in list {
                    if let Some(n) = desc_field(&desc, "NAME") {
                        entries.insert(n.to_string(), (d, desc));
                    }
                }
            }
        }
        _ => return Ok(()),
    }
    let mut dropped = Vec::new();
    for pkg in want {
        if let Some((_, desc)) = entries.remove(*pkg) {
            if let Some(file) = desc_field(&desc, "FILENAME") {
                dropped.push(file.to_string());
            }
            emit(&format!("repo {} drop-pkg {}", name, pkg));
        }
    }
    if dropped.is_empty() {
        return Ok(());
    }
    let list: Vec<(String, String)> = entries.into_values().collect();
    write_atomic(&db_tar, &tar_db(&list)?)?;
    link_db(&dir, name)?;
    sync_copy(sync, name, &fs::read(&db_tar)?)?;
    for old in dropped {
        if !plain_package_name(&old) {
            continue;
        }
        let p = dir.join(&old);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) && fs::remove_file(&p).is_ok() {
            emit(&format!("repo {} drop {}", name, old));
        }
    }
    emit(&format!("repo {} db {}", name, list.len()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "# Generated by makepkg\n\
pkgname = foo\n\
pkgbase = foo\n\
pkgver = 1:2.0-3\n\
pkgdesc = A foo\n\
url = https://example.org/foo\n\
builddate = 1700000000\n\
packager = Unknown Packager\n\
size = 12345\n\
arch = x86_64\n\
license = MIT\n\
license = GPL\n\
depend = glibc\n\
depend = bar>=1.0\n\
optdepend = baz: for baz\n\
provides = libfoo.so=2-64\n\
makedepend = cmake\n";

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("ae-lrepo-{}-{}", std::process::id(), name));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
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

    fn item(dir: &Path, info: &str, ver_file: &str) -> Item {
        let staged = dir.join(format!("1-{}", ver_file));
        fs::write(&staged, format!("bytes of {}", ver_file)).unwrap();
        Item {
            staged,
            info: parse_pkginfo(info).unwrap(),
            ext: ".pkg.tar.zst".to_string(),
        }
    }

    #[test]
    fn ensure_all_creates_empty_dbs_once() {
        let t = Tmp::new("ensure");
        let root = t.0.join("repos");
        let sync = t.0.join("sync");
        fs::create_dir_all(&sync).unwrap();
        let mut ev = Vec::new();
        ensure_all_in(&root, &sync, me(), &mut |e| ev.push(e.to_string())).unwrap();
        for name in ["ae-aur", "ae-abs", "ae-local"] {
            let dir = root.join(name);
            assert!(dir.join(format!("{}.db.tar", name)).is_file());
            assert!(
                fs::symlink_metadata(dir.join(format!("{}.db", name)))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            let db = fs::read(dir.join(format!("{}.db.tar", name))).unwrap();
            assert!(read_db(&db).unwrap().is_empty());
            assert_eq!(fs::read(sync.join(format!("{}.db", name))).unwrap(), db);
            assert!(ev.iter().any(|e| e == &format!("repo {} empty", name)));
        }
        // Second call is a no-op (no new empty events).
        let mut ev2 = Vec::new();
        ensure_all_in(&root, &sync, me(), &mut |e| ev2.push(e.to_string())).unwrap();
        assert!(ev2.is_empty());
    }

    #[test]
    fn pkginfo_parses_and_rejects_odd_names() {
        let p = parse_pkginfo(INFO).unwrap();
        assert_eq!(p.name, "foo");
        assert_eq!(p.version, "1:2.0-3");
        assert_eq!(p.license, ["MIT", "GPL"]);
        assert_eq!(p.depends, ["glibc", "bar>=1.0"]);
        assert_eq!(p.isize, "12345");
        for bad_name in ["../x", "-x", ".x", "a b", "a/b", ""] {
            let t = INFO.replace("pkgname = foo", &format!("pkgname = {}", bad_name));
            assert!(parse_pkginfo(&t).is_err(), "{:?}", bad_name);
        }
        // Two pkgnames, a section-header lookalike, a control char.
        assert!(parse_pkginfo(&format!("{}pkgname = bar\n", INFO)).is_err());
        assert!(parse_pkginfo(&INFO.replace("depend = glibc", "depend = %NAME%")).is_err());
        assert!(parse_pkginfo(&INFO.replace("A foo", "A\u{1b}[2Jfoo")).is_err());
        // No arch, no version.
        assert!(parse_pkginfo(&INFO.replace("arch = x86_64\n", "")).is_err());
        assert!(parse_pkginfo(&INFO.replace("pkgver = 1:2.0-3\n", "")).is_err());
    }

    #[test]
    fn desc_has_the_sections_libalpm_reads() {
        let p = parse_pkginfo(INFO).unwrap();
        let d = make_desc(&p, "foo-1:2.0-3-x86_64.pkg.tar.zst", 99, &"a".repeat(64));
        assert_eq!(
            desc_field(&d, "FILENAME"),
            Some("foo-1:2.0-3-x86_64.pkg.tar.zst")
        );
        assert_eq!(desc_field(&d, "NAME"), Some("foo"));
        assert_eq!(desc_field(&d, "CSIZE"), Some("99"));
        assert!(d.contains("%DEPENDS%\nglibc\nbar>=1.0\n\n"));
        assert!(d.contains("%LICENSE%\nMIT\nGPL\n\n"));
        assert!(!d.contains("%REPLACES%"));
        assert!(!d.contains("%MD5SUM%"));
        assert!(d.ends_with("\n\n"));
    }

    #[test]
    fn tar_db_roundtrips_and_headers_check_out() {
        let e = vec![
            ("b-1-1".to_string(), "%NAME%\nb\n\n".to_string()),
            ("a-1-1".to_string(), "%NAME%\na\n\n".repeat(100)),
        ];
        let t = tar_db(&e).unwrap();
        assert_eq!(t.len() % 512, 0);
        let back = read_db(&t).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].0, "a-1-1"); // sorted
        assert_eq!(back[0].1, e[1].1);
        assert_eq!(back[1], e[0]);
        // Header checksum, recomputed the way readers do.
        let h = &t[..512];
        let stored = parse_octal(&h[148..156]).unwrap() as u32;
        let mut tmp = h.to_vec();
        tmp[148..156].fill(b' ');
        assert_eq!(stored, tmp.iter().map(|&b| b as u32).sum::<u32>());
        assert_eq!(&h[257..262], b"ustar");
        // Truncated / unterminated input is an error, not a panic.
        assert!(read_db(&t[..t.len() - 1024]).is_err());
        assert!(read_db(&t[..700]).is_err());
        assert!(tar_db(&[("x".repeat(120), String::new())]).is_err());
    }

    #[test]
    fn publish_writes_db_links_and_replaces_old_versions() {
        let t = Tmp::new("pub");
        let root = t.0.join("repos");
        let sync = t.0.join("sync");
        fs::create_dir_all(&sync).unwrap();
        let mut ev = Vec::new();

        let v1 = item(&t.0, INFO, "foo-1");
        publish(&root, &sync, me(), LocalRepo::Aur, vec![v1], &mut |e| {
            ev.push(e.to_string())
        })
        .unwrap();
        let dir = root.join("ae-aur");
        assert!(dir.join("foo-1:2.0-3-x86_64.pkg.tar.zst").is_file());
        assert!(
            fs::symlink_metadata(dir.join("ae-aur.db"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let db = fs::read(dir.join("ae-aur.db.tar")).unwrap();
        assert_eq!(read_db(&db).unwrap().len(), 1);
        assert_eq!(fs::read(sync.join("ae-aur.db")).unwrap(), db);
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o755);
        assert!(
            ev.iter()
                .any(|e| e == "repo ae-aur add foo-1:2.0-3-x86_64.pkg.tar.zst")
        );

        // Newer version of foo + a second package.
        let v2 = item(&t.0, &INFO.replace("1:2.0-3", "1:2.1-1"), "foo-2");
        let other = item(
            &t.0,
            &INFO
                .replace("pkgname = foo", "pkgname = bar")
                .replace("pkgbase = foo", "pkgbase = bar"),
            "bar-1",
        );
        publish(
            &root,
            &sync,
            me(),
            LocalRepo::Aur,
            vec![v2, other],
            &mut |e| ev.push(e.to_string()),
        )
        .unwrap();
        assert!(!dir.join("foo-1:2.0-3-x86_64.pkg.tar.zst").exists());
        assert!(dir.join("foo-1:2.1-1-x86_64.pkg.tar.zst").is_file());
        let db = read_db(&fs::read(dir.join("ae-aur.db.tar")).unwrap()).unwrap();
        let names: Vec<&str> = db.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(names, ["bar-1:2.0-3", "foo-1:2.1-1"]);
        assert!(
            ev.iter()
                .any(|e| e == "repo ae-aur drop foo-1:2.0-3-x86_64.pkg.tar.zst")
        );
        assert!(ev.last().unwrap().ends_with("db 2"));
        // The repos do not leak into each other; the others start empty so
        // a pacman.conf that lists all three can `-Sy` right away.
        let abs = fs::read(root.join("ae-abs/ae-abs.db.tar")).unwrap();
        assert!(read_db(&abs).unwrap().is_empty());
        assert!(sync.join("ae-abs.db").is_file());
        assert!(
            !root
                .join("ae-abs")
                .join("foo-1:2.1-1-x86_64.pkg.tar.zst")
                .exists()
        );
    }

    #[test]
    fn publish_refuses_a_foreign_or_open_repo_dir() {
        let t = Tmp::new("deny");
        let root = t.0.join("repos");
        fs::create_dir_all(root.join("ae-abs")).unwrap();
        fs::set_permissions(root.join("ae-abs"), fs::Permissions::from_mode(0o777)).unwrap();
        let v = item(&t.0, INFO, "foo-1");
        let r = publish(
            &root,
            &t.0.join("nosync"),
            me(),
            LocalRepo::Abs,
            vec![v],
            &mut |_| {},
        );
        assert!(r.is_err());
        // Wrong owner.
        let t2 = Tmp::new("deny2");
        let v = item(&t2.0, INFO, "foo-1");
        let r = publish(
            &t2.0.join("r"),
            &t2.0,
            me() + 1,
            LocalRepo::Abs,
            vec![v],
            &mut |_| {},
        );
        assert!(r.is_err());
    }

    #[test]
    fn archive_ext_and_plain_names() {
        assert_eq!(
            archive_ext("1-foo-1-1-any.pkg.tar.zst").unwrap(),
            ".pkg.tar.zst"
        );
        assert!(archive_ext("foo.zip").is_err());
        assert!(archive_ext("foo.pkg.tar.zst/../x").is_err());
        assert!(plain_package_name("foo-1-1-any.pkg.tar.zst"));
        assert!(!plain_package_name("../foo-1-1-any.pkg.tar.zst"));
        assert!(!plain_package_name(".lock"));
        assert!(!plain_package_name("foo.txt"));
    }
}
