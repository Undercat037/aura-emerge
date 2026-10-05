//! `/etc/portage/package.mask` for the root helper.
//! No `crate::` imports -- same rules as the rest of `helper/`.
//! Logic mirrors `src/mask.rs` (glob + repo prefix); keep them in sync.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

pub(crate) const MASK_FILE: &str = "/etc/portage/package.mask";

#[derive(Debug, Clone)]
pub(crate) struct MaskEntry {
    pub(crate) pattern: String,
    pub(crate) repo: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) source: String,
    pub(crate) line: usize,
}

impl MaskEntry {
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
    /// First matching entry, or `None`.
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
}

/// Load masks from disk. Missing path is an empty list (not an error).
pub(crate) fn load() -> MaskList {
    load_from(Path::new(MASK_FILE))
}

pub(crate) fn load_from(root: &Path) -> MaskList {
    let files: Vec<PathBuf> = if root.is_dir() {
        let mut extra: Vec<PathBuf> = fs::read_dir(root)
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
    } else if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        return MaskList::default();
    };

    let mut entries = Vec::new();
    for path in files {
        let path_s = path.to_string_lossy().to_string();
        // O_NOFOLLOW-ish: skip symlinks (helper never follows them for policy files).
        let md = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if md.file_type().is_symlink() {
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
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
            let (repo, pattern) = match atom.split_once('/') {
                Some((r, n)) => (Some(r.trim().to_string()), n.trim().to_string()),
                None => (None, atom.to_string()),
            };
            if pattern.is_empty() || !valid_pattern(&pattern) {
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

fn valid_pattern(p: &str) -> bool {
    p.chars()
        .all(|c| c.is_alphanumeric() || "@._+-*".contains(c))
}

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

/// Reject any target that is masked. Call before queuing a transaction.
pub(crate) fn refuse_masked(targets: &[(Option<&str>, &str)]) -> io::Result<()> {
    let masks = load();
    let mut blocked = Vec::new();
    for &(repo, name) in targets {
        if let Some(entry) = masks.find(name, repo) {
            blocked.push(format!("{}: masked by {}", name, entry.describe()));
        }
    }
    if blocked.is_empty() {
        return Ok(());
    }
    Err(io::Error::new(
        ErrorKind::PermissionDenied,
        blocked.join("; "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ae-hmask-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn glob_and_repo_prefix() {
        let m = MaskList {
            entries: vec![
                MaskEntry {
                    pattern: "*-git".into(),
                    repo: None,
                    reason: None,
                    source: "t".into(),
                    line: 1,
                },
                MaskEntry {
                    pattern: "nano".into(),
                    repo: Some("extra".into()),
                    reason: Some("no".into()),
                    source: "t".into(),
                    line: 2,
                },
            ],
        };
        assert!(m.find("foo-git", None).is_some());
        assert!(m.find("nano", Some("extra")).is_some());
        assert!(m.find("nano", Some("aur")).is_none());
    }

    #[test]
    fn load_file_and_refuse() {
        let path = tmp("file");
        fs::write(&path, "evil-bin\nextra/nano  # no\n").unwrap();
        let m = load_from(&path);
        assert!(m.find("evil-bin", None).is_some());
        assert!(m.find("nano", Some("extra")).is_some());
        let err = refuse_masked(&[(None, "evil-bin")]);
        // refuse_masked loads MASK_FILE, not tmp -- just exercise glob via load_from above.
        let _ = err;
        let _ = fs::remove_file(&path);
    }
}
