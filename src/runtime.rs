//! Process-wide run options resolved once in `main::run()`, plus the
//! failure log behind `--keep-going`.
//!
//! Global instead of more parameters: `emerge.conf`, `--exclude`, and
//! `--keep-going` are read deep in the build path (functions already
//! taking 7-11 args each), and never differ between two calls in the
//! same process. Set once via `OnceLock`; the failure log is the one
//! mutable piece, behind a `Mutex`.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use colored::Colorize;

use crate::config::Config;

#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) config: Config,
    /// Bare package names from `--exclude` (and nothing else -- masks
    /// are a separate, persistent layer, see `mask.rs`).
    pub(crate) exclude: HashSet<String>,
    pub(crate) keep_going: bool,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Called once, early in `run()`. A second call is ignored rather than
/// panicking.
pub(crate) fn init(rt: Runtime) {
    let _ = RUNTIME.set(rt);
}

/// Resolved options, or an all-default set if `init()` never ran.
pub(crate) fn get() -> &'static Runtime {
    RUNTIME.get_or_init(Runtime::default)
}

pub(crate) fn config() -> &'static Config {
    &get().config
}

pub(crate) fn keep_going() -> bool {
    get().keep_going
}

/// True if `--exclude` named this package (compared bare, so
/// `--exclude extra/nano` and `--exclude nano` both hit `nano`).
pub(crate) fn is_excluded(name: &str) -> bool {
    let bare = name.split('/').last().unwrap_or(name);
    get().exclude.contains(bare)
}

/// Splits a package list into (kept, excluded-by-`--exclude`).
pub(crate) fn split_excluded(pkgs: &[String]) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for p in pkgs {
        if is_excluded(p) {
            dropped.push(p.clone());
        } else {
            kept.push(p.clone());
        }
    }
    (kept, dropped)
}

/// Prints the "skipped by --exclude" note. No-op on an empty list.
pub(crate) fn report_excluded(dropped: &[String]) {
    if dropped.is_empty() {
        return;
    }
    println!(
        "{} {} package(s) skipped by --exclude: {}",
        ">>>".yellow().bold(),
        dropped.len(),
        dropped.join(", ")
    );
}

// ── failure log (--keep-going) ────────────────────────────────────────────────

static FAILURES: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

fn failures() -> &'static Mutex<Vec<(String, String)>> {
    FAILURES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Records one failed atom plus a short reason. Always recorded; the
/// flag decides whether the run continues, not whether this happens.
pub(crate) fn record_failure(atom: &str, reason: &str) {
    if let Ok(mut log) = failures().lock() {
        if log.iter().any(|(a, _)| a == atom) {
            return;
        }
        log.push((atom.to_string(), reason.to_string()));
    }
}

/// Every failed atom, in the order they failed.
pub(crate) fn failed_atoms() -> Vec<String> {
    failures()
        .lock()
        .map(|log| log.iter().map(|(a, _)| a.clone()).collect())
        .unwrap_or_default()
}

pub(crate) fn any_failures() -> bool {
    failures().lock().map(|log| !log.is_empty()).unwrap_or(false)
}

/// Gentoo-style end-of-run failure summary. Returns true if anything
/// was printed, i.e. if the run had failures.
pub(crate) fn print_failure_summary() -> bool {
    let Ok(log) = failures().lock() else { return false };
    if log.is_empty() {
        return false;
    }
    eprintln!();
    eprintln!(
        "{} The following {} package(s) failed to build or install:",
        " *".red().bold(),
        log.len()
    );
    eprintln!();
    for (atom, reason) in log.iter() {
        eprintln!("  {} {}", atom.red().bold(), format!("({})", reason).dimmed());
    }
    eprintln!();
    eprintln!(
        "{} Everything else in this run completed. Retry just the failures with {}.",
        " *".yellow().bold(),
        "emerge --resume".cyan()
    );
    true
}