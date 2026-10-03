//! Focused fixture coverage for the first-party dependency audit.
//!
//! The fixture roots mirror only the production roots owned by the audit. Documentation, tests and
//! benchmarks are deliberately created beside them in one test to prove they are outside scope.

use super::rust_dependencies::RustDependencyKind;
use super::{FirstPartyDepsRule, audit_first_party_deps, audit_javascript_source};
use moth::first_party_js::{InventoriedJsSource, inventoried_javascript_sources};
use std::fs;
use std::path::Path;
use tempfile::{TempDir, tempdir};

fn fixture_workspace() -> TempDir {
    let workspace = tempdir().expect("temp dir");
    for root in super::FIRST_PARTY_SOURCE_ROOTS {
        fs::create_dir_all(workspace.path().join(root)).expect("first-party root");
    }
    write_fixture_file(
        workspace.path(),
        "Cargo.toml",
        "[package]\nname = \"moth\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n",
    );
    write_fixture_file(workspace.path(), "src/lib.rs", "");
    write_rust_package(workspace.path(), "moth-lexical", "");
    write_rust_package(
        workspace.path(),
        "moth-mon",
        "[dependencies]\nmoth-lexical = { path = \"../moth-lexical\" }\n",
    );
    workspace
}

fn write_fixture_file(workspace: &Path, relative: &str, contents: &str) {
    let path = workspace.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent");
    }
    fs::write(path, contents).expect("fixture file");
}

fn write_rust_package(workspace: &Path, name: &str, dependencies: &str) {
    write_fixture_file(
        workspace,
        &format!("crates/{name}/Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{dependencies}"
        ),
    );
    write_fixture_file(workspace, &format!("crates/{name}/src/lib.rs"), "");
}

fn findings_for(workspace: &TempDir) -> Vec<super::FirstPartyDepsFinding> {
    audit_first_party_deps(workspace.path())
        .expect("fixture roots and Cargo workspace are readable")
        .findings
}

fn assert_has_rule(findings: &[super::FirstPartyDepsFinding], rule: FirstPartyDepsRule) {
    assert!(
        findings.iter().any(|finding| finding.rule == rule),
        "missing {rule:?} in {findings:?}"
    );
}

#[test]
fn fixture_with_allowed_runtime_import_and_no_js_manifests_passes() {
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
    for (source_label, source) in [
        ("require.js", r#"const helper = require("./helper.js");"#),
        (
            "dynamic-import.js",
            r#"const lazy = import("./helper.js");"#,
        ),
        ("re-export.js", r#"export * from "lodash";"#),
    ] {
        let findings = audit_javascript_source(source_label, source);

        assert_eq!(
            findings.len(),
            1,
            "{source_label} should produce one finding: {findings:?}"
        );
        let finding = &findings[0];
        assert_eq!(
            finding.rule,
            FirstPartyDepsRule::UnapprovedModuleImport,
            "{source_label} should reach the unapproved-module rule: {findings:?}"
        );
        assert_eq!(
            finding.file, source_label,
            "{source_label} should retain its source label: {findings:?}"
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
fn the_audit_inspects_the_compiler_owned_javascript_inventory() {
    let workspace = fixture_workspace();

    let state = audit_first_party_deps(workspace.path()).expect("fixture roots are readable");

    assert_eq!(
        state.javascript_source_count,
        inventoried_javascript_sources().len(),
        "empty roots leave only the compiler-owned inventory, which the audit must still inspect"
    );
    assert!(
        state.findings.is_empty(),
        "the shipped inventory must satisfy the first-party policy: {:?}",
        state.findings
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

#[test]
fn rust_libraries_allow_only_the_forward_workspace_direction() {
    let workspace = fixture_workspace();
    write_rust_package(
        workspace.path(),
        "moth-mon",
        "[dependencies]\nlexical = { package = \"moth-lexical\", path = \"../moth-lexical\" }\n",
    );
    write_fixture_file(
        workspace.path(),
        "Cargo.toml",
        "[package]\nname = \"moth\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n\
         [dependencies]\nmoth-mon = { path = \"crates/moth-mon\" }\n\
         moth-lexical = { path = \"crates/moth-lexical\" }\n",
    );

    let state = audit_first_party_deps(workspace.path()).expect("legal Cargo direction");
    assert!(state.findings.is_empty(), "{:?}", state.findings);
    let cargo_roots: Vec<&str> = state
        .audited_roots
        .iter()
        .filter(|root| root.ends_with("Cargo.toml"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        cargo_roots,
        [
            "Cargo.toml",
            "crates/moth-lexical/Cargo.toml",
            "crates/moth-mon/Cargo.toml",
        ],
        "the report must name manifests actually inspected by Cargo"
    );
}

#[test]
fn rust_libraries_reject_reverse_normal_dev_and_build_edges() {
    for (section, kind) in [
        ("dependencies", RustDependencyKind::Normal),
        ("dev-dependencies", RustDependencyKind::Dev),
        ("build-dependencies", RustDependencyKind::Build),
    ] {
        for (package, dependency, path) in [
            ("moth-mon", "moth", "../.."),
            ("moth-lexical", "moth", "../.."),
            ("moth-lexical", "moth-mon", "../moth-mon"),
        ] {
            let workspace = fixture_workspace();
            write_rust_package(workspace.path(), "moth-mon", "");
            write_rust_package(
                workspace.path(),
                package,
                &format!("[{section}]\n{dependency} = {{ path = \"{path}\" }}\n"),
            );

            let findings = findings_for(&workspace);
            assert_eq!(findings.len(), 1, "{package}/{section}: {findings:?}");
            let finding = &findings[0];
            assert_eq!(finding.rule, FirstPartyDepsRule::RustDependencyDirection);
            assert_eq!(finding.file, format!("crates/{package}/Cargo.toml"));
            let edge = finding.rust_dependency.as_ref().expect("typed Cargo edge");
            assert_eq!(edge.package, package);
            assert_eq!(edge.dependency, dependency);
            assert_eq!(edge.kind, kind);
            assert_eq!(edge.rename, None);
        }
    }
}

#[test]
fn renamed_compiler_dependency_retains_canonical_package_identity() {
    let workspace = fixture_workspace();
    write_rust_package(
        workspace.path(),
        "moth-mon",
        "[dev-dependencies]\ncompiler_alias = { package = \"moth\", path = \"../..\" }\n",
    );

    let findings = findings_for(&workspace);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.rule, FirstPartyDepsRule::RustDependencyDirection);
    assert_eq!(finding.file, "crates/moth-mon/Cargo.toml");
    let edge = finding.rust_dependency.as_ref().expect("typed Cargo edge");
    assert_eq!(edge.package, "moth-mon");
    assert_eq!(edge.dependency, "moth");
    assert_eq!(edge.kind, RustDependencyKind::Dev);
    assert_eq!(edge.rename.as_deref(), Some("compiler_alias"));
    assert!(finding.message.contains("'moth'"), "{finding}");
}

#[test]
fn optional_renamed_compiler_dependency_retains_canonical_identity() {
    let workspace = fixture_workspace();
    write_rust_package(
        workspace.path(),
        "moth-mon",
        "[dependencies]\ncompiler_alias = { package = \"moth\", path = \"../..\", optional = true }\n",
    );

    let findings = findings_for(&workspace);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.rule, FirstPartyDepsRule::RustDependencyDirection);
    assert_eq!(finding.file, "crates/moth-mon/Cargo.toml");
    let edge = finding.rust_dependency.as_ref().expect("typed Cargo edge");
    assert_eq!(edge.package, "moth-mon");
    assert_eq!(edge.dependency, "moth");
    assert_eq!(edge.kind, RustDependencyKind::Normal);
    assert_eq!(edge.rename.as_deref(), Some("compiler_alias"));
    assert!(finding.message.contains("'moth'"), "{finding}");
}

#[test]
fn inactive_target_dependency_is_still_rejected() {
    let workspace = fixture_workspace();
    write_rust_package(
        workspace.path(),
        "moth-lexical",
        "[target.'cfg(any())'.dependencies]\nmoth-mon = { path = \"../moth-mon\" }\n",
    );

    let findings = findings_for(&workspace);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.rule, FirstPartyDepsRule::RustDependencyDirection);
    assert_eq!(finding.file, "crates/moth-lexical/Cargo.toml");
    let edge = finding.rust_dependency.as_ref().expect("typed Cargo edge");
    assert_eq!(edge.package, "moth-lexical");
    assert_eq!(edge.dependency, "moth-mon");
    assert_eq!(edge.kind, RustDependencyKind::Normal);
    assert_eq!(edge.rename, None);
    assert!(finding.message.contains("'moth-mon'"), "{finding}");
}

#[test]
fn extracted_libraries_reject_other_first_party_workspace_packages() {
    for package in ["moth-mon", "moth-lexical"] {
        let workspace = fixture_workspace();
        write_rust_package(workspace.path(), "utility", "");
        write_rust_package(
            workspace.path(),
            package,
            "[dependencies]\nutility = { path = \"../utility\" }\n",
        );

        let findings = findings_for(&workspace);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(
            findings[0].rule,
            FirstPartyDepsRule::RustDependencyDirection
        );
        let edge = findings[0]
            .rust_dependency
            .as_ref()
            .expect("typed Cargo edge");
        assert_eq!(edge.package, package);
        assert_eq!(edge.dependency, "utility");
    }
}

#[test]
fn missing_extracted_workspace_packages_fail_closed() {
    for package in ["moth-mon", "moth-lexical"] {
        let workspace = fixture_workspace();
        write_rust_package(workspace.path(), "moth-mon", "");
        fs::remove_dir_all(workspace.path().join("crates").join(package))
            .expect("remove configured package");

        let error = audit_first_party_deps(workspace.path())
            .expect_err("missing configured packages cannot pass the dependency audit");
        assert!(
            error.contains(&format!("missing configured workspace package '{package}'")),
            "{error}"
        );
    }
}

#[test]
fn cargo_metadata_failure_cannot_pass_the_dependency_audit() {
    let workspace = fixture_workspace();
    write_fixture_file(workspace.path(), "Cargo.toml", "[package\n");

    let error = audit_first_party_deps(workspace.path())
        .expect_err("failed Cargo metadata cannot produce a clean dependency report");
    assert!(error.contains("cargo metadata"), "{error}");
    assert!(error.contains("failed"), "{error}");
}

#[test]
fn compiler_and_xtask_dependencies_are_forbidden_even_when_excluded_from_workspace_members() {
    for forbidden in ["moth", "xtask"] {
        let workspace = fixture_workspace();
        write_fixture_file(
            workspace.path(),
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"excluded\"]\nresolver = \"3\"\n",
        );
        write_fixture_file(
            workspace.path(),
            "excluded/Cargo.toml",
            &format!(
                "[package]\nname = \"{forbidden}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            ),
        );
        write_fixture_file(workspace.path(), "excluded/src/lib.rs", "");
        write_rust_package(
            workspace.path(),
            "moth-mon",
            &format!("[dependencies]\n{forbidden} = {{ path = \"../../excluded\" }}\n"),
        );

        let findings = findings_for(&workspace);
        assert_eq!(findings.len(), 1, "{forbidden}: {findings:?}");
        assert_eq!(
            findings[0].rule,
            FirstPartyDepsRule::RustDependencyDirection
        );
        let edge = findings[0]
            .rust_dependency
            .as_ref()
            .expect("typed Cargo edge");
        assert_eq!(edge.package, "moth-mon");
        assert_eq!(edge.dependency, forbidden);
        assert_eq!(edge.kind, RustDependencyKind::Normal);
    }
}
