//! Tokenization, header parsing, dependency sorting, and AST construction for Stage 0 project
//! config files.
//!
//! WHAT: loads `config.moth` and any reachable builder/core source-backed package files through the normal
//! frontend pipeline up to AST, then hands the folded AST off to config validation.
//! WHY: config uses normal Moth syntax, so reusing tokenizer → headers → dependency sort →
//! AST keeps Stage 0 aligned with the rest of the language and lets config values benefit from
//! constant folding and type checking, including imported package constants.

use crate::build_system::create_project_modules::extract_source_code;
use crate::build_system::create_project_modules::import_scanning::extract_import_provider_references;
use crate::build_system::create_project_modules::root_validation::validate_source_package_roots;
use crate::build_system::create_project_modules::source_package_discovery::prepare_source_package_roots;
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
use crate::compiler_frontend::paths::import_resolution::ImportPathResolutionError;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::{ImportRootPolicy, ProjectPathResolver};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::projects::settings::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

pub(super) struct ParsedConfigFile {
    pub(super) ast: Ast,
    pub(super) errors: Vec<CompilerDiagnostic>,
    /// The interned source identity of the authored `config.moth` file.
    ///
    /// WHY: validation must distinguish declarations authored in config from imported support
    /// declarations so only authored declarations are treated as config keys. This is the same
    /// identity used for tokenization, duplicate diagnostic classification and AST entry identity,
    /// so authored-scope comparisons never re-canonicalize or convert back to `PathBuf`.
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
    //  Config Path Resolver
    // -------------------------
    // The canonical config path is the only filesystem identity used for resolver construction.
    // WHY: deriving the resolver directory from the already-canonical config parent avoids a
    // second canonicalization of the authored path, which could fall back to a different
    // directory when the caller-provided spelling is relative or non-canonical.
    let canonical_dir = match canonical_config.parent() {
        Some(parent) => parent.to_path_buf(),
        None => {
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(CompilerMessages::from_error(
                CompilerError::file_error(
                    &canonical_config,
                    "Canonical config path has no parent directory; cannot construct a config resolver",
                    string_table,
                ),
                string_table.clone(),
            ));
        }
    };

    let path_resolver_start = crate::timing::start_pipeline_timing();
    let prepared_source_package_roots = match prepare_source_package_roots(
        &services.frontend_surface.source_packages,
        string_table,
    ) {
        Ok(roots) => roots,
        Err(messages) => {
            log_config_stage_timing("config.parse.path_resolver", path_resolver_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(messages);
        }
    };
    if let Err(messages) =
        validate_source_package_roots(&prepared_source_package_roots, string_table)
    {
        log_config_stage_timing("config.parse.path_resolver", path_resolver_start);
        log_config_stage_timing("config.parse.total", parse_total_start);
        return Err(messages);
    }

    let project_path_resolver = match ProjectPathResolver::new(
        canonical_dir.clone(),
        canonical_dir,
        prepared_source_package_roots,
        &services.frontend_surface.source_file_kinds,
    ) {
        Ok(resolver) => {
            resolver.with_import_root_policy(ImportRootPolicy::SourceAndBindingPackagesOnly)
        }
        Err(error) => {
            log_config_stage_timing("config.parse.path_resolver", path_resolver_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };
    log_config_stage_timing("config.parse.path_resolver", path_resolver_start);

    // -------------------------
    //  Build Config Source Set
    // -------------------------
    let source_set_start = crate::timing::start_pipeline_timing();
    let source_set = match build_config_source_set(
        &canonical_config,
        services,
        &project_path_resolver,
        &mut errors,
        string_table,
    ) {
        Ok(source_set) => source_set,
        Err(messages) => {
            log_config_stage_timing("config.parse.source_set", source_set_start);
            log_config_stage_timing("config.parse.total", parse_total_start);
            return Err(messages);
        }
    };
    log_config_stage_timing("config.parse.source_set", source_set_start);

    // -------------------------
    //  Authored Config Identity
    // -------------------------
    // Construct the one exact authored `InternedPath` before file preparation and reuse it for
    // tokenization, duplicate diagnostic classification, AST entry identity and validation
    // ownership. WHY: a single interned identity keeps authored-scope classification exact without
    // re-canonicalizing or converting paths back to `PathBuf` during diagnostic handling.
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
    //  Tokenize and Prepare All Files
    // -------------------------
    let prepare_files_start = crate::timing::start_pipeline_timing();
    let mut prepared_outputs = Vec::with_capacity(source_set.len());

    for file_path in &source_set {
        let is_authored_config = file_path == &canonical_config;

        // The authored config file keeps the caller-provided spelling as its interned scope.
        // Imported support files use their resolver-derived logical path so they stay non-entry.
        let scope = if is_authored_config {
            authored_scope.clone()
        } else {
            match project_path_resolver.logical_path_for_canonical_file(file_path, string_table) {
                Ok(logical_path) => {
                    match InternedPath::try_from_filesystem_path(&logical_path, string_table) {
                        Ok(interned) => interned,
                        Err(non_utf8) => {
                            log_config_stage_timing(
                                "config.parse.prepare_files_total",
                                prepare_files_start,
                            );
                            log_config_stage_timing("config.parse.total", parse_total_start);
                            return Err(CompilerMessages::from_error(
                                CompilerError::file_error(
                                    &non_utf8.path,
                                    format!(
                                        "Config scope path {:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths.",
                                        non_utf8.path
                                    ),
                                    string_table,
                                ),
                                string_table.clone(),
                            ));
                        }
                    }
                }
                Err(error) => {
                    log_config_stage_timing(
                        "config.parse.prepare_files_total",
                        prepare_files_start,
                    );
                    log_config_stage_timing("config.parse.total", parse_total_start);
                    return Err(CompilerMessages::from_error(error, string_table.clone()));
                }
            }
        };

        // The authored config file is the entry file. Imported support files receive the
        // canonical config path as a non-matching entry sentinel so they remain non-entry.
        let entry_file_path = if is_authored_config {
            config_path
        } else {
            canonical_config.as_path()
        };

        let prepared_output = match prepare_one_config_file(
            file_path,
            scope,
            entry_file_path,
            &authored_scope,
            services,
            &mut errors,
            string_table,
        ) {
            Ok(output) => output,
            Err(messages) => {
                log_config_stage_timing("config.parse.prepare_files_total", prepare_files_start);
                log_config_stage_timing("config.parse.total", parse_total_start);
                return Err(messages);
            }
        };

        let output = match prepared_output {
            Some(output) => output,
            None => continue,
        };

        prepared_outputs.push(output);
    }
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
        Some(&project_path_resolver),
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

    let ast_result = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            import_environment: sorted.import_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
        },
        AstBuildContext {
            external_package_registry,
            style_directives: services.style_directives,
            string_table,
            entry_dir: authored_scope.clone(),
            build_profile: crate::compiler_frontend::FrontendBuildProfile::Dev,
            project_path_resolver: Some(project_path_resolver),
            path_format_config: PathStringFormatConfig::default(),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
        },
    );
    log_config_stage_timing("config.parse.ast", ast_start);

    let ast = match ast_result {
        Ok(ast) => ast,
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
//  Config Source Set
// -------------------------

/// Build the set of source files that config parsing must compile.
///
/// WHAT: starts from the authored `config.moth` and BFS-follows imports into builder/core
/// source-backed package files only. External package imports are tracked but do not add files.
/// WHY: config expressions may reference imported package constants, so those files must be
/// parsed and folded, but project-local files and relative imports are rejected by policy.
fn build_config_source_set(
    canonical_config: &Path,
    services: &ProjectConfigParseServices<'_>,
    project_path_resolver: &ProjectPathResolver,
    errors: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Vec<PathBuf>, CompilerMessages> {
    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let mut source_set: Vec<PathBuf> = Vec::new();

    queue.push_back(canonical_config.to_path_buf());

    while let Some(file_path) = queue.pop_front() {
        let canonical_file = match std::fs::canonicalize(&file_path) {
            Ok(path) => path,
            Err(error) => {
                return Err(CompilerMessages::from_error(
                    CompilerError::file_error(
                        &file_path,
                        format!("Failed to canonicalize config source path: {error}"),
                        string_table,
                    ),
                    string_table.clone(),
                ));
            }
        };

        if !visited.insert(canonical_file.clone()) {
            continue;
        }

        source_set.push(canonical_file.clone());

        let import_references = match extract_import_provider_references(
            &canonical_file,
            services.style_directives,
            string_table,
        ) {
            Ok(references) => references,
            Err(_error) => {
                // Tokenization/import extraction errors for this file will be reported during
                // `prepare_one_config_file` instead, so skip duplicates here.
                continue;
            }
        };

        for provider in &import_references {
            let import_path = &provider.path;
            // Virtual package imports (e.g. @core/math) are allowed and need no file discovery.
            if services
                .frontend_surface
                .binding_packages
                .is_virtual_package_import(import_path, string_table)
            {
                continue;
            }

            let resolved = match project_path_resolver
                .resolve_import_to_source_file_with_public_surface_fallback(
                    import_path,
                    &canonical_file,
                    string_table,
                ) {
                Ok(resolved) => resolved.path,
                Err(ImportPathResolutionError::Diagnostic(diagnostic)) => {
                    errors.push(*diagnostic);
                    continue;
                }
                Err(ImportPathResolutionError::Infrastructure(error)) => {
                    return Err(CompilerMessages::from_error(error, string_table.clone()));
                }
            };

            if !visited.contains(&resolved) {
                queue.push_back(resolved);
            }
        }
    }

    Ok(source_set)
}

// -------------------------
//  Per-File Preparation
// -------------------------

/// Tokenize and header-parse one file that belongs to the config source set.
///
/// WHAT: runs the same per-file preparation as normal module compilation, but applies
/// config-specific token validation only to the authored config file.
fn prepare_one_config_file(
    file_path: &Path,
    scope: InternedPath,
    entry_file_path: &Path,
    authored_scope: &InternedPath,
    services: &ProjectConfigParseServices<'_>,
    errors: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<Option<FileFrontendPrepareOutput>, CompilerMessages> {
    let source = extract_source_code(file_path, string_table)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    // The caller already interned the file's scope identity, so tokenization reuses it directly
    // without a second `InternedPath::try_from_filesystem_path` round-trip.
    let mut token_stream = match tokenize(
        &source,
        &scope,
        TokenizerEntryMode::SourceFile,
        services.style_directives,
        string_table,
        None,
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(*error);
            return Ok(None);
        }
    };
    token_stream.canonical_os_path = Some(file_path.to_path_buf());

    // Only the authored config file needs structural validation. Comparing the already-interned
    // scope to the authored identity keeps classification exact without filesystem recanonicalization.
    let is_authored_config = &scope == authored_scope;

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
            // Classify authored duplicate declarations by direct interned scope equality so the
            // canonical authored file is the only one remapped to a config `DuplicateKey`.
            if is_duplicate_config_header_error(&error.diagnostic)
                && &error.diagnostic.primary_location.scope == authored_scope
            {
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

    // Only validate config structural restrictions for the authored config file.
    // Imported package files may contain functions, types, and other support surfaces.
    if is_authored_config {
        errors.extend(validate_authored_config_surface(&output.headers));
    }

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
/// Imports are allowed when they pass the config import-root policy, so they are not rejected
/// here. Start-body validation happens later through `validation.rs` and AST const facts.
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
