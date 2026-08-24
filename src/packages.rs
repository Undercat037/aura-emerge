//! Package resolve/probe, ABS builds, portageq, --info, @preserved-rebuild.

use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::*;

// ── Package info ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct PkgInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) repo: String,
    /// "N" new, "U" upgrade, "D" downgrade, "R" reinstall
    pub(crate) status: String,
}

/// Install status: N/U/D/R.
pub(crate) fn pkg_status(name: &str, new_ver: &str) -> String {
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
    let cmp: i32 = std::process::Command::new(VERCMP_BIN)
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

/// Display atom (repo/name-ver).
pub(crate) fn format_atom(p: &PkgInfo) -> String {
    if p.repo.is_empty() {
        format!("{}-{}", p.name, p.version)
    } else {
        format!("{}/{}-{}", p.repo, p.name, p.version)
    }
}

/// Colored N/U/D/R badge.
pub(crate) fn status_colored(status: &str) -> String {
    match status {
        "N" => status.green().bold().to_string(),
        "U" => status.yellow().bold().to_string(),
        "D" => status.red().bold().to_string(),
        _   => status.cyan().bold().to_string(),
    }
}

// ── Explicit / dependency flag helpers ──────────────────────────────────────

/// pacman -D --asexplicit (AUR/ABS/--select). Best-effort if not installed.
pub(crate) fn mark_asexplicit(pkgs: &[String]) {
    let bare: Vec<String> = pkgs.iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    if bare.is_empty() { return; }

    let mut args: Vec<&str> = vec![PACMAN_BIN, "-D", "--asexplicit"];
    args.extend(bare.iter().map(String::as_str));
    let _ = Command::new(SUDO_BIN)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// pacman -D --asdeps (--deselect / transitive deps). Best-effort.
pub(crate) fn mark_asdeps(pkgs: &[String]) {
    let bare: Vec<String> = pkgs.iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    if bare.is_empty() { return; }

    let mut args: Vec<&str> = vec![PACMAN_BIN, "-D", "--asdeps"];
    args.extend(bare.iter().map(String::as_str));
    let _ = Command::new(SUDO_BIN)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Probe official repos via -Sp --print-format. Some(infos) or None.
pub(crate) fn probe_official(pkgs: &[String]) -> Option<Vec<PkgInfo>> {
    let mut args = vec!["-Sp", "--print-format", "%n %v", "--color", "never"];
    let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
    args.extend_from_slice(&pkg_refs);

    let output = Command::new(PACMAN_BIN)
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

/// Split into found/missing. Batch -Sp first, then per-name for misses.
pub(crate) fn probe_official_split(pkgs: &[String]) -> (Vec<PkgInfo>, Vec<String>) {
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

/// AUR RPC info → (found, missing). Missing is not a hard error.
pub(crate) fn resolve_aur_split(pkgs: &[String]) -> (Vec<PkgInfo>, Vec<String>) {
    let infos = crate::aur::rpc_info(pkgs);
    let by_name: HashMap<&str, &crate::aur::AurPkgInfo> =
        infos.iter().map(|i| (i.name.as_str(), i)).collect();

    let mut result = Vec::new();
    let mut missing = Vec::new();
    for pkg in pkgs {
        let bare = pkg.split('/').last().unwrap_or(pkg);
        match by_name.get(bare) {
            Some(info) => {
                let status = pkg_status(&info.name, &info.version);
                result.push(PkgInfo {
                    name: info.name.clone(),
                    version: info.version.clone(),
                    repo: "aur".to_string(),
                    status,
                });
            }
            None => missing.push(pkg.clone()),
        }
    }
    (result, missing)
}


// ── Emerge-style output ───────────────────────────────────────────────────────

pub(crate) fn print_emerge_plan(pkgs: &[PkgInfo]) {
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

pub(crate) fn print_emerge_emerging(pkgs: &[PkgInfo]) {
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

/// Refresh versions from pacman -Q after install (plan may be stale).
pub(crate) fn refresh_installed_versions(pkgs: &mut [PkgInfo]) {
    for p in pkgs.iter_mut() {
        let out = Command::new(PACMAN_BIN)
            .args(["-Q", &p.name])
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).into_owned();
                if let Some(ver) = s.split_whitespace().nth(1) {
                    if !ver.is_empty() {
                        p.version = ver.to_string();
                    }
                }
            }
        }
    }
}

pub(crate) fn print_emerge_completed(pkgs: &[PkgInfo]) {
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

// ── ABS (Arch Build System) support ──────────────────────────────────────────

/// pkgver from ABS GitLab .SRCINFO (no clone).
pub(crate) fn abs_get_version(pkg: &str) -> String {
    // Try to fetch .SRCINFO from GitLab raw API
    let url = format!("{}/{}/raw/HEAD/.SRCINFO", ABS_GITLAB_BASE, pkg);
    if let Some(text) = crate::http::get(&url, 5) {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("pkgver = ") {
                return line["pkgver = ".len()..].trim().to_string();
            }
        }
    }
    "?".to_string()
}

/// validpgpkeys=(...) IDs, uppercased.
pub(crate) fn parse_validpgpkeys(pkgbuild_text: &str) -> Vec<String> {
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

/// Key present in local keyring?
pub(crate) fn gpg_key_present(key: &str) -> bool {
    Command::new(GPG_BIN)
        .args(["--list-keys", key])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ensure validpgpkeys in keyring; --autopgp imports, else print recv-keys.
pub(crate) fn ensure_pgp_keys(pkgbuild_path: &std::path::Path, autopgp: bool) {
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


/// Build isolation: bwrap preferred; None if missing or --no-sandbox.
/// (pkgctl build was dropped; pkgctl only used for repo clone.)
#[derive(Clone, Copy, PartialEq)]
enum BuildIsolation {
    Bwrap,
    None,
}

fn choose_build_isolation(no_sandbox: bool) -> BuildIsolation {
    if no_sandbox {
        return BuildIsolation::None;
    }
    if crate::sandbox::bwrap_available() {
        return BuildIsolation::Bwrap;
    }
    eprintln!(
        "{} bubblewrap (bwrap) not found -- building without isolation.",
        ">>> Warning:".yellow().bold()
    );
    eprintln!(
        "{} install bubblewrap (emerge bubblewrap), or pass --no-sandbox to silence this warning.",
        ">>> Hint:".yellow().bold()
    );
    BuildIsolation::None
}

/// Satisfiable without AUR (synced repo or already installed).
/// Not the same as already_satisfied (provides-aware).
fn is_satisfiable_without_aur(name: &str) -> bool {
    let bare = name.split(['<', '>', '=']).next().unwrap_or(name);
    let synced = Command::new(PACMAN_BIN).args(["-Si", bare]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if synced { return true; }
    Command::new(PACMAN_BIN).args(["-Qi", bare]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// pacman -T: dep already satisfied (exact or provides).
fn already_satisfied(name: &str) -> bool {
    Command::new(PACMAN_BIN).args(["-T", name]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Recursively build AUR pkg + unsatisfiable .SRCINFO deps.
/// `building` = cycle guard; `built` = shared-dep cache. None on hard fail.
/// edit only when is_top_level; skip_srcinfo_regen only with edit.

/// Regenerates `<dir>/.SRCINFO` from a just-edited PKGBUILD via
/// `makepkg --printsrcinfo`, unless `skip_srcinfo_regen` is set.
///
/// The one place aura-emerge does re-execute PKGBUILD content after an
/// edit -- `--printsrcinfo` sources the whole file, the risk
/// `bash_ast.rs`'s and `srcinfo_dependencies`'s (aur.rs) doc comments
/// describe for parsing an arbitrary PKGBUILD. The difference here is
/// trust: this only runs against content the person just wrote in
/// their own `$EDITOR`, after `verify_local_clone_or_rescan` already
/// re-scanned and passed it. `skip_srcinfo_regen` exists for a manual
/// review-then-regenerate workflow instead.
///
/// Best-effort, never fatal: a failure is a warning, and the build
/// continues against the `.SRCINFO` already on disk -- a stale
/// dependency list is recoverable, aborting the whole build over a
/// `--printsrcinfo` hiccup would not be.
/// Best-effort defensive reset after handing the terminal to `$EDITOR`.
/// Some editors (nvim with a true-color theme, especially on
/// kitty/wezterm/foot) set the terminal's default fg/bg/cursor via OSC
/// 10/11/12 and don't always restore them, making our own correct
/// `.yellow().bold()` codes look colorless afterward. `\x1b[0m` resets
/// SGR state; the OSC resets drop any override back to default. All
/// four are no-ops on an unaffected terminal, safe to call
/// unconditionally.
fn reset_terminal_colors_after_editor() {
    print!("\x1b[0m\x1b]110\x07\x1b]111\x07\x1b]112\x07");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

// ── --pkgbuild-view: show/diff PKGBUILD before building, offer edit ────────

/// Outcome of a `--pkgbuild-view` prompt for one top-level package.
pub(crate) struct PkgbuildViewOutcome {
    /// Whether to continue building this package at all.
    pub(crate) proceed: bool,
    /// Whether the PKGBUILD was actually opened (and possibly modified)
    /// in $EDITOR as part of this step - callers that need to re-verify/
    /// regenerate `.SRCINFO` after an edit only do so when this is true.
    pub(crate) edited: bool,
}

/// Where `--pkgbuild-view` keeps the last-shown copy of each pkgbase's
/// PKGBUILD, purely so the *next* run can show a diff instead of the
/// whole file again. Purely a UX cache, no security role (unlike the AUR
/// scanner's own fetched-vs-clone comparison) - if `$HOME`/
/// `$XDG_CACHE_HOME` can't be resolved, or `pkgbase` doesn't look like a
/// safe filename, callers just fall back to showing the full file every
/// time instead of failing.
fn pkgbuild_view_cache_path(pkgbase: &str) -> Option<std::path::PathBuf> {
    if pkgbase.is_empty() || pkgbase.contains(['/', '\\']) {
        return None;
    }
    let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if xdg.is_empty() {
            std::path::PathBuf::from(std::env::var("HOME").ok()?).join(".cache")
        } else {
            std::path::PathBuf::from(xdg)
        }
    } else {
        std::path::PathBuf::from(std::env::var("HOME").ok()?).join(".cache")
    };
    Some(base.join("aura-emerge/pkgbuild-view").join(format!("{}.PKGBUILD", pkgbase)))
}

/// `--pkgbuild-view`: show the PKGBUILD about to be built for a
/// directly-requested (top-level) package -- a diff against the last-
/// shown copy when cached, the full file otherwise -- and ask for
/// confirmation before the build starts. Declining offers to open it in
/// `$EDITOR` instead of just failing the package outright.
///
/// Only called for a top-level target, same restriction `--edit`
/// already applies -- nobody wants a prompt for every transitive dep.
pub(crate) fn pkgbuild_view_step(pkgbase: &str, dir: &std::path::Path) -> PkgbuildViewOutcome {
    let pkgbuild_path = dir.join("PKGBUILD");
    let Ok(current) = fs::read_to_string(&pkgbuild_path) else {
        eprintln!(
            "{} could not read PKGBUILD for '{}' -- skipping --pkgbuild-view for it.",
            ">>> Warning:".yellow().bold(),
            pkgbase
        );
        return PkgbuildViewOutcome { proceed: true, edited: false };
    };

    let cache_path = pkgbuild_view_cache_path(pkgbase);
    let previous = cache_path.as_ref().and_then(|p| fs::read_to_string(p).ok());

    println!();
    println!("{} PKGBUILD for {}:", ">>>".green().bold(), pkgbase.bold());
    println!();
    match &previous {
        Some(prev) if *prev == current => {
            println!("    ({})", "unchanged since last shown".dimmed());
        }
        Some(prev) => {
    // Best-effort unified diff via system `diff`.
            let tmp_prev = std::env::temp_dir()
                .join(format!("aura-emerge-pkgbuild-view-{}-prev", std::process::id()));
            let mut shown_diff = false;
            if std::path::Path::new("/usr/bin/diff").exists() && fs::write(&tmp_prev, prev).is_ok() {
                let diff_out = Command::new("/usr/bin/diff")
                    .args(["-u", "--label", "PKGBUILD (previous)", "--label", "PKGBUILD (current)"])
                    .arg(&tmp_prev)
                    .arg(&pkgbuild_path)
                    .output();
                let _ = fs::remove_file(&tmp_prev);
                if let Ok(out) = diff_out {
                    if !out.stdout.is_empty() {
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                        shown_diff = true;
                    }
                }
            }
            if !shown_diff {
                println!("{}", current);
            }
        }
        None => println!("{}", current),
    }
    println!();

    // Update cache with what was just shown.
    if let Some(path) = &cache_path {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, &current);
    }

    eprint!("{} Continue with this build? [Y/n] ", ">>>".yellow().bold());
    io::stderr().flush().ok();
    let answer = read_line_raw();
    if answer.trim().is_empty() || answer.trim().eq_ignore_ascii_case("y") {
        return PkgbuildViewOutcome { proceed: true, edited: false };
    }

    eprint!("{} Open it in $EDITOR instead of skipping it? [y/N] ", ">>>".yellow().bold());
    io::stderr().flush().ok();
    let answer2 = read_line_raw();
    if !answer2.trim().eq_ignore_ascii_case("y") {
        eprintln!("{} skipping '{}'.", ">>>".red().bold(), pkgbase);
        return PkgbuildViewOutcome { proceed: false, edited: false };
    }

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "nano".to_string());
    println!("{} Opening {} in {}...", ">>>".green().bold(), "PKGBUILD".bold(), editor.green().bold());
    println!("{} Save and close the editor to continue building.", ">>>".yellow().bold());
    Command::new(&editor).arg(&pkgbuild_path).status().ok();
    reset_terminal_colors_after_editor();

    PkgbuildViewOutcome { proceed: true, edited: true }
}

pub(crate) fn maybe_regen_srcinfo(dir: &std::path::Path, skip_srcinfo_regen: bool) {
    if skip_srcinfo_regen {
        eprintln!(
            "{} {} set -- not regenerating .SRCINFO.",
            ">>> Note:".yellow().bold(),
            "--skip-srcinfo-regen".cyan()
        );
        eprintln!(
            "    dependency resolution below still reflects the {} .SRCINFO.",
            "pre-edit".bold()
        );
        eprintln!("    changed depends/makedepends/checkdepends? Regenerate it yourself first:");
        eprintln!(
            "      {}",
            format!("(cd {} && makepkg --printsrcinfo > .SRCINFO)", dir.display()).cyan()
        );
        return;
    }

    println!("{} Regenerating .SRCINFO from the edited PKGBUILD...", ">>>".green().bold());
    let output = Command::new(MAKEPKG_BIN)
        .arg("--printsrcinfo")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(out) if out.status.success() && !out.stdout.is_empty() => {
            if fs::write(dir.join(".SRCINFO"), &out.stdout).is_ok() {
                println!(
                    "{} .SRCINFO regenerated ({}).",
                    ">>>".green().bold(),
                    dir.join(".SRCINFO").display().to_string().dimmed()
                );
            } else {
                eprintln!(
                    "{} could not write {} -- continuing with the previous .SRCINFO.",
                    ">>> Warning:".yellow().bold(),
                    dir.join(".SRCINFO").display().to_string().dimmed()
                );
                eprintln!("    pass {} to skip this step next time.", "--skip-srcinfo-regen".cyan());
            }
        }
        _ => {
            eprintln!(
                "{} `makepkg --printsrcinfo` failed in {} -- continuing with the previous .SRCINFO, which may not reflect your edit.",
                ">>> Warning:".yellow().bold(),
                dir.display().to_string().dimmed()
            );
            eprintln!("    pass {} to skip this step next time.", "--skip-srcinfo-regen".cyan());
        }
    }
}

fn resolve_and_build_aur(
    pkg: &str,
    build_root: &std::path::Path,
    ask: bool,
    skippgp: bool,
    mark_asdeps: bool,
    edit: bool,
    is_top_level: bool,
    skip_srcinfo_regen: bool,
    isolation: BuildIsolation,
    deep: bool,
    unshare_net_build: bool,
    building: &mut HashSet<String>,
    built: &mut HashMap<String, Vec<String>>,
    pkgbuild_view: bool,
) -> Option<Vec<String>> {
    // Fast-path: skip rebuild if cache already has current source.
    if let Some(cached) = built.get(pkg) {
        return Some(cached.clone());
    }

    let Some((dir, pkgbase)) = crate::aur::clone_or_resolve(pkg, build_root) else {
        eprintln!("{} '{}' not found in the AUR", ">>> Error:".red().bold(), pkg);
        return None;
    };

    if let Some(cached) = built.get(&pkgbase) {
        return Some(cached.clone());
    }
    if !building.insert(pkgbase.clone()) {
        eprintln!("{} circular AUR dependency involving '{}'", ">>> Error:".red().bold(), pkgbase);
        return None;
    }

    let fetched = crate::security::scan_aur_pkgbuilds_or_abort(&[pkgbase.clone()]);
    crate::security::verify_local_clone_or_rescan(&pkgbase, &dir, fetched.get(&pkgbase));

    // --edit: top-level only (not recursive deps).
    if edit && is_top_level {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "nano".to_string());
        let pkgbuild = dir.join("PKGBUILD");
        println!("{} Opening {} in {}...", ">>>".green().bold(), "PKGBUILD".bold(), editor.green().bold());
        println!("{} Save and close the editor to continue building.", ">>>".yellow().bold());
        Command::new(&editor).arg(&pkgbuild).status().ok();
        reset_terminal_colors_after_editor();
        crate::security::verify_local_clone_or_rescan(&pkgbase, &dir, fetched.get(&pkgbase));
        maybe_regen_srcinfo(&dir, skip_srcinfo_regen);
    }

    // --pkgbuild-view: after --edit; top-level only.
    if pkgbuild_view && is_top_level {
        let outcome = pkgbuild_view_step(&pkgbase, &dir);
        if !outcome.proceed {
            building.remove(&pkgbase);
            return None;
        }
        if outcome.edited {
            crate::security::verify_local_clone_or_rescan(&pkgbase, &dir, fetched.get(&pkgbase));
            maybe_regen_srcinfo(&dir, skip_srcinfo_regen);
        }
    }

    let srcinfo_path = dir.join(".SRCINFO");

    // .SRCINFO pkgbase should match the cloned name.
    if let Some(declared) = crate::aur::srcinfo_pkgbase(&srcinfo_path) {
        if declared != pkgbase {
            eprintln!(
                "{} '{}' clone's .SRCINFO declares pkgbase '{}', which doesn't match -- the repo may be stale or the AUR git branch mismatched.",
                ">>> Warning:".yellow().bold(),
                pkgbase,
                declared
            );
        }
    }

    let deps = crate::aur::srcinfo_dependencies(&srcinfo_path, &current_arch()).unwrap_or_else(|| {
        eprintln!(
            "{} '{}' has no readable .SRCINFO -- proceeding without a dependency list (the bwrap/makepkg build will still catch a genuinely missing dependency, just later and less clearly).",
            ">>> Warning:".yellow().bold(),
            pkgbase
        );
        Vec::new()
    });

    let mut aur_dep_tarballs = Vec::new();
    for dep in &deps {
        if already_satisfied(dep) || is_satisfiable_without_aur(dep) {
            continue;
        }
        if !deep {
            // --aur without --aur-deep: error on AUR-only deps.
            eprintln!(
                "{} '{}' depends on '{}', which is only available from the AUR -- re-run with {} to also build AUR-only dependencies.",
                ">>> Error:".red().bold(),
                pkgbase,
                dep,
                "--aur-deep".cyan()
            );
            building.remove(&pkgbase);
            return None;
        }
        match resolve_and_build_aur(dep, build_root, ask, skippgp, true, false, false, skip_srcinfo_regen, isolation, deep, unshare_net_build, building, built, false) {
            Some(mut tars) => aur_dep_tarballs.append(&mut tars),
            None => {
                eprintln!(
                    "{} '{}' depends on '{}', which isn't in the official repos, already installed, or resolvable as an AUR package",
                    ">>> Error:".red().bold(),
                    pkgbase,
                    dep
                );
                building.remove(&pkgbase);
                return None;
            }
        }
    }

    let build_started = std::time::SystemTime::now();
    let result = match isolation {
        BuildIsolation::Bwrap => build_with_sandbox(&dir, &pkgbase, ask, mark_asdeps, skippgp, &aur_dep_tarballs, unshare_net_build).then(|| find_built_packages(&dir, build_started)),
        BuildIsolation::None => {
            if !install_local_tarballs(&aur_dep_tarballs, ask, true) {
                eprintln!("{} failed to install locally-built AUR dependencies for '{}'", ">>> Error:".red().bold(), pkgbase);
                building.remove(&pkgbase);
                return None;
            }
            legacy_makepkg_si(&dir, ask, mark_asdeps, skippgp).then(|| find_built_packages(&dir, build_started))
        }
    };

    building.remove(&pkgbase);
    if let Some(tars) = &result {
        built.insert(pkgbase.clone(), tars.clone());
    }
    result
}

/// Root directory AUR builds happen under, mirroring `abs_build_base()`.
///
/// Lives under the user's cache dir (`$XDG_CACHE_HOME` or
/// `~/.cache/aura-emerge/build/aur`), not the old shared
/// `/var/tmp/aura-emerge-aur` -- `/var/tmp` is sticky/multi-user, and
/// every other bit of persistent state already lives under
/// `~/.cache/aura-emerge`. Falls back to `/var/tmp` only if neither
/// `$XDG_CACHE_HOME` nor `$HOME` is set.
pub(crate) fn aur_build_base() -> std::path::PathBuf {
    build_base_dir("aur")
}

/// See `aur_build_base()`.
pub(crate) fn abs_build_base() -> std::path::PathBuf {
    build_base_dir("abs")
}

fn build_base_dir(name: &str) -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return std::path::PathBuf::from(xdg).join("aura-emerge/build").join(name);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return std::path::PathBuf::from(home).join(".cache/aura-emerge/build").join(name);
        }
    }
    std::path::PathBuf::from(format!("/var/tmp/aura-emerge-{}", name))
}

/// Wipes an AUR_BUILD_BASE/ABS_BUILD_BASE-style tree. Falls back to
/// `sudo rm -rf` if plain removal fails.
///
/// Why a plain `remove_dir_all` can fail on a user-owned tree: `package()`
/// runs for real even inside fakeroot -- fakeroot fakes ownership
/// reporting, not `chmod`. A restrictive mode (`install -d -m 700 ...`)
/// leaves a real non-writable entry behind, and removing it needs write
/// permission on its parent -- so even the owning user can hit
/// `Permission denied`. `sudo` (used elsewhere for this class of
/// trusted operation) is simpler than poking at permission bits first.
fn clear_build_base(dir: &std::path::Path) -> std::io::Result<()> {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        let dir_s = dir.to_string_lossy().to_string();
        let ok = Command::new(SUDO_BIN)
            .args([RM_BIN, "-rf", "--"])
            .arg(&dir_s)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Err(e);
        }
    }
    Ok(())
}

/// Install packages from the AUR by cloning their git repos directly and
/// building through the same isolation ladder as `--abs`
/// (`bwrap` -> unsandboxed `makepkg -si`, see `choose_build_isolation`)
/// instead of shelling out to `aura -A`. `pkgctl` is not involved here
/// at all -- only `--abs` ever calls it, and only for `repo clone`.
pub(crate) fn aur_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, edit: bool, no_sandbox: bool, skip_srcinfo_regen: bool, deep: bool, unshare_net_build: bool, pkgbuild_view: bool) -> bool {
    if !std::path::Path::new("/usr/bin/git").exists() {
        eprintln!("{} required binary not found: /usr/bin/git", ">>> Fatal:".red().bold());
        return false;
    }

    let bare: Vec<String> = pkgs.iter().map(|p| p.split('/').last().unwrap_or(p).to_string()).collect();
    if bare.is_empty() { return false; }

    if pretend {
        println!();
        println!("{}", "These are the packages that would be merged, in order:".green().bold());
        println!();
        for p in &bare {
            println!("[{}] {} (AUR)", "aur".green(), p.green().bold());
        }
        println!();
        println!("{}: {} package(s)", "Total".bold(), bare.len());
        println!();
        return true;
    }

    // Refresh sync DBs before dep resolve.
    println!("{} Synchronizing package databases...", ">>>".green().bold());
    if !Command::new(SUDO_BIN).args([PACMAN_BIN, "-Sy"]).status().map(|s| s.success()).unwrap_or(false) {
        eprintln!("{} failed to synchronize pacman databases", ">>> Error:".red().bold());
        return false;
    }

    // Pre-scan requested names; per-pkgbase scan still runs later.
    crate::security::scan_aur_pkgbuilds_or_abort(&bare);

    let isolation = choose_build_isolation(no_sandbox);

    // Wipe build root each run (VCS cache lives under SRCDEST).
    let build_base = aur_build_base();
    if build_base.exists() {
        // Fail loud on wipe -- stale clones break the next run.
        if let Err(e) = clear_build_base(&build_base) {
            eprintln!(
                "{} could not clear stale build directory {}: {}",
                ">>> Fatal:".red().bold(),
                build_base.display(),
                e
            );
            eprintln!("    sudo rm -rf failed too -- check what's holding onto it, e.g.:");
            eprintln!("      {}", format!("sudo lsof +D {}", build_base.display()).cyan());
            return false;
        }
    }
    if std::fs::create_dir_all(&build_base).is_err() {
        eprintln!("{} could not create build directory {}", ">>> Fatal:".red().bold(), build_base.display());
        return false;
    }

    let mut building = HashSet::new();
    let mut built = HashMap::new();
    let mut all_ok = true;

    for (i, pkg) in bare.iter().enumerate() {
        println!(
            "{} Emerging ({} of {}) {} (AUR)",
            ">>>".green().bold(),
            (i + 1).to_string().yellow().bold(),
            bare.len().to_string().yellow().bold(),
            pkg.green().bold()
        );
        println!();
        if resolve_and_build_aur(pkg, &build_base, ask, skippgp, oneshot, edit, true, skip_srcinfo_regen, isolation, deep, unshare_net_build, &mut building, &mut built, pkgbuild_view).is_none() {
            all_ok = false;
        }
    }

    all_ok
}

/// Upgrades every foreign (AUR-or-local) installed package newer in the
/// AUR than what's installed -- replaces `aura -Au`. "Foreign" means
/// `pacman -Qm` (installed but not in a synced repo); an ABS-installed
/// package shows up here too and is silently skipped once the AUR RPC
/// doesn't know its name (no separate ABS-upgrade path yet).
///
/// `pretend` prints the would-upgrade list without touching anything
/// (mirrors `aura -Au --dryrun`). Real runs delegate to `aur_install()`
/// -- the same bwrap-sandboxed path as a fresh AUR install.
// ── --devel / --check-devel: upstream drift check for -git/-hg/-svn/-bzr ───

/// Whether `name` looks like an Arch "devel package" by naming
/// convention. Only `-git` sources are actually parsed (see
/// `extract_git_source_url`) -- `-hg`/`-svn`/`-bzr` are recognized so
/// they show as "couldn't determine" rather than invisible, but aren't
/// checked yet.
fn is_devel_pkg(name: &str) -> bool {
    ["-git", "-hg", "-svn", "-bzr"].iter().any(|suf| name.ends_with(suf))
}

/// Small best-effort extraction of the first `git+` VCS source URL from
/// a PKGBUILD's `source=()` array text. Not full bash parsing (only the
/// AST pass in bash_ast.rs does that) -- a miss just skips the devel
/// check for that package, so a wrong or partial parse costs nothing.
///
/// Returns `(url, branch)`; `branch` is `Some` only when `#branch=...`
/// was present (otherwise `git ls-remote` checks the default branch).
fn extract_git_source_url(pkgbuild_src: &str) -> Option<(String, Option<String>)> {
    let idx = pkgbuild_src.find("git+")?;
    let rest = &pkgbuild_src[idx + "git+".len()..];
    let end = rest.find(['\'', '"', ' ', '\n', '\t']).unwrap_or(rest.len());
    let raw = &rest[..end];
    let (url_part, frag) = match raw.split_once('#') {
        Some((u, f)) => (u, Some(f)),
        None => (raw, None),
    };
    if url_part.is_empty() {
        return None;
    }
    let branch = frag.and_then(|f| {
        f.split('&').find_map(|kv| kv.strip_prefix("branch=")).map(str::to_string)
    });
    Some((url_part.to_string(), branch))
}

/// `git ls-remote <url> [branch|HEAD]`, no local clone involved at all -
/// just asks the remote what its current commit is. Returns the commit
/// hash from the first line of output, or `None` on any failure
/// (network, bad URL, private repo, `git` missing, ...).
fn git_ls_remote_head(url: &str, branch: Option<&str>) -> Option<String> {
    let refname = branch.unwrap_or("HEAD");
    let out = Command::new("git")
        .args(["ls-remote", url, refname])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next()?.split_whitespace().next().map(str::to_string)
}

/// Where `--devel`/`--check-devel` remember the last upstream commit
/// hash seen for each devel package, so a *second* run can tell "moved"
/// from "first time we've ever looked". `name<TAB>hash` per line, same
/// spirit as `news.rs`'s read-state file.
fn devel_state_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::Path::new(&home).join(".cache/aura-emerge/devel.state"))
}

fn load_devel_state() -> HashMap<String, String> {
    let Some(path) = devel_state_path() else { return Default::default() };
    let Ok(text) = fs::read_to_string(path) else { return Default::default() };
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(name, hash)| (name.trim().to_string(), hash.trim().to_string()))
        .collect()
}

fn save_devel_state(state: &HashMap<String, String>) {
    let Some(path) = devel_state_path() else {
        eprintln!(">>> Warning: could not determine $HOME, devel state not saved");
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            eprintln!(">>> Warning: could not create {}, devel state not saved", parent.display());
            return;
        }
    }
    let tmp = path.with_extension("tmp");
    let write_result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        for (name, hash) in state {
            writeln!(f, "{}\t{}", name, hash)?;
        }
        Ok(())
    })();
    if write_result.is_err() || fs::rename(&tmp, &path).is_err() {
        eprintln!(">>> Warning: failed to save devel state");
    }
}

enum DevelStatus {
    /// Upstream HEAD differs from the hash recorded last time this
    /// package was checked.
    Moved,
    Unchanged,
    /// First time this package has ever been checked (no prior
    /// baseline) - the current hash gets recorded either way, but
    /// there's nothing to have "moved" relative to yet, so this is
    /// never treated as an upgrade candidate on its own.
    FirstSeen,
    /// Couldn't fetch the PKGBUILD, couldn't find a `git+` source in
    /// it, or `git ls-remote` itself failed. Never treated as "moved" -
    /// "can't tell" is not the same as "out of date".
    Unknown,
}

/// Check one devel package's upstream against the last-recorded hash in
/// `state` (mutated in place with whatever hash was just seen, so the
/// caller only needs to `save_devel_state` once after a whole batch).
fn check_devel_pkg(pkg: &str, state: &mut HashMap<String, String>) -> DevelStatus {
    let Some(pkgbuild_src) = crate::security::fetch_aur_pkgbuild(pkg) else {
        return DevelStatus::Unknown;
    };
    let Some((url, branch)) = extract_git_source_url(&pkgbuild_src) else {
        return DevelStatus::Unknown;
    };
    let Some(hash) = git_ls_remote_head(&url, branch.as_deref()) else {
        return DevelStatus::Unknown;
    };
    let status = match state.get(pkg) {
        Some(prev) if *prev == hash => DevelStatus::Unchanged,
        Some(_) => DevelStatus::Moved,
        None => DevelStatus::FirstSeen,
    };
    state.insert(pkg.to_string(), hash);
    status
}

/// `--check-devel`: report which installed devel packages have upstream
/// commits beyond what was last recorded, without building or installing
/// anything. See `-u --devel` (the `devel` block inside
/// `aur_upgrade_all`) for the version that folds this into a real
/// upgrade run.
pub(crate) fn check_devel_all() -> bool {
    let foreign = match Command::new(PACMAN_BIN)
        .args(["-Qm"])
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => {
            eprintln!("{} failed to list foreign (AUR/local) packages (pacman -Qm)", ">>> Error:".red().bold());
            return false;
        }
    };

    let devel_names: Vec<String> = foreign
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| is_devel_pkg(n))
        .map(str::to_string)
        .collect();

    if devel_names.is_empty() {
        println!(">>> No installed -git/-hg/-svn/-bzr packages found.");
        return true;
    }

    println!("{} Checking upstream for {} devel package(s)...", ">>>".green().bold(), devel_names.len());
    let mut state = load_devel_state();
    let mut moved: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for name in &devel_names {
        match check_devel_pkg(name, &mut state) {
            DevelStatus::Moved => moved.push(name.clone()),
            DevelStatus::Unchanged | DevelStatus::FirstSeen => {}
            DevelStatus::Unknown => unknown.push(name.clone()),
        }
    }
    save_devel_state(&state);

    println!();
    if moved.is_empty() {
        println!("{} No devel package(s) with upstream changes.", ">>>".green().bold());
    } else {
        println!("{} {} devel package(s) with upstream changes:", ">>>".yellow().bold(), moved.len());
        for n in &moved {
            println!("  [{} {:<4}] {}", "ebuild".green(), "U".yellow().bold(), n.yellow().bold());
        }
        println!();
        println!(">>> Use `emerge -u --devel` to rebuild these along with the normal upgrade.");
    }
    if !unknown.is_empty() {
        println!();
        println!(
            "{} could not determine upstream status for (missing git+ source, fetch failed, or non-git VCS): {}",
            "Note:".dimmed(),
            unknown.join(", ")
        );
    }
    true
}

pub(crate) fn aur_upgrade_all(pretend: bool, ask: bool, skippgp: bool, no_sandbox: bool, skip_srcinfo_regen: bool, deep: bool, unshare_net_build: bool, devel: bool) -> bool {
    let foreign = match Command::new(PACMAN_BIN)
        .args(["-Qm"])
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => {
            eprintln!("{} failed to list foreign (AUR/local) packages (pacman -Qm)", ">>> Error:".red().bold());
            return false;
        }
    };

    let installed: Vec<(String, String)> = foreign
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let name = parts.next()?.to_string();
            let ver = parts.next()?.to_string();
            Some((name, ver))
        })
        .collect();

    if installed.is_empty() {
        println!(">>> No foreign (AUR/local) packages installed - nothing to upgrade.");
        return true;
    }

    let names: Vec<String> = installed.iter().map(|(n, _)| n.clone()).collect();
    let latest = crate::aur::rpc_info(&names);
    let latest_by_name: HashMap<&str, &crate::aur::AurPkgInfo> =
        latest.iter().map(|i| (i.name.as_str(), i)).collect();

    let mut to_upgrade: Vec<(String, String, String)> = Vec::new(); // (name, old, new)
    let mut not_in_aur: Vec<String> = Vec::new();
    for (name, installed_ver) in &installed {
        match latest_by_name.get(name.as_str()) {
            Some(info) => {
                let cmp: i32 = Command::new(VERCMP_BIN)
                    .args([installed_ver.as_str(), info.version.as_str()])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0))
                    .unwrap_or(0);
                if cmp < 0 {
                    to_upgrade.push((name.clone(), installed_ver.clone(), info.version.clone()));
                }
            }
            None => not_in_aur.push(name.clone()),
        }
    }

    if !not_in_aur.is_empty() {
        // Expected/routine for anything installed via --abs (ABS builds
        // aren't in the AUR at all) - a quiet note, not a warning.
        println!(
            ">>> {} foreign package(s) not found in the AUR (likely --abs-built) - skipped: {}",
            not_in_aur.len(),
            not_in_aur.join(", ")
        );
    }

    // --devel: also catch upstream drift vercmp misses.
    if devel {
        let already: std::collections::HashSet<&str> =
            to_upgrade.iter().map(|(n, _, _)| n.as_str()).collect();
        let candidates: Vec<&String> = installed
            .iter()
            .map(|(n, _)| n)
            .filter(|n| is_devel_pkg(n) && !already.contains(n.as_str()))
            .collect();
        if !candidates.is_empty() {
            println!(">>> Checking upstream for {} devel package(s)...", candidates.len());
            let mut state = load_devel_state();
            for name in candidates {
                if matches!(check_devel_pkg(name, &mut state), DevelStatus::Moved) {
                    let old = installed
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    to_upgrade.push((name.clone(), old, "devel (upstream moved)".to_string()));
                }
            }
            save_devel_state(&state);
        }
    }

    if to_upgrade.is_empty() {
        println!(">>> No AUR packages out of date.");
        return true;
    }

    println!();
    for (name, old, new) in &to_upgrade {
        println!("[{} {:<4}] {} [{} -> {}]", "ebuild".green(), "U".yellow().bold(), name.yellow().bold(), old, new);
    }
    println!();
    println!("{}: {} AUR package(s) to upgrade", "Total".bold(), to_upgrade.len());
    println!();

    if pretend {
        return true;
    }

    let upgrade_names: Vec<String> = to_upgrade.iter().map(|(n, _, _)| n.clone()).collect();
    aur_install(&upgrade_names, false, ask, false, skippgp, false, no_sandbox, skip_srcinfo_regen, deep, unshare_net_build, false)
}

/// Installs already-built `*.pkg.tar.*` files directly via `pacman -U`
/// -- used for AUR-only dependencies that a recursive
/// `resolve_and_build_aur()` call already produced and that therefore
/// can never be reached by `pacman -S` (they're not in any sync repo).
/// This is bwrap's equivalent of what `pkgctl build -I` used to inject
/// into its chroot. No-op success on an empty slice.
fn install_local_tarballs(tarballs: &[String], ask: bool, mark_asdeps: bool) -> bool {
    if tarballs.is_empty() {
        return true;
    }
    println!("{} Installing {} locally-built AUR dependency(ies)...", ">>>".green().bold(), tarballs.len());
    let mut args: Vec<&str> = vec![PACMAN_BIN, "-U", "--needed"];
    if mark_asdeps { args.push("--asdeps"); }
    if !ask { args.push("--noconfirm"); }
    let tar_refs: Vec<&str> = tarballs.iter().map(String::as_str).collect();
    Command::new(SUDO_BIN)
        .args(&args)
        .args(&tar_refs)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Runs the untrusted PKGBUILD-defined functions (`pkgver`/`prepare`/
/// `build`/`check`/`package`) for the package at `build_dir` through
/// the bwrap sandbox in `sandbox.rs`, then installs the result the
/// normal, trusted way. See that module's doc for why the three steps
/// below are split this way.
///
/// `aur_dep_tarballs` are already-built AUR-only dependencies, installed
/// via `pacman -U` before anything else since `pacman -S` can't find
/// them. Pass `&[]` for a plain ABS build with no local AUR deps.
///
/// `unshare_net_build`: when set, step 2 (the sandboxed build) splits
/// into two bwrap invocations instead of one -- see the comment before
/// step 2.
///
/// Returns `false` on failure. Dependency resolution prefers `.SRCINFO`
/// when present; only when that's absent and the PKGBUILD's dependency
/// arrays can't be statically resolved either does this fall back to
/// plain unsandboxed `makepkg -si` for this one package rather than
/// guessing at a partial dependency list.
fn build_with_sandbox(build_dir: &std::path::Path, pkgbase: &str, ask: bool, oneshot: bool, skippgp: bool, aur_dep_tarballs: &[String], unshare_net_build: bool) -> bool {
    let pkgbuild_src = match fs::read_to_string(build_dir.join("PKGBUILD")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} could not read PKGBUILD for '{}': {}", ">>> Error:".red().bold(), pkgbase, e);
            return false;
        }
    };

    // 1a. Classify deps (official vs AUR) before installing any AUR dep.
    let arch = current_arch();
    let srcinfo_path = build_dir.join(".SRCINFO");
    let all_deps: Option<Vec<String>> = if srcinfo_path.exists() {
        crate::aur::srcinfo_dependencies(&srcinfo_path, &arch)
    } else {
        crate::bash_ast::pkgbuild_dependencies(&pkgbuild_src, &arch)
    };
    let repo_deps: Option<Vec<String>> = all_deps.map(|deps| {
        // Skip if already provides-satisfied (e.g. zlib-ng-compat).
        deps.into_iter()
            .filter(|d| !already_satisfied(d))
            .filter(|d| is_satisfiable_without_aur(d))
            .collect()
    });

    // 0. Locally-built AUR-only dependencies -- see doc comment above.
    if !install_local_tarballs(aur_dep_tarballs, ask, true) {
        eprintln!("{} failed to install locally-built AUR dependencies for '{}'", ">>> Error:".red().bold(), pkgbase);
        return false;
    }

    // 1b. Official deps via pacman -S (outside sandbox).
    match repo_deps {
        Some(deps) if !deps.is_empty() => {
            println!("{} Installing {} declared dependency(ies) via pacman...", ">>>".green().bold(), deps.len());
            let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed", "--asdeps"];
            if !ask { args.push("--noconfirm"); }
            let dep_refs: Vec<&str> = deps.iter().map(String::as_str).collect();
            let ok = Command::new(SUDO_BIN)
                .args(&args)
                .args(&dep_refs)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                eprintln!("{} failed to install dependencies for '{}'", ">>> Error:".red().bold(), pkgbase);
                return false;
            }
        }
        Some(_) => {} // no declared dependencies, nothing to do
        None => {
            eprintln!(
                "{} could not statically resolve every dependency for '{}' (no .SRCINFO, and the PKGBUILD has a dynamic array entry or depends/makedepends isn't a plain array) -- building without the sandbox for this package.",
                ">>> Warning:".yellow().bold(),
                pkgbase
            );
            if unshare_net_build {
                eprintln!(
                    "{} this fallback runs plain `makepkg -si` with no bwrap sandbox at all, so --unshare-net-build has no effect here -- '{}' builds with full network access this time.",
                    ">>> Warning:".yellow().bold(),
                    pkgbase
                );
            }
            return legacy_makepkg_si(build_dir, ask, oneshot, skippgp);
        }
    }

    // 2. Untrusted build steps inside bwrap (no -s/-i).
    let mut makepkg_args: Vec<&str> = Vec::new();
    if !ask { makepkg_args.push("--noconfirm"); }
    if skippgp { makepkg_args.push("--skippgpcheck"); }

    let real_gnupg = std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".gnupg"));
    let (extra_dest_dirs, default_source_cache) = resolve_dest_dirs(build_dir);
    let user_configured: Vec<&str> = extra_dest_dirs
        .iter()
        .filter(|(name, _)| !(*name == "SRCDEST" && default_source_cache.is_some()))
        .map(|(v, _)| *v)
        .collect();
    if !user_configured.is_empty() {
        println!(
            "{} makepkg.conf sets {} outside the build directory -- binding {} into the sandbox so this build can still write there.",
            ">>>".green().bold(),
            user_configured.join(", "),
            if user_configured.len() == 1 { "it" } else { "them" }
        );
    }
    if let Some(cache) = &default_source_cache {
        println!(
            "{} caching sources under {} -- VCS sources (-git/-hg/-svn) fetch incrementally on rebuild instead of re-cloning; pass SRCDEST in makepkg.conf to change this.",
            ">>>".green().bold(),
            cache.display()
        );
    }

    let build_started = std::time::SystemTime::now();
    let build_ok = if unshare_net_build {
        println!(
            "{} --unshare-net-build set -- fetching/extracting sources with network access, then building '{}' with none.",
            ">>>".green().bold(),
            pkgbase
        );
        let mut nobuild_args = makepkg_args.clone();
        nobuild_args.push("--nobuild");
        let fetch_ok = crate::sandbox::sandboxed_makepkg(MAKEPKG_BIN, build_dir, &nobuild_args, real_gnupg.as_deref(), &extra_dest_dirs, true)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !fetch_ok {
            eprintln!("{} fetching/extracting sources failed for '{}'", ">>> Error:".red().bold(), pkgbase);
            return false;
        }
        let mut noextract_args = makepkg_args.clone();
        noextract_args.push("--noextract");
        crate::sandbox::sandboxed_makepkg(MAKEPKG_BIN, build_dir, &noextract_args, real_gnupg.as_deref(), &extra_dest_dirs, false)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        crate::sandbox::sandboxed_makepkg(MAKEPKG_BIN, build_dir, &makepkg_args, real_gnupg.as_deref(), &extra_dest_dirs, true)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !build_ok {
        eprintln!("{} sandboxed build failed for '{}'", ">>> Error:".red().bold(), pkgbase);
        if unshare_net_build {
            eprintln!(
                "    if this failed reaching for the network during build() -- check for an unvendored dependency fetch (cargo/go/pip/npm resolving its graph mid-build instead of in prepare()) and re-run without --unshare-net-build if that's expected for this package."
            );
        }
        return false;
    }

    // 3. Install the built package(s), outside the sandbox: another
    //    plain pacman operation, no PKGBUILD code involved.
    let pkg_files = find_built_packages(build_dir, build_started);
    if pkg_files.is_empty() {
        eprintln!("{} sandboxed build for '{}' produced no package file", ">>> Error:".red().bold(), pkgbase);
        return false;
    }
    let mut args: Vec<&str> = vec![PACMAN_BIN, "-U"];
    if !ask { args.push("--noconfirm"); }
    if oneshot { args.push("--asdeps"); }
    let pkg_refs: Vec<&str> = pkg_files.iter().map(String::as_str).collect();
    Command::new(SUDO_BIN)
        .args(&args)
        .args(&pkg_refs)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Every `*.pkg.tar.*` file this build actually produced.
///
/// Searches `PKGDEST` (resolved the same way it's bound into the
/// sandbox) when the user has one configured outside `build_dir` --
/// that's where makepkg actually writes it, so searching `build_dir`
/// alone used to find nothing and report "produced no package file"
/// even on a successful build. Falls back to `build_dir` itself,
/// makepkg's default when `PKGDEST` isn't set.
///
/// Unlike `build_dir` (freshly cloned each run, so nothing stale sits
/// there), a configured `PKGDEST` is reused across builds and likely
/// already has unrelated files in it. `not_before` (the caller's
/// timestamp for just before this build started, with slack for coarse
/// mtime resolution) filters those out.
fn find_built_packages(build_dir: &std::path::Path, not_before: std::time::SystemTime) -> Vec<String> {
    let search_dir = extra_makepkg_dest_dirs(build_dir)
        .into_iter()
        .find(|(name, _)| *name == "PKGDEST")
        .map(|(_, path)| path)
        .unwrap_or_else(|| build_dir.to_path_buf());
    let cutoff = not_before
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(not_before);

    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&search_dir) else { return out };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.contains(".pkg.tar") {
            continue;
        }
        match e.metadata().and_then(|m| m.modified()) {
            Ok(mtime) if mtime >= cutoff => {}
    // Stale or unreadable mtime -- don't guess.
            _ => continue,
        }
        out.push(path.to_string_lossy().to_string());
    }
    out
}

/// The original, unsandboxed `makepkg -si` path -- used when `--no-sandbox`
/// is passed, when bwrap isn't installed, or as the fallback for a
/// PKGBUILD whose dependencies couldn't be statically resolved.
fn legacy_makepkg_si(build_dir: &std::path::Path, ask: bool, oneshot: bool, skippgp: bool) -> bool {
    let mut makepkg_args = vec!["-si"];
    if !ask    { makepkg_args.push("--noconfirm"); }
    if oneshot { makepkg_args.push("--asdeps"); }
    if skippgp { makepkg_args.push("--skippgpcheck"); }

    let mut cmd = Command::new(MAKEPKG_BIN);
    cmd.args(&makepkg_args).current_dir(build_dir);

    // Unsandboxed path: real $HOME, plain makepkg.
    for (var, path) in resolve_dest_dirs(build_dir).0 {
        let _ = fs::create_dir_all(&path);
        cmd.env(var, &path);
    }

    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Build and install packages from ABS via `pkgctl repo clone` + `makepkg -si`
/// (or, by default, the bwrap-sandboxed equivalent -- see `build_with_sandbox`).
pub(crate) fn abs_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, edit: bool, autopgp: bool, no_sandbox: bool, skip_srcinfo_regen: bool, unshare_net_build: bool, pkgbuild_view: bool) -> bool {
    for bin in &[PKGCTL_BIN, MAKEPKG_BIN] {
        if !std::path::Path::new(bin).exists() {
            eprintln!(">>> Fatal: required binary not found: {}", bin);
            if *bin == PKGCTL_BIN {
                eprintln!(">>> Hint: install devtools with: emerge devtools");
            }
            return false;
        }
    }

    let pkg_infos: Vec<PkgInfo> = pkgs.iter().filter_map(|pkg| {
        let bare = pkg.split('/').last().unwrap_or(pkg);
        if !validate_pkg(bare) || bare.contains('/') {
            eprintln!(">>> Error: invalid package name '{}' - skipping", bare);
            return None;
        }
        let version = abs_get_version(bare);

        let status = pkg_status(bare, &version);
        Some(PkgInfo { name: bare.to_string(), version, repo: "abs".to_string(), status })
    }).collect();

    if pkg_infos.is_empty() { return false; }

    let isolation = choose_build_isolation(no_sandbox);

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

    // Clean stale build dir from interrupted previous run.
    let build_base = abs_build_base();
    if build_base.exists() {
    // Same wipe pattern as aur_build_base() in aur_install.
        if let Err(e) = clear_build_base(&build_base) {
            eprintln!(
                "{} could not clear stale build directory {}: {}",
                ">>> Fatal:".red().bold(),
                build_base.display(),
                e
            );
            eprintln!("    sudo rm -rf failed too -- check what's holding onto it, e.g.:");
            eprintln!("      {}", format!("sudo lsof +D {}", build_base.display()).cyan());
            return false;
        }
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
            eprintln!(">>> Error: suspicious path for '{}' - skipping", info.name);
            all_ok = false;
            continue;
        }

        if pkg_dir.exists() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }
        let _ = std::fs::create_dir_all(&build_base);

        println!("{} Fetching {} from ABS via pkgctl...", ">>>".green().bold(), info.name.green().bold());
        let checkout_ok = Command::new(PKGCTL_BIN)
            .args(["repo", "clone", "--protocol=https", &info.name])
            .current_dir(&build_base)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !checkout_ok {
            eprintln!("{} pkgctl repo clone failed for '{}'", ">>> Error:".red().bold(), info.name);
            eprintln!("{} package may not exist in ABS (it must be a pkgbase, not a split-package output name). Try without --abs or use --aur.", ">>> Note:".yellow().bold());
            all_ok = false;
            continue;
        }

    // pkgctl clones into <build_base>/<pkg>/ with PKGBUILD at top.
        let build_dir = pkg_dir.clone();

    // --edit for ABS (top-level).
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
            reset_terminal_colors_after_editor();
            maybe_regen_srcinfo(&build_dir, skip_srcinfo_regen);
        }

    // --pkgbuild-view for ABS (same as AUR path).
        if pkgbuild_view {
            let outcome = pkgbuild_view_step(&info.name, &build_dir);
            if !outcome.proceed {
                let _ = std::fs::remove_dir_all(&pkg_dir);
                all_ok = false;
                continue;
            }
            if outcome.edited {
                maybe_regen_srcinfo(&build_dir, skip_srcinfo_regen);
            }
        }

    // Check/import validpgpkeys before makepkg.
        if !skippgp {
            ensure_pgp_keys(&build_dir.join("PKGBUILD"), autopgp);
        }

        let build_ok = match isolation {
            BuildIsolation::Bwrap => build_with_sandbox(&build_dir, &info.name, ask, oneshot, skippgp, &[], unshare_net_build),
            BuildIsolation::None => legacy_makepkg_si(&build_dir, ask, oneshot, skippgp),
        };

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

// ── --install-pkgbuild: install an arbitrary local PKGBUILD checkout ──────────

/// `--install-pkgbuild <PATH>`: build and install a local PKGBUILD
/// checkout through the normal emerge pipeline (scanner + bwrap
/// sandbox) instead of a bare, unaudited `makepkg -si` -- same trust
/// model as an AUR clone, just pointed at a directory already on disk.
///
/// Unlike `aur_install`/`abs_install`, no AUR RPC lookup or `pkgctl repo
/// clone`: `path` is trusted to be a real checkout already, and this
/// just runs it through the same scan -> (optional view/edit) ->
/// sandboxed build -> install sequence every other path uses.
///
/// Returns the `.SRCINFO`-declared `pkgname`(s) built on success (for
/// the caller to record in world.set with the "Err/" prefix -- see
/// `world_set::pkg_world_entry`), or `None` on failure.
pub(crate) fn pkgbuild_local_install(
    path: &std::path::Path,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
    pkgbuild_view: bool,
) -> Option<Vec<String>> {
    let pkgbuild_path = path.join("PKGBUILD");
    if !pkgbuild_path.is_file() {
        eprintln!("{} no PKGBUILD found in {}", ">>> Error:".red().bold(), path.display());
        return None;
    }

    // Log label (pkgbase role for local checkouts).
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string());

    println!("{} Scanning {} for suspicious patterns...", ">>>".green().bold(), label.bold());
    // Same scanner as AUR/ABS, pointed at local files.
    crate::security::verify_local_clone_or_rescan(&label, path, None);

    if pkgbuild_view {
        let outcome = pkgbuild_view_step(&label, path);
        if !outcome.proceed {
            return None;
        }
        if outcome.edited {
            crate::security::verify_local_clone_or_rescan(&label, path, None);
        }
    }

    // Generate .SRCINFO if missing (local PKGBUILD often lacks one).
    if !path.join(".SRCINFO").exists() {
        maybe_regen_srcinfo(path, skip_srcinfo_regen);
    }

    if !skippgp {
        ensure_pgp_keys(&pkgbuild_path, false);
    }

    let isolation = choose_build_isolation(no_sandbox);
    let build_ok = match isolation {
        BuildIsolation::Bwrap => build_with_sandbox(path, &label, ask, oneshot, skippgp, &[], unshare_net_build),
        BuildIsolation::None => legacy_makepkg_si(path, ask, oneshot, skippgp),
    };
    if !build_ok {
        eprintln!("{} build failed for {}", ">>> Error:".red().bold(), path.display());
        return None;
    }

    // Prefer .SRCINFO pkgnames (handles split packages).
    Some(
        crate::aur::srcinfo_pkgnames(&path.join(".SRCINFO"))
            .unwrap_or_else(|| vec![label.clone()]),
    )
}

// ── portageq shim ─────────────────────────────────────────────────────────────

/// Called when the binary is invoked as "portageq" (via symlink).
/// Answers fish shell completion queries so emerge.fish doesn't crash.
pub(crate) fn portageq_shim(args: &[String]) {
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
            // args: eroot repo - return any existing dir
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
        // Unknown command - exit silently so fish doesn't crash
        _ => {}
    }
}

// ── --info ──────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub(crate) fn extract_version_token(text: &str) -> Option<String> {
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

/// Trimmed stdout, or None on failure.
pub(crate) fn cmd_stdout(bin: &str, args: &[&str]) -> Option<String> {
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

/// Current arch (uname -m, else compile-time ARCH).
pub(crate) fn current_arch() -> String {
    cmd_stdout(UNAME_BIN, &["-m"]).unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

/// User-configured PKGDEST/SRCDEST/etc. outside build_dir (for sandbox binds).
pub(crate) fn extra_makepkg_dest_dirs(build_dir: &std::path::Path) -> Vec<(&'static str, std::path::PathBuf)> {
    let vars = read_makepkg_vars();
    ["PKGDEST", "SRCDEST", "SRCPKGDEST", "BUILDDIR"]
        .into_iter()
        .filter_map(|name| {
            let raw = vars.get(name)?;
            if raw.is_empty() {
                return None;
            }
            let path = std::path::PathBuf::from(raw);
            if !path.is_absolute() || path.starts_with(build_dir) {
                return None; // relative (resolves inside build_dir's cwd anyway) or already covered
            }
            Some((name, path))
        })
        .collect()
}

/// Default SRCDEST under ~/.cache/aura-emerge/sources (survives build-dir wipes).
pub(crate) fn source_cache_dir() -> Option<std::path::PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("aura-emerge/sources"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".cache/aura-emerge/sources"))
}

/// Dest dirs for sandbox: user config + default source cache if no SRCDEST.
pub(crate) fn resolve_dest_dirs(build_dir: &std::path::Path) -> (Vec<(&'static str, std::path::PathBuf)>, Option<std::path::PathBuf>) {
    let mut dirs = extra_makepkg_dest_dirs(build_dir);
    let mut default_cache = None;
    if !dirs.iter().any(|(name, _)| *name == "SRCDEST") {
        if let Some(cache) = source_cache_dir() {
            dirs.push(("SRCDEST", cache.clone()));
            default_cache = Some(cache);
        }
    }
    (dirs, default_cache)
}

/// makepkg.conf vars (system + user overrides), via bash source.
pub(crate) fn read_makepkg_vars() -> HashMap<String, String> {
    let watched = [
        "CARCH", "CHOST", "CFLAGS", "CXXFLAGS", "LDFLAGS", "RUSTFLAGS",
        "MAKEFLAGS", "OPTIONS", "BUILDENV", "PKGEXT",
        "PKGDEST", "SRCDEST", "SRCPKGDEST", "BUILDDIR",
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

/// Active user makepkg.conf path (makepkg lookup order).
pub(crate) fn user_makepkg_conf_path() -> String {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return format!("{}/pacman/makepkg.conf", xdg);
    }
    if let Ok(home) = std::env::var("HOME") {
        return format!("{}/.config/pacman/makepkg.conf", home);
    }
    String::new()
}

/// pacman.conf repos in file order (Server/Include resolved one level).
pub(crate) fn parse_pacman_repos() -> Vec<(String, Vec<String>)> {
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

pub(crate) fn read_meminfo() -> Option<(u64, u64, u64, u64)> {
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

pub(crate) fn world_set_stats() -> Option<(usize, u64)> {
    let meta = fs::metadata(WORLD_SET_FILE).ok()?;
    let content = fs::read_to_string(WORLD_SET_FILE).ok()?;
    let count = content.lines().filter(|l| !l.trim().is_empty()).count();
    Some((count, meta.len()))
}

/// emerge --info: system/build summary (Gentoo-style).
pub(crate) fn print_system_info() {
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
        "{} {} ({}, linux {}, {})",
        "aura-emerge".bold(),
        env!("CARGO_PKG_VERSION"),
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


// @preserved-rebuild: pacman -Qi deps → pacman -T → offer reinstall.

/// pacman -Qi (all installed), LC_ALL=C.
pub(crate) fn pacman_qi_all() -> Option<String> {
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

/// "glibc>=2.38" → "glibc".
pub(crate) fn strip_version_operator(atom: &str) -> String {
    match atom.find(['<', '>', '=']) {
        Some(i) => atom[..i].to_string(),
        None => atom.to_string(),
    }
}

/// All "Depends On" from pacman -Qi (handles continuation lines).
pub(crate) fn parse_depends_on_all(qi_text: &str) -> HashSet<String> {
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

/// pacman -T: which atoms are currently unsatisfied.
pub(crate) fn missing_via_pacman_t(deps: &[String]) -> Vec<String> {
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

/// @preserved-rebuild: find unsatisfied deps, offer pacman -S --asdeps.
pub(crate) fn preserved_rebuild(pretend: bool, ask: bool) {
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

    // These are all official-repo dependencies (surfaced via `pacman -T`),
    // so this is a plain pacman install -- no AUR resolution involved.
    let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--asdeps"];
    if !ask { args.push("--noconfirm"); }
    if run_cmd(SUDO_BIN, &args, &missing) {
        mark_asdeps(&missing);
    }
}