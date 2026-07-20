/*
Copyright (C) 2026 Undercat037
This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

aura-emerge: A gentoo-like wrapper for the aura AUR helper.
*/

mod world_set;
mod packages;
mod security;

use clap::Parser;
use clap_complete::Shell;
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead};
use std::process::{Command, Stdio};

use world_set::*;
use packages::*;
use security::*;

// ── Binary paths ───────────────────────────────────────────────────────

pub(crate) const AURA_BIN:   &str = "/usr/bin/aura";
pub(crate) const PACMAN_BIN: &str = "/usr/bin/pacman";
pub(crate) const SUDO_BIN:   &str = "/usr/bin/sudo";
pub(crate) const TEE_BIN:    &str = "/usr/bin/tee";
pub(crate) const MV_BIN:     &str = "/usr/bin/mv";
pub(crate) const RM_BIN:     &str = "/usr/bin/rm";
pub(crate) const MAKEPKG_BIN: &str = "/usr/bin/makepkg";
pub(crate) const ASP_BIN: &str = "/usr/bin/asp";
pub(crate) const GPG_BIN: &str = "/usr/bin/gpg";
/// Keyserver used for automatic/suggested PGP key imports (--abs). Same
/// SKS-successor pool most distro tooling defaults to.
pub(crate) const PGP_KEYSERVER: &str = "keyserver.ubuntu.com";

/// Base URL for Arch Linux packaging repos on GitLab
pub(crate) const ABS_GITLAB_BASE: &str = "https://gitlab.archlinux.org/archlinux/packaging/packages";

// ── Files ─────────────────────────────────────────────────────────────────────

pub(crate) const WORLD_SET_FILE:  &str = "/etc/emerge/world.set";
pub(crate) const ABS_BUILD_BASE:  &str = "/tmp/aura-emerge-abs";
pub(crate) const WORLD_SET_TMP:  &str = "/etc/emerge/world.set.tmp";
pub(crate) const PACMAN_CONF: &str = "/etc/pacman.conf";
pub(crate) const MAKEPKG_CONF_SYSTEM: &str = "/etc/makepkg.conf";
pub(crate) const BASH_BIN: &str = "/usr/bin/bash";
pub(crate) const UNAME_BIN: &str = "/usr/bin/uname";

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

/// Read one line from stdin, byte by byte, via the raw fd — bypassing
/// Rust's `Stdin`, which over-reads into its own buffer. This tool follows
/// a "y/N" prompt by spawning a child (aura/pacman) that inherits stdin
/// and expects its own interactive read right after; any bytes the parent
/// over-read are lost to the child. Reading one byte at a time avoids that.
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

fn main() {
    if let Err(e) = run() {
        eprintln!("{} {:#}", ">>> Error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    reset_sigpipe();

    // If invoked as "portageq" (symlink), act as the shim
    let argv: Vec<String> = std::env::args().collect();
    let invoked_as = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if invoked_as == "portageq" {
        portageq_shim(&argv);
        return Ok(());
    }

    let cli = Cli::parse();

    // Shell completion generation: no aura/pacman needed, no world.set
    // touched — this just prints a script to stdout. Handled before
    // check_binaries() so it also works in a clean chroot/build environment.
    if let Some(shell) = cli.gen_completions {
        let mut cmd = <Cli as clap::CommandFactory>::command();
        clap_complete::generate(shell, &mut cmd, "emerge", &mut io::stdout());
        return Ok(());
    }

    if cli.help {
        print_help();
        return Ok(());
    }

    if cli.version {
        println!("aura-emerge {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.info {
        print_system_info();
        return Ok(());
    }

    check_binaries();

    if cli.aur && cli.only_repos {
        eprintln!(">>> Error: --aur and --only-repos are mutually exclusive.");
        std::process::exit(1);
    }

    // --abs only means anything for an install (it picks the build source);
    // search has no ABS-backed lookup, so silently accepting it here would
    // make it look like the search got filtered when it didn't.
    if cli.abs && (cli.search || cli.searchdesc) {
        eprintln!(">>> Error: --abs is not valid with -s/--searchdesc (it only applies to installs).");
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
        return Ok(());
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
            return Ok(());
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
        return Ok(());
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
            return Ok(());
        }
        // fall through to update or install
    }

    // --regen: regenerate package metadata cache
    if cli.regen {
        println!("{} Regenerating package metadata cache...", ">>>".green().bold());
        run_cmd(SUDO_BIN, &[PACMAN_BIN, "-Fy"], &[]);
        return Ok(());
    }

    // --regen-world: re-resolve repo prefixes for all entries in world.set
    if cli.regen_world {
        regen_world_set()?;
        return Ok(());
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
                    return Ok(());
                }
                println!();
                for p in &to_remove {
                    println!("[{}] {}", "unmerge".red().bold(), p);
                }
                println!();
                println!("Total: {} package(s) to prune", to_remove.len());
                println!();
                if cli.pretend { return Ok(()); }
                let mut args = vec![PACMAN_BIN, "-Rns"];
                if !cli.ask { args.push("--noconfirm"); }
                run_cmd(SUDO_BIN, &args, &to_remove);
            }
            Err(_) => eprintln!(">>> Error: failed to list installed packages"),
        }
        return Ok(());
    }

    // --resume: re-run last interrupted aura/pacman operation
    if cli.resume {
        println!(">>> Attempting to resume last interrupted transaction...");
        if cli.skipfirst {
            // Can't truly replicate portage skipfirst on pacman, so we warn
            println!(">>> Note: --skipfirst is not directly supported; resuming normally.");
        }
        let mut args = vec![PACMAN_BIN, "-Syu"];
        if !cli.ask {
            args.push("--noconfirm");
        }
        let success = run_cmd(SUDO_BIN, &args, &[]);
        if !success {
            std::process::exit(1);
        }
        return Ok(());
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
        add_to_world_set(&target_pkgs, None)?;
        // world.set now says these are explicitly wanted — mirror that onto
        // pacman's own bookkeeping (no-ops for anything not yet installed).
        mark_asexplicit(&target_pkgs);
        return Ok(());
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
        remove_from_world_set(&target_pkgs)?;
        // Opposite of --select: no longer wanted by world.set, so demote to
        // a dependency in pacman's bookkeeping — makes it eligible for
        // --depclean like any other transitive dependency.
        mark_asdeps(&target_pkgs);
        return Ok(());
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
        return Ok(());
    }

    // 4. Depclean — true leaf orphans (pacman -Qtdq) PLUS anything
    //    explicitly installed but no longer tracked in world.set (drifted
    //    in via a raw `pacman -S`, or left behind by --deselect). This
    //    mirrors real Portage's --depclean, which removes anything that's
    //    neither in @world nor required by something that is, rather than
    //    only pacman's own "unreferenced leaf" notion of an orphan.
    if cli.depclean {
        println!(">>> Calculating dependencies... done!");
        println!(">>> Checking for orphaned and untracked packages...");

        let true_orphans: Vec<String> = match Command::new(PACMAN_BIN).arg("-Qtdq").output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Err(_) => {
                eprintln!(">>> Error: Failed to check for orphans.");
                Vec::new()
            }
        };

        // Explicitly-installed packages that world.set no longer lists.
        let mut untracked: Vec<String> = Vec::new();
        if !is_safe_path(WORLD_SET_FILE) {
            eprintln!(">>> Warning: {} is a symlink — skipping world.set cross-check", WORLD_SET_FILE);
        } else if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
            let world_bare: HashSet<String> = io::BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().split('/').last().unwrap_or("").to_string())
                .collect();

            match Command::new(PACMAN_BIN).arg("-Qeq").output() {
                Ok(out) => {
                    untracked = String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty() && !world_bare.contains(l))
                        .collect();
                }
                Err(_) => eprintln!(">>> Warning: failed to list explicitly installed packages"),
            }
        } else {
            eprintln!(">>> Warning: cannot open {} — skipping world.set cross-check", WORLD_SET_FILE);
        }

        let mut candidates: Vec<String> = true_orphans;
        for p in untracked {
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
        candidates.sort();
        candidates.dedup();

        if candidates.is_empty() {
            println!();
            println!(">>> No orphaned or untracked packages were found on your system.");
            return Ok(());
        }

        println!();
        for o in &candidates {
            println!("[{}] {}", "unmerge".red().bold(), o);
        }
        println!();
        println!("Total: {} package(s) to remove", candidates.len());
        println!();

        // pacman rejects -n/--nosave together with --print, and
        // --nosave is meaningless for a dry run anyway (nothing gets
        // removed, so there's nothing to skip saving configs for).
        let mut pacman_args = if cli.pretend {
            vec!["-Rs", "--print"]
        } else {
            vec!["-Rns"]
        };
        if !cli.ask && !cli.pretend {
            pacman_args.push("--noconfirm");
        }

        // --print never touches the system, so it doesn't need root —
        // running it through sudo anyway just makes `-p` (pretend)
        // prompt for a password to print a list, which defeats the
        // point of a dry run.
        if cli.pretend {
            run_cmd(PACMAN_BIN, &pacman_args, &candidates);
        } else {
            let mut sudo_args = vec![PACMAN_BIN];
            sudo_args.extend(pacman_args);
            run_cmd(SUDO_BIN, &sudo_args, &candidates);
        }
        return Ok(());
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
            if let Err(e) = remove_from_world_set(&target_pkgs) {
                eprintln!(">>> Warning: package(s) unmerged but world.set was not updated: {:#}", e);
            }
        }
        return Ok(());
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
            if cli.pretend { return Ok(()); }
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
                if cli.pretend { return Ok(()); }
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
                if cli.pretend { return Ok(()); }
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
                if cli.pretend { return Ok(()); }
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
                        // makepkg -si marks explicit correctly on its own
                        // *unless* the package was already installed as a
                        // dependency beforehand — force it so world.set
                        // membership and pacman's bookkeeping always agree.
                        mark_asexplicit(&target_pkgs);
                        if let Err(e) = add_to_world_set(&target_pkgs, Some("abs")) {
                            eprintln!(">>> Warning: package(s) built but world.set was not updated: {:#}", e);
                        }
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
                        if let Err(e) = add_to_world_set(&official_names, None) {
                            eprintln!(">>> Warning: package(s) installed but world.set was not updated: {:#}", e);
                        }
                    }
                    if !aur_names.is_empty() {
                        // aura -A doesn't always leave the explicit bit set
                        // the way `pacman -S` does — force it so world.set
                        // and pacman's bookkeeping always agree.
                        mark_asexplicit(&aur_names);
                        if let Err(e) = add_to_world_set(&aur_names, Some("aur")) {
                            eprintln!(">>> Warning: package(s) installed but world.set was not updated: {:#}", e);
                        }
                    }
                }
            }

            if !success {
                eprintln!(">>> Warning: not all requested packages were installed successfully.");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}