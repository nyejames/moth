//! Header parser entry point.
//!
//! WHAT: parses individual token streams into per-file header outputs, then splits module-wide
//! header work into two explicit phases: provider-independent `PreparedHeaderSyntax` and
//! provider-dependent `BoundModuleHeaders`.
//! WHY: syntax preparation must complete before provider interfaces exist, so callers prepare
//! retained syntax first and bind it later without retokenizing or reparsing source.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::arena::{HeaderStats, TokenStats};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, InvalidDeclarationReason,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::constant_dependencies::{
    ConstantDependencyInput, add_constant_initializer_dependencies,
};
use crate::compiler_frontend::headers::dependency_canonicalization::canonicalize_local_ordering_hints;
use crate::compiler_frontend::headers::file_parser::parse_headers_in_file;
use crate::compiler_frontend::headers::import_environment::{
    ImportEnvironmentInput, prepare_import_environment,
};
use crate::compiler_frontend::headers::public_exports::build_public_exports;
use crate::compiler_frontend::headers::symbol_collection::build_module_symbols;
use crate::compiler_frontend::headers::types::HeaderParseContext;
pub use crate::compiler_frontend::headers::types::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareOutput, FileImport, FileRole,
    Header, HeaderKind, HeaderParseOptions, LocalDeclarationOrderingHint, PreparedHeaderSyntax,
    TopLevelConstFragment,
};
// HeaderExportMode is re-exported for focused AST tests that construct Header values with
// explicit export modes. Production code calls HeaderExportMode::is_public() through the
// header field, so this re-export is only reached from test modules.
#[cfg(test)]
pub use crate::compiler_frontend::headers::types::HeaderExportMode;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_config_file, file_name_is_module_root_file,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use std::path::Path;

/// Parse one tokenized file using the supplied string table.
///
/// WHAT: computes the file role, builds the header parse context, and delegates to the file parser.
/// WHY: fused frontend preparation owns local-table creation and merging in the pipeline layer,
/// while the header stage owns only header parsing against whichever table the caller provides.
pub fn parse_file_headers_with_table(
    file_tokens: &mut FileTokens,
    entry_file_path: &Path,
    options: &HeaderParseOptions,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareError> {
    let HeaderParseOptions { entry_file_id, .. } = options;

    let is_entry_file = match (*entry_file_id, file_tokens.file_id) {
        (Some(expected_id), Some(current_id)) => expected_id == current_id,
        _ => file_tokens.src_path.to_path_buf(string_table) == entry_file_path,
    };

    let source_path = file_tokens
        .canonical_os_path
        .as_deref()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| file_tokens.src_path.to_path_buf(string_table));
    // Stage 0 keeps support (`+*.moth`) and facade roots out of `ModuleRootTable`, so the
    // resolver-based check only recognises normal `@*.moth` roots. Fall back to the canonical
    // filename policy so a `+*.moth` support-package root is export-capable when prepared as
    // part of a consumer compilation.
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
        .as_ref()
        .is_some_and(|resolver| resolver.is_module_root_file(&source_path));

    let file_role = if is_entry_file {
        match options.active_root_role {
            ModuleRootRole::Normal => FileRole::ActiveModuleRoot,
            ModuleRootRole::Support | ModuleRootRole::ProjectPackageFacade => {
                FileRole::ActiveApiOnlyModuleRoot
            }
        }
    } else if is_prepared_module_root || is_module_root_file_by_name {
        FileRole::ImportedModuleRoot
    } else {
        FileRole::Normal
    };

    let mut parse_context = HeaderParseContext {
        file_role,
        is_config_file,
        string_table,
        const_template_offset,
        runtime_fragment_offset,
    };

    parse_headers_in_file(file_tokens, &mut parse_context)
}

/// Parse headers from an already-tokenized file against a local string-table fork, then merge
/// the local delta back into the module/global table and remap all StringIds in the output.
///
/// WHAT: this is the per-file header-parsing half of preparation for callers that already have
///       a `FileTokens` stream, such as config parsing that runs token-level validation first.
/// WHY: callers that need the raw token stream before header parsing still get the same local-fork
///      merge/remap behavior without repeating tokenization.
pub fn prepare_file_from_tokens(
    mut file_tokens: FileTokens,
    entry_file_path: &Path,
    options: &HeaderParseOptions,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareError> {
    let fork_source = string_table.fork_source();
    let (mut local_string_table, base_len) = fork_source.fork_for_module().into_parts();

    let file_output = parse_file_headers_with_table(
        &mut file_tokens,
        entry_file_path,
        options,
        &mut local_string_table,
        const_template_offset,
        runtime_fragment_offset,
    );

    let remap = string_table.merge_delta_from(&local_string_table, base_len);

    match file_output {
        Ok(mut output) => {
            output.remap_string_ids(&remap);
            Ok(output)
        }
        Err(mut error) => {
            error.remap_string_ids(&remap);
            Err(error)
        }
    }
}

/// Aggregate per-file frontend preparation outputs into provider-independent
/// `PreparedHeaderSyntax`.
///
/// WHAT: consumes already-remapped `FileFrontendPrepareOutput` values, builds the module-wide
/// symbol package, and collects retained header/import shells, root-activity/fragment metadata,
/// and token/header statistics.
/// WHY: this is the only phase that discovers module-wide top-level declaration syntax. It must
/// complete before provider interfaces are available so binding can consume retained syntax
/// without retokenizing or reparsing source.
pub fn prepare_header_syntax(
    prepared_files: Vec<FileFrontendPrepareOutput>,
    string_table: &mut StringTable,
) -> Result<PreparedHeaderSyntax, DiagnosticBag> {
    let module_symbols = build_module_symbols(&prepared_files, string_table)?;

    let mut headers: Vec<Header> = Vec::new();
    let mut top_level_const_fragments = Vec::new();
    let mut runtime_fragment_count = 0usize;
    let mut has_non_trivial_root_body = false;
    let mut token_stats = TokenStats::default();

    for output in &prepared_files {
        token_stats.add(&output.token_stats);
    }

    for output in prepared_files {
        headers.extend(output.headers);
        top_level_const_fragments.extend(output.top_level_const_fragments);
        runtime_fragment_count += output.runtime_fragment_count;
        has_non_trivial_root_body |= output.has_non_trivial_root_body;
    }

    let header_stats = HeaderStats::from_headers_and_symbols(&headers, &module_symbols);
    let const_fragment_count = top_level_const_fragments.len();

    Ok(PreparedHeaderSyntax {
        headers,
        top_level_const_fragments,
        entry_runtime_fragment_count: runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
    })
}

/// Bind retained `PreparedHeaderSyntax` against provider interfaces to produce
/// `BoundModuleHeaders`.
///
/// WHAT: resolves public exports, builds the import environment, canonicalizes dependency edges,
/// and completes constant initializer dependencies. Does not retokenize source or reparse
/// declaration syntax — it consumes only the retained `PreparedHeaderSyntax`.
/// WHY: these facts depend on provider interfaces and the project path resolver, so they cannot
/// be known during syntax preparation. Keeping binding separate lets the build system schedule
/// it after required providers have compiled.
pub fn bind_module_headers(
    prepared: PreparedHeaderSyntax,
    external_package_registry: &ExternalPackageRegistry,
    external_import_resolution_table: &ExternalImportResolutionTable,
    source_provider_imports: &crate::compiler_frontend::public_interface::SourceProviderImportSet<
        '_,
    >,
    project_path_resolver: Option<&ProjectPathResolver>,
    string_table: &mut StringTable,
) -> Result<BoundModuleHeaders, DiagnosticBag> {
    let PreparedHeaderSyntax {
        mut headers,
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
            external_package_registry,
            source_provider_imports,
            string_table,
        )
        .map_err(|boxed_diagnostic| {
            let mut bag = DiagnosticBag::new();
            bag.push(*boxed_diagnostic);
            bag
        })?;
    }

    let import_environment = prepare_import_environment(ImportEnvironmentInput {
        module_symbols: &mut module_symbols,
        external_package_registry,
        external_import_resolution_table,
        source_provider_imports,
        string_table,
    })
    .map_err(|messages| DiagnosticBag::from_diagnostics(messages.into_diagnostics()))?;

    canonicalize_local_ordering_hints(
        &mut headers,
        &import_environment,
        &module_symbols.file_imports_by_source,
    )?;

    let _constant_report = add_constant_initializer_dependencies(ConstantDependencyInput {
        headers: &mut headers,
        module_symbols: &module_symbols,
        import_environment: &import_environment,
        string_table,
    })?;

    Ok(BoundModuleHeaders {
        headers,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
        import_environment,
    })
}

/// Validate provider-dependent prelude collisions from retained declaration shells.
///
/// WHAT: rejects declaration names that reuse prelude functions and generic parameters that reuse
/// prelude types, preserving their authored names and locations.
/// WHY: prelude membership is provider-dependent, so syntax preparation retains these shells
/// uniformly and binding validates them once the provider interface exists. Import-alias generic
/// collisions remain syntax-owned; same-file and imported visible-type collisions remain AST-owned.
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
