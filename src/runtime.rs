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

use crate::theme::Themed;
use colored::Colorize;

use crate::config::Config;

#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) config: Config,
    /// Bare package names from `--exclude` (masks are a separate layer).
    pub(crate) exclude: HashSet<String>,
    pub(crate) keep_going: bool,
    /// Live makepkg output (`--debug` / `AE_DEBUG=1` / `--quiet-build=n`).
    pub(crate) debug: bool,
    /// `emerge -n` / pacman `--needed`.
    pub(crate) noreplace: bool,
    /// `--with-optdeps`: also install optdepends (as dependencies).
    pub(crate) with_optdeps: bool,
    /// `--nospinner`: no live Jobs row.
    pub(crate) nospinner: bool,
    /// `--load-average`: hold new builds while the 1-min load is above this.
    pub(crate) load_average: Option<f32>,
    /// Max concurrent official-repo installs (`--jobsr` / `--jobs`).
    pub(crate) jobsr: u32,
    /// Max concurrent AUR/ABS builds (`--jobsa`).
    pub(crate) jobsa: u32,
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

pub(crate) fn show_build_output() -> bool {
    get().debug
}

// ── --load-average ────────────────────────────────────────────────────────────

/// Builds running right now; only touched when `--load-average` is set.
static ACTIVE_JOBS: Mutex<usize> = Mutex::new(0);
static LOAD_NOTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One running build's place in the pool; freed on drop.
pub(crate) struct JobSlot(bool);

impl Drop for JobSlot {
    fn drop(&mut self) {
        if self.0 {
            let mut n = ACTIVE_JOBS.lock().unwrap_or_else(|e| e.into_inner());
            *n = n.saturating_sub(1);
        }
    }
}

/// 1-minute load average, `None` if /proc/loadavg can't be read.
fn load_1min() -> Option<f32> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Called by a build worker before it starts a job. Returns at once
/// without `--load-average`. With it, a new build waits while the load
/// is above the limit - but never when nothing else is running (as in
/// Portage), so a busy machine can't stall the run for good. Applies to
/// the `--jobsa` pool (AUR/ABS builds), not to repo installs.
pub(crate) fn acquire_job_slot() -> JobSlot {
    let Some(limit) = get().load_average else {
        return JobSlot(false);
    };
    loop {
        {
            let mut n = ACTIVE_JOBS.lock().unwrap_or_else(|e| e.into_inner());
            let load = load_1min().unwrap_or(0.0);
            if *n == 0 || load <= limit {
                *n += 1;
                return JobSlot(true);
            }
            if !LOAD_NOTE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                println!(
                    "{} load average {:.2} is above --load-average={}: holding new builds until it drops.",
                    ">>>".t_yellow().bold(),
                    load,
                    limit
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// True if `--exclude` named this package (bare match).
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
        ">>>".t_yellow().bold(),
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
    failures()
        .lock()
        .map(|log| !log.is_empty())
        .unwrap_or(false)
}

/// Gentoo-style end-of-run failure summary. Returns true if anything
/// was printed, i.e. if the run had failures.
pub(crate) fn print_failure_summary() -> bool {
    let Ok(log) = failures().lock() else {
        return false;
    };
    if log.is_empty() {
        return false;
    }
    eprintln!();
    eprintln!(
        "{} The following {} package(s) failed to build or install:",
        " *".t_red().bold(),
        log.len()
    );
    eprintln!();
    for (atom, reason) in log.iter() {
        eprintln!(
            "  {} {}",
            atom.t_red().bold(),
            format!("({})", reason).dimmed()
        );
    }
    eprintln!();
    eprintln!(
        "{} Everything else in this run completed. Retry just the failures with {}.",
        " *".t_yellow().bold(),
        "emerge --resume".t_cyan()
    );
    true
}
