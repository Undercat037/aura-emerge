use clap::Parser;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

const WORLD_SET_FILE: &str = "/etc/emerge/world.set";
const WORLD_SET_TMP: &str = "/etc/emerge/world.set.tmp";

/// Emerge-like wrapper for Arch Linux using Aura
#[derive(Parser, Debug)]
#[command(
    name = "emerge",
    bin_name = "emerge",
    about = "Portage-like wrapper for Arch Linux using Aura",
    version = "1.14.0 (aura-emerge)\nAuthor: Undercat037"
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

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_pkg(pkg: &str) -> bool {
    if pkg.starts_with('-') {
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

// ── Package info ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PkgInfo {
    name: String,
    version: String,
    is_new: bool,
}

/// Probe official repos in a single call using --print-format.
/// Returns Some(infos) if all packages are found, None if not found.
fn probe_official(pkgs: &[String]) -> Option<Vec<PkgInfo>> {
    let mut args = vec!["-Sp", "--print-format", "%n %v", "--color", "never"];
    let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
    args.extend_from_slice(&pkg_refs);

    let output = Command::new("aura")
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

    if infos.is_empty() {
        None
    } else {
        Some(infos)
    }
}

/// Fetch AUR package info via -Ai output parsing.
fn resolve_aur(pkgs: &[String]) -> Vec<PkgInfo> {
    let mut result = Vec::new();
    for pkg in pkgs {
        let output = Command::new("aura")
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
    Command::new("pacman")
        .args(["-Q", pkg])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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

// ── Main ──────────────────────────────────────────────────────────────────────

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
                println!(">>> Searching in AUR for '{}'...", cli.packages.join(" "));
                run_cmd("aura", &["-Ai"], &cli.packages, false);
            } else {
                // Single probe: try official first
                let found = probe_official(&cli.packages).is_some();
                if found {
                    println!(">>> Searching for '{}'...", cli.packages.join(" "));
                    run_cmd("aura", &["-Si"], &cli.packages, false);
                } else {
                    println!(
                        ">>> '{}' not found in official repos, searching AUR...",
                        cli.packages.join(" ")
                    );
                    run_cmd("aura", &["-Ai"], &cli.packages, false);
                }
            }
        } else if cli.aur {
            println!(">>> Searching in AUR for '{}'...", cli.packages.join(" "));
            run_cmd("aura", &["-As"], &cli.packages, false);
        } else {
            println!(">>> Searching for '{}'...", cli.packages.join(" "));
            run_cmd("aura", &["-Ss"], &cli.packages, false);
            println!();
            println!(">>> Searching in AUR for '{}'...", cli.packages.join(" "));
            run_cmd("aura", &["-As"], &cli.packages, false);
        }
        return;
    }

    // 2. Sync
    if cli.sync {
        println!(">>> Syncing package databases...");
        run_cmd("aura", &["-Sy"], &[], false);
        return;
    }

    // 3. Update @world
    if cli.update && (cli.packages.is_empty() || cli.packages.contains(&"@world".to_string())) {
        println!(">>> Calculating dependencies... done!");
        println!();
        println!(">>> Upgrading system (official repos)...");
        let mut s_args = vec!["-Syu"];
        if cli.verbose {
            s_args.push("--verbose");
        }
        run_cmd("aura", &s_args, &[], false);

        println!(">>> Upgrading AUR packages...");
        run_cmd("aura", &["-Au"], &[], false);

        println!();
        println!(">>> Auto-cleaning packages...");
        return;
    }

    // 4. Depclean (orphans)
    if cli.depclean {
        println!(">>> Calculating dependencies... done!");
        println!(">>> Checking for orphaned packages...");

        match Command::new("pacman").arg("-Qtdq").output() {
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

        let valid_pkgs = validate_packages(&cli.packages);
        if valid_pkgs.is_empty() {
            return;
        }

        println!("Calculating dependencies... done!");
        println!();
        for p in &valid_pkgs {
            println!("[unmerge     ] {}", p);
        }
        println!();
        println!(">>> Unmerging {}...", valid_pkgs.join(", "));

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
            if cli.pretend {
                return;
            }
            print_emerge_emerging(&pkg_infos);

            let mut aur_args = vec!["-A"];
            aur_args.extend(&base_args);
            success = run_cmd("aura", &aur_args, &target_pkgs, false);
        } else {
            // Single probe — get info and check existence in one call
            if let Some(pkg_infos) = probe_official(&target_pkgs) {
                print_emerge_plan(&pkg_infos);
                if cli.pretend {
                    return;
                }
                print_emerge_emerging(&pkg_infos);

                let mut off_args = vec!["-S"];
                if cli.verbose {
                    off_args.push("--verbose");
                }
                off_args.extend(&base_args);
                success = run_cmd("aura", &off_args, &target_pkgs, false);
            } else {
                println!(
                    ">>> Not found in official repos. Searching AUR for '{}'...",
                    target_pkgs.join(", ")
                );
                let pkg_infos = resolve_aur(&target_pkgs);
                print_emerge_plan(&pkg_infos);
                if cli.pretend {
                    return;
                }
                print_emerge_emerging(&pkg_infos);

                let mut aur_args = vec!["-A"];
                aur_args.extend(&base_args);
                success = run_cmd("aura", &aur_args, &target_pkgs, false);
            }
        }

        if success && !cli.oneshot && !cli.pretend {
            println!();
            println!(">>> Auto-cleaning packages...");
            add_to_world_set(&target_pkgs);
        }
    }
}

// ── Command helpers ───────────────────────────────────────────────────────────

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


// ── world.set ─────────────────────────────────────────────────────────────────

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

/// Atomic write: tee to .tmp then mv to final path.
fn write_world_set(packages: &[String]) {
    let write_ok = {
        let child_proc = Command::new("sudo")
            .arg("tee")
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
                    Ok(_) => {
                        eprintln!(">>> Error: sudo tee exited with non-zero status.");
                        false
                    }
                    Err(e) => {
                        eprintln!(">>> Error waiting for sudo tee: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                eprintln!(">>> Error: Failed to spawn sudo tee: {}", e);
                false
            }
        }
    };

    if !write_ok {
        return;
    }

    match Command::new("sudo")
        .args(["mv", WORLD_SET_TMP, WORLD_SET_FILE])
        .status()
    {
        Ok(s) if s.success() => println!(">>> world.set updated."),
        Ok(_) => eprintln!(">>> Error: sudo mv failed when finalizing world.set."),
        Err(e) => eprintln!(">>> Error: sudo mv could not be spawned: {}", e),
    }
}