//! Portage-style profiles via `/etc/portage/make.profile` (symlink).
//!
//! Layout (same as Gentoo):
//! ```text
//! /etc/portage/make.profile -> …/profiles/default/linux/amd64/systemd
//! …/profiles/base/packages           *bash *coreutils …
//! …/profiles/base/package.mask
//! …/profiles/base/package.protect
//! …/profiles/default/linux/parent   ../base
//! …/profiles/.../make.defaults      ARCH=amd64
//! ```
//!
//! For cynosure: a leaf profile can `package.mask` rival init systems
//! (e.g. mask `systemd` on an openrc profile) while `packages` lists the
//! minimal `@system` set to bootstrap.
//!
//! Profile `package.mask` supports `-atom` / `!atom` to drop a mask
//! inherited from a parent profile (PMS).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::mask::{glob_match, valid_pattern};
use crate::theme::Themed;
use colored::Colorize;

pub(crate) const MAKE_PROFILE: &str = "/etc/portage/make.profile";

/// Profile package set (`*pkg` = @system). Gentoo name is `packages`;
/// `package.profile` is still accepted as a legacy AE name.
const PROFILE_PKG_FILE: &str = "packages";
const PROFILE_PKG_LEGACY: &str = "package.profile";

/// Profile tree root (`eselect profile` style). Cynosure/portage layout only.
pub(crate) const PROFILE_ROOTS: &[&str] = &["/var/db/repos/portage/profiles"];

#[derive(Debug, Default)]
pub(crate) struct Profile {
    /// Resolved directories in inheritance order (parents first, leaf last).
    pub(crate) chain: Vec<PathBuf>,
    /// Bare names from `packages` lines starting with `*` (`@system`).
    pub(crate) system: Vec<String>,
    /// All `packages` entries (system + profile set); protected like world.
    pub(crate) packages: Vec<String>,
    /// Flattened profile `package.mask` entries (after `-atom` / `!atom` removals).
    pub(crate) masks: Vec<(String, Option<String>)>, // (pattern, repo)
    /// Key vars from make.defaults (ARCH, …).
    pub(crate) defaults: Vec<(String, String)>,
}

static PROFILE: OnceLock<Option<Profile>> = OnceLock::new();

/// Loaded once; `None` if no make.profile symlink / unreadable.
pub(crate) fn current() -> Option<&'static Profile> {
    PROFILE.get_or_init(load).as_ref()
}

fn load() -> Option<Profile> {
    let link = Path::new(MAKE_PROFILE);
    let target = match std::fs::read_link(link) {
        Ok(t) => {
            if t.is_absolute() {
                t
            } else {
                link.parent().unwrap_or(Path::new("/etc/portage")).join(t)
            }
        }
        Err(_) => {
            // Real dir here = leftover user config; --set-profile removes it.
            if link.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false) {
                eprintln!(
                    "{} {} is a user directory, not a profile symlink (run --set-profile to replace it)",
                    ">>> Warning:".t_yellow().bold(),
                    MAKE_PROFILE
                );
            }
            return None;
        }
    };
    let target = canonicalize_nofollow(&target).unwrap_or(target);
    if !target.is_dir() {
        eprintln!(
            "{} {} does not resolve to a profile directory",
            ">>> Warning:".t_yellow().bold(),
            MAKE_PROFILE
        );
        return None;
    }

    let chain = match expand_parents(&target) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} profile parent chain: {}",
                ">>> Warning:".t_yellow().bold(),
                e
            );
            vec![target]
        }
    };

    let mut system = Vec::new();
    let mut packages = Vec::new();
    let mut masks = Vec::new();
    let mut defaults = Vec::new();
    let mut seen_sys = HashSet::new();
    let mut seen_pkg = HashSet::new();

    for dir in &chain {
        load_packages(
            dir,
            &mut system,
            &mut packages,
            &mut seen_sys,
            &mut seen_pkg,
        );
        load_mask(dir, &mut masks);
        load_defaults(dir, &mut defaults);
    }

    Some(Profile {
        chain,
        system,
        packages,
        masks,
        defaults,
    })
}

fn canonicalize_nofollow(p: &Path) -> Option<PathBuf> {
    // Don't require full canonicalize (may fail on missing intermediate).
    std::fs::canonicalize(p).ok()
}

/// Depth-first left-to-right parent expansion (PMS).
fn expand_parents(leaf: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    expand_rec(leaf, &mut out, &mut seen, 0)?;
    Ok(out)
}

fn expand_rec(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err("profile parent depth > 64".into());
    }
    let key = dir.to_string_lossy().to_string();
    if !seen.insert(key) {
        return Err(format!("cycle involving {}", dir.display()));
    }
    let parent_file = dir.join("parent");
    if parent_file.is_file() {
        let text = crate::read_to_string_nofollow(&parent_file)
            .map_err(|e| format!("{}: {}", parent_file.display(), e))?;
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let p = if let Some((repo, path)) = t.split_once(':') {
                resolve_repo_profile(repo, path).unwrap_or_else(|| dir.join(path))
            } else {
                dir.join(t)
            };
            let p = canonicalize_nofollow(&p).unwrap_or(p);
            expand_rec(&p, out, seen, depth + 1)?;
        }
    }
    if !out.iter().any(|d| d == dir) {
        out.push(dir.to_path_buf());
    }
    Ok(())
}

fn resolve_repo_profile(repo: &str, path: &str) -> Option<PathBuf> {
    // gentoo:default/linux/amd64 → /var/db/repos/gentoo/profiles/default/linux/amd64
    let cand = PathBuf::from(format!("/var/db/repos/{}/profiles/{}", repo, path));
    if cand.is_dir() {
        return Some(cand);
    }
    for root in PROFILE_ROOTS {
        if root.contains(repo) {
            let p = Path::new(root).join(path);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

fn load_packages(
    dir: &Path,
    system: &mut Vec<String>,
    packages: &mut Vec<String>,
    seen_sys: &mut HashSet<String>,
    seen_pkg: &mut HashSet<String>,
) {
    // Prefer Gentoo `packages`; fall back to legacy AE `package.profile`.
    let mut path = dir.join(PROFILE_PKG_FILE);
    if !path.is_file() {
        path = dir.join(PROFILE_PKG_LEGACY);
    }
    let Ok(text) = crate::read_to_string_nofollow(&path) else {
        return;
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // `-*` discards parent system set (PMS).
        if t == "-*" {
            system.clear();
            seen_sys.clear();
            continue;
        }
        let (sys, atom) = if let Some(rest) = t.strip_prefix('*') {
            (true, rest.trim())
        } else if let Some(rest) = t.strip_prefix('-') {
            // removal of a package from the set
            let bare = rest.trim().split('/').next_back().unwrap_or(rest.trim());
            system.retain(|n| n != bare);
            packages.retain(|n| n != bare);
            seen_sys.remove(bare);
            seen_pkg.remove(bare);
            continue;
        } else {
            (false, t)
        };
        if atom.is_empty() {
            continue;
        }
        let bare = atom.split('/').next_back().unwrap_or(atom).to_string();
        if seen_pkg.insert(bare.clone()) {
            packages.push(bare.clone());
        }
        if sys && seen_sys.insert(bare.clone()) {
            system.push(bare);
        }
    }
}

fn load_mask(dir: &Path, masks: &mut Vec<(String, Option<String>)>) {
    let path = dir.join("package.mask");
    let Ok(text) = crate::read_to_string_nofollow(&path) else {
        return;
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let atom = t.split('#').next().unwrap_or(t).trim();
        if atom.is_empty() {
            continue;
        }
        // PMS: `-atom` removes a mask inherited from a parent profile.
        // `!atom` accepted as an AE alias (same effect).
        let (remove, atom) = if let Some(rest) = atom.strip_prefix('-') {
            (true, rest.trim())
        } else if let Some(rest) = atom.strip_prefix('!') {
            (true, rest.trim())
        } else {
            (false, atom)
        };
        if atom.is_empty() {
            continue;
        }
        let (repo, pattern) = match atom.split_once('/') {
            Some((r, n)) if !n.is_empty() && valid_pattern(n) => {
                (Some(r.to_string()), n.to_string())
            }
            _ if valid_pattern(atom) => (None, atom.to_string()),
            _ => continue,
        };
        if remove {
            masks.retain(|(p, r)| p != &pattern || r != &repo);
        } else {
            masks.push((pattern, repo));
        }
    }
}

fn load_defaults(dir: &Path, defaults: &mut Vec<(String, String)>) {
    let path = dir.join("make.defaults");
    let Ok(text) = crate::read_to_string_nofollow(&path) else {
        return;
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        if k.is_empty() {
            continue;
        }
        // Later profiles override earlier for the same key.
        if let Some(slot) = defaults.iter_mut().find(|(key, _)| key == k) {
            slot.1 = v.to_string();
        } else {
            defaults.push((k.to_string(), v.to_string()));
        }
    }
}

/// True if the active profile masks this package (before user package.mask).
pub(crate) fn is_masked(name: &str, repo: Option<&str>) -> bool {
    let Some(p) = current() else {
        return false;
    };
    let bare = name.split('/').next_back().unwrap_or(name);
    p.masks.iter().any(|(pat, mrepo)| {
        let repo_ok = match (mrepo.as_deref(), repo) {
            (None, _) => true,
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
        };
        repo_ok && glob_match(pat, bare)
    })
}

/// Bare `@system` package names (protected like world).
pub(crate) fn system_names() -> HashSet<String> {
    current()
        .map(|p| p.system.iter().cloned().collect())
        .unwrap_or_default()
}

/// Bare names of every profile `packages` entry (protected like world).
pub(crate) fn package_names() -> HashSet<String> {
    current()
        .map(|p| p.packages.iter().cloned().collect())
        .unwrap_or_default()
}

/// Dirs of the active profile chain (parents first) -- `package.protect` lookup.
pub(crate) fn chain_dirs() -> Vec<PathBuf> {
    current().map(|p| p.chain.clone()).unwrap_or_default()
}

/// Human-readable path of the leaf profile.
pub(crate) fn show_path() -> Option<String> {
    current().and_then(|p| p.chain.last().map(|d| d.display().to_string()))
}

/// Subdirectories of `dir` (dot-dirs skipped).
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            out.push(e.path());
        }
    }
    out
}

/// Profile files; filters `desc/`, `updates/` and similar leaf dirs.
fn has_profile_files(dir: &Path) -> bool {
    dir.join(PROFILE_PKG_FILE).is_file()
        || dir.join(PROFILE_PKG_LEGACY).is_file()
        || dir.join("make.defaults").is_file()
        || dir.join("parent").is_file()
}

/// Profiles = last directories (leaves) of the tree under each root.
/// Returns (root, relative path).
pub(crate) fn list_profiles() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for root in PROFILE_ROOTS {
        let root_p = Path::new(root);
        if !root_p.is_dir() {
            continue;
        }
        let mut q = VecDeque::from([root_p.to_path_buf()]);
        while let Some(dir) = q.pop_front() {
            let kids = subdirs(&dir);
            if kids.is_empty() {
                if has_profile_files(&dir)
                    && let Ok(rel) = dir.strip_prefix(root_p)
                {
                    let s = rel.display().to_string();
                    if !s.is_empty() {
                        out.push((root.to_string(), s));
                    }
                }
            } else {
                q.extend(kids);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// True if `target` is one of the leaf profiles from `list_profiles`.
fn is_listed_leaf(target: &Path) -> bool {
    let t = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    list_profiles().into_iter().any(|(root, rel)| {
        let full = PathBuf::from(root).join(rel);
        std::fs::canonicalize(&full).unwrap_or(full) == t
    })
}

/// Numbered list for `eselect profile list` / `--list-profiles`.
/// Returns (1-based index, root, relative path, is_current).
pub(crate) fn list_profiles_numbered() -> Vec<(usize, String, String, bool)> {
    let current = show_path().and_then(|p| {
        let pb = PathBuf::from(&p);
        std::fs::canonicalize(&pb).ok().or(Some(pb))
    });
    let mut out = Vec::new();
    for (i, (root, rel)) in list_profiles().into_iter().enumerate() {
        let full = PathBuf::from(&root).join(&rel);
        let full_c = std::fs::canonicalize(&full).unwrap_or_else(|_| full.clone());
        let is_cur = current.as_ref().map(|c| c == &full_c).unwrap_or(false);
        out.push((i + 1, root, rel, is_cur));
    }
    out
}

/// Print eselect-style table (stdout).
pub(crate) fn print_list() {
    let rows = list_profiles_numbered();
    if rows.is_empty() {
        println!("No profiles found under:\n  {}", PROFILE_ROOTS.join("\n  "));
        return;
    }
    for (n, _root, rel, cur) in &rows {
        let mark = if *cur { " * " } else { "   " };
        println!("{:>4}{}{}", n, mark, rel);
    }
    if let Some(p) = show_path() {
        println!();
        println!("Current profile symlink: {}", MAKE_PROFILE);
        println!("  -> {}", p);
    } else {
        println!();
        println!("{} not set", MAKE_PROFILE);
    }
}

/// Set `/etc/portage/make.profile` → profile by 1-based index or relative/absolute path.
pub(crate) fn set_profile(spec: &str) -> Result<(), String> {
    let target = resolve_set_spec(spec)?;
    if !target.is_dir() {
        return Err(format!("not a profile directory: {}", target.display()));
    }
    if !is_listed_leaf(&target) {
        return Err(format!(
            "not a leaf profile of the tree: {} (try --list-profiles)",
            target.display()
        ));
    }
    // Prefer relative link from /etc/portage when possible (Portage style).
    let link_path = Path::new(MAKE_PROFILE);
    let parent = link_path.parent().unwrap_or(Path::new("/etc/portage"));
    let link_val = pathdiff_relative(parent, &target).unwrap_or_else(|| target.clone());

    // Drop the old symlink, or a leftover user dir (remove_dir_all never
    // follows symlinks inside it).
    if let Ok(md) = link_path.symlink_metadata() {
        let res = if md.is_dir() {
            eprintln!(
                "{} removing leftover user profile dir {}",
                ">>>".t_yellow().bold(),
                MAKE_PROFILE
            );
            std::fs::remove_dir_all(link_path)
        } else {
            std::fs::remove_file(link_path)
        };
        res.map_err(|e| format!("cannot remove {}: {} (need root?)", MAKE_PROFILE, e))?;
    }

    std::os::unix::fs::symlink(&link_val, link_path).map_err(|e| {
        format!(
            "cannot create {} -> {}: {} (need root?)",
            MAKE_PROFILE,
            link_val.display(),
            e
        )
    })?;
    println!(
        "{} {} -> {}",
        ">>>".t_green().bold(),
        MAKE_PROFILE,
        target.display()
    );
    Ok(())
}

fn resolve_set_spec(spec: &str) -> Result<PathBuf, String> {
    let spec = spec.trim();
    if let Ok(n) = spec.parse::<usize>() {
        let rows = list_profiles_numbered();
        return rows
            .into_iter()
            .find(|(i, _, _, _)| *i == n)
            .map(|(_, root, rel, _)| PathBuf::from(root).join(rel))
            .ok_or_else(|| format!("no profile number {} (try --list-profiles)", n));
    }
    let p = PathBuf::from(spec);
    if p.is_dir() {
        return Ok(std::fs::canonicalize(&p).unwrap_or(p));
    }
    // relative to a known root
    for root in PROFILE_ROOTS {
        let cand = Path::new(root).join(spec);
        if cand.is_dir() {
            return Ok(std::fs::canonicalize(&cand).unwrap_or(cand));
        }
    }
    Err(format!(
        "unknown profile '{}' (path or index from --list-profiles)",
        spec
    ))
}

/// Relative path from `from_dir` to `to` (for compact symlinks).
fn pathdiff_relative(from_dir: &Path, to: &Path) -> Option<PathBuf> {
    let from = std::fs::canonicalize(from_dir).ok()?;
    let to = std::fs::canonicalize(to).ok()?;
    let mut f: Vec<_> = from.components().collect();
    let mut t: Vec<_> = to.components().collect();
    while !f.is_empty() && !t.is_empty() && f[0] == t[0] {
        f.remove(0);
        t.remove(0);
    }
    let mut out = PathBuf::new();
    for _ in &f {
        out.push("..");
    }
    for c in t {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Some(out)
}

/// Print current profile + chain (for `--info` / `--list-profiles`).
pub(crate) fn print_info() {
    match current() {
        None => {
            println!(
                "profile: none ({} not set — bootstrap with a aura-emerge profile tree)",
                MAKE_PROFILE
            );
        }
        Some(p) => {
            println!(
                "profile: {}",
                p.chain
                    .last()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default()
            );
            if p.chain.len() > 1 {
                println!("  parents:");
                for d in &p.chain[..p.chain.len() - 1] {
                    println!("    {}", d.display());
                }
            }
            if !p.system.is_empty() {
                println!("  @system: {} package(s)", p.system.len());
            }
            if !p.packages.is_empty() {
                println!(
                    "  packages: {} total ({} in @system)",
                    p.packages.len(),
                    p.system.len()
                );
            }
            if !p.masks.is_empty() {
                println!("  package.mask: {} entr(y/ies)", p.masks.len());
            }
            for (k, v) in &p.defaults {
                if matches!(k.as_str(), "ARCH" | "ELIBC" | "KERNEL" | "USERLAND") {
                    println!("  {}={}", k, v);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn packages_star_builds_system_set() {
        let dir = std::env::temp_dir().join(format!("ae-prof-pkg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join("packages"),
            "*bash\n*coreutils\nnano\n# comment\n*systemd\n",
        );
        let mut system = Vec::new();
        let mut packages = Vec::new();
        let mut seen_sys = HashSet::new();
        let mut seen_pkg = HashSet::new();
        load_packages(
            &dir,
            &mut system,
            &mut packages,
            &mut seen_sys,
            &mut seen_pkg,
        );
        assert!(system.contains(&"bash".into()));
        assert!(system.contains(&"coreutils".into()));
        assert!(system.contains(&"systemd".into()));
        assert!(packages.contains(&"nano".into()));
        assert_eq!(system.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn packages_wins_over_legacy_package_profile() {
        let dir = std::env::temp_dir().join(format!("ae-prof-new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("packages"), "*bash\nnano\n");
        write(&dir.join("package.profile"), "*old\n");
        let mut system = Vec::new();
        let mut packages = Vec::new();
        load_packages(
            &dir,
            &mut system,
            &mut packages,
            &mut HashSet::new(),
            &mut HashSet::new(),
        );
        assert_eq!(system, vec!["bash".to_string()]);
        assert_eq!(packages, vec!["bash".to_string(), "nano".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_package_profile_file_still_read() {
        let dir = std::env::temp_dir().join(format!("ae-prof-old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("package.profile"), "*old\n");
        let mut system = Vec::new();
        let mut packages = Vec::new();
        load_packages(
            &dir,
            &mut system,
            &mut packages,
            &mut HashSet::new(),
            &mut HashSet::new(),
        );
        assert_eq!(system, vec!["old".to_string()]);
        assert!(has_profile_files(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_mask_minus_removes_parent_entry() {
        let dir = std::env::temp_dir().join(format!("ae-prof-unmask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join("package.mask"),
            "systemd\naur/*-bin\n-systemd\n!aur/*-bin\nextra/linux\n",
        );
        let mut masks = Vec::new();
        load_mask(&dir, &mut masks);
        assert!(!masks.iter().any(|(p, _)| p == "systemd"));
        assert!(!masks.iter().any(|(p, _)| p == "*-bin"));
        assert!(
            masks
                .iter()
                .any(|(p, r)| p == "linux" && r.as_deref() == Some("extra"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn packages_minus_star_clears_system() {
        let dir = std::env::temp_dir().join(format!("ae-prof-clr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("packages"), "*bash\n-*\n*openrc\n");
        let mut system = Vec::new();
        let mut packages = Vec::new();
        let mut seen_sys = HashSet::new();
        let mut seen_pkg = HashSet::new();
        load_packages(
            &dir,
            &mut system,
            &mut packages,
            &mut seen_sys,
            &mut seen_pkg,
        );
        assert_eq!(system, vec!["openrc".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_mask_parses_repo_prefix() {
        let dir = std::env::temp_dir().join(format!("ae-prof-mask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join("package.mask"),
            "systemd\naur/*-bin\n# hi\nextra/linux\n",
        );
        let mut masks = Vec::new();
        load_mask(&dir, &mut masks);
        assert!(masks.iter().any(|(p, r)| p == "systemd" && r.is_none()));
        assert!(
            masks
                .iter()
                .any(|(p, r)| p == "*-bin" && r.as_deref() == Some("aur"))
        );
        assert!(
            masks
                .iter()
                .any(|(p, r)| p == "linux" && r.as_deref() == Some("extra"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parent_chain_order_parents_first() {
        let base = std::env::temp_dir().join(format!("ae-prof-chain-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let parent = base.join("base");
        let leaf = base.join("leaf");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&leaf).unwrap();
        write(&parent.join("packages"), "*bash\n");
        write(&leaf.join("parent"), "../base\n");
        write(&leaf.join("packages"), "*openrc\n");
        let chain = expand_parents(&leaf).unwrap();
        assert!(chain.len() >= 2);
        assert_eq!(chain[0].file_name().unwrap(), "base");
        assert_eq!(chain.last().unwrap().file_name().unwrap(), "leaf");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn pathdiff_goes_up_then_down() {
        // /a/b -> /a/c/d  => ../c/d
        let a = std::env::temp_dir().join(format!("ae-diff-{}", std::process::id()));
        let from = a.join("b");
        let to = a.join("c").join("d");
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        let rel = pathdiff_relative(&from, &to).unwrap();
        let s = rel.to_string_lossy().replace('\\', "/");
        assert!(s.contains("c"), "rel={}", s);
        assert!(s.contains("d"), "rel={}", s);
        let _ = fs::remove_dir_all(&a);
    }

    #[test]
    fn profile_packages_field_populated() {
        let dir = std::env::temp_dir().join(format!("ae-prof-all-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("packages"), "*a\nb\n");
        write(&dir.join("make.defaults"), "ARCH=amd64\n");
        let mut system = Vec::new();
        let mut packages = Vec::new();
        let mut seen_sys = HashSet::new();
        let mut seen_pkg = HashSet::new();
        load_packages(
            &dir,
            &mut system,
            &mut packages,
            &mut seen_sys,
            &mut seen_pkg,
        );
        let mut defaults = Vec::new();
        load_defaults(&dir, &mut defaults);
        let prof = Profile {
            chain: vec![dir.clone()],
            system,
            packages,
            masks: Vec::new(),
            defaults,
        };
        assert!(prof.packages.contains(&"a".into()));
        assert!(prof.packages.contains(&"b".into()));
        assert_eq!(prof.defaults[0].0, "ARCH");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaf_dirs_only() {
        let base = std::env::temp_dir().join(format!("ae-prof-leaf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        write(&base.join("a/parent"), "../base\n");
        write(&base.join("a/b/parent"), "..\n");
        write(&base.join("desc/x.desc"), "x\n");
        assert!(!subdirs(&base.join("a")).is_empty());
        assert!(subdirs(&base.join("a/b")).is_empty());
        assert!(has_profile_files(&base.join("a/b")));
        assert!(!has_profile_files(&base.join("desc")));
        let _ = fs::remove_dir_all(&base);
    }
}
