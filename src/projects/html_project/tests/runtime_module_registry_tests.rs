//! Tests for the builder-owned core JS runtime module registry.
//!
//! WHAT: proves that the v1 registry contains exactly `@moth/runtime` and that
//!       its source exports the expected `mothOk` and `mothErr` wrapper functions.

use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

#[test]
fn v1_registry_contains_exactly_moth_runtime() {
    let registry = RuntimeModuleRegistry::v1();

    let modules = registry.registered_modules();
    assert_eq!(
        modules.len(),
        1,
        "v1 registry should contain exactly one module"
    );
    assert_eq!(
        modules[0].specifier, "@moth/runtime",
        "v1 registry should contain @moth/runtime"
    );
}

#[test]
fn v1_runtime_module_source_exports_moth_ok_and_moth_err() {
    let registry = RuntimeModuleRegistry::v1();
    let source = registry
        .module_source("@moth/runtime")
        .expect("@moth/runtime should have source");

    assert!(
        source.contains("export function mothOk"),
        "runtime source should export mothOk"
    );
    assert!(
        source.contains("export function mothErr"),
        "runtime source should export mothErr"
    );
}

#[test]
fn v1_runtime_source_produces_success_wrapper() {
    let registry = RuntimeModuleRegistry::v1();
    let source = registry
        .module_source("@moth/runtime")
        .expect("@moth/runtime should have source");

    assert!(
        source.contains("{ ok: true, value: value }"),
        "mothOk should produce {{ ok: true, value: value }} wrapper"
    );
}

#[test]
fn v1_runtime_source_produces_error_wrapper() {
    let registry = RuntimeModuleRegistry::v1();
    let source = registry
        .module_source("@moth/runtime")
        .expect("@moth/runtime should have source");

    assert!(
        source.contains("{ ok: false, error: { code, message } }"),
        "mothErr should produce {{ ok: false, error: {{ code, message }} }} wrapper"
    );
}

#[test]
fn empty_registry_has_no_modules() {
    let registry = RuntimeModuleRegistry::empty();
    assert!(registry.registered_modules().is_empty());
    assert!(!registry.is_registered("@moth/runtime"));
    assert!(registry.module_source("@moth/runtime").is_none());
}

#[test]
fn is_registered_finds_exact_specifier() {
    let registry = RuntimeModuleRegistry::v1();
    assert!(registry.is_registered("@moth/runtime"));
    assert!(!registry.is_registered("@moth/other-runtime"));
    assert!(!registry.is_registered("./helper.js"));
}

#[test]
fn is_exported_name_finds_registered_names() {
    let registry = RuntimeModuleRegistry::v1();
    assert!(registry.is_exported_name("@moth/runtime", "mothOk"));
    assert!(registry.is_exported_name("@moth/runtime", "mothErr"));
}

#[test]
fn is_exported_name_rejects_unknown_names() {
    let registry = RuntimeModuleRegistry::v1();
    assert!(!registry.is_exported_name("@moth/runtime", "nope"));
    assert!(!registry.is_exported_name("@moth/other-runtime", "mothOk"));
}
