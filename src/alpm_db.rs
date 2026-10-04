//! Read-only libalpm (step 1). No transactions / -U / helper.
//! Alpm is !Send — open handle per call.

use std::collections::HashMap;

use alpm::{Alpm, Package};
use alpm_utils::alpm_with_conf;
use alpm_utils::config::Config as PacmanConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlpmPkg {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) repo: String,
    pub(crate) description: String,
}

fn open() -> Option<Alpm> {
    let conf = PacmanConfig::new().ok()?;
    alpm_with_conf(&conf).ok()
}

fn bare(name: &str) -> &str {
    name.split('/').last().unwrap_or(name)
}

fn pkg_to_info(pkg: &Package, repo: &str) -> AlpmPkg {
    AlpmPkg {
        name: pkg.name().to_string(),
        version: pkg.version().to_string(),
        repo: repo.to_string(),
        description: pkg.desc().unwrap_or("").to_string(),
    }
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
                out.insert(bare.to_string(), pkg_to_info(pkg, db.name()));
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
