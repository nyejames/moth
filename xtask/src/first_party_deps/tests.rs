//! Focused fixture coverage for the first-party dependency audit.
//!
//! The fixture roots mirror only the production roots owned by the audit. Documentation, tests and
//! benchmarks are deliberately created beside them in one test to prove they are outside scope.

use super::{
    FIRST_PARTY_DEPS_SCHEMA_VERSION, FIRST_PARTY_SOURCE_ROOTS, FirstPartyDepsRule,
    RUNTIME_REGISTRY_RELATIVE_PATH, RUNTIME_SOURCE_LABEL, audit_first_party_deps,
    audit_javascript_source, extract_runtime_source, started_report,
};
use crate::report_file::ReportRunIdentity;
use std::fs;
use std::path::Path;
use tempfile::{TempDir, tempdir};

const CLEAN_RUNTIME_SOURCE: &str = r#"export function mothOk(value) {
    return { ok: true, value: value };
}

export function mothErr(code, message) {
    return { ok: false, error: { code, message } };
}
"#;

fn fixture_workspace() -> TempDir {
    let workspace = tempdir().expect("temp dir");
    for root in FIRST_PARTY_SOURCE_ROOTS {
        fs::create_dir_all(workspace.path().join(root)).expect("first-party root");
    }
    write_runtime_source(workspace.path(), CLEAN_RUNTIME_SOURCE);
    workspace
}

fn write_runtime_source(workspace: &Path, source: &str) {
    let path = workspace.join(RUNTIME_REGISTRY_RELATIVE_PATH);
    fs::create_dir_all(path.parent().expect("runtime registry parent")).expect("runtime parent");
    fs::write(
        path,
        format!("const MOTH_RUNTIME_SOURCE_V1: &str = r#\"{source}\"#;\n"),
    )
    .expect("runtime registry");
}

fn write_fixture_file(workspace: &Path, relative: &str, contents: &str) {
    let path = workspace.join(relative);
    fs::create_dir_all(path.parent().expect("fixture file parent")).expect("fixture parent");
    fs::write(path, contents).expect("fixture file");
}

fn findings_for(workspace: &TempDir) -> Vec<super::FirstPartyDepsFinding> {
    audit_first_party_deps(workspace.path())
        .expect("fixture roots and runtime registry should be readable")
        .1
}

fn assert_has_rule(findings: &[super::FirstPartyDepsFinding], rule: FirstPartyDepsRule) {
    assert!(
        findings.iter().any(|finding| finding.rule == rule),
        "expected {rule:?} in findings, got {findings:?}"
    );
}

#[test]
fn current_workspace_first_party_roots_are_clean() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest has a parent");

    let (audited_file_count, findings) =
        audit_first_party_deps(workspace_root).expect("current first-party roots are readable");

    assert!(
        audited_file_count > 0,
        "the guard must inspect implementation files rather than an empty path list"
    );
    assert!(
        findings.is_empty(),
        "first-party dependency findings: {findings:?}"
    );
}

#[test]
fn fixture_with_allowed_runtime_import_and_no_manifests_passes() {
    let workspace = fixture_workspace();
    write_fixture_file(
        workspace.path(),
        "src/projects/html_project/binding_packages/web/canvas.js",
        r#"import { mothOk, mothErr } from "@moth/runtime";
import "./helpers.js";
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
    assert_has_rule(&findings, FirstPartyDepsRule::UnapprovedBareImport);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("lodash")),
        "the finding should identify the rejected module: {findings:?}"
    );
}

#[test]
fn rejects_require_and_dynamic_import_bare_specifiers() {
    let source = r#"
const leftPad = require("left-pad");
const npm = import('some-npm');
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        2,
        "each import form should be reported: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| { finding.rule == FirstPartyDepsRule::UnapprovedBareImport })
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("left-pad"))
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("some-npm"))
    );
}

#[test]
fn detects_side_effect_and_re_export_import_forms() {
    let source = r#"
import "lodash-side-effect";
export { value } from 'lodash-reexport';
export * from "lodash-star";
"#;
    let findings = audit_javascript_source("fixture.mjs", source);

    assert_eq!(
        findings.len(),
        3,
        "all static import forms should be reported: {findings:?}"
    );
    for module in ["lodash-side-effect", "lodash-reexport", "lodash-star"] {
        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains(module)),
            "missing finding for {module}: {findings:?}"
        );
    }
}

#[test]
fn relative_absolute_url_and_allowed_runtime_specifiers_pass() {
    let source = r#"
import "./local.js";
import "../parent.js";
import "/absolute.js";
import "https://example.test/library.js";
import "data:text/javascript,export default 1";
import { mothOk } from "@moth/runtime";
"#;

    assert!(
        audit_javascript_source("fixture.js", source).is_empty(),
        "only third-party bare specifiers should be rejected"
    );
}

#[test]
fn comments_and_non_import_builtins_do_not_create_findings() {
    let source = r#"
// import x from "lodash";
/* require("left-pad") */
const value = Math.max(Date.now(), JSON.parse("1"));
"#;

    assert!(audit_javascript_source("fixture.js", source).is_empty());
}

#[test]
fn regex_literals_with_quotes_do_not_hide_later_imports() {
    let source =
        "const q = text.replace(/\"/g, \"&quot;\");\nimport { thing } from \"left-pad\";\n";
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        1,
        "regex quotes must not desynchronize import scanning: {findings:?}"
    );
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedBareImport);
    assert!(findings[0].message.contains("left-pad"));
}

#[test]
fn regex_literals_after_assignment_do_not_hide_later_imports() {
    let source = "const pattern = /\"/g;\nimport x from \"lodash\";\n";
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        1,
        "assignment regex quotes must not hide later imports: {findings:?}"
    );
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedBareImport);
    assert!(findings[0].message.contains("lodash"));
}

#[test]
fn dynamic_imports_inside_template_interpolations_are_reported() {
    let source = "const module = `${await import(\"lodash\")}`;\n";
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        1,
        "template interpolations contain executable imports: {findings:?}"
    );
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedBareImport);
    assert!(findings[0].message.contains("lodash"));
}

#[test]
fn import_meta_does_not_treat_a_later_from_binding_as_an_import() {
    let source = "const base = import.meta.url\nconst from = \"lodash\"\n";

    assert!(
        audit_javascript_source("fixture.js", source).is_empty(),
        "import.meta must not scan later from bindings as specifiers"
    );
}

#[test]
fn inline_runtime_source_is_scanned_with_the_same_allowlist() {
    let workspace = fixture_workspace();
    write_runtime_source(
        workspace.path(),
        "import x from \"runtime-third-party\";\nexport function mothOk() { return x; }\n",
    );

    let findings = findings_for(&workspace);
    assert!(
        findings.iter().any(|finding| {
            finding.file == RUNTIME_SOURCE_LABEL
                && finding.rule == FirstPartyDepsRule::UnapprovedBareImport
                && finding.message.contains("runtime-third-party")
        }),
        "runtime source must be included in the audit: {findings:?}"
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
fn malformed_inline_runtime_source_is_a_typed_finding() {
    let workspace = fixture_workspace();
    let path = workspace.path().join(RUNTIME_REGISTRY_RELATIVE_PATH);
    fs::write(
        path,
        "const MOTH_RUNTIME_SOURCE_V1: &str = r#\"unterminated;\n",
    )
    .expect("malformed runtime registry");

    assert_has_rule(
        &findings_for(&workspace),
        FirstPartyDepsRule::InvalidRuntimeSource,
    );
}

#[test]
fn started_report_is_incomplete_until_the_walk_finishes() {
    let report = started_report(ReportRunIdentity::started("first-party-deps", None));

    assert_eq!(report.schema_version, FIRST_PARTY_DEPS_SCHEMA_VERSION);
    assert!(!report.run.completed);
    assert_eq!(report.audited_file_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn extracts_only_the_named_runtime_raw_string() {
    let registry = format!(
        "const unrelated: &str = \"not runtime\";\nconst MOTH_RUNTIME_SOURCE_V1: &str = r#\"{CLEAN_RUNTIME_SOURCE}\"#;\n"
    );

    assert_eq!(
        extract_runtime_source(&registry).expect("runtime source"),
        CLEAN_RUNTIME_SOURCE
    );
}
