//! Read-only libalpm. No commits / -U / helper (a prepare-only plan is the
//! one transaction allowed here).
//! Alpm is !Send — open handle per call.

use std::collections::HashMap;

use alpm::{Alpm, Package, PackageReason, PrepareData, SigLevel, TransFlag};
use alpm_utils::alpm_with_conf;
use alpm_utils::config::Config as PacmanConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlpmPkg {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) repo: String,
    pub(crate) description: String,
    pub(crate) installed: bool,
    /// Installed version from local db (empty if not installed).
    pub(crate) installed_version: String,
    /// Download size in bytes (sync package file size).
    pub(crate) size: i64,
    pub(crate) url: String,
    pub(crate) licenses: String,
    /// pkgbase (ABS clone name); equals `name` when not split.
    pub(crate) base: String,
    pub(crate) depends: String,
    pub(crate) makedepends: String,
    pub(crate) provides: String,
    pub(crate) conflicts: String,
    /// Unix build date (0 if unknown).
    pub(crate) build_date: i64,
}

/// User-side handle: read-only queries. Signature verification is
/// disabled here so libalpm never opens `/etc/pacman.d/gnupg` as a
/// non-root process (gpg: "unsafe ownership on homedir"). Real

fn open() -> Option<Alpm> {
    let conf = PacmanConfig::new().ok()?;
    let mut alpm = alpm_with_conf(&conf).ok()?;
    disable_user_sig_verify(&mut alpm);
    Some(alpm)
}

fn disable_user_sig_verify(alpm: &mut Alpm) {
    // alpm 5: SigLevel is set on the handle, not per-Db (DbMut has
    // siglevel() getter only). User reads never need verify; installs
    // go through the root helper which keeps pacman.conf levels.
    let none = SigLevel::NONE;
    let _ = alpm.set_default_siglevel(none);
    let _ = alpm.set_local_file_siglevel(none);
    let _ = alpm.set_remote_file_siglevel(none);
}

fn bare(name: &str) -> &str {
    name.split('/').next_back().unwrap_or(name)
}

fn join_deps<'a, I>(iter: I) -> String
where
    I: IntoIterator,
    I::Item: std::fmt::Display,
{
    iter.into_iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn pkg_to_info(pkg: &Package, repo: &str, installed: bool, installed_version: &str) -> AlpmPkg {
    let licenses = pkg
        .licenses()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    AlpmPkg {
        name: pkg.name().to_string(),
        version: pkg.version().to_string(),
        repo: repo.to_string(),
        description: pkg.desc().unwrap_or("").to_string(),
        installed,
        installed_version: installed_version.to_string(),
        size: pkg.size(),
        url: pkg.url().unwrap_or("").to_string(),
        licenses,
        base: pkg.base().unwrap_or_else(|| pkg.name()).to_string(),
        depends: join_deps(pkg.depends()),
        makedepends: join_deps(pkg.makedepends()),
        provides: join_deps(pkg.provides()),
        conflicts: join_deps(pkg.conflicts()),
        build_date: pkg.build_date(),
    }
}

fn local_ver(alpm: &Alpm, name: &str) -> (bool, String) {
    match alpm.localdb().pkg(name) {
        Ok(p) => (true, p.version().to_string()),
        Err(_) => (false, String::new()),
    }
}

/// Sync repo names in pacman.conf order (= priority).
pub(crate) fn sync_db_names() -> Vec<String> {
    open().map_or_else(Vec::new, |a| {
        a.syncdbs().iter().map(|d| d.name().to_string()).collect()
    })
}

pub(crate) fn is_installed(name: &str) -> bool {
    installed_version(name).is_some()
}

/// Provided name -> installed package that provides it (a package's own
/// name excluded). `aura-emerge-git` providing `aura-emerge` -> one entry.
pub(crate) fn installed_providers() -> HashMap<String, String> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let mut m = HashMap::new();
    for p in alpm.localdb().pkgs() {
        for d in p.provides() {
            let n = d.name().to_string();
            if n != p.name() {
                m.entry(n).or_insert_with(|| p.name().to_string());
            }
        }
    }
    m
}

pub(crate) fn installed_names() -> std::collections::HashSet<String> {
    let Some(alpm) = open() else {
        return std::collections::HashSet::new();
    };
    alpm.localdb()
        .pkgs()
        .iter()
        .map(|p| p.name().to_string())
        .collect()
}

/// name → version for every installed package (one alpm open).
pub(crate) fn installed_versions() -> HashMap<String, String> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    alpm.localdb()
        .pkgs()
        .iter()
        .map(|p| (p.name().to_string(), p.version().to_string()))
        .collect()
}

/// Local packages not in any sync db (`pacman -Qm`): name + version.
pub(crate) fn foreign_packages() -> Vec<(String, String)> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pkg in alpm.localdb().pkgs() {
        let name = pkg.name();
        let mut in_sync = false;
        for db in alpm.syncdbs() {
            if db.pkg(name).is_ok() {
                in_sync = true;
                break;
            }
        }
        if !in_sync {
            out.push((name.to_string(), pkg.version().to_string()));
        }
    }
    out
}

/// Official upgrades: (name, installed, available, repo).
pub(crate) fn upgradeable_detail() -> Vec<(String, String, String, String)> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pkg in alpm.localdb().pkgs() {
        let name = pkg.name();
        let local_ver = pkg.version().to_string();
        for db in alpm.syncdbs() {
            if let Ok(sp) = db.pkg(name) {
                // libalpm (sync_sysupgrade) looks only at the FIRST sync db
                // that carries the name; a newer copy in a later repo is
                // never used. Mirror that, or the plan shows upgrades the

                let new_ver = sp.version().to_string();
                if alpm::vercmp(local_ver.as_str(), new_ver.as_str()) == std::cmp::Ordering::Less {
                    out.push((name.to_string(), local_ver, new_ver, db.name().to_string()));
                }
                break;
            }
        }
    }
    out
}

pub(crate) fn installed_version(name: &str) -> Option<String> {
    let alpm = open()?;
    alpm.localdb()
        .pkg(bare(name))
        .ok()
        .map(|p| p.version().to_string())
}

/// Lookup sync packages. `repo/name` pins the repo; a miss in that repo
/// is a miss (no fallback to other dbs). Bare names take the first db
/// that has them (pacman.conf order).
pub(crate) fn find_sync_many(names: &[String]) -> HashMap<String, AlpmPkg> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for name in names {
        let (pinned, bare_name) = match name.split_once('/') {
            Some((r, n))
                if !r.is_empty()
                    && !n.is_empty()
                    && r != "aur"
                    && r != "abs"
                    && r != "Err"
                    && r.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
            {
                (Some(r), n)
            }
            _ => (None, bare(name)),
        };
        if let Some(repo) = pinned {
            if let Some(pkg) = sync_pkg(&alpm, Some(repo), bare_name) {
                // Key by bare so callers that strip prefixes still match;
                // the AlpmPkg carries the real repo name.
                let (inst, iver) = local_ver(&alpm, bare_name);
                out.insert(bare_name.to_string(), pkg_to_info(pkg, repo, inst, &iver));
            }
            continue;
        }
        if let Some(pkg) = sync_pkg(&alpm, None, bare_name) {
            let repo = pkg.db().map(|d| d.name()).unwrap_or("");
            let (inst, iver) = local_ver(&alpm, bare_name);
            out.insert(bare_name.to_string(), pkg_to_info(pkg, repo, inst, &iver));
        }
    }
    out
}

pub(crate) fn probe_sync_split(names: &[String]) -> (Vec<AlpmPkg>, Vec<String>) {
    let found_map = find_sync_many(names);
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for name in names {
        let bare = bare(name).to_string();
        match found_map.get(&bare) {
            Some(p) => {
                // If the caller pinned a repo, require the found pkg is
                // from that repo (find_sync_many already enforces this,
                // but re-check so a bare-name collision can't sneak in).
                if let Some((repo, _)) = name.split_once('/')
                    && repo != "aur"
                    && repo != "abs"
                    && repo != "Err"
                    && !repo.is_empty()
                    && p.repo != repo
                {
                    missing.push(name.clone());
                    continue;
                }
                found.push(p.clone());
            }
            None => missing.push(name.clone()),
        }
    }
    (found, missing)
}

/// First sync db (pacman.conf order) holding `name`; `repo` pins one.
fn sync_pkg<'a>(alpm: &'a Alpm, repo: Option<&str>, name: &str) -> Option<&'a Package> {
    alpm.syncdbs()
        .iter()
        .filter(|d| repo.is_none_or(|r| d.name() == r))
        .find_map(|d| d.pkg(name).ok())
}

/// Full install plan for `targets` (`[repo/]name`): the targets plus
/// whatever libalpm pulls in, deps first -- what `pacman -Sp` printed.
/// Prepare only, nothing is committed. NO_LOCK, so a normal user can

pub(crate) fn plan_sync(
    targets: &[String],
) -> Result<(Vec<AlpmPkg>, Vec<crate::conflict::Conflict>), String> {
    let mut alpm = open().ok_or_else(|| "cannot open libalpm".to_string())?;
    alpm.trans_init(TransFlag::NO_LOCK)
        .map_err(|e| format!("trans_init: {}", e))?;
    let res = plan_in(&mut alpm, targets);
    let _ = alpm.trans_release();
    res
}

/// What `trans_prepare` said, with everything copied out of the error
/// so the borrow on the handle ends before the handle is used again.
enum Prepared {
    Ready,
    Blocked(Vec<(String, String, String, String)>),
    Failed(String),
}

fn plan_in(
    alpm: &mut Alpm,
    targets: &[String],
) -> Result<(Vec<AlpmPkg>, Vec<crate::conflict::Conflict>), String> {
    for t in targets {
        let (repo, name) = match t.split_once('/') {
            Some((r, n)) => (Some(r), n),
            None => (None, t.as_str()),
        };
        let pkg = sync_pkg(alpm, repo, name).ok_or_else(|| format!("{}: not found", t))?;
        alpm.trans_add_pkg(pkg)
            .map_err(|e| format!("{}: {}", t, e))?;
    }
    let prepared = match alpm.trans_prepare() {
        Ok(_) => Prepared::Ready,
        Err(e) => match e.data() {
            Some(PrepareData::ConflictingDeps(list)) => Prepared::Blocked(
                list.iter()
                    .map(|c| {
                        let (a, b) = (c.package1(), c.package2());
                        (
                            a.name().to_string(),
                            a.version().to_string(),
                            b.name().to_string(),
                            b.version().to_string(),
                        )
                    })
                    .collect(),
            ),
            Some(PrepareData::UnsatisfiedDeps(list)) => {
                let top: Vec<String> = list
                    .iter()
                    .take(4)
                    .map(|d| format!("{} needs {}", d.target(), d.depend()))
                    .collect();
                if top.is_empty() {
                    Prepared::Failed(format!("prepare: {}", e))
                } else {
                    Prepared::Failed(format!("unsatisfied: {}", top.join(", ")))
                }
            }
            _ => Prepared::Failed(format!("prepare: {}", e)),
        },
    };
    let planned = |alpm: &Alpm| -> Vec<AlpmPkg> {
        alpm.trans_add()
            .iter()
            .map(|p| {
                let repo = p.db().map_or("", |d| d.name());
                pkg_to_info(p, repo, false, "")
            })
            .collect()
    };
    match prepared {
        // trans_add() is already dependency-sorted by prepare.
        Prepared::Ready => Ok((planned(alpm), Vec::new())),
        // libalpm resolved the dependencies before it found the blocker,
        // so the whole plan is still there -- just not sorted.
        Prepared::Blocked(pairs) => {
            let add: std::collections::HashSet<String> = alpm
                .trans_add()
                .iter()
                .map(|p| p.name().to_string())
                .collect();
            let blockers = pairs
                .into_iter()
                .map(|(n1, v1, n2, v2)| {
                    // `new` is the side that is part of the transaction.
                    let (new_name, new_ver, old_name, old_ver) =
                        if add.contains(&n1) || !add.contains(&n2) {
                            (n1, v1, n2, v2)
                        } else {
                            (n2, v2, n1, v1)
                        };
                    crate::conflict::Conflict {
                        new_name,
                        new_ver,
                        old_name,
                        old_ver,
                    }
                })
                .collect();
            Ok((planned(alpm), blockers))
        }
        Prepared::Failed(msg) => Err(msg),
    }
}

/// Installed packages that depend on `name` (libalpm's "Required By").
pub(crate) fn required_by(name: &str) -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let out: Vec<String> = match alpm.localdb().pkg(name) {
        Ok(p) => p.required_by().iter().map(|s| s.to_string()).collect(),
        Err(_) => Vec::new(),
    };
    out
}

// ── pre-flight conflicts for packages libalpm has no record of (AUR) ─────────

/// An incoming package as the AUR RPC, a .SRCINFO (ABS) or the sync db
/// describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Incoming {
    pub(crate) name: String,
    pub(crate) ver: String,
    pub(crate) provides: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

/// `foo>=1.2` -> ("foo", Some((">=", "1.2"))).
fn split_dep(s: &str) -> (&str, Option<(&str, &str)>) {
    for op in ["<=", ">=", "=", "<", ">"] {
        if let Some(i) = s.find(op) {
            return (&s[..i], Some((op, &s[i + op.len()..])));
        }
    }
    (s, None)
}

fn op_ok(op: &str, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        "<" => ord == Less,
        "<=" => ord != Greater,
        "=" => ord == Equal,
        ">=" => ord != Less,
        ">" => ord == Greater,
        _ => false,
    }
}

/// libalpm's rule: does `dep` match the package `name ver` or one of its
/// `provides` (a versioned dep needs a versioned `foo=1` provide)?
fn dep_matches(dep: &str, name: &str, ver: &str, provides: &[String]) -> bool {
    let (dn, c) = split_dep(dep.trim());
    if dn.is_empty() {
        return false;
    }
    if dn == name {
        return match c {
            None => true,
            Some((op, v)) => op_ok(op, alpm::vercmp(ver, v)),
        };
    }
    for p in provides {
        let (pn, pc) = split_dep(p.trim());
        if pn != dn {
            continue;
        }
        match (c, pc) {
            (None, _) => return true,
            (Some((op, v)), Some(("=", pv))) if op_ok(op, alpm::vercmp(pv, v)) => return true,
            _ => {}
        }
    }
    false
}

/// Blockers between `incoming` and the installed system (both ways: the
/// new package's `conflicts` and the installed ones'), plus between the
/// incoming packages themselves. Installed packages that are themselves
/// incoming (upgrades) are not blockers.
pub(crate) fn preflight_conflicts(incoming: &[Incoming]) -> Vec<crate::conflict::Conflict> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut out: Vec<crate::conflict::Conflict> = Vec::new();
    let mut push = |c: crate::conflict::Conflict| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    let names: Vec<&str> = incoming.iter().map(|i| i.name.as_str()).collect();
    for p in alpm.localdb().pkgs() {
        if names.contains(&p.name()) {
            continue;
        }
        let lprov: Vec<String> = p.provides().iter().map(|d| d.to_string()).collect();
        let lconf: Vec<String> = p.conflicts().iter().map(|d| d.to_string()).collect();
        for i in incoming {
            let hit = i
                .conflicts
                .iter()
                .any(|c| dep_matches(c, p.name(), p.version().as_str(), &lprov))
                || lconf
                    .iter()
                    .any(|c| dep_matches(c, &i.name, &i.ver, &i.provides));
            if hit {
                push(crate::conflict::Conflict {
                    new_name: i.name.clone(),
                    new_ver: i.ver.clone(),
                    old_name: p.name().to_string(),
                    old_ver: p.version().to_string(),
                });
            }
        }
    }
    for c in incoming_conflicts(incoming) {
        push(c);
    }
    out
}

/// Blockers among the incoming packages themselves (`i` declares a
/// conflict that `j` matches). Both directions are listed when both
/// declare it; `conflict::plan_with` folds those into one blocker.
pub(crate) fn incoming_conflicts(incoming: &[Incoming]) -> Vec<crate::conflict::Conflict> {
    let mut out: Vec<crate::conflict::Conflict> = Vec::new();
    for i in incoming {
        for j in incoming {
            if i.name != j.name
                && i.conflicts
                    .iter()
                    .any(|c| dep_matches(c, &j.name, &j.ver, &j.provides))
            {
                let c = crate::conflict::Conflict {
                    new_name: i.name.clone(),
                    new_ver: i.ver.clone(),
                    old_name: j.name.clone(),
                    old_ver: j.ver.clone(),
                };
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// A package libalpm planned (sync db) as an `Incoming`, so repo, AUR
/// and ABS packages of one run can be checked against each other.
pub(crate) fn incoming_from(p: &AlpmPkg) -> Incoming {
    let list = |s: &str| -> Vec<String> { s.split_whitespace().map(String::from).collect() };
    Incoming {
        name: p.name.clone(),
        ver: p.version.clone(),
        provides: list(&p.provides),
        conflicts: list(&p.conflicts),
    }
}

pub(crate) fn pkg_status(name: &str, new_ver: &str) -> String {
    match installed_version(name) {
        None => "N".to_string(),
        Some(installed) if installed == new_ver => "R".to_string(),
        Some(installed) => match alpm::vercmp(installed.as_str(), new_ver) {
            std::cmp::Ordering::Less => "U".to_string(),
            std::cmp::Ordering::Equal => "R".to_string(),
            std::cmp::Ordering::Greater => "D".to_string(),
        },
    }
}

/// bare → Some(repo) | Some("None") for local builds | key absent.
pub(crate) fn repos_batch(names: &[String]) -> HashMap<String, Option<String>> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    let local = alpm.localdb();
    for name in names {
        let bare = bare(name).to_string();
        if let Ok(pkg) = local.pkg(bare.as_str()) {
            let ver = pkg.version().to_string();
            let mut hits = Vec::new();
            for db in alpm.syncdbs() {
                if let Ok(sp) = db.pkg(bare.as_str())
                    && sp.version().as_str() == ver
                {
                    hits.push(db.name().to_string());
                }
            }
            let repo = match hits.len() {
                1 => Some(hits.remove(0)),
                _ => Some("None".to_string()),
            };
            result.insert(bare, repo);
        } else {
            for db in alpm.syncdbs() {
                if db.pkg(bare.as_str()).is_ok() {
                    result.insert(bare.clone(), Some(db.name().to_string()));
                    break;
                }
            }
        }
    }
    result
}

// ── local inventory (prune / depclean) ───────────────────────────────────────

/// Explicitly installed names (`pacman -Qeq`).
pub(crate) fn explicit_names() -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    alpm.localdb()
        .pkgs()
        .iter()
        .filter(|p| p.reason() == PackageReason::Explicit)
        .map(|p| p.name().to_string())
        .collect()
}

pub(crate) fn explicit_set() -> std::collections::HashSet<String> {
    explicit_names().into_iter().collect()
}

/// Orphans: asdeps, nothing requires them, nothing lists them as optdepend.
/// Same idea as `pacman -Qttdq`.
pub(crate) fn orphan_names() -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    alpm.localdb()
        .pkgs()
        .iter()
        .filter(|p| p.reason() == PackageReason::Depend)
        .filter(|p| p.required_by().is_empty() && p.optional_for().is_empty())
        .map(|p| p.name().to_string())
        .collect()
}

/// Full depclean set: orphans plus deps freed by removing them
/// (fixed point). `keep` is never touched, and whatever it needs stays.
pub(crate) fn orphan_closure(keep: &std::collections::HashSet<String>) -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let cand: Vec<_> = alpm
        .localdb()
        .pkgs()
        .iter()
        .filter(|p| p.reason() == PackageReason::Depend && !keep.contains(p.name()))
        .collect();
    let mut gone: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut order: Vec<String> = Vec::new();
    loop {
        let mut grew = false;
        for p in &cand {
            if gone.contains(p.name()) {
                continue;
            }
            let req_free = p.required_by().iter().all(|n| gone.contains(n));
            let opt_free = p.optional_for().iter().all(|n| gone.contains(n));
            if req_free && opt_free {
                gone.insert(p.name().to_string());
                order.push(p.name().to_string());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    order
}

/// Missing optional deps of installed packages that exist in a sync db.
pub(crate) fn missing_optdeps(names: &[String]) -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let Ok(pkg) = alpm.localdb().pkg(bare(name)) else {
            continue;
        };
        for dep in pkg.optdepends() {
            let dn = dep.name().to_string();
            let have = alpm.localdb().pkgs().find_satisfier(dn.as_str()).is_some();
            let in_sync = alpm.syncdbs().iter().any(|d| d.pkg(dn.as_str()).is_ok());
            if !have && in_sync && !out.contains(&dn) {
                out.push(dn);
            }
        }
    }
    out
}

// ── search / depends / -T ────────────────────────────────────────────────────

/// Sync-db search (`pacman -Ss`).
/// `all_repos`: false = one row per package name (first repo in conf order);

pub(crate) fn search_sync(term: &str, _in_desc: bool, all_repos: bool) -> Vec<AlpmPkg> {
    if term.is_empty() {
        return Vec::new();
    }
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let local = alpm.localdb();
    let mut out = Vec::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut seen_repo_name = std::collections::HashSet::new();
    for db in alpm.syncdbs() {
        let Ok(pkgs) = db.search([term].iter().copied()) else {
            continue;
        };
        let repo = db.name();
        for pkg in pkgs {
            let name = pkg.name().to_string();
            if all_repos {
                if !seen_repo_name.insert((repo.to_string(), name.clone())) {
                    continue;
                }
            } else if !seen_names.insert(name.clone()) {
                continue;
            }
            let (installed, iver) = match local.pkg(name.as_str()) {
                Ok(lp) => (true, lp.version().to_string()),
                Err(_) => (false, String::new()),
            };
            out.push(pkg_to_info(pkg, repo, installed, &iver));
        }
    }
    out
}

/// Arch repos whose sources live in the ABS GitLab (what `--abs` can clone).
const ABS_REPOS: [&str; 3] = ["core", "extra", "multilib"];

/// ABS catalog search: the Arch repos above only, one row per name.
pub(crate) fn search_abs(term: &str) -> Vec<AlpmPkg> {
    let mut seen = std::collections::HashSet::new();
    search_sync(term, false, true)
        .into_iter()
        .filter(|p| ABS_REPOS.contains(&p.repo.as_str()) && seen.insert(p.name.clone()))
        .collect()
}

/// Exact sync lookup by bare name.
pub(crate) fn find_sync(name: &str) -> Option<AlpmPkg> {
    let alpm = open()?;
    let bare = bare(name);
    for db in alpm.syncdbs() {
        if let Ok(pkg) = db.pkg(bare) {
            let (inst, iver) = local_ver(&alpm, bare);
            return Some(pkg_to_info(pkg, db.name(), inst, &iver));
        }
    }
    None
}

/// Depends On for each name (sync first, then local). Bare dep names.
pub(crate) fn depends_map(names: &[String]) -> HashMap<String, std::collections::HashSet<String>> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for name in names {
        let bare = bare(name);
        let pkg = {
            let mut found = None;
            for db in alpm.syncdbs() {
                if let Ok(p) = db.pkg(bare) {
                    found = Some(p);
                    break;
                }
            }
            found.or_else(|| alpm.localdb().pkg(bare).ok())
        };
        let Some(pkg) = pkg else {
            continue;
        };
        let mut deps = std::collections::HashSet::new();
        for dep in pkg.depends() {
            // Dep::name() is the atom without operators in alpm 5
            deps.insert(dep.name().to_string());
        }
        map.insert(bare.to_string(), deps);
    }
    map
}

/// Unsatisfied dependency atoms (`pacman -T`).
pub(crate) fn unsatisfied(deps: &[String]) -> Vec<String> {
    let Some(alpm) = open() else {
        return deps.to_vec();
    };
    let mut missing = Vec::new();
    for dep in deps {
        // Prefer local satisfier (installed provides/version).
        let ok = alpm.localdb().pkgs().find_satisfier(dep.as_str()).is_some();
        if !ok {
            missing.push(dep.clone());
        }
    }
    missing
}

/// True when `dep` (may be versioned, e.g. `foo>=1.2`) is satisfied by
/// the local db **or** any sync db via libalpm's provider/version logic.
/// Replaces ad-hoc name-only lookups for "is this available without AUR?".
pub(crate) fn satisfies_without_aur(dep: &str) -> bool {
    let Some(alpm) = open() else {
        return false;
    };
    if alpm.localdb().pkgs().find_satisfier(dep).is_some() {
        return true;
    }
    // Sync dbs: exact name, then provides (version constraints included
    // when the atom carries them — same path pacman uses).
    if alpm.syncdbs().find_satisfier(dep).is_some() {
        return true;
    }
    false
}

// ── replacements for `pacman -Qi/-Ql/-Qo/-Ssq` output parsing ────────────────

/// Every dependency name of every installed package (what `pacman -Qi`
/// shows under "Depends On", versions already stripped).
pub(crate) fn all_depends() -> std::collections::HashSet<String> {
    let Some(alpm) = open() else {
        return std::collections::HashSet::new();
    };
    let mut out = std::collections::HashSet::new();
    for pkg in alpm.localdb().pkgs() {
        for dep in pkg.depends() {
            out.insert(dep.name().to_string());
        }
    }
    out
}

/// Absolute paths owned by installed packages (`pacman -Qlq`), directories
/// skipped, narrowed by `keep` while walking so the full list is never built.
pub(crate) fn owned_paths(keep: impl Fn(&str) -> bool) -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for pkg in alpm.localdb().pkgs() {
        let files = pkg.files();
        for f in files.files() {
            // alpm 5: File::name() is raw bytes.
            let name = f.name();
            if name.last() == Some(&b'/') {
                continue;
            }
            let abs = format!("/{}", String::from_utf8_lossy(name));
            if keep(&abs) {
                out.push(abs);
            }
        }
    }
    out
}

/// path -> owning package (`pacman -Qo`), exact path match. Paths nobody
/// owns are simply absent from the map.
pub(crate) fn owners_of(paths: &[String]) -> HashMap<String, String> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let want: HashMap<&[u8], &String> = paths
        .iter()
        .map(|p| (p.trim_start_matches('/').as_bytes(), p))
        .collect();
    let mut out = HashMap::new();
    for pkg in alpm.localdb().pkgs() {
        let files = pkg.files();
        for f in files.files() {
            if let Some(orig) = want.get(f.name()) {
                out.insert((*orig).clone(), pkg.name().to_string());
            }
        }
    }
    out
}

/// Sync packages that satisfy each dependency atom, by exact name first,
/// then by `provides`. Returns (package names, atoms nobody provides).
pub(crate) fn sync_providers(deps: &[String]) -> (Vec<String>, Vec<String>) {
    let Some(alpm) = open() else {
        return (Vec::new(), deps.to_vec());
    };
    let mut found: Vec<String> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for dep in deps {
        let want = dep.as_str();
        let mut hit: Option<String> = None;
        for db in alpm.syncdbs() {
            if let Ok(p) = db.pkg(want) {
                hit = Some(p.name().to_string());
                break;
            }
        }
        if hit.is_none() {
            'scan: for db in alpm.syncdbs() {
                for p in db.pkgs() {
                    if p.provides().iter().any(|d| d.name() == want) {
                        hit = Some(p.name().to_string());
                        break 'scan;
                    }
                }
            }
        }
        match hit {
            Some(h) => {
                if !found.contains(&h) {
                    found.push(h);
                }
            }
            None => unresolved.push(dep.clone()),
        }
    }
    (found, unresolved)
}

/// All sync package names, deduped, in pacman.conf order (`pacman -Ssq`).
pub(crate) fn sync_pkg_names() -> Vec<String> {
    let Some(alpm) = open() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for db in alpm.syncdbs() {
        for p in db.pkgs() {
            if seen.insert(p.name().to_string()) {
                out.push(p.name().to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_strips() {
        assert_eq!(bare("extra/nano"), "nano");
    }

    fn inc(name: &str, ver: &str, provides: &[&str], conflicts: &[&str]) -> Incoming {
        Incoming {
            name: name.into(),
            ver: ver.into(),
            provides: provides.iter().map(|s| s.to_string()).collect(),
            conflicts: conflicts.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn incoming_conflict_through_provides() {
        let a = inc("pipewire-jack", "1-1", &["jack"], &["jack"]);
        let b = inc("jack2-git", "2-1", &["jack"], &["jack"]);
        let c = inc("unrelated", "1-1", &[], &[]);
        let out = incoming_conflicts(&[a, b, c]);
        // both declare it, both directions are reported
        assert_eq!(out.len(), 2);
        assert!(
            out.iter()
                .all(|x| x.new_name != "unrelated" && x.old_name != "unrelated")
        );
    }

    #[test]
    fn incoming_does_not_conflict_with_itself() {
        let a = inc("foo-git", "1-1", &["foo"], &["foo"]);
        assert!(incoming_conflicts(&[a]).is_empty());
    }

    #[test]
    fn versioned_incoming_conflict_respects_the_version() {
        let old = inc("lib", "1.0-1", &[], &[]);
        let new = inc("app", "1-1", &[], &["lib<2"]);
        assert_eq!(incoming_conflicts(&[new.clone(), old]).len(), 1);
        let fixed = inc("lib", "2.1-1", &[], &[]);
        assert!(incoming_conflicts(&[new, fixed]).is_empty());
    }

    #[test]
    fn planned_package_becomes_incoming() {
        let p = AlpmPkg {
            name: "x".into(),
            version: "1-1".into(),
            repo: "extra".into(),
            description: String::new(),
            installed: false,
            installed_version: String::new(),
            size: 0,
            url: String::new(),
            licenses: String::new(),
            base: "x".into(),
            depends: String::new(),
            makedepends: String::new(),
            provides: "a b=1".into(),
            conflicts: "c".into(),
            build_date: 0,
        };
        let i = incoming_from(&p);
        assert_eq!(i.provides, vec!["a", "b=1"]);
        assert_eq!(i.conflicts, vec!["c"]);
    }

    #[test]
    fn vercmp_ordering() {
        assert_eq!(alpm::vercmp("1.0-1", "1.0-1"), std::cmp::Ordering::Equal);
        assert_eq!(alpm::vercmp("1.0-1", "1.0-2"), std::cmp::Ordering::Less);
        assert_eq!(alpm::vercmp("2.0-1", "1.9-1"), std::cmp::Ordering::Greater);
    }
}
