//! `/etc/portage/package.mask` and `/etc/portage/package.unmask`.
//!
//! Mask: packages this machine never installs (Portage `package.mask`
//! with Arch atoms). Unmask: explicit exceptions that override both
//! user `package.mask` and profile `package.mask`.
//!

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::theme::Themed;
use colored::Colorize;

pub(crate) const MASK_FILE: &str = "/etc/portage/package.mask";
pub(crate) const UNMASK_FILE: &str = "/etc/portage/package.unmask";

#[derive(Debug, Clone)]
pub(crate) struct MaskEntry {
    /// Name pattern, `*` allowed.
    pub(crate) pattern: String,
    /// Repo prefix the entry was written with, if any ("aur", "abs",
    /// "extra", ...). `None` means "from anywhere".
    pub(crate) repo: Option<String>,
    /// Text after `#` on the entry's own line.
    pub(crate) reason: Option<String>,
    pub(crate) source: String,
    pub(crate) line: usize,
}

impl MaskEntry {
    /// "aur/*-bin (/etc/portage/mask:7)" for messages.
    pub(crate) fn describe(&self) -> String {
        let atom = match &self.repo {
            Some(r) => format!("{}/{}", r, self.pattern),
            None => self.pattern.clone(),
        };
        format!("{} ({}:{})", atom, self.source, self.line)
    }
}

#[derive(Debug, Default)]
pub(crate) struct MaskList {
    entries: Vec<MaskEntry>,
}

impl MaskList {
    /// First entry matching this package, or `None`.
    ///
    /// `repo` is the actual source ("aur", "abs", an official repo

    pub(crate) fn find(&self, name: &str, repo: Option<&str>) -> Option<&MaskEntry> {
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

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

static MASKS: OnceLock<MaskList> = OnceLock::new();
static UNMASKS: OnceLock<MaskList> = OnceLock::new();

/// Parsed masks, read once per process.
pub(crate) fn masks() -> &'static MaskList {
    MASKS.get_or_init(|| load_list(MASK_FILE, "mask"))
}

/// Parsed unmasks, read once per process.
pub(crate) fn unmasks() -> &'static MaskList {
    UNMASKS.get_or_init(|| load_list(UNMASK_FILE, "unmask"))
}

/// User-mask entry, if any (ignores unmask — callers that need a hard
/// block should use [`is_blocked`]).
pub(crate) fn find(name: &str, repo: Option<&str>) -> Option<&'static MaskEntry> {
    masks().find(name, repo)
}

/// True if `/etc/portage/package.unmask` matches this package.
pub(crate) fn is_unmasked(name: &str, repo: Option<&str>) -> bool {
    unmasks().find(name, repo).is_some()
}

fn load_list(root_path: &str, kind: &str) -> MaskList {
    let root = PathBuf::from(root_path);

    // File or directory of files (dotfiles skipped) — same as Portage.
    let files: Vec<PathBuf> = if root.is_dir() {
        let mut extra: Vec<PathBuf> = std::fs::read_dir(&root)
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
        vec![root]
    };

    let mut entries = Vec::new();
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
            if atom.is_empty() {
                continue;
            }
            // =cat/foo used to parse as repo "=cat" and silently never
            // match anything; split_repo_atom makes that (and every
            // version-scoped atom) a loud skip instead.
            let (repo, pattern) = match split_repo_atom(atom) {
                Ok(pair) => pair,
                Err(why) => {
                    eprintln!(
                        "{} {}:{}: unusable {} entry '{}' ({} - skipped)",
                        ">>> Warning:".t_yellow().bold(),
                        path_s,
                        i + 1,
                        kind,
                        atom,
                        why
                    );
                    continue;
                }
            };
            if pattern.is_empty() || !valid_pattern(&pattern) {
                eprintln!(
                    "{} {}:{}: invalid {} entry '{}' (skipped)",
                    ">>> Warning:".t_yellow().bold(),
                    path_s,
                    i + 1,
                    kind,
                    atom
                );
                continue;
            }
            entries.push(MaskEntry {
                pattern,
                repo,
                reason,
                source: path_s.clone(),
                line: i + 1,
            });
        }
    }
    MaskList { entries }
}

/// Same character set as a package name, plus `*`.
pub(crate) fn valid_pattern(p: &str) -> bool {
    p.chars()
        .all(|c| c.is_alphanumeric() || "@._+-*".contains(c))
}

const VERSION_OP_ERR: &str = "version-scoped atoms (=, >=, <=, >, ~) are not supported - entries match the package name at every version; drop the operator if that is what you want";

/// Splits a mask/hold atom into its optional repo prefix and name
/// pattern. Portage-style operators on the repo (`=extra/foo`,
/// `>=extra/foo`) used to parse into the repo name ("=extra") and then
/// never match anything -- silently. Such entries are now reported and
/// skipped, because matching here is name-level only and an entry that
/// can never match is a hole, not a no-op. `!` (blocker prefix) is
/// meaningless in these files and dropped.
pub(crate) fn split_repo_atom(atom: &str) -> Result<(Option<String>, String), &'static str> {
    let atom = atom.trim();
    let (repo, pattern) = match atom.split_once('/') {
        Some((r, n)) => (Some(r.trim()), n.trim()),
        None => (None, atom),
    };
    let repo = repo.map(|r| r.trim_start_matches('!'));
    let versioned = |s: &str| s.starts_with(['=', '<', '>', '~']);
    if let Some(r) = repo {
        if r.is_empty() {
            return Err("empty repo prefix");
        }
        if versioned(r) {
            return Err(VERSION_OP_ERR);
        }
    }
    if pattern.is_empty() {
        return Err("empty package pattern");
    }
    if versioned(pattern) {
        return Err(VERSION_OP_ERR);
    }
    Ok((repo.map(str::to_string), pattern.to_string()))
}

/// Splits a user-typed / world-style atom into (bare name, repo).
/// `aur/`, `abs/` and official repo prefixes are kept; `Err/` (unknown
/// local source) and `@set` names carry no repo -- only unprefixed
/// entries can ever match those.
pub(crate) fn atom_repo(atom: &str) -> (String, Option<String>) {
    match atom.split_once('/') {
        Some(("Err", n)) => (n.to_string(), None),
        Some((r, n)) if !n.is_empty() && !n.contains('/') => {
            (n.to_string(), Some(r.to_string()))
        }
        _ => (atom.to_string(), None),
    }
}

/// Wildcard match, `*` = any run of characters. Two-pointer walk.
pub(crate) fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);

    while ni < n.len() {
        if pi < p.len() && (p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ── enforcement helpers ───────────────────────────────────────────────────────

/// Splits a package list into (allowed, blocked-with-the-entry).
/// Respects `package.unmask` and profile masks (via [`is_blocked`]).
pub(crate) fn split_masked(
    pkgs: &[String],
    repo: Option<&str>,
) -> (Vec<String>, Vec<(String, Option<&'static MaskEntry>)>) {
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    for p in pkgs {
        if is_blocked(p, repo) {
            blocked.push((p.clone(), find(p, repo)));
        } else {
            allowed.push(p.clone());
        }
    }
    (allowed, blocked)
}

/// Prints the block notice for masked packages. No-op on an empty list.
pub(crate) fn report_blocked(blocked: &[(String, Option<&MaskEntry>)]) {
    if blocked.is_empty() {
        return;
    }
    eprintln!();
    eprintln!(
        "{} The following package(s) are masked and will not be installed:",
        " *".t_red().bold()
    );
    eprintln!();
    for (name, entry) in blocked {
        eprintln!("  {}", name.t_red().bold());
        match entry {
            Some(entry) => {
                eprintln!("    masked by {}", entry.describe().dimmed());
                if let Some(reason) = &entry.reason {
                    eprintln!("    reason: {}", reason);
                }
            }
            None => {
                eprintln!("    {}", "masked by profile package.mask".dimmed());
            }
        }
    }
    eprintln!();
    eprintln!(
        "{} Edit {} / {} (file or directory) to change that.",
        " *".t_yellow().bold(),
        MASK_FILE,
        UNMASK_FILE
    );
}

/// Plan-wide gate: every package in the resolved transaction (targets
/// *and* deps) is checked. Masked deps must not slip through just
/// because the top-level atom was clean.
pub(crate) fn allow_plan<'a, I>(pkgs: I) -> bool
where
    I: IntoIterator<Item = (&'a str, Option<&'a str>)>,
{
    let mut blocked: Vec<(String, Option<&MaskEntry>)> = Vec::new();
    for (name, repo) in pkgs {
        if is_blocked(name, repo) {
            blocked.push((name.to_string(), find(name, repo)));
        }
    }
    if blocked.is_empty() {
        return true;
    }
    report_blocked(&blocked);
    false
}

/// Explicit-request gate for atoms that may carry their own repo
/// prefix (`aur/foo`, `extra/nano`, bare `nano`): the atom's own prefix
/// picks the repo the mask lookup runs in, so a repo-prefixed mask
/// entry fires for exactly the targets it was written for. Reports and
/// returns false if anything is masked (callers should abort, not just
/// filter).
pub(crate) fn allow_explicit_atoms(pkgs: &[String]) -> bool {
    let mut blocked: Vec<(String, Option<&MaskEntry>)> = Vec::new();
    for p in pkgs {
        let (bare, repo) = atom_repo(p);
        if is_blocked(&bare, repo.as_deref()) {
            blocked.push((p.clone(), find(&bare, repo.as_deref())));
        }
    }
    if blocked.is_empty() {
        return true;
    }
    report_blocked(&blocked);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[(&str, Option<&str>)]) -> MaskList {
        MaskList {
            entries: entries
                .iter()
                .enumerate()
                .map(|(i, (pat, repo))| MaskEntry {
                    pattern: pat.to_string(),
                    repo: repo.map(str::to_string),
                    reason: None,
                    source: "test".to_string(),
                    line: i + 1,
                })
                .collect(),
        }
    }

    #[test]
    fn glob_star_matches_any_run() {
        assert!(glob_match("emacs-*", "emacs-nox"));
        assert!(glob_match("*-git", "foo-git"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("emacs-*", "emacs"));
        assert!(!glob_match("*-git", "foo-git2"));
    }

    #[test]
    fn bare_entry_masks_exactly_that_name() {
        let m = list(&[("emacs", None), ("emacs-*", None)]);
        assert!(m.find("emacs", None).is_some());
        assert!(m.find("extra/emacs", Some("extra")).is_some());
        assert!(m.find("emacs-nox", None).is_some());
        assert!(m.find("emacsclient", None).is_none());
    }

    #[test]
    fn prefixed_entry_only_fires_for_that_repo() {
        let m = list(&[("nano", Some("extra"))]);
        assert!(m.find("nano", Some("extra")).is_some());
        assert!(m.find("nano", Some("aur")).is_none());
        assert!(m.find("nano", None).is_none());
    }

    #[test]
    fn version_scoped_atoms_are_rejected_loudly() {
        // These used to parse the operator into the repo name ("=extra")
        // and then never match anything -- silently.
        assert!(split_repo_atom("=extra/foo-1.2").is_err());
        assert!(split_repo_atom(">=extra/foo").is_err());
        assert!(split_repo_atom("~extra/foo").is_err());
        assert!(split_repo_atom("=foo-1.2").is_err());
        assert!(split_repo_atom("<foo").is_err());
        // Plain atoms, with or without a repo, keep parsing.
        assert_eq!(
            split_repo_atom("extra/nano").unwrap(),
            (Some("extra".to_string()), "nano".to_string())
        );
        assert_eq!(
            split_repo_atom("nano").unwrap(),
            (None, "nano".to_string())
        );
        // The `!` blocker prefix carries no meaning here; drop it.
        assert_eq!(
            split_repo_atom("!aur/foo").unwrap(),
            (Some("aur".to_string()), "foo".to_string())
        );
        assert!(split_repo_atom("/foo").is_err());
    }

    #[test]
    fn atoms_split_into_bare_and_repo() {
        assert_eq!(
            atom_repo("extra/nano"),
            ("nano".to_string(), Some("extra".to_string()))
        );
        assert_eq!(
            atom_repo("aur/foo"),
            ("foo".to_string(), Some("aur".to_string()))
        );
        // Err/ is an unknown source, not a repo.
        assert_eq!(atom_repo("Err/x"), ("x".to_string(), None));
        assert_eq!(atom_repo("nano"), ("nano".to_string(), None));
        assert_eq!(atom_repo("@world"), ("@world".to_string(), None));
    }
}

/// User `package.mask` **or** active profile `package.mask`, unless
/// matched by `package.unmask`.
pub(crate) fn is_blocked(name: &str, repo: Option<&str>) -> bool {
    if is_unmasked(name, repo) || is_unmasked(name, None) {
        return false;
    }
    if find(name, repo).is_some() {
        return true;
    }
    crate::profile::is_masked(name, repo)
}
