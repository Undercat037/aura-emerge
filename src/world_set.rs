//! Handling of /etc/emerge/world.set: the explicit-install tracking file.
//!
//! Split out of main.rs — covers repo-prefix resolution for world.set
//! entries, the add/remove/regen/write operations on the file itself,
//! custom package sets under /etc/emerge/sets.d/, and declarative
//! provisioning from world.set (bare `emerge @world`, no -u).

use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

use crate::*;

/// Get the repository a package was installed from (e.g. "cachyos-extra-v3", "aur").
///
/// Forces LC_ALL=C since pacman's field names are locale-dependent.
/// Tries `pacman -Qi` (installed pkg, no "Repository" line = AUR/foreign),
/// then falls back to `pacman -Si` for not-yet-installed packages.
pub(crate) fn get_pkg_repo(pkg: &str) -> Option<String> {
    let bare = pkg.split('/').last().unwrap_or(pkg);

    fn pacman_c(args: &[&str]) -> Option<String> {
        let out = Command::new("/usr/bin/pacman")
            .args(args)
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        }
    }

    fn first_repo(stdout: &str) -> Option<String> {
        for line in stdout.lines() {
            // -Qi (C locale) uses "Installed From", -Si uses "Repository"
            if line.starts_with("Installed From") || line.starts_with("Repository") {
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    let r = val.trim().to_string();
                    if !r.is_empty() { return Some(r); }
                }
            }
        }
        None
    }

    // 1. Local DB — authoritative for installed packages
    if let Some(stdout) = pacman_c(&["-Qi", bare]) {
        return first_repo(&stdout);  // None here = locally built, no repo field
    }

    // 2. Sync DB — for packages not yet installed (--select etc.)
    if let Some(stdout) = pacman_c(&["-Si", bare]) {
        return first_repo(&stdout);
    }

    None
}

/// Format package entry for world.set.
/// - Known repo (official/AUR overlay)  → "repo/name"
/// - Locally built (pacman "None")       → "forced_prefix/name" if known, else "Err/name"
/// - Not installed, not in sync DB       → bare "name"
pub(crate) fn pkg_world_entry(pkg: &str, forced_prefix: Option<&str>) -> String {
    let bare = pkg.split('/').last().unwrap_or(pkg);
    match get_pkg_repo(bare) {
        Some(repo) if repo != "None" => format!("{}/{}", repo, bare),
        Some(_) /* "None" = local build */ => {
            let prefix = forced_prefix.unwrap_or("Err");
            format!("{}/{}", prefix, bare)
        }
        None => bare.to_string(),
    }
}

// ── custom sets (/etc/emerge/sets.d/<name>.set, invoked as @<name>) ────────────

/// Is `name` (the part after '@') safe to use as a set filename?
/// Deliberately conservative — this becomes part of a filesystem path.
pub(crate) fn valid_set_name(name: &str) -> bool {
    !name.is_empty()
        && name != "world"
        && name != "preserved-rebuild"
        && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Read /etc/emerge/sets.d/<name>.set: one package atom per line, blank
/// lines and '#' comments ignored. Entries are passed through the same
/// validate_pkg() every other package name is, so a malformed set file
/// can't smuggle a flag-looking token into an aura/pacman invocation.
pub(crate) fn read_custom_set(name: &str) -> Result<Vec<String>> {
    let path = format!("{}/{}.set", SETS_DIR, name);

    if !is_safe_path(&path) {
        bail!("{} is a symlink — refusing to read", path);
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("no such set: @{} (expected {})", name, path))?;

    let pkgs: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| {
            if validate_pkg(l) {
                true
            } else {
                eprintln!(">>> Warning: invalid entry in @{} (skipped): {}", name, l);
                false
            }
        })
        .collect();

    Ok(pkgs)
}

/// List every custom set under SETS_DIR (bare names, no ".set", no '@'),
/// sorted. Used by `--list-sets` and by shell completion for `@<TAB>`.
pub(crate) fn list_custom_sets() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(SETS_DIR) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("set") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

// ── declarative provisioning from world.set (bare `emerge @world`) ─────────────

/// `emerge @world` with no `-u`: install whatever world.set lists that
/// isn't already on this system. Unlike the `-u @world` full upgrade,
/// this never touches a package that's already installed and never
/// consults the sync databases on its own — it's meant for "this machine
/// is missing packages world.set says it should have" (a fresh install,
/// or one that fell behind a dotfiles-tracked world.set), not for
/// upgrading what's already there.
///
/// Each entry's recorded repo prefix decides how it gets installed:
/// official-repo entries go through `aura -S`, `aur/` entries through
/// `aura -A` (with the usual PKGBUILD scan first). `abs/` entries were
/// locally built from ABS and can't be reproduced unattended, so they're
/// listed and skipped — same for entries with no resolvable prefix
/// (`Err/` or bare names) — the person running this decides how to
/// handle those themselves (`emerge <pkg> --abs`, or a plain `emerge
/// <pkg>` once the source is known).
///
/// Returns Ok(true) if everything resolvable installed cleanly (or this
/// was a --pretend run), Ok(false) if something failed.
pub(crate) fn provision_from_world_set(pretend: bool, ask: bool, verbose: bool) -> Result<bool> {
    println!("{} Provisioning system from world.set...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink — refusing to read", WORLD_SET_FILE);
    }

    let entries: Vec<String> = match fs::File::open(WORLD_SET_FILE) {
        Ok(file) => io::BufReader::new(file)
            .lines()
            .map_while(io::Result::ok)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => {
            println!(">>> world.set not found — nothing to provision.");
            return Ok(true);
        }
    };

    if entries.is_empty() {
        println!(">>> world.set is empty — nothing to provision.");
        return Ok(true);
    }

    // Packages already on this system are left alone entirely.
    let installed: HashSet<String> = Command::new(PACMAN_BIN)
        .arg("-Qq")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut official_missing: Vec<String> = Vec::new();
    let mut aur_missing: Vec<String> = Vec::new();
    let mut abs_missing: Vec<String> = Vec::new();
    let mut unresolved_missing: Vec<String> = Vec::new();

    for entry in &entries {
        let mut parts = entry.splitn(2, '/');
        let first = parts.next().unwrap_or("");
        let rest = parts.next();
        let (prefix, bare) = match rest {
            Some(name) => (Some(first), name.to_string()),
            None => (None, first.to_string()),
        };

        if installed.contains(&bare) {
            continue;
        }

        match prefix {
            Some("aur") => aur_missing.push(bare),
            Some("abs") => abs_missing.push(bare),
            Some("Err") | None => unresolved_missing.push(bare),
            Some(_official_repo) => official_missing.push(bare),
        }
    }

    let total = official_missing.len() + aur_missing.len() + abs_missing.len() + unresolved_missing.len();
    if total == 0 {
        println!("{} Nothing to do — every world.set package is already installed.", ">>>".green().bold());
        return Ok(true);
    }

    println!();
    println!("{}", "These are the packages that would be merged, in order:".green().bold());
    println!();
    println!("Calculating dependencies... done!");
    println!();
    for p in official_missing.iter().chain(aur_missing.iter()) {
        println!("[{} {:<4}] {}", "ebuild".green(), "N".green().bold(), p.green().bold());
    }
    for p in &abs_missing {
        println!("[{} {:<4}] {} (built from ABS — needs `emerge {} --abs`)", "ebuild".green(), "N".yellow().bold(), p.yellow().bold(), p);
    }
    for p in &unresolved_missing {
        println!("[{} {:<4}] {} (source unknown — install manually)", "ebuild".green(), "N".red().bold(), p.red().bold());
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), total);
    println!();

    if pretend {
        return Ok(true);
    }

    let mut overall_ok = true;

    if !official_missing.is_empty() {
        println!("{} Installing {} package(s) from official repos...", ">>>".green().bold(), official_missing.len());
        let mut args = vec!["-S", "--needed"];
        if verbose { args.push("--verbose"); }
        if !ask { args.push("--noconfirm"); }
        if !run_cmd(AURA_BIN, &args, &official_missing) {
            overall_ok = false;
            eprintln!(">>> Warning: some official-repo package(s) failed to install.");
        }
    }

    if !aur_missing.is_empty() {
        println!("{} Installing {} AUR package(s)...", ">>>".green().bold(), aur_missing.len());
        scan_aur_pkgbuilds_or_abort(&aur_missing);
        let mut args = vec!["-A", "--needed"];
        if !ask { args.push("--noconfirm"); }
        if run_cmd(AURA_BIN, &args, &aur_missing) {
            // aura -A doesn't always leave the explicit bit set the way
            // `pacman -S` does — force it so world.set and pacman's own
            // bookkeeping stay in agreement.
            mark_asexplicit(&aur_missing);
        } else {
            overall_ok = false;
            eprintln!(">>> Warning: some AUR package(s) failed to install.");
        }
    }

    if !abs_missing.is_empty() {
        eprintln!(
            "{} {} package(s) were built from ABS and can't be reproduced unattended — \
            install them yourself: `emerge <pkg> --abs`",
            " *".yellow().bold(), abs_missing.len()
        );
        for p in &abs_missing { eprintln!("     {}", p); }
    }
    if !unresolved_missing.is_empty() {
        eprintln!(
            "{} {} package(s) have no recorded source — install manually: `emerge <pkg>`",
            " *".yellow().bold(), unresolved_missing.len()
        );
        for p in &unresolved_missing { eprintln!("     {}", p); }
    }

    Ok(overall_ok)
}


// ── world.set ─────────────────────────────────────────────────────────────────

/// Re-resolve repository prefixes for every package in world.set.
/// Reads current entries, calls get_pkg_repo() for each, rewrites the file.
/// Useful after locale fixes, repo migrations, or manual edits.
pub(crate) fn regen_world_set() -> Result<()> {
    println!("{} Regenerating world.set repository prefixes...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink — refusing to modify", WORLD_SET_FILE);
    }

    let file = fs::File::open(WORLD_SET_FILE)
        .with_context(|| format!("cannot open {}", WORLD_SET_FILE))?;

    let entries: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let mut updated: Vec<String> = Vec::new();
    let mut changed = 0usize;

    for entry in &entries {
        let bare = entry.split('/').last().unwrap_or(entry);
        let new_entry = pkg_world_entry(bare, None);
        if &new_entry != entry {
            println!("  {} -> {}", entry, new_entry);
            changed += 1;
        }
        updated.push(new_entry);
    }

    if changed == 0 {
        println!("{} world.set is already up to date.", ">>>".green().bold());
        return Ok(());
    }

    updated.sort();
    write_world_set(&updated)?;
    println!("{} world.set updated ({} entries changed).", ">>>".green().bold(), changed);
    Ok(())
}

pub(crate) fn add_to_world_set(packages: &[String], forced_prefix: Option<&str>) -> Result<()> {
    println!("{} Adding to world.set...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink — refusing to read", WORLD_SET_FILE);
    }

    // current_set keyed by bare package name → full "repo/name" or "name" entry
    let mut current_set: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
        for line in io::BufReader::new(file).lines().map_while(Result::ok) {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() && validate_pkg(&trimmed) {
                // bare name is the part after the last '/'
                let bare = trimmed.split('/').last().unwrap_or(&trimmed).to_string();
                current_set.insert(bare, trimmed);
            }
        }
    }

    let mut changed = false;
    for pkg in packages {
        let bare = pkg.split('/').last().unwrap_or(pkg).to_string();
        let entry = pkg_world_entry(&bare, forced_prefix);
        // Always overwrite: re-resolves repo on every install so stale entries
        // (e.g. "aur/nano" left from a locale-parsing bug) get corrected.
        let stale = current_set.get(&bare).map(|e| e != &entry).unwrap_or(true);
        if stale {
            current_set.insert(bare, entry);
            changed = true;
        }
    }

    if !changed { return Ok(()); }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted)
}

pub(crate) fn remove_from_world_set(packages: &[String]) -> Result<()> {
    println!(">>> Removing from world.set...");

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink — refusing to read", WORLD_SET_FILE);
    }

    // key = bare name, value = full entry
    let mut current_set: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
        for line in io::BufReader::new(file).lines().map_while(Result::ok) {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() && validate_pkg(&trimmed) {
                let bare = trimmed.split('/').last().unwrap_or(&trimmed).to_string();
                current_set.insert(bare, trimmed);
            }
        }
    }

    let mut changed = false;
    for pkg in packages {
        // match on bare name regardless of whether user passed "repo/pkg" or "pkg"
        let bare = pkg.split('/').last().unwrap_or(pkg).to_string();
        if current_set.remove(&bare).is_some() {
            changed = true;
        }
    }

    if !changed { return Ok(()); }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted)
}


// ── --resume state ───────────────────────────────────────────────────────────
//
// Persists the exact argv-equivalent (flags + resolved package list) of the
// last non-pretend install/@world update, so `emerge --resume` can literally
// replay it rather than just running a blind `pacman -Syu`. Cleared on
// success; left in place on failure/interruption so the next --resume picks
// up where things left off.

/// Save the given argv-style tokens as the resumable state. Best-effort —
/// a failure here shouldn't abort the (already in-progress) real operation.
pub(crate) fn save_resume_state(args: &[String]) {
    if !is_safe_path(RESUME_TMP) || !is_safe_path(RESUME_FILE) {
        eprintln!(">>> Warning: refusing to save resume state — symlink detected");
        return;
    }

    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", RESUME_TMP]).status();

    let child_proc = Command::new(SUDO_BIN)
        .arg(TEE_BIN)
        .arg(RESUME_TMP)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn();

    let write_ok = match child_proc {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                for a in args {
                    let _ = writeln!(stdin, "{}", a);
                }
            }
            child.wait().map(|s| s.success()).unwrap_or(false)
        }
        Err(_) => false,
    };

    if !write_ok {
        eprintln!(">>> Warning: failed to save resume state");
        return;
    }

    let _ = Command::new(SUDO_BIN)
        .args([MV_BIN, RESUME_TMP, RESUME_FILE])
        .status();
}

/// Drop the saved resume state — call after a fully successful operation.
pub(crate) fn clear_resume_state() {
    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", RESUME_FILE]).status();
}

/// Load the saved resume state, if any. Returns None if there's nothing to
/// resume (file missing, empty, or a symlink we refuse to follow).
pub(crate) fn load_resume_state() -> Option<Vec<String>> {
    if !is_safe_path(RESUME_FILE) {
        return None;
    }
    let file = fs::File::open(RESUME_FILE).ok()?;
    let lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() { None } else { Some(lines) }
}

pub(crate) fn write_world_set(packages: &[String]) -> Result<()> {
    if !is_safe_path(WORLD_SET_TMP) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_TMP);
    }
    if !is_safe_path(WORLD_SET_FILE) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_FILE);
    }

    // Remove stale tmp file
    let _ = Command::new(SUDO_BIN)
        .args([RM_BIN, "-f", WORLD_SET_TMP])
        .status();

    let write_ok = {
        let child_proc = Command::new(SUDO_BIN)
            .arg(TEE_BIN)
            .arg(WORLD_SET_TMP)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn();

        match child_proc {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    for pkg in packages {
                        if let Err(e) = writeln!(stdin, "{}", pkg) {
                            eprintln!(">>> Error writing to world.set pipeline: {}", e);
                        }
                    }
                }
                match child.wait() {
                    Ok(s) if s.success() => true,
                    Ok(_) => { eprintln!(">>> Error: sudo tee exited with non-zero status."); false }
                    Err(e) => { eprintln!(">>> Error waiting for sudo tee: {}", e); false }
                }
            }
            Err(e) => { eprintln!(">>> Error: Failed to spawn sudo tee: {}", e); false }
        }
    };

    if !write_ok {
        bail!("failed to write {} via sudo tee", WORLD_SET_TMP);
    }

    let status = Command::new(SUDO_BIN)
        .args([MV_BIN, WORLD_SET_TMP, WORLD_SET_FILE])
        .status()
        .context("sudo mv could not be spawned")?;

    if !status.success() {
        bail!("sudo mv failed when finalizing world.set");
    }

    println!(">>> world.set updated.");
    Ok(())
}