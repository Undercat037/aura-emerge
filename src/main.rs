use clap::Parser;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

const WORLD_SET_FILE: &str = "/etc/emerge/world.set";

/// Emerge-like wrapper for Arch Linux using Aura
#[derive(Parser, Debug)]
#[command(
    name = "emerge",
    bin_name = "emerge",
    about = "Portage-like wrapper for Arch Linux using Aura",
    version = "1.12.0 (aura-emerge)\nAuthor: Undercat037"
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
    #[arg(short = 'O', long = "noreplace")]
    noreplace: bool,

    // Dummy flags for compatibility
    #[arg(short = 'D', long = "deep")]
    deep: bool,
    #[arg(short = 'N', long = "newuse")]
    newuse: bool,
    #[arg(short = 'e', long = "emptytree")]
    emptytree: bool,

    /// Packages to install or '@world'
    packages: Vec<String>,
}

/// Validate a package name — reject flags and characters invalid in package names.
fn validate_pkg(pkg: &str) -> bool {
    if pkg.starts_with('-') {
        return false;
    }
    pkg.chars()
        .all(|c| c.is_alphanumeric() || "@._+-/".contains(c))
}

/// Filter a slice of package names, printing a warning for each invalid one.
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

fn main() {
    let cli = Cli::parse();

    // 1. Search
    if cli.search {
        if cli.packages.is_empty() {
            eprintln!(">>> Error: Specify search term.");
            std::process::exit(1);
        }

        if cli.verbose {
            if cli.aur {
                run_cmd("aura", &["-Ai"], &cli.packages, false);
            } else {
                let found = run_cmd_quiet("aura", &["-Si"], &cli.packages);
                if found {
                    run_cmd("aura", &["-Si"], &cli.packages, false);
                } else {
                    // Inform the user about the fallback so they are not confused
                    println!(">>> Not found in official repos, searching AUR...");
                    run_cmd("aura", &["-Ai"], &cli.packages, false);
                }
            }
        } else if cli.aur {
            run_cmd("aura", &["-As"], &cli.packages, false);
        } else {
            run_cmd("aura", &["-Ss"], &cli.packages, false);
            println!();
            run_cmd("aura", &["-As"], &cli.packages, false);
        }
        return;
    }

    // 2. Sync
    if cli.sync {
        println!(">>> Syncing database...");
        run_cmd("aura", &["-Sy"], &[], false);
        return;
    }

    // 3. Update
    if cli.update && (cli.packages.is_empty() || cli.packages.contains(&"@world".to_string())) {
        println!(">>> Updating system (@world)...");

        let mut s_args = vec!["-Syu"];
        if cli.verbose {
            s_args.push("--verbose");
        }
        run_cmd("aura", &s_args, &[], false);

        // Update AUR packages
        run_cmd("aura", &["-Au"], &[], false);
        return;
    }

    // 4. Depclean (orphans)
    if cli.depclean {
        println!(">>> Checking for orphans...");

        let output = Command::new("pacman").arg("-Qtdq").output();

        match output {
            Ok(out) => {
                let orphans_str = String::from_utf8_lossy(&out.stdout);
                let orphans: Vec<String> = orphans_str
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if orphans.is_empty() {
                    println!(">>> No orphans found. System is clean!");
                    return;
                }

                println!(">>> Found {} orphan(s).", orphans.len());
                let mut pacman_args = vec!["pacman", "-Rns"];

                if cli.pretend {
                    pacman_args.push("--print");
                }
                if !cli.ask && !cli.pretend {
                    pacman_args.push("--noconfirm");
                }

                run_cmd("sudo", &pacman_args, &orphans, false);
            }
            Err(_) => eprintln!(">>> Error: Failed to check for orphans."),
        }
        return;
    }

    // 5. Unmerge (remove)
    if cli.unmerge {
        if cli.packages.is_empty() {
            eprintln!(">>> Error: Specify packages to remove.");
            std::process::exit(1);
        }

        // Validate package names before passing them to aura
        let valid_pkgs = validate_packages(&cli.packages);
        if valid_pkgs.is_empty() {
            return;
        }

        println!(">>> Unmerge mode: {:?}", valid_pkgs);

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

        let success = run_cmd("aura", &aura_args, &valid_pkgs, false);

        if success && !cli.pretend {
            remove_from_world_set(&valid_pkgs);
        }
        return;
    }

    // 6. Install
    if !cli.packages.is_empty() {
        // Filter out world aliases, then validate remaining package names
        let raw_pkgs: Vec<String> = cli
            .packages
            .iter()
            .filter(|p| *p != "world" && *p != "@world")
            .cloned()
            .collect();

        let target_pkgs = validate_packages(&raw_pkgs);

        if target_pkgs.is_empty() {
            return;
        }

        println!(">>> Install mode: {:?}", target_pkgs);

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
            let mut aur_args = vec!["-A"];
            if cli.pretend {
                aur_args.push("--dryrun");
            }
            aur_args.extend(&base_args);
            success = run_cmd("aura", &aur_args, &target_pkgs, false);
        } else {
            // Silently probe official repos first
            println!(">>> Checking official repositories...");
            let off_exists = run_cmd_quiet("aura", &["-Si"], &target_pkgs);

            if off_exists {
                let mut off_args = vec!["-S"];
                if cli.pretend {
                    off_args.push("--print");
                }
                if cli.verbose {
                    off_args.push("--verbose");
                }
                off_args.extend(&base_args);
                success = run_cmd("aura", &off_args, &target_pkgs, false);
            } else {
                // Inform the user so the switch to AUR is not silent/confusing
                println!(">>> Not found in official repos. Trying AUR...");
                let mut aur_args = vec!["-A"];
                if cli.pretend {
                    aur_args.push("--dryrun");
                }
                aur_args.extend(&base_args);
                success = run_cmd("aura", &aur_args, &target_pkgs, false);
            }
        }

        // Save to world.set only if install succeeded and not oneshot/pretend
        if success && !cli.oneshot && !cli.pretend {
            add_to_world_set(&target_pkgs);
        }
    }
}

/// Execute a command, inheriting stdio. Returns true on success.
fn run_cmd(prog: &str, args: &[&str], packages: &[String], _ignore_fail: bool) -> bool {
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

/// Execute silently (stdout/stderr suppressed). Used for probing without noise.
fn run_cmd_quiet(prog: &str, args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new(prog);
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }
    cmd.stderr(Stdio::null()).stdout(Stdio::null());
    match cmd.status() {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

fn add_to_world_set(packages: &[String]) {
    println!(">>> Adding to world.set...");

    let mut current_set: HashSet<String> = HashSet::new();
    if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
        let reader = io::BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                current_set.insert(trimmed);
            }
        }
    }

    let mut changed = false;
    for pkg in packages {
        if current_set.insert(pkg.clone()) {
            changed = true;
        }
    }

    if !changed {
        return;
    }

    let mut sorted: Vec<String> = current_set.into_iter().collect();
    sorted.sort();

    write_world_set(&sorted);
}

fn remove_from_world_set(packages: &[String]) {
    println!(">>> Removing from world.set...");

    let mut current_set: HashSet<String> = HashSet::new();
    if let Ok(file) = fs::File::open(WORLD_SET_FILE) {
        let reader = io::BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                current_set.insert(trimmed);
            }
        }
    }

    let mut changed = false;
    for pkg in packages {
        if current_set.remove(pkg) {
            changed = true;
        }
    }

    if !changed {
        return;
    }

    let mut sorted: Vec<String> = current_set.into_iter().collect();
    sorted.sort();

    write_world_set(&sorted);
}

/// Write a sorted package list to world.set via sudo tee.
/// Checks exit code and reports failure instead of panicking.
fn write_world_set(packages: &[String]) {
    let child_proc = Command::new("sudo")
        .arg("tee")
        .arg(WORLD_SET_FILE)
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
                Ok(status) if status.success() => println!(">>> world.set updated."),
                Ok(_) => eprintln!(">>> Error: sudo tee exited with non-zero status."),
                Err(e) => eprintln!(">>> Error waiting for sudo tee: {}", e),
            }
        }
        Err(e) => eprintln!(">>> Error: Failed to spawn sudo tee: {}", e),
    }
}