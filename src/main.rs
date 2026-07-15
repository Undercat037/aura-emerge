/*
Copyright (C) 2026 Undercat037
This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

aura-emerge: A gentoo-like wrapper for the aura AUR helper.
*/

use clap::Parser;
use clap_complete::Shell;
use colored::Colorize;
use std::collections::{HashMap, HashSet};
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
const MAKEPKG_BIN: &str = "/usr/bin/makepkg";
const ASP_BIN: &str = "/usr/bin/asp";

/// Base URL for Arch Linux packaging repos on GitLab
const ABS_GITLAB_BASE: &str = "https://gitlab.archlinux.org/archlinux/packaging/packages";

// ── Files ─────────────────────────────────────────────────────────────────────

const WORLD_SET_FILE:  &str = "/etc/emerge/world.set";
const ABS_BUILD_BASE:  &str = "/tmp/aura-emerge-abs";
const WORLD_SET_TMP:  &str = "/etc/emerge/world.set.tmp";
const PACMAN_CONF: &str = "/etc/pacman.conf";
const MAKEPKG_CONF_SYSTEM: &str = "/etc/makepkg.conf";
const BASH_BIN: &str = "/usr/bin/bash";
const UNAME_BIN: &str = "/usr/bin/uname";

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Portage-like wrapper for Arch Linux using Aura
#[derive(Parser, Debug)]
#[command(
    name = "emerge",
    bin_name = "emerge",
    version = concat!(env!("CARGO_PKG_VERSION"), " (aura-emerge)"),
    disable_help_flag = true,
    disable_version_flag = true,
)]
struct Cli {
    /// Show help information
    #[arg(short = 'h', long = "help", action = clap::ArgAction::SetTrue)]
    help: bool,

    /// Show version
    #[arg(short = 'V', long = "version", action = clap::ArgAction::SetTrue)]
    version: bool,

    /// Search for packages
    #[arg(short = 's', long)]
    search: bool,

    /// Sync package database
    #[arg(long)]
    sync: bool,

    /// Force-refresh all databases even if up to date (like pacman -Syy)
    #[arg(long)]
    refresh: bool,

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

    /// Only use official repos: never search or install from the AUR
    #[arg(long = "only-repos")]
    only_repos: bool,

    /// Build from ABS (Arch Build System) source
    #[arg(long = "abs")]
    abs: bool,

    /// Skip PGP signature checks when building from ABS (passes --skippgpcheck to makepkg)
    #[arg(long = "skippgp")]
    skippgp: bool,

    /// Open $EDITOR on PKGBUILD before building (use with --abs)
    #[arg(long = "edit")]
    edit: bool,

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

    /// Re-resolve repository prefixes for all packages in world.set
    #[arg(long = "regen-world")]
    regen_world: bool,

    /// Search package descriptions (--searchdesc)
    #[arg(long = "searchdesc")]
    searchdesc: bool,

    /// Explicitly add packages to world.set (--select)
    #[arg(long = "select")]
    select: bool,

    /// Remove packages from world.set without unmerging (--deselect)
    #[arg(long = "deselect")]
    deselect: bool,

    /// Show verbose slot/conflict info (informational)
    #[arg(long = "verbose-conflicts")]
    verbose_conflicts: bool,

    // ── Gentoo compat flags (accepted silently, no-op) ──────────────────────

    // Actions
    #[arg(long = "list-sets")]              list_sets: bool,
    #[arg(long = "metadata")]               metadata: bool,
    #[arg(long = "check-news")]             check_news: bool,
    #[arg(long = "clean")]                  clean: bool,
    #[arg(long = "config")]                 config: bool,

    // Output control
    #[arg(short = 'q', long = "quiet")]     quiet: bool,
    #[arg(long = "nospinner")]              nospinner: bool,
    #[arg(long = "noconfmem")]              noconfmem: bool,
    #[arg(long = "color")]                  color: Option<String>,
    #[arg(long = "columns")]               columns: bool,
    #[arg(long = "ignore-default-opts")]    ignore_default_opts: bool,

    // Dependency / graph control
    #[arg(short = 'O', long = "nodeps")]    nodeps: bool,
    #[arg(short = 'o', long = "onlydeps")]  onlydeps: bool,
    #[arg(short = 't', long = "tree")]      tree: bool,
    #[arg(long = "complete-graph")]         complete_graph: bool,
    #[arg(long = "changed-use")]            changed_use: bool,
    #[arg(long = "keep-going")]             keep_going: bool,
    #[arg(long = "backtrack")]              backtrack: Option<u32>,
    #[arg(long = "jobs")]                   jobs: Option<u32>,
    #[arg(long = "load-average")]           load_average: Option<f32>,
    #[arg(long = "exclude")]                exclude: Option<String>,

    // Binary pkg flags (emerge -k/-K/-g/-G/-b/-B)
    #[arg(short = 'k', long = "usepkg")]          usepkg: bool,
    #[arg(short = 'K', long = "usepkgonly")]       usepkgonly: bool,
    #[arg(short = 'g', long = "getbinpkg")]        getbinpkg: bool,
    #[arg(short = 'G', long = "getbinpkgonly")]    getbinpkgonly: bool,
    #[arg(short = 'b', long = "buildpkg")]         buildpkg: bool,
    #[arg(short = 'B', long = "buildpkgonly")]     buildpkgonly: bool,

    // Fetch flags
    #[arg(short = 'f', long = "fetchonly")]        fetchonly: bool,
    #[arg(short = 'F', long = "fetch-all-uri")]    fetch_all_uri: bool,

    // Misc compat
    #[arg(short = 'l', long = "changelog")]        changelog: bool,
    #[arg(long = "newrepo")]                        newrepo: bool,
    #[arg(long = "reinstall")]                      reinstall: Option<String>,
    #[arg(long = "quiet-build")]                    quiet_build: Option<String>,
    #[arg(long = "with-bdeps")]                     with_bdeps: Option<String>,
    #[arg(long = "alert", short = 'A')]             alert: bool,

    /// Generate a shell completion script and print it to stdout.
    /// Used by packaging (PKGBUILD) to install completions — not meant
    /// for interactive use, hence hidden from --help.
    #[arg(long = "gen-completions", hide = true, value_name = "SHELL")]
    gen_completions: Option<Shell>,

    /// Display info about the system, mirroring `emerge --info`
    #[arg(long = "info")]
    info: bool,

    /// Packages to install or '@world'
    packages: Vec<String>,
}

fn print_help() {
    println!("aura-emerge: command-line interface to the Portage system (Arch Linux)");
    println!("Usage:");
    println!("   emerge [ options ] [ action ] [ package | @set ] [ ... ]");
    println!("   emerge [ options ] [ action ] < @world >");
    println!("   emerge < --sync | --info >");
    println!("   emerge --resume [ --pretend | --ask | --skipfirst ]");
    println!("   emerge --help");
    println!("Options: -[1aCcDehNnpsuVv]");
    println!("          [ --abs                        ] [ --aur        ]");
    println!("          [ --only-repos                 ] [ --skippgp    ]");
    println!("          [ --deep                       ] [ --deep       ]");
    println!("          [ --emptytree                  ] [ --newuse     ]");
    println!("          [ --noreplace                  ] [ --oneshot    ]");
    println!("          [ --pretend                    ] [ --skipfirst  ]");
    println!("          [ --verbose-conflicts          ] [ --with-bdeps ]");
    println!("Actions:  [ --depclean  | --deselect | --prune      | --regen       ]");
    println!("          [ --resume    | --search   | --select     | --searchdesc  ]");
    println!("          [ --sync      | --unmerge  | --update     | --regen-world ]");
    println!("          [ --version   | --info                                    ]");
    println!();
    println!("   For more help consult the README: https://github.com/Undercat037/aura-emerge");
    println!();
    println!("Author: Undercat037");
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
    repo: String,
    /// "N" new, "U" upgrade, "D" downgrade, "R" reinstall
    status: String,
}

/// Compute install status: N=new, U=upgrade, D=downgrade, R=reinstall.
fn pkg_status(name: &str, new_ver: &str) -> String {
    let out = std::process::Command::new(PACMAN_BIN)
        .args(["-Q", name])
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let installed = match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.split_whitespace().nth(1).unwrap_or("").to_string()
        }
        _ => return "N".to_string(),
    };
    if installed.is_empty() { return "N".to_string(); }
    if installed == new_ver  { return "R".to_string(); }
    // Compare: if installed > new_ver it's a downgrade
    // Use pacman vercmp
    let cmp: i32 = std::process::Command::new("vercmp")
        .args([&installed, new_ver])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0))
        .unwrap_or(0);
    match cmp {
        c if c < 0 => "U".to_string(),
        0           => "R".to_string(),
        _           => "D".to_string(),
    }
}

/// Format package atom for display.
fn format_atom(p: &PkgInfo) -> String {
    if p.repo.is_empty() {
        format!("{}-{}", p.name, p.version)
    } else {
        format!("{}/{}-{}", p.repo, p.name, p.version)
    }
}

/// Colored status badge.
fn status_colored(status: &str) -> String {
    match status {
        "N" => status.green().bold().to_string(),
        "U" => status.yellow().bold().to_string(),
        "D" => status.red().bold().to_string(),
        _   => status.cyan().bold().to_string(),
    }
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
            let (repo, bare_name) = if name.contains('/') {
                let mut p = name.splitn(2, '/');
                (p.next().unwrap_or("").to_string(), p.next().unwrap_or(&name).to_string())
            } else {
                (String::new(), name.clone())
            };
            let status = pkg_status(&bare_name, &version);
            PkgInfo { name: bare_name, version, repo, status }
        })
        .collect();

    if infos.is_empty() { None } else { Some(infos) }
}

/// Probe official repos and split the requested packages into those that were
/// found (with full PkgInfo) and those that were not.
///
/// aura/pacman's `-Sp` fails (or simply omits a line) for the *whole* batch
/// query if even a single package name doesn't match anything in the sync
/// databases. Querying everything in one shot and treating a failure as
/// "nothing found" would then incorrectly push every package — including the
/// ones that DO exist officially — over to the AUR. To avoid that, we first
/// try the fast batch probe; if it doesn't account for every package we asked
/// about, we fall back to probing the missing ones individually so a single
/// bad name can't poison the rest.
fn probe_official_split(pkgs: &[String]) -> (Vec<PkgInfo>, Vec<String>) {
    let bare_of = |p: &str| p.split('/').last().unwrap_or(p).to_string();

    let mut found: Vec<PkgInfo> = Vec::new();
    let mut found_names: HashSet<String> = HashSet::new();

    if let Some(infos) = probe_official(pkgs) {
        for info in infos {
            found_names.insert(info.name.clone());
            found.push(info);
        }
    }

    // Anything the batch call didn't confirm gets re-checked one at a time.
    let unresolved: Vec<&String> = pkgs
        .iter()
        .filter(|p| !found_names.contains(&bare_of(p)))
        .collect();

    let mut missing: Vec<String> = Vec::new();
    for pkg in unresolved {
        match probe_official(std::slice::from_ref(pkg)) {
            Some(mut infos) if !infos.is_empty() => found.append(&mut infos),
            _ => missing.push(pkg.clone()),
        }
    }

    (found, missing)
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
            let status = pkg_status(&name, &version);
            result.push(PkgInfo { name, version, repo: "aur".to_string(), status });
        }
    }
    result
}


/// Get the repository a package was installed from (e.g. "cachyos-extra-v3", "aur").
///
/// pacman output is locale-dependent ("Repository" / "Сховище" / "Repository" etc.).
/// We force LC_ALL=C so field names are always English.
///
/// Strategy:
///   1. LC_ALL=C pacman -Qi <pkg> — local DB; knows the exact repo of the
///      installed package. No "Repository" line → AUR/foreign build.
///   2. LC_ALL=C pacman -Si <pkg> — sync DB fallback for not-yet-installed
///      packages (e.g. --select). Takes the first hit (highest-priority repo).
fn get_pkg_repo(pkg: &str) -> Option<String> {
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
fn pkg_world_entry(pkg: &str, forced_prefix: Option<&str>) -> String {
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

// ── Emerge-style output ───────────────────────────────────────────────────────

fn print_emerge_plan(pkgs: &[PkgInfo]) {
    println!();
    println!("{}", "These are the packages that would be merged, in order:".green().bold());
    println!();
    println!("Calculating dependencies... done!");
    println!();
    for p in pkgs {
        if p.status == "D" {
            println!("{} {}: downgrading package!", " *".yellow().bold(), p.name.bold());
        }
        println!("[{}  {:<4} ] {}",
            "ebuild".green(),
            status_colored(&p.status),
            format_atom(p).green().bold()
        );
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), pkgs.len());
    println!();
}

fn print_emerge_emerging(pkgs: &[PkgInfo]) {
    let total = pkgs.len();
    let pfx = ">>>".green().bold();
    println!("{} Verifying ebuild manifests", pfx);
    for (i, p) in pkgs.iter().enumerate() {
        println!("{} Emerging ({} of {}) {}",
            pfx,
            (i + 1).to_string().yellow().bold(),
            total.to_string().yellow().bold(),
            format_atom(p).green().bold()
        );
    }
    println!();
}

fn print_emerge_completed(pkgs: &[PkgInfo]) {
    let total = pkgs.len();
    let pfx = ">>>".green().bold();
    for (i, p) in pkgs.iter().enumerate() {
        let atom = format_atom(p).green().bold().to_string();
        let n = (i + 1).to_string().yellow().bold().to_string();
        let t = total.to_string().yellow().bold().to_string();
        println!("{} Installing ({} of {}) {}", pfx, n, t, atom);
        println!("{} Completed  ({} of {}) {}", pfx, n, t, atom);
    }
    let tg = total.to_string().green().bold().to_string();
    println!("{} Jobs: {} of {} complete", pfx, tg, tg);
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

// ── ABS (Arch Build System) support ──────────────────────────────────────────

/// Query .SRCINFO from ABS GitLab (raw) to get pkgver without cloning.
fn abs_get_version(pkg: &str) -> String {
    // Try to fetch .SRCINFO from GitLab raw API
    let url = format!("{}/{}/raw/HEAD/.SRCINFO", ABS_GITLAB_BASE, pkg);
    let output = Command::new("curl")
        .args(["-sf", "--max-time", "5", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("pkgver = ") {
                    return line["pkgver = ".len()..].trim().to_string();
                }
            }
        }
    }
    "?".to_string()
}

/// Build and install packages from ABS (Arch Build System) via asp.
/// Uses `asp checkout <pkg>` to fetch the official PKGBUILD, then runs makepkg -si.
fn abs_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, edit: bool) -> bool {
    for bin in &[ASP_BIN, MAKEPKG_BIN] {
        if !std::path::Path::new(bin).exists() {
            eprintln!(">>> Fatal: required binary not found: {}", bin);
            if *bin == ASP_BIN {
                eprintln!(">>> Hint: install asp with: emerge asp");
            }
            return false;
        }
    }

    let pkg_infos: Vec<PkgInfo> = pkgs.iter().filter_map(|pkg| {
        let bare = pkg.split('/').last().unwrap_or(pkg);
        if !validate_pkg(bare) || bare.contains('/') {
            eprintln!(">>> Error: invalid package name '{}' — skipping", bare);
            return None;
        }
        let version = abs_get_version(bare);

        let status = pkg_status(bare, &version);
        Some(PkgInfo { name: bare.to_string(), version, repo: "abs".to_string(), status })
    }).collect();

    if pkg_infos.is_empty() { return false; }

    println!();
    println!("{}", "These are the packages that would be merged, in order:".green().bold());
    println!();
    println!("Calculating dependencies... done!");
    println!();
    for p in &pkg_infos {
        if p.status == "D" {
            println!("{} {}: downgrading package!", " *".yellow().bold(), p.name.bold());
        }
        let atom = format!("{} (ABS)", format_atom(p));
        println!("[{}  {:<4} ] {}",
            "ebuild".green(),
            status_colored(&p.status),
            atom.green().bold()
        );
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), pkg_infos.len());
    println!();

    if pretend { return true; }

    // Clean up any stale build dir from interrupted previous run
    let build_base = std::path::PathBuf::from(ABS_BUILD_BASE);
    if build_base.exists() {
        let _ = std::fs::remove_dir_all(&build_base);
    }
    let mut all_ok = true;

    for (i, info) in pkg_infos.iter().enumerate() {
        let pfx = ">>>".green().bold();
        println!("{} Emerging ({} of {}) {} (ABS)",
            pfx,
            (i + 1).to_string().yellow().bold(),
            pkg_infos.len().to_string().yellow().bold(),
            format_atom(info).green().bold()
        );
        println!();

        let pkg_dir = build_base.join(&info.name);

        if !pkg_dir.starts_with(&build_base) {
            eprintln!(">>> Error: suspicious path for '{}' — skipping", info.name);
            all_ok = false;
            continue;
        }

        if pkg_dir.exists() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }
        let _ = std::fs::create_dir_all(&build_base);

        println!("{} Fetching {} from ABS via asp...", ">>>".green().bold(), info.name.green().bold());
        let checkout_ok = Command::new(ASP_BIN)
            .args(["checkout", &info.name])
            .current_dir(&build_base)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !checkout_ok {
            eprintln!("{} asp checkout failed for '{}'", ">>> Error:".red().bold(), info.name);
            eprintln!("{} package may not exist in ABS. Try without --abs or use --aur.", ">>> Note:".yellow().bold());
            all_ok = false;
            continue;
        }

        // asp creates <pkg>/trunk/ — that's where PKGBUILD lives
        let trunk_dir = pkg_dir.join("trunk");
        let build_dir = if trunk_dir.exists() { trunk_dir } else { pkg_dir.clone() };

        // Open PKGBUILD in $EDITOR before building if --edit is set
        if edit {
            let editor = std::env::var("EDITOR")
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| "nano".to_string());
            let pkgbuild = build_dir.join("PKGBUILD");
            println!("{} Opening {} in {}...",
                ">>>".green().bold(),
                "PKGBUILD".bold(),
                editor.green().bold()
            );
            println!("{} Save and close the editor to continue building.",
                ">>>".yellow().bold()
            );
            Command::new(&editor)
                .arg(&pkgbuild)
                .status()
                .ok();
        }

        let mut makepkg_args = vec!["-si"];
        if !ask    { makepkg_args.push("--noconfirm"); }
        if oneshot { makepkg_args.push("--asdeps"); }
        if skippgp { makepkg_args.push("--skippgpcheck"); }

        let build_ok = Command::new(MAKEPKG_BIN)
            .args(&makepkg_args)
            .current_dir(&build_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !build_ok {
            eprintln!("{} makepkg failed for '{}'", ">>> Error:".red().bold(), info.name);
            if !skippgp {
                eprintln!("{} if the build failed due to a missing PGP key, run:", ">>> Hint:".yellow().bold());
                eprintln!(">>>   gpg --recv-keys <key-id>");
                eprintln!(">>> Or retry with --skippgp to bypass signature checks.");
            }
            all_ok = false;
        }

        let _ = std::fs::remove_dir_all(&pkg_dir);
    }

    all_ok
}

// ── portageq shim ─────────────────────────────────────────────────────────────

/// Called when the binary is invoked as "portageq" (via symlink).
/// Answers fish shell completion queries so emerge.fish doesn't crash.
fn portageq_shim(args: &[String]) {
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "envvar" => {
            match args.get(2).map(String::as_str).unwrap_or("") {
                "EROOT" | "ROOT" => println!("/"),
                "PORTDIR"        => println!("/var/db/pkg"),
                "DISTDIR"        => println!("/var/cache/distfiles"),
                _                => {}
            }
        }
        "get_repos" => {
            // One dummy repo is enough; fish just needs a non-empty list
            println!("arch");
        }
        "get_repo_path" => {
            // args: eroot repo — return any existing dir
            println!("/var/db/pkg");
        }
        "get_repo_news_path" => {
            println!("/var/db/pkg");
        }
        "match_pkgs" | "best_version" => {
            let pkg = args.get(3).or_else(|| args.get(2)).map(String::as_str).unwrap_or("");
            if let Ok(out) = Command::new(PACMAN_BIN)
                .args(["-Q", pkg])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
            {
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    let mut p = line.splitn(2, ' ');
                    if let (Some(n), Some(v)) = (p.next(), p.next()) {
                        println!("{}-{}", n, v);
                    }
                }
            }
        }
        "list_repo_pkgs" => {
            if let Ok(out) = Command::new(PACMAN_BIN)
                .args(["-Ssq"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
            {
                print!("{}", String::from_utf8_lossy(&out.stdout));
            }
        }
        // Unknown command — exit silently so fish doesn't crash
        _ => {}
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

// ── --info ──────────────────────────────────────────────────────────────────

/// Run a command and return trimmed stdout, or None on any failure.
/// Pull the first bare `X.Y[.Z]` version token out of a string — used to
/// dig a clean version number out of `aura --version`'s output, which
/// mixes ASCII-art banner rows in with the text and isn't safe to grab a
/// whole line from.
fn extract_version_token(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .map(|tok| tok.trim_start_matches('v'))
        .find(|tok| {
            !tok.is_empty()
                && tok.chars().next().unwrap().is_ascii_digit()
                && tok.contains('.')
                && tok.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .map(str::to_string)
}

fn cmd_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Merge /etc/makepkg.conf with the user-level override exactly the way
/// makepkg itself resolves precedence: the system file is sourced first,
/// then (if present) $XDG_CONFIG_HOME/pacman/makepkg.conf — falling back to
/// ~/.config/pacman/makepkg.conf — then the legacy ~/.makepkg.conf, each
/// later file able to override values set before it. We let bash do the
/// actual sourcing (arrays, `$(nproc)`, quoting rules) instead of
/// hand-parsing shell syntax ourselves, then just read the results back.
fn read_makepkg_vars() -> HashMap<String, String> {
    let watched = [
        "CARCH", "CHOST", "CFLAGS", "CXXFLAGS", "LDFLAGS", "RUSTFLAGS",
        "MAKEFLAGS", "OPTIONS", "BUILDENV", "PKGEXT",
    ];
    let mut dump = String::new();
    for v in &watched {
        dump.push_str(&format!("echo \"{v}=${{{v}[*]}}\"\n"));
    }

    let script = format!(
        r#"
source {sys} 2>/dev/null
if [ -n "$XDG_CONFIG_HOME" ] && [ -f "$XDG_CONFIG_HOME/pacman/makepkg.conf" ]; then
    source "$XDG_CONFIG_HOME/pacman/makepkg.conf" 2>/dev/null
elif [ -f "$HOME/.config/pacman/makepkg.conf" ]; then
    source "$HOME/.config/pacman/makepkg.conf" 2>/dev/null
fi
[ -f "$HOME/.makepkg.conf" ] && source "$HOME/.makepkg.conf" 2>/dev/null
{dump}"#,
        sys = MAKEPKG_CONF_SYSTEM,
        dump = dump,
    );

    let mut map = HashMap::new();
    if let Ok(output) = Command::new(BASH_BIN)
        .arg("-c")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((k, v)) = line.split_once('=') {
                    map.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    map
}

/// The path to whichever user-level makepkg.conf override would actually
/// take effect, mirroring makepkg's own lookup order.
fn user_makepkg_conf_path() -> String {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return format!("{}/pacman/makepkg.conf", xdg);
    }
    if let Ok(home) = std::env::var("HOME") {
        return format!("{}/.config/pacman/makepkg.conf", home);
    }
    String::new()
}

/// Parse /etc/pacman.conf for repository sections in file order — which is
/// also pacman's own resolution priority: on a name clash between repos,
/// the one listed first wins. Each repo's Server/Include lines are kept;
/// Include lines are resolved one level deep (into the mirrorlist) so the
/// actual mirror is visible instead of just the pointer file.
fn parse_pacman_repos() -> Vec<(String, Vec<String>)> {
    let mut repos: Vec<(String, Vec<String>)> = Vec::new();
    let Ok(file) = fs::File::open(PACMAN_CONF) else { return repos; };
    let reader = io::BufReader::new(file);

    let mut current: Option<(String, Vec<String>)> = None;
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(repo) = current.take() {
                repos.push(repo);
            }
            let name = trimmed.trim_start_matches('[').trim_end_matches(']').to_string();
            if name != "options" {
                current = Some((name, Vec::new()));
            }
            continue;
        }
        let Some((_, lines)) = current.as_mut() else { continue };
        if let Some((key, val)) = trimmed.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            if key == "Include" {
                let resolved = fs::read_to_string(val).ok().and_then(|inc| {
                    inc.lines()
                        .map(str::trim)
                        .find(|l| l.starts_with("Server") && !l.starts_with('#'))
                        .map(str::to_string)
                });
                match resolved {
                    Some(server) => lines.push(format!("Include = {}  ({})", val, server)),
                    None => lines.push(format!("Include = {}", val)),
                }
            } else if key == "Server" {
                lines.push(format!("Server = {}", val));
            }
        }
    }
    if let Some(repo) = current.take() {
        repos.push(repo);
    }
    repos
}

fn read_meminfo() -> Option<(u64, u64, u64, u64)> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut mem_total = 0u64;
    let mut mem_free = 0u64;
    let mut swap_total = 0u64;
    let mut swap_free = 0u64;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let val: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => mem_total = val,
            "MemFree:" => mem_free = val,
            "SwapTotal:" => swap_total = val,
            "SwapFree:" => swap_free = val,
            _ => {}
        }
    }
    Some((mem_total, mem_free, swap_total, swap_free))
}

fn world_set_stats() -> Option<(usize, u64)> {
    let meta = fs::metadata(WORLD_SET_FILE).ok()?;
    let content = fs::read_to_string(WORLD_SET_FILE).ok()?;
    let count = content.lines().filter(|l| !l.trim().is_empty()).count();
    Some((count, meta.len()))
}

/// `emerge --info`: a system/build-environment summary in the spirit of
/// Gentoo's `emerge --info`, adapted to aura/pacman. Not a literal
/// impersonation of Portage's output — field names are relabeled where the
/// underlying concept differs — but the overall shape (uname/mem block,
/// Repositories block, flag dump, world set) is intentionally familiar.
fn print_system_info() {
    let aura_ver = cmd_stdout(AURA_BIN, &["--version"])
        .and_then(|s| extract_version_token(&s))
        .map(|v| format!("aura {}", v))
        .unwrap_or_else(|| "aura unknown".to_string());
    let pacman_ver = cmd_stdout(PACMAN_BIN, &["--version"])
        .and_then(|s| {
            s.lines().find_map(|l| {
                let lower = l.to_lowercase();
                lower.find("pacman").map(|idx| l[idx..].trim().to_string())
            })
        })
        .unwrap_or_else(|| "unknown".to_string());
    let kernel = cmd_stdout(UNAME_BIN, &["-r"]).unwrap_or_else(|| "unknown".to_string());
    let arch = cmd_stdout(UNAME_BIN, &["-m"]).unwrap_or_else(|| "unknown".to_string());
    let uname_full = cmd_stdout(UNAME_BIN, &["-srvm"]).unwrap_or_else(|| "unknown".to_string());
    let cpu_model = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!(
        "{} {} ({}, {}, linux {}, {})",
        "aura-emerge".bold(),
        env!("CARGO_PKG_VERSION"),
        aura_ver,
        pacman_ver,
        kernel,
        arch
    );
    println!("{}", "=".repeat(70));
    println!("System uname: {}", uname_full);
    println!("CPU: {}", cpu_model);
    if let Some((mem_total, mem_free, swap_total, swap_free)) = read_meminfo() {
        println!("KiB Mem:    {} total, {} free", mem_total, mem_free);
        println!("KiB Swap:   {} total, {} free", swap_total, swap_free);
    }
    println!();

    println!("Repositories:");
    println!();
    let repos = parse_pacman_repos();
    if repos.is_empty() {
        println!("    (could not read {})", PACMAN_CONF);
    }
    for (i, (name, lines)) in repos.iter().enumerate() {
        println!("{}", name);
        println!("    priority: {} (order of appearance in {})", i + 1, PACMAN_CONF);
        for l in lines {
            println!("    {}", l);
        }
        println!();
    }

    let user_conf = user_makepkg_conf_path();
    let user_conf_active = !user_conf.is_empty() && std::path::Path::new(&user_conf).exists();
    if user_conf_active {
        println!("makepkg.conf: {} -> overridden by {}", MAKEPKG_CONF_SYSTEM, user_conf);
    } else {
        println!("makepkg.conf: {} (no user override present)", MAKEPKG_CONF_SYSTEM);
    }
    println!();

    let vars = read_makepkg_vars();
    let get = |k: &str| vars.get(k).cloned().unwrap_or_default();
    for key in ["CARCH", "CHOST", "CFLAGS", "CXXFLAGS", "LDFLAGS", "RUSTFLAGS",
                "MAKEFLAGS", "OPTIONS", "BUILDENV", "PKGEXT"] {
        println!("{}=\"{}\"", key, get(key));
    }
    println!();

    match world_set_stats() {
        Some((count, size)) => println!(
            "world.set: {} package(s), {} bytes ({})",
            count, size, WORLD_SET_FILE
        ),
        None => println!("world.set: not found ({})", WORLD_SET_FILE),
    }
}

fn main() {

    // If invoked as "portageq" (symlink), act as the shim
    let argv: Vec<String> = std::env::args().collect();
    let invoked_as = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if invoked_as == "portageq" {
        portageq_shim(&argv);
        return;
    }

    let cli = Cli::parse();

    // Shell completion generation: no aura/pacman needed, no world.set
    // touched — this just prints a script to stdout. Handled before
    // check_binaries() so it also works in a clean chroot/build environment.
    if let Some(shell) = cli.gen_completions {
        let mut cmd = <Cli as clap::CommandFactory>::command();
        clap_complete::generate(shell, &mut cmd, "emerge", &mut io::stdout());
        return;
    }

    if cli.help {
        print_help();
        return;
    }

    if cli.version {
        println!("aura-emerge {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if cli.info {
        print_system_info();
        return;
    }

    check_binaries();

    if cli.aur && cli.only_repos {
        eprintln!(">>> Error: --aur and --only-repos are mutually exclusive.");
        std::process::exit(1);
    }

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
            println!("{} Searching descriptions for '{}'...", ">>>".green().bold(), target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-Ss"], &target_pkgs);
            if !cli.only_repos {
                println!();
                println!("{} Searching {} descriptions for '{}'...", ">>>".green().bold(), "AUR".cyan().bold(), target_pkgs.join(" "));
                run_cmd(AURA_BIN, &["-As"], &target_pkgs);
            }
            return;
        }

        if cli.verbose {
            if cli.aur {
                println!("{} Searching in {} for '{}'...", ">>>".green().bold(), "AUR".cyan().bold(), target_pkgs.join(" "));
                run_cmd(AURA_BIN, &["-Ai"], &target_pkgs);
            } else {
                let found = probe_official(&target_pkgs).is_some();
                if found {
                    println!("{} Searching for '{}'...", ">>>".green().bold(), target_pkgs.join(" "));
                    run_cmd(AURA_BIN, &["-Si"], &target_pkgs);
                } else if cli.only_repos {
                    println!(
                        ">>> '{}' not found in official repos. (--only-repos set, not searching AUR)",
                        target_pkgs.join(" ")
                    );
                } else {
                    println!(
                        ">>> '{}' not found in official repos, searching AUR...",
                        target_pkgs.join(" ")
                    );
                    run_cmd(AURA_BIN, &["-Ai"], &target_pkgs);
                }
            }
        } else if cli.aur {
            println!("{} Searching in {} for '{}'...", ">>>".green().bold(), "AUR".cyan().bold(), target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-As"], &target_pkgs);
        } else {
            println!("{} Searching for '{}'...", ">>>".green().bold(), target_pkgs.join(" "));
            run_cmd(AURA_BIN, &["-Ss"], &target_pkgs);
            if !cli.only_repos {
                println!();
                println!("{} Searching in {} for '{}'...", ">>>".green().bold(), "AUR".cyan().bold(), target_pkgs.join(" "));
                run_cmd(AURA_BIN, &["-As"], &target_pkgs);
            }
        }
        return;
    }

    // 2. Sync — sync DB, then continue to install if packages given
    if cli.sync {
        let sync_flag = if cli.refresh { "-Syy" } else { "-Sy" };
        println!("{} Syncing package databases{}...",
            ">>>".green().bold(),
            if cli.refresh { " (force refresh)" } else { "" }
        );
        run_cmd(AURA_BIN, &[sync_flag], &[]);
        if target_pkgs.is_empty() && !has_world {
            return;
        }
        // fall through to update or install
    }

    // --regen: regenerate package metadata cache
    if cli.regen {
        println!("{} Regenerating package metadata cache...", ">>>".green().bold());
        run_cmd(SUDO_BIN, &[PACMAN_BIN, "-Fy"], &[]);
        return;
    }

    // --regen-world: re-resolve repo prefixes for all entries in world.set
    if cli.regen_world {
        regen_world_set();
        return;
    }

    // --prune: remove installed packages not in world.set
    if cli.prune {
        println!("{} Pruning packages not in world.set...", ">>>".green().bold());
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
                    println!("[{}] {}", "unmerge".red().bold(), p);
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
        add_to_world_set(&target_pkgs, None);
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

    // 3a. Mixed: specific pkgs + @world (e.g. emerge nano @world)
    //     Install the named packages first, then fall through to world update
    if has_world && !target_pkgs.is_empty() && !cli.update {
        println!("{} Installing specified packages before @world update...", ">>>".green().bold());
        // will fall through to install block below, then world update happens separately
    }

    // 3. Update @world — triggered by -u or bare @world/world
    if cli.update || has_world {
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
        println!("{} Auto-cleaning packages...", ">>>".green().bold());
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
                    println!("[{}] {}", "unmerge".red().bold(), o);
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

        println!("{} This action can remove important packages! In order to be safer, use", " *".yellow().bold());
        println!("{} `emerge -p --depclean <atom>` to check for reverse dependencies before", " *".yellow().bold());
        println!("{} removing packages.", " *".yellow().bold());
        println!();
        println!("{} These are the packages that would be unmerged:", ">>>".green().bold());
        println!();
        for p in &target_pkgs {
            let bare = p.split('/').last().unwrap_or(p);
            let ver = {
                let out = std::process::Command::new(PACMAN_BIN)
                    .args(["-Q", bare]).env("LC_ALL", "C")
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .output();
                match out {
                    Ok(o) if o.status.success() =>
                        String::from_utf8_lossy(&o.stdout)
                            .split_whitespace().nth(1)
                            .unwrap_or("?").to_string(),
                    _ => "?".to_string(),
                }
            };
            let repo = get_pkg_repo(bare).unwrap_or_default();
            let atom = if repo.is_empty() { bare.to_string() } else { format!("{}/{}", repo, bare) };
            println!(" {}", atom);
            println!("    selected: {}", ver);
            println!("   protected: none");
            println!("     omitted: none");
        }
        println!();
        println!("{} {} packages are slated for removal.", ">>>".green().bold(), "'Selected'".yellow().bold());
        println!("{} {} and {} packages will not be removed.", ">>>".green().bold(), "'Protected'".green(), "'omitted'".cyan());
        println!();
        println!("{} Unmerging {}...", ">>>".green().bold(), target_pkgs.join(", ").bold());

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

        let mut success: bool;
        let mut installed_infos: Vec<PkgInfo> = Vec::new();

        if cli.abs {
            success = abs_install(&target_pkgs, cli.pretend, cli.ask, cli.oneshot, cli.skippgp, cli.edit);
        } else if cli.aur {
            let pkg_infos = resolve_aur(&target_pkgs);
            print_emerge_plan(&pkg_infos);
            if cli.pretend { return; }
            print_emerge_emerging(&pkg_infos);
            let mut aur_args = vec!["-A"];
            aur_args.extend(&base_args);
            success = run_cmd(AURA_BIN, &aur_args, &target_pkgs);
            if success { installed_infos = pkg_infos; }
        } else {
            // Probe official repos in a way that's safe against partial matches:
            // a single unmatched name no longer drags every other package into
            // an AUR search.
            let (official_infos, missing) = probe_official_split(&target_pkgs);

            if missing.is_empty() {
                // Everything found in official repos.
                print_emerge_plan(&official_infos);
                if cli.pretend { return; }
                print_emerge_emerging(&official_infos);
                let mut off_args = vec!["-S"];
                if cli.verbose { off_args.push("--verbose"); }
                off_args.extend(&base_args);
                success = run_cmd(AURA_BIN, &off_args, &target_pkgs);
                if success { installed_infos = official_infos; }
            } else if cli.only_repos {
                // --only-repos: never touch the AUR. Unresolved names are
                // skipped (with a warning) rather than aborting the whole
                // request — packages that *were* found officially still get
                // installed, same as e.g. `pacman -S` reporting "target not
                // found" for one name without refusing the rest.
                eprintln!(
                    ">>> Warning: --only-repos is set; the following package(s) were not \
                    found in official repos and will be skipped (AUR was not searched):"
                );
                for m in &missing {
                    eprintln!("    {}", m);
                }

                if official_infos.is_empty() {
                    eprintln!(">>> Error: no requested packages were found in official repos.");
                    std::process::exit(1);
                }

                print_emerge_plan(&official_infos);
                if cli.pretend {
                    // Not everything could be resolved — still exit non-zero
                    // so scripts can detect the partial result.
                    std::process::exit(1);
                }
                print_emerge_emerging(&official_infos);

                let official_names: Vec<String> =
                    official_infos.iter().map(|p| p.name.clone()).collect();
                let mut off_args = vec!["-S"];
                if cli.verbose { off_args.push("--verbose"); }
                off_args.extend(&base_args);
                let off_success = run_cmd(AURA_BIN, &off_args, &official_names);
                if off_success {
                    installed_infos = official_infos;
                }
                // Even on a clean install of the found packages, some
                // requested names were never resolved — surface that as an
                // overall failure/non-zero exit. The tail below still
                // credits world.set / prints "Completed" for whatever did
                // succeed, based on installed_infos.
                success = false;
            } else if official_infos.is_empty() {
                // Nothing found officially at all — search AUR for everything.
                println!(
                    ">>> Not found in official repos. Searching AUR for '{}'...",
                    missing.join(", ")
                );
                let pkg_infos = resolve_aur(&missing);
                print_emerge_plan(&pkg_infos);
                if cli.pretend { return; }
                print_emerge_emerging(&pkg_infos);
                let mut aur_args = vec!["-A"];
                aur_args.extend(&base_args);
                success = run_cmd(AURA_BIN, &aur_args, &missing);
                if success { installed_infos = pkg_infos; }
            } else {
                // Mixed case: some packages are official, some need the AUR.
                println!(
                    ">>> Not found in official repos: '{}'. Searching AUR...",
                    missing.join(", ")
                );
                let aur_infos = resolve_aur(&missing);
                let mut all_infos = official_infos.clone();
                all_infos.extend(aur_infos.clone());
                print_emerge_plan(&all_infos);
                if cli.pretend { return; }
                print_emerge_emerging(&all_infos);

                let official_names: Vec<String> =
                    official_infos.iter().map(|p| p.name.clone()).collect();
                let mut off_args = vec!["-S"];
                if cli.verbose { off_args.push("--verbose"); }
                off_args.extend(&base_args);
                success = run_cmd(AURA_BIN, &off_args, &official_names);
                if success {
                    installed_infos.extend(official_infos);
                }

                let mut aur_args = vec!["-A"];
                aur_args.extend(&base_args);
                let aur_success = run_cmd(AURA_BIN, &aur_args, &missing);
                if aur_success {
                    installed_infos.extend(aur_infos);
                } else {
                    success = false;
                }
            }
        }

        if !cli.pretend {
            if !installed_infos.is_empty() {
                print_emerge_completed(&installed_infos);
            }

            if !cli.oneshot {
                if cli.abs {
                    // abs_install() doesn't populate installed_infos (single
                    // build source, no partial-success case to track).
                    if success {
                        println!("{} Auto-cleaning packages...", ">>>".green().bold());
                        add_to_world_set(&target_pkgs, Some("abs"));
                    }
                } else if !installed_infos.is_empty() {
                    println!("{} Auto-cleaning packages...", ">>>".green().bold());
                    // Record only what actually got installed, with the
                    // correct repo prefix per package. This matters for a
                    // mixed official+AUR install where one half can succeed
                    // while the other fails — we must not lose track of the
                    // half that worked, and must not force an "aur" prefix
                    // onto an official package or vice versa.
                    let official_names: Vec<String> = installed_infos.iter()
                        .filter(|p| p.repo != "aur")
                        .map(|p| p.name.clone())
                        .collect();
                    let aur_names: Vec<String> = installed_infos.iter()
                        .filter(|p| p.repo == "aur")
                        .map(|p| p.name.clone())
                        .collect();
                    if !official_names.is_empty() {
                        add_to_world_set(&official_names, None);
                    }
                    if !aur_names.is_empty() {
                        add_to_world_set(&aur_names, Some("aur"));
                    }
                }
            }

            if !success {
                eprintln!(">>> Warning: not all requested packages were installed successfully.");
                std::process::exit(1);
            }
        }
    }
}

// ── world.set ─────────────────────────────────────────────────────────────────

/// Re-resolve repository prefixes for every package in world.set.
/// Reads current entries, calls get_pkg_repo() for each, rewrites the file.
/// Useful after locale fixes, repo migrations, or manual edits.
fn regen_world_set() {
    println!("{} Regenerating world.set repository prefixes...", ">>>".green().bold());

    if !is_safe_path(WORLD_SET_FILE) {
        eprintln!(">>> Warning: {} is a symlink — refusing to modify", WORLD_SET_FILE);
        std::process::exit(1);
    }

    let entries: Vec<String> = match fs::File::open(WORLD_SET_FILE) {
        Ok(file) => io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => {
            eprintln!(">>> Error: cannot open {}", WORLD_SET_FILE);
            std::process::exit(1);
        }
    };

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
        return;
    }

    updated.sort();
    write_world_set(&updated);
    println!("{} world.set updated ({} entries changed).", ">>>".green().bold(), changed);
}

fn add_to_world_set(packages: &[String], forced_prefix: Option<&str>) {
    println!("{} Adding to world.set...", ">>>".green().bold());

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
        let entry = pkg_world_entry(&bare, forced_prefix);
        // Always overwrite: re-resolves repo on every install so stale entries
        // (e.g. "aur/nano" left from a locale-parsing bug) get corrected.
        let stale = current_set.get(&bare).map(|e| e != &entry).unwrap_or(true);
        if stale {
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