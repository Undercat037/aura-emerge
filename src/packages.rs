//! Package resolve/probe, ABS builds, portageq, --info, @preserved-rebuild.

use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::*;

// ── Package info ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PkgInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) repo: String,
    /// "N" new, "U" upgrade, "D" downgrade, "R" reinstall
    pub(crate) status: String,
}

/// Install status: N/U/D/R via libalpm.
pub(crate) fn pkg_status(name: &str, new_ver: &str) -> String {
    crate::alpm_db::pkg_status(name, new_ver)
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
        _ => status.cyan().bold().to_string(),
    }
}

/// Run a build command (makepkg / bwrap+makepkg).
///
/// Quiet by default: stdout/stderr are captured so the Gentoo-style
/// `>>> Emerging` / `>>> Installing` lines stay readable. On failure the
/// captured log is dumped. Live output with `--debug`, `AE_DEBUG=1`, or
/// `--quiet-build=n`. With `--log PATH`, output is always captured into
/// the session file (and still printed live when debug is on).
fn run_build_cmd(mut cmd: Command, label: &str) -> Result<(), String> {
    let logging = crate::logbook::session_active();
    let live = crate::runtime::show_build_output();

    // Live-only path: no session log, just inherit stdio.
    if live && !logging {
        return if cmd.status().map(|s| s.success()).unwrap_or(false) {
            Ok(())
        } else {
            Err(String::new())
        };
    }

    // Capture when quiet, or when a session log needs the full output.
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    match cmd.output() {
        Ok(out) => {
            let combined = {
                let mut s = String::new();
                if !out.stdout.is_empty() {
                    s.push_str(&String::from_utf8_lossy(&out.stdout));
                    if !s.ends_with('\n') {
                        s.push('\n');
                    }
                }
                if !out.stderr.is_empty() {
                    s.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                s
            };
            if logging && !combined.is_empty() {
                crate::logbook::session_write_output(&format!(
                    "--- build: {} ---\n{}",
                    label, combined
                ));
            }
            if out.status.success() {
                // --debug + --log: replay captured output so the terminal
                // still sees the full build while the file gets a copy.
                if live && !combined.is_empty() {
                    print!("{}", combined);
                    if !combined.ends_with('\n') {
                        println!();
                    }
                }
                Ok(())
            } else {
                if !combined.trim().is_empty() {
                    eprintln!(
                        "{} build log for '{}' (re-run with {} or {} for live output):",
                        ">>>".yellow().bold(),
                        label,
                        "--debug".cyan(),
                        "AE_DEBUG=1".cyan()
                    );
                    eprint!("{}", combined);
                    if !combined.ends_with('\n') {
                        eprintln!();
                    }
                }
                Err(combined)
            }
        }
        Err(e) => {
            eprintln!(
                "{} failed to spawn build for '{}': {}",
                ">>> Error:".red().bold(),
                label,
                e
            );
            Err(String::new())
        }
    }
}

// ── Explicit / dependency flag helpers ──────────────────────────────────────

/// libalpm `--asexplicit` (AUR/ABS/--select). Best-effort if not installed.
pub(crate) fn mark_asexplicit(pkgs: &[String]) {
    let bare: Vec<String> = pkgs
        .iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    if bare.is_empty() {
        return;
    }
    let _ = crate::rootops::set_reason(true, &bare);
}

/// libalpm `--asdeps` (--deselect / transitive deps). Best-effort.
pub(crate) fn mark_asdeps(pkgs: &[String]) {
    let bare: Vec<String> = pkgs
        .iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    if bare.is_empty() {
        return;
    }
    let _ = crate::rootops::set_reason(false, &bare);
}

/// make.conf plus any `package.env` layers matching this build dir
/// (`.../build/<aur|abs>/<pkgbase>`; pkgbase and every `.SRCINFO`
/// pkgname are tried against the atoms).
fn build_config(build_dir: &std::path::Path) -> crate::config::Config {
    let repo = build_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());
    let mut names: Vec<String> = Vec::new();
    if let Some(b) = build_dir.file_name().and_then(|n| n.to_str()) {
        names.push(b.to_string());
    }
    if let Some(v) = crate::aur::srcinfo_pkgnames(&build_dir.join(".SRCINFO")) {
        for n in v {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let applied = crate::package_env::applied_for(repo, &names);
    for c in &applied.conflicts {
        eprintln!("{} {}", ">>> Warning:".yellow().bold(), c);
    }
    crate::runtime::config().layered(&applied.vars, &applied.files)
}

/// Materializes make.conf's build flags as a makepkg.conf and returns
/// its path, for `makepkg --config`.
///
/// Why a generated file instead of environment variables: makepkg
/// *sources* makepkg.conf, and a plain `CFLAGS=...` assignment in there
/// overwrites whatever the environment had. So exporting CFLAGS at
/// makepkg would be silently discarded a second later. The generated
/// file sources the system config first and then overrides it, which
/// puts our values last in the only order that counts.
///
/// Written to the sandbox scratch directory rather than the build
/// directory -- see `sandbox::scratch_dir`. `None` when make.conf
/// sets no build flags, which leaves makepkg's own config lookup
/// untouched.
fn makepkg_conf_override(
    build_dir: &std::path::Path,
    cfg: &crate::config::Config,
) -> Option<String> {
    let text = crate::config::makepkg_override_conf(cfg)?;
    let dir = crate::sandbox::scratch_dir(build_dir);
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("makepkg.conf");
    fs::write(&path, text).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// Probe official sync dbs via libalpm. Some(infos) or None if none found.
pub(crate) fn probe_official(pkgs: &[String]) -> Option<Vec<PkgInfo>> {
    let (found, _missing) = crate::alpm_db::probe_sync_split(pkgs);
    if found.is_empty() {
        return None;
    }
    Some(
        found
            .into_iter()
            .map(|p| {
                let status = pkg_status(&p.name, &p.version);
                PkgInfo {
                    name: p.name,
                    version: p.version,
                    repo: p.repo,
                    status,
                }
            })
            .collect(),
    )
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

    // The lookups above only confirm the typed names exist. The plan
    // itself must carry the dependencies too, or `--tree` has nothing
    // to nest and "Calculating dependencies" would be a lie.
    if missing.is_empty() && !found.is_empty() {
        found = expand_with_deps(found);
    }

    (found, missing)
}

/// Asks libalpm for the whole transaction (targets + deps, deps first).
/// On failure keeps the bare targets and says why, so a broken resolve
/// is visible instead of silently looking like "no dependencies".
fn expand_with_deps(targets: Vec<PkgInfo>) -> Vec<PkgInfo> {
    let atoms: Vec<String> = targets
        .iter()
        .map(|p| format!("{}/{}", p.repo, p.name))
        .collect();
    match crate::alpm_db::plan_sync(&atoms) {
        Ok(planned) if !planned.is_empty() => planned
            .into_iter()
            .map(|p| {
                let status = pkg_status(&p.name, &p.version);
                PkgInfo {
                    name: p.name,
                    version: p.version,
                    repo: p.repo,
                    status,
                }
            })
            .collect(),
        Ok(_) => targets,
        Err(e) => {
            eprintln!(
                "{} dependency resolution failed ({}); showing requested packages only",
                ">>> Warning:".yellow().bold(),
                e
            );
            targets
        }
    }
}

/// AUR RPC info → (found, missing). Missing is not a hard error.

/// Suggest close package names when an exact atom is missing (Portage-style).
pub(crate) fn print_similar_names(term: &str) {
    let bare = term.split('/').last().unwrap_or(term);
    if bare.is_empty() {
        return;
    }
    eprintln!("{} searching for similar names...", "emerge:".yellow());

    // Try full term, then progressively shorter prefixes (Portage-ish).
    let mut stems: Vec<&str> = vec![bare];
    if bare.len() > 4 {
        stems.push(&bare[..bare.len().saturating_sub(2)]);
    }
    if bare.len() > 6 {
        stems.push(&bare[..3]);
    }
    let mut hits: Vec<String> = Vec::new();
    for stem in stems {
        if stem.is_empty() {
            continue;
        }
        for p in crate::alpm_db::search_sync(stem, false, true) {
            hits.push(format!("{}/{}", p.repo, p.name));
        }
        if hits.len() >= 5 {
            break;
        }
    }
    if hits.len() < 5 {
        for h in crate::aur::rpc_search(bare, false) {
            hits.push(format!("aur/{}", h.name));
            if hits.len() >= 8 {
                break;
            }
        }
    }
    hits.sort_by(|a, b| {
        let an = a.split('/').last().unwrap_or(a);
        let bn = b.split('/').last().unwrap_or(b);
        let ap = an.starts_with(bare) as i8;
        let bp = bn.starts_with(bare) as i8;
        bp.cmp(&ap).then_with(|| an.len().cmp(&bn.len()))
    });
    hits.dedup();
    hits.truncate(5);
    if hits.is_empty() {
        eprintln!("{} no similar package names found.", "emerge:".yellow());
        return;
    }
    eprintln!(
        "{} Maybe you meant any of these: {}",
        "emerge:".yellow(),
        hits.join(", ")
    );
}

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

/// `pkgs` is the plan to show; `requested` is what was typed on the
/// command line -- with `tree`, anything else in `pkgs` nests under
/// whichever package's "Depends On" names it, `emerge -t`-style.
/// `deep`: `None` = direct deps only, `Some(0)` = every level,
/// `Some(n)` = capped at `n` levels.
pub(crate) fn print_emerge_plan(
    pkgs: &[PkgInfo],
    tree: bool,
    deep: Option<u32>,
    requested: &[String],
) {
    println!();
    println!(
        "{}",
        "These are the packages that would be merged, in order:"
            .green()
            .bold()
    );
    println!();
    crate::candy::calculating_deps_done();
    println!();

    let requested: HashSet<&str> = requested.iter().map(|p| bare_of(p)).collect();
    let entries: Vec<(&PkgInfo, usize)> = if tree {
        build_plan_tree(pkgs, &requested, deep)
    } else {
        pkgs.iter().map(|p| (p, 0)).collect()
    };

    for (p, depth) in &entries {
        let prefix = if *depth == 0 {
            String::new()
        } else {
            format!("{}`-- ", "  ".repeat(depth - 1))
        };
        println!(
            "[{}  {:<4} ] {}{}",
            "ebuild".green(),
            status_colored(&p.status),
            prefix,
            format_atom(p).green().bold()
        );
    }
    println!();
    println!("{}: {} package(s)", "Total".bold(), pkgs.len());
    println!();
}

/// Portage-style `--ask`: one prompt after the plan, before any work.
/// Returns `true` to proceed. When `ask` is false, always proceeds.
///
/// `action` is the verb in the question ("merge" / "unmerge" / ...).
/// After a yes, callers must keep package managers non-interactive.
///
/// Accepted answers match Portage: empty / y / yes (case-insensitive).
/// Anything else aborts.

/// Source-built glibc is a footgun: a bad build can break every dynamic
/// binary on the system (including pacman). Warn hard and require an
/// explicit yes before abs/aur rebuilds of these packages.
pub(crate) fn warn_critical_libc(names: &[String], source: &str) -> bool {
    const CRITICAL: &[&str] = &["glibc", "lib32-glibc"];
    let hits: Vec<&str> = names
        .iter()
        .map(|n| n.split('/').last().unwrap_or(n.as_str()))
        .filter(|n| CRITICAL.iter().any(|c| c == n))
        .collect();
    if hits.is_empty() {
        return true;
    }
    eprintln!();
    eprintln!(
        "{} about to rebuild {} from {}:",
        "!!!".red().bold(),
        hits.join(", ").bold(),
        source.yellow().bold()
    );
    eprintln!(
        "{} a failed or partial {} install can leave the system unable to run",
        "!!!".red().bold(),
        "glibc".bold()
    );
    eprintln!(
        "{} dynamic binaries (including pacman). Prefer official repo packages",
        "!!!".red().bold()
    );
    eprintln!(
        "{} unless you intentionally need a source rebuild.",
        "!!!".red().bold()
    );
    eprintln!();
    print!(
        "Really proceed with source {}? [{}/{}] ",
        source,
        "Yes".green().bold(),
        "No".red().bold()
    );
    let _ = io::stdout().flush();
    let answer = crate::read_line_raw();
    let ok = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if !ok {
        println!("{} Quitting.", ">>>".yellow().bold());
    }
    ok
}

pub(crate) fn confirm_merge(ask: bool) -> bool {
    confirm_action(ask, "merge")
}

/// Same as `confirm_merge`, with a custom verb (`unmerge`, `prune`, ...).
pub(crate) fn confirm_action(ask: bool, action: &str) -> bool {
    if !ask {
        return true;
    }
    // No >>> on the question -- Portage prints it bare; arrows are for
    // status lines. Yes/No colored so the choice is readable at a glance.
    print!(
        "Would you like to {} these packages? [{}/{}] ",
        action,
        "Yes".green().bold(),
        "No".red().bold()
    );
    let _ = io::stdout().flush();
    let answer = crate::read_line_raw();
    let ok = matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    );
    if !ok {
        println!("{} Quitting.", ">>>".yellow().bold());
    }
    ok
}

fn bare_of(p: &str) -> &str {
    p.split('/').last().unwrap_or(p)
}

/// Per-package "Depends On", via `pacman -Si` (sync db, so this also
/// works for a not-yet-installed package -- what the tree needs to
/// explain why each dependency showed up in the plan).
pub(crate) fn depends_on_map(names: &[String]) -> HashMap<String, HashSet<String>> {
    crate::alpm_db::depends_map(names)
}

/// Orders `pkgs` for tree display: `requested` at depth 0, everything
/// else nested under whichever package's "Depends On" names it --
/// recursively when `deep`, one level otherwise. An unmatched
/// dependency (a `provides` match, or beyond `--deep`'s reach) still
/// shows at depth 1 -- no fabricated parent, just real information.
///
/// Skips `-Si` entirely when there's nothing to explain -- AUR-only
/// installs, where the plan doesn't resolve transitive AUR deps yet,
/// are just `pkgs == requested`.
pub(crate) fn build_plan_tree<'a>(
    pkgs: &'a [PkgInfo],
    requested: &HashSet<&str>,
    deep: Option<u32>,
) -> Vec<(&'a PkgInfo, usize)> {
    let (top, extra): (Vec<&PkgInfo>, Vec<&PkgInfo>) = pkgs
        .iter()
        .partition(|p| requested.contains(p.name.as_str()));
    if extra.is_empty() {
        return top.into_iter().map(|p| (p, 0)).collect();
    }

    let names: Vec<String> = pkgs.iter().map(|p| p.name.clone()).collect();
    let deps_by_name = depends_on_map(&names);
    // None -> direct deps, Some(0) -> unbounded, Some(n) -> capped.
    let max_depth = match deep {
        None => 1,
        Some(0) => usize::MAX,
        Some(n) => n as usize,
    };
    group_by_parent(top, extra, &deps_by_name, max_depth)
}

/// Split out from `build_plan_tree` so it's testable without shelling
/// out to pacman for `deps_by_name`. DFS, not BFS: each parent is
/// immediately followed by its own subtree, which is what makes the
/// indentation read as a tree.
fn group_by_parent<'a>(
    top: Vec<&'a PkgInfo>,
    mut remaining: Vec<&'a PkgInfo>,
    deps_by_name: &HashMap<String, HashSet<String>>,
    max_depth: usize,
) -> Vec<(&'a PkgInfo, usize)> {
    let mut out: Vec<(&PkgInfo, usize)> = Vec::new();
    let mut placed: HashSet<&str> = HashSet::new();

    for parent in &top {
        out.push((parent, 0));
        placed.insert(parent.name.as_str());
        place_children(
            parent.name.as_str(),
            0,
            max_depth,
            &mut remaining,
            &mut placed,
            deps_by_name,
            &mut out,
        );
    }

    // Nothing claimed within max_depth still shows, just without a
    // specific parent to nest under.
    for p in remaining {
        if placed.insert(p.name.as_str()) {
            out.push((p, 1));
        }
    }
    out
}

/// Places `parent_name`'s still-unplaced children right after it,
/// recursing into their own children (bounded by `max_depth`). Each
/// child is removed from `remaining` before recursing into it, so a
/// dependency cycle in the data can't loop forever.
fn place_children<'a>(
    parent_name: &str,
    parent_depth: usize,
    max_depth: usize,
    remaining: &mut Vec<&'a PkgInfo>,
    placed: &mut HashSet<&'a str>,
    deps_by_name: &HashMap<String, HashSet<String>>,
    out: &mut Vec<(&'a PkgInfo, usize)>,
) {
    if parent_depth >= max_depth {
        return;
    }
    let Some(deps) = deps_by_name.get(parent_name) else {
        return;
    };

    let mut children: Vec<&'a PkgInfo> = Vec::new();
    let mut i = 0;
    while i < remaining.len() {
        if deps.contains(&remaining[i].name) && !placed.contains(remaining[i].name.as_str()) {
            children.push(remaining.remove(i));
        } else {
            i += 1;
        }
    }

    for child in children {
        placed.insert(child.name.as_str());
        out.push((child, parent_depth + 1));
        place_children(
            child.name.as_str(),
            parent_depth + 1,
            max_depth,
            remaining,
            placed,
            deps_by_name,
            out,
        );
    }
}

pub(crate) fn print_emerge_emerging(pkgs: &[PkgInfo]) {
    // Plan size only; per-package lines are printed live by the
    // installers (Installing / Compiling / Completed).
    crate::progress::begin(pkgs.len());
    println!();
}

/// Refresh versions from the local db after install (plan may be stale).
pub(crate) fn refresh_installed_versions(pkgs: &mut [PkgInfo]) {
    for p in pkgs.iter_mut() {
        if let Some(ver) = crate::alpm_db::installed_version(&p.name) {
            if !ver.is_empty() {
                p.version = ver;
            }
        }
    }
}

/// Final summary after a run. Per-package Installing/Completed are
/// emitted live during `repo_install_landed`; this only prints Jobs.
pub(crate) fn print_emerge_completed(pkgs: &[PkgInfo]) {
    if pkgs.is_empty() {
        return;
    }
    crate::progress::finish();
    println!();
}

// ── ABS (Arch Build System) support ──────────────────────────────────────────

/// `[epoch:]pkgver-pkgrel` from ABS GitLab .SRCINFO (no clone).
/// Full version, not bare pkgver: `vercmp` skips the release when one
/// side lacks it, so `0.12.5-1.1` vs `0.12.5` read as a reinstall.
pub(crate) fn abs_get_version(pkg: &str) -> String {
    let url = format!("{}/{}/raw/HEAD/.SRCINFO", ABS_GITLAB_BASE, pkg);
    if let Some(text) = crate::http::get(&url, 5) {
        return srcinfo_full_version(&text).unwrap_or_else(|| "?".to_string());
    }
    "?".to_string()
}

/// First `epoch`/`pkgver`/`pkgrel` in a .SRCINFO (the pkgbase block).
fn srcinfo_full_version(text: &str) -> Option<String> {
    let (mut epoch, mut ver, mut rel) = (None, None, None);
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "epoch" if epoch.is_none() => epoch = Some(v),
            "pkgver" if ver.is_none() => ver = Some(v),
            "pkgrel" if rel.is_none() => rel = Some(v),
            _ => {}
        }
    }
    let ver = ver?;
    let mut out = match epoch.filter(|e| e != "0") {
        Some(e) => format!("{}:{}", e, ver),
        None => ver,
    };
    if let Some(r) = rel {
        out.push('-');
        out.push_str(&r);
    }
    Some(out)
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

/// Pull a 16-hex key id out of makepkg/gpg noise ("unknown public key",
/// Ukrainian "невідомий публічний ключ", etc.).
pub(crate) fn pgp_keys_from_log(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        // English + Ukrainian makepkg/gpg messages + gpg --status-fd.
        let hit = lower.contains("unknown public key")
            || line.contains("невідомий публічний ключ")
            || lower.contains("using unknown key")
            || lower.contains("no_pubkey")
            || lower.contains("no public key");
        if !hit {
            continue;
        }
        // Last hex run of length >= 8 on the line is the key id.
        let mut best = None;
        for word in line.split(|c: char| !c.is_ascii_hexdigit()) {
            if word.len() >= 8 && word.len() <= 40 && word.chars().all(|c| c.is_ascii_hexdigit()) {
                best = Some(word.to_ascii_uppercase());
            }
        }
        if let Some(k) = best {
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// When the build log omits the key id, probe `*.asc` next to sources
/// with `gpg --list-packets` / status for the issuer.
pub(crate) fn pgp_keys_from_asc_dir(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return out,
    };
    for e in rd.flatten() {
        let p = e.path();
        let is_asc = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("asc"))
            .unwrap_or(false);
        if !is_asc {
            continue;
        }
        let Ok(outp) = Command::new(GPG_BIN)
            .args(["--list-packets", "--batch"])
            .arg(&p)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        else {
            continue;
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&outp.stdout),
            String::from_utf8_lossy(&outp.stderr)
        );
        // "keyid: 0x514BBE2EB8E1961F" or "issuer key ID 514BBE2EB8E1961F"
        for line in text.lines() {
            let lower = line.to_ascii_lowercase();
            if !(lower.contains("keyid") || lower.contains("issuer")) {
                continue;
            }
            for word in line.split(|c: char| !c.is_ascii_hexdigit()) {
                if word.len() >= 8
                    && word.len() <= 40
                    && word.chars().all(|c| c.is_ascii_hexdigit())
                {
                    let k = word.to_ascii_uppercase();
                    if !out.contains(&k) {
                        out.push(k);
                    }
                }
            }
        }
    }
    out
}

/// Import one key from the default keyserver. Prints status.
pub(crate) fn import_pgp_key(key: &str) -> bool {
    if !std::path::Path::new(GPG_BIN).exists() {
        return false;
    }
    if gpg_key_present(key) {
        return true;
    }
    print!("    {} ... ", key);
    let _ = io::stdout().flush();
    let ok = Command::new(GPG_BIN)
        .args(["--keyserver", PGP_KEYSERVER, "--recv-keys", key])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    println!(
        "{}",
        if ok {
            "ok".green().to_string()
        } else {
            "failed".red().to_string()
        }
    );
    ok
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
            let _ = import_pgp_key(key);
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

/// Build isolation: bwrap is required; `None` only with `--no-sandbox`.
/// (pkgctl build was dropped; pkgctl only used for repo clone.)
#[derive(Clone, Copy, PartialEq)]
enum BuildIsolation {
    Bwrap,
    None,
}

/// Missing bwrap is a hard stop, not a warning: a silent fallback would
/// make the sandbox promise depend on an optional package.
fn choose_build_isolation(no_sandbox: bool) -> BuildIsolation {
    if no_sandbox {
        return BuildIsolation::None;
    }
    if crate::sandbox::bwrap_available() {
        return BuildIsolation::Bwrap;
    }
    eprintln!(
        "{} bubblewrap (bwrap) not found -- refusing to build without isolation.",
        ">>> Error:".red().bold()
    );
    eprintln!(
        "{} install it (`emerge bubblewrap`), or pass --no-sandbox to build unisolated on purpose.",
        ">>> Hint:".yellow().bold()
    );
    std::process::exit(1);
}

/// Satisfiable without AUR (synced repo or already installed).
/// Not the same as already_satisfied (provides-aware).
fn is_satisfiable_without_aur(name: &str) -> bool {
    let bare = name.split(['<', '>', '=']).next().unwrap_or(name);
    crate::alpm_db::find_sync(bare).is_some() || crate::alpm_db::is_installed(bare)
}

/// Dep already satisfied by the local db (exact name, provides, version).
fn already_satisfied(name: &str) -> bool {
    crate::alpm_db::unsatisfied(&[name.to_string()]).is_empty()
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
    Some(
        base.join("aura-emerge/pkgbuild-view")
            .join(format!("{}.PKGBUILD", pkgbase)),
    )
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
        return PkgbuildViewOutcome {
            proceed: true,
            edited: false,
        };
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
            let tmp_prev = std::env::temp_dir().join(format!(
                "aura-emerge-pkgbuild-view-{}-prev",
                std::process::id()
            ));
            let mut shown_diff = false;
            if std::path::Path::new("/usr/bin/diff").exists() && fs::write(&tmp_prev, prev).is_ok()
            {
                let diff_out = Command::new("/usr/bin/diff")
                    .args([
                        "-u",
                        "--label",
                        "PKGBUILD (previous)",
                        "--label",
                        "PKGBUILD (current)",
                    ])
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
        return PkgbuildViewOutcome {
            proceed: true,
            edited: false,
        };
    }

    eprint!(
        "{} Open it in $EDITOR instead of skipping it? [y/N] ",
        ">>>".yellow().bold()
    );
    io::stderr().flush().ok();
    let answer2 = read_line_raw();
    if !answer2.trim().eq_ignore_ascii_case("y") {
        eprintln!("{} skipping '{}'.", ">>>".red().bold(), pkgbase);
        return PkgbuildViewOutcome {
            proceed: false,
            edited: false,
        };
    }

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "nano".to_string());
    println!(
        "{} Opening {} in {}...",
        ">>>".green().bold(),
        "PKGBUILD".bold(),
        editor.green().bold()
    );
    println!(
        "{} Save and close the editor to continue building.",
        ">>>".yellow().bold()
    );
    Command::new(&editor).arg(&pkgbuild_path).status().ok();
    reset_terminal_colors_after_editor();

    PkgbuildViewOutcome {
        proceed: true,
        edited: true,
    }
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
            format!(
                "(cd {} && makepkg --printsrcinfo > .SRCINFO)",
                dir.display()
            )
            .cyan()
        );
        return;
    }

    println!(
        "{} Regenerating .SRCINFO from the edited PKGBUILD...",
        ">>>".green().bold()
    );
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
                eprintln!(
                    "    pass {} to skip this step next time.",
                    "--skip-srcinfo-regen".cyan()
                );
            }
        }
        _ => {
            eprintln!(
                "{} `makepkg --printsrcinfo` failed in {} -- continuing with the previous .SRCINFO, which may not reflect your edit.",
                ">>> Warning:".yellow().bold(),
                dir.display().to_string().dimmed()
            );
            eprintln!(
                "    pass {} to skip this step next time.",
                "--skip-srcinfo-regen".cyan()
            );
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
    unshare_net_build: bool,
    building: &mut HashSet<String>,
    built: &mut HashMap<String, Vec<String>>,
    pkgbuild_view: bool,
) -> Option<Vec<String>> {
    // Fast-path: skip rebuild if cache already has current source.
    if let Some(cached) = built.get(pkg) {
        return Some(cached.clone());
    }

    // Checked here, not just at the command line, so recursive AUR
    // deps can't smuggle a masked package in either.
    if let Some(entry) = crate::mask::find(pkg, Some("aur")) {
        eprintln!(
            "{} '{}' is masked by {}{}",
            ">>> Error:".red().bold(),
            pkg,
            entry.describe(),
            if is_top_level {
                ""
            } else {
                " (pulled in as a dependency)"
            }
        );
        if let Some(reason) = &entry.reason {
            eprintln!("    reason: {}", reason);
        }
        return None;
    }

    let Some((dir, pkgbase)) = crate::aur::clone_or_resolve(pkg, build_root) else {
        eprintln!(
            "{} '{}' not found in the AUR",
            ">>> Error:".red().bold(),
            pkg
        );
        return None;
    };

    if let Some(cached) = built.get(&pkgbase) {
        return Some(cached.clone());
    }
    if !building.insert(pkgbase.clone()) {
        eprintln!(
            "{} circular AUR dependency involving '{}'",
            ">>> Error:".red().bold(),
            pkgbase
        );
        return None;
    }

    // Stage 1: clone done -> Emerging (scan / review / deps follow).
    if !is_top_level {
        crate::progress::grow(1);
    }
    let stage_n = crate::progress::take();
    let stage_atom = crate::progress::atom(
        "aur",
        pkg,
        &crate::aur::srcinfo_version(&dir.join(".SRCINFO")).unwrap_or_default(),
    );
    crate::progress::line(crate::progress::Stage::Emerging, stage_n, &stage_atom);

    let fetched = crate::security::scan_aur_pkgbuilds_or_abort(&[pkgbase.clone()]);
    crate::security::verify_local_clone_or_rescan(&pkgbase, &dir, fetched.get(&pkgbase));

    // --edit: top-level only (not recursive deps).
    if edit && is_top_level {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "nano".to_string());
        let pkgbuild = dir.join("PKGBUILD");
        println!(
            "{} Opening {} in {}...",
            ">>>".green().bold(),
            "PKGBUILD".bold(),
            editor.green().bold()
        );
        println!(
            "{} Save and close the editor to continue building.",
            ">>>".yellow().bold()
        );
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
        // AUR-only deps are always resolved and built recursively.
        match resolve_and_build_aur(
            dep,
            build_root,
            ask,
            skippgp,
            true,
            false,
            false,
            skip_srcinfo_regen,
            isolation,
            unshare_net_build,
            building,
            built,
            false,
        ) {
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
    // Stage 2: deps are in, makepkg starts.
    crate::progress::line(crate::progress::Stage::Installing, stage_n, &stage_atom);
    let result = match isolation {
        BuildIsolation::Bwrap => build_with_sandbox(
            &dir,
            &pkgbase,
            ask,
            mark_asdeps,
            skippgp,
            &aur_dep_tarballs,
            unshare_net_build,
        )
        .then(|| find_built_packages(&dir, build_started)),
        BuildIsolation::None => {
            if !install_local_tarballs(&aur_dep_tarballs, ask, true) {
                eprintln!(
                    "{} failed to install locally-built AUR dependencies for '{}'",
                    ">>> Error:".red().bold(),
                    pkgbase
                );
                building.remove(&pkgbase);
                return None;
            }
            legacy_makepkg_si(&dir, ask, mark_asdeps, skippgp)
                .then(|| find_built_packages(&dir, build_started))
        }
    };

    building.remove(&pkgbase);
    if let Some(tars) = &result {
        built.insert(pkgbase.clone(), tars.clone());
        crate::progress::line(crate::progress::Stage::Completed, stage_n, &stage_atom);
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
            return std::path::PathBuf::from(xdg)
                .join("aura-emerge/build")
                .join(name);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return std::path::PathBuf::from(home)
                .join(".cache/aura-emerge/build")
                .join(name);
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
pub(crate) fn aur_install(
    pkgs: &[String],
    pretend: bool,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    edit: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
    pkgbuild_view: bool,
) -> bool {
    if !warn_critical_libc(pkgs, "AUR") {
        return false;
    }
    if !std::path::Path::new("/usr/bin/git").exists() {
        eprintln!(
            "{} required binary not found: /usr/bin/git",
            ">>> Fatal:".red().bold()
        );
        return false;
    }

    let bare: Vec<String> = pkgs
        .iter()
        .map(|p| p.split('/').last().unwrap_or(p).to_string())
        .collect();
    if bare.is_empty() {
        return false;
    }

    if pretend {
        println!();
        println!(
            "{}",
            "These are the packages that would be merged, in order:"
                .green()
                .bold()
        );
        println!();
        for p in &bare {
            println!("[{}] {} (AUR)", "aur".green(), p.green().bold());
        }
        println!();
        println!("{}: {} package(s)", "Total".bold(), bare.len());
        println!();
        return true;
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
            eprintln!(
                "      {}",
                format!("sudo lsof +D {}", build_base.display()).cyan()
            );
            return false;
        }
    }
    if std::fs::create_dir_all(&build_base).is_err() {
        eprintln!(
            "{} could not create build directory {}",
            ">>> Fatal:".red().bold(),
            build_base.display()
        );
        return false;
    }

    crate::progress::reserve(bare.len());
    let jobsa = crate::runtime::get().jobsa.max(1) as usize;

    // jobsa == 1 (or single package): sequential, shared dep cache.
    // jobsa > 1: parallel top-level builds; each worker has its own
    // building/built map (shared AUR deps may be built more than once,
    // which is safe — the second install is a no-op / reinstall).
    if jobsa <= 1 || bare.len() <= 1 {
        let mut building = HashSet::new();
        let mut built = HashMap::new();
        let mut all_ok = true;
        for (i, pkg) in bare.iter().enumerate() {
            let timer = crate::logbook::Timer::start();
            let result = resolve_and_build_aur(
                pkg,
                &build_base,
                ask,
                skippgp,
                oneshot,
                edit,
                true,
                skip_srcinfo_regen,
                isolation,
                unshare_net_build,
                &mut building,
                &mut built,
                pkgbuild_view,
            );
            if result.is_some() {
                crate::logbook::log_merge_one("aur", pkg, timer.elapsed());
            }
            if result.is_none() {
                all_ok = false;
                crate::runtime::record_failure(pkg, "AUR build failed");
                let left = bare.len() - (i + 1);
                if !crate::runtime::keep_going() && left > 0 {
                    eprintln!(
                        "{} stopping after the first failure - {} package(s) not attempted. Pass {} to build the rest and get a summary at the end.",
                        ">>> Error:".red().bold(),
                        left,
                        "--keep-going".cyan()
                    );
                    break;
                }
            }
        }
        return all_ok;
    }

    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    let all_ok = Arc::new(AtomicBool::new(true));
    let queue: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(bare.clone()));
    let workers = jobsa.min(bare.len());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let queue = queue.clone();
            let all_ok = all_ok.clone();
            let build_base = &build_base;
            scope.spawn(move || {
                loop {
                    if !all_ok.load(AtomicOrdering::Relaxed) && !crate::runtime::keep_going() {
                        break;
                    }
                    let pkg = {
                        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                        q.pop()
                    };
                    let Some(pkg) = pkg else { break };
                    let mut building = HashSet::new();
                    let mut built = HashMap::new();
                    let timer = crate::logbook::Timer::start();
                    let result = resolve_and_build_aur(
                        &pkg,
                        build_base,
                        ask,
                        skippgp,
                        oneshot,
                        edit,
                        true,
                        skip_srcinfo_regen,
                        isolation,
                        unshare_net_build,
                        &mut building,
                        &mut built,
                        pkgbuild_view,
                    );
                    if result.is_some() {
                        crate::logbook::log_merge_one("aur", &pkg, timer.elapsed());
                    } else {
                        all_ok.store(false, AtomicOrdering::Relaxed);
                        crate::runtime::record_failure(&pkg, "AUR build failed");
                        if !crate::runtime::keep_going() {
                            // Drop remaining so other workers exit.
                            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                            q.clear();
                            break;
                        }
                    }
                }
            });
        }
    });

    all_ok.load(AtomicOrdering::Relaxed)
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
    ["-git", "-hg", "-svn", "-bzr"]
        .iter()
        .any(|suf| name.ends_with(suf))
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
    let end = rest
        .find(['\'', '"', ' ', '\n', '\t'])
        .unwrap_or(rest.len());
    let raw = &rest[..end];
    let (url_part, frag) = match raw.split_once('#') {
        Some((u, f)) => (u, Some(f)),
        None => (raw, None),
    };
    if url_part.is_empty() {
        return None;
    }
    let branch = frag.and_then(|f| {
        f.split('&')
            .find_map(|kv| kv.strip_prefix("branch="))
            .map(str::to_string)
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
    text.lines()
        .next()?
        .split_whitespace()
        .next()
        .map(str::to_string)
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
    let Some(path) = devel_state_path() else {
        return Default::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Default::default();
    };
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
            eprintln!(
                ">>> Warning: could not create {}, devel state not saved",
                parent.display()
            );
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
    let foreign = crate::alpm_db::foreign_packages();

    let devel_names: Vec<String> = foreign
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| is_devel_pkg(n))
        .map(str::to_string)
        .collect();

    if devel_names.is_empty() {
        println!(">>> No installed -git/-hg/-svn/-bzr packages found.");
        return true;
    }

    println!(
        "{} Checking upstream for {} devel package(s)...",
        ">>>".green().bold(),
        devel_names.len()
    );
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
        println!(
            "{} No devel package(s) with upstream changes.",
            ">>>".green().bold()
        );
    } else {
        println!(
            "{} {} devel package(s) with upstream changes:",
            ">>>".yellow().bold(),
            moved.len()
        );
        for n in &moved {
            println!(
                "  [{} {:<4}] {}",
                "ebuild".green(),
                "U".yellow().bold(),
                n.yellow().bold()
            );
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

pub(crate) fn aur_upgrade_plan(devel: bool) -> Vec<String> {
    let installed: Vec<(String, String)> = crate::alpm_db::foreign_packages();

    if installed.is_empty() {
        println!(">>> No foreign (AUR/local) packages installed - nothing to upgrade.");
        return Vec::new();
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
                if alpm::vercmp(installed_ver.as_str(), info.version.as_str())
                    == std::cmp::Ordering::Less
                {
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
            println!(
                ">>> Checking upstream for {} devel package(s)...",
                candidates.len()
            );
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

    // --exclude and the mask hold a package at its installed version,
    // same as `pacman --ignore` does for the official half.
    let mut held: Vec<String> = Vec::new();
    to_upgrade.retain(|(name, _, _)| {
        if crate::runtime::is_excluded(name) {
            held.push(format!("{} (--exclude)", name));
            return false;
        }
        if let Some(entry) = crate::mask::find(name, Some("aur")) {
            held.push(format!("{} (masked by {})", name, entry.describe()));
            return false;
        }
        true
    });
    if !held.is_empty() {
        println!(
            ">>> {} AUR package(s) held back: {}",
            held.len(),
            held.join(", ")
        );
    }

    if to_upgrade.is_empty() {
        println!(">>> No AUR packages out of date.");
        return Vec::new();
    }

    println!();
    for (name, old, new) in &to_upgrade {
        println!(
            "[{} {:<4}] {} [{} -> {}]",
            "ebuild".green(),
            "U".yellow().bold(),
            name.yellow().bold(),
            old,
            new
        );
    }
    println!();
    println!(
        "{}: {} AUR package(s) to upgrade",
        "Total".bold(),
        to_upgrade.len()
    );
    println!();

    to_upgrade.into_iter().map(|(n, _, _)| n).collect()
}

/// Full-upgrade AUR half: build the names from `aur_upgrade_plan`.
pub(crate) fn aur_upgrade_names(
    names: &[String],
    ask: bool,
    skippgp: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
) -> bool {
    if names.is_empty() {
        return true;
    }
    aur_install(
        names,
        false,
        ask,
        false,
        skippgp,
        false,
        no_sandbox,
        skip_srcinfo_regen,
        unshare_net_build,
        false,
    )
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
    // What package() produced is still untrusted input to this root
    // step. Pin (path + sha256) what is about to be audited; the root
    // helper installs only bytes that hash to the same value.
    let pinned = match crate::rootops::pin(tarballs) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{} cannot read the built package(s): {}",
                ">>> Error:".red().bold(),
                e
            );
            return false;
        }
    };
    // Audit the archive (setuid, .INSTALL, hooks, ...) first.
    if !crate::security::audit_built_packages(tarballs, ask) {
        return false;
    }
    if !crate::rootops::unchanged(&pinned) {
        eprintln!(
            "{} a built package changed while it was being audited - not installing.",
            ">>> Error:".red().bold()
        );
        return false;
    }
    println!(
        "{} Installing {} locally-built AUR dependency(ies)...",
        ">>>".green().bold(),
        tarballs.len()
    );
    let opts = crate::helper::validate::FileOpts {
        needed: true,
        asdeps: mark_asdeps,
    };
    match crate::rootops::install_files(&pinned, opts, &mut |ev| crate::progress::on_hook_event(ev))
    {
        Ok(()) => {
            for t in tarballs {
                let name = std::path::Path::new(t)
                    .file_name()
                    .map_or_else(|| t.clone(), |n| n.to_string_lossy().into_owned());
                println!("{} Installed {}", ">>>".green().bold(), name);
            }
            true
        }
        Err(e) => {
            eprintln!("{} {}", ">>> Error:".red().bold(), e);
            false
        }
    }
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
fn build_with_sandbox(
    build_dir: &std::path::Path,
    pkgbase: &str,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    aur_dep_tarballs: &[String],
    unshare_net_build: bool,
) -> bool {
    // Cleans up the fakeroot shim's scratch dir (now outside build_dir,
    // see sandbox::shim_root / FakerootShimGuard) on every exit path below.
    let _fakeroot_shim_guard = crate::sandbox::FakerootShimGuard::new(build_dir);

    let pkgbuild_src = match fs::read_to_string(build_dir.join("PKGBUILD")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "{} could not read PKGBUILD for '{}': {}",
                ">>> Error:".red().bold(),
                pkgbase,
                e
            );
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
        eprintln!(
            "{} failed to install locally-built AUR dependencies for '{}'",
            ">>> Error:".red().bold(),
            pkgbase
        );
        return false;
    }

    // 1b. Official deps via pacman -S (outside sandbox).
    match repo_deps {
        Some(deps) if !deps.is_empty() => {
            println!(
                "{} Installing {} declared dependency(ies) via libalpm...",
                ">>>".green().bold(),
                deps.len()
            );
            let mut dep_names: Vec<String> =
                deps.iter().map(|d| strip_version_operator(d)).collect();
            dep_names.sort();
            dep_names.dedup();
            let ok = match crate::alpm_install_quiet(&dep_names, true, true) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("{} {}", ">>> Error:".red().bold(), e);
                    false
                }
            };
            if !ok {
                eprintln!(
                    "{} failed to install dependencies for '{}'",
                    ">>> Error:".red().bold(),
                    pkgbase
                );
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
    // -f: overwrite an existing package file in the build dir (reinstall /
    // same-version rebuild otherwise dies with "Package already built").
    let mut makepkg_args: Vec<&str> = vec!["-f"];
    if !ask {
        makepkg_args.push("--noconfirm");
    }
    if skippgp {
        makepkg_args.push("--skippgpcheck");
    }

    // make.conf's build flags, added before makepkg_args is cloned
    // for the --nobuild/--noextract split below so both halves get it.
    let build_cfg = build_config(build_dir);
    let conf_override = makepkg_conf_override(build_dir, &build_cfg);
    if let Some(path) = &conf_override {
        // Same for every package: show once per run.
        if crate::progress::once("build-flags") {
            crate::progress::note(&format!(
                "{} applying build flags from {}",
                ">>>".green().bold(),
                build_cfg
                    .files
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        makepkg_args.push("--config");
        makepkg_args.push(path.as_str());
    }

    // Keys for the sandbox ring: PKGBUILD validpgpkeys + any extras
    // accumulated on a previous failure (retry path).
    let mut sandbox_keys: Vec<String> = {
        let pb = build_dir.join("PKGBUILD");
        std::fs::read_to_string(&pb)
            .map(|text| parse_validpgpkeys(&text))
            .unwrap_or_default()
    };

    let real_gnupg = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".gnupg"));
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

    let build_started = std::time::SystemTime::now();
    let build_ok = if unshare_net_build {
        println!(
            "{} --unshare-net-build set -- fetching/extracting sources with network access, then building '{}' with none.",
            ">>>".green().bold(),
            pkgbase
        );
        let mut nobuild_args = makepkg_args.clone();
        nobuild_args.push("--nobuild");
        let fetch_ok = run_build_cmd(
            crate::sandbox::sandboxed_makepkg(
                MAKEPKG_BIN,
                build_dir,
                &nobuild_args,
                real_gnupg.as_deref(),
                &extra_dest_dirs,
                &sandbox_keys,
                true,
            ),
            pkgbase,
        )
        .is_ok();
        if !fetch_ok {
            eprintln!(
                "{} fetching/extracting sources failed for '{}'",
                ">>> Error:".red().bold(),
                pkgbase
            );
            return false;
        }
        let mut noextract_args = makepkg_args.clone();
        noextract_args.push("--noextract");
        run_build_cmd(
            crate::sandbox::sandboxed_makepkg(
                MAKEPKG_BIN,
                build_dir,
                &noextract_args,
                real_gnupg.as_deref(),
                &extra_dest_dirs,
                &sandbox_keys,
                false,
            ),
            pkgbase,
        )
    } else {
        run_build_cmd(
            crate::sandbox::sandboxed_makepkg(
                MAKEPKG_BIN,
                build_dir,
                &makepkg_args,
                real_gnupg.as_deref(),
                &extra_dest_dirs,
                &sandbox_keys,
                true,
            ),
            pkgbase,
        )
    };

    // On PGP failure, import keys cited in the log / .asc and retry once
    // with those keys in the sandbox keyring.
    let build_ok = match build_ok {
        Ok(()) => true,
        Err(log) => {
            let mut keys = pgp_keys_from_log(&log);
            if keys.is_empty() {
                keys = pgp_keys_from_asc_dir(build_dir);
                if keys.is_empty() {
                    let src = build_dir.join("src");
                    if src.is_dir() {
                        keys = pgp_keys_from_asc_dir(&src);
                    }
                }
            }
            let can_retry = !skippgp && !keys.is_empty();
            if can_retry {
                println!(
                    "{} Importing {} PGP key(s) into the sandbox keyring...",
                    ">>>".green().bold(),
                    keys.len()
                );
                for k in &keys {
                    let _ = import_pgp_key(k);
                    if !sandbox_keys.iter().any(|x| x == k) {
                        sandbox_keys.push(k.clone());
                    }
                }
                println!(
                    "{} Retrying build for '{}'...",
                    ">>>".green().bold(),
                    pkgbase
                );
                let retry = run_build_cmd(
                    crate::sandbox::sandboxed_makepkg(
                        MAKEPKG_BIN,
                        build_dir,
                        &makepkg_args,
                        real_gnupg.as_deref(),
                        &extra_dest_dirs,
                        &sandbox_keys,
                        true,
                    ),
                    pkgbase,
                );
                if retry.is_ok() {
                    true
                } else {
                    eprintln!(
                        "{} sandboxed build failed for '{}'",
                        ">>> Error:".red().bold(),
                        pkgbase
                    );
                    false
                }
            } else {
                eprintln!(
                    "{} sandboxed build failed for '{}'",
                    ">>> Error:".red().bold(),
                    pkgbase
                );
                if unshare_net_build {
                    eprintln!(
                        "    if this failed reaching for the network during build() -- check for an unvendored dependency fetch (cargo/go/pip/npm resolving its graph mid-build instead of in prepare()) and re-run without --unshare-net-build if that's expected for this package."
                    );
                }
                false
            }
        }
    };
    if !build_ok {
        return false;
    }

    // 3. Install the built package(s), outside the sandbox: another
    //    plain pacman operation, no PKGBUILD code involved.
    let pkg_files = find_built_packages(build_dir, build_started);
    if pkg_files.is_empty() {
        eprintln!(
            "{} sandboxed build for '{}' produced no package file",
            ">>> Error:".red().bold(),
            pkgbase
        );
        return false;
    }
    // Pin before audit so the helper installs only the audited bytes.
    let pinned = match crate::rootops::pin(&pkg_files) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{} cannot read the built package(s): {}",
                ">>> Error:".red().bold(),
                e
            );
            return false;
        }
    };
    if !crate::security::audit_built_packages(&pkg_files, ask) {
        return false;
    }
    if !crate::rootops::unchanged(&pinned) {
        eprintln!(
            "{} a built package changed while it was being audited - not installing.",
            ">>> Error:".red().bold()
        );
        return false;
    }
    // -U through the root helper (libalpm), not sudo pacman.
    let opts = crate::helper::validate::FileOpts {
        needed: false,
        asdeps: oneshot,
    };
    match crate::rootops::install_files(&pinned, opts, &mut |ev| crate::progress::on_hook_event(ev))
    {
        Ok(()) => true,
        Err(e) => {
            eprintln!("{} {}", ">>> Error:".red().bold(), e);
            false
        }
    }
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
fn find_built_packages(
    build_dir: &std::path::Path,
    not_before: std::time::SystemTime,
) -> Vec<String> {
    let search_dir = extra_makepkg_dest_dirs(build_dir)
        .into_iter()
        .find(|(name, _)| *name == "PKGDEST")
        .map(|(_, path)| path)
        .unwrap_or_else(|| build_dir.to_path_buf());
    let cutoff = not_before
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(not_before);

    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&search_dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
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
    // Also cleans up the generated makepkg.conf below (same scratch
    // dir; the sandboxed path has its own guard, this one needs its own).
    let _scratch_guard = crate::sandbox::FakerootShimGuard::new(build_dir);

    // -f: same as the sandboxed path -- allow rebuild over an existing tarball.
    let mut makepkg_args = vec!["-sif"];
    if !ask {
        makepkg_args.push("--noconfirm");
    }
    if oneshot {
        makepkg_args.push("--asdeps");
    }
    if skippgp {
        makepkg_args.push("--skippgpcheck");
    }

    let build_cfg = build_config(build_dir);
    let conf_override = makepkg_conf_override(build_dir, &build_cfg);
    if let Some(path) = &conf_override {
        makepkg_args.push("--config");
        makepkg_args.push(path.as_str());
        // Unsandboxed, makepkg sources the user's own makepkg.conf
        // *after* ours, so a var set there wins over make.conf --
        // warn about it. (Can't happen inside the sandbox: $HOME is an
        // empty tmpfs.) Read the user's file directly, not through
        // read_makepkg_vars() (which merges with the system config and
        // would flag CFLAGS on every machine).
        let user_conf = user_makepkg_conf_path();
        let user_text = std::fs::read_to_string(&user_conf).unwrap_or_default();
        let assigns = |key: &str| {
            user_text.lines().any(|l| {
                let l = l.trim();
                !l.starts_with('#') && l.starts_with(key) && l[key.len()..].starts_with('=')
            })
        };
        let overlap: Vec<&str> = build_cfg
            .build_vars
            .iter()
            .map(|(k, _)| k.as_str())
            .filter(|k| assigns(k))
            .collect();
        if !overlap.is_empty() {
            eprintln!(
                "{} building without the sandbox, so {} from {} takes precedence over make.conf for: {}",
                ">>> Warning:".yellow().bold(),
                "makepkg.conf".bold(),
                user_conf,
                overlap.join(", ")
            );
        }
    }

    let mut cmd = Command::new(MAKEPKG_BIN);
    cmd.args(&makepkg_args).current_dir(build_dir);

    // Unsandboxed path: real $HOME, plain makepkg.
    for (var, path) in resolve_dest_dirs(build_dir).0 {
        let _ = fs::create_dir_all(&path);
        cmd.env(var, &path);
    }

    let label = build_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("makepkg");
    run_build_cmd(cmd, label).is_ok()
}

/// Build and install packages from ABS via `pkgctl repo clone` + `makepkg -si`
/// (or, by default, the bwrap-sandboxed equivalent -- see `build_with_sandbox`).
/// `skip_plan`: when true, the caller already printed the emerge plan and
/// confirmed — do not print another plan or prompt (mixed abs+aur+repo).
pub(crate) fn abs_install(
    pkgs: &[String],
    pretend: bool,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    edit: bool,
    autopgp: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
    pkgbuild_view: bool,
    skip_plan: bool,
) -> bool {
    if !warn_critical_libc(pkgs, "ABS") {
        return false;
    }
    for bin in &[PKGCTL_BIN, MAKEPKG_BIN] {
        if !std::path::Path::new(bin).exists() {
            eprintln!(">>> Fatal: required binary not found: {}", bin);
            if *bin == PKGCTL_BIN {
                eprintln!(">>> Hint: install devtools with: emerge devtools");
            }
            return false;
        }
    }

    let pkg_infos: Vec<PkgInfo> = pkgs
        .iter()
        .filter_map(|pkg| {
            let bare = pkg.split('/').last().unwrap_or(pkg);
            if !validate_pkg(bare) || bare.contains('/') {
                eprintln!(">>> Error: invalid package name '{}' - skipping", bare);
                return None;
            }
            if let Some(entry) = crate::mask::find(bare, Some("abs")) {
                eprintln!(
                    "{} '{}' is masked by {}",
                    ">>> Error:".red().bold(),
                    bare,
                    entry.describe()
                );
                if let Some(reason) = &entry.reason {
                    eprintln!("    reason: {}", reason);
                }
                crate::runtime::record_failure(bare, "masked");
                return None;
            }
            let version = abs_get_version(bare);

            let status = pkg_status(bare, &version);
            Some(PkgInfo {
                name: bare.to_string(),
                version,
                repo: "abs".to_string(),
                status,
            })
        })
        .collect();

    if pkg_infos.is_empty() {
        return false;
    }

    let isolation = choose_build_isolation(no_sandbox);

    if !skip_plan {
        println!();
        println!(
            "{}",
            "These are the packages that would be merged, in order:"
                .green()
                .bold()
        );
        println!();
        crate::candy::calculating_deps_done();
        println!();
        for p in &pkg_infos {
            let atom = format!("{} (ABS)", format_atom(p));
            println!(
                "[{}  {:<4} ] {}",
                "ebuild".green(),
                status_colored(&p.status),
                atom.green().bold()
            );
        }
        println!();
        println!("{}: {} package(s)", "Total".bold(), pkg_infos.len());
        println!();

        if pretend {
            return true;
        }

        // --ask: same one-shot plan prompt as the AUR/repo paths in main.
        // (main passes the real `cli.ask` for direct abs/ installs; upgrade
        // already confirmed before calling us with ask=false.)
        if !confirm_merge(ask) {
            return true;
        }
    } else if pretend {
        return true;
    }

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
            eprintln!(
                "      {}",
                format!("sudo lsof +D {}", build_base.display()).cyan()
            );
            return false;
        }
    }
    crate::progress::reserve(pkg_infos.len());
    // Interactive edit/view must stay sequential on the main thread.
    let jobsa = if edit || pkgbuild_view {
        1
    } else {
        crate::runtime::get().jobsa.max(1) as usize
    };
    abs_build_many(
        &pkg_infos,
        &build_base,
        ask,
        oneshot,
        skippgp,
        edit,
        autopgp,
        skip_srcinfo_regen,
        isolation,
        unshare_net_build,
        pkgbuild_view,
        jobsa,
    )
}

/// Build one ABS package (clone → optional edit/view → makepkg → install).
/// Safe to call from a worker thread (progress I/O is locked).
fn abs_build_one(
    info: &PkgInfo,
    build_base: &std::path::Path,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    edit: bool,
    autopgp: bool,
    skip_srcinfo_regen: bool,
    isolation: BuildIsolation,
    unshare_net_build: bool,
    pkgbuild_view: bool,
) -> bool {
    let stage_n = crate::progress::take();
    let stage_atom = format_atom(info);
    crate::progress::line(crate::progress::Stage::Emerging, stage_n, &stage_atom);

    let pkg_dir = build_base.join(&info.name);
    if !pkg_dir.starts_with(build_base) {
        eprintln!(">>> Error: suspicious path for '{}' - skipping", info.name);
        crate::progress::abort_one();
        return false;
    }

    if pkg_dir.exists() {
        let _ = std::fs::remove_dir_all(&pkg_dir);
    }
    let _ = std::fs::create_dir_all(build_base);

    let mut clone = Command::new(PKGCTL_BIN);
    clone
        .args(["repo", "clone", "--protocol=https", &info.name])
        .current_dir(build_base);
    if !crate::runtime::get().debug {
        clone
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let checkout_ok = clone.status().map(|s| s.success()).unwrap_or(false);
    if !checkout_ok {
        eprintln!(
            "{} pkgctl repo clone failed for '{}'",
            ">>> Error:".red().bold(),
            info.name
        );
        eprintln!(
            "{} package may not exist in ABS (it must be a pkgbase, not a split-package output name). Try without --abs or use --aur.",
            ">>> Note:".yellow().bold()
        );
        crate::progress::abort_one();
        crate::runtime::record_failure(&info.name, "ABS clone failed");
        return false;
    }

    let build_dir = pkg_dir.clone();

    if edit {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "nano".to_string());
        let pkgbuild = build_dir.join("PKGBUILD");
        println!(
            "{} Opening {} in {}...",
            ">>>".green().bold(),
            "PKGBUILD".bold(),
            editor.green().bold()
        );
        println!(
            "{} Save and close the editor to continue building.",
            ">>>".yellow().bold()
        );
        Command::new(&editor).arg(&pkgbuild).status().ok();
        reset_terminal_colors_after_editor();
        maybe_regen_srcinfo(&build_dir, skip_srcinfo_regen);
    }

    if pkgbuild_view {
        let outcome = pkgbuild_view_step(&info.name, &build_dir);
        if !outcome.proceed {
            let _ = std::fs::remove_dir_all(&pkg_dir);
            crate::progress::abort_one();
            return false;
        }
        if outcome.edited {
            maybe_regen_srcinfo(&build_dir, skip_srcinfo_regen);
        }
    }

    if !skippgp {
        ensure_pgp_keys(&build_dir.join("PKGBUILD"), autopgp);
    }

    let timer = crate::logbook::Timer::start();
    crate::progress::line(crate::progress::Stage::Installing, stage_n, &stage_atom);
    let build_ok = match isolation {
        BuildIsolation::Bwrap => build_with_sandbox(
            &build_dir,
            &info.name,
            ask,
            oneshot,
            skippgp,
            &[],
            unshare_net_build,
        ),
        BuildIsolation::None => legacy_makepkg_si(&build_dir, ask, oneshot, skippgp),
    };

    let _ = std::fs::remove_dir_all(&pkg_dir);

    if build_ok {
        crate::logbook::log_merge_one("abs", &format_atom(info), timer.elapsed());
        crate::progress::line(crate::progress::Stage::Completed, stage_n, &stage_atom);
        true
    } else {
        eprintln!(
            "{} makepkg failed for '{}'",
            ">>> Error:".red().bold(),
            info.name
        );
        if !skippgp {
            eprintln!(
                "{} if this failed on a missing PGP key not listed in validpgpkeys, \
                find the key ID in the error above and run:",
                ">>> Hint:".yellow().bold()
            );
            eprintln!(
                ">>>   gpg --keyserver {} --recv-keys <key-id>",
                PGP_KEYSERVER
            );
            eprintln!(">>> Or retry with --autopgp (auto-import) or --skippgp (bypass checks).");
        }
        crate::progress::abort_one();
        crate::runtime::record_failure(&info.name, "ABS build failed");
        false
    }
}

/// Build ABS packages with up to `jobsa` concurrent workers.
fn abs_build_many(
    pkg_infos: &[PkgInfo],
    build_base: &std::path::Path,
    ask: bool,
    oneshot: bool,
    skippgp: bool,
    edit: bool,
    autopgp: bool,
    skip_srcinfo_regen: bool,
    isolation: BuildIsolation,
    unshare_net_build: bool,
    pkgbuild_view: bool,
    jobsa: usize,
) -> bool {
    if pkg_infos.is_empty() {
        return true;
    }
    if jobsa <= 1 || pkg_infos.len() <= 1 {
        let mut all_ok = true;
        for info in pkg_infos {
            if !abs_build_one(
                info,
                build_base,
                ask,
                oneshot,
                skippgp,
                edit,
                autopgp,
                skip_srcinfo_regen,
                isolation,
                unshare_net_build,
                pkgbuild_view,
            ) {
                all_ok = false;
                if !crate::runtime::keep_going() {
                    break;
                }
            }
        }
        return all_ok;
    }

    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    let all_ok = Arc::new(AtomicBool::new(true));
    let queue: Arc<Mutex<Vec<PkgInfo>>> = Arc::new(Mutex::new(pkg_infos.to_vec()));
    let workers = jobsa.min(pkg_infos.len());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let queue = queue.clone();
            let all_ok = all_ok.clone();
            scope.spawn(move || loop {
                if !all_ok.load(AtomicOrdering::Relaxed) && !crate::runtime::keep_going() {
                    break;
                }
                let info = {
                    let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                    q.pop()
                };
                let Some(info) = info else { break };
                if !abs_build_one(
                    &info,
                    build_base,
                    ask,
                    oneshot,
                    skippgp,
                    edit,
                    autopgp,
                    skip_srcinfo_regen,
                    isolation,
                    unshare_net_build,
                    pkgbuild_view,
                ) {
                    all_ok.store(false, AtomicOrdering::Relaxed);
                    if !crate::runtime::keep_going() {
                        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                        q.clear();
                        break;
                    }
                }
            });
        }
    });

    all_ok.load(AtomicOrdering::Relaxed)
}

/// Shared --jobsa pool for mixed abs/ + aur/ builds (both sources at once).
/// Official-repo packages are expected to be installed already.
pub(crate) fn source_builds_parallel(
    abs_pkgs: &[String],
    aur_pkgs: &[String],
    oneshot: bool,
    skippgp: bool,
    edit: bool,
    autopgp: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
    pkgbuild_view: bool,
    ask_aur: bool,
) -> bool {
    #[derive(Clone)]
    enum Job {
        Abs(PkgInfo),
        Aur(String),
    }

    let isolation = choose_build_isolation(no_sandbox);

    // Prepare ABS infos + build base.
    let abs_infos: Vec<PkgInfo> = abs_pkgs
        .iter()
        .filter_map(|bare| {
            if !validate_pkg(bare) || bare.contains('/') {
                eprintln!(">>> Error: invalid package name '{}' - skipping", bare);
                return None;
            }
            if let Some(entry) = crate::mask::find(bare, Some("abs")) {
                eprintln!(
                    "{} '{}' is masked by {}",
                    ">>> Error:".red().bold(),
                    bare,
                    entry.describe()
                );
                crate::runtime::record_failure(bare, "masked");
                return None;
            }
            let version = abs_get_version(bare);
            let status = pkg_status(bare, &version);
            Some(PkgInfo {
                name: bare.clone(),
                version,
                repo: "abs".to_string(),
                status,
            })
        })
        .collect();

    if !abs_infos.is_empty() {
        for bin in &[PKGCTL_BIN, MAKEPKG_BIN] {
            if !std::path::Path::new(bin).exists() {
                eprintln!(">>> Fatal: required binary not found: {}", bin);
                return false;
            }
        }
        if !warn_critical_libc(
            &abs_infos.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "ABS",
        ) {
            return false;
        }
    }
    if !aur_pkgs.is_empty() {
        if !warn_critical_libc(aur_pkgs, "AUR") {
            return false;
        }
        if !std::path::Path::new("/usr/bin/git").exists() {
            eprintln!(
                "{} required binary not found: /usr/bin/git",
                ">>> Fatal:".red().bold()
            );
            return false;
        }
        crate::security::scan_aur_pkgbuilds_or_abort(aur_pkgs);
    }

    let abs_base = abs_build_base();
    if !abs_infos.is_empty() {
        if abs_base.exists() {
            if let Err(e) = clear_build_base(&abs_base) {
                eprintln!(
                    "{} could not clear stale ABS build directory {}: {}",
                    ">>> Fatal:".red().bold(),
                    abs_base.display(),
                    e
                );
                return false;
            }
        }
        let _ = std::fs::create_dir_all(&abs_base);
    }

    let aur_base = aur_build_base();
    if !aur_pkgs.is_empty() {
        if aur_base.exists() {
            if let Err(e) = clear_build_base(&aur_base) {
                eprintln!(
                    "{} could not clear stale AUR build directory {}: {}",
                    ">>> Fatal:".red().bold(),
                    aur_base.display(),
                    e
                );
                return false;
            }
        }
        if std::fs::create_dir_all(&aur_base).is_err() {
            eprintln!(
                "{} could not create AUR build directory {}",
                ">>> Fatal:".red().bold(),
                aur_base.display()
            );
            return false;
        }
    }

    let mut jobs: Vec<Job> = abs_infos.iter().cloned().map(Job::Abs).collect();
    jobs.extend(aur_pkgs.iter().cloned().map(Job::Aur));
    if jobs.is_empty() {
        return true;
    }

    crate::progress::reserve(jobs.len());

    // Interactive edit/view forces serial.
    let jobsa = if edit || pkgbuild_view {
        1
    } else {
        crate::runtime::get().jobsa.max(1) as usize
    };

    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    let all_ok = Arc::new(AtomicBool::new(true));
    let queue: Arc<Mutex<Vec<Job>>> = Arc::new(Mutex::new(jobs));
    let workers = jobsa.min(queue.lock().map(|q| q.len()).unwrap_or(1).max(1));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let queue = queue.clone();
            let all_ok = all_ok.clone();
            let abs_base = &abs_base;
            let aur_base = &aur_base;
            scope.spawn(move || loop {
                if !all_ok.load(AtomicOrdering::Relaxed) && !crate::runtime::keep_going() {
                    break;
                }
                let job = {
                    let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                    q.pop()
                };
                let Some(job) = job else { break };
                let ok = match job {
                    Job::Abs(info) => abs_build_one(
                        &info,
                        abs_base,
                        false,
                        oneshot,
                        skippgp,
                        edit,
                        autopgp,
                        skip_srcinfo_regen,
                        isolation,
                        unshare_net_build,
                        pkgbuild_view,
                    ),
                    Job::Aur(pkg) => {
                        let mut building = HashSet::new();
                        let mut built = HashMap::new();
                        let timer = crate::logbook::Timer::start();
                        let result = resolve_and_build_aur(
                            &pkg,
                            aur_base,
                            ask_aur,
                            skippgp,
                            oneshot,
                            edit,
                            true,
                            skip_srcinfo_regen,
                            isolation,
                            unshare_net_build,
                            &mut building,
                            &mut built,
                            pkgbuild_view,
                        );
                        if result.is_some() {
                            crate::logbook::log_merge_one("aur", &pkg, timer.elapsed());
                            true
                        } else {
                            crate::runtime::record_failure(&pkg, "AUR build failed");
                            false
                        }
                    }
                };
                if !ok {
                    all_ok.store(false, AtomicOrdering::Relaxed);
                    if !crate::runtime::keep_going() {
                        let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
                        q.clear();
                        break;
                    }
                }
            });
        }
    });

    all_ok.load(AtomicOrdering::Relaxed)
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
/// the caller to record in world with the "Err/" prefix -- see
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
        eprintln!(
            "{} no PKGBUILD found in {}",
            ">>> Error:".red().bold(),
            path.display()
        );
        return None;
    }

    // Log label (pkgbase role for local checkouts).
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string());

    println!(
        "{} Scanning {} for suspicious patterns...",
        ">>>".green().bold(),
        label.bold()
    );
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

    crate::progress::reserve(1);
    let stage_n = crate::progress::take();
    let stage_atom = crate::progress::atom(
        "local",
        &label,
        &crate::aur::srcinfo_version(&path.join(".SRCINFO")).unwrap_or_default(),
    );
    crate::progress::line(crate::progress::Stage::Emerging, stage_n, &stage_atom);
    crate::progress::line(crate::progress::Stage::Installing, stage_n, &stage_atom);

    let isolation = choose_build_isolation(no_sandbox);
    let build_ok = match isolation {
        BuildIsolation::Bwrap => {
            build_with_sandbox(path, &label, ask, oneshot, skippgp, &[], unshare_net_build)
        }
        BuildIsolation::None => legacy_makepkg_si(path, ask, oneshot, skippgp),
    };
    if build_ok {
        crate::progress::line(crate::progress::Stage::Completed, stage_n, &stage_atom);
    }
    if !build_ok {
        eprintln!(
            "{} build failed for {}",
            ">>> Error:".red().bold(),
            path.display()
        );
        return None;
    }

    // Prefer .SRCINFO pkgnames (handles split packages).
    Some(
        crate::aur::srcinfo_pkgnames(&path.join(".SRCINFO")).unwrap_or_else(|| vec![label.clone()]),
    )
}

// ── portageq shim ─────────────────────────────────────────────────────────────

/// Called when the binary is invoked as "portageq" (via symlink).
/// Answers fish shell completion queries so emerge.fish doesn't crash.
pub(crate) fn portageq_shim(args: &[String]) {
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "envvar" => match args.get(2).map(String::as_str).unwrap_or("") {
            "EROOT" | "ROOT" => println!("/"),
            "PORTDIR" => println!("/var/db/pkg"),
            "DISTDIR" => println!("/var/cache/distfiles"),
            _ => {}
        },
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
            let pkg = args
                .get(3)
                .or_else(|| args.get(2))
                .map(String::as_str)
                .unwrap_or("");
            if let Some(ver) = crate::alpm_db::installed_version(pkg) {
                let bare = pkg.split('/').last().unwrap_or(pkg);
                println!("{}-{}", bare, ver);
            }
        }
        "list_repo_pkgs" => {
            for name in crate::alpm_db::sync_pkg_names() {
                println!("{}", name);
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
pub(crate) fn extra_makepkg_dest_dirs(
    build_dir: &std::path::Path,
) -> Vec<(&'static str, std::path::PathBuf)> {
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
pub(crate) fn resolve_dest_dirs(
    build_dir: &std::path::Path,
) -> (
    Vec<(&'static str, std::path::PathBuf)>,
    Option<std::path::PathBuf>,
) {
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

/// make.conf's build vars, shell-expanded through the same
/// generated makepkg.conf that `--config` hands to makepkg, so `--info`
/// shows what a build actually gets (e.g. `CXXFLAGS="$CFLAGS ..."`
/// resolved against make.conf's own `CFLAGS`, not the raw config text).
fn effective_build_vars(cfg: &crate::config::Config) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(conf) = crate::config::makepkg_override_conf(cfg) else {
        return map;
    };

    let mut dump = String::new();
    for key in crate::config::BUILD_VARS {
        dump.push_str(&format!("echo \"{key}=${{{key}[*]}}\"\n"));
    }
    let script = format!("{conf}\n{dump}");

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

/// makepkg.conf vars (system + user overrides), via bash source.
pub(crate) fn read_makepkg_vars() -> HashMap<String, String> {
    let watched = [
        "CARCH",
        "CHOST",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "RUSTFLAGS",
        "MAKEFLAGS",
        "OPTIONS",
        "BUILDENV",
        "PKGEXT",
        "PKGDEST",
        "SRCDEST",
        "SRCPKGDEST",
        "BUILDDIR",
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
    let Ok(file) = fs::File::open(PACMAN_CONF) else {
        return repos;
    };
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
            let name = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string();
            if name != "options" {
                current = Some((name, Vec::new()));
            }
            continue;
        }
        let Some((_, lines)) = current.as_mut() else {
            continue;
        };
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
    // Crate (build-time) + libalpm (runtime): a mismatch explains breakage.
    let pacman_ver = format!(
        "alpm crate v{} - libalpm v{}",
        env!("AE_ALPM_CRATE_VERSION"),
        alpm::version()
    );
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
    let rt = crate::runtime::get();
    println!(
        "Jobs:       --jobsr={} (repo), --jobsa={} (aur/abs)",
        rt.jobsr, rt.jobsa
    );
    println!();

    println!("Repositories:");
    println!();
    let repos = parse_pacman_repos();
    if repos.is_empty() {
        println!("    (could not read {})", PACMAN_CONF);
    }
    for (i, (name, lines)) in repos.iter().enumerate() {
        println!("{}", name);
        println!(
            "    priority: {} (order of appearance in {})",
            i + 1,
            PACMAN_CONF
        );
        for l in lines {
            println!("    {}", l);
        }
        println!();
    }

    let user_conf = user_makepkg_conf_path();
    let user_conf_active = !user_conf.is_empty() && std::path::Path::new(&user_conf).exists();
    if user_conf_active {
        println!(
            "makepkg.conf: {} -> overridden by {}",
            MAKEPKG_CONF_SYSTEM, user_conf
        );
    } else {
        println!(
            "makepkg.conf: {} (no user override present)",
            MAKEPKG_CONF_SYSTEM
        );
    }
    println!();

    let vars = read_makepkg_vars();
    let get = |k: &str| vars.get(k).cloned().unwrap_or_default();
    for key in [
        "CARCH",
        "CHOST",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "RUSTFLAGS",
        "MAKEFLAGS",
        "OPTIONS",
        "BUILDENV",
        "PKGEXT",
    ] {
        println!("{}=\"{}\"", key, get(key));
    }
    println!();

    // make.conf, printed after makepkg.conf so the two read in the
    // order they actually apply.
    let cfg = crate::runtime::config();
    if cfg.files.is_empty() {
        println!("make.conf: none found ({})", crate::config::SYSTEM_CONF);
    } else {
        println!(
            "make.conf: {}",
            cfg.files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ")
        );
        if !cfg.default_flags.is_empty() {
            println!(
                "    EMERGE_DEFAULT_OPTS=\"{}\"",
                cfg.default_flags.join(" ")
            );
        }
        // Resolved by actually sourcing the generated makepkg.conf, not
        // printed as written -- a value like CXXFLAGS="$CFLAGS ..." is
        // only meaningful once $CFLAGS itself is expanded.
        let resolved = effective_build_vars(cfg);
        for (key, _) in &cfg.build_vars {
            let shown = resolved
                .get(key)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            println!("    {}=\"{}\"  (overrides makepkg.conf)", key, shown);
        }
        // BUILDENV/OPTIONS that only FEATURES touched.
        let mut seen: Vec<&str> = cfg.build_vars.iter().map(|(k, _)| k.as_str()).collect();
        for (arr, _) in crate::config::feature_edits(cfg) {
            if seen.contains(&arr) {
                continue;
            }
            seen.push(arr);
            let shown = resolved
                .get(arr)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            println!("    {}=\"{}\"  (from FEATURES)", arr, shown);
        }
    }

    let masks = crate::mask::masks();
    if masks.is_empty() {
        println!("mask: no entries ({})", crate::mask::MASK_FILE);
    } else {
        println!(
            "mask: {} entry(ies) ({})",
            masks.len(),
            crate::mask::MASK_FILE
        );
    }
    println!();

    match world_set_stats() {
        Some((count, size)) => println!(
            "world: {} package(s), {} bytes ({})",
            count, size, WORLD_SET_FILE
        ),
        None => println!("world: not found ({})", WORLD_SET_FILE),
    }

    match crate::logbook::read_stats() {
        Some(stats) if stats.merges > 0 || stats.unmerges > 0 => println!(
            "log: {} merge(s), {} unmerge(s), {} total build time ({})",
            stats.merges,
            stats.unmerges,
            crate::logbook::fmt_duration(stats.total_build_time),
            crate::logbook::LOG_FILE
        ),
        Some(_) => println!("log: no events yet ({})", crate::logbook::LOG_FILE),
        None => println!(
            "log: not found or unreadable ({})",
            crate::logbook::LOG_FILE
        ),
    }
}

// @preserved-rebuild: installed deps → unsatisfied check → offer reinstall.

/// "glibc>=2.38" → "glibc".
pub(crate) fn strip_version_operator(atom: &str) -> String {
    match atom.find(['<', '>', '=']) {
        Some(i) => atom[..i].to_string(),
        None => atom.to_string(),
    }
}

/// Which atoms are currently unsatisfied (libalpm, like `pacman -T`).
pub(crate) fn missing_via_pacman_t(deps: &[String]) -> Vec<String> {
    if deps.is_empty() {
        return Vec::new();
    }
    crate::alpm_db::unsatisfied(deps)
}

/// @preserved-rebuild: find unsatisfied deps, offer to install them as deps.
pub(crate) fn preserved_rebuild(pretend: bool, _ask: bool) {
    println!(
        "{} Checking installed packages for missing dependencies...",
        ">>>".green().bold()
    );

    let all_deps = crate::alpm_db::all_depends();
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
        println!(
            "[{} {:<4}] {}",
            "ebuild".green(),
            "N".green().bold(),
            m.green().bold()
        );
    }
    println!();
    println!(
        "{}: {} missing dependenc{}",
        "Total".bold(),
        missing.len(),
        if missing.len() == 1 { "y" } else { "ies" }
    );
    println!();

    if pretend {
        return;
    }

    print!(
        "{} Reinstall the missing dependencies above? [y/N] ",
        ">>>".yellow().bold()
    );
    io::stdout().flush().ok();
    let answer = read_line_raw();
    let confirmed = answer.trim().eq_ignore_ascii_case("y");

    if !confirmed {
        println!(">>> Aborted.");
        return;
    }

    // Missing deps may be provides (sonames, virtual names): map them to
    // real sync packages first, libalpm installs by package name.
    let (targets, unresolved) = crate::alpm_db::sync_providers(&missing);
    if !unresolved.is_empty() {
        eprintln!(
            "{} no sync package provides: {}",
            " *".yellow().bold(),
            unresolved.join(", ")
        );
    }
    if targets.is_empty() {
        return;
    }
    if let Err(e) = crate::alpm_install_quiet(&targets, true, true) {
        eprintln!("{} {}", ">>> Error:".red().bold(), e);
    }
}
#[cfg(test)]
mod plan_tree_tests {
    use super::*;

    fn pkg(name: &str, status: &str) -> PkgInfo {
        PkgInfo {
            name: name.to_string(),
            version: "1.0-1".to_string(),
            repo: "extra".to_string(),
            status: status.to_string(),
        }
    }

    #[test]
    fn no_extras_is_flat_at_depth_zero() {
        let nano = pkg("nano", "N");
        let vim = pkg("vim", "N");
        let top = vec![&nano, &vim];
        let out = group_by_parent(top, Vec::new(), &HashMap::new(), 1);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, d)| *d == 0));
    }

    #[test]
    fn dependency_nests_under_its_declared_parent() {
        let requested = pkg("openconnect", "N");
        let pulled_in = pkg("gnutls", "N");
        let top = vec![&requested];
        let extra = vec![&pulled_in];
        let mut deps = HashMap::new();
        deps.insert(
            "openconnect".to_string(),
            HashSet::from(["gnutls".to_string()]),
        );
        let out = group_by_parent(top, extra, &deps, 1);
        assert_eq!(out, vec![(&requested, 0), (&pulled_in, 1)]);
    }

    #[test]
    fn unattributed_dependency_still_shown_at_depth_one() {
        // pacman resolved it (e.g. via a provides), but it isn't in
        // anyone's literal "Depends On" -- still real info, not dropped.
        let requested = pkg("openconnect", "N");
        let mystery = pkg("some-provider", "N");
        let out = group_by_parent(vec![&requested], vec![&mystery], &HashMap::new(), 1);
        assert_eq!(out, vec![(&requested, 0), (&mystery, 1)]);
    }

    #[test]
    fn a_dependency_is_never_placed_twice() {
        // Two requested packages both declare the same dependency --
        // it should nest under the first one only.
        let a = pkg("a", "N");
        let b = pkg("b", "N");
        let shared = pkg("shared-lib", "N");
        let mut deps = HashMap::new();
        deps.insert("a".to_string(), HashSet::from(["shared-lib".to_string()]));
        deps.insert("b".to_string(), HashSet::from(["shared-lib".to_string()]));
        let out = group_by_parent(vec![&a, &b], vec![&shared], &deps, 1);
        assert_eq!(out, vec![(&a, 0), (&shared, 1), (&b, 0)]);
    }

    #[test]
    fn shallow_stops_at_one_level() {
        // Without --deep: lib-a nests under app (direct dep), but
        // lib-b (a dependency of lib-a, not of app) isn't reachable at
        // depth 1 -- it still shows, just without a specific parent.
        let app = pkg("app", "N");
        let lib_a = pkg("lib-a", "N");
        let lib_b = pkg("lib-b", "N");
        let mut deps = HashMap::new();
        deps.insert("app".to_string(), HashSet::from(["lib-a".to_string()]));
        deps.insert("lib-a".to_string(), HashSet::from(["lib-b".to_string()]));
        let out = group_by_parent(vec![&app], vec![&lib_a, &lib_b], &deps, 1);
        assert_eq!(out, vec![(&app, 0), (&lib_a, 1), (&lib_b, 1)]);
    }

    #[test]
    fn deep_nests_through_every_level() {
        // With --deep (max_depth = usize::MAX): the same chain nests
        // lib-b under lib-a, at depth 2, instead of dropping to the
        // no-parent bucket.
        let app = pkg("app", "N");
        let lib_a = pkg("lib-a", "N");
        let lib_b = pkg("lib-b", "N");
        let mut deps = HashMap::new();
        deps.insert("app".to_string(), HashSet::from(["lib-a".to_string()]));
        deps.insert("lib-a".to_string(), HashSet::from(["lib-b".to_string()]));
        let out = group_by_parent(vec![&app], vec![&lib_a, &lib_b], &deps, usize::MAX);
        assert_eq!(out, vec![(&app, 0), (&lib_a, 1), (&lib_b, 2)]);
    }

    #[test]
    fn deep_handles_a_dependency_cycle_without_looping() {
        // Data shouldn't be able to happen in practice (pacman wouldn't
        // let a installed this way), but the recursion must not hang
        // if a's deps somehow name b and b's deps somehow name a back.
        let app = pkg("app", "N");
        let a = pkg("a", "N");
        let b = pkg("b", "N");
        let mut deps = HashMap::new();
        deps.insert("app".to_string(), HashSet::from(["a".to_string()]));
        deps.insert("a".to_string(), HashSet::from(["b".to_string()]));
        deps.insert("b".to_string(), HashSet::from(["a".to_string()]));
        let out = group_by_parent(vec![&app], vec![&a, &b], &deps, usize::MAX);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], (&app, 0));
    }
}
