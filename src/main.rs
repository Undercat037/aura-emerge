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
    version = "1.11.0 (aura-emerge)\nAuthor: Undercat037"
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
    #[arg(short = 'D', long = "deep")] deep: bool,
    #[arg(short = 'N', long = "newuse")] newuse: bool,
    #[arg(short = 'e', long = "emptytree")] emptytree: bool,

    /// Packages to install or '@world'
    packages: Vec<String>,
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
                    run_cmd("aura", &["-Ai"], &cli.packages, false);
                }
            }
        } else {
            if cli.aur {
                run_cmd("aura", &["-As"], &cli.packages, false);
            } else {
                run_cmd("aura", &["-Ss"], &cli.packages, false);
                println!();
                run_cmd("aura", &["-As"], &cli.packages, false);
            }
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

    // 4. Depclean (Orphans)
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

                println!(">>> Found {} orphans.", orphans.len());
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

    // 5. Unmerge (Remove)
    if cli.unmerge {
        if cli.packages.is_empty() {
            eprintln!(">>> Error: Specify packages to remove.");
            std::process::exit(1);
        }

        println!(">>> Unmerge mode: {:?}", cli.packages);

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

        let success = run_cmd("aura", &aura_args, &cli.packages, false);

        if success && !cli.pretend {
            remove_from_world_set(&cli.packages);
        }
        return;
    }

    // 6. Install
    if !cli.packages.is_empty() {
        let target_pkgs: Vec<String> = cli
            .packages
            .iter()
            .filter(|p| *p != "world" && *p != "@world")
            .cloned()
            .collect();

        if target_pkgs.is_empty() {
            return;
        }

        println!(">>> Install mode: {:?}", target_pkgs);

        let mut base_args = Vec::new();
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
            if cli.pretend { aur_args.push("--dryrun"); }
            aur_args.extend(&base_args);
            success = run_cmd("aura", &aur_args, &target_pkgs, false);
        } else {
            // Check if packages exist in official repos first
            println!(">>> Checking official repositories...");
            let off_exists = run_cmd_quiet("aura", &["-Si"], &target_pkgs);

            if off_exists {
                // Install from official repos
                let mut off_args = vec!["-S"];
                if cli.pretend { off_args.push("--print"); }
                if cli.verbose { off_args.push("--verbose"); }
                off_args.extend(&base_args);
                success = run_cmd("aura", &off_args, &target_pkgs, false);
            } else {
                // Not found in official repos, try AUR
                println!(">>> Not found in official repos. Trying AUR...");
                let mut aur_args = vec!["-A"];
                if cli.pretend { aur_args.push("--dryrun"); }
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

/// Execute a command. If `ignore_fail` is true, the program will not output its own errors,
/// allowing the package manager to handle them (useful for fallbacks).
fn run_cmd(prog: &str, args: &[&str], packages: &[String], ignore_fail: bool) -> bool {
    let mut cmd = Command::new(prog);
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }
    match cmd.status() {
        Ok(s) => {
            if ignore_fail {
                s.success()
            } else {
                s.success()
            }
        }
        Err(e) => {
            eprintln!(">>> Execution error ({}): {}", prog, e);
            false
        }
    }
}

/// Execute silently (no stdout/stderr). Returns true if successful.
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

    write_to_world_set(&sorted);
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

    write_to_world_set(&sorted);
}

/// Safely write to world.set without panics/expects
fn write_to_world_set(packages: &[String]) {
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
                Ok(status) => {
                    if status.success() {
                        println!(">>> world.set updated.");
                    } else {
                        eprintln!(">>> Error: sudo tee exited with non-zero status.");
                    }
                }
                Err(e) => eprintln!(">>> Error waiting for sudo tee: {}", e),
            }
        }
        Err(e) => eprintln!(">>> Error: Failed to spawn sudo tee: {}", e),
    }
}