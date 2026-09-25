//! `/etc/portage/package.mask`: packages this machine never installs,
//! Portage's `package.mask` with Arch atoms.
//!
//! Same path can be either a plain file or a directory -- exactly like
//! real Portage's `package.mask`. As a directory, every regular file
//! inside is read (any name, no `.mask` extension required, dotfiles
//! skipped); as a file, it's read directly. Order across files in a
//! directory is by filename.
//!
//! Separate from the PKGBUILD scanner in `security.rs`: the scanner
//! judges code, the mask is a standing decision needing no
//! justification. Refused even when clean, pulled in as a transitive
//! dependency, or requested by name -- not a warning you click through.
//!
//! Format, one entry per line:
//!
//! ```text
//! ayugram-desktop-bin            # bare name: masked from any source
//! aur/*-bin                      # only from the AUR, '*' allowed
//! extra/nano                     # only from that official repo
//! *-git                          # anything matching, any source
//! ```
//!
//! A trailing `#` comment is kept and shown as the reason when the
//! mask fires.
//!
//! `--exclude` is the one-run version of this (see `runtime.rs`); a
//! mask is persistent and applies to dependencies too.

use std::path::PathBuf;
use std::sync::OnceLock;

use colored::Colorize;

pub(crate) const MASK_FILE: &str = "/etc/portage/package.mask";

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
    /// name) when the caller knows it. A prefixed entry only fires for
    /// that repo; `None` matches only unprefixed entries.
    pub(crate) fn find(&self, name: &str, repo: Option<&str>) -> Option<&MaskEntry> {
        let bare = name.split('/').last().unwrap_or(name);
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

/// Parsed masks, read once per process.
pub(crate) fn masks() -> &'static MaskList {
    MASKS.get_or_init(load)
}

pub(crate) fn find(name: &str, repo: Option<&str>) -> Option<&'static MaskEntry> {
    masks().find(name, repo)
}

fn load() -> MaskList {
    let root = PathBuf::from(MASK_FILE);

    // `package.mask` is either a plain file, or a directory of files
    // (any name, dotfiles skipped) -- same as real Portage. Only one
    // of the two shapes exists on disk at a time, so no merge needed
    // between them; `--regen` migration is what has to decide which
    // shape to write.
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
        if !crate::is_safe_path(&path_s) {
            eprintln!(
                "{} {} is a symlink - refusing to read",
                ">>> Warning:".yellow().bold(),
                path_s
            );
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (atom, reason) = match line.split_once('#') {
                Some((a, r)) => (a.trim(), Some(r.trim().to_string()).filter(|s| !s.is_empty())),
                None => (line, None),
            };
            if atom.is_empty() {
                continue;
            }
            let (repo, pattern) = match atom.split_once('/') {
                Some((r, n)) => (Some(r.trim().to_string()), n.trim().to_string()),
                None => (None, atom.to_string()),
            };
            if pattern.is_empty() || !valid_pattern(&pattern) {
                eprintln!(
                    "{} {}:{}: invalid mask entry '{}' (skipped)",
                    ">>> Warning:".yellow().bold(),
                    path_s,
                    i + 1,
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
fn valid_pattern(p: &str) -> bool {
    p.chars()
        .all(|c| c.is_alphanumeric() || "@._+-*".contains(c))
}

/// Wildcard match, `*` = any run of characters. Two-pointer walk.
fn glob_match(pattern: &str, name: &str) -> bool {
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
pub(crate) fn split_masked(
    pkgs: &[String],
    repo: Option<&str>,
) -> (Vec<String>, Vec<(String, &'static MaskEntry)>) {
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    for p in pkgs {
        match find(p, repo) {
            Some(entry) => blocked.push((p.clone(), entry)),
            None => allowed.push(p.clone()),
        }
    }
    (allowed, blocked)
}

/// Prints the block notice for masked packages. No-op on an empty list.
pub(crate) fn report_blocked(blocked: &[(String, &MaskEntry)]) {
    if blocked.is_empty() {
        return;
    }
    eprintln!();
    eprintln!(
        "{} The following package(s) are masked and will not be installed:",
        " *".red().bold()
    );
    eprintln!();
    for (name, entry) in blocked {
        eprintln!("  {}", name.red().bold());
        eprintln!("    masked by {}", entry.describe().dimmed());
        if let Some(reason) = &entry.reason {
            eprintln!("    reason: {}", reason);
        }
    }
    eprintln!();
    eprintln!(
        "{} Edit {} (a file, or a directory of files) to change that.",
        " *".yellow().bold(),
        MASK_FILE
    );
}

/// Explicit-request gate: reports and returns false if anything in
/// `pkgs` is masked (callers should abort, not just filter).
pub(crate) fn allow_explicit(pkgs: &[String], repo: Option<&str>) -> bool {
    let (_, blocked) = split_masked(pkgs, repo);
    if blocked.is_empty() {
        return true;
    }
    report_blocked(&blocked);
    false
}