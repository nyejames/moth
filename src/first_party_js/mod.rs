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

use crate::backends::js::collection_javascript_helpers;
use crate::backends::js::package_bindings::core::core_javascript_helpers;
use crate::builder_surface::core_packages::core_javascript_inline_expressions;
use crate::projects::html_project::external_js::parser::{
    parsed_js_module::JsDiagnosticKind, scan_exports,
};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// One compiler-owned JavaScript fragment inspected by first-party dependency validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoriedJsSource {
    pub label: String,
    pub source: String,
}

/// The policy category of a first-party JavaScript import finding.
///
/// Unapproved imports are module-loading forms other than a supported named static
/// import of a registered runtime module. Invalid runtime imports are
/// supported-import-shaped statements against a registered module whose form or
/// imported name is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstPartyJavascriptImportFindingKind {
    UnapprovedModuleImport,
    InvalidRuntimeImport,
}

/// A structured first-party JavaScript import finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstPartyJavascriptImportFinding {
    pub kind: FirstPartyJavascriptImportFindingKind,
    pub message: String,
}

/// Core package helper bodies, `@core/collections` runtime helpers, inline lowering templates and
/// registered runtime-module sources.
pub fn inventoried_javascript_sources() -> Vec<InventoriedJsSource> {
    let mut sources = Vec::new();

    for helper in core_javascript_helpers() {
        sources.push(InventoriedJsSource {
            label: format!("core-js-helper:{}", helper.name),
            source: helper.source.to_owned(),
        });
    }
    for helper in collection_javascript_helpers() {
        sources.push(InventoriedJsSource {
            label: format!("core-collections-js-helper:{}", helper.name),
            source: helper.source,
        });
    }

    for (label, source) in core_javascript_inline_expressions() {
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

/// Structured findings for import, require and re-export forms that first-party JavaScript must
/// not use.
pub fn javascript_import_findings(source: &str) -> Vec<FirstPartyJavascriptImportFinding> {
    scan_exports(source, &RuntimeModuleRegistry::v1())
        .diagnostics
        .into_iter()
        .filter_map(|diagnostic| {
            let kind = match diagnostic.kind {
                JsDiagnosticKind::DynamicImport
                | JsDiagnosticKind::ArbitraryImport
                | JsDiagnosticKind::CommonJsRequire
                | JsDiagnosticKind::ReExportFrom => {
                    FirstPartyJavascriptImportFindingKind::UnapprovedModuleImport
                }
                JsDiagnosticKind::UnsupportedRuntimeImportForm
                | JsDiagnosticKind::UnknownRuntimeImportName => {
                    FirstPartyJavascriptImportFindingKind::InvalidRuntimeImport
                }
                _ => return None,
            };
            Some(FirstPartyJavascriptImportFinding {
                kind,
                message: diagnostic.message,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
