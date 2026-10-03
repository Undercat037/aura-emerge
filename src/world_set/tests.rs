//! Unit tests for `world_set`.

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
