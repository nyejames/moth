//! Focused fixture coverage for the first-party dependency audit.
//!
//! The fixture roots mirror only the production roots owned by the audit. Documentation, tests and
//! benchmarks are deliberately created beside them in one test to prove they are outside scope.

use super::{
    FIRST_PARTY_DEPS_SCHEMA_VERSION, FirstPartyDepsRule, audit_first_party_deps,
    audit_javascript_source, started_report,
};
use crate::report_file::ReportRunIdentity;
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
fn rejects_require_and_dynamic_import_regardless_of_argument() {
    let source = r#"
const leftPad = require("left-pad");
const npm = import('some-npm');
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("require()")),
        "require must be rejected: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("import()")),
        "dynamic import must be rejected: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.rule == FirstPartyDepsRule::UnapprovedModuleImport)
    );
}

#[test]
fn rejects_dynamic_import_inside_an_exported_function_body() {
    let source = "export function load() { return import(\"lodash\"); }\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        !findings.is_empty(),
        "exported bodies must still be scanned: {findings:?}"
    );
}

#[test]
fn rejects_comment_hidden_from_clause_before_the_real_specifier() {
    let source = "import { mothOk /* from \"@moth/runtime\" */ } from \"lodash\";\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("lodash")),
        "comment text must not supply the module specifier: {findings:?}"
    );
}

#[test]
fn rejects_require_with_a_comment_before_the_argument_list() {
    let source = "require /* dependency */ (\"lodash\");\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("require()")),
        "comments cannot hide require calls: {findings:?}"
    );
}

#[test]
fn postfix_increment_division_does_not_hide_a_later_import() {
    let source = "let value = 1; value++ / 2; import(\"lodash\");\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        !findings.is_empty(),
        "postfix ++ division must not be classified as a regex: {findings:?}"
    );
}

#[test]
fn rejects_variable_interpolated_and_concatenated_dynamic_loads() {
    for source in [
        "import(module_name)\n",
        "import(`package-${name}`)\n",
        "require(module_name)\n",
        "import(\"@moth/runtime\" + \"/something-else\")\n",
    ] {
        let findings = audit_javascript_source("fixture.js", source);
        assert!(
            !findings.is_empty(),
            "unproven module load must fail closed: {source:?}"
        );
        assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedModuleImport);
    }
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
}

#[test]
fn reports_from_bindings_and_string_named_import_clauses() {
    let source = r#"
import { from as source } from "lodash-from";
import { "feature-name" as feature } from "lodash-named";
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        2,
        "clause names must not hide the module: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("lodash-from"))
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("lodash-named"))
    );
}

#[test]
fn reports_dynamic_import_of_a_template_literal_module() {
    let source = "const module = import(`lodash`);\n";
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        1,
        "template module names are still dynamic imports: {findings:?}"
    );
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedModuleImport);
}

#[test]
fn relative_absolute_url_and_data_imports_are_rejected() {
    let source = r#"
import "./local.js";
import "../parent.js";
import "/absolute.js";
import "https://example.test/library.js";
import "http://example.test/library.js";
import "data:text/javascript,export default 1";
import "//cdn.example.test/library.js";
"#;
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        7,
        "only registered runtime modules are allowed: {findings:?}"
    );
}

#[test]
fn allowed_runtime_specifier_passes() {
    let source = "import { mothOk } from \"@moth/runtime\";\n";

    assert!(
        audit_javascript_source("fixture.js", source).is_empty(),
        "the registered runtime module remains the only allowed import"
    );
}

#[test]
fn escaped_runtime_looking_specifiers_are_not_allowlisted() {
    let source = "import { mothOk } from \"@moth/ru\\ntime\";\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        !findings.is_empty(),
        "escape sequences must not forge an allowed specifier: {findings:?}"
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
fn division_does_not_hide_a_later_dynamic_import() {
    let source = "const ratio = 1 / 2; import(\"lodash\");\n";
    let findings = audit_javascript_source("fixture.js", source);

    assert_eq!(
        findings.len(),
        1,
        "numeric division must not be treated as a regex that swallows the import: {findings:?}"
    );
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedModuleImport);
}

#[test]
fn regex_after_return_does_not_hide_a_later_import() {
    let source = "function f() { return /\"/; } import(\"lodash\");\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        !findings.is_empty(),
        "a regex after return must not swallow a later import: {findings:?}"
    );
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
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedModuleImport);
    assert!(findings[0].message.contains("left-pad"));
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
    assert_eq!(findings[0].rule, FirstPartyDepsRule::UnapprovedModuleImport);
}

#[test]
fn rejects_dynamic_import_inside_nested_template_interpolation_braces() {
    let source = "const module = `${(() => { return import(\"lodash\"); })()}`;\n";
    let findings = audit_javascript_source("fixture.js", source);
    assert!(
        !findings.is_empty(),
        "nested interpolation blocks still execute imports: {findings:?}"
    );
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

    assert_eq!(report.schema_version, FIRST_PARTY_DEPS_SCHEMA_VERSION);
    assert!(!report.run.completed);
    assert_eq!(report.visited_file_count, 0);
    assert_eq!(report.javascript_source_count, 0);
    assert!(report.findings.is_empty());
}
