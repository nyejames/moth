//! First-party JavaScript inventory and import policy for emission and validation.
//!
//! WHAT: names compiler-owned Core helper bodies, inline lowering templates and runtime-module
//!       sources, and applies the zero-third-party import policy through the HTML JS scanner.
//! WHY: first-party packages promise no third-party JavaScript dependencies. That promise needs
//!      one inventory consumed by emission and validation, and one scanner owned by the HTML
//!      JS parser, rather than a second lexer or a Rust string-literal scrape.
//!
//! # What this module owns
//! - The inventory of compiler-owned first-party JavaScript that is not a physical `.js` asset.
//! - Mapping HTML JS scanner diagnostics onto the first-party import policy.
//!
//! # What this module does NOT own
//! - Physical `.js` assets such as `canvas.js`; those stay on disk and are walked by xtask.
//! - Package-manager manifests, vendored directories, or workspace traversal.
//! - Generated runtime glue, documentation, tests or user-owned JavaScript.

use crate::backends::js::package_bindings::core::core_javascript_helpers;
use crate::builder_surface::core_packages::{
    register_core_math_package, register_core_random_package, register_core_text_package,
    register_core_time_package,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::projects::html_project::external_js::parser::first_party_javascript_import_messages;
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// One compiler-owned JavaScript fragment inspected by first-party dependency validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoriedJsSource {
    pub label: String,
    pub source: String,
}

/// Helper bodies, inline lowering templates and registered runtime-module sources.
pub fn inventoried_javascript_sources() -> Vec<InventoriedJsSource> {
    let mut sources = Vec::new();

    for helper in core_javascript_helpers() {
        sources.push(InventoriedJsSource {
            label: format!("core-js-helper:{}", helper.name),
            source: helper.source.to_owned(),
        });
    }

    let mut registry = ExternalPackageRegistry::new();
    register_core_math_package(&mut registry);
    register_core_text_package(&mut registry);
    register_core_random_package(&mut registry);
    register_core_time_package(&mut registry);

    for (label, source) in registry.javascript_inline_expressions() {
        sources.push(InventoriedJsSource {
            label: format!("inline-js:{label}"),
            source,
        });
    }

    for module in RuntimeModuleRegistry::v1().registered_modules() {
        sources.push(InventoriedJsSource {
            label: format!("runtime-module:{}", module.specifier),
            source: module.source.clone(),
        });
    }

    sources
}

/// Messages for import, require and re-export forms that first-party JavaScript must not use.
pub fn javascript_import_findings(source: &str) -> Vec<String> {
    first_party_javascript_import_messages(source)
}

#[cfg(test)]
mod tests {
    use super::{inventoried_javascript_sources, javascript_import_findings};

    #[test]
    fn inventory_includes_runtime_helpers_and_inline_templates() {
        let sources = inventoried_javascript_sources();
        assert!(
            sources
                .iter()
                .any(|source| source.label == "runtime-module:@moth/runtime"
                    && source.source.contains("export function mothOk")),
            "runtime module source must come from RuntimeModuleRegistry: {sources:?}"
        );
        assert!(
            sources
                .iter()
                .any(|source| source.label == "core-js-helper:__moth_text_length"),
            "text helpers must be inventoried: {sources:?}"
        );
        assert!(
            sources
                .iter()
                .any(|source| source.label.contains("inline-js:@core/math")),
            "math inline expressions must be inventoried: {sources:?}"
        );
    }

    #[test]
    fn inventoried_javascript_has_no_third_party_imports() {
        for source in inventoried_javascript_sources() {
            let findings = javascript_import_findings(&source.source);
            assert!(
                findings.is_empty(),
                "{} has first-party import findings: {findings:?}",
                source.label
            );
        }
    }

    #[test]
    fn only_registered_runtime_modules_are_allowed_imports() {
        assert!(
            javascript_import_findings("import { mothOk } from \"@moth/runtime\";\n").is_empty()
        );
        assert!(!javascript_import_findings("import \"./local.js\";\n").is_empty());
        assert!(
            !javascript_import_findings("import \"https://example.test/lib.js\";\n").is_empty()
        );
        assert!(!javascript_import_findings("const module = import(name);\n").is_empty());
        assert!(!javascript_import_findings("const value = require(name);\n").is_empty());
    }
}
