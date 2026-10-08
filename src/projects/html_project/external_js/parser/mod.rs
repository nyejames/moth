//! HTML JavaScript `@moth.*` annotation parser and runtime-module scanner.
//!
//! WHAT: turns a single JS source file into a `ParsedJsModule` containing opaque types,
//!       callable signatures, literal constants, registered runtime imports and diagnostics.
//! WHY: this parser stays independent from compiler diagnostics and package registration so
//!      provider and built-in package registration can share one source model while rejecting
//!      receiver-shaped signatures consistently; its scanner also owns first-party module-loading
//!      policy.
//!
//! ## Module layout
//!
//! - `parsed_js_module`: parser-owned data model (spans, diagnostics, signatures).
//! - `comment_extractor`: finds `/** ... */` blocks and extracts `@moth.*` annotations.
//! - `export_scanner`: finds supported JS exports, counts parameters and provides shared JS
//!   lexical skipping used by import scanning.
//! - `import_scan`: validates static import / `require()` forms against the runtime module
//!   registry as an extra `ExportScanner` implementation.
//! - `signature_parser`: parses the Moth parameter/return syntax inside `@moth.sig`.
//! - `binding`: matches extracted annotations to scanned exports, validates signatures and
//!   constants, and deduplicates runtime imports.
//! - `mod.rs` (this file): orchestrates extraction → scanning → binding.

mod binding;
mod comment_extractor;
mod export_scanner;
mod import_scan;
pub(crate) mod parsed_js_module;
mod signature_parser;

pub(crate) use export_scanner::scan_exports;

#[cfg(test)]
mod tests;

use comment_extractor::extract_annotations;
use parsed_js_module::ParsedJsModule;

use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// Parses a single JS source file into a `ParsedJsModule` using an explicit registry.
///
/// WHAT: extracts annotations and scans JS exports/imports before binding the parser-owned facts.
/// This function does not interact with `ExternalPackageRegistry` or `CompilerDiagnostic`.
/// It returns parser-local data that package registration and provider code convert later.
pub(crate) fn parse_js_module(source: &str, registry: &RuntimeModuleRegistry) -> ParsedJsModule {
    let comment_result = extract_annotations(source, registry);
    let export_result = scan_exports(source, registry);

    binding::bind_js_module(source, registry, comment_result, export_result)
}
