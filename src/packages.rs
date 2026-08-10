//! Package resolution/probing, ABS builds, the portageq shim, `--info`
//! system info, and the @preserved-rebuild dependency check.
//!
//! Split out of main.rs — these are all "package operation" helpers that
//! main() dispatches into.

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

/// Compute install status: N=new, U=upgrade, D=downgrade, R=reinstall.
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

/// Format package atom for display.
pub(crate) fn format_atom(p: &PkgInfo) -> String {
    if p.repo.is_empty() {
        format!("{}-{}", p.name, p.version)
    } else {
        format!("{}/{}-{}", p.repo, p.name, p.version)
    }
}

/// Colored status badge.
pub(crate) fn status_colored(status: &str) -> String {
    match status {
        "N" => status.green().bold().to_string(),
        "U" => status.yellow().bold().to_string(),
        "D" => status.red().bold().to_string(),
        _   => status.cyan().bold().to_string(),
    }
}

// ── Explicit / dependency flag helpers ──────────────────────────────────────

/// Force the given packages to be flagged as explicitly installed via
/// `pacman -D --asexplicit`. Used after AUR/ABS installs (where the
/// install path — aura/makepkg — doesn't always leave the explicit bit
/// set the way a plain `pacman -S` would) and by `--select`, so world.set
/// membership and pacman's own bookkeeping never disagree.
///
/// Best-effort: silently no-ops for names that aren't actually installed
/// (e.g. `--select` on a package not yet pulled in) — that's an expected,
/// non-error case, not something worth warning about.
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

/// Opposite of mark_asexplicit(): flags the given packages as
/// installed-as-dependency via `pacman -D --asdeps`. Used by `--deselect`
/// so a package dropped from world.set is no longer treated as something
/// the user explicitly wants — it becomes eligible for `--depclean` /
/// `pacman -Qtdq` orphan cleanup like any other transitive dependency.
///
/// Same best-effort semantics as mark_asexplicit().
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

/// Probe official repos in a single call using --print-format.
/// Returns Some(infos) if packages are found, None otherwise.
///
/// `-Sp --print-format` is a plain pacman feature (aura just forwarded it
/// unchanged) -- calling pacman directly here needs no privilege
/// escalation, same as when this went through aura.
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

/// Probe official repos and split packages into found/not-found.
///
/// Batch `-Sp` fails for the whole query if one name doesn't exist, which
/// would wrongly push everything to AUR. So we batch-probe first, then
/// fall back to probing individually anything the batch missed.
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

/// Fetch AUR package info via the official AUR RPC, split into resolved
/// hits and names AUR genuinely doesn't have.
///
/// A single batched `aur::rpc_info()` call replaces what used to be one
/// `aura -Ai <pkg>` subprocess per name -- `aura -Ai` exited successfully
/// with empty stdout for a name AUR doesn't know about (it never failed
/// the command), so the split here works the same way: a name missing
/// from the RPC result set is "not found", not an error. This returns a
/// PkgInfo only for names the RPC actually described; everything else
/// comes back in the second Vec so the caller can drop it from the
/// install args and report it instead of silently failing the whole
/// batch (a batch install of 10 packages where one no longer exists
/// should still install the other 9).
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

/// Re-query the version pacman actually has on disk for each package,
/// in place.
///
/// `installed_infos` is built from `resolve_aur_split`/`probe_official`
/// *before* the real install runs — for AUR targets specifically, that's
/// before `aura -A` does its git pull, so if upstream pushes a new
/// PKGBUILD version between resolution and build (exactly what happens
/// on a fast-moving -git-style package, or just unlucky timing), the
/// version aura-emerge planned against is already stale by the time the
/// build finishes. pacman/aura install whatever the freshly-pulled
/// PKGBUILD says regardless, so the display should catch up to that,
/// not the other way around. Called right after a successful install,
/// before `print_emerge_completed` — best-effort: a `pacman -Q` miss
/// just leaves the planned version in place rather than erroring out.
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

/// Query .SRCINFO from ABS GitLab (raw) to get pkgver without cloning.
pub(crate) fn abs_get_version(pkg: &str) -> String {
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

/// Extract PGP key IDs from a PKGBUILD's `validpgpkeys=(...)` array.
/// Handles multi-line arrays and either quote style; returns uppercased tokens.
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

/// Is this key already in the local GPG keyring?
pub(crate) fn gpg_key_present(key: &str) -> bool {
    Command::new(GPG_BIN)
        .args(["--list-keys", key])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check `validpgpkeys` against the local keyring; with `--autopgp` import
/// missing keys from keyserver.ubuntu.com, otherwise print the exact
/// `gpg --recv-keys` command so the user isn't left guessing. Runs before
/// makepkg so this is caught ahead of a build failure.
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


/// Which isolation mechanism is available for the untrusted part of a
/// build (`pkgver()/prepare()/build()/check()/package()`). `pkgctl
/// build`'s own systemd-nspawn chroot used to be tried first here, but
/// it syncs/bootstraps a whole fresh root filesystem per invocation --
/// a single corrupted mirror download during that bootstrap is enough
/// to fail an otherwise-unrelated single-package build. That cost isn't
/// worth what it buys over `bwrap` (which reuses the real, already-live
/// filesystem read-only instead of rebuilding one), so `bwrap` is now
/// the primary sandbox and `pkgctl` is only ever invoked for `pkgctl
/// repo clone` (see `abs_install`), never `pkgctl build`. `None` (no
/// isolation at all) remains the last-resort fallback when bubblewrap
/// isn't installed, or when `--no-sandbox` is passed explicitly.
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

/// Checks whether a bare package name is satisfiable without touching
/// the AUR at all -- either present in a synced official repo, or
/// already installed locally (covers a previously-built-from-AUR
/// package too, so a shared dependency across runs isn't rebuilt).
/// Used by `resolve_and_build_aur` to decide whether a `.SRCINFO`
/// dependency needs to be recursively resolved as an AUR package.
fn is_satisfiable_without_aur(name: &str) -> bool {
    let bare = name.split(['<', '>', '=']).next().unwrap_or(name);
    let synced = Command::new(PACMAN_BIN).args(["-Si", bare]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if synced { return true; }
    Command::new(PACMAN_BIN).args(["-Qi", bare]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Recursively resolves and builds an AUR package `pkg`, building
/// whatever `.SRCINFO`-declared dependencies of it aren't satisfiable
/// from the official repos or already installed (i.e. other AUR
/// packages) first. `building` guards against dependency cycles;
/// `built` caches already-built pkgbases so a dependency shared by more
/// than one package in the same run isn't built twice. Returns the
/// tarball path(s) built for `pkg` itself (so a *parent* call can
/// install them via `pacman -U` before its own bwrap build -- see
/// `install_local_tarballs`), or `None` on any failure -- a missing
/// dependency is a hard stop, never a guess.
///
/// Each resolved pkgbase is run through the same PKGBUILD scanner
/// (`security::scan_aur_pkgbuilds_or_abort`) used for top-level targets
/// before its build starts -- recursively-pulled-in dependencies get
/// exactly the same scrutiny as something the user typed directly.
fn resolve_and_build_aur(
    pkg: &str,
    build_root: &std::path::Path,
    ask: bool,
    skippgp: bool,
    mark_asdeps: bool,
    isolation: BuildIsolation,
    building: &mut HashSet<String>,
    built: &mut HashMap<String, Vec<String>>,
) -> Option<Vec<String>> {
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

    let srcinfo_path = dir.join(".SRCINFO");

    // Cheap integrity check: the `.SRCINFO` a package's own repo ships
    // should declare itself as the same pkgbase we cloned by. A mismatch
    // doesn't necessarily mean anything malicious (stale/uncommitted
    // `.SRCINFO` is a known AUR foot-gun maintainers hit on their own
    // packages), but it's cheap to catch and worth a heads-up either way.
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
        if is_satisfiable_without_aur(dep) {
            continue;
        }
        match resolve_and_build_aur(dep, build_root, ask, skippgp, true, isolation, building, built) {
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

    let result = match isolation {
        BuildIsolation::Bwrap => build_with_sandbox(&dir, &pkgbase, ask, mark_asdeps, skippgp, &aur_dep_tarballs).then(|| find_built_packages(&dir)),
        BuildIsolation::None => {
            if !install_local_tarballs(&aur_dep_tarballs, ask, true) {
                eprintln!("{} failed to install locally-built AUR dependencies for '{}'", ">>> Error:".red().bold(), pkgbase);
                building.remove(&pkgbase);
                return None;
            }
            legacy_makepkg_si(&dir, ask, mark_asdeps, skippgp).then(|| find_built_packages(&dir))
        }
    };

    building.remove(&pkgbase);
    if let Some(tars) = &result {
        built.insert(pkgbase.clone(), tars.clone());
    }
    result
}

/// Root directory AUR builds happen under, mirroring `ABS_BUILD_BASE`.
pub(crate) const AUR_BUILD_BASE: &str = "/tmp/aura-emerge-aur";

/// Install packages from the AUR by cloning their git repos directly and
/// building through the same isolation ladder as `--abs`
/// (`bwrap` -> unsandboxed `makepkg -si`, see `choose_build_isolation`)
/// instead of shelling out to `aura -A`. `pkgctl` is not involved here
/// at all -- only `--abs` ever calls it, and only for `repo clone`.
pub(crate) fn aur_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, no_sandbox: bool) -> bool {
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

    // Refresh the sync databases before resolving anything below --
    // `pacman -S`/`-Si` (used both for the declared-dependency install in
    // build_with_sandbox and for every is_satisfiable_without_aur() check
    // during AUR-dependency resolution) only ever sees whatever the last
    // `-Sy` left behind; a stale db is the easy way to get a spurious
    // "target not found" for a package that's actually available.
    // `pkgctl build` used to cover this on its own (it synced its own
    // chroot's db every run) -- now that bwrap is the primary sandbox and
    // doesn't have its own separate db, this has to happen explicitly.
    //
    // Deliberately plain `-Sy`, not `-Syu`: this is meant to freshen
    // dependency resolution for the AUR package(s) requested here, not to
    // opportunistically upgrade the whole system as a side effect of
    // installing one AUR package -- that's `--update`'s job. This does
    // still carry the usual `-Sy`-without-`-u` partial-upgrade caveat
    // (a newly-synced dependency pulled in below could need a newer
    // library than what's on the rest of the system), which only a real
    // `-Syu` fully avoids.
    println!("{} Synchronizing package databases...", ">>>".green().bold());
    if !Command::new(SUDO_BIN).args([PACMAN_BIN, "-Sy"]).status().map(|s| s.success()).unwrap_or(false) {
        eprintln!("{} failed to synchronize pacman databases", ">>> Error:".red().bold());
        return false;
    }

    // Pre-check by the requested name(s) before doing any work at all --
    // the authoritative per-pkgbase scan still happens inside
    // resolve_and_build_aur() once the real pkgbase is known (matters
    // for split packages, where the requested name isn't the AUR git
    // branch the scanner needs).
    crate::security::scan_aur_pkgbuilds_or_abort(&bare);

    let isolation = choose_build_isolation(no_sandbox);

    let build_base = std::path::PathBuf::from(AUR_BUILD_BASE);
    if build_base.exists() {
        let _ = std::fs::remove_dir_all(&build_base);
    }
    if std::fs::create_dir_all(&build_base).is_err() {
        eprintln!("{} could not create build directory {}", ">>> Fatal:".red().bold(), AUR_BUILD_BASE);
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
        if resolve_and_build_aur(pkg, &build_base, ask, skippgp, oneshot, isolation, &mut building, &mut built).is_none() {
            all_ok = false;
        }
    }

    all_ok
}

/// Upgrades every foreign (AUR-or-local) installed package that's newer
/// in the AUR than what's installed -- replaces `aura -Au`. "Foreign"
/// here means `pacman -Qm` (installed but not in a synced official repo),
/// which is also what `aura -Au` itself worked from; a package installed
/// from ABS shows up here too and is silently skipped once the AUR RPC
/// doesn't know its name (no separate ABS-upgrade path exists yet).
///
/// `pretend` prints the would-upgrade list and returns without touching
/// anything (mirrors `aura -Au --dryrun`). Real runs delegate the actual
/// build+install to `aur_install()` -- the same bwrap-sandboxed
/// path used for a fresh AUR install, so an upgrade gets exactly the same
/// isolation and PKGBUILD scanning as `emerge --aur <pkg>` does.
pub(crate) fn aur_upgrade_all(pretend: bool, ask: bool, skippgp: bool, no_sandbox: bool) -> bool {
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
        println!(">>> No foreign (AUR/local) packages installed — nothing to upgrade.");
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
        // aren't in the AUR at all) — a quiet note, not a warning.
        println!(
            ">>> {} foreign package(s) not found in the AUR (likely --abs-built) — skipped: {}",
            not_in_aur.len(),
            not_in_aur.join(", ")
        );
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
    aur_install(&upgrade_names, false, ask, false, skippgp, no_sandbox)
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
/// `build`/`check`/`package`) for the package rooted at `build_dir`
/// through the bwrap sandbox in `sandbox.rs`, then installs the result
/// the normal, trusted way. See that module's doc comment for why the
/// three steps below are split the way they are.
///
/// `aur_dep_tarballs` are already-built AUR-only dependencies (from a
/// recursive `resolve_and_build_aur()` call on this pkg's own
/// `.SRCINFO`) -- installed straight from disk via `pacman -U` before
/// anything else, since `pacman -S` below has no way to find them.
/// Pass `&[]` for a plain ABS build with no local AUR dependencies.
///
/// Returns `false` on any failure. Dependency resolution prefers
/// `.SRCINFO` when the checkout has one (see step 1a below); only when
/// that's absent and the PKGBUILD's dependency arrays can't be
/// statically resolved either (dynamic elements, non-literal
/// assignment, ...) does this fall back to the plain unsandboxed
/// `makepkg -si` path for this one package rather than guessing at a
/// partial dependency list.
fn build_with_sandbox(build_dir: &std::path::Path, pkgbase: &str, ask: bool, oneshot: bool, skippgp: bool, aur_dep_tarballs: &[String]) -> bool {
    let pkgbuild_src = match fs::read_to_string(build_dir.join("PKGBUILD")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} could not read PKGBUILD for '{}': {}", ">>> Error:".red().bold(), pkgbase, e);
            return false;
        }
    };

    // 1a. Figure out which of this package's declared dependencies are
    //     reachable via the official repos at all -- computed *before*
    //     step 0 installs anything below, since once an AUR-only dep is
    //     installed from its tarball it would also pass a plain
    //     "is it already installed?" check, and we specifically need to
    //     keep it OUT of the `pacman -S` list in that case: `-S` (even
    //     with `--needed`) still resolves its target against the sync
    //     databases first and fails with "target not found" for a name
    //     that isn't in any of them, already-installed or not.
    //
    //     `.SRCINFO` is tried first when present (always true for an AUR
    //     clone, sometimes true for --abs) -- it's a static, checked-in,
    //     generated file that already flattens arch-specific arrays
    //     correctly, so reading it is both safer (no PKGBUILD parsing
    //     needed at all) and more complete than the AST fallback below.
    //     `pkgbuild_dependencies` (tree-sitter over the actual PKGBUILD)
    //     only kicks in when there's no `.SRCINFO` to read.
    let arch = current_arch();
    let srcinfo_path = build_dir.join(".SRCINFO");
    let all_deps: Option<Vec<String>> = if srcinfo_path.exists() {
        crate::aur::srcinfo_dependencies(&srcinfo_path, &arch)
    } else {
        crate::bash_ast::pkgbuild_dependencies(&pkgbuild_src, &arch)
    };
    let repo_deps: Option<Vec<String>> = all_deps.map(|deps| {
        deps.into_iter().filter(|d| is_satisfiable_without_aur(d)).collect()
    });

    // 0. Locally-built AUR-only dependencies -- see doc comment above.
    if !install_local_tarballs(aur_dep_tarballs, ask, true) {
        eprintln!("{} failed to install locally-built AUR dependencies for '{}'", ">>> Error:".red().bold(), pkgbase);
        return false;
    }

    // 1b. The rest, outside the sandbox: plain `pacman -S`, no
    //     PKGBUILD code involved at all, so this is exactly as
    //     trustworthy as any other pacman install.
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
            return legacy_makepkg_si(build_dir, ask, oneshot, skippgp);
        }
    }

    // 2. The untrusted part, inside bwrap. Deliberately no `-s`
    //    (dependencies are already handled above) and no `-i`
    //    (installation happens separately in step 3) -- see
    //    sandbox.rs's doc comment for why those two specifically stay
    //    outside the jail.
    let mut makepkg_args: Vec<&str> = Vec::new();
    if !ask { makepkg_args.push("--noconfirm"); }
    if skippgp { makepkg_args.push("--skippgpcheck"); }

    let real_gnupg = std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".gnupg"));
    let extra_dest_dirs = extra_makepkg_dest_dirs(build_dir);
    if !extra_dest_dirs.is_empty() {
        let var_names: Vec<&str> = extra_dest_dirs.iter().map(|(v, _)| *v).collect();
        println!(
            "{} makepkg.conf sets {} outside the build directory -- binding {} into the sandbox so this build can still write there.",
            ">>>".green().bold(),
            var_names.join(", "),
            if extra_dest_dirs.len() == 1 { "it" } else { "them" }
        );
    }
    let build_ok = crate::sandbox::sandboxed_makepkg(MAKEPKG_BIN, build_dir, &makepkg_args, real_gnupg.as_deref(), &extra_dest_dirs)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !build_ok {
        eprintln!("{} sandboxed build failed for '{}'", ">>> Error:".red().bold(), pkgbase);
        return false;
    }

    // 3. Install the built package(s), outside the sandbox: another
    //    plain pacman operation, no PKGBUILD code involved.
    let pkg_files = find_built_packages(build_dir);
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

/// Every `*.pkg.tar.*` file sitting directly in `build_dir` after a
/// build -- `build_dir` is a freshly-cloned checkout each run (see the
/// stale-dir cleanup in `abs_install`), so there's nothing stale left
/// over from a previous build to accidentally pick up here.
fn find_built_packages(build_dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(build_dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.contains(".pkg.tar") {
                        out.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
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

    Command::new(MAKEPKG_BIN)
        .args(&makepkg_args)
        .current_dir(build_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build and install packages from ABS via `pkgctl repo clone` + `makepkg -si`
/// (or, by default, the bwrap-sandboxed equivalent -- see `build_with_sandbox`).
pub(crate) fn abs_install(pkgs: &[String], pretend: bool, ask: bool, oneshot: bool, skippgp: bool, edit: bool, autopgp: bool, no_sandbox: bool) -> bool {
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
            eprintln!(">>> Error: invalid package name '{}' — skipping", bare);
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

        // pkgctl clones straight into <build_base>/<pkg>/ with PKGBUILD at
        // the top level — unlike the old asp/svntogit layout, there's no
        // trunk/ subdirectory to descend into.
        let build_dir = pkg_dir.clone();

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

        let build_ok = match isolation {
            BuildIsolation::Bwrap => build_with_sandbox(&build_dir, &info.name, ask, oneshot, skippgp, &[]),
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

// ── --info ──────────────────────────────────────────────────────────────────

/// Pull the first bare `X.Y[.Z]` version token out of a string. No longer
/// called now that `print_system_info()` doesn't shell out to `aura
/// --version`, but kept (small, generically useful for any "banner text
/// + version" stdout) rather than deleted along with its one call site.
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

/// Run a command and return trimmed stdout, or None on any failure.
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

/// This machine's pacman/makepkg arch (`CARCH` in makepkg terms) -- used
/// to pick out `depends_<arch>=(...)`-style arrays alongside the plain
/// `depends=(...)` ones. Falls back to the arch this binary itself was
/// compiled for if `uname -m` can't be run, which is a reasonable
/// default -- aura-emerge only ever ships/runs on the arch it's built for.
pub(crate) fn current_arch() -> String {
    cmd_stdout(UNAME_BIN, &["-m"]).unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

/// makepkg destination-directory overrides (`PKGDEST`/`SRCDEST`/
/// `SRCPKGDEST`/`BUILDDIR`) that resolve to somewhere *outside*
/// `build_dir` -- `sandboxed_makepkg`'s base `--bind build_dir build_dir`
/// only makes `build_dir` itself writable inside the jail, so a
/// configured destination elsewhere (a `~/.makepkg.conf` with e.g.
/// `PKGDEST=$HOME/pkgs`, a common setup) would otherwise fail to write
/// once the build actually finishes -- see `sandbox::sandboxed_makepkg`'s
/// doc comment for how each pair returned here gets bound+exported into
/// the jail.
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

/// Merge /etc/makepkg.conf with user overrides using makepkg's own
/// precedence order. Sourced via bash so arrays/`$(nproc)`/quoting are
/// handled correctly instead of hand-parsed.
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

/// The path to whichever user-level makepkg.conf override would actually
/// take effect, mirroring makepkg's own lookup order.
pub(crate) fn user_makepkg_conf_path() -> String {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return format!("{}/pacman/makepkg.conf", xdg);
    }
    if let Ok(home) = std::env::var("HOME") {
        return format!("{}/.config/pacman/makepkg.conf", home);
    }
    String::new()
}

/// Parse /etc/pacman.conf repo sections in file order (= pacman's own
/// priority order). Keeps Server/Include lines, resolving Include one
/// level deep into the mirrorlist.
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

/// `emerge --info`: a system/build-environment summary in the spirit of
/// Gentoo's `emerge --info`, adapted to aura/pacman (relabeled fields,
/// same overall shape: uname/mem, repos, flags, world set).
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


// ── @preserved-rebuild ──────────────────────────────────────────────────────
//
// No Portage-style preserved-libs tracking on Arch, so this isn't a literal
// port. Instead: collect every dependency of every installed package (via
// `pacman -Qi`), hand the deduplicated list to `pacman -T` (which understands
// provides/virtual packages, unlike a naive string diff), and offer to
// reinstall whatever comes back unsatisfied.

/// Run `pacman -Qi` (info for every installed package) forced to the C
/// locale, since pacman's field labels are gettext-translated and this
/// parser expects English regardless of the user's actual locale.
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

/// Strip a version-comparison operator/suffix off a dependency atom:
/// "glibc>=2.38" -> "glibc", "libc.so=6-64" -> "libc.so", "bash" -> "bash".
pub(crate) fn strip_version_operator(atom: &str) -> String {
    match atom.find(['<', '>', '=']) {
        Some(i) => atom[..i].to_string(),
        None => atom.to_string(),
    }
}

/// Parse every "Depends On" field out of a `pacman -Qi` dump (blocks
/// separated by blank lines), stripping version operators and
/// deduplicating. Handles pacman's indented continuation lines.
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

/// Ask pacman which of these atoms are actually unsatisfied right now.
/// `pacman -T` correctly resolves provides/virtual packages, unlike
/// comparing against `pacman -Qq` by hand.
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

/// `@preserved-rebuild`: scan every installed package's declared
/// dependencies, find the ones that aren't actually satisfied on the
/// system, and offer to reinstall them (`aura -S`, i.e. official repos —
/// a dependency that vanished is essentially always a repo package).
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