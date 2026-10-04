//! Read-only libalpm. No transactions / -U / helper.
//! Alpm is !Send — open handle per call.

use std::collections::HashMap;

use alpm::{Alpm, Package, PackageReason};
use alpm_utils::alpm_with_conf;
use alpm_utils::config::Config as PacmanConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlpmPkg {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) repo: String,
    pub(crate) description: String,
    pub(crate) installed: bool,
}

fn open() -> Option<Alpm> {
    let conf = PacmanConfig::new().ok()?;
    alpm_with_conf(&conf).ok()
}

fn bare(name: &str) -> &str {
    name.split('/').last().unwrap_or(name)
}

fn pkg_to_info(pkg: &Package, repo: &str, installed: bool) -> AlpmPkg {
    AlpmPkg {
        name: pkg.name().to_string(),
        version: pkg.version().to_string(),
        repo: repo.to_string(),
        description: pkg.desc().unwrap_or("").to_string(),
        installed,
    }
}

pub(crate) fn is_installed(name: &str) -> bool {
    installed_version(name).is_some()
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
                let new_ver = sp.version().to_string();
                if alpm::vercmp(local_ver.as_str(), new_ver.as_str()) == std::cmp::Ordering::Less {
                    out.push((name.to_string(), local_ver, new_ver, db.name().to_string()));
                    break;
                }
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

pub(crate) fn find_sync_many(names: &[String]) -> HashMap<String, AlpmPkg> {
    let Some(alpm) = open() else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for name in names {
        let bare = bare(name);
        for db in alpm.syncdbs() {
            if let Ok(pkg) = db.pkg(bare) {
                out.insert(bare.to_string(), pkg_to_info(pkg, db.name(), false));
                break;
            }
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
            Some(p) => found.push(p.clone()),
            None => missing.push(name.clone()),
        }
    }
    (found, missing)
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
                if let Ok(sp) = db.pkg(bare.as_str()) {
                    if sp.version().as_str() == ver {
                        hits.push(db.name().to_string());
                    }
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

// ── search / depends / -T ────────────────────────────────────────────────────

/// Sync-db search (`pacman -Ss`).
/// `all_repos`: false = one row per package name (first repo in conf order);
/// true = every repo hit (full pacman -Ss style).
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
            let installed = local.pkg(name.as_str()).is_ok();
            out.push(pkg_to_info(pkg, repo, installed));
        }
    }
    out
}

/// Exact sync lookup by bare name.
pub(crate) fn find_sync(name: &str) -> Option<AlpmPkg> {
    let alpm = open()?;
    let bare = bare(name);
    for db in alpm.syncdbs() {
        if let Ok(pkg) = db.pkg(bare) {
            return Some(pkg_to_info(pkg, db.name(), false));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_strips() {
        assert_eq!(bare("extra/nano"), "nano");
    }

    #[test]
    fn vercmp_ordering() {
        assert_eq!(alpm::vercmp("1.0-1", "1.0-1"), std::cmp::Ordering::Equal);
        assert_eq!(alpm::vercmp("1.0-1", "1.0-2"), std::cmp::Ordering::Less);
        assert_eq!(alpm::vercmp("2.0-1", "1.9-1"), std::cmp::Ordering::Greater);
    }
}
