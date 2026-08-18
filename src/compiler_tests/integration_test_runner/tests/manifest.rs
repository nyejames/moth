//! Self-tests for canonical manifest parsing and fixture ordering.
//!
//! WHAT: protects manifest validation and its authoritative case order.
//! WHY: manifest metadata controls which canonical fixtures the runner executes.

use super::super::fixture::load_test_suite_from_root;
use super::super::manifest::parse_manifest_file;
use super::super::{CaseRole, EXPECT_FILE_NAME, INPUT_DIR_NAME, MANIFEST_FILE_NAME};
use crate::compiler_tests::test_support::unused_temp_path;
use std::fs;
use std::path::Path;

fn parse_manifest_source(
    name: &str,
    source: &str,
) -> Result<Vec<super::super::ManifestCaseSpec>, String> {
    let root = unused_temp_path(name);
    fs::create_dir_all(&root).expect("should create manifest test root");
    let path = root.join(MANIFEST_FILE_NAME);
    fs::write(&path, source).expect("should write manifest");
    let result = parse_manifest_file(&path);
    fs::remove_dir_all(&root).expect("should clean up manifest test root");
    result
}

fn write_success_fixture(root: &Path, case_name: &str) {
    let case_root = root.join(case_name);
    let input_root = case_root.join(INPUT_DIR_NAME);
    fs::create_dir_all(&input_root).expect("should create fixture input directory");
    fs::write(input_root.join("@page.moth"), "#[:ok]\n").expect("should write fixture source");
    fs::write(
        case_root.join(EXPECT_FILE_NAME),
        "[backends.html]\nmode = \"success\"\nwarnings = \"forbid\"\nsuccess_contract = \"acceptance_only\"\n",
    )
    .expect("should write expect file");
}

#[test]
fn rejects_manifest_case_without_tags() {
    let root = unused_temp_path("manifest_missing_tags");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("manifest missing tags should be rejected");
    };
    assert!(
        error.contains("missing required tags"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn rejects_manifest_case_with_unknown_role() {
    let root = unused_temp_path("manifest_unknown_role");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\"]\nrole = \"unknown\"\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("unknown manifest role should be rejected");
    };
    assert!(error.contains("case"), "unexpected: {error}");
    assert!(error.contains("unknown"), "unexpected: {error}");
    assert!(error.contains("role"), "unexpected: {error}");

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn parses_every_supported_manifest_role() {
    let supported_roles = [
        ("primary", CaseRole::Primary),
        ("boundary", CaseRole::Boundary),
        ("backend", CaseRole::Backend),
        ("adversarial", CaseRole::Adversarial),
        ("smoke", CaseRole::Smoke),
    ];

    for (spelling, expected) in supported_roles {
        assert_eq!(CaseRole::parse(spelling), Ok(expected));
    }
}

#[test]
fn retains_primary_manifest_case_without_contract_for_policy_evaluation() {
    let root = unused_temp_path("manifest_primary_without_contract");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\"]\nrole = \"primary\"\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root)
        .expect("cross-case primary policy should be evaluated after loading");
    assert_eq!(suite.cases[0].role, Some(CaseRole::Primary));
    assert_eq!(suite.cases[0].contract, None);

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn rejects_manifest_case_with_empty_contract() {
    let root = unused_temp_path("manifest_empty_contract");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\"]\ncontract = \" \"\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("empty manifest contract should be rejected");
    };
    assert!(error.contains("case"), "unexpected: {error}");
    assert!(error.contains("empty"), "unexpected: {error}");
    assert!(error.contains("contract"), "unexpected: {error}");

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn retains_duplicate_primary_contracts_for_policy_evaluation() {
    let root = unused_temp_path("manifest_duplicate_primary_contract");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case_a");
    write_success_fixture(&root, "case_b");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case_a\"\npath = \"case_a\"\ntags = [\"coverage\"]\ncontract = \"language.example\"\nrole = \"primary\"\n\n[[case]]\nid = \"case_b\"\npath = \"case_b\"\ntags = [\"coverage\"]\ncontract = \"language.example\"\nrole = \"primary\"\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root)
        .expect("cross-case primary policy should be evaluated after loading");
    assert_eq!(suite.cases.len(), 2);
    assert_eq!(suite.cases[0].contract, Some("language.example".to_owned()));
    assert_eq!(suite.cases[1].contract, Some("language.example".to_owned()));

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn permits_shared_contracts_for_distinct_non_primary_roles() {
    let root = unused_temp_path("manifest_shared_non_primary_contract");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case_a");
    write_success_fixture(&root, "case_b");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case_a\"\npath = \"case_a\"\ntags = [\"coverage\"]\ncontract = \"language.example\"\nrole = \"boundary\"\n\n[[case]]\nid = \"case_b\"\npath = \"case_b\"\ntags = [\"coverage\"]\ncontract = \"language.example\"\nrole = \"backend\"\n",
    )
    .expect("should write manifest");

    let suite =
        load_test_suite_from_root(&root).expect("non-primary cases may share a semantic contract");
    assert_eq!(suite.cases.len(), 2);
    assert_eq!(suite.cases[0].role, Some(CaseRole::Boundary));
    assert_eq!(suite.cases[1].role, Some(CaseRole::Backend));

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn retains_unclassified_manifest_metadata_as_optional() {
    let root = unused_temp_path("manifest_unclassified_metadata");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\"]\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root).expect("unclassified case should load");
    let case = &suite.cases[0];
    assert_eq!(case.case_id, "case");
    assert_eq!(case.tags, vec!["coverage"]);
    assert_eq!(case.contract, None);
    assert_eq!(case.role, None);

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn manifest_order_is_preserved() {
    let root = unused_temp_path("manifest_order");
    fs::create_dir_all(&root).expect("should create root");

    write_success_fixture(&root, "case_a");
    write_success_fixture(&root, "case_b");
    write_success_fixture(&root, "case_c");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case_b\"\npath = \"case_b\"\ntags = [\"ordered\"]\n\n[[case]]\nid = \"case_a\"\npath = \"case_a\"\ntags = [\"ordered\"]\n\n[[case]]\nid = \"case_c\"\npath = \"case_c\"\ntags = [\"ordered\"]\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root).expect("suite should load");
    let names = suite
        .cases
        .iter()
        .map(|case| case.display_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["case_b [html]", "case_a [html]", "case_c [html]"]
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn manifest_must_declare_every_fixture_directory() {
    let root = unused_temp_path("manifest_authoritative");
    fs::create_dir_all(&root).expect("should create root");

    write_success_fixture(&root, "case_a");
    write_success_fixture(&root, "case_b");

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case_a\"\npath = \"case_a\"\ntags = [\"coverage\"]\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("manifest should reject undeclared fixtures");
    };
    assert!(
        error.contains("undeclared fixtures"),
        "unexpected error: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[test]
fn rejects_unsafe_manifest_case_paths() {
    let unsafe_paths = [
        "/outside",
        "../outside",
        "./case",
        "nested/../case",
        "nested/./case",
        "C:/outside",
    ];

    for unsafe_path in unsafe_paths {
        let result = parse_manifest_source(
            "manifest_unsafe_case_path",
            &format!("[[case]]\nid = \"case\"\npath = \"{unsafe_path}\"\ntags = [\"coverage\"]\n"),
        );
        let Err(error) = result else {
            panic!("unsafe manifest path should be rejected: {unsafe_path}");
        };
        assert!(error.contains("invalid path"), "unexpected: {error}");
        assert!(error.contains(unsafe_path), "unexpected: {error}");
    }
}

#[test]
fn rejects_whitespace_padded_manifest_metadata() {
    let padded_fields = [
        ("id", " case "),
        ("path", " case "),
        ("tags", " tag "),
        ("contract", " contract "),
    ];

    for (field, value) in padded_fields {
        let source = match field {
            "tags" => format!("[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"{value}\"]\n"),
            "contract" => format!(
                "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\"]\ncontract = \"{value}\"\n"
            ),
            "path" => {
                format!("[[case]]\nid = \"case\"\npath = \"{value}\"\ntags = [\"coverage\"]\n")
            }
            _ => {
                format!("[[case]]\n{field} = \"{value}\"\npath = \"case\"\ntags = [\"coverage\"]\n")
            }
        };
        let result = parse_manifest_source("manifest_padded_metadata", &source);
        let Err(error) = result else {
            panic!("padded manifest metadata should be rejected: {field}");
        };
        assert!(
            error.contains("leading or trailing whitespace"),
            "unexpected: {error}"
        );
    }
}

#[test]
fn rejects_duplicate_manifest_tags() {
    let result = parse_manifest_source(
        "manifest_duplicate_tags",
        "[[case]]\nid = \"case\"\npath = \"case\"\ntags = [\"coverage\", \"coverage\"]\n",
    );
    let Err(error) = result else {
        panic!("duplicate manifest tags should be rejected");
    };
    assert!(
        error.contains("duplicate tag 'coverage'"),
        "unexpected: {error}"
    );
}

#[test]
fn accepts_nested_manifest_path_and_preserves_metadata_order() {
    let root = unused_temp_path("manifest_nested_path");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "nested/case");
    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"nested/case\"\ntags = [\"zeta\", \"alpha\"]\n",
    )
    .expect("should write manifest");

    let suite = load_test_suite_from_root(&root).expect("nested manifest path should load");
    assert_eq!(suite.cases[0].manifest_relative_path, "nested/case");
    assert_eq!(suite.cases[0].tags, vec!["zeta", "alpha"]);

    fs::remove_dir_all(&root).expect("should clean up temp fixture root");
}

#[cfg(unix)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_manifest_fixture_symlink_escape() {
    let root = unused_temp_path("manifest_fixture_symlink_escape");
    let outside = unused_temp_path("manifest_fixture_symlink_escape_target");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&outside, "case");
    let link = root.join("link");
    if symlink_directory(&outside.join("case"), &link).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        fs::remove_dir_all(&outside).expect("should clean up target");
        return;
    }
    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"case\"\npath = \"link\"\ntags = [\"coverage\"]\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("fixture symlink escaping the suite root should be rejected");
    };
    assert!(
        error.contains("case") && error.contains("outside"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
    fs::remove_dir_all(&outside).expect("should clean up target");
}

#[cfg(any(unix, windows))]
#[test]
fn rejects_duplicate_canonical_fixture_root_through_in_suite_alias() {
    let root = unused_temp_path("manifest_duplicate_canonical_root");
    fs::create_dir_all(&root).expect("should create root");
    write_success_fixture(&root, "case_a");

    let alias = root.join("alias");
    if symlink_directory(&root.join("case_a"), &alias).is_err() {
        fs::remove_dir_all(&root).expect("should clean up root");
        return;
    }

    fs::write(
        root.join(MANIFEST_FILE_NAME),
        "[[case]]\nid = \"primary_case\"\npath = \"case_a\"\ntags = [\"coverage\"]\n\n[[case]]\nid = \"alias_case\"\npath = \"alias\"\ntags = [\"coverage\"]\n",
    )
    .expect("should write manifest");

    let Err(error) = load_test_suite_from_root(&root) else {
        panic!("duplicate canonical fixture root should be rejected");
    };
    assert!(
        error.contains("primary_case")
            && error.contains("case_a")
            && error.contains("alias_case")
            && error.contains("alias"),
        "error must retain both conflicting case ids and authored paths: {error}"
    );
    assert!(
        error.contains("duplicate canonical fixture root"),
        "unexpected: {error}"
    );

    fs::remove_dir_all(&root).expect("should clean up root");
}
