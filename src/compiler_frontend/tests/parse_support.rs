//! Parser-stage frontend test support.
//!
//! WHAT: turns single-file source snippets into ASTs or typed diagnostics.
//! WHY: parser and AST diagnostics tests need a stable frontend setup without depending on HIR
//!      lowering or borrow-checker helpers.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput, AstBuildResult};
use crate::compiler_frontend::compiler_errors::{CompilerError, compiler_error_to_diagnostic};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderParseOptions, bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::resolve_module_dependencies;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenizerEntryMode};
use std::sync::Arc;

pub(crate) fn test_project_path_resolver() -> ProjectPathResolver {
    let cwd = std::env::temp_dir();
    ProjectPathResolver::new(
        cwd.clone(),
        cwd,
        crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("test path resolver should be valid")
}

pub(crate) fn parse_single_file_ast_build_result(
    source: &str,
) -> Result<(AstBuildResult, StringTable), Box<CompilerDiagnostic>> {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let file_path = std::path::PathBuf::from("@page.moth");

    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: Some(test_project_path_resolver()),
        active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
    };

    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(0)),
    )?;

    let output =
        prepare_file_from_tokens(file_tokens, &file_path, &options, &mut string_table, 0, 0)
            .map_err(|error| match error {
                crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Diagnosed(
                    error,
                ) => error.diagnostic,
                crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Infrastructure(
                    error,
                ) => panic!("single-file test preparation hit infrastructure failure: {error:?}"),
            })?;

    let prepared_syntax =
        prepare_header_syntax(vec![output], &mut string_table).map_err(|bag| {
            Box::new(
                bag.into_diagnostics()
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| {
                        compiler_error_to_diagnostic(&CompilerError::compiler_error(
                            "unknown header syntax preparation error",
                        ))
                    }),
            )
        })?;

    let headers = bind_module_headers(
        prepared_syntax,
        external_package_registry.as_ref(),
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    )
    .map_err(|bag| {
        Box::new(
            bag.into_diagnostics()
                .into_iter()
                .next()
                .unwrap_or_else(|| {
                    compiler_error_to_diagnostic(&CompilerError::compiler_error(
                        "unknown header binding error",
                    ))
                }),
        )
    })?;

    let sorted = resolve_module_dependencies(headers, &mut string_table).map_err(|bag| {
        Box::new(
            bag.into_diagnostics()
                .into_iter()
                .next()
                .unwrap_or_else(|| {
                    compiler_error_to_diagnostic(&CompilerError::compiler_error(
                        "unknown dependency sorting error",
                    ))
                }),
        )
    })?;

    let entry_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let build_result = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry,
            style_directives: &style_directives,
            string_table: &mut string_table,
            entry_dir: entry_path,
            build_profile: FrontendBuildProfile::Dev,
            project_path_resolver: Some(test_project_path_resolver()),
            path_format_config: PathStringFormatConfig::default(),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
        },
    )
    .map_err(|messages| {
        if let Some(diagnostic) = messages.first_error() {
            Box::new(diagnostic.clone())
        } else {
            panic!("frontend parsing failed without an error diagnostic")
        }
    })?;

    Ok((build_result, string_table))
}

pub(crate) fn parse_single_file_ast_result(
    source: &str,
) -> Result<(Ast, StringTable), Box<CompilerDiagnostic>> {
    parse_single_file_ast_build_result(source)
        .map(|(build_result, string_table)| (build_result.ast, string_table))
}

pub(crate) fn parse_single_file_ast(source: &str) -> (Ast, StringTable) {
    parse_single_file_ast_result(source).expect("source should parse into AST")
}

pub(crate) fn parse_single_file_ast_diagnostic(source: &str) -> CompilerDiagnostic {
    match parse_single_file_ast_result(source) {
        Ok(_) => panic!("source should fail during frontend parsing"),
        Err(diagnostic) => *diagnostic,
    }
}

/// Tokenizes one source file through a [`CompilerFrontend`] instance for test suites.
///
/// WHAT: provides a test-support helper that calls the real `tokenize_source` implementation.
/// WHY: tokenization-only tests need access to the tokenizer without taking ownership of the
///      frontend's string table, and this keeps test-only entry points out of production code.
pub(crate) fn tokenize_source_for_test(
    frontend: &mut CompilerFrontend,
    source_code: &str,
    module_path: &std::path::Path,
    tokenizer_entry_mode: TokenizerEntryMode,
) -> Result<FileTokens, Box<CompilerDiagnostic>> {
    CompilerFrontend::tokenize_source(
        &frontend.source_files,
        &frontend.style_directives,
        source_code,
        module_path,
        tokenizer_entry_mode,
        &mut frontend.string_table,
    )
    .map_err(|error| match error {
        crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Diagnosed(
            error,
        ) => error.diagnostic,
        crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Infrastructure(
            error,
        ) => panic!("tokenization test hit infrastructure failure: {error:?}"),
    })
}
