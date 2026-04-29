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
    version = "1.4.0 (aura-emerge)\nAuthor: Undercat037"
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

    // Dummy flags for compatibility
    #[arg(short = 'D', long = "deep")] deep: bool,
    #[arg(short = 'N', long = "newuse")] newuse: bool,
    #[arg(short = 'e', long = "emptytree")] emptytree: bool,
    #[arg(short = 'v', long = "verbose")] verbose: bool,

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
        let args = if cli.aur { vec!["-As"] } else { vec!["-Ss"] };
        run_aura(&args, &cli.packages);
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
        // Update repos, then AUR
        run_aura(&["-Syu"], &[]);
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
        let mut target_pkgs = Vec::new();
        for pkg in &cli.packages {
            if pkg == "world" || pkg == "@world" {
                continue; // Ignore "world" literal
            }
            target_pkgs.push(pkg.clone());
        }

        if target_pkgs.is_empty() {
            return;
        }

        println!(">>> Install mode: {:?}", target_pkgs);

        let mut aura_args = if cli.aur { vec!["-A"] } else { vec!["-S"] };

        if cli.pretend {
            aura_args.push("--print");
        }
        if !cli.ask && !cli.pretend {
            aura_args.push("--noconfirm");
        }
        if cli.oneshot {
            aura_args.push("--asdeps");
        }

        let success = run_aura(&aura_args, &target_pkgs);

        // Save to world.set if successful
        if success && !cli.oneshot && !cli.pretend {
            add_to_world_set(&target_pkgs);
        }
    }
}

/// Run aura command
fn run_aura(args: &[&str], packages: &[String]) -> bool {
    let mut cmd = Command::new("aura");
    cmd.args(args);
    for p in packages {
        cmd.arg(p);
    }

    match cmd.status() {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!(">>> Aura execution error: {}", e);
            false
        }
    }
}

/// Add to world.set via sudo
fn add_to_world_set(packages: &[String]) {
    println!(">>> Adding to world.set...");

    // Read existing
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

    // Add new
    let mut changed = false;
    for pkg in packages {
        if current_set.insert(pkg.clone()) {
            changed = true;
        }
    }

    if !changed {
        return;
    }

    // Sort
    let mut sorted_set: Vec<String> = current_set.into_iter().collect();
    sorted_set.sort();

    // Write via sudo tee
    let mut child = Command::new("sudo")
        .arg("tee")
        .arg(WORLD_SET_FILE)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("Failed to run sudo tee");

    if let Some(mut stdin) = child.stdin.take() {
        for pkg in sorted_set {
            if let Err(e) = writeln!(stdin, "{}", pkg) {
                eprintln!(">>> Write error: {}", e);
            }
        }
    }

    child.wait().expect("sudo tee failed");
    println!(">>> world.set updated.");
}