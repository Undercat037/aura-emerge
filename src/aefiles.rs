//! Files AE Force leaves next to a package file:
//! `<file>.aenew` (new version), `<file>.aeforce` (the old one),
//! `<file>.aepark` (left by an interrupted install); `.<n>` when taken.
//! Found by looking beside every file an installed package owns, so
//! nothing has to be remembered between runs.

use crate::helper::validate::{AE_EXTS, AE_FORCE_EXT, AE_NEW_EXT};
use crate::theme::Themed;
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::process::Command;

pub(crate) struct Left {
    pub(crate) path: String,
    /// "new" | "old" | "park"
    pub(crate) kind: &'static str,
}

fn kind_of(ext: &str) -> &'static str {
    match ext {
        AE_NEW_EXT => "new",
        AE_FORCE_EXT => "old",
        _ => "park",
    }
}

/// `name.aenew` / `name.aenew.3` -> (`name`, `.aenew`).
fn split_ae(name: &str) -> Option<(&str, &'static str)> {
    for ext in AE_EXTS {
        if let Some(stem) = name.strip_suffix(ext) {
            return Some((stem, ext));
        }
        if let Some((head, n)) = name.rsplit_once('.') {
            if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
                if let Some(stem) = head.strip_suffix(ext) {
                    return Some((stem, ext));
                }
            }
        }
    }
    None
}

pub(crate) fn scan() -> Vec<Left> {
    let owned: HashSet<String> = crate::alpm_db::owned_paths(|_| true).into_iter().collect();
    let dirs: HashSet<&str> = owned
        .iter()
        .filter_map(|p| p.rsplit_once('/').map(|(d, _)| d))
        .collect();
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(if d.is_empty() { "/" } else { d }) else {
            continue;
        };
        for e in rd.flatten() {
            let Some(name) = e.file_name().to_str().map(String::from) else {
                continue;
            };
            let Some((stem, ext)) = split_ae(&name) else {
                continue;
            };
            if !owned.contains(&format!("{}/{}", d, stem)) {
                continue;
            }
            if e.file_type().map_or(true, |t| t.is_dir()) {
                continue;
            }
            out.push(Left {
                path: format!("{}/{}", d, name),
                kind: kind_of(ext),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn tag(kind: &str) -> String {
    match kind {
        "new" => format!("[{}  ]", "new".t_green().bold()),
        "old" => format!("[{}  ]", "old".t_yellow().bold()),
        _ => format!("[{}]", "park".t_red().bold()),
    }
}

fn show(items: &[&Left]) {
    for l in items {
        println!("{} {}", tag(l.kind), l.path);
    }
}

pub(crate) fn list() {
    let all = scan();
    if all.is_empty() {
        println!("{} No AE Force files found.", ">>>".t_green().bold());
        return;
    }
    show(&all.iter().collect::<Vec<_>>());
    println!();
    println!("{}: {} file(s)", "Total".bold(), all.len());
    println!(" new  = new version kept aside, old = your previous file,");
    println!(" park = left by an interrupted install.");
    println!(" Remove them with: emerge --aeclean [new|old|park|all]");
}

/// Removes the chosen kind after a question (Enter = No).
pub(crate) fn clean(kind: &str) -> bool {
    if !matches!(kind, "new" | "old" | "park" | "all") {
        eprintln!(
            "{} --aeclean takes new, old, park or all.",
            ">>> Error:".t_red().bold()
        );
        return false;
    }
    let all = scan();
    let pick: Vec<&Left> = all
        .iter()
        .filter(|l| kind == "all" || l.kind == kind)
        .collect();
    if pick.is_empty() {
        println!("{} No AE Force files to remove.", ">>>".t_green().bold());
        return true;
    }
    show(&pick);
    print!(
        "\n{} Remove {} file(s)? [{}/{}] ",
        ">>>".t_yellow().bold(),
        pick.len(),
        "Yes".t_green().bold(),
        "No".t_red().bold()
    );
    let _ = io::stdout().flush();
    let ans = crate::read_line_raw();
    if !matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("{} Quitting.", ">>>".t_yellow().bold());
        return false;
    }
    let paths: Vec<&str> = pick
        .iter()
        .map(|l| l.path.as_str())
        .filter(|p| fs::symlink_metadata(p).map_or(false, |m| !m.is_dir()))
        .collect();
    let mut ok = true;
    for chunk in paths.chunks(200) {
        let st = Command::new(crate::SUDO_BIN)
            .args([crate::RM_BIN, "-f", "--"])
            .args(chunk)
            .status();
        ok &= matches!(st, Ok(s) if s.success());
    }
    if ok {
        println!(
            "{} Removed {} file(s).",
            ">>>".t_green().bold(),
            paths.len()
        );
    } else {
        eprintln!("{} rm failed for some files.", ">>> Error:".t_red().bold());
    }
    ok
}
