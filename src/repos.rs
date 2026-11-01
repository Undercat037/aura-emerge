//! Portage-style repos.conf — frontend view (`--info`).
//! Parsing and libalpm registration: `helper::reposconf`.

pub(crate) use crate::helper::reposconf::{
    GENERATED_PACMAN_FRAGMENT, REPOS_CONF, apply_to_alpm, load_effective, read_mirrorlist,
    write_generated_fragment,
};

use std::path::Path;

/// Print the repos.conf view for `emerge --info`.
pub(crate) fn print_info(localrepo: bool) {
    let cfg = load_effective(localrepo);
    println!("repos.conf: {}", REPOS_CONF);
    if !Path::new(REPOS_CONF).exists() {
        println!("    (not present — using pacman.conf only unless FEATURES=repos)");
        return;
    }
    for w in &cfg.warnings {
        println!("    warning: {}", w);
    }
    if cfg.entries.is_empty() {
        println!("    (no repository sections)");
        return;
    }
    println!();
    for (i, e) in cfg.entries.iter().enumerate() {
        println!("{}", e.name);
        println!(
            "    priority: {} (effective order {}, from {}:{})",
            e.priority,
            i + 1,
            e.source,
            e.line
        );
        if let Some(ref sig) = e.verify_signature {
            println!("    SigLevel = {}", sig);
        }
        if let Some(ref loc) = e.location {
            println!("    location = {}", loc);
        }
        for u in &e.sync_uris {
            println!("    Server = {}", u);
        }
        for inc in &e.includes {
            println!("    Include = {}", inc.display());
            match read_mirrorlist(inc) {
                Ok(urls) => {
                    for (j, u) in urls.iter().enumerate() {
                        if j < 3 {
                            println!("        Server = {}", u);
                        } else if j == 3 {
                            println!("        … {} more from mirrorlist", urls.len() - 3);
                            break;
                        }
                    }
                    if urls.is_empty() {
                        println!("        (mirrorlist empty or unreadable servers)");
                    }
                }
                Err(err) => println!("        ({} )", err),
            }
        }
        println!();
    }
    println!(
        "    pacman fragment: {} (Include from pacman.conf)",
        GENERATED_PACMAN_FRAGMENT
    );
}
