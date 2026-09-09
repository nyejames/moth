//! Header parser entry point.
//!
//! WHAT: parses individual token streams into per-file header outputs, then splits module-wide
//! header work into two explicit phases: provider-independent `PreparedHeaderSyntax` and
//! provider-dependent `BoundModuleHeaders`.
//! WHY: syntax preparation must complete before provider interfaces exist, so callers prepare
//! retained syntax first and bind it later without retokenizing or reparsing source.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::arena::{HeaderStats, TokenStats};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticBag, InvalidConfigReason,
    InvalidDeclarationReason,
};
pub(crate) use crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::{
    BindingEnvironmentInput, prepare_binding_environment,
};
use crate::compiler_frontend::headers::constant_dependencies::{
    ConstantDependencyInput, add_constant_initializer_dependencies,
};
use crate::compiler_frontend::headers::dependency_canonicalization::canonicalize_local_ordering_hints;
use crate::compiler_frontend::headers::file_parser::parse_headers_in_file;
use crate::compiler_frontend::headers::public_exports::build_public_exports;
use crate::compiler_frontend::headers::symbol_collection::build_module_symbols;
use crate::compiler_frontend::headers::types::HeaderParseContext;
pub(crate) use crate::compiler_frontend::headers::types::SourcePreparationDelta;
pub use crate::compiler_frontend::headers::types::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareFailure,
    FileFrontendPrepareOutput, FileRole, Header, HeaderKind, HeaderParseOptions,
    LocalDeclarationOrderingHint, LocalDeclarationOrderingHintOrigin, PreparedHeaderSyntax,
    RetainedDependencyClause, TopLevelConstFragment,
};
// HeaderExportMode is re-exported for focused AST tests that construct Header values with
// explicit export modes. Production code calls HeaderExportMode::is_public() through the
// header field, so this re-export is only reached from test modules.
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    find_config_qualifier_marker, normalize_source_build_config_contract,
};
#[cfg(test)]
pub use crate::compiler_frontend::headers::types::HeaderExportMode;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceDatabase, SourceId, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_config_file, file_name_is_module_root_file,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use std::mem;
use std::path::Path;

/// Parse one tokenized file using the supplied string table.
///
/// WHAT: computes the file role, builds the header parse context, and delegates to the file parser.
/// WHY: fused frontend preparation owns local-table creation and merging in the pipeline layer,
/// while the header stage owns only header parsing against whichever table the caller provides.
///
/// The caller lends the source's original live span builder so per-file diagnostics and warnings
/// can retain their exact primary and related byte ranges before the result crosses a preparation boundary.
pub fn parse_file_headers_with_table(
    file_tokens: &mut FileTokens,
    entry_file_path: &Path,
    options: &HeaderParseOptions<'_>,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let file_id = file_tokens.file_id;
    let HeaderParseOptions { entry_file_id, .. } = options;

    let is_entry_file = entry_file_id.map_or_else(
        || file_tokens.src_path.to_path_buf(string_table) == entry_file_path,
        |expected_id| expected_id == file_id,
    );

    let source_path = file_tokens
        .canonical_os_path
        .as_deref()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| file_tokens.src_path.to_path_buf(string_table));
    // Directory Stage 0 supplies normal and support roots through `ModuleRootTable`. Keep the
    // canonical filename check as a fallback for synthetic or otherwise unindexed preparation so
    // a `+*.moth` support-package root remains export-capable in those contexts too.
    let is_module_root_file_by_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(file_name_is_module_root_file);
    let is_config_file = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(file_name_is_config_file);
    let is_prepared_module_root = options
        .project_path_resolver
        .is_some_and(|resolver| resolver.is_module_root_file(&source_path));

    let file_role = if is_entry_file {
        options
            .entry_file_role
            .unwrap_or(match options.active_root_role {
                ModuleRootRole::Normal => FileRole::ActiveModuleRoot,
                ModuleRootRole::Support | ModuleRootRole::ProjectPackageFacade => {
                    FileRole::ActiveApiOnlyModuleRoot
                }
            })
    } else if is_prepared_module_root || is_module_root_file_by_name {
        FileRole::ImportedModuleRoot
    } else {
        FileRole::Normal
    };

    let mut parse_context = HeaderParseContext {
        file_role,
        is_config_file,
        string_table,
        span_builder,
        const_template_offset,
        runtime_fragment_offset,
    };
    let file_output = parse_headers_in_file(file_tokens, file_id, &mut parse_context);
    capture_preparation_spans(file_output, file_id, span_builder)
}

/// Attach exact primary and related spans to warnings and diagnostics emitted for one source file.
///
/// WHAT: encodes the already-produced byte bounds at the header entry boundary, where the
///       caller still owns the source's original span builder.
/// WHY: this covers direct, pipeline and config preparation without threading span state through
///       shared declaration parsers. Infrastructure capture failures stay on the existing
///       preparation lane and cannot invent a range end.
fn capture_preparation_spans(
    mut file_output: Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure>,
    file_id: SourceId,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    match &mut file_output {
        Ok(output) => {
            for warning in &mut output.warnings {
                warning
                    .capture_preparation_span(file_id, span_builder)
                    .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            }
        }
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            for warning in &mut error.warnings {
                warning
                    .capture_preparation_span(file_id, span_builder)
                    .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            }
            error
                .diagnostic
                .capture_preparation_span(file_id, span_builder)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
        }
        Err(FileFrontendPrepareFailure::Infrastructure(_)) => {}
    }

    file_output
}

/// Parse headers from an already-tokenized file against a local string-table fork, then merge
/// the local delta back into the module/global table and remap all StringIds in the output.
///
/// WHAT: this is the per-file header-parsing half of preparation for callers that already ran
///       tokenization, such as config parsing that runs token-level validation first.
/// WHY: the caller lends the original source span builder so this boundary can capture diagnostic
///      ranges, then retains that same builder for later retained-token span resolution.
pub(crate) fn prepare_file_from_tokens(
    mut file_tokens: FileTokens,
    entry_file_path: &Path,
    options: &HeaderParseOptions<'_>,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let fork_source = string_table.fork_source();
    let (mut local_string_table, base_len) = fork_source.fork_for_module().into_parts();

    let file_output = parse_file_headers_with_table(
        &mut file_tokens,
        entry_file_path,
        options,
        &mut local_string_table,
        const_template_offset,
        runtime_fragment_offset,
        span_builder,
    );

    let remap = string_table.merge_delta_from(&local_string_table, base_len);

    match file_output {
        Ok(mut output) => {
            output
                .remap_string_ids(&remap)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            output
                .freeze_path_syntax(string_table)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            Ok(output)
        }
        Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
            error.remap_string_ids(&remap);
            Err(FileFrontendPrepareFailure::Diagnosed(error))
        }
        Err(error @ FileFrontendPrepareFailure::Infrastructure(_)) => Err(error),
    }
}

/// Failure lanes for aggregation while source span storage remains live.
#[derive(Debug)]
pub(crate) enum HeaderPreparationFailure {
    Diagnosed(DiagnosticBag),
    Infrastructure(CompilerError),
}

impl From<DiagnosticBag> for HeaderPreparationFailure {
    fn from(diagnostics: DiagnosticBag) -> Self {
        Self::Diagnosed(diagnostics)
    }
}

/// Aggregate per-file frontend preparation outputs into provider-independent
/// `PreparedHeaderSyntax`.
///
/// WHAT: moves declaration and dependency facts out of already-remapped per-file outputs to build
/// the module-wide symbol package, retained headers, fragments and statistics. The caller keeps
/// each source's live span builder, including when aggregation returns diagnostics.
/// WHY: this is the only phase that discovers module-wide top-level declaration syntax. It must
/// complete before provider interfaces are available so binding can consume retained syntax
/// without retokenizing or reparsing source.
pub fn prepare_header_syntax(
    prepared_files: &mut [FileFrontendPrepareOutput],
    string_table: &mut StringTable,
    capture: &mut impl FnMut(SourceId, &mut CompilerDiagnostic) -> Result<(), CompilerError>,
) -> Result<PreparedHeaderSyntax, HeaderPreparationFailure> {
    let source_build_config_contracts =
        collect_source_build_config_contracts(prepared_files, string_table, capture)?;
    let module_symbols = build_module_symbols(prepared_files, string_table, capture)?;

    let mut headers: Vec<Header> = Vec::new();
    let mut top_level_const_fragments = Vec::new();
    let mut runtime_fragment_count = 0usize;
    let mut has_non_trivial_root_body = false;
    let mut token_stats = TokenStats::default();

    for output in prepared_files {
        token_stats.add(&output.token_stats);
        headers.extend(mem::take(&mut output.headers));
        top_level_const_fragments.extend(mem::take(&mut output.top_level_const_fragments));
        runtime_fragment_count += output.runtime_fragment_count;
        has_non_trivial_root_body |= output.has_non_trivial_root_body;
    }

    let header_stats = HeaderStats::from_headers_and_symbols(&headers, &module_symbols);
    let const_fragment_count = top_level_const_fragments.len();

    Ok(PreparedHeaderSyntax {
        headers,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count: runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
    })
}

/// Find retained parameter/field defaults and body tokens carrying `#Config`.
///
/// Header preparation has already parsed declaration shells for signatures and record payloads,
/// while function/start bodies remain token slices. Inspecting both retained representations keeps
/// illegal nested placements ahead of AST without adding a recursive expression walk.
pub(crate) fn find_config_qualifier_marker_in_header(
    header: &Header,
    string_table: &StringTable,
) -> Option<(SourceLocation, bool)> {
    let retained_member_marker = match &header.kind {
        HeaderKind::Constant { declaration } => {
            find_config_qualifier_marker(&declaration.initializer_tokens, string_table)
        }
        HeaderKind::Function { signature, .. } => signature
            .parameters
            .iter()
            .find_map(|parameter| {
                find_config_qualifier_marker(&parameter.default_tokens, string_table)
            }),
        HeaderKind::Struct { fields, .. } => fields
            .iter()
            .find_map(|field| find_config_qualifier_marker(&field.default_tokens, string_table)),
        HeaderKind::Choice { variants, .. } => variants.iter().find_map(|variant| {
            let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
                fields,
            } = &variant.payload
            else {
                return None;
            };
            fields
                .iter()
                .find_map(|field| find_config_qualifier_marker(&field.default_tokens, string_table))
        }),
        HeaderKind::Trait { declaration } => declaration
            .requirements
            .iter()
            .flat_map(|requirement| requirement.signature.parameters.iter())
            .find_map(|parameter| {
                find_config_qualifier_marker(&parameter.default_tokens, string_table)
            }),
        _ => None,
    };

    retained_member_marker
        .or_else(|| find_config_qualifier_marker(&header.tokens.tokens, string_table))
}

/// Collect source-owned `#Config` contract shells and reject all non-declaration placements.
///
/// The declaration shell itself remains in `PreparedHeaderSyntax::headers` for the later
/// config-resolution barrier. This pass only creates the provider-independent contract carrier and
/// performs flat retained-token placement checks; it never constructs an expression or turns the
/// source contract into a module symbol.
fn collect_source_build_config_contracts(
    prepared_files: &[FileFrontendPrepareOutput],
    string_table: &mut StringTable,
    capture: &mut impl FnMut(SourceId, &mut CompilerDiagnostic) -> Result<(), CompilerError>,
) -> Result<Vec<SourceBuildConfigContract>, HeaderPreparationFailure> {
    let mut contracts = Vec::new();
    let mut diagnostics = DiagnosticBag::new();

    for output in prepared_files {
        // The project config source has its own direct-project qualifier consumer. Leaving its
        // top-level qualifier diagnostics on that path keeps config.moth semantics unchanged.
        let is_config_file = output
            .source_file
            .name()
            .is_some_and(|name| file_name_is_config_file(string_table.resolve(name)));
        if is_config_file {
            continue;
        }

        for header in &output.headers {
            let report_marker = |location: SourceLocation, adjacent: bool| {
                if adjacent {
                    CompilerDiagnostic::invalid_config_reason(
                        header.tokens.src_path.name(),
                        InvalidConfigReason::ConfigQualifierInvalidPlacement,
                        location,
                    )
                } else {
                    CompilerDiagnostic::common_syntax_mistake(
                        CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                        location,
                    )
                }
            };

            if let Some((location, adjacent)) =
                find_config_qualifier_marker_in_header(header, string_table)
            {
                let mut diagnostic = report_marker(location, adjacent);
                capture(output.file_id, &mut diagnostic)
                    .map_err(HeaderPreparationFailure::Infrastructure)?;
                diagnostics.push(diagnostic);
                continue;
            }

            let HeaderKind::Constant { declaration } = &header.kind else {
                continue;
            };
            let Some(qualifier) = &declaration.config_qualifier else {
                continue;
            };
            let Some(name) = header.tokens.src_path.name() else {
                let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
                    None,
                    InvalidConfigReason::ConfigContractNameInvalid,
                    header.name_location.clone(),
                );
                capture(output.file_id, &mut diagnostic)
                    .map_err(HeaderPreparationFailure::Infrastructure)?;
                diagnostics.push(diagnostic);
                continue;
            };

            match normalize_source_build_config_contract(
                name,
                header.name_location.clone(),
                SourceSpan::new(output.file_id, declaration.span),
                qualifier,
                &declaration.initializer_tokens,
                string_table,
            ) {
                Ok(contract) => contracts.push(contract),
                Err(mut diagnostic) => {
                    capture(output.file_id, &mut diagnostic)
                        .map_err(HeaderPreparationFailure::Infrastructure)?;
                    diagnostics.push(*diagnostic);
                }
            }
        }
    }

    if diagnostics.has_errors() {
        return Err(HeaderPreparationFailure::Diagnosed(diagnostics));
    }
    Ok(contracts)
}

/// Bind retained `PreparedHeaderSyntax` against provider interfaces to produce
/// `BoundModuleHeaders`.
///
/// WHAT: resolves public exports, builds the header binding environment, canonicalizes dependency edges,
/// and completes constant initializer dependencies. Does not retokenize source or reparse
/// declaration syntax — it consumes only the retained `PreparedHeaderSyntax`.
/// WHY: these facts depend on provider interfaces and the project path resolver, so they cannot
/// be known during syntax preparation. Keeping binding separate lets the build system schedule
/// it after required providers have compiled.
pub(in crate::compiler_frontend) fn bind_module_headers(
    prepared: PreparedHeaderSyntax,
    external_package_registry: &ExternalPackageRegistry,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<
        '_,
    >,
    project_path_resolver: Option<&ProjectPathResolver>,
    source_files: &SourceDatabase,
    string_table: &mut StringTable,
) -> Result<BoundModuleHeaders, DiagnosticBag> {
    let PreparedHeaderSyntax {
        mut headers,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        mut module_symbols,
    } = prepared;

    validate_prelude_declaration_shells(&headers, external_package_registry, string_table)?;

    if let Some(resolver) = project_path_resolver {
        build_public_exports(
            &mut module_symbols,
            &headers,
            resolver,
            source_files,
            external_package_registry,
            source_provider_dependencies,
            string_table,
        )
        .map_err(|boxed_diagnostic| {
            let mut bag = DiagnosticBag::new();
            bag.push(*boxed_diagnostic);
            bag
        })?;
    }

    let binding_environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &module_symbols,
        external_package_registry,
        external_dependency_resolution_table,
        source_provider_dependencies,
        source_files,
        string_table,
    })
    .map_err(|messages| DiagnosticBag::from_diagnostics(messages.into_diagnostics()))?;

    canonicalize_local_ordering_hints(
        &mut headers,
        &binding_environment,
        &module_symbols.file_dependency_clauses_by_source,
        &module_symbols.dependency_selections_by_source,
        string_table,
    )?;

    let _constant_report = add_constant_initializer_dependencies(ConstantDependencyInput {
        headers: &mut headers,
        module_symbols: &module_symbols,
        binding_environment: &binding_environment,
        string_table,
    })?;

    Ok(BoundModuleHeaders {
        headers,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
        binding_environment,
    })
}

/// Validate provider-dependent prelude collisions from retained declaration shells.
///
/// WHAT: rejects declaration names that reuse prelude functions and generic parameters that reuse
/// prelude types, preserving their authored names and locations.
/// WHY: prelude membership is provider-dependent, so syntax preparation retains these shells
/// uniformly and binding validates them once the provider interface exists. Dependency-alias generic
/// collisions remain syntax-owned; same-file and dependency-bound visible-type collisions remain AST-owned.
fn validate_prelude_declaration_shells(
    headers: &[Header],
    external_package_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
) -> Result<(), DiagnosticBag> {
    let mut collision_bag = DiagnosticBag::new();
    for header in headers {
        if let Some(name) = header.tokens.src_path.name()
            && external_package_registry.is_prelude_function(string_table.resolve(name))
        {
            collision_bag.push(CompilerDiagnostic::reserved_builtin_name(
                name,
                header.name_location.to_owned(),
            ));
        }

        let generic_parameters = match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => Some(generic_parameters),
            _ => None,
        };

        if let Some(generic_parameters) = generic_parameters {
            for parameter in &generic_parameters.parameters {
                if !external_package_registry.is_prelude_type(string_table.resolve(parameter.name))
                {
                    continue;
                }

                collision_bag.push(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericParameterNameCollision {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.location.to_owned(),
                ));
            }
        }
    }

    if collision_bag.has_errors() {
        return Err(collision_bag);
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/parse_file_headers_tests.rs"]
pub(crate) mod parse_file_headers_tests;
