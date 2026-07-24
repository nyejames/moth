//! HTML JavaScript external import provider.
//!
//! WHAT: implements `ExternalImportProvider` for single-file JavaScript binding modules annotated
//!       with Moth `@moth.*` metadata, turning parsed JS modules into typed external
//!       package registry entries.
//! WHY: project-local `.js` imports and built-in JS-backed packages need a compiler frontend
//!      surface before AST can resolve calls and types.

use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, ResolvedExternalImport,
    RuntimeAssetIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::package_registration::{
    register_parsed_js_module, required_runtime_imports_from_parsed,
};
use crate::projects::html_project::external_js::parser::parse_js_module;
use crate::projects::html_project::external_js::parser::parsed_js_module::{
    JsParserDiagnostic, ParsedJsModule,
};
use crate::projects::html_project::external_js::path_identity::{
    sanitized_path_stem, stable_path_hash_hex,
};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use std::path::Path;

/// HTML-owned JS external import provider.
///
/// WHAT: parses `.js` files with `@moth.opaque` and `@moth.sig` annotations and registers
///       discovered symbols in the shared `ExternalPackageRegistry`.
/// WHY: this is the bridge between the HTML builder's JS parser and the compiler frontend's
///      external package system.
#[derive(Debug)]
pub struct JsExternalImportProvider;

impl JsExternalImportProvider {
    /// Creates a new JS external import provider.
    pub fn new() -> Self {
        Self
    }
}

impl ExternalImportProvider for JsExternalImportProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("html-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        static SUPPORTED_EXTENSIONS: std::sync::OnceLock<Vec<ExternalFileExtension>> =
            std::sync::OnceLock::new();
        SUPPORTED_EXTENSIONS
            .get_or_init(|| vec![ExternalFileExtension::from("js")])
            .as_slice()
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        let source = match std::fs::read_to_string(&request.canonical_source_path) {
            Ok(content) => content,
            Err(error) => {
                return Err(CompilerMessages::from_error(
                    CompilerError::file_error(
                        &request.canonical_source_path,
                        format!(
                            "Failed to read JS import '{}': {error}",
                            request.canonical_source_path.display()
                        ),
                        context.string_table,
                    ),
                    context.string_table.clone(),
                ));
            }
        };

        let parsed = parse_js_module(&source, &RuntimeModuleRegistry::v1());

        let js_source_path = match InternedPath::try_from_filesystem_path(
            &request.canonical_source_path,
            context.string_table,
        ) {
            Ok(path) => path,
            Err(NonUtf8PathComponent { path: bad_path }) => {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::file_error(
                        &bad_path,
                        format!(
                            "JS import source path {bad_path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                        ),
                        context.string_table,
                    ),
                    context.string_table,
                ));
            }
        };

        let mut diagnostics = convert_js_parser_diagnostics(
            &parsed.diagnostics,
            &js_source_path,
            context.string_table,
        );

        // Reject project-local JS imports that declare receiver-style signatures.
        // WHY: source-authored `@moth.sig` signatures must expose free functions and opaque
        //      types, not `this` receiver parameters. The shared parser still classifies
        //      receiver-shaped signatures so every registration boundary can reject them.
        diagnostics.extend(reject_receiver_methods_in_project_local_js(
            &parsed,
            &js_source_path,
            context.string_table,
        ));

        if diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
        {
            return Err(CompilerMessages::from_diagnostics(
                diagnostics,
                context.string_table.clone(),
            ));
        }

        let package_path = js_provider_package_path(&request.canonical_source_path);
        let package_id = context
            .package_registry
            .register_package(
                package_path,
                crate::builder_surface::PackageOrigin::ProjectLocal,
            )
            .map_err(|error| CompilerMessages::from_error(error, context.string_table.clone()))?;

        let registered = register_parsed_js_module(package_id, &parsed, context.package_registry)
            .map_err(|error| {
            CompilerMessages::from_error(error, context.string_table.clone())
        })?;

        let required_runtime_imports = required_runtime_imports_from_parsed(&parsed);

        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: registered.exported_types,
            exported_free_functions: registered.exported_free_functions,
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: request.canonical_source_path,
                asset_kind: "js".to_owned(),
            }),
            diagnostics,
            required_runtime_imports,
        }))
    }
}

// ------------------------------------------
//  Package path
// ------------------------------------------

fn js_provider_package_path(canonical_source_path: &Path) -> String {
    let safe_stem = sanitized_path_stem(canonical_source_path, "module");
    let hash = stable_path_hash_hex(canonical_source_path);

    format!("@html-js/{safe_stem}-{hash}")
}

// ------------------------------------------
//  Receiver method rejection
// ------------------------------------------

fn reject_receiver_methods_in_project_local_js(
    parsed: &ParsedJsModule,
    js_source_path: &InternedPath,
    string_table: &mut StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut diagnostics = Vec::new();
    let path = js_source_path.clone();

    for receiver_method in &parsed.receiver_methods {
        let message = format!(
            "JS module signature for '{}' uses a 'this' receiver parameter. Project-local JS imports must expose free functions and opaque types only.",
            receiver_method.moth_name
        );
        let message_id = string_table.intern(&message);
        let location = js_parser_source_location(path.clone(), &receiver_method.annotation_span);

        diagnostics.push(CompilerDiagnostic::invalid_external_module(
            path.clone(),
            message_id,
            location,
        ));
    }

    diagnostics
}

// ------------------------------------------
//  Diagnostic conversion
// ------------------------------------------

fn convert_js_parser_diagnostics(
    parser_diagnostics: &[JsParserDiagnostic],
    js_source_path: &InternedPath,
    string_table: &mut StringTable,
) -> Vec<CompilerDiagnostic> {
    let mut diagnostics = Vec::with_capacity(parser_diagnostics.len());
    let path = js_source_path.clone();

    for parser_diagnostic in parser_diagnostics {
        let message_id = string_table.intern(&parser_diagnostic.message);
        let location = js_parser_source_location(path.clone(), &parser_diagnostic.span);

        diagnostics.push(CompilerDiagnostic::invalid_external_module(
            path.clone(),
            message_id,
            location,
        ));
    }

    diagnostics
}

fn js_parser_source_location(
    path: InternedPath,
    span: &crate::projects::html_project::external_js::parser::parsed_js_module::JsSourceSpan,
) -> SourceLocation {
    let start = CharPosition {
        line_number: span.line as i32,
        char_column: span.column as i32,
    };
    let end_column = if span.byte_end > span.byte_start {
        span.column.saturating_add(span.byte_end - span.byte_start)
    } else {
        span.column.saturating_add(1)
    };
    let end = CharPosition {
        line_number: span.line as i32,
        char_column: end_column as i32,
    };

    SourceLocation::new(path, start, end)
}
