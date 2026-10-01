//! `/etc/portage/package.env`: per-package build-flag overrides,
//! Portage's `package.env`.
//!
//! Same shape as `package.mask`: a plain file, or a directory of files
//! (any name, dotfiles skipped, read in filename order).
//!
//! One entry per line: an atom, then one or more env file names.
//!
//! ```text
//! aur/*-git        no-lto            # repo prefix and '*' allowed
//! ttf-comic-sans   fast.conf  quiet  # several envs, applied in order
//! ```
//!
//! Env files live in `/etc/portage/env/` and use `make.conf` syntax;
//! only the build vars count (`CFLAGS`, `OPTIONS`, ...).
//!
//! Layering: make.conf, then every matching entry in file/line order,
//! then each env in the order named. Last to set a var wins; the var
//! is replaced whole, not merged. If two layers set the same var to
//! different values the build prints which one won.
//!
//! Only affects AUR/ABS builds -- official packages aren't built here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use colored::Colorize;

use crate::config::BuildValue;

pub(crate) const ENV_FILE: &str = "/etc/portage/package.env";
pub(crate) const ENV_DIR: &str = "/etc/portage/env";

struct Entry {
    pattern: String,
    repo: Option<String>,
    envs: Vec<String>,
    source: String,
    line: usize,
}

impl Entry {
    fn matches(&self, repo: Option<&str>, names: &[String]) -> bool {
        let repo_ok = match (&self.repo, repo) {
            (None, _) => true,
            (Some(want), Some(have)) => want == have,
            (Some(_), None) => false,
        };
        repo_ok
            && names
                .iter()
                .any(|n| crate::mask::glob_match(&self.pattern, n))
    }

    fn at(&self) -> String {
        format!("{}:{}", self.source, self.line)
    }
}

/// What applies to one build.
#[derive(Default)]
pub(crate) struct Applied {
    /// Build vars to lay over make.conf's, in `BUILD_VARS` order.
    pub(crate) vars: Vec<(String, BuildValue)>,
    /// Env files that contributed, for the "applying" line.
    pub(crate) files: Vec<PathBuf>,
    /// Same-var-different-value notes, already formatted.
    pub(crate) conflicts: Vec<String>,
}

static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();

fn entries() -> &'static [Entry] {
    ENTRIES.get_or_init(load)
}

fn valid_env_name(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with('.')
        && n.chars().all(|c| c.is_alphanumeric() || "._+-".contains(c))
}

fn load() -> Vec<Entry> {
    let root = PathBuf::from(ENV_FILE);
    let files: Vec<PathBuf> = if root.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&root)
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
        v.sort();
        v
    } else {
        vec![root]
    };

    let mut out = Vec::new();
    for path in files {
        if !path.is_file() {
            continue;
        }
        let path_s = path.to_string_lossy().to_string();
        if !crate::is_safe_path(&path_s) {
            warn(&format!("{} is a symlink - refusing to read", path_s));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, raw) in text.lines().enumerate() {
            let body = raw.split('#').next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let mut toks = body.split_whitespace();
            let atom = toks.next().unwrap_or("");
            let envs: Vec<String> = toks.map(str::to_string).collect();
            let (repo, pattern) = match atom.split_once('/') {
                Some((r, n)) => (Some(r.to_string()), n.to_string()),
                None => (None, atom.to_string()),
            };
            if envs.is_empty()
                || !crate::mask::valid_pattern(&pattern)
                || envs.iter().any(|e| !valid_env_name(e))
            {
                warn(&format!(
                    "{}:{}: invalid entry '{}' (skipped)",
                    path_s,
                    i + 1,
                    body
                ));
                continue;
            }
            out.push(Entry {
                pattern,
                repo,
                envs,
                source: path_s.clone(),
                line: i + 1,
            });
        }
    }
    out
}

fn warn(msg: &str) {
    eprintln!("{} {}", ">>> Warning:".yellow().bold(), msg);
}

/// Resolves the overrides for a build of `names` (pkgbase plus every
/// pkgname) from `repo` ("aur"/"abs"). Empty when nothing matches.
pub(crate) fn applied_for(repo: Option<&str>, names: &[String]) -> Applied {
    let mut res = Applied::default();
    // var -> (where it was set, value as shown)
    let mut set: HashMap<String, (String, String)> = HashMap::new();
    let mut merged: HashMap<String, BuildValue> = HashMap::new();

    for e in entries().iter().filter(|e| e.matches(repo, names)) {
        for env in &e.envs {
            let path = Path::new(ENV_DIR).join(env);
            let Some(vars) = crate::config::load_env_file(&path) else {
                warn(&format!("{}: env '{}' unusable (skipped)", e.at(), env));
                continue;
            };
            if !res.files.contains(&path) {
                res.files.push(path.clone());
            }
            let origin = format!("{} [{}]", e.at(), env);
            for (k, v) in vars {
                let shown = v.display();
                if let Some((prev_at, prev_val)) = set.get(&k) {
                    if *prev_val != shown {
                        res.conflicts.push(format!(
                            "package.env: {} = '{}' from {} overrides '{}' from {}",
                            k, shown, origin, prev_val, prev_at
                        ));
                    }
                }
                set.insert(k.clone(), (origin.clone(), shown));
                merged.insert(k, v);
            }
        }
    }

    res.vars = crate::config::BUILD_VARS
        .iter()
        .filter_map(|k| merged.remove(*k).map(|v| (k.to_string(), v)))
        .collect();
    res
}
