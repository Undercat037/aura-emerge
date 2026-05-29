/*
Copyright (C) 2026 Undercat037
This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

aura-emerge: A gentoo-like wrapper for the aura AUR helper.
*/

use clap::Parser;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

// ── Binary paths ───────────────────────────────────────────────────────

const AURA_BIN:   &str = "/usr/bin/aura";
const PACMAN_BIN: &str = "/usr/bin/pacman";
const SUDO_BIN:   &str = "/usr/bin/sudo";
const TEE_BIN:    &str = "/usr/bin/tee";
const MV_BIN:     &str = "/usr/bin/mv";
const RM_BIN:     &str = "/usr/bin/rm";

// ── Files ─────────────────────────────────────────────────────────────────────

const WORLD_SET_FILE: &str = "/etc/emerge/world.set";
const WORLD_SET_TMP:  &str = "/etc/emerge/world.set.tmp";

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Portage-like wrapper for Arch Linux using Aura
#[derive(Parser, Debug)]
#[command(
    name = "emerge",
    bin_name = "emerge",
    about = "Portage-like wrapper for Arch Linux using Aura",
    version = "1.18.0 (aura-emerge)\nAuthor: Undercat037"
)]
struct Cli {
    /// Search for packages
    #[arg(short = 's', long)]
    search: bool,

    /// Sync package database
    #[arg(long)]
    sync: bool,

    /// Update packages
    #[arg(short = 'u', long)]
    update: bool,

    /// Remove orphans
    #[arg(short = 'c', long = "depclean")]
    depclean: bool,

    /// Remove specific packages
    #[arg(short = 'C', long = "unmerge")]
    unmerge: bool,

    /// Pretend (dry run)
    #[arg(short = 'p', long = "pretend")]
    pretend: bool,

    /// Ask before applying changes
    #[arg(short = 'a', long = "ask")]
    ask: bool,

    /// Install as dependency (no world.set)
    #[arg(short = '1', long = "oneshot")]
    oneshot: bool,

    /// Explicitly force AUR only
    #[arg(long = "aur")]
    aur: bool,

    /// Verbose output / detailed info in search mode (-sv = aura -Si/-Ai)
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Do not reinstall if already installed (pacman --needed)
    #[arg(short = 'n', long = "noreplace")]
    noreplace: bool,

    // Dummy flags for compatibility
    /// Consider the whole dependency tree
    #[arg(short = 'D', long = "deep")]
    deep: bool,
    
    /// Include installed pkgs with changed USE flags
    #[arg(short = 'N', long = "newuse")]
    newuse: bool,
    
    /// Reinstall all world pkgs
    #[arg(short = 'e', long = "emptytree")]
    emptytree: bool,

    /// Resume a previously interrupted merge
    #[arg(long = "resume")]
    resume: bool,

    /// Skip the first package in the resume list
    #[arg(long = "skipfirst")]
    skipfirst: bool,

    /// Remove packages not in world.set (prune)
    #[arg(long = "prune")]
    prune: bool,

    /// Regenerate package metadata cache (regen)
    #[arg(long = "regen")]
    regen: bool,

    /// Search package descriptions (--searchdesc)
    #[arg(long = "searchdesc")]
    searchdesc: bool,

    /// Explicitly add packages to world.set (--select)
    #[arg(long = "select")]
    select: bool,

    /// Remove packages from world.set without unmerging (--deselect)
    #[arg(long = "deselect")]
    deselect: bool,

    /// Pull in build-time dependencies (informational)
    #[arg(long = "with-bdeps")]
    with_bdeps: bool,

    /// Show verbose slot/conflict info (informational)
    #[arg(long = "verbose-conflicts")]
    verbose_conflicts: bool,

    /// Packages to install or '@world'
    packages: Vec<String>,
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_pkg(pkg: &str) -> bool {
    if pkg.starts_with('-') || pkg.contains("..") || pkg.contains("//") {
        return false;
    }
    pkg.chars()
        .all(|c| c.is_alphanumeric() || "@._+-/".contains(c))
}

fn validate_packages(packages: &[String]) -> Vec<String> {
    packages
        .iter()
        .filter(|p| {
            if !validate_pkg(p) {
                eprintln!(">>> Invalid package name (skipped): {}", p);
                false
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

// ── Binary existence check ────────────────────────────────────────────────────

/// Abort early if required binaries are missing.
fn check_binaries() {
    for bin in &[AURA_BIN, PACMAN_BIN, SUDO_BIN, TEE_BIN, MV_BIN, RM_BIN] {
        if !std::path::Path::new(bin).exists() {
            eprintln!(">>> Fatal: required binary not found: {}", bin);
            std::process::exit(1);
        }
    }
}

// ── Symlink guard ─────────────────────────────────────────────────────────────

/// Returns true if the path is safe (not a symlink, or does not exist yet).
fn is_safe_path(path: &str) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => !meta.file_type().is_symlink(),
        Err(_) => true,
    }
}

// ── Package info ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PkgInfo {
    name: String,
    version: String,
    is_new: bool,
}

/// Probe official repos in a single call using --print-format.
/// Returns Some(infos) if packages are found, None otherwise.
fn probe_official(pkgs: &[String]) -> Option<Vec<PkgInfo>> {
    let mut args = vec!["-Sp", "--print-format", "%n %v", "--color", "never"];
    let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
    args.extend_from_slice(&pkg_refs);

    let output = Command::new(AURA_BIN)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let infos: Vec<PkgInfo> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut parts = line.splitn(2, ' ');
            let name = parts.next().unwrap_or("").to_string();
            let version = parts.next().unwrap_or("").to_string();
            let is_new = !is_installed(&name);
            PkgInfo { name, version, is_new }
        })
        .collect();

    if infos.is_empty() { None } else { Some(infos) }
}

/// Fetch AUR package info via -Ai output parsing.
fn resolve_aur(pkgs: &[String]) -> Vec<PkgInfo> {
    let mut result = Vec::new();
    for pkg in pkgs {
        let output = Command::new(AURA_BIN)
            .args(["-Ai", pkg])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut name = pkg.clone();
            let mut version = String::from("?");
            for line in stdout.lines() {
                if line.starts_with("Name") {
                    if let Some(v) = line.split(':').nth(1) {
                        name = v.trim().to_string();
                    }
                } else if line.starts_with("Version") {
                    if let Some(v) = line.split(':').nth(1) {
                        version = v.trim().to_string();
                    }
                }
            }
            let is_new = !is_installed(&name);
            result.push(PkgInfo { name, version, is_new });
        }
    }
    result
}

/// Check if a package is currently installed.
fn is_installed(pkg: &str) -> bool {
    Command::new(PACMAN_BIN)
        .args(["-Q", pkg])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the repository a package was installed from (e.g. "cachyos-extra-v3", "aur").
/// Falls back to bare package name if unknown.
fn get_pkg_repo(pkg: &str) -> Option<String> {
    // Try pacman -Qi first (installed packages)
    let output = Command::new(PACMAN_BIN)
        .args(["-Qi", pkg])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // "From local DB: ..."  or  "Repository      : core"
            if line.starts_with("Repository") || line.starts_with("From") {
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    let repo = val.trim().to_string();
                    if !repo.is_empty() && repo != "Unknown Packager" {
                        return Some(repo);
                    }
                }
            }
        }
        // Check if it's an AUR package: no Repository field means local/AUR
        // pacman -Qi on AUR package won't have a Repository line
        let has_repo = stdout.lines().any(|l| l.starts_with("Repository"));
        if !has_repo {
            return Some("aur".to_string());
        }
    }
    None
}

/// Format package entry for world.set: "repo/name" if repo known, else just "name".
fn pkg_world_entry(pkg: &str) -> String {
    // If already has a slash (user passed repo/pkg), keep as-is
    if pkg.contains('/') {
        return pkg.to_string();
    }
    match get_pkg_repo(pkg) {
        Some(repo) => format!("{}/{}", repo, pkg),
        None => pkg.to_string(),
    }
}

// ── Emerge-style output ───────────────────────────────────────────────────────

fn print_emerge_plan(pkgs: &[PkgInfo]) {
    println!("\nThese are the packages that would be merged, in order:\n");
    println!("Calculating dependencies... done!");
    println!();
    for p in pkgs {
        let status = if p.is_new { "N" } else { "U" };
        println!("[ebuild  {:<4} ] {}-{}", status, p.name, p.version);
    }
    println!();
    println!("Total: {} package(s)", pkgs.len());
    println!();
}

fn print_emerge_emerging(pkgs: &[PkgInfo]) {
    for (i, p) in pkgs.iter().enumerate() {
        println!(
            ">>> Emerging ({} of {}) {}-{}",
            i + 1,
            pkgs.len(),
            p.name,
            p.version
        );
    }
    println!();
}

// ── Command helper ────────────────────────────────────────────────────────────

fn run_cmd(prog: &str, args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new(prog);
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }
    match cmd.status() {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!(">>> Execution error ({}): {}", prog, e);
            false
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    check_binaries();

    let cli = Cli::parse();

    // Detect @world / world in package list
    let has_world = cli.packages.iter()
        .any(|p| p == "@world");

    // Build validated package list (excluding world aliases)
    let target_pkgs: Vec<String> = validate_packages(
        &cli.packages
            .iter()
            .filter(|p| *p != "@world")
            .cloned()
            .collect::<Vec<_>>(),
    );

    // 1. Search (including --searchdesc)
    if cli.search || cli.searchdesc {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: Specify search term.");
            std::process::exit(1);
        }

        if cli.searchdesc {
            // Search descriptions: pacman -Ss for official, aura -As for AUR
            println!(">>> Searching descriptions for '{}'...", target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-Ss"], &target_pkgs);
            println!();
            println!(">>> Searching AUR descriptions for '{}'...", target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-As"], &target_pkgs);
            return;
        }

        if cli.verbose {
            if cli.aur {
                println!(">>> Searching in AUR for '{}'...", target_pkgs.join(" "));
                run_cmd(AURA_BIN, &["-Ai"], &target_pkgs);
            } else {
                let found = probe_official(&target_pkgs).is_some();
                if found {
                    println!(">>> Searching for '{}'...", target_pkgs.join(" "));
                    run_cmd(AURA_BIN, &["-Si"], &target_pkgs);
                } else {
                    println!(
                        ">>> '{}' not found in official repos, searching AUR...",
                        target_pkgs.join(" ")
                    );
                    run_cmd(AURA_BIN, &["-Ai"], &target_pkgs);
                }
            }
        } else if cli.aur {
            println!(">>> Searching in AUR for '{}'...", target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-As"], &target_pkgs);
        } else {
            println!(">>> Searching for '{}'...", target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-Ss"], &target_pkgs);
            println!();
            println!(">>> Searching in AUR for '{}'...", target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-As"], &target_pkgs);
        }
        return;
    }

    // 2. Sync — sync DB, then continue to install if packages given
    if cli.sync {
        println!(">>> Syncing package databases...");
        run_cmd(AURA_BIN, &["-Sy"], &[]);
        if target_pkgs.is_empty() && !has_world {
            return;
        }
        // fall through to update or install
    }

    // --regen: regenerate package metadata cache
    if cli.regen {
        println!(">>> Regenerating package metadata cache...");
        run_cmd(SUDO_BIN, &[PACMAN_BIN, "-Fy"], &[]);
        return;
    }

    // --prune: remove installed packages not in world.set
    if cli.prune {
        println!(">>> Pruning packages not in world.set...");
        if !is_safe_path(WORLD_SET_FILE) {
            eprintln!(">>> Warning: {} is a symlink — refusing to read", WORLD_SET_FILE);
            std::process::exit(1);
        }
        let world_bare: HashSet<String> = match fs::File::open(WORLD_SET_FILE) {
            Ok(file) => io::BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().split('/').last().unwrap_or("").to_string())
                .collect(),
            Err(_) => {
                eprintln!(">>> Error: cannot open world.set");
                std::process::exit(1);
            }
        };
        let output = Command::new(PACMAN_BIN)
            .args(["-Qeq"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(out) => {
                let installed: Vec<String> = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                let to_remove: Vec<String> = installed
                    .into_iter()
                    .filter(|p| !world_bare.contains(p))
                    .collect();
                if to_remove.is_empty() {
                    println!(">>> Nothing to prune. All explicitly installed packages are in world.set.");
                    return;
                }
                println!();
                for p in &to_remove {
                    println!("[unmerge     ] {}", p);
                }
                println!();
                println!("Total: {} package(s) to prune", to_remove.len());
                println!();
                if cli.pretend { return; }
                let mut args = vec![PACMAN_BIN, "-Rns"];
                if !cli.ask { args.push("--noconfirm"); }
                run_cmd(SUDO_BIN, &args, &to_remove);
            }
            Err(_) => eprintln!(">>> Error: failed to list installed packages"),
        }
        return;
    }

    // --resume: re-run last interrupted aura/pacman operation
    if cli.resume {
        println!(">>> Attempting to resume last interrupted transaction...");
        let mut args = vec!["-S", "--needed"];
        if cli.skipfirst {
            // Can't truly replicate portage skipfirst on pacman, so we warn
            println!(">>> Note: --skipfirst is not directly supported; resuming normally.");
        }
        if !cli.ask { args.push("--noconfirm"); }
        run_cmd(SUDO_BIN, &[PACMAN_BIN, "--noconfirm", "-Syu"], &[]);
        return;
    }

    // --select: explicitly add packages to world.set without installing
    if cli.select {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: specify packages to add to world.set.");
            std::process::exit(1);
        }
        for p in &target_pkgs {
            println!(">>> Selecting {} into world.set...", p);
        }
        add_to_world_set(&target_pkgs);
        return;
    }

    // --deselect: remove packages from world.set without unmerging
    if cli.deselect {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: specify packages to deselect from world.set.");
            std::process::exit(1);
        }
        for p in &target_pkgs {
            println!(">>> Deselecting {} from world.set (package stays installed)...", p);
        }
        remove_from_world_set(&target_pkgs);
        return;
    }

    // 3. Update @world — triggered by -u or bare @world/world
    if cli.update || (has_world && target_pkgs.is_empty()) {
        println!(">>> Calculating dependencies... done!");
        println!();
        println!(">>> Upgrading system (official repos)...");
        let mut s_args = vec!["-Syu"];
        if cli.verbose {
            s_args.push("--verbose");
        }
        run_cmd(AURA_BIN, &s_args, &[]);

        println!(">>> Upgrading AUR packages...");
        run_cmd(AURA_BIN, &["-Au"], &[]);

        println!();
        println!(">>> Auto-cleaning packages...");
        return;
    }

    // 4. Depclean (orphans)
    if cli.depclean {
        println!(">>> Calculating dependencies... done!");
        println!(">>> Checking for orphaned packages...");

        match Command::new(PACMAN_BIN).arg("-Qtdq").output() {
            Ok(out) => {
                let orphans_str = String::from_utf8_lossy(&out.stdout);
                let orphans: Vec<String> = orphans_str
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if orphans.is_empty() {
                    println!();
                    println!(">>> No orphaned packages were found on your system.");
                    return;
                }

                println!();
                for o in &orphans {
                    println!("[unmerge     ] {}", o);
                }
                println!();
                println!("Total: {} orphaned package(s) to remove", orphans.len());
                println!();

                let mut pacman_args = vec![PACMAN_BIN, "-Rns"];
                if cli.pretend {
                    pacman_args.push("--print");
                }
                if !cli.ask && !cli.pretend {
                    pacman_args.push("--noconfirm");
                }
                run_cmd(SUDO_BIN, &pacman_args, &orphans);
            }
            Err(_) => eprintln!(">>> Error: Failed to check for orphans."),
        }
        return;
    }

    // 5. Unmerge (remove)
    if cli.unmerge {
        if target_pkgs.is_empty() {
            eprintln!(">>> Error: Specify packages to remove.");
            std::process::exit(1);
        }

        println!("Calculating dependencies... done!");
        println!();
        for p in &target_pkgs {
            println!("[unmerge     ] {}", p);
        }
        println!();
        println!(">>> Unmerging {}...", target_pkgs.join(", "));

        let mut aura_args = vec!["-R"];
        if cli.pretend {
            aura_args.push("--print");
        }
        if !cli.ask && !cli.pretend {
            aura_args.push("--noconfirm");
        }
        if cli.verbose {
            aura_args.push("--verbose");
        }

        let success = run_cmd(AURA_BIN, &aura_args, &target_pkgs);
        if success && !cli.pretend {
            remove_from_world_set(&target_pkgs);
        }
        return;
    }

    // 6. Install
    if !target_pkgs.is_empty() {
        let mut base_args: Vec<&str> = Vec::new();
        if !cli.ask && !cli.pretend {
            base_args.push("--noconfirm");
        }
        if cli.oneshot {
            base_args.push("--asdeps");
        }
        if cli.noreplace {
            base_args.push("--needed");
        }

        let success: bool;

        if cli.aur {
            let pkg_infos = resolve_aur(&target_pkgs);
            print_emerge_plan(&pkg_infos);
            if cli.pretend { return; }
            print_emerge_emerging(&pkg_infos);

            let mut aur_args = vec!["-A"];
            aur_args.extend(&base_args);
            success = run_cmd(AURA_BIN, &aur_args, &target_pkgs);
        } else if let Some(pkg_infos) = probe_official(&target_pkgs) {
            print_emerge_plan(&pkg_infos);
            if cli.pretend { return; }
            print_emerge_emerging(&pkg_infos);

            let mut off_args = vec!["-S"];
            if cli.verbose { off_args.push("--verbose"); }
            off_args.extend(&base_args);
            success = run_cmd(AURA_BIN, &off_args, &target_pkgs);
        } else {
            println!(
                ">>> Not found in official repos. Searching AUR for '{}'...",
                target_pkgs.join(", ")
            );
            let pkg_infos = resolve_aur(&target_pkgs);
            print_emerge_plan(&pkg_infos);
            if cli.pretend { return; }
            print_emerge_emerging(&pkg_infos);

            let mut aur_args = vec!["-A"];
            aur_args.extend(&base_args);
            success = run_cmd(AURA_BIN, &aur_args, &target_pkgs);
        }

        if success && !cli.oneshot && !cli.pretend {
            println!();
            println!(">>> Auto-cleaning packages...");
            add_to_world_set(&target_pkgs);
        }
    }
}

// ── world.set ─────────────────────────────────────────────────────────────────

fn add_to_world_set(packages: &[String]) {
    println!(">>> Adding to world.set...");

    if !is_safe_path(WORLD_SET_FILE) {
        eprintln!(">>> Warning: {} is a symlink — refusing to read", WORLD_SET_FILE);
        return;
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
        let entry = pkg_world_entry(&bare);
        if current_set.get(&bare).map(|e| e != &entry).unwrap_or(true) {
            current_set.insert(bare, entry);
            changed = true;
        }
    }

    if !changed { return; }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted);
}

fn remove_from_world_set(packages: &[String]) {
    println!(">>> Removing from world.set...");

    if !is_safe_path(WORLD_SET_FILE) {
        eprintln!(">>> Warning: {} is a symlink — refusing to read", WORLD_SET_FILE);
        return;
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

    if !changed { return; }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted);
}

/// Atomic write: tee to .tmp then mv to final path.
fn write_world_set(packages: &[String]) {
    if !is_safe_path(WORLD_SET_TMP) {
        eprintln!(">>> Refusing to write: {} is a symlink", WORLD_SET_TMP);
        return;
    }
    if !is_safe_path(WORLD_SET_FILE) {
        eprintln!(">>> Refusing to write: {} is a symlink", WORLD_SET_FILE);
        return;
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

    if !write_ok { return; }

    match Command::new(SUDO_BIN)
        .args([MV_BIN, WORLD_SET_TMP, WORLD_SET_FILE])
        .status()
    {
        Ok(s) if s.success() => println!(">>> world.set updated."),
        Ok(_) => eprintln!(">>> Error: sudo mv failed when finalizing world.set."),
        Err(e) => eprintln!(">>> Error: sudo mv could not be spawned: {}", e),
    }
}