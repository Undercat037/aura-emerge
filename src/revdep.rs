//! `--revdep-rebuild`: find installed binaries whose shared-library
//! dependencies no longer resolve, and fix them.
//!
//! Different from `@preserved-rebuild`, which only checks *declared*
//! deps via `pacman -T`. This reads the ELF files directly: after an
//! `icu`/`openssl`/`boost` soname bump, a package can satisfy every
//! declared dependency while its binaries link against a `.so` that no
//! longer exists. Gentoo hides this behind preserved-libs; here it's
//! "rebuild it once you notice", automated.
//!
//! How it works:
//!   1. `pacman -Qlq`, narrowed to directories binaries/libs live in.
//!   2. Each file's ELF header is parsed directly (`DT_NEEDED`,
//!      `DT_RPATH`/`DT_RUNPATH`) instead of shelling out to `ldd` --
//!      faster on a whole-system sweep, and doesn't hand an untrusted
//!      binary to the loader.
//!   3. A `DT_NEEDED` that resolves to nothing in the library search
//!      path (`/etc/ld.so.conf*` + `$ORIGIN`-expanded runpath) is broken.
//!   4. Broken files are mapped to packages via one batched `pacman -Qo`.
//!   5. Foreign (AUR/local) packages get rebuilt normally; repo
//!      packages can't be rebuilt on a binary distro, so the missing
//!      sonames are looked up in the file database (`pacman -F`) and
//!      their providers offered for install instead.
//!
//! Known limits: wrong-arch libraries at the right path count as
//! present, `dlopen()`ed plugins are invisible, and files replaced
//! outside pacman are judged on what's on disk now.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use colored::Colorize;

use crate::*;

/// Directory prefixes worth scanning; everything else is data.
const SCAN_PREFIXES: &[&str] = &[
    "/usr/bin/",
    "/usr/lib/",
    "/usr/lib32/",
    "/usr/libexec/",
    "/usr/local/lib/",
    "/opt/",
];

/// Search directories every dynamic linker has without being told.
const DEFAULT_LIB_DIRS: &[&str] = &["/usr/lib", "/usr/lib32", "/lib", "/lib32", "/usr/local/lib"];

/// Skip anything absurdly large before reading it into memory.
const MAX_ELF_BYTES: u64 = 256 * 1024 * 1024;

// ── minimal ELF reader ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct ElfDyn {
    needed: Vec<String>,
    /// `DT_RUNPATH`/`DT_RPATH`, `$ORIGIN` already expanded.
    runpath: Vec<PathBuf>,
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

fn is_elf(head: &[u8]) -> bool {
    head.len() >= 4 && head[0] == 0x7f && &head[1..4] == b"ELF"
}

/// C string at `off` in the string table slice.
fn cstr_at(strtab: &[u8], off: usize) -> Option<String> {
    let rest = strtab.get(off..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    String::from_utf8(rest[..end].to_vec()).ok()
}

/// Parses just enough of an ELF file to know what it links against.
/// `None` for anything that isn't a little-endian dynamic ELF.
fn read_elf_dyn(path: &Path, data: &[u8]) -> Option<ElfDyn> {
    if !is_elf(data) {
        return None;
    }
    let is_64 = match data.get(4)? {
        1 => false,
        2 => true,
        _ => return None,
    };
    if *data.get(5)? != 1 {
        return None; // not little-endian
    }
    let e_type = u16_at(data, 16)?;
    if e_type != 2 && e_type != 3 {
        return None; // not ET_EXEC / ET_DYN
    }

    let (phoff, phentsize, phnum) = if is_64 {
        (u64_at(data, 0x20)? as usize, u16_at(data, 0x36)? as usize, u16_at(data, 0x38)? as usize)
    } else {
        (u32_at(data, 0x1C)? as usize, u16_at(data, 0x2A)? as usize, u16_at(data, 0x2C)? as usize)
    };
    if phentsize == 0 || phnum == 0 {
        return None;
    }

    // PT_LOAD segments, for turning a vaddr into a file offset.
    let mut loads: Vec<(u64, u64, u64)> = Vec::new(); // (vaddr, filesz, offset)
    let mut dynamic: Option<(usize, usize)> = None; // (offset, filesz)

    for i in 0..phnum {
        let base = phoff.checked_add(i.checked_mul(phentsize)?)?;
        let p_type = u32_at(data, base)?;
        let (p_offset, p_vaddr, p_filesz) = if is_64 {
            (u64_at(data, base + 8)?, u64_at(data, base + 16)?, u64_at(data, base + 32)?)
        } else {
            (
                u32_at(data, base + 4)? as u64,
                u32_at(data, base + 8)? as u64,
                u32_at(data, base + 16)? as u64,
            )
        };
        match p_type {
            1 => loads.push((p_vaddr, p_filesz, p_offset)),
            2 => dynamic = Some((p_offset as usize, p_filesz as usize)),
            _ => {}
        }
    }

    let (dyn_off, dyn_size) = dynamic?; // no PT_DYNAMIC: statically linked
    let vaddr_to_off = |vaddr: u64| -> Option<usize> {
        loads
            .iter()
            .find(|(v, sz, _)| vaddr >= *v && vaddr < v.saturating_add(*sz))
            .map(|(v, _, off)| (off + (vaddr - v)) as usize)
    };

    let entry_size = if is_64 { 16 } else { 8 };
    let mut needed_offsets: Vec<usize> = Vec::new();
    let mut runpath_offsets: Vec<usize> = Vec::new();
    let mut strtab_vaddr: Option<u64> = None;
    let mut strsz: usize = 0;

    let mut off = dyn_off;
    let end = dyn_off.saturating_add(dyn_size).min(data.len());
    while off + entry_size <= end {
        let (tag, val) = if is_64 {
            (u64_at(data, off)? as i64, u64_at(data, off + 8)?)
        } else {
            (u32_at(data, off)? as i32 as i64, u32_at(data, off + 4)? as u64)
        };
        match tag {
            0 => break,                                        // DT_NULL
            1 => needed_offsets.push(val as usize),             // DT_NEEDED
            5 => strtab_vaddr = Some(val),                      // DT_STRTAB
            10 => strsz = val as usize,                         // DT_STRSZ
            15 | 29 => runpath_offsets.push(val as usize),      // DT_RPATH / DT_RUNPATH
            _ => {}
        }
        off += entry_size;
    }

    if needed_offsets.is_empty() && runpath_offsets.is_empty() {
        return Some(ElfDyn { needed: Vec::new(), runpath: Vec::new() });
    }

    let strtab_off = vaddr_to_off(strtab_vaddr?)?;
    let strtab_end = if strsz > 0 {
        strtab_off.saturating_add(strsz).min(data.len())
    } else {
        data.len()
    };
    let strtab = data.get(strtab_off..strtab_end)?;

    let needed: Vec<String> = needed_offsets.iter().filter_map(|o| cstr_at(strtab, *o)).collect();

    let origin = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/"));
    let mut runpath = Vec::new();
    for o in runpath_offsets {
        let Some(raw) = cstr_at(strtab, o) else { continue };
        for part in raw.split(':').filter(|p| !p.is_empty()) {
            let expanded = part
                .replace("${ORIGIN}", &origin.to_string_lossy())
                .replace("$ORIGIN", &origin.to_string_lossy());
            runpath.push(PathBuf::from(expanded));
        }
    }

    Some(ElfDyn { needed, runpath })
}

// ── library search path ───────────────────────────────────────────────────────

/// Directories the dynamic linker searches, from `/etc/ld.so.conf` and
/// its includes plus the built-in defaults.
fn linker_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = DEFAULT_LIB_DIRS.iter().map(PathBuf::from).collect();

    fn read_conf(path: &Path, dirs: &mut Vec<PathBuf>, depth: usize) {
        if depth > 4 {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else { return };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("include") {
                let pattern = rest.trim();
                // Only the "dir/*.conf" shape ld.so.conf actually uses.
                if let Some((dir, suffix)) = pattern.rsplit_once('/') {
                    let Ok(entries) = std::fs::read_dir(dir) else { continue };
                    let want_ext = suffix.trim_start_matches('*');
                    let mut paths: Vec<PathBuf> = entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.to_string_lossy().ends_with(want_ext))
                        .collect();
                    paths.sort();
                    for p in paths {
                        read_conf(&p, dirs, depth + 1);
                    }
                }
                continue;
            }
            if line.starts_with('/') {
                dirs.push(PathBuf::from(line));
            }
        }
    }

    read_conf(Path::new("/etc/ld.so.conf"), &mut dirs, 0);
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Filenames present in each search directory. Built once so the
/// per-`DT_NEEDED` check is a hash lookup rather than a `stat`.
fn library_index(dirs: &[PathBuf]) -> HashMap<String, Vec<PathBuf>> {
    let mut index: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            index.entry(name).or_default().push(e.path());
        }
    }
    index
}

/// Whether a `DT_NEEDED` name resolves to something that exists.
/// `Path::exists()` follows symlinks, so a dangling link reads as
/// missing -- exactly the case being looked for.
fn resolves(name: &str, index: &HashMap<String, Vec<PathBuf>>, runpath: &[PathBuf]) -> bool {
    if name.contains('/') {
        return Path::new(name).exists();
    }
    for dir in runpath {
        if dir.join(name).exists() {
            return true;
        }
    }
    index
        .get(name)
        .map(|paths| paths.iter().any(|p| p.exists()))
        .unwrap_or(false)
}

// ── scanning ──────────────────────────────────────────────────────────────────

/// Every path pacman owns, narrowed to `SCAN_PREFIXES`.
fn owned_paths() -> Vec<String> {
    let out = Command::new(PACMAN_BIN)
        .arg("-Qlq")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with('/'))
        .filter(|l| SCAN_PREFIXES.iter().any(|p| l.starts_with(p)))
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// One file with at least one unresolvable `DT_NEEDED`.
#[derive(Debug)]
pub(crate) struct BrokenFile {
    pub(crate) path: String,
    pub(crate) missing: Vec<String>,
}

fn scan_broken() -> Vec<BrokenFile> {
    let dirs = linker_dirs();
    let index = library_index(&dirs);
    let paths = owned_paths();

    println!(
        "{} Scanning {} installed file(s) against {} library director(ies)...",
        ">>>".green().bold(),
        paths.len(),
        dirs.len()
    );

    let mut broken = Vec::new();
    for p in &paths {
        let path = Path::new(p);
        // Symlinks point at a real file that's scanned on its own turn.
        let Ok(meta) = std::fs::symlink_metadata(path) else { continue };
        if !meta.is_file() || meta.len() < 64 || meta.len() > MAX_ELF_BYTES {
            continue;
        }
        let Ok(data) = std::fs::read(path) else { continue };
        let Some(info) = read_elf_dyn(path, &data) else { continue };
        if info.needed.is_empty() {
            continue;
        }
        let missing: Vec<String> = info
            .needed
            .iter()
            .filter(|n| !resolves(n, &index, &info.runpath))
            .cloned()
            .collect();
        if !missing.is_empty() {
            broken.push(BrokenFile { path: p.clone(), missing });
        }
    }
    broken
}

/// `pacman -Qo` for a batch of files, chunked for very long lists.
fn owners_of(files: &[String]) -> HashMap<String, String> {
    let mut owners = HashMap::new();
    for chunk in files.chunks(256) {
        let out = Command::new(PACMAN_BIN)
            .arg("-Qo")
            .args(chunk)
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let Ok(out) = out else { continue };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            // "/usr/bin/foo is owned by bar 1.2.3-1"
            let Some((path, rest)) = line.split_once(" is owned by ") else { continue };
            let Some(pkg) = rest.split_whitespace().next() else { continue };
            owners.insert(path.trim().to_string(), pkg.to_string());
        }
    }
    owners
}

/// Foreign packages (`pacman -Qm`): AUR, ABS, anything built locally.
fn foreign_packages() -> HashSet<String> {
    let out = Command::new(PACMAN_BIN)
        .arg("-Qmq")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => HashSet::new(),
    }
}

/// Packages that provide a missing soname, via the file database.
fn providers_of(soname: &str) -> Vec<String> {
    let out = Command::new(PACMAN_BIN)
        .args(["-Fq", soname])
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        // -Fq prints "repo/name"; world.set and pacman -S both take the
        // bare name, and the repo is visible in the report anyway.
        .map(|l| l.split('/').last().unwrap_or(l).to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

// ── the action ────────────────────────────────────────────────────────────────

/// `--revdep-rebuild`. Returns false if anything failed, so `run()` can
/// exit non-zero.
pub(crate) fn revdep_rebuild(
    pretend: bool,
    ask: bool,
    skippgp: bool,
    no_sandbox: bool,
    skip_srcinfo_regen: bool,
    unshare_net_build: bool,
) -> bool {
    println!(
        "{} Checking installed binaries for broken library links...",
        ">>>".green().bold()
    );

    let broken = scan_broken();
    if broken.is_empty() {
        println!();
        println!(">>> No broken dynamic links were found.");
        return true;
    }

    let files: Vec<String> = broken.iter().map(|b| b.path.clone()).collect();
    let owners = owners_of(&files);

    // package -> (files, missing sonames)
    let mut by_pkg: BTreeMap<String, (Vec<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut orphan_files: Vec<&BrokenFile> = Vec::new();
    for b in &broken {
        match owners.get(&b.path) {
            Some(pkg) => {
                let entry = by_pkg.entry(pkg.clone()).or_default();
                entry.0.push(b.path.clone());
                entry.1.extend(b.missing.iter().cloned());
            }
            None => orphan_files.push(b),
        }
    }

    println!();
    println!("{}", "These packages have binaries linking against missing libraries:".yellow().bold());
    println!();
    for (pkg, (files, missing)) in &by_pkg {
        println!(
            "[{} {:<4}] {} ({} file(s))",
            "ebuild".green(),
            "R".cyan().bold(),
            pkg.yellow().bold(),
            files.len()
        );
        for so in missing {
            println!("      missing: {}", so.red());
        }
        for f in files.iter().take(3) {
            println!("      {}", f.dimmed());
        }
        if files.len() > 3 {
            println!("      {}", format!("... and {} more", files.len() - 3).dimmed());
        }
    }
    if !orphan_files.is_empty() {
        println!();
        println!(
            "{} {} broken file(s) belong to no installed package (left alone):",
            " *".yellow().bold(),
            orphan_files.len()
        );
        for b in orphan_files.iter().take(10) {
            println!("     {} ({})", b.path, b.missing.join(", "));
        }
    }

    // --exclude / mask still apply here too.
    let all_pkgs: Vec<String> = by_pkg.keys().cloned().collect();
    let (candidates, excluded) = crate::runtime::split_excluded(&all_pkgs);
    crate::runtime::report_excluded(&excluded);
    let (candidates, masked) = crate::mask::split_masked(&candidates, None);
    crate::mask::report_blocked(&masked);

    let foreign = foreign_packages();
    let (to_rebuild, repo_pkgs): (Vec<String>, Vec<String>) =
        candidates.into_iter().partition(|p| foreign.contains(p));

    // Repo packages: fix is whatever ships the lost soname.
    let mut missing_sonames: BTreeSet<String> = BTreeSet::new();
    for pkg in &repo_pkgs {
        if let Some((_, missing)) = by_pkg.get(pkg) {
            missing_sonames.extend(missing.iter().cloned());
        }
    }
    let mut providers: BTreeSet<String> = BTreeSet::new();
    let mut unprovided: Vec<String> = Vec::new();
    for so in &missing_sonames {
        let found = providers_of(so);
        if found.is_empty() {
            unprovided.push(so.clone());
        } else {
            providers.extend(found);
        }
    }
    let installed_now: HashSet<String> = Command::new(PACMAN_BIN)
        .arg("-Qq")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let mut to_install: Vec<String> = providers
        .into_iter()
        .filter(|p| !installed_now.contains(p))
        .collect();
    let (kept_installs, excluded_installs) = crate::runtime::split_excluded(&to_install);
    crate::runtime::report_excluded(&excluded_installs);
    let (kept_installs, masked_installs) = crate::mask::split_masked(&kept_installs, None);
    crate::mask::report_blocked(&masked_installs);
    to_install = kept_installs;

    println!();
    if !to_rebuild.is_empty() {
        println!(
            "{} {} AUR/local package(s) to rebuild: {}",
            ">>>".green().bold(),
            to_rebuild.len(),
            to_rebuild.join(", ")
        );
    }
    if !to_install.is_empty() {
        println!(
            "{} {} package(s) provide the missing libraries and are not installed: {}",
            ">>>".green().bold(),
            to_install.len(),
            to_install.join(", ")
        );
    }
    if !unprovided.is_empty() {
        println!(
            "{} no package in the file database provides: {}",
            " *".yellow().bold(),
            unprovided.join(", ")
        );
        println!(
            "     run `{}` if the file database is stale, or check whether these were removed upstream.",
            "emerge --regen".cyan()
        );
    }
    if !repo_pkgs.is_empty() && to_install.is_empty() && unprovided.is_empty() {
        println!(
            "{} {} official-repo package(s) are affected but the libraries they want are already installed - check for a mixed 32/64-bit case.",
            " *".yellow().bold(),
            repo_pkgs.len()
        );
    }

    if to_rebuild.is_empty() && to_install.is_empty() {
        println!();
        println!(">>> Nothing to do automatically.");
        return true;
    }

    if pretend {
        return true;
    }

    println!();
    print!(
        "{} Rebuild/install the package(s) above? [y/N] ",
        ">>>".yellow().bold()
    );
    std::io::stdout().flush().ok();
    let answer = read_line_raw();
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!(">>> Aborted.");
        return true;
    }

    let mut ok = true;

    if !to_install.is_empty() {
        let mut args: Vec<&str> = vec![PACMAN_BIN, "-S", "--needed", "--asdeps"];
        if !ask {
            args.push("--noconfirm");
        }
        if !run_cmd(SUDO_BIN, &args, &to_install) {
            eprintln!(
                "{} failed to install the library package(s)",
                ">>> Error:".red().bold()
            );
            for p in &to_install {
                crate::runtime::record_failure(p, "library provider install failed");
            }
            ok = false;
            if !crate::runtime::keep_going() {
                return false;
            }
        }
    }

    if !to_rebuild.is_empty() {
        println!(
            "{} Rebuilding {} AUR/local package(s)...",
            ">>>".green().bold(),
            to_rebuild.len()
        );
        scan_aur_pkgbuilds_or_abort(&to_rebuild);
        if !aur_install(
            &to_rebuild,
            false,
            ask,
            false,
            skippgp,
            false,
            no_sandbox,
            skip_srcinfo_regen,
            unshare_net_build,
            false,
        ) {
            ok = false;
        }
    }

    ok
}