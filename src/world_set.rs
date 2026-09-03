//! /etc/emerge/world.set: explicit-install tracking, custom sets, and
//! `@world` provisioning (no -u).

use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

use crate::*;

/// Repo a package came from (e.g. "extra", "aur"). LC_ALL=C.
/// Tries -Qi first, then -Si for not-yet-installed.
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
            if line.starts_with("Installed From") || line.starts_with("Repository") {
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    let r = val.trim().to_string();
                    if !r.is_empty() { return Some(r); }
                }
            }
        }
        None
    }

    // Local DB first; None = local build with no repo field.
    if let Some(stdout) = pacman_c(&["-Qi", bare]) {
        return first_repo(&stdout);
    }

    // Sync DB for not-yet-installed (--select etc.)
    if let Some(stdout) = pacman_c(&["-Si", bare]) {
        return first_repo(&stdout);
    }

    None
}

/// world.set entry: "repo/name", "Err/name" (local), or bare "name".
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

/// Safe set name (becomes a filesystem path under sets.d/).
pub(crate) fn valid_set_name(name: &str) -> bool {
    !name.is_empty()
        && name != "world"
        && name != "preserved-rebuild"
        && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Read sets.d/<name>.set (one atom/line, # comments; validate_pkg each).
pub(crate) fn read_custom_set(name: &str) -> Result<Vec<String>> {
    let path = format!("{}/{}.set", SETS_DIR, name);

    if !is_safe_path(&path) {
        bail!("{} is a symlink - refusing to read", path);
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

/// Read a --batchinstall list (same format as a custom set, any path).
pub(crate) fn read_batch_file(path: &str) -> Result<Vec<String>> {
    if !is_safe_path(path) {
        bail!("{} is a symlink - refusing to read", path);
    }

    let file = fs::File::open(path)
        .with_context(|| format!("could not open batch file: {}", path))?;

    let pkgs: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| {
            if validate_pkg(l) {
                true
            } else {
                eprintln!(">>> Warning: invalid entry in {} (skipped): {}", path, l);
                false
            }
        })
        .collect();

    Ok(pkgs)
}

/// Bare set names under SETS_DIR, sorted (--list-sets / completion).
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

/// Provision missing packages from world.set (bare `@world`, no -u).
/// Prefix picks the source; `abs/` is listed only; bare always resolved;
/// `Err/` only with err_install. Fixes world.set prefixes on success.
pub(crate) fn provision_from_world_set(pretend: bool, ask: bool, verbose: bool, err_install: bool, no_sandbox: bool, skip_srcinfo_regen: bool, unshare_net_build: bool) -> Result<bool> {
    println!("{} Provisioning system from world.set...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
    }

    let entries: Vec<String> = match fs::File::open(WORLD_SET_FILE) {
        Ok(file) => io::BufReader::new(file)
            .lines()
            .map_while(io::Result::ok)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => {
            println!(">>> world.set not found - nothing to provision.");
            return Ok(true);
        }
    };

    if entries.is_empty() {
        println!(">>> world.set is empty - nothing to provision.");
        return Ok(true);
    }

    // Already installed → skip.
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
    // Err/: list only unless --err-install. Bare: always retry.
    let mut err_missing: Vec<String> = Vec::new();
    let mut bare_missing: Vec<String> = Vec::new();

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
            Some("Err") => err_missing.push(bare),
            None => bare_missing.push(bare),
            Some(_official_repo) => official_missing.push(bare),
        }
    }

    // Bare always resolved; Err/ only with --err-install.
    let mut to_resolve: Vec<String> = bare_missing.clone();
    if err_install {
        to_resolve.extend(err_missing.iter().cloned());
    }

    // Resolve bare/Err so the plan shows real sources.
    let mut resolved_official: Vec<String> = Vec::new();
    let mut resolved_aur: Vec<String> = Vec::new();
    if !to_resolve.is_empty() {
        let (found, missing) = probe_official_split(&to_resolve);
        resolved_official = found.into_iter().map(|p| p.name).collect();
        resolved_aur = missing;
    }

    let unresolved_listed: Vec<String> = if err_install { Vec::new() } else { err_missing.clone() };

    let total = official_missing.len() + aur_missing.len() + abs_missing.len()
        + unresolved_listed.len() + resolved_official.len() + resolved_aur.len();
    if total == 0 {
        println!("{} Nothing to do - every world.set package is already installed.", ">>>".green().bold());
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
    for p in &resolved_official {
        println!("[{} {:<4}] {} (source was unresolved - found in official repos)", "ebuild".green(), "N".green().bold(), p.green().bold());
    }
    for p in &resolved_aur {
        println!("[{} {:<4}] {} (source was unresolved - will try the AUR)", "ebuild".green(), "N".cyan().bold(), p.cyan().bold());
    }
    for p in &abs_missing {
        println!("[{} {:<4}] {} (built from ABS - needs `emerge {} --abs`)", "ebuild".green(), "N".yellow().bold(), p.yellow().bold(), p);
    }
    for p in &unresolved_listed {
        println!("[{} {:<4}] {} (installed from an unknown source - retry with --err-install, or install manually)", "ebuild".green(), "N".red().bold(), p.red().bold());
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
        let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed"];
        if verbose { args.push("--verbose"); }
        if !ask { args.push("--noconfirm"); }
        if !run_cmd(SUDO_BIN, &args, &official_missing) {
            overall_ok = false;
            eprintln!(">>> Warning: some official-repo package(s) failed to install.");
        }
    }

    if !aur_missing.is_empty() {
        println!("{} Installing {} AUR package(s)...", ">>>".green().bold(), aur_missing.len());
        scan_aur_pkgbuilds_or_abort(&aur_missing);
        // aur_install sets explicit on success; no mark_asexplicit needed.
        if !aur_install(&aur_missing, false, ask, false, false, false, no_sandbox, skip_srcinfo_regen, unshare_net_build, false) {
            overall_ok = false;
            eprintln!(">>> Warning: some AUR package(s) failed to install.");
        }
    }

    if !resolved_official.is_empty() {
        println!(
            "{} Installing {} previously-unresolved package(s) from official repos...",
            ">>>".green().bold(), resolved_official.len()
        );
        let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed"];
        if verbose { args.push("--verbose"); }
        if !ask { args.push("--noconfirm"); }
        if run_cmd(SUDO_BIN, &args, &resolved_official) {
            // Fix world.set prefix now that the real repo is known.
            if let Err(e) = add_to_world_set(&resolved_official, None) {
                eprintln!(">>> Warning: package(s) installed but world.set was not updated: {:#}", e);
            }
        } else {
            overall_ok = false;
            eprintln!(">>> Warning: some previously-unresolved package(s) failed to install from official repos.");
        }
    }

    if !resolved_aur.is_empty() {
        println!(
            "{} Installing {} previously-unresolved package(s) via the AUR...",
            ">>>".green().bold(), resolved_aur.len()
        );
        scan_aur_pkgbuilds_or_abort(&resolved_aur);
        if aur_install(&resolved_aur, false, ask, false, false, false, no_sandbox, skip_srcinfo_regen, unshare_net_build, false) {
            if let Err(e) = add_to_world_set(&resolved_aur, Some("aur")) {
                eprintln!(">>> Warning: package(s) installed but world.set was not updated: {:#}", e);
            }
        } else {
            overall_ok = false;
            eprintln!(
                ">>> Warning: some previously-unresolved package(s) were not found anywhere \
                (neither official repos nor AUR) or failed to install."
            );
        }
    }

    if !abs_missing.is_empty() {
        eprintln!(
            "{} {} package(s) were built from ABS and can't be reproduced unattended - \
            install them yourself: `emerge <pkg> --abs`",
            " *".yellow().bold(), abs_missing.len()
        );
        for p in &abs_missing { eprintln!("     {}", p); }
    }
    if !unresolved_listed.is_empty() {
        eprintln!(
            "{} {} package(s) are installed from an unknown source - retry with \
            `--err-install` to attempt the normal official/AUR install path, or install manually.",
            " *".yellow().bold(), unresolved_listed.len()
        );
        for p in &unresolved_listed { eprintln!("     {}", p); }
    }

    Ok(overall_ok)
}


// ── world.set ─────────────────────────────────────────────────────────────────

/// Re-resolve repo prefixes for every world.set entry.
pub(crate) fn regen_world_set() -> Result<()> {
    println!("{} Regenerating world.set repository prefixes...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink - refusing to modify", WORLD_SET_FILE);
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
        // Keep abs/aur prefix if re-resolution can't find a live repo.
        let old_prefix = entry.split('/').next().filter(|p| *p == "abs" || *p == "aur");
        let new_entry = pkg_world_entry(bare, old_prefix);
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

/// Re-resolve prefixes in a custom set. sort=true also alphabetizes
/// (drops comments/blank-line grouping).
pub(crate) fn regen_set(name: &str, sort: bool) -> Result<()> {
    println!("{} Regenerating prefixes for @{}...", ">>>".green().bold(), name);

    let path = format!("{}/{}.set", SETS_DIR, name);
    if !is_safe_path(&path) {
        bail!("{} is a symlink - refusing to modify", path);
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("no such set: @{} (expected {})", name, path))?;

    let raw_lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .collect();

    if raw_lines.iter().all(|l| l.trim().is_empty()) {
        println!(">>> @{} is empty or does not exist ({}) - nothing to regenerate.", name, path);
        return Ok(());
    }

    let mut changed = 0usize;
    // sort=false: rewrite in place (keep comments/blanks).
    let mut rewritten: Vec<String> = Vec::new();
    // Package entries only (input for --regen-sort).
    let mut pkg_entries: Vec<String> = Vec::new();

    for raw in &raw_lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            rewritten.push(raw.clone());
            continue;
        }
        if !validate_pkg(trimmed) {
            eprintln!(">>> Warning: invalid entry in @{} (left as-is): {}", name, trimmed);
            rewritten.push(raw.clone());
            continue;
        }

        let bare = trimmed.split('/').last().unwrap_or(trimmed);
        // Keep abs/aur if re-resolution misses (same as regen_world_set).
        let old_prefix = trimmed.split('/').next().filter(|p| *p == "abs" || *p == "aur");
        let new_entry = pkg_world_entry(bare, old_prefix);
        if new_entry != trimmed {
            println!("  {} -> {}", trimmed, new_entry);
            changed += 1;
        }
        pkg_entries.push(new_entry.clone());
        rewritten.push(new_entry);
    }

    let out_lines: Vec<String> = if sort {
        let mut sorted = pkg_entries.clone();
        sorted.sort();
        sorted.dedup();
        let order_changed = sorted != pkg_entries;
        if changed == 0 && !order_changed {
            println!("{} @{} is already up to date (sorted).", ">>>".green().bold(), name);
            return Ok(());
        }
        sorted
    } else {
        if changed == 0 {
            println!("{} @{} is already up to date.", ">>>".green().bold(), name);
            return Ok(());
        }
        rewritten
    };

    let tmp = format!("{}.tmp", path);
    if !is_safe_path(&tmp) {
        bail!("{} is a symlink - refusing to write", tmp);
    }
    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", &tmp]).status();

    let write_ok = {
        let child_proc = Command::new(SUDO_BIN)
            .arg(TEE_BIN)
            .arg(&tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn();
        match child_proc {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    for line in &out_lines {
                        let _ = writeln!(stdin, "{}", line);
                    }
                }
                child.wait().map(|s| s.success()).unwrap_or(false)
            }
            Err(_) => false,
        }
    };
    if !write_ok {
        bail!("failed to write {} via sudo tee", tmp);
    }

    let status = Command::new(SUDO_BIN)
        .args([MV_BIN, &tmp, &path])
        .status()
        .context("sudo mv could not be spawned")?;
    if !status.success() {
        bail!("sudo mv failed when finalizing {}", path);
    }

    if sort {
        println!("{} @{} updated ({} entries changed, re-sorted).", ">>>".green().bold(), name, changed);
    } else {
        println!("{} @{} updated ({} entries changed).", ">>>".green().bold(), name, changed);
    }
    Ok(())
}

pub(crate) fn add_to_world_set(packages: &[String], forced_prefix: Option<&str>) -> Result<()> {
    println!("{} Adding to world.set...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
    }

    // bare name → full "repo/name" entry
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
        let bare = pkg.split('/').last().unwrap_or(pkg).to_string();
        let entry = pkg_world_entry(&bare, forced_prefix);
        // Overwrite so stale prefixes get corrected.
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
        bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
    }

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


// --resume: argv of the last non-pretend install; cleared on success.

/// Save resume argv (best-effort; failure must not abort the real op).
pub(crate) fn save_resume_state(args: &[String]) {
    if !is_safe_path(RESUME_TMP) || !is_safe_path(RESUME_FILE) {
        eprintln!(">>> Warning: refusing to save resume state - symlink detected");
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

/// Clear resume state after a fully successful operation.
pub(crate) fn clear_resume_state() {
    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", RESUME_FILE]).status();
}

/// Load resume argv, if any.
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

// --undo: one step of install/unmerge history (no version snapshot for -u).

pub(crate) enum LastAction {
    Install,
    Unmerge,
    Update,
}

impl LastAction {
    fn tag(&self) -> &'static str {
        match self {
            LastAction::Install => "install",
            LastAction::Unmerge => "unmerge",
            LastAction::Update => "update",
        }
    }
}

/// Save undo state (best-effort).
pub(crate) fn save_last_action(kind: LastAction, atoms: &[String]) {
    if atoms.is_empty() { return; }
    if !is_safe_path(LASTACTION_TMP) || !is_safe_path(LASTACTION_FILE) {
        eprintln!(">>> Warning: refusing to save undo state - symlink detected");
        return;
    }

    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", LASTACTION_TMP]).status();

    let child_proc = Command::new(SUDO_BIN)
        .arg(TEE_BIN)
        .arg(LASTACTION_TMP)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn();

    let write_ok = match child_proc {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = writeln!(stdin, "{}", kind.tag());
                for a in atoms {
                    let _ = writeln!(stdin, "{}", a);
                }
            }
            child.wait().map(|s| s.success()).unwrap_or(false)
        }
        Err(_) => false,
    };

    if !write_ok {
        eprintln!(">>> Warning: failed to save undo state");
        return;
    }

    let _ = Command::new(SUDO_BIN)
        .args([MV_BIN, LASTACTION_TMP, LASTACTION_FILE])
        .status();
}

/// Clear undo state after --undo acts on it.
pub(crate) fn clear_last_action() {
    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", LASTACTION_FILE]).status();
}

/// Load undo state: (kind tag, atoms).
pub(crate) fn load_last_action() -> Option<(String, Vec<String>)> {
    if !is_safe_path(LASTACTION_FILE) {
        return None;
    }
    let file = fs::File::open(LASTACTION_FILE).ok()?;
    let mut lines = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    let kind = lines.next()?;
    let atoms: Vec<String> = lines.collect();
    if atoms.is_empty() { None } else { Some((kind, atoms)) }
}

pub(crate) fn write_world_set(packages: &[String]) -> Result<()> {
    if !is_safe_path(WORLD_SET_TMP) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_TMP);
    }
    if !is_safe_path(WORLD_SET_FILE) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_FILE);
    }

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