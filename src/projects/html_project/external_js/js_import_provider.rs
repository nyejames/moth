//! HTML JavaScript external import provider.
//!
//! WHAT: implements `ExternalImportProvider` for single-file JavaScript binding modules annotated
//!       with Moth `@moth.*` metadata, turning parsed JS modules into typed external
//!       package registry entries.
//! WHY: project-local `.js` imports and built-in JS-backed packages need a compiler frontend
//!      surface before AST can resolve calls and types.

use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, ResolvedExternalImport,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{imports, token_lexical};
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticSeverity, InvalidExternalModuleReason, PreparedFailure,
    SourceSpanCapacityResource,
};
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternError};
use crate::projects::html_project::external_js::package_registration::{
    JsModuleRegistrationError, register_parsed_js_module, required_runtime_imports_from_parsed,
};
use crate::projects::html_project::external_js::parser::parse_js_module;
use crate::projects::html_project::external_js::parser::parsed_js_module::{
    JsParserDiagnostic, ParsedJsModule,
};
use crate::projects::html_project::external_js::runtime_assets::js_runtime_asset_identity;
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// HTML-owned JS external import provider.
///
/// WHAT: parses `.js` files with `@moth.opaque`, `@moth.sig` and `@moth.const` annotations and registers
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
    ) -> Result<Option<ResolvedExternalImport>, PreparedFailure> {
        let bytes = match std::fs::read(&request.canonical_source_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(InfrastructureFailure::filesystem(
                    CompilerStage::SourceLoading,
                    &request.canonical_source_path,
                    format!(
                        "Failed to read JS import '{}': {error}",
                        request.canonical_source_path.display()
                    ),
                )
                .into());
            }
        };

        let js_source_path = match context
            .path_fork
            .try_intern_portable_path(request.logical_source_path.as_str(), context.string_table)
        {
            Ok(path) => path,
            Err(PathInternError::NonUtf8(non_utf8)) => {
                return Err(InfrastructureFailure::filesystem(
                    CompilerStage::ProjectBuildAssembly,
                    &non_utf8.path,
                    format!(
                        "JS import source path {:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths.",
                        non_utf8.path
                    ),
                )
                .into());
            }
            Err(PathInternError::TableFull) => {
                return Err(context.capture_diagnostic_failure(DiagnosticBag::from(
                    token_lexical::source_span_capacity(
                        0,
                        u32::MAX,
                        SourceSpanCapacityResource::LogicalPath,
                        None,
                    ),
                )));
            }
            Err(PathInternError::BaseMismatch { .. }) => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "logical path merge base is not a structural prefix of the destination table",
                )
                .into());
            }
        };

        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                let utf8_detail = {
                    let mut diagnostic_strings = DiagnosticStringBuilder::new(context.string_table);
                    diagnostic_strings.intern(&error.utf8_error().to_string())
                };
                return Err(context.capture_diagnostic_failure(DiagnosticBag::from(
                    imports::invalid_external_module(
                        js_source_path,
                        InvalidExternalModuleReason::NonUtf8Source { utf8_detail },
                        request.source_span,
                    ),
                )));
            }
        };

        let parsed = parse_js_module(&source, &RuntimeModuleRegistry::v1());

        let diagnostics = {
            let mut diagnostic_strings = DiagnosticStringBuilder::new(context.string_table);
            let mut diagnostics = convert_js_parser_diagnostics(
                &parsed.diagnostics,
                js_source_path,
                request.source_span,
                &mut diagnostic_strings,
            );
            // Package paths reserve `#`, so diagnose the unregistrable logical path here.
            if request.logical_source_path.as_str().contains('#') {
                diagnostics.push(imports::invalid_external_module(
                    js_source_path,
                    InvalidExternalModuleReason::PathContainsPackageSeparator,
                    request.source_span,
                ));
            }

            // Reject project-local JS imports that declare receiver-style signatures.
            // WHY: callable `@moth.sig` signatures must expose free functions rather than `this`
            //      receiver parameters. The parser classifies receiver-shaped signatures so every
            //      registration boundary can reject them.
            diagnostics.extend(reject_receiver_methods_in_project_local_js(
                &parsed,
                js_source_path,
                &mut diagnostic_strings,
            ));
            diagnostics
        };

        // All diagnostics here are error severity, so a successful import carries none.
        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Error)
        {
            return Err(
                context.capture_diagnostic_failure(DiagnosticBag::from_diagnostics(diagnostics))
            );
        }

        let package_path = js_provider_package_path(&request.logical_source_path);
        let package_id = match context.package_registry.register_package(
            &package_path,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        ) {
            Ok(package_id) => package_id,
            Err(error) => {
                return Err(js_registration_failure(
                    JsModuleRegistrationError::from(error),
                    js_source_path,
                    request.source_span,
                    context,
                ));
            }
        };

        let registered =
            match register_parsed_js_module(package_id, &parsed, context.package_registry) {
                Ok(registered) => registered,
                Err(error) => {
                    return Err(js_registration_failure(
                        error,
                        js_source_path,
                        request.source_span,
                        context,
                    ));
                }
            };

        let required_runtime_imports = required_runtime_imports_from_parsed(&parsed);

        let runtime_asset = js_runtime_asset_identity(
            StablePackageIdentity::binding(PackageOrigin::ProjectLocal, &package_path),
            &request.logical_source_path,
            request.canonical_source_path,
            request.source_span,
        )
        .map_err(PreparedFailure::from)?;

        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: registered.exported_types,
            exported_free_functions: registered.exported_free_functions,
            runtime_asset: Some(runtime_asset),
            required_runtime_imports,
        }))
    }
}

/// Converts a JS module registration failure into the failure the frontend should report.
///
/// WHAT: registry invariant failures stay infrastructure failures, while rejected user declarations
///       become `invalid_external_module` diagnostics anchored at the importing source.
/// WHY: package-path and module-symbol registration reject the same user input, so one mapping keeps
///      their diagnostic identity identical.
fn js_registration_failure(
    error: JsModuleRegistrationError,
    js_source_path: PathId,
    source_span: Option<SourceSpan>,
    context: &mut ExternalImportProviderContext,
) -> PreparedFailure {
    match error {
        JsModuleRegistrationError::Infrastructure(failure) => failure.into(),
        JsModuleRegistrationError::ParserDiagnostic { parser_detail } => {
            let parser_detail =
                DiagnosticStringBuilder::new(context.string_table).intern(&parser_detail);
            let diagnostic = imports::invalid_external_module(
                js_source_path,
                InvalidExternalModuleReason::ParserDiagnostic { parser_detail },
                source_span,
            );
            context.capture_diagnostic_failure(DiagnosticBag::from(diagnostic))
        }
    }
}

// ------------------------------------------
//  Package path
// ------------------------------------------

/// Build the stable project-local package path for one JS source.
///
/// WHAT: spells the package as `@html-js/` plus the full portable logical source path.
/// WHY: two JS files at different logical locations must register distinct packages, and the
///      spelling must not vary with the checkout root the file was found in.
pub(crate) fn js_provider_package_path(logical_source_path: &PortableResourcePath) -> String {
    format!("@html-js/{}", logical_source_path.as_str())
}

// ------------------------------------------
//  Receiver method rejection
// ------------------------------------------

fn reject_receiver_methods_in_project_local_js(
    parsed: &ParsedJsModule,
    js_source_path: PathId,
    string_table: &mut DiagnosticStringBuilder,
) -> Vec<DiagnosticDraft> {
    let mut diagnostics = Vec::new();

    for receiver_method in &parsed.receiver_methods {
        let moth_name = string_table.intern(&receiver_method.moth_name);
        diagnostics.push(imports::invalid_external_module(
            js_source_path,
            InvalidExternalModuleReason::ReceiverMethod { moth_name },
            None,
        ));
    }

    diagnostics
}

// ------------------------------------------
//  Diagnostic conversion
// ------------------------------------------

fn convert_js_parser_diagnostics(
    parser_diagnostics: &[JsParserDiagnostic],
    js_source_path: PathId,
    import_span: Option<SourceSpan>,
    string_table: &mut DiagnosticStringBuilder,
) -> Vec<DiagnosticDraft> {
    let mut diagnostics = Vec::with_capacity(parser_diagnostics.len());

    for parser_diagnostic in parser_diagnostics {
        let parser_detail = string_table.intern(&parser_diagnostic.message);
        diagnostics.push(imports::invalid_external_module(
            js_source_path,
            InvalidExternalModuleReason::ParserDiagnostic { parser_detail },
            import_span,
        ));
    }

    diagnostics
}
