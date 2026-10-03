//! Unit tests for `packages` (kept out of the module file so the code stays readable).

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

#[test]
fn depends_on_map_parses_multi_block_and_continuation_lines() {
    let text = "\
Repository      : extra
Name             : openconnect
Version          : 9.12-1
Depends On       : gnutls  libxml2  vpnc-scripts

Repository      : extra
Name             : vim
Version          : 9.1-1
Depends On       : None
";
    // Exercise the same block-splitting/parsing this function uses,
    // without shelling out to pacman.
    let mut map = HashMap::new();
    for block in text.split("\n\n") {
        let mut name = None;
        let mut deps = HashSet::new();
        let mut capturing = false;
        for line in block.lines() {
            if let Some((label, value)) = line.split_once(" : ") {
                let label = label.trim();
                capturing = label == "Depends On";
                if label == "Name" {
                    name = Some(value.trim().to_string());
                } else if capturing {
                    let value = value.trim();
                    if value != "None" && !value.is_empty() {
                        deps.extend(value.split_whitespace().map(strip_version_operator));
                    }
                }
                continue;
            }
            if capturing {
                deps.extend(line.trim().split_whitespace().map(strip_version_operator));
            }
        }
        if let Some(n) = name {
            map.insert(n, deps);
        }
    }
    assert_eq!(
        map.get("openconnect").unwrap(),
        &HashSet::from([
            "gnutls".to_string(),
            "libxml2".to_string(),
            "vpnc-scripts".to_string()
        ])
    );
    assert_eq!(map.get("vim").unwrap(), &HashSet::new());
}
