//! Package conflicts, shown like Portage blockers:
//!
//! ```text
//! [ebuild  N    ] pipewire-jack-1:1.4.2-1
//! [conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
//! ```
//!
//! Parses the helper's line `conflicting dependencies: new [ver] vs
//! installed [ver]; ...` and prints the plan. The question is in
//! packages.rs. Removed packages are remembered for `--undo`.
//!
//! Blockers are also looked for *before* anything is installed, for every
//! source of a run (repo plan, AUR, ABS): each source registers what it
//! brings (`register_incoming`) and `take_planned` adds the conflicts the
//! incoming packages have with each other across sources.

use crate::theme::Themed;
use colored::Colorize;
use std::sync::Mutex;

/// Blocker pair: `new_*` incoming, `old_*` installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Conflict {
    pub(crate) new_name: String,
    pub(crate) new_ver: String,
    pub(crate) old_name: String,
    pub(crate) old_ver: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub(crate) pairs: Vec<Conflict>,
    /// Cut-off or malformed entries (the wire line is capped).
    pub(crate) dropped: usize,
}

/// `name [ver]` -> (name, ver). Both parts must be non-empty.
fn name_ver(s: &str) -> Option<(String, String)> {
    let s = s.trim().strip_suffix(']')?;
    let (name, ver) = s.rsplit_once(" [")?;
    let ok = |t: &str| !t.is_empty() && !t.contains(char::is_whitespace) && !t.contains(['[', ']']);
    (ok(name) && ok(ver)).then(|| (name.to_string(), ver.to_string()))
}

fn pair(s: &str) -> Option<Conflict> {
    let (l, r) = s.split_once(" vs ")?;
    let (new_name, new_ver) = name_ver(l)?;
    let (old_name, old_ver) = name_ver(r)?;
    Some(Conflict {
        new_name,
        new_ver,
        old_name,
        old_ver,
    })
}

/// Parses the helper's message (prefix optional). Repeats are folded;
/// a truncated entry is counted in `dropped`, not guessed.
pub(crate) fn parse(detail: &str) -> Parsed {
    let body = detail.trim();
    let body = body
        .strip_prefix("conflicting dependencies:")
        .unwrap_or(body);
    let mut out = Parsed::default();
    for part in body.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        match pair(part) {
            Some(c) if !out.pairs.contains(&c) => out.pairs.push(c),
            Some(_) => {}
            None => out.dropped += 1,
        }
    }
    out
}

/// An installed package that has to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unmerge {
    pub(crate) name: String,
    pub(crate) ver: String,
    /// `name-ver` of every incoming package it blocks.
    pub(crate) blocking: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Incoming packages that are part of a conflict: (name, ver).
    pub(crate) merge: Vec<(String, String)>,
    /// Installed side, removable.
    pub(crate) unmerge: Vec<Unmerge>,
    /// Both sides incoming: removing an installed package cannot help.
    pub(crate) hard: Vec<(String, String)>,
}

fn label(name: &str, ver: &str) -> String {
    format!("{}-{}", name, ver)
}

pub(crate) fn plan(pairs: &[Conflict]) -> Plan {
    plan_with(pairs, &[])
}

/// Like `plan`, but `incoming` is every name in the transaction. libalpm
/// reports each pair once, so a package that is only ever the `old` side
/// of a pair (both sides incoming) would otherwise look removable.
pub(crate) fn plan_with(pairs: &[Conflict], incoming: &[String]) -> Plan {
    let mut p = Plan::default();
    for c in pairs {
        let n = (c.new_name.clone(), c.new_ver.clone());
        if !p.merge.contains(&n) {
            p.merge.push(n);
        }
    }
    for c in pairs {
        let blocked_by = label(&c.new_name, &c.new_ver);
        // Installed side is incoming too: hard blocker.
        if p.merge.iter().any(|(n, _)| *n == c.old_name) || incoming.contains(&c.old_name) {
            let h = (
                label(&c.new_name, &c.new_ver),
                label(&c.old_name, &c.old_ver),
            );
            // `a vs b` and `b vs a` are one blocker (both declare it).
            let seen = p
                .hard
                .iter()
                .any(|(a, b)| (*a == h.0 && *b == h.1) || (*a == h.1 && *b == h.0));
            if !seen {
                p.hard.push(h);
            }
            continue;
        }
        match p.unmerge.iter_mut().find(|u| u.name == c.old_name) {
            Some(u) => {
                if !u.blocking.contains(&blocked_by) {
                    u.blocking.push(blocked_by);
                }
            }
            None => p.unmerge.push(Unmerge {
                name: c.old_name.clone(),
                ver: c.old_ver.clone(),
                blocking: vec![blocked_by],
            }),
        }
    }
    p
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Merge,
    Conflict,
    Blocks,
}

/// One list line, uncoloured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) kind: Kind,
    pub(crate) atom: String,
    pub(crate) note: Option<String>,
}

fn blocking_note(name: &str, by: &[String]) -> String {
    format!("(\"{}\" is blocking {})", name, by.join(", "))
}

pub(crate) fn rows(plan: &Plan) -> Vec<Row> {
    let mut v = Vec::new();
    for (n, ver) in &plan.merge {
        v.push(Row {
            kind: Kind::Merge,
            atom: label(n, ver),
            note: None,
        });
    }
    for u in &plan.unmerge {
        v.push(Row {
            kind: Kind::Conflict,
            atom: label(&u.name, &u.ver),
            note: Some(blocking_note(&u.name, &u.blocking)),
        });
    }
    for (a, b) in &plan.hard {
        v.push(Row {
            kind: Kind::Blocks,
            atom: b.clone(),
            note: Some(format!("(\"{}\" is blocking {})", b, a)),
        });
    }
    v
}

/// Same width as `[ebuild  N    ]`.
fn tag(kind: Kind) -> String {
    match kind {
        Kind::Merge => format!("[{}  {:<4} ]", "ebuild".t_green(), "N".t_green().bold()),
        Kind::Conflict => format!("[{}     ]", "conflict".t_red().bold()),
        Kind::Blocks => format!("[{} {:<4} ]", "blocks".t_red().bold(), "B".t_red().bold()),
    }
}

/// Prints the list and Portage's explanation under it.
/// `dropped` = entries the helper line could not carry.
pub(crate) fn print(plan: &Plan, dropped: usize) {
    println!();
    for r in rows(plan) {
        let atom = match r.kind {
            Kind::Merge => r.atom.t_green().bold().to_string(),
            Kind::Conflict | Kind::Blocks => r.atom.t_red().bold().to_string(),
        };
        match &r.note {
            Some(n) => println!("{} {} {}", tag(r.kind), atom, n),
            None => println!("{} {}", tag(r.kind), atom),
        }
    }
    if dropped > 0 {
        println!(
            "{} ... and more conflicts that did not fit ({} not shown)",
            " *".t_yellow().bold(),
            dropped
        );
    }
    println!();
    if plan.hard.is_empty() {
        println!(
            "{} The packages above cannot be installed at the same time as the",
            " *".t_yellow().bold()
        );
        println!(
            "{} installed ones marked {}; those would be unmerged first.",
            " *".t_yellow().bold(),
            "[conflict]".t_red().bold()
        );
    } else {
        eprintln!(
            "{} The above package list contains packages which cannot be",
            " * Error:".t_red().bold()
        );
        eprintln!(
            "{} installed at the same time on the same system.",
            " *".t_red().bold()
        );
    }
    println!();
}

/// Blockers found while *planning* (`plan_sync`), to be listed under the
/// ebuild list by `print_emerge_plan`: (pairs, every incoming name).
static PLANNED: Mutex<Option<(Vec<Conflict>, Vec<String>)>> = Mutex::new(None);

/// Several sources add to it (repo plan, AUR pre-flight); repeats fold.
pub(crate) fn note_planned(pairs: Vec<Conflict>, incoming: Vec<String>) {
    if pairs.is_empty() {
        return;
    }
    if let Ok(mut g) = PLANNED.lock() {
        let (p, i) = g.get_or_insert_with(|| (Vec::new(), Vec::new()));
        for c in pairs {
            if !p.contains(&c) {
                p.push(c);
            }
        }
        for n in incoming {
            if !i.contains(&n) {
                i.push(n);
            }
        }
    }
}

/// The plan list's blockers, kept for the one question asked before
/// anything is installed (`preflight_confirm`).
static PREFLIGHT: Mutex<Option<Plan>> = Mutex::new(None);
/// Installed packages the user already agreed to unmerge in that question.
static APPROVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Asked right after the plan, before any package is touched. Hard
/// blockers stop the run; removable ones need one yes (always asked --
/// installed packages go). False = stop.
pub(crate) fn preflight_confirm() -> bool {
    let Some(plan) = PREFLIGHT.lock().ok().and_then(|mut g| g.take()) else {
        return true;
    };
    if !plan.hard.is_empty() {
        eprintln!("{} Nothing was installed.", ">>> Error:".t_red().bold());
        std::process::exit(1);
    }
    if plan.unmerge.is_empty() {
        return true;
    }
    if !crate::packages::confirm_conflict_unmerge(true) {
        return false;
    }
    if let Ok(mut a) = APPROVED.lock() {
        a.extend(plan.unmerge.iter().map(|u| u.name.clone()));
    }
    true
}

/// True if every name was already approved by `preflight_confirm`.
pub(crate) fn is_approved(names: &[String]) -> bool {
    APPROVED
        .lock()
        .map(|a| !names.is_empty() && names.iter().all(|n| a.contains(n)))
        .unwrap_or(false)
}

/// Everything this run is about to bring, from every source (repo plan,
/// AUR, ABS), so blockers *between* sources show up in the plan too.
static REGISTRY: Mutex<Vec<crate::alpm_db::Incoming>> = Mutex::new(Vec::new());

/// Adds to the registry; the same name + version is kept once.
pub(crate) fn register_incoming(items: Vec<crate::alpm_db::Incoming>) {
    if let Ok(mut g) = REGISTRY.lock() {
        for it in items {
            if !g.iter().any(|x| x.name == it.name && x.ver == it.ver) {
                g.push(it);
            }
        }
    }
}

/// Folds the cross-source conflicts into what the sources noted. Pure,
/// so it is testable without libalpm.
fn merge_cross(
    taken: Option<(Vec<Conflict>, Vec<String>)>,
    cross: Vec<Conflict>,
    names: Vec<String>,
) -> Option<(Vec<Conflict>, Vec<String>)> {
    match taken {
        Some((mut pairs, mut inc)) => {
            for c in cross {
                if !pairs.contains(&c) {
                    pairs.push(c);
                }
            }
            for n in names {
                if !inc.contains(&n) {
                    inc.push(n);
                }
            }
            Some((pairs, inc))
        }
        None if !cross.is_empty() => Some((cross, names)),
        None => None,
    }
}

pub(crate) fn take_planned() -> Option<(Vec<Conflict>, Vec<String>)> {
    let taken = PLANNED.lock().ok().and_then(|mut g| g.take());
    let reg: Vec<crate::alpm_db::Incoming> = REGISTRY
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    // One source alone was already checked against the system.
    let cross = if reg.len() > 1 {
        crate::alpm_db::incoming_conflicts(&reg)
    } else {
        Vec::new()
    };
    merge_cross(taken, cross, reg.into_iter().map(|r| r.name).collect())
}

/// The `[conflict]` / `[blocks]` lines for the plan list (the ebuild
/// lines are already printed by the caller), then the explanation.
pub(crate) fn print_planned(
    pairs: &[Conflict],
    incoming: &[String],
    needed_by: &dyn Fn(&str) -> Vec<String>,
) {
    let plan = plan_with(pairs, incoming);
    if let Ok(mut g) = PREFLIGHT.lock() {
        *g = Some(plan.clone());
    }
    for r in rows(&plan).into_iter().filter(|r| r.kind != Kind::Merge) {
        let atom = r.atom.t_red().bold().to_string();
        match &r.note {
            Some(n) => println!("{} {} {}", tag(r.kind), atom, n),
            None => println!("{} {}", tag(r.kind), atom),
        }
    }
    print_origins(&plan);
    println!();
    if plan.hard.is_empty() {
        println!(
            "{} Packages marked {} are installed and block the ones above;",
            " *".t_yellow().bold(),
            "[conflict]".t_red().bold()
        );
        println!(
            "{} they are unmerged first if you go on.",
            " *".t_yellow().bold()
        );
        println!();
        print_dependents(&plan, needed_by);
    } else {
        println!(
            "{} Some of the packages above cannot be installed at the same time.",
            " * Error:".t_red().bold()
        );
    }
}

/// Which set / world line lists each package in a conflict, so it is
/// clear what to edit if the other side should win.
fn print_origins(plan: &Plan) {
    let mut names: Vec<(String, &str)> = Vec::new();
    for (n, _) in &plan.merge {
        names.push((n.clone(), "to be merged"));
    }
    for u in &plan.unmerge {
        names.push((u.name.clone(), "installed, would be unmerged"));
    }
    for (n, what) in names {
        let from = crate::world_set::origins_of(&n);
        if !from.is_empty() {
            println!(
                "{} {} ({}) is listed in: {}",
                " *".t_yellow().bold(),
                n,
                what,
                from.join(", ")
            );
        }
    }
}

/// What still needs the packages that are about to be unmerged. They go
/// with `-Rdd`, so these stay installed but unsatisfied.
pub(crate) fn print_dependents(plan: &Plan, needed_by: &dyn Fn(&str) -> Vec<String>) {
    let mut any = false;
    for u in &plan.unmerge {
        let deps: Vec<String> = needed_by(&u.name)
            .into_iter()
            .filter(|d| !plan.unmerge.iter().any(|x| x.name == *d))
            .collect();
        if deps.is_empty() {
            continue;
        }
        any = true;
        println!(
            "{} {} is required by: {}",
            " *".t_yellow().bold(),
            u.name.t_red().bold(),
            deps.join(", ")
        );
    }
    if any {
        println!(
            "{} Unmerge ignores dependencies (-Rdd): those stay installed, broken.",
            " *".t_yellow().bold()
        );
        println!();
    }
}

/// Installed packages removed to resolve conflicts in this run:
/// (name, was in world). Saved with the install for `--undo`.
static REMOVED: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

pub(crate) fn record_removed(items: &[(String, bool)]) {
    if let Ok(mut v) = REMOVED.lock() {
        for it in items {
            if !v.iter().any(|(n, _)| *n == it.0) {
                v.push(it.clone());
            }
        }
    }
}

pub(crate) fn take_removed() -> Vec<(String, bool)> {
    REMOVED
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(nn: &str, nv: &str, on: &str, ov: &str) -> Conflict {
        Conflict {
            new_name: nn.into(),
            new_ver: nv.into(),
            old_name: on.into(),
            old_ver: ov.into(),
        }
    }

    #[test]
    fn parses_helper_line() {
        let p = parse(
            "conflicting dependencies: pipewire-jack [1:1.4.2-1] vs jack2 [1.9.22-1]; \
             pipewire-jack [1:1.4.2-1] vs jack [1.9-1]",
        );
        assert_eq!(p.dropped, 0);
        assert_eq!(
            p.pairs,
            vec![
                c("pipewire-jack", "1:1.4.2-1", "jack2", "1.9.22-1"),
                c("pipewire-jack", "1:1.4.2-1", "jack", "1.9-1"),
            ]
        );
    }

    #[test]
    fn prefix_is_optional_and_repeats_fold() {
        let p = parse("a [1-1] vs b [2-1]; a [1-1] vs b [2-1]");
        assert_eq!(p.pairs, vec![c("a", "1-1", "b", "2-1")]);
        assert_eq!(p.dropped, 0);
    }

    #[test]
    fn truncated_tail_is_counted_not_guessed() {
        let p = parse("conflicting dependencies: a [1-1] vs b [2-1]; c [3-1] vs d [4");
        assert_eq!(p.pairs, vec![c("a", "1-1", "b", "2-1")]);
        assert_eq!(p.dropped, 1);
        assert_eq!(parse("conflicting dependencies:"), Parsed::default());
        assert_eq!(parse("unsatisfied: x needs y").dropped, 1);
    }

    #[test]
    fn plan_groups_by_installed_package() {
        let pairs = vec![
            c("pipewire-jack", "1-1", "jack2", "1.9-1"),
            c("other", "2-1", "jack2", "1.9-1"),
            c("pipewire-jack", "1-1", "jack", "0.1-1"),
        ];
        let p = plan(&pairs);
        assert_eq!(
            p.merge,
            vec![
                ("pipewire-jack".to_string(), "1-1".to_string()),
                ("other".to_string(), "2-1".to_string())
            ]
        );
        assert_eq!(p.unmerge.len(), 2);
        assert_eq!(p.unmerge[0].name, "jack2");
        assert_eq!(
            p.unmerge[0].blocking,
            vec!["pipewire-jack-1-1", "other-2-1"]
        );
        assert!(p.hard.is_empty());
    }

    #[test]
    fn both_sides_incoming_is_a_hard_blocker() {
        let p = plan(&[c("a", "1-1", "b", "1-1"), c("b", "1-1", "x", "9-1")]);
        assert_eq!(p.hard, vec![("a-1-1".to_string(), "b-1-1".to_string())]);
        assert_eq!(p.unmerge.len(), 1);
        assert_eq!(p.unmerge[0].name, "x");
    }

    #[test]
    fn both_directions_fold_into_one_hard_blocker() {
        // a conflicts b and b conflicts a: libalpm-style pairs both ways
        let pairs = [c("a", "1-1", "b", "1-1"), c("b", "1-1", "a", "1-1")];
        let p = plan_with(&pairs, &["a".to_string(), "b".to_string()]);
        assert_eq!(p.hard.len(), 1);
        assert!(p.unmerge.is_empty());
    }

    #[test]
    fn cross_source_conflict_without_prior_notes() {
        let cross = vec![c("jack2-git", "2-1", "pipewire-jack", "1-1")];
        let names = vec!["jack2-git".to_string(), "pipewire-jack".to_string()];
        let (pairs, inc) = merge_cross(None, cross.clone(), names.clone()).unwrap();
        assert_eq!(pairs, cross);
        // both sides are incoming: the plan must call it a hard blocker
        assert_eq!(plan_with(&pairs, &inc).hard.len(), 1);
    }

    #[test]
    fn cross_source_folds_into_noted_pairs_once() {
        let noted = c("a", "1-1", "x", "9-1");
        let cross = vec![noted.clone(), c("a", "1-1", "b", "1-1")];
        let (pairs, inc) = merge_cross(
            Some((vec![noted.clone()], vec!["a".to_string()])),
            cross,
            vec!["a".to_string(), "b".to_string()],
        )
        .unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(inc, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn nothing_noted_and_no_cross_is_none() {
        assert!(merge_cross(None, Vec::new(), vec!["a".to_string()]).is_none());
    }

    #[test]
    fn incoming_old_side_is_a_hard_blocker() {
        // libalpm reports the pair once: b is only ever the `old` side.
        let pairs = [c("a", "1-1", "b", "1-1")];
        assert!(plan(&pairs).hard.is_empty());
        let p = plan_with(&pairs, &["a".to_string(), "b".to_string()]);
        assert_eq!(p.hard, vec![("a-1-1".to_string(), "b-1-1".to_string())]);
        assert!(p.unmerge.is_empty());
    }

    #[test]
    fn helper_line_with_prefix_is_parsed_once_stripped() {
        // What `install_files`/`install` return carries "helper refused: ".
        let line = "helper refused: conflicting dependencies: quickshell [0.3.2-1.1] vs noctalia-qs [0.0.12-2]";
        assert_eq!(parse(line).pairs.len(), 0);
        let p = parse(line.strip_prefix("helper refused: ").unwrap());
        assert_eq!(
            p.pairs,
            vec![c("quickshell", "0.3.2-1.1", "noctalia-qs", "0.0.12-2")]
        );
    }

    #[test]
    fn dependents_skip_other_blockers() {
        let p = plan(&[c("q", "1-1", "qs", "1-1"), c("q", "1-1", "qs2", "1-1")]);
        let f = |n: &str| match n {
            "qs" => vec!["shell".to_string(), "qs2".to_string()],
            _ => vec![],
        };
        // Must not panic; qs2 is itself being unmerged, so only `shell` counts.
        print_dependents(&p, &f);
    }

    #[test]
    fn rows_read_like_portage() {
        let p = plan(&[c("pipewire-jack", "1:1.4.2-1", "jack2", "1.9.22-1")]);
        let r = rows(&p);
        assert_eq!(
            r,
            vec![
                Row {
                    kind: Kind::Merge,
                    atom: "pipewire-jack-1:1.4.2-1".into(),
                    note: None
                },
                Row {
                    kind: Kind::Conflict,
                    atom: "jack2-1.9.22-1".into(),
                    note: Some("(\"jack2\" is blocking pipewire-jack-1:1.4.2-1)".into())
                },
            ]
        );
    }
}
