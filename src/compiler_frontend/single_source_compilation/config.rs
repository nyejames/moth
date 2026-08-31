//! Project config compilation service.
//!
//! WHAT: the compiler-owned stage sequence for one authored `config.moth` — tokenization,
//!       declaration-shell preparation, interface binding for the single authored source, local
//!       declaration ordering and AST semantics — stopping at folded AST values.
//! WHY:  config is written in normal Moth syntax but must bootstrap before source-package discovery
//!       exists, so it needs a shorter path than canonical module compilation. That path is a named
//!       compiler service rather than a build-owned stage sequence: the build system supplies the
//!       source and consumes folded values, and never composes preparation, binding, ordering or
//!       AST itself.
//!
//! The service produces no HIR, borrow facts, link facts or public interface. It owns the config
//! dialect surface — which declaration shapes and dependency clauses `config.moth` accepts — and
//! the authored key-name spans config diagnostics underline. Config key schema and the application
//! of folded values to project settings stay build-owned.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticKind, InvalidConfigReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, Header, HeaderKind, HeaderParseOptions,
    bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;

use std::collections::HashMap;
use std::path::Path;

/// One authored config source and the capability surface it compiles against.
pub(crate) struct ConfigCompilationRequest<'a> {
    /// The config path exactly as the project spelled it.
    ///
    /// WHY: this is the authored identity every config diagnostic reports and every authored-scope
    ///      comparison uses, so it must not be replaced by the canonical form.
    pub(crate) authored_path: &'a Path,
    /// The canonical filesystem path the authored config resolved to.
    pub(crate) canonical_path: &'a Path,
    pub(crate) source_code: &'a str,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) binding_packages: &'a ExternalPackageRegistry,
}

/// The folded config source a caller validates and applies.
pub(crate) struct CompiledConfigSource {
    pub(crate) ast: Ast,
    /// The interned source identity of the authored `config.moth` file.
    ///
    /// WHY: validation uses the same identity as tokenization and AST entry construction, so
    ///      authored-scope comparisons never re-canonicalize or convert back to `PathBuf`.
    pub(crate) authored_scope: InternedPath,
    /// Header-owned key-name spans keyed by the full declaration path.
    ///
    /// Preserved before AST consumes the headers so key diagnostics can underline the authored
    /// name while downstream setting diagnostics keep using the declaration value location.
    pub(crate) authored_key_name_locations: HashMap<InternedPath, SourceLocation>,
}

/// Compile one authored `config.moth` to folded AST values.
///
/// Every stage failure is returned as diagnostics; nothing partial is handed back.
pub(crate) fn compile_config_source(
    request: ConfigCompilationRequest<'_>,
    string_table: &mut StringTable,
) -> Result<CompiledConfigSource, CompilerMessages> {
    // Construct the one exact authored `InternedPath` before file preparation and reuse it for
    // tokenization, AST entry identity and diagnostic ownership.
    let authored_scope = InternedPath::try_from_filesystem_path(request.authored_path, string_table)
        .map_err(|non_utf8| {
            CompilerMessages::from_error(
                CompilerError::file_error(
                    &non_utf8.path,
                    format!(
                        "Config path {:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths.",
                        non_utf8.path
                    ),
                    string_table,
                ),
                string_table.clone(),
            )
        })?;

    // 1. Tokenize and prepare the single authored file, then apply the config dialect surface.
    let mut surface_errors = Vec::new();
    let prepared_file =
        prepare_config_file(&request, &authored_scope, &mut surface_errors, string_table)?;
    if !surface_errors.is_empty() {
        return Err(CompilerMessages::from_diagnostics(
            surface_errors,
            string_table.clone(),
        ));
    }
    let prepared_file = prepared_file.into_iter().collect::<Vec<_>>();

    // 2. Aggregate retained syntax and bind it against the builder's provider interfaces.
    //
    // WHY: syntax preparation is provider-independent and binding resolves retained shells against
    //      provider interfaces. Both phases share the same duplicate-key diagnostic routing, so the
    //      error path is classified once.
    let bound_headers =
        match prepare_header_syntax(prepared_file, string_table).and_then(|prepared| {
            bind_module_headers(
                prepared,
                request.binding_packages,
                &ExternalImportResolutionTable::default(),
                &SourceProviderDependencySet::default(),
                None,
                string_table,
            )
        }) {
            Ok(headers) => headers,
            Err(bag) => {
                return Err(CompilerMessages::from_diagnostics(
                    classify_header_diagnostics(bag, &authored_scope),
                    string_table.clone(),
                ));
            }
        };

    // 3. Order local declarations.
    let sorted =
        resolve_module_dependencies(bound_headers, &ContentSourceTargets::empty(), string_table)
            .map_err(|bag| {
                CompilerMessages::from_diagnostics(bag.into_diagnostics(), string_table.clone())
            })?;

    // 4. Preserve key-name spans before AST consumes the headers. The full header path becomes the
    //    declaration ID, so validation can recover the exact span without rebuilding an identity.
    let authored_key_name_locations =
        collect_authored_config_key_name_locations(&sorted.headers, &authored_scope);

    // 5. Fold the ordered declarations. Config stops here: no HIR, borrow facts or interface.

    let ast = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: std::sync::Arc::new(request.binding_packages.clone()),
            style_directives: request.style_directives,
            string_table,
            entry_dir: authored_scope.clone(),
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: None,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Config,
        },
    )?
    .ast;

    Ok(CompiledConfigSource {
        ast,
        authored_scope,
        authored_key_name_locations,
    })
}

// -------------------------
//  Per-File Preparation
// -------------------------

/// Tokenize and header-parse the single authored config file, then apply the config dialect surface.
///
/// Dependency clauses are rejected from the retained structural shell before interface binding can
/// resolve a package or filesystem target.
fn prepare_config_file(
    request: &ConfigCompilationRequest<'_>,
    authored_scope: &InternedPath,
    errors: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Option<FileFrontendPrepareOutput>, CompilerMessages> {
    // The authored scope identity is already interned, so tokenization reuses it directly without a
    // second `InternedPath::try_from_filesystem_path` round-trip.
    // Config is one self-contained file and never participates in provider binding, so the
    // placeholder file identity only stamps shells that are rejected as `ConfigImportUnsupported`
    // immediately after preparation. It is intentionally isolated from every module/package
    // identity space.
    let mut token_stream = match tokenize(
        request.source_code,
        authored_scope,
        TokenizerEntryMode::SourceFile,
        request.style_directives,
        string_table,
        Some(FileId(0)),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(*error);
            return Ok(None);
        }
    };
    token_stream.canonical_os_path = Some(request.canonical_path.to_path_buf());

    let output = match prepare_file_from_tokens(
        token_stream,
        request.authored_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
    ) {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            errors.extend(error.warnings);
            if is_duplicate_config_header_error(&error.diagnostic) {
                errors.push(config_diagnostic(
                    None,
                    InvalidConfigReason::DuplicateKey,
                    error.diagnostic.primary_location.clone(),
                ));
            } else {
                errors.push(*error.diagnostic);
            }
            return Ok(None);
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };

    for dependency_clause in &output.file_dependency_clauses {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::ConfigImportUnsupported,
            dependency_clause.location.clone(),
        ));
    }
    for file_reference in output.structural_file_references.iter() {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::FileValuePathUnsupported,
            file_reference.location.clone(),
        ));
    }
    errors.extend(validate_authored_config_surface(&output.headers));

    Ok(Some(output))
}

// -------------------------
//  Authored Key-Name Spans
// -------------------------

/// Collect the authored key-name spans for config key-identity diagnostics.
///
/// Imported support declarations are excluded because they are not config entries.
fn collect_authored_config_key_name_locations(
    headers: &[Header],
    authored_scope: &InternedPath,
) -> HashMap<InternedPath, SourceLocation> {
    let mut key_name_locations = HashMap::new();
    for header in headers {
        let HeaderKind::Constant { .. } = &header.kind else {
            continue;
        };
        if header.source_file != *authored_scope {
            continue;
        }
        key_name_locations.insert(
            header.tokens.src_path.to_owned(),
            header.name_location.clone(),
        );
    }
    key_name_locations
}

// -------------------------
//  Config Dialect Surface
// -------------------------

/// Reject unsupported surfaces in the authored `config.moth` file after header parsing has
/// normalized declaration shapes.
///
/// WHY: config uses frontend parsing for expression semantics, but config is not a normal module.
/// It is compile-time-only, so runtime declarations such as functions and standalone templates are
/// rejected before AST. Type aliases, structs, and choices are allowed as support declarations
/// because they can be referenced by compile-time constant expressions. Trait surfaces are
/// source-module metadata and are deliberately kept out of config.
/// Dependency clauses are rejected from `FileFrontendPrepareOutput.file_dependency_clauses` before
/// this declaration validation. Start-body validation happens later through the caller's config
/// validation and AST const facts.
fn validate_authored_config_surface(headers: &[Header]) -> Vec<CompilerDiagnostic> {
    let mut errors = Vec::new();

    for header in headers {
        let reason = match &header.kind {
            HeaderKind::Function { .. } => Some(InvalidConfigReason::FunctionUnsupported),
            HeaderKind::ConstTemplate { .. } => {
                Some(InvalidConfigReason::StandaloneTemplateUnsupported)
            }
            HeaderKind::Trait { .. } => Some(InvalidConfigReason::TraitDeclarationUnsupported),
            HeaderKind::TraitConformance { .. } => {
                Some(InvalidConfigReason::TraitConformanceUnsupported)
            }
            HeaderKind::TraitIncompatibility { .. } => {
                Some(InvalidConfigReason::TraitIncompatibilityUnsupported)
            }
            HeaderKind::Constant { .. }
            | HeaderKind::StartFunction
            | HeaderKind::Struct { .. }
            | HeaderKind::Choice { .. }
            | HeaderKind::TypeAlias { .. } => None,
        };

        if let Some(reason) = reason {
            errors.push(config_diagnostic(
                header.tokens.src_path.name(),
                reason,
                header.name_location.clone(),
            ));
        }
    }

    errors
}

// -------------------------
//  Duplicate Classification
// -------------------------

/// Re-route an authored duplicate declaration to the config key vocabulary.
fn classify_header_diagnostics(
    bag: DiagnosticBag,
    authored_scope: &InternedPath,
) -> Vec<CompilerDiagnostic> {
    bag.into_diagnostics()
        .into_iter()
        .map(|diagnostic| {
            if is_authored_config_duplicate(&diagnostic, authored_scope) {
                config_diagnostic(
                    None,
                    InvalidConfigReason::DuplicateKey,
                    diagnostic.primary_location.clone(),
                )
            } else {
                diagnostic
            }
        })
        .collect()
}

fn is_duplicate_config_header_error(diagnostic: &CompilerDiagnostic) -> bool {
    matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    )
}

fn is_authored_config_duplicate(
    diagnostic: &CompilerDiagnostic,
    authored_scope: &InternedPath,
) -> bool {
    // Classify authored duplicate declarations by direct interned scope equality.
    // WHY: the authored config file was tokenized with this exact interned identity, so a
    // duplicate declaration whose primary location shares that scope is an authored duplicate.
    // Comparing interned identity avoids converting paths back to `PathBuf` or canonicalizing
    // during diagnostic handling.
    is_duplicate_config_header_error(diagnostic)
        && diagnostic.primary_location.scope == *authored_scope
}

fn config_diagnostic(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(key, reason, location)
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
