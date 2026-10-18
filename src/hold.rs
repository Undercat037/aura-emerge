//! `/etc/portage/package.protect` and `/etc/portage/package.freeze`.
//!
//! Same atom syntax as `package.mask` (`*`, optional `repo/` prefix, `#`
//! comments; file or directory of files).
//!
//! * **protect** — must not be removed (`-C`, `-c`, `--prune`, conflict
//!   unmerge). `--deselect` may still drop the name from world.
//! * **freeze** — must not be upgraded (`-u` / sysupgrade; also
//!   `EMERGE_FREEZE` in make.conf). Stricter than pacman's `IgnorePkg`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::mask::{glob_match, valid_pattern};
use crate::theme::Themed;
use colored::Colorize;

pub(crate) const PROTECT_FILE: &str = "/etc/portage/package.protect";
pub(crate) const FREEZE_FILE: &str = "/etc/portage/package.freeze";
const FREEZE_CONF_KEY: &str = "EMERGE_FREEZE";

// ── shared entry ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct HoldEntry {
    pub(crate) pattern: String,
    pub(crate) repo: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) source: String,
    pub(crate) line: usize,
}

impl HoldEntry {
    pub(crate) fn describe(&self) -> String {
        let atom = match &self.repo {
            Some(r) => format!("{}/{}", r, self.pattern),
            None => self.pattern.clone(),
        };
        format!("{} ({}:{})", atom, self.source, self.line)
    }
}

#[derive(Debug, Default)]
pub(crate) struct HoldList {
    entries: Vec<HoldEntry>,
}

impl HoldList {
    pub(crate) fn find(&self, name: &str, repo: Option<&str>) -> Option<&HoldEntry> {
        let bare = name.split('/').next_back().unwrap_or(name);
        self.entries.iter().find(|e| {
            let repo_ok = match (&e.repo, repo) {
                (None, _) => true,
                (Some(want), Some(have)) => want == have,
                (Some(_), None) => false,
            };
            repo_ok && glob_match(&e.pattern, bare)
        })
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Exact (non-glob) patterns for libalpm `--ignore`.
    pub(crate) fn exact_ignore_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| !e.pattern.contains('*'))
            .map(|e| e.pattern.clone())
            .collect()
    }
}

fn push_entry(
    entries: &mut Vec<HoldEntry>,
    atom: &str,
    reason: Option<String>,
    source: &str,
    line: usize,
    kind: &str,
) {
    if atom.is_empty() {
        return;
    }
    let (repo, pattern) = match atom.split_once('/') {
        Some((r, n)) => (Some(r.trim().to_string()), n.trim().to_string()),
        None => (None, atom.to_string()),
    };
    if pattern.is_empty() || !valid_pattern(&pattern) {
        eprintln!(
            "{} {}:{}: invalid {} entry '{}' (skipped)",
            ">>> Warning:".t_yellow().bold(),
            source,
            line,
            kind,
            atom
        );
        return;
    }
    entries.push(HoldEntry {
        pattern,
        repo,
        reason,
        source: source.to_string(),
        line,
    });
}

fn load_path_entries(root: &Path, kind: &str, entries: &mut Vec<HoldEntry>) {
    let files: Vec<PathBuf> = if root.is_dir() {
        let mut extra: Vec<PathBuf> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                !p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(true)
            })
            .collect();
        extra.sort();
        extra
    } else {
        vec![root.to_path_buf()]
    };

    for path in files {
        if !path.is_file() {
            continue;
        }
        let path_s = path.to_string_lossy().to_string();
        let text = match crate::read_to_string_nofollow(&path) {
            Ok(t) => t,
            Err(e) if crate::is_symlink_open_error(&e) => {
                eprintln!(
                    "{} {} is a symlink - refusing to read",
                    ">>> Warning:".t_yellow().bold(),
                    path_s
                );
                continue;
            }
            Err(_) => continue,
        };
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (atom, reason) = match line.split_once('#') {
                Some((a, r)) => (
                    a.trim(),
                    Some(r.trim().to_string()).filter(|s| !s.is_empty()),
                ),
                None => (line, None),
            };
            push_entry(entries, atom, reason, &path_s, i + 1, kind);
        }
    }
}

fn report_held(held: &[(String, &HoldEntry)], verb: &str) {
    if held.is_empty() {
        return;
    }
    eprintln!(
        "{} {} package(s) {}:",
        ">>>".t_yellow().bold(),
        held.len(),
        verb
    );
    for (name, e) in held {
        let why = e.reason.as_deref().unwrap_or("");
        if why.is_empty() {
            eprintln!("    {}  [{}]", name, e.describe());
        } else {
            eprintln!("    {}  [{}] {}", name, e.describe(), why);
        }
    }
}

// ── protect ──────────────────────────────────────────────────────────────────

static PROTECT: OnceLock<HoldList> = OnceLock::new();

fn load_protect() -> HoldList {
    let mut entries = Vec::new();
    load_path_entries(Path::new(PROTECT_FILE), "protect", &mut entries);
    HoldList { entries }
}

pub(crate) fn protect_list() -> &'static HoldList {
    PROTECT.get_or_init(load_protect)
}

pub(crate) fn find_protect(name: &str, repo: Option<&str>) -> Option<&'static HoldEntry> {
    protect_list().find(name, repo)
}

pub(crate) fn is_protected(name: &str) -> bool {
    if find_protect(name, None).is_some() {
        return true;
    }
    // Profile @system packages are never depcleaned/unmerged by accident.
    let bare = name.split('/').next_back().unwrap_or(name);
    crate::profile::system_names().contains(bare)
}

pub(crate) fn split_protected(pkgs: &[String]) -> (Vec<String>, Vec<(String, &'static HoldEntry)>) {
    let mut ok = Vec::new();
    let mut held = Vec::new();
    for p in pkgs {
        match find_protect(p, None) {
            Some(e) => held.push((p.clone(), e)),
            None => ok.push(p.clone()),
        }
    }
    (ok, held)
}

pub(crate) fn report_protected(held: &[(String, &HoldEntry)]) {
    report_held(held, "protected from removal");
}

// ── freeze ───────────────────────────────────────────────────────────────────

static FREEZE: OnceLock<HoldList> = OnceLock::new();

fn load_freeze() -> HoldList {
    let mut entries = Vec::new();

    // make.conf EMERGE_FREEZE="a b *-git"
    if let Ok(text) = crate::read_to_string_nofollow(Path::new(crate::config::SYSTEM_CONF)) {
        for (i, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.starts_with('#') {
                continue;
            }
            let Some(rest) = t
                .strip_prefix(FREEZE_CONF_KEY)
                .and_then(|s| s.trim().strip_prefix('='))
            else {
                continue;
            };
            let rest = rest.trim().trim_matches('"').trim_matches('\'');
            for tok in rest.split_whitespace() {
                push_entry(
                    &mut entries,
                    tok,
                    None,
                    "make.conf EMERGE_FREEZE",
                    i + 1,
                    "freeze",
                );
            }
        }
    }

    load_path_entries(Path::new(FREEZE_FILE), "freeze", &mut entries);
    HoldList { entries }
}

pub(crate) fn freeze_list() -> &'static HoldList {
    FREEZE.get_or_init(load_freeze)
}

pub(crate) fn find_freeze(name: &str, repo: Option<&str>) -> Option<&'static HoldEntry> {
    freeze_list().find(name, repo)
}

pub(crate) fn is_frozen(name: &str) -> bool {
    find_freeze(name, None).is_some()
}

pub(crate) fn split_frozen(pkgs: &[String]) -> (Vec<String>, Vec<(String, &'static HoldEntry)>) {
    let mut ok = Vec::new();
    let mut held = Vec::new();
    for p in pkgs {
        match find_freeze(p, None) {
            Some(e) => held.push((p.clone(), e)),
            None => ok.push(p.clone()),
        }
    }
    (ok, held)
}

pub(crate) fn report_frozen(held: &[(String, &HoldEntry)]) {
    report_held(held, "frozen (not upgraded)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_entry_describe_with_repo() {
        let e = HoldEntry {
            pattern: "linux".into(),
            repo: Some("extra".into()),
            reason: Some("pin".into()),
            source: "test".into(),
            line: 1,
        };
        let d = e.describe();
        assert!(d.contains("extra/linux"));
        assert!(d.contains("test:1"));
    }

    #[test]
    fn exact_ignore_skips_globs() {
        let list = HoldList {
            entries: vec![
                HoldEntry {
                    pattern: "foo".into(),
                    repo: None,
                    reason: None,
                    source: "t".into(),
                    line: 1,
                },
                HoldEntry {
                    pattern: "*-git".into(),
                    repo: None,
                    reason: None,
                    source: "t".into(),
                    line: 2,
                },
            ],
        };
        let names = list.exact_ignore_names();
        assert_eq!(names, vec!["foo".to_string()]);
    }
}
