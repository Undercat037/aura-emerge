//! /etc/portage/world: explicit-install tracking, custom sets, and
//! `@world` provisioning (no -u).

use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

use crate::*;

/// Repo a package came from (e.g. "extra", "aur"). LC_ALL=C.
/// Tries -Qi first, then -Si for not-yet-installed.
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
            if line.starts_with("Installed From") || line.starts_with("Repository") {
                if let Some(val) = line.splitn(2, ':').nth(1) {
                    let r = val.trim().to_string();
                    if !r.is_empty() {
                        return Some(r);
                    }
                }
            }
        }
        None
    }

    // Local DB first; None = local build with no repo field.
    if let Some(stdout) = pacman_c(&["-Qi", bare]) {
        return first_repo(&stdout);
    }

    // Sync DB for not-yet-installed (--select etc.)
    if let Some(stdout) = pacman_c(&["-Si", bare]) {
        return first_repo(&stdout);
    }

    None
}

// ── batch repo resolution ────────────────────────────────────────────────
// Resolves a whole package list in at most two pacman spawns total
// (was one, sometimes two, per package -- see pkg_world_entry_from).

/// `key : value` line, exact match on key.

/// Batch repo lookup via libalpm. Some("None") = local build.
pub(crate) fn get_pkg_repos_batch(
    names: &[String],
) -> std::collections::HashMap<String, Option<String>> {
    let bares: Vec<String> = names
        .iter()
        .map(|n| n.split('/').last().unwrap_or(n).to_string())
        .collect();
    if bares.is_empty() {
        return std::collections::HashMap::new();
    }
    crate::alpm_db::repos_batch(&bares)
}

/// world entry ("repo/name", "Err/name" local, or bare "name") from an
/// already-resolved get_pkg_repos_batch lookup.
pub(crate) fn pkg_world_entry_from(
    bare: &str,
    forced_prefix: Option<&str>,
    repos: &std::collections::HashMap<String, Option<String>>,
) -> String {
    match repos.get(bare) {
        Some(Some(repo)) if repo != "None" => format!("{}/{}", repo, bare),
        Some(Some(_)) /* "None" = local build */ => {
            format!("{}/{}", forced_prefix.unwrap_or("Err"), bare)
        }
        _ => bare.to_string(),
    }
}

// ── custom sets (/etc/portage/sets/<name>[.set], invoked as @<name>) ──────────

/// Safe set name (becomes a filesystem path under sets/).
pub(crate) fn valid_set_name(name: &str) -> bool {
    !name.is_empty()
        && name != "world"
        && name != "preserved-rebuild"
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Where `@<name>` lives: `sets/<name>` (Portage style) or
/// `sets/<name>.set`. Both existing at once is an error rather than a
/// silent pick -- `@game-kit` would otherwise mean different things
/// depending on which file happened to win. Neither existing returns
/// the `.set` path, so the caller's "no such set" message has
/// something to point at.
pub(crate) fn resolve_set_path(name: &str) -> Result<String> {
    resolve_set_path_in(SETS_DIR, name)
}

fn resolve_set_path_in(dir: &str, name: &str) -> Result<String> {
    let bare = format!("{}/{}", dir, name);
    let ext = format!("{}/{}.set", dir, name);
    // symlink_metadata: a symlink counts as "exists" so the nofollow
    // open later can refuse it with its usual message.
    let is_entry = |p: &str| {
        fs::symlink_metadata(p)
            .map(|m| !m.is_dir())
            .unwrap_or(false)
    };
    match (is_entry(&bare), is_entry(&ext)) {
        (true, true) => bail!(
            "set @{} is ambiguous: both {} and {} exist - remove or rename one",
            name,
            bare,
            ext
        ),
        (true, false) => Ok(bare),
        _ => Ok(ext),
    }
}

/// Read sets/<name> or sets/<name>.set (one atom/line, # comments;
/// validate_pkg each).
pub(crate) fn read_custom_set(name: &str) -> Result<Vec<String>> {
    let path = resolve_set_path(name)?;

    let file = match open_nofollow(std::path::Path::new(&path)) {
        Ok(f) => f,
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to read", path)
        }
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "no such set: @{} (expected {}/{} or {}/{}.set)",
                    name, SETS_DIR, name, SETS_DIR, name
                )
            });
        }
    };

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

/// Read a --batchinstall list (same format as a custom set, any path).
pub(crate) fn read_batch_file(path: &str) -> Result<Vec<String>> {
    let file = match open_nofollow(std::path::Path::new(path)) {
        Ok(f) => f,
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to read", path)
        }
        Err(e) => {
            return Err(e).with_context(|| format!("could not open batch file: {}", path));
        }
    };

    let pkgs: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| {
            if validate_pkg(l) {
                true
            } else {
                eprintln!(">>> Warning: invalid entry in {} (skipped): {}", path, l);
                false
            }
        })
        .collect();

    Ok(pkgs)
}

/// Bare set names under SETS_DIR, sorted (--list-sets / completion).
/// `foo` and `foo.set` both list as `foo`; names that wouldn't be a
/// valid `@name` (dotfiles, `*.tmp`, editor backups) are skipped.
pub(crate) fn list_custom_sets() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(SETS_DIR) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(true) {
                continue;
            }
            let file = entry.file_name();
            let Some(file) = file.to_str() else { continue };
            let stem = file.strip_suffix(".set").unwrap_or(file);
            if valid_set_name(stem) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

// ── declarative provisioning from world (bare `emerge @world`) ─────────────

/// Drops packages `--exclude`/the mask cover, recording each in `held`.
/// Provisioning shouldn't abort over one masked entry -- it should
/// skip it and provision the rest.
fn hold_back(list: &mut Vec<String>, repo: Option<&str>, held: &mut Vec<String>) {
    list.retain(|name| {
        if crate::runtime::is_excluded(name) {
            held.push(format!("{} (--exclude)", name));
            return false;
        }
        if let Some(entry) = crate::mask::find(name, repo) {
            held.push(format!("{} (masked by {})", name, entry.describe()));
            return false;
        }
        true
    });
}

/// Provision missing packages from world (bare `@world`, no -u).
/// Prefix picks the source; `abs/` is listed only; bare always resolved;
/// `Err/` only with err_install. Fixes world prefixes on success.
pub(crate) fn provision_from_world_set(
    pretend: bool,
    ask: bool,
    verbose: bool,
    err_install: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
) -> Result<bool> {
    println!("{} Provisioning system from world...", ">>>".green().bold());

    let entries: Vec<String> = match open_nofollow(std::path::Path::new(WORLD_SET_FILE)) {
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
        }
        Ok(file) => io::BufReader::new(file)
            .lines()
            .map_while(io::Result::ok)
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => {
            println!(">>> world not found - nothing to provision.");
            return Ok(true);
        }
    };

    if entries.is_empty() {
        println!(">>> world is empty - nothing to provision.");
        return Ok(true);
    }

    // Already installed → skip.
    let installed: HashSet<String> = crate::alpm_db::installed_names();

    let mut official_missing: Vec<String> = Vec::new();
    let mut aur_missing: Vec<String> = Vec::new();
    let mut abs_missing: Vec<String> = Vec::new();
    // Err/: list only unless --err-install. Bare: always retry.
    let mut err_missing: Vec<String> = Vec::new();
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

    // Applied per source so a repo-prefixed mask entry only fires
    // against that source, and before the plan is printed.
    let mut held: Vec<String> = Vec::new();
    hold_back(&mut official_missing, None, &mut held);
    hold_back(&mut aur_missing, Some("aur"), &mut held);
    hold_back(&mut abs_missing, Some("abs"), &mut held);
    hold_back(&mut bare_missing, None, &mut held);
    hold_back(&mut err_missing, None, &mut held);
    if !held.is_empty() {
        println!(
            ">>> {} world entry(ies) held back: {}",
            held.len(),
            held.join(", ")
        );
    }

    // Bare always resolved; Err/ only with --err-install.
    let mut to_resolve: Vec<String> = bare_missing.clone();
    if err_install {
        to_resolve.extend(err_missing.iter().cloned());
    }

    // Resolve bare/Err so the plan shows real sources.
    let mut resolved_official: Vec<String> = Vec::new();
    let mut resolved_aur: Vec<String> = Vec::new();
    if !to_resolve.is_empty() {
        let (found, missing) = probe_official_split(&to_resolve);
        resolved_official = found.into_iter().map(|p| p.name).collect();
        resolved_aur = missing;
    }

    let unresolved_listed: Vec<String> = if err_install {
        Vec::new()
    } else {
        err_missing.clone()
    };

    let total = official_missing.len()
        + aur_missing.len()
        + abs_missing.len()
        + unresolved_listed.len()
        + resolved_official.len()
        + resolved_aur.len();
    if total == 0 {
        println!(
            "{} Nothing to do - every world package is already installed.",
            ">>>".green().bold()
        );
        return Ok(true);
    }

    println!();
    println!(
        "{}",
        "These are the packages that would be merged, in order:"
            .green()
            .bold()
    );
    println!();
    println!("Calculating dependencies... done!");
    println!();
    for p in official_missing.iter().chain(aur_missing.iter()) {
        println!(
            "[{} {:<4}] {}",
            "ebuild".green(),
            "N".green().bold(),
            p.green().bold()
        );
    }
    for p in &resolved_official {
        println!(
            "[{} {:<4}] {} (source was unresolved - found in official repos)",
            "ebuild".green(),
            "N".green().bold(),
            p.green().bold()
        );
    }
    for p in &resolved_aur {
        println!(
            "[{} {:<4}] {} (source was unresolved - will try the AUR)",
            "ebuild".green(),
            "N".cyan().bold(),
            p.cyan().bold()
        );
    }
    for p in &abs_missing {
        println!(
            "[{} {:<4}] {} (built from ABS - needs `emerge {} --abs`)",
            "ebuild".green(),
            "N".yellow().bold(),
            p.yellow().bold(),
            p
        );
    }
    for p in &unresolved_listed {
        println!("[{} {:<4}] {} (installed from an unknown source - retry with --err-install, or install manually)", "ebuild".green(), "N".red().bold(), p.red().bold());
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), total);
    println!();

    if pretend {
        return Ok(true);
    }

    let mut overall_ok = true;

    if !official_missing.is_empty() {
        println!(
            "{} Installing {} package(s) from official repos...",
            ">>>".green().bold(),
            official_missing.len()
        );
        let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed"];
        if verbose {
            args.push("--verbose");
        }
        if !ask {
            args.push("--noconfirm");
        }
        let snapshot = world_installed_snapshot();
        let install_ok = crate::pacman_install(&args, &official_missing);
        reconcile_world_after_install(&snapshot);
        if !install_ok {
            overall_ok = false;
            eprintln!(">>> Warning: some official-repo package(s) failed to install.");
            if !crate::runtime::keep_going() {
                eprintln!(
                    ">>> Stopping here; pass --keep-going to continue with the rest of world."
                );
                return Ok(false);
            }
        }
    }

    if !aur_missing.is_empty() {
        println!(
            "{} Installing {} AUR package(s)...",
            ">>>".green().bold(),
            aur_missing.len()
        );
        scan_aur_pkgbuilds_or_abort(&aur_missing);
        // aur_install sets explicit on success; no mark_asexplicit needed.
        if !aur_install(
            &aur_missing,
            false,
            ask,
            false,
            false,
            false,
            no_sandbox,
            skip_srcinfo_regen,
            unshare_net_build,
            false,
        ) {
            overall_ok = false;
            eprintln!(">>> Warning: some AUR package(s) failed to install.");
            if !crate::runtime::keep_going() {
                eprintln!(
                    ">>> Stopping here; pass --keep-going to continue with the rest of world."
                );
                return Ok(false);
            }
        }
    }

    if !resolved_official.is_empty() {
        println!(
            "{} Installing {} previously-unresolved package(s) from official repos...",
            ">>>".green().bold(),
            resolved_official.len()
        );
        let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed"];
        if verbose {
            args.push("--verbose");
        }
        if !ask {
            args.push("--noconfirm");
        }
        let snapshot = world_installed_snapshot();
        let install_ok = crate::pacman_install(&args, &resolved_official);
        reconcile_world_after_install(&snapshot);
        if install_ok {
            // Fix world prefix now that the real repo is known.
            if let Err(e) = add_to_world_set(&resolved_official, None) {
                eprintln!(
                    ">>> Warning: package(s) installed but world was not updated: {:#}",
                    e
                );
            }
        } else {
            overall_ok = false;
            eprintln!(">>> Warning: some previously-unresolved package(s) failed to install from official repos.");
            if !crate::runtime::keep_going() {
                return Ok(false);
            }
        }
    }

    if !resolved_aur.is_empty() {
        println!(
            "{} Installing {} previously-unresolved package(s) via the AUR...",
            ">>>".green().bold(),
            resolved_aur.len()
        );
        scan_aur_pkgbuilds_or_abort(&resolved_aur);
        if aur_install(
            &resolved_aur,
            false,
            ask,
            false,
            false,
            false,
            no_sandbox,
            skip_srcinfo_regen,
            unshare_net_build,
            false,
        ) {
            if let Err(e) = add_to_world_set(&resolved_aur, Some("aur")) {
                eprintln!(
                    ">>> Warning: package(s) installed but world was not updated: {:#}",
                    e
                );
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
            "{} {} package(s) were built from ABS and can't be reproduced unattended - \
            install them yourself: `emerge <pkg> --abs`",
            " *".yellow().bold(),
            abs_missing.len()
        );
        for p in &abs_missing {
            eprintln!("     {}", p);
        }
    }
    if !unresolved_listed.is_empty() {
        eprintln!(
            "{} {} package(s) are installed from an unknown source - retry with \
            `--err-install` to attempt the normal official/AUR install path, or install manually.",
            " *".yellow().bold(),
            unresolved_listed.len()
        );
        for p in &unresolved_listed {
            eprintln!("     {}", p);
        }
    }

    Ok(overall_ok)
}

// ── world ─────────────────────────────────────────────────────────────────

/// Re-resolve repo prefixes for every world entry.
pub(crate) fn regen_world_set() -> Result<()> {
    println!(
        "{} Regenerating world repository prefixes...",
        ">>>".green().bold()
    );

    let file = match open_nofollow_rw(std::path::Path::new(WORLD_SET_FILE)) {
        Ok(f) => f,
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to modify", WORLD_SET_FILE);
        }
        Err(e) => {
            return Err(e).with_context(|| format!("cannot open {}", WORLD_SET_FILE));
        }
    };

    let entries: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let bares: Vec<String> = entries
        .iter()
        .map(|e| e.split('/').last().unwrap_or(e).to_string())
        .collect();
    let repos = get_pkg_repos_batch(&bares);

    let mut updated: Vec<String> = Vec::new();
    let mut changed = 0usize;

    for entry in &entries {
        let bare = entry.split('/').last().unwrap_or(entry);
        // Keep abs/aur prefix if re-resolution can't find a live repo.
        let old_prefix = entry
            .split('/')
            .next()
            .filter(|p| *p == "abs" || *p == "aur");
        let new_entry = pkg_world_entry_from(bare, old_prefix, &repos);
        if &new_entry != entry {
            println!("  {} -> {}", entry, new_entry);
            changed += 1;
        }
        updated.push(new_entry);
    }

    if changed == 0 {
        println!("{} world is already up to date.", ">>>".green().bold());
        return Ok(());
    }

    updated.sort();
    write_world_set(&updated)?;
    println!(
        "{} world updated ({} entries changed).",
        ">>>".green().bold(),
        changed
    );
    Ok(())
}

/// Re-resolve prefixes in a custom set. sort=true also alphabetizes
/// (drops comments/blank-line grouping).
pub(crate) fn regen_set(name: &str, sort: bool) -> Result<()> {
    println!(
        "{} Regenerating prefixes for @{}...",
        ">>>".green().bold(),
        name
    );

    let path = resolve_set_path(name)?;
    let file = match open_nofollow_rw(std::path::Path::new(&path)) {
        Ok(f) => f,
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to modify", path);
        }
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "no such set: @{} (expected {}/{} or {}/{}.set)",
                    name, SETS_DIR, name, SETS_DIR, name
                )
            });
        }
    };

    let raw_lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .collect();

    if raw_lines.iter().all(|l| l.trim().is_empty()) {
        println!(
            ">>> @{} is empty or does not exist ({}) - nothing to regenerate.",
            name, path
        );
        return Ok(());
    }

    // Batch-resolve every valid entry's repo up front -- one or two
    // pacman calls for the whole set instead of one per line.
    let bares: Vec<String> = raw_lines
        .iter()
        .map(|l| l.trim())
        .filter(|t| !t.is_empty() && !t.starts_with('#') && validate_pkg(t))
        .map(|t| t.split('/').last().unwrap_or(t).to_string())
        .collect();
    let repos = get_pkg_repos_batch(&bares);

    let mut changed = 0usize;
    // sort=false: rewrite in place (keep comments/blanks).
    let mut rewritten: Vec<String> = Vec::new();
    // Package entries only (input for --regen-sort).
    let mut pkg_entries: Vec<String> = Vec::new();

    for raw in &raw_lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            rewritten.push(raw.clone());
            continue;
        }
        if !validate_pkg(trimmed) {
            eprintln!(
                ">>> Warning: invalid entry in @{} (left as-is): {}",
                name, trimmed
            );
            rewritten.push(raw.clone());
            continue;
        }

        let bare = trimmed.split('/').last().unwrap_or(trimmed);
        // Keep abs/aur if re-resolution misses (same as regen_world_set).
        let old_prefix = trimmed
            .split('/')
            .next()
            .filter(|p| *p == "abs" || *p == "aur");
        let new_entry = pkg_world_entry_from(bare, old_prefix, &repos);
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
            println!(
                "{} @{} is already up to date (sorted).",
                ">>>".green().bold(),
                name
            );
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
        bail!("{} is a symlink - refusing to write", tmp);
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
        println!(
            "{} @{} updated ({} entries changed, re-sorted).",
            ">>>".green().bold(),
            name,
            changed
        );
    } else {
        println!(
            "{} @{} updated ({} entries changed).",
            ">>>".green().bold(),
            name,
            changed
        );
    }
    Ok(())
}

pub(crate) fn add_to_world_set(packages: &[String], forced_prefix: Option<&str>) -> Result<()> {
    println!("{} Adding to world...", ">>>".green().bold());

    // bare name → full "repo/name" entry
    let mut current_set: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    match open_nofollow(std::path::Path::new(WORLD_SET_FILE)) {
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
        }
        Ok(file) => {
            for line in io::BufReader::new(file).lines().map_while(Result::ok) {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() && validate_pkg(&trimmed) {
                    let bare = trimmed.split('/').last().unwrap_or(&trimmed).to_string();
                    current_set.insert(bare, trimmed);
                }
            }
        }
        Err(_) => {}
    }

    let bares: Vec<String> = packages
        .iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    let repos = get_pkg_repos_batch(&bares);

    let mut changed = false;
    for pkg in packages {
        let bare = pkg.split('/').last().unwrap_or(pkg).to_string();
        let entry = pkg_world_entry_from(&bare, forced_prefix, &repos);
        // Overwrite so stale prefixes get corrected.
        let stale = current_set.get(&bare).map(|e| e != &entry).unwrap_or(true);
        if stale {
            current_set.insert(bare, entry);
            changed = true;
        }
    }

    if !changed {
        return Ok(());
    }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted)
}

// Conflict-removal reconciliation
//
// Detection removes the package from world if pacman removed it while resolving a conflict.

fn installed_bare_names() -> HashSet<String> {
    crate::alpm_db::installed_names()
}

/// Call before a `pacman -S` that might conflict-remove another
/// installed package. Cheap: two reads, no writes.
pub(crate) fn world_installed_snapshot() -> HashSet<String> {
    let world_bare: HashSet<String> = match open_nofollow(std::path::Path::new(WORLD_SET_FILE)) {
        Ok(file) => io::BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .map(|l| l.trim().split('/').last().unwrap_or("").to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => return HashSet::new(),
    };
    let installed = installed_bare_names();
    world_bare.intersection(&installed).cloned().collect()
}

/// Call after that same `pacman -S` with the snapshot from before it.
/// Anything that was installed then and isn't now got removed by
/// pacman, not us -- untrack it. Best-effort; never fails the caller.
pub(crate) fn reconcile_world_after_install(before: &HashSet<String>) {
    if before.is_empty() {
        return;
    }
    let installed_after = installed_bare_names();
    let vanished: Vec<String> = before
        .iter()
        .filter(|name| !installed_after.contains(*name))
        .cloned()
        .collect();
    if vanished.is_empty() {
        return;
    }
    println!(
        "{} pacman removed {} while resolving a conflict during this install \
        - removing from world too (it can't be reinstalled the way it was): {}",
        ">>>".yellow().bold(),
        vanished.len(),
        vanished.join(", ")
    );
    if let Err(e) = remove_from_world_set(&vanished) {
        eprintln!(
            ">>> Warning: conflict-removed package(s) but world was not updated: {:#}",
            e
        );
    }
}

pub(crate) fn remove_from_world_set(packages: &[String]) -> Result<()> {
    println!(">>> Removing from world...");

    let mut current_set: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    match open_nofollow(std::path::Path::new(WORLD_SET_FILE)) {
        Err(e) if is_symlink_open_error(&e) => {
            bail!("{} is a symlink - refusing to read", WORLD_SET_FILE);
        }
        Ok(file) => {
            for line in io::BufReader::new(file).lines().map_while(Result::ok) {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() && validate_pkg(&trimmed) {
                    let bare = trimmed.split('/').last().unwrap_or(&trimmed).to_string();
                    current_set.insert(bare, trimmed);
                }
            }
        }
        Err(_) => {}
    }

    let mut changed = false;
    for pkg in packages {
        let bare = pkg.split('/').last().unwrap_or(pkg).to_string();
        if current_set.remove(&bare).is_some() {
            changed = true;
        }
    }

    if !changed {
        return Ok(());
    }

    let mut sorted: Vec<String> = current_set.into_values().collect();
    sorted.sort();
    write_world_set(&sorted)
}

// --resume: argv of the last non-pretend install; cleared on success.

/// Save resume argv (best-effort; failure must not abort the real op).
pub(crate) fn save_resume_state(args: &[String]) {
    if !is_safe_path(RESUME_TMP) || !is_safe_path(RESUME_FILE) {
        eprintln!(">>> Warning: refusing to save resume state - symlink detected");
        return;
    }

    let _ = Command::new(SUDO_BIN)
        .args([RM_BIN, "-f", RESUME_TMP])
        .status();

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

/// Clear resume state after a fully successful operation.
pub(crate) fn clear_resume_state() {
    let _ = Command::new(SUDO_BIN)
        .args([RM_BIN, "-f", RESUME_FILE])
        .status();
}

/// Load resume argv, if any.
pub(crate) fn load_resume_state() -> Option<Vec<String>> {
    let file = match open_nofollow(std::path::Path::new(RESUME_FILE)) {
        Ok(f) => f,
        Err(_) => return None,
    };
    let lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

// --undo: one step of install/unmerge history (no version snapshot for -u).

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

/// Save undo state (best-effort).
pub(crate) fn save_last_action(kind: LastAction, atoms: &[String]) {
    if atoms.is_empty() {
        return;
    }
    if !is_safe_path(LASTACTION_TMP) || !is_safe_path(LASTACTION_FILE) {
        eprintln!(">>> Warning: refusing to save undo state - symlink detected");
        return;
    }

    let _ = Command::new(SUDO_BIN)
        .args([RM_BIN, "-f", LASTACTION_TMP])
        .status();

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

/// Clear undo state after --undo acts on it.
pub(crate) fn clear_last_action() {
    let _ = Command::new(SUDO_BIN)
        .args([RM_BIN, "-f", LASTACTION_FILE])
        .status();
}

/// Load undo state: (kind tag, atoms).
pub(crate) fn load_last_action() -> Option<(String, Vec<String>)> {
    let file = match open_nofollow(std::path::Path::new(LASTACTION_FILE)) {
        Ok(f) => f,
        Err(_) => return None,
    };
    let mut lines = io::BufReader::new(file)
        .lines()
        .map_while(io::Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    let kind = lines.next()?;
    let atoms: Vec<String> = lines.collect();
    if atoms.is_empty() {
        None
    } else {
        Some((kind, atoms))
    }
}

pub(crate) fn write_world_set(packages: &[String]) -> Result<()> {
    if !is_safe_path(WORLD_SET_TMP) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_TMP);
    }
    if !is_safe_path(WORLD_SET_FILE) {
        bail!("Refusing to write: {} is a symlink", WORLD_SET_FILE);
    }

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
                            eprintln!(">>> Error writing to world pipeline: {}", e);
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
        bail!("failed to write {} via sudo tee", WORLD_SET_TMP);
    }

    let status = Command::new(SUDO_BIN)
        .args([MV_BIN, WORLD_SET_TMP, WORLD_SET_FILE])
        .status()
        .context("sudo mv could not be spawned")?;

    if !status.success() {
        bail!("sudo mv failed when finalizing world");
    }

    println!(">>> world updated.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("ae-sets-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn set_path_accepts_bare_name_and_dot_set() {
        let dir = temp("either");
        let d = dir.to_str().unwrap();
        fs::write(dir.join("a"), "nano\n").unwrap();
        fs::write(dir.join("b.set"), "vim\n").unwrap();
        assert_eq!(resolve_set_path_in(d, "a").unwrap(), format!("{}/a", d));
        assert_eq!(resolve_set_path_in(d, "b").unwrap(), format!("{}/b.set", d));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_path_with_both_files_is_ambiguous() {
        let dir = temp("both");
        let d = dir.to_str().unwrap();
        fs::write(dir.join("cust-set"), "nano\n").unwrap();
        fs::write(dir.join("cust-set.set"), "vim\n").unwrap();
        let err = resolve_set_path_in(d, "cust-set").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_path_for_missing_set_points_at_dot_set() {
        let dir = temp("none");
        let d = dir.to_str().unwrap();
        assert_eq!(
            resolve_set_path_in(d, "nope").unwrap(),
            format!("{}/nope.set", d)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_directory_with_the_set_name_is_not_a_set() {
        let dir = temp("dir");
        let d = dir.to_str().unwrap();
        fs::create_dir(dir.join("x")).unwrap();
        fs::write(dir.join("x.set"), "nano\n").unwrap();
        assert_eq!(resolve_set_path_in(d, "x").unwrap(), format!("{}/x.set", d));
        let _ = fs::remove_dir_all(dir);
    }
}
