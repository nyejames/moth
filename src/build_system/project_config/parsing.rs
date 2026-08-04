//! Tokenization, header parsing, dependency sorting, and AST construction for Stage 0 project
//! config files.
//!
//! WHAT: loads one self-contained `config.moth` through the normal frontend pipeline up to AST,
//! then hands the folded AST off to config validation.
//! WHY: config uses normal Moth syntax, but bootstrap must finish before source-package discovery.
//! Reusing tokenizer → headers → dependency sort → AST preserves constant folding and type
//! checking without constructing a second import graph or package resolver.

use crate::build_system::create_project_modules::extract_source_code;
use crate::build_system::project_config::ProjectConfigParseServices;
use std::sync::Arc;

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticKind, InvalidConfigReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareOutput, Header, HeaderKind, HeaderParseOptions, bind_module_headers,
    prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::module_dependencies::resolve_module_dependencies;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::projects::settings::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;

use std::collections::HashMap;
use std::path::Path;

pub(super) struct ParsedConfigFile {
    pub(super) ast: Ast,
    pub(super) errors: Vec<CompilerDiagnostic>,
    /// The interned source identity of the authored `config.moth` file.
    ///
    /// WHY: validation uses the same identity as tokenization and AST entry construction, so
    /// authored-scope comparisons never re-canonicalize or convert back to `PathBuf`.
    pub(super) authored_scope: InternedPath,
    /// Header-owned key-name spans keyed by the full declaration path.
    ///
    /// Preserved before AST consumes the headers so key diagnostics can underline the authored
    /// name while downstream setting diagnostics keep using the declaration value location.
    pub(super) authored_key_name_locations: HashMap<InternedPath, SourceLocation>,
}

// -------------------------
//  Config Parsing Entry
// -------------------------

/// Parse `config.moth` through tokenizer → headers → dependency sort → AST.
///
/// WHY: value validation happens later, but the pipeline must surface all structural errors before
/// Stage 0 tries to apply any settings.
pub(super) fn parse_config_file(
    config_path: &Path,
    services: &ProjectConfigParseServices<'_>,
    string_table: &mut StringTable,
) -> Result<ParsedConfigFile, CompilerMessages> {
    let parse_total_start = crate::timing::start_pipeline_timing();
    let mut errors = Vec::new();

    let canonicalize_start = crate::timing::start_pipeline_timing();
    let canonical_config = match std::fs::canonicalize(config_path) {
        Ok(canonical_config) => canonical_config,
        Err(error) => {
            log_config_stage_timing("config.parse.canonicalize", canonicalize_start);
            log_config_stage_timing("config.parse.total", parse_total_start);

            return Err(CompilerMessages::from_error(
                CompilerError::file_error(
                    config_path,
                    format!("Failed to canonicalize config path: {error}"),
                    string_table,
                ),
                string_table.clone(),
            ));
        }
    };
    log_config_stage_timing("config.parse.canonicalize", canonicalize_start);

    // -------------------------
    //  Authored Config Identity
    // -------------------------
    // Construct the one exact authored `InternedPath` before file preparation and reuse it for
    // tokenization, AST entry identity and validation ownership.
    let authored_scope =
        InternedPath::try_from_filesystem_path(config_path, string_table).map_err(|non_utf8| {
            log_config_stage_timing("config.parse.total", parse_total_start);
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

    // -------------------------
    //  Tokenize and Prepare Config
    // -------------------------
    let prepare_files_start = crate::timing::start_pipeline_timing();
    let prepared_output = prepare_config_file(
        &canonical_config,
        authored_scope.clone(),
        config_path,
        services,
        &mut errors,
        string_table,
    )?;
    let prepared_outputs = prepared_output.into_iter().collect::<Vec<_>>();
    log_config_stage_timing("config.parse.prepare_files_total", prepare_files_start);

    if !errors.is_empty() {
        log_config_stage_timing("config.parse.total", parse_total_start);
        return Err(CompilerMessages::from_diagnostics(
            errors,
            string_table.clone(),
        ));
    }

    // -------------------------
    //  Header Syntax Preparation + Interface Binding
    // -------------------------
    // WHY: syntax preparation is provider-independent and binding resolves retained shells
    // against provider interfaces. Both phases share the same config-specific duplicate-key
    // diagnostic routing, so the error path is extracted once.
    let headers_start = crate::timing::start_pipeline_timing();

    let collect_header_diagnostics =
        |bag: DiagnosticBag,
         errors: &mut Vec<CompilerDiagnostic>,
         authored_scope: &InternedPath| {
            for diagnostic in bag.diagnostics() {
                if is_authored_config_duplicate(diagnostic, authored_scope) {
                    errors.push(config_diagnostic(
                        None,
                        InvalidConfigReason::DuplicateKey,
                        diagnostic.primary_location.clone(),
                    ));
                } else {
                    errors.push(diagnostic.clone());
                }
            }
        };

    let prepared = match prepare_header_syntax(prepared_outputs, string_table) {
        Ok(prepared) => prepared,
        Err(bag) => {
            collect_header_diagnostics(bag, &mut errors, &authored_scope);
            log_config_stage_timing("config.parse.headers", headers_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(CompilerMessages::from_diagnostics(
                errors,
                string_table.clone(),
            ));
        }
    };

    let bound_headers = match bind_module_headers(
        prepared,
        &services.frontend_surface.binding_packages,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderImportSet::default(),
        None,
        string_table,
    ) {
        Ok(headers) => headers,
        Err(bag) => {
            collect_header_diagnostics(bag, &mut errors, &authored_scope);
            log_config_stage_timing("config.parse.headers", headers_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(CompilerMessages::from_diagnostics(
                errors,
                string_table.clone(),
            ));
        }
    };
    log_config_stage_timing("config.parse.headers", headers_start);

    // -------------------------
    //  Dependency Sorting
    // -------------------------
    let dependency_sort_start = crate::timing::start_pipeline_timing();

    let sorted = match resolve_module_dependencies(bound_headers, string_table) {
        Ok(sorted) => sorted,
        Err(bag) => {
            errors.extend(bag.into_diagnostics());
            log_config_stage_timing("config.parse.dependency_sort", dependency_sort_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(CompilerMessages::from_diagnostics(
                errors,
                string_table.clone(),
            ));
        }
    };
    log_config_stage_timing("config.parse.dependency_sort", dependency_sort_start);

    // -------------------------
    //  Authored Key-Name Spans
    // -------------------------
    // Preserve key-name spans before AST consumes the headers. The full header path becomes the
    // declaration ID, so validation can recover the exact span without rebuilding an identity.
    let authored_key_name_locations =
        collect_authored_config_key_name_locations(&sorted.headers, &authored_scope);

    // -------------------------
    //  AST Construction
    // -------------------------
    let ast_start = crate::timing::start_pipeline_timing();

    let external_package_registry = Arc::new(services.frontend_surface.binding_packages.clone());
    let config_root = canonical_config.parent().ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error("Canonical config path has no project root"),
            string_table,
        )
    })?;
    let path_resolver = ProjectPathResolver::new(
        config_root.to_path_buf(),
        config_root.to_path_buf(),
        PreparedSourcePackageRoots::empty(),
        &services.frontend_surface.source_file_kinds,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let ast_result = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            import_environment: sorted.import_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
            external_package_registry,
            style_directives: services.style_directives,
            string_table,
            entry_dir: authored_scope.clone(),
            build_profile: crate::compiler_frontend::FrontendBuildProfile::Dev,
            project_path_resolver: Some(path_resolver),
            path_format_config: PathStringFormatConfig::default(),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
        },
    );
    log_config_stage_timing("config.parse.ast", ast_start);

    let ast = match ast_result {
        Ok(build_result) => build_result.ast,
        Err(messages) => {
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(messages);
        }
    };

    log_config_stage_timing("config.parse.total", parse_total_start);

    Ok(ParsedConfigFile {
        ast,
        errors,
        authored_scope,
        authored_key_name_locations,
    })
}

/// Record a config-parse stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  config parsing uses dotted `config.parse.*` metric names. The start
///      token is zero-sized when `timers` is off, so regular builds do not read
///      clocks for instrumentation-only measurements.
fn log_config_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

// -------------------------
//  Per-File Preparation
// -------------------------

/// Tokenize and header-parse the single authored config file.
///
/// An import is rejected from the retained structural shell before interface binding can resolve
/// a package or filesystem target.
fn prepare_config_file(
    file_path: &Path,
    scope: InternedPath,
    entry_file_path: &Path,
    services: &ProjectConfigParseServices<'_>,
    errors: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Option<FileFrontendPrepareOutput>, CompilerMessages> {
    let source = extract_source_code(file_path, string_table)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    // The caller already interned the file's scope identity, so tokenization reuses it directly
    // without a second `InternedPath::try_from_filesystem_path` round-trip.
    // Config is one self-contained file and never participates in provider binding, so the
    // placeholder file identity only stamps shells that are rejected as
    // `ConfigImportUnsupported` immediately after preparation. It is intentionally isolated
    // from every module/package identity space.
    let mut token_stream = match tokenize(
        &source,
        &scope,
        TokenizerEntryMode::SourceFile,
        services.style_directives,
        string_table,
        Some(crate::compiler_frontend::symbols::identity::FileId(0)),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(*error);
            return Ok(None);
        }
    };
    token_stream.canonical_os_path = Some(file_path.to_path_buf());

    let output = match prepare_file_from_tokens(
        token_stream,
        entry_file_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
    ) {
        Ok(output) => output,
        Err(error) => {
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
    };

    for file_import in &output.file_imports {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::ConfigImportUnsupported,
            file_import.location.clone(),
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
//  Structural Validation
// -------------------------

/// Reject unsupported surfaces in the authored `config.moth` file after header parsing has
/// normalized declaration shapes.
///
/// WHY: Stage 0 config uses frontend parsing for expression semantics, but config is not a normal
/// module. It is compile-time-only, so runtime declarations such as functions and standalone
/// templates are rejected before AST. Type aliases, structs, and choices are allowed as support
/// declarations because they can be referenced by compile-time constant expressions. Trait
/// surfaces are source-module metadata and are deliberately kept out of config.
/// Imports are rejected from `FileFrontendPrepareOutput.file_imports` before this declaration
/// validation. Start-body validation happens later through `validation.rs` and AST const facts.
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
