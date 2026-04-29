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
    version = "1.8.0 (aura-emerge)\nAuthor: Undercat037"
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

    /// Pretend (dry run)
    #[arg(short = 'p', long = "pretend")]
    pretend: bool,

    /// Ask before applying changes
    #[arg(short = 'a', long = "ask")]
    ask: bool,

    /// Install as dependency (no world.set)
    #[arg(short = '1', long = "oneshot")]
    oneshot: bool,

    /// Explicitly use AUR
    #[arg(long = "aur")]
    aur: bool,

    /// Verbose output / detailed info in search mode (-sv = aura -Si/-Ai)
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

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

fn main() {
    let cli = Cli::parse();

    // 1. Search
    if cli.search {
        if cli.packages.is_empty() {
            eprintln!(">>> Error: Specify search term.");
            std::process::exit(1);
        }

        if cli.verbose {
            // -sv: show detailed package info
            if cli.aur {
                // --aur explicitly specified — go straight to AUR
                run_aura(&["-Ai"], &cli.packages);
            } else {
                // silently probe official repos first
                let found = run_aura_quiet(&["-Si"], &cli.packages);
                if found {
                    // found in official repos — show normally
                    run_aura(&["-Si"], &cli.packages);
                } else {
                    // not in official repos — fall back to AUR without extra error output
                    run_aura(&["-Ai"], &cli.packages);
                }
            }
        } else {
            // -s: regular search — official repos + AUR
            if cli.aur {
                run_aura(&["-As"], &cli.packages);
            } else {
                run_aura(&["-Ss"], &cli.packages);
                println!();
                run_aura(&["-As"], &cli.packages);
            }
        }
        return;
    }

    // 2. Sync
    if cli.sync {
        println!(">>> Syncing database...");
        run_aura(&["-Sy"], &[]);
        return;
    }

    // 3. Update
    if cli.update && (cli.packages.is_empty() || cli.packages.contains(&"@world".to_string())) {
        println!(">>> Updating system (@world)...");

        let mut s_args = vec!["-Syu"];
        if cli.verbose {
            s_args.push("--verbose");
        }
        run_aura(&s_args, &[]);

        // Update AUR packages
        run_aura(&["-Au"], &[]);
        return;
    }

    // 4. Depclean
    if cli.depclean {
        println!(">>> Removing orphans...");
        run_aura(&["-O"], &[]);
        return;
    }

    // 5. Install
    if !cli.packages.is_empty() {
        // Filter out 'world' and '@world' literals
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

        let mut aura_args = if cli.aur { vec!["-A"] } else { vec!["-S"] };

        if cli.pretend {
            if cli.aur {
                aura_args.push("--dryrun");  // aura -A --dryrun
            } else {
                aura_args.push("--print");   // aura -S --print
            }
        }
        if !cli.ask && !cli.pretend {
            aura_args.push("--noconfirm");
        }
        if cli.oneshot {
            aura_args.push("--asdeps");
        }
        if cli.verbose && !cli.aur {
            aura_args.push("--verbose");
        }

        let success = run_aura(&aura_args, &target_pkgs);

        // Save to world.set only if install succeeded and not oneshot/pretend
        if success && !cli.oneshot && !cli.pretend {
            add_to_world_set(&target_pkgs);
        }
    }
}

/// Run aura, inherit all stdio, return success.
fn run_aura(args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new("aura");
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }
    match cmd.status() {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!(">>> Aura execution error: {}", e);
            false
        }
    }
}

/// Run aura with stderr and stdout suppressed — used for silent probing.
/// Returns true if the command succeeded (package found).
fn run_aura_quiet(args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new("aura");
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

/// Add packages to world.set via sudo tee.
fn add_to_world_set(packages: &[String]) {
    println!(">>> Adding to world.set...");

    // Read existing entries
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

    // Add new packages, track whether anything changed
    let mut changed = false;
    for pkg in packages {
        if current_set.insert(pkg.clone()) {
            changed = true;
        }
    }

    if !changed {
        return;
    }

    // Sort alphabetically before writing
    let mut sorted: Vec<String> = current_set.into_iter().collect();
    sorted.sort();

    // Write back via sudo tee (world.set is root-owned)
    let mut child = Command::new("sudo")
        .arg("tee")
        .arg(WORLD_SET_FILE)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("Failed to run sudo tee");

    if let Some(mut stdin) = child.stdin.take() {
        for pkg in sorted {
            if let Err(e) = writeln!(stdin, "{}", pkg) {
                eprintln!(">>> Write error: {}", e);
            }
        }
    }

    child.wait().expect("sudo tee failed");
    println!(">>> world.set updated.");
}