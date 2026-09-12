//! Focused fixture coverage for the first-party dependency audit.
//!
//! The fixture roots mirror only the production roots owned by the audit. Documentation, tests and
//! benchmarks are deliberately created beside them in one test to prove they are outside scope.

use super::{
    FIRST_PARTY_DEPS_SCHEMA_VERSION, FirstPartyDepsRule, audit_first_party_deps,
    audit_javascript_source, started_report,
};
use crate::report_file::ReportRunIdentity;
use moth::first_party_js::{InventoriedJsSource, inventoried_javascript_sources};
use std::fs;
use std::path::Path;
use tempfile::{TempDir, tempdir};

fn fixture_workspace() -> TempDir {
    let workspace = tempdir().expect("temp dir");
    for root in super::FIRST_PARTY_SOURCE_ROOTS {
        fs::create_dir_all(workspace.path().join(root)).expect("first-party root");
    }
    workspace
}

fn write_fixture_file(workspace: &Path, relative: &str, contents: &str) {
    let path = workspace.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent");
    }
    fs::write(path, contents).expect("fixture file");
}

fn findings_for(workspace: &TempDir) -> Vec<super::FirstPartyDepsFinding> {
    let (_visited, _javascript, findings) =
        audit_first_party_deps(workspace.path()).expect("fixture roots are readable");
    findings
}

fn assert_has_rule(findings: &[super::FirstPartyDepsFinding], rule: FirstPartyDepsRule) {
    assert!(
        findings.iter().any(|finding| finding.rule == rule),
        "missing {rule:?} in {findings:?}"
    );
}

#[test]
fn fixture_with_allowed_runtime_import_and_no_manifests_passes() {
    let workspace = fixture_workspace();
    write_fixture_file(
        workspace.path(),
        "src/projects/html_project/binding_packages/web/canvas.js",
        r#"import { mothOk, mothErr } from "@moth/runtime";
const values = [Math, JSON, Date, Uint8Array];
"#,
    );

    let findings = findings_for(&workspace);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn paths_used_by_docs_tests_and_benchmarks_are_outside_scope() {
    let workspace = fixture_workspace();
    for path in [
        "docs/package.json",
        "tests/cases/package.json",
        "benchmarks/package.json",
        "src/projects/html_project/external_js/runtime_glue/package.json",
        "src/projects/html_project/external_js/runtime_glue/generated.js",
    ] {
        write_fixture_file(workspace.path(), path, "{}\n");
    }

    let findings = findings_for(&workspace);
    assert!(
        findings.is_empty(),
        "outside-scope paths must not be scanned: {findings:?}"
    );
}

#[test]
fn rejects_package_json_under_a_first_party_root() {
    let workspace = fixture_workspace();
    write_fixture_file(workspace.path(), "packages/html/package.json", "{}\n");

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::PackageManagerManifest,
    );
}

#[test]
fn rejects_a_lockfile_under_a_first_party_root() {
    let workspace = fixture_workspace();
    write_fixture_file(
        workspace.path(),
        "src/builder_surface/core_packages/package-lock.json",
        "{}\n",
    );

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::PackageManagerManifest,
    );
}

#[test]
fn rejects_deno_package_metadata_under_a_first_party_root() {
    let workspace = fixture_workspace();
    write_fixture_file(workspace.path(), "packages/html/deno.json", "{}\n");

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::PackageManagerManifest,
    );
}

#[test]
fn rejects_a_node_modules_directory() {
    let workspace = fixture_workspace();
    fs::create_dir_all(workspace.path().join("packages/html/node_modules"))
        .expect("node_modules directory");

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::VendoredDependencyRoot,
    );
}

#[test]
fn rejects_a_vendor_directory() {
    let workspace = fixture_workspace();
    fs::create_dir_all(workspace.path().join("packages/html/vendor")).expect("vendor directory");

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::VendoredDependencyRoot,
    );
}

#[test]
fn rejects_an_unapproved_static_import() {
    let workspace = fixture_workspace();
    write_fixture_file(
        workspace.path(),
        "src/projects/html_project/binding_packages/untrusted.mjs",
        "import x from \"lodash\";\n",
    );

    let findings = findings_for(&workspace);
    assert_has_rule(&findings, FirstPartyDepsRule::UnapprovedModuleImport);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("lodash")),
        "the finding should identify the rejected module: {findings:?}"
    );
}

#[test]
fn rejects_require_dynamic_import_and_re_export_module_loading() {
    let source = r#"
const helper = require("./helper.js");
const lazy = import("./helper.js");
export * from "lodash";
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        3,
        "require, dynamic import and star re-export are each module loading: {findings:?}"
    );
    for form in [
        "CommonJS `require()`",
        "Dynamic `import()`",
        "Re-export forms",
    ] {
        assert!(
            findings.iter().any(|finding| {
                finding.rule == FirstPartyDepsRule::UnapprovedModuleImport
                    && finding.message.contains(form)
            }),
            "{form} must reach the unapproved-module rule: {findings:?}"
        );
    }
}

#[test]
fn syntax_only_exports_are_not_dependencies_and_invalid_runtime_imports_are_typed() {
    let source = r#"
const localName = value;
export { localName };
module.exports = value;
exports.foo = value;
import { unknown } from "@moth/runtime";
import * as runtime from "@moth/runtime";
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        2,
        "only the two invalid registered-runtime imports should remain: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.rule == FirstPartyDepsRule::InvalidRuntimeImport),
        "invalid runtime imports need their own rule: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.rule != FirstPartyDepsRule::UnapprovedModuleImport),
        "local and CommonJS export syntax must not be third-party dependency findings: {findings:?}"
    );
}

#[test]
fn missing_first_party_root_fails_closed() {
    let workspace = fixture_workspace();
    fs::remove_dir_all(workspace.path().join("packages")).expect("remove root");

    let error = audit_first_party_deps(workspace.path())
        .expect_err("a missing production root cannot be treated as a clean empty tree");
    assert!(
        error.contains("first-party root"),
        "unexpected error: {error}"
    );
}

#[test]
fn started_report_is_incomplete_until_the_walk_finishes() {
    let report = started_report(ReportRunIdentity::started("first-party-deps", None));

    assert_eq!(FIRST_PARTY_DEPS_SCHEMA_VERSION, 3);
    assert_eq!(report.schema_version, FIRST_PARTY_DEPS_SCHEMA_VERSION);
    assert!(!report.run.completed);
    assert_eq!(report.visited_file_count, 0);
    assert_eq!(report.javascript_source_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn the_audit_inspects_the_compiler_owned_javascript_inventory() {
    let workspace = fixture_workspace();

    let (_visited, javascript_sources, findings) =
        audit_first_party_deps(workspace.path()).expect("fixture roots are readable");

    assert_eq!(
        javascript_sources,
        inventoried_javascript_sources().len(),
        "empty roots leave only the compiler-owned inventory, which the audit must still inspect"
    );
    assert!(
        findings.is_empty(),
        "the shipped inventory must satisfy the first-party policy: {findings:?}"
    );
}

#[test]
fn an_inventoried_source_with_a_forbidden_import_is_rejected_under_its_label() {
    let mut state = super::ScanState::default();
    let sources = [InventoriedJsSource {
        label: "moth::first_party_js::fixture".to_owned(),
        source: "import x from \"lodash\";\n".to_owned(),
    }];

    super::scan_inventoried_javascript(&sources, &mut state);

    assert_eq!(state.javascript_source_count, 1);
    assert_eq!(
        state.findings.len(),
        1,
        "an inventoried source must be scanned, not only counted: {:?}",
        state.findings
    );
    let finding = &state.findings[0];
    assert_eq!(finding.rule, FirstPartyDepsRule::UnapprovedModuleImport);
    assert_eq!(finding.file, "moth::first_party_js::fixture");
}
