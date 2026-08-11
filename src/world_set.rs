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
/// always listed and skipped (needs `emerge <pkg> --abs` by hand).
///
/// Entries with no resolvable prefix come in two flavors, handled
/// differently:
/// - A bare entry with *no* prefix at all means nothing could be
///   determined about it when it was written (not installed, not in any
///   sync DB at the time) — there's nothing to lose by just trying the
///   normal install path (official repos first, AUR on a miss), so this
///   always happens.
/// - An `Err/` entry means the package *is* installed, just from a
///   source pacman/aura couldn't identify (e.g. built by hand outside
///   aura-emerge). That's a bit more suspect, so by default these are
///   only listed; pass `err_inst` (the CLI's `--err-inst`) to have them
///   go through the same normal-resolution path instead.
///
/// Whatever gets installed this way has its world.set entry corrected to
/// the real resolved prefix, since `Err/`/bare was never accurate.
///
/// Returns Ok(true) if everything resolvable installed cleanly (or this
/// was a --pretend run), Ok(false) if something failed.
pub(crate) fn provision_from_world_set(pretend: bool, ask: bool, verbose: bool, err_inst: bool, no_sandbox: bool, off_src_regen: bool) -> Result<bool> {
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
    // Installed from somewhere unidentifiable (Err/) — only listed unless --err-inst.
    let mut err_missing: Vec<String> = Vec::new();
    // No prefix at all (not installed, no repo found when written) — always retried.
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

    // Bare entries always get a normal-resolution attempt; Err/ entries
    // only join in when --err-inst is passed.
    let mut to_resolve: Vec<String> = bare_missing.clone();
    if err_inst {
        to_resolve.extend(err_missing.iter().cloned());
    }

    // Resolve up front (official repos vs. AUR) so the plan below reflects
    // where each package will actually come from, instead of a generic
    // "unknown" bucket.
    let mut resolved_official: Vec<String> = Vec::new();
    let mut resolved_aur: Vec<String> = Vec::new();
    if !to_resolve.is_empty() {
        let (found, missing) = probe_official_split(&to_resolve);
        resolved_official = found.into_iter().map(|p| p.name).collect();
        resolved_aur = missing;
    }

    // Err/ entries left un-attempted this run (only relevant without --err-inst).
    let unresolved_listed: Vec<String> = if err_inst { Vec::new() } else { err_missing.clone() };

    let total = official_missing.len() + aur_missing.len() + abs_missing.len()
        + unresolved_listed.len() + resolved_official.len() + resolved_aur.len();
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
    for p in &resolved_official {
        println!("[{} {:<4}] {} (source was unresolved — found in official repos)", "ebuild".green(), "N".green().bold(), p.green().bold());
    }
    for p in &resolved_aur {
        println!("[{} {:<4}] {} (source was unresolved — will try the AUR)", "ebuild".green(), "N".cyan().bold(), p.cyan().bold());
    }
    for p in &abs_missing {
        println!("[{} {:<4}] {} (built from ABS — needs `emerge {} --abs`)", "ebuild".green(), "N".yellow().bold(), p.yellow().bold(), p);
    }
    for p in &unresolved_listed {
        println!("[{} {:<4}] {} (installed from an unknown source — retry with --err-inst, or install manually)", "ebuild".green(), "N".red().bold(), p.red().bold());
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
        // aur_install() builds+installs via the bwrap-isolated path
        // (see packages.rs) and, unlike the old `aura -A`, always leaves
        // the explicit bit set on success — no separate mark_asexplicit()
        // call needed here the way the old `aura -A` path required.
        if !aur_install(&aur_missing, false, ask, false, false, false, no_sandbox, off_src_regen) {
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
            // Now that the real repo is known, fix the world.set entry
            // (it was previously Err/<name> or a bare <name>).
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
        if aur_install(&resolved_aur, false, ask, false, false, false, no_sandbox, off_src_regen) {
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
            "{} {} package(s) were built from ABS and can't be reproduced unattended — \
            install them yourself: `emerge <pkg> --abs`",
            " *".yellow().bold(), abs_missing.len()
        );
        for p in &abs_missing { eprintln!("     {}", p); }
    }
    if !unresolved_listed.is_empty() {
        eprintln!(
            "{} {} package(s) are installed from an unknown source — retry with \
            `--err-inst` to attempt the normal official/AUR install path, or install manually.",
            " *".yellow().bold(), unresolved_listed.len()
        );
        for p in &unresolved_listed { eprintln!("     {}", p); }
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
        // See regen_set() below for why we pass the existing abs/aur
        // prefix through as a fallback instead of None.
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

/// Re-resolve repository prefixes for every entry in a custom set file
/// (`/etc/emerge/sets.d/<name>.set`), the same idea as `regen_world_set()`
/// but for a set instead of world.set.
///
/// By default (`sort == false`) this preserves the file exactly as
/// written: line order, `#` comments, and blank-line grouping all survive
/// — each package line just has its repo prefix re-resolved in place.
/// Pass `sort == true` (the `--regen-sort` flag) to instead collapse the
/// file down to a flat, deduplicated, alphabetically sorted list of
/// entries — which, unavoidably, drops the comments and blank-line
/// grouping, since there's nothing sensible to sort them relative to.
pub(crate) fn regen_set(name: &str, sort: bool) -> Result<()> {
    println!("{} Regenerating prefixes for @{}...", ">>>".green().bold(), name);

    let path = format!("{}/{}.set", SETS_DIR, name);
    if !is_safe_path(&path) {
        bail!("{} is a symlink — refusing to modify", path);
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("no such set: @{} (expected {})", name, path))?;

    let raw_lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .collect();

    if raw_lines.iter().all(|l| l.trim().is_empty()) {
        println!(">>> @{} is empty or does not exist ({}) — nothing to regenerate.", name, path);
        return Ok(());
    }

    let mut changed = 0usize;
    // Line-for-line rewrite used when `sort == false`: comments and blank
    // lines are copied through untouched, package lines get their prefix
    // patched in place.
    let mut rewritten: Vec<String> = Vec::new();
    // Package entries only (post re-resolution, in original order), used
    // as the input to `--regen-sort`.
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
        // Preserve an existing abs/aur prefix as a fallback if
        // re-resolution can't find a live repo — otherwise a regen
        // collapses a known-origin entry down to a generic, henceforth
        // indistinguishable "Err/<name>". See regen_world_set() for the
        // same fix on world.set proper.
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
        bail!("{} is a symlink — refusing to write", tmp);
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

// ── --undo (last action) state ─────────────────────────────────────────────
//
// Remembers the most recent successful install or unmerge so `emerge --undo`
// can reverse it. Deliberately narrow in scope: install-undo only ever
// covers packages recorded as brand new (never an upgrade/reinstall), and
// unmerge-undo just reinstalls the removed atoms via the normal path — it
// can't restore an exact prior version if that's no longer current in the
// repo/AUR. Only one step of history is kept, same as --resume.

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

/// Save the kind of action plus the affected atoms as the undoable state.
/// Best-effort — a failure here shouldn't abort the (already completed)
/// real operation, it just means `--undo` won't have anything to work with.
pub(crate) fn save_last_action(kind: LastAction, atoms: &[String]) {
    if atoms.is_empty() { return; }
    if !is_safe_path(LASTACTION_TMP) || !is_safe_path(LASTACTION_FILE) {
        eprintln!(">>> Warning: refusing to save undo state — symlink detected");
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

/// Drop the saved undo state — call once `--undo` has acted on it, so the
/// same action can't be replayed a second time.
pub(crate) fn clear_last_action() {
    let _ = Command::new(SUDO_BIN).args([RM_BIN, "-f", LASTACTION_FILE]).status();
}

/// Load the saved undo state, if any: (kind tag, affected atoms).
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