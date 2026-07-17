//! Handling of /etc/emerge/world.set: the explicit-install tracking file.
//!
//! Split out of main.rs — covers repo-prefix resolution for world.set
//! entries and the add/remove/regen/write operations on the file itself.

use anyhow::{bail, Context, Result};
use colored::Colorize;
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