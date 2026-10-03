//! Unit tests for `aur` (kept out of the module file so the code stays readable).

use super::*;

#[test]
fn srcinfo_dependencies_sums_strips_versions_and_dedupes() {
    let dir = std::env::temp_dir().join(format!("aur-srcinfo-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".SRCINFO");
    std::fs::write(
        &path,
        "pkgbase = foo\n\tpkgver = 1.0\n\tmakedepends = cmake\n\tdepends = glibc>=2.38\n\npkgname = foo\n\tdepends = zlib\n\tdepends = glibc\n",
    )
    .unwrap();
    let deps = srcinfo_dependencies(&path, "x86_64").unwrap();
    assert_eq!(deps, vec!["cmake", "glibc", "zlib"]);
    assert_eq!(srcinfo_pkgbase(&path), Some("foo".to_string()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn srcinfo_dependencies_includes_current_arch_suffix_only() {
    let dir = std::env::temp_dir().join(format!("aur-srcinfo-arch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".SRCINFO");
    std::fs::write(
        &path,
        "pkgbase = foo\n\tdepends = glibc\n\tdepends_x86_64 = lib32-glibc\n\tdepends_aarch64 = some-aarch64-only-lib\n",
    )
    .unwrap();
    let deps = srcinfo_dependencies(&path, "x86_64").unwrap();
    assert_eq!(deps, vec!["glibc", "lib32-glibc"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn srcinfo_dependencies_empty_file_is_some_empty() {
    let dir = std::env::temp_dir().join(format!("aur-srcinfo-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(".SRCINFO");
    std::fs::write(&path, "pkgbase = bare\n\tpkgver = 1.0\n").unwrap();
    assert_eq!(srcinfo_dependencies(&path, "x86_64"), Some(Vec::new()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn srcinfo_dependencies_missing_file_is_none() {
    let path = Path::new("/nonexistent/definitely/.SRCINFO");
    assert_eq!(srcinfo_dependencies(path, "x86_64"), None);
}

#[test]
fn extract_json_string_field_finds_value() {
    let json = r#"{"results":[{"Name":"foo","PackageBase":"foo-base"}]}"#;
    assert_eq!(
        extract_json_string_field(json, "PackageBase"),
        Some("foo-base".to_string())
    );
}

#[test]
fn extract_json_string_field_missing_is_none() {
    let json = r#"{"results":[]}"#;
    assert_eq!(extract_json_string_field(json, "PackageBase"), None);
}

#[test]
fn extract_json_string_field_handles_escaped_quotes() {
    let json = r#"{"Description":"A \"quoted\" word, and a slash \/ too"}"#;
    assert_eq!(
        extract_json_string_field(json, "Description"),
        Some("A \"quoted\" word, and a slash / too".to_string())
    );
}

#[test]
fn extract_json_string_field_null_is_none() {
    let json = r#"{"Maintainer":null}"#;
    assert_eq!(extract_json_string_field(json, "Maintainer"), None);
}

#[test]
fn parse_pkg_results_multiple_objects() {
    let json = r#"{"resultcount":2,"results":[
        {"Name":"foo","Version":"1.0-1","PackageBase":"foo","Description":"the foo pkg","NumVotes":10,"Popularity":1.5,"OutOfDate":null,"Maintainer":"alice"},
        {"Name":"bar","Version":"2.0-1","PackageBase":"bar-base","Description":"has a \"quote\"","NumVotes":0,"Popularity":0.0,"OutOfDate":1700000000,"Maintainer":null}
    ],"type":"search"}"#;
    let results = parse_pkg_results(json);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].name, "foo");
    assert_eq!(results[0].version, "1.0-1");
    assert_eq!(results[0].num_votes, 10);
    assert!(!results[0].out_of_date);
    assert_eq!(results[0].maintainer, Some("alice".to_string()));
    assert_eq!(results[1].pkgbase, "bar-base");
    assert_eq!(results[1].description, "has a \"quote\"");
    assert!(results[1].out_of_date);
    assert_eq!(results[1].maintainer, None);
}

#[test]
fn parse_pkg_results_empty_array() {
    let json = r#"{"resultcount":0,"results":[],"type":"search"}"#;
    assert!(parse_pkg_results(json).is_empty());
}
