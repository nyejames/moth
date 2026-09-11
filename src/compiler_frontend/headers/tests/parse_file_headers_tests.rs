//! Header parsing regression tests.
//!
//! WHAT: validates top-level declaration classification, signature extraction, dependency edge
//!       generation, dependency normalization, and header-level diagnostics.
//! WHY: headers are the first compiler stage after tokenization; incorrect classification or
//!      dependency edges break everything downstream.

use super::*;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, DiagnosticBag, DiagnosticKind,
    DiagnosticLabelMessage, DiagnosticPayload, DiagnosticToken, InvalidChoiceVariantReason,
    InvalidConfigReason, InvalidDeclarationReason, InvalidDependencyClauseReason,
    InvalidFunctionSignatureReason, InvalidSignatureMemberReason, InvalidThisUsageReason,
    InvalidTypeAnnotationReason, ReservedNameOwner, RuleDiagnosticKind, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionReturnSyntax, FunctionSignatureSyntax, ReturnChannelSyntax, ReturnSlotSyntax,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalPackageRegistry, ExternalReturnAlias, ExternalSymbolId, ExternalSymbolPath,
    ExternalTypeDef, ExternalTypeId, external_success_returns,
};
use crate::compiler_frontend::headers::const_fragments::create_top_level_const_template;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelectionRange, HeaderBuildContext, HeaderExportMode,
    HeaderParseFailure, RetainedDependencyClause,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::test_support::TestSourceContext;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FilePathSyntax, FileTokens, Token, TokenKind, TokenizerEntryMode,
};
use crate::compiler_frontend::traits::syntax::ConformanceTargetKind;

fn source_span_for(source: &str, needle: &str, occurrence: usize) -> SourceSpan {
    let (start, matched) = source
        .match_indices(needle)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("expected occurrence {occurrence} of {needle:?}"));
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(start as u32, matched.len() as u32, &mut span_builder)
            .expect("test source span should fit"),
    )
}

use crate::compiler_frontend::value_mode::ValueMode;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[path = "parse_file_headers_cast_trait_tests.rs"]
mod parse_file_headers_cast_trait_tests;
#[path = "parse_file_headers_declaration_tests.rs"]
mod parse_file_headers_declaration_tests;
#[path = "parse_file_headers_default_tests.rs"]
mod parse_file_headers_default_tests;
#[path = "parse_file_headers_dependency_tests.rs"]
mod parse_file_headers_dependency_tests;
#[path = "parse_file_headers_entry_runtime_tests.rs"]
mod parse_file_headers_entry_runtime_tests;
#[path = "parse_file_headers_export_tests.rs"]
mod parse_file_headers_export_tests;
#[path = "parse_file_headers_legacy_import_tests.rs"]
mod parse_file_headers_legacy_import_tests;
#[path = "parse_file_headers_module_role_tests.rs"]
mod parse_file_headers_module_role_tests;
#[path = "parse_file_headers_multifile_tests.rs"]
mod parse_file_headers_multifile_tests;
#[path = "parse_file_headers_prepared_tests.rs"]
mod parse_file_headers_prepared_tests;
#[path = "parse_file_headers_provider_collision_tests.rs"]
mod parse_file_headers_provider_collision_tests;
#[path = "parse_file_headers_signature_choice_tests.rs"]
mod parse_file_headers_signature_choice_tests;
#[path = "parse_file_headers_source_config_tests.rs"]
mod parse_file_headers_source_config_tests;
#[path = "parse_file_headers_span_tests.rs"]
mod parse_file_headers_span_tests;
#[path = "parse_file_headers_trait_tests.rs"]
mod parse_file_headers_trait_tests;
#[path = "parse_file_headers_type_span_tests.rs"]
mod parse_file_headers_type_span_tests;

#[derive(Debug)]
struct HeaderTestDiagnostics {
    diagnostics: Vec<CompilerDiagnostic>,
    string_table: StringTable,
}

struct HeaderTestPrepareContext<'a> {
    source_id: SourceId,
    entry_file_path: &'a Path,
    options: &'a HeaderParseOptions<'a>,
    style_directives: &'a StyleDirectiveRegistry,
}

pub(crate) fn prepare_single_file(
    source: &str,
    file_path: &Path,
    entry_file_path: &Path,
    string_table: &mut StringTable,
) -> (FileFrontendPrepareOutput, ExtendedSpanBuilder) {
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let interned_path = InternedPath::try_from_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");

    let output = prepare_file_from_tokens(
        file_tokens,
        entry_file_path,
        &options,
        string_table,
        0,
        0,
        &mut span_builder,
    )
    .expect("preparation should succeed");
    (output, span_builder)
}

fn prepare_test_source_file(
    source: &str,
    file_path: &Path,
    context: &HeaderTestPrepareContext<'_>,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let interned_path = InternedPath::try_from_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        context.style_directives,
        string_table,
        context.source_id,
        span_builder,
    )
    .map_err(FileFrontendPrepareFailure::from_tokenization)?;

    prepare_file_from_tokens(
        file_tokens,
        context.entry_file_path,
        context.options,
        string_table,
        const_template_offset,
        runtime_fragment_offset,
        span_builder,
    )
}

fn prepare_tampered_path_clause(source: &str, file_path: &str) -> FileFrontendPrepareFailure {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from(file_path);
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let path_token = file_tokens
        .tokens
        .iter_mut()
        .find(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("expected a path token");
    if let TokenKind::Path(id) = &mut path_token.kind {
        *id = crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE;
    }

    match prepare_file_from_tokens(
        file_tokens,
        &file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
        &mut span_builder,
    ) {
        Ok(_) => panic!("a tampered path handle must fail preparation"),
        Err(error) => error,
    }
}

fn expect_prepare_infrastructure(error: FileFrontendPrepareFailure, case: &str) {
    match error {
        FileFrontendPrepareFailure::Infrastructure(_) => {}
        FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError { diagnostic, .. }) => {
            panic!(
                "{case}: malformed path lookup must not fabricate a user diagnostic: {:?}",
                diagnostic.payload
            )
        }
    }
}

#[test]
fn private_dependency_clause_propagates_path_lookup_infrastructure_failure() {
    expect_prepare_infrastructure(
        prepare_tampered_path_clause("@core/math sin\n", "src/helper.moth"),
        "private dependency clause",
    );
}

#[test]
fn public_dependency_clause_propagates_path_lookup_infrastructure_failure() {
    expect_prepare_infrastructure(
        prepare_tampered_path_clause("export:\n    @core/math sin\n;\n", "src/@page.moth"),
        "public dependency clause",
    );
}

#[test]
fn file_preparation_reports_wrong_table_path_lookup_as_infrastructure() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        "@core/math sin\n",
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let other_path = InternedPath::from_single_str("other.moth", &mut string_table);
    let mut other_span_builder = ExtendedSpanBuilder::new();
    let other_tokens = tokenize(
        "@other/path sin\n",
        &other_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut other_span_builder,
    )
    .expect("other file should tokenize");
    let other_path_syntax = (*other_tokens.path_syntax).clone();
    // The stream keeps its own identity; only the path table is another file's.
    let swapped = FileTokens::new_with_identity(
        file_tokens.src_path,
        file_tokens.file_id,
        file_tokens.canonical_os_path,
        file_tokens.tokens,
        other_path_syntax,
    );

    expect_prepare_infrastructure(
        match prepare_file_from_tokens(
            swapped,
            &file_path,
            &HeaderParseOptions::default(),
            &mut string_table,
            0,
            0,
            &mut span_builder,
        ) {
            Ok(_) => panic!("a wrong file-owned path table must fail preparation"),
            Err(error) => error,
        },
        "public preparation boundary",
    );
}

fn prepare_active_root_with_role(
    source: &str,
    file_path: &Path,
    active_root_role: ModuleRootRole,
    string_table: &mut StringTable,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: None,
        entry_file_role: None,
        active_root_role,
    };
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: file_path,
        options: &options,
        style_directives: &style_directives,
    };

    prepare_test_source_file(
        source,
        file_path,
        &context,
        string_table,
        0,
        0,
        span_builder,
    )
}

fn expect_aggregation_diagnostics(failure: HeaderPreparationFailure) -> DiagnosticBag {
    match failure {
        HeaderPreparationFailure::Diagnosed(bag) => bag,
        HeaderPreparationFailure::Infrastructure(error) => {
            panic!("aggregation fixture infrastructure failure: {error:?}")
        }
    }
}

/// Test helper: run both header preparation and binding, returning the raw result.
fn prepare_and_bind_headers_result(
    mut prepared_outputs: Vec<FileFrontendPrepareOutput>,
    _span_builders: &mut [ExtendedSpanBuilder],
    external_package_registry: &ExternalPackageRegistry,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    project_path_resolver: Option<&ProjectPathResolver>,
    string_table: &mut StringTable,
) -> Result<BoundModuleHeaders, DiagnosticBag> {
    let prepared = prepare_header_syntax(
        &mut prepared_outputs,
        string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    )
    .map_err(expect_aggregation_diagnostics)?;
    bind_module_headers(
        prepared,
        external_package_registry,
        external_dependency_resolution_table,
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        project_path_resolver,
        &crate::compiler_frontend::source::SourceDatabase::empty(),
        string_table,
    )
    .map_err(expect_aggregation_diagnostics)
}

pub(crate) fn parse_single_file_headers(source: &str) -> BoundModuleHeaders {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, mut span_builder) =
        prepare_single_file(source, &file_path, &file_path, &mut string_table);

    prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should parse")
}

fn parse_single_file_headers_with_warnings(
    source: &str,
) -> (
    BoundModuleHeaders,
    Vec<crate::compiler_frontend::compiler_messages::CompilerDiagnostic>,
) {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, mut span_builder) =
        prepare_single_file(source, &file_path, &file_path, &mut string_table);
    let warnings = output.warnings.clone();

    let headers = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should parse");

    (headers, warnings)
}

pub(crate) fn parse_single_file_headers_with_table(
    source: &str,
) -> (BoundModuleHeaders, StringTable) {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, mut span_builder) =
        prepare_single_file(source, &file_path, &file_path, &mut string_table);

    let headers = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should parse");

    (headers, string_table)
}

fn parse_single_file_headers_with_entry(
    source: &str,
    file_path: &str,
    entry_file_path: &str,
) -> Result<BoundModuleHeaders, HeaderTestDiagnostics> {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from(file_path);
    let entry_file_path = PathBuf::from(entry_file_path);
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");

    let prepare_result = prepare_file_from_tokens(
        file_tokens,
        &entry_file_path,
        &options,
        &mut string_table,
        0,
        0,
        &mut span_builder,
    );

    let output = match prepare_result {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
            diagnostic, ..
        })) => {
            return Err(HeaderTestDiagnostics {
                diagnostics: vec![diagnostic],
                string_table,
            });
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            panic!("header test fixture hit infrastructure failure: {error:?}")
        }
    };

    prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver,
        &mut string_table,
    )
    .map_err(|bag| HeaderTestDiagnostics {
        diagnostics: bag.into_diagnostics(),
        string_table,
    })
}

fn expect_header_error(
    result: Result<BoundModuleHeaders, HeaderTestDiagnostics>,
    message: &str,
) -> HeaderTestDiagnostics {
    match result {
        Ok(_) => panic!("{message}"),
        Err(errors) => errors,
    }
}

fn first_function_signature(headers: &BoundModuleHeaders) -> &FunctionSignatureSyntax {
    headers
        .headers
        .iter()
        .find_map(|header| match &header.kind {
            HeaderKind::Function { signature, .. } => Some(signature),
            _ => None,
        })
        .expect("expected function header")
}

fn start_function_header(headers: &BoundModuleHeaders) -> &Header {
    headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::StartFunction))
        .expect("expected start function header")
}

fn non_start_header_names(headers: &BoundModuleHeaders, string_table: &StringTable) -> Vec<String> {
    headers
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .filter_map(|header| {
            header
                .tokens
                .src_path
                .name()
                .map(|name| string_table.resolve(name).to_owned())
        })
        .collect()
}

fn symbol_tokens_in_header_body(header: &Header, string_table: &StringTable) -> Vec<String> {
    header
        .tokens
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::Symbol(symbol) => Some(string_table.resolve(symbol).to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn start_function_dependencies_stay_empty_even_with_imported_runtime_template_tokens() {
    let headers = parse_single_file_headers("func basic()\n[basic]\n");
    let start_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::StartFunction))
        .expect("expected start function header");

    assert!(
        start_header.local_ordering_hints.is_empty(),
        "start function headers must not carry dependency-graph edges"
    );
}

#[test]
fn compile_time_constant_headers_are_parsed() {
    let headers = parse_single_file_headers("theme #= \"dark\"\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Constant { .. })),
        "expected compile-time constant header"
    );
}

fn prepare_source_contract_syntax(source: &str) -> Result<PreparedHeaderSyntax, DiagnosticBag> {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _) = prepare_single_file(source, &file_path, &file_path, &mut string_table);
    prepare_header_syntax(
        &mut [output],
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
    )
    .map_err(expect_aggregation_diagnostics)
}

#[test]
fn malformed_nested_children_wrapper_constant_initializer_reports_eof_delimiter_error() {
    let result = parse_single_file_headers_with_entry(
        "broken #= [$children([:<tr>[$slot]</tr>):\n<table>\n    [$children([:<td>[$slot]</td>):[$slot]]\n</table>\n]\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "nested unterminated '$children(..)' wrapper templates should fail instead of hanging"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedEndOfFile { .. }
    )));
}

/// Verifies that header preparation and binding correctly aggregate per-file outputs from multiple source files.
///
/// WHAT: entry file contributes runtime templates, const templates, and a start function;
///       a non-entry package file contributes declarations; a module-root file contributes its
///       public surface.
/// WHY: this is the primary observable boundary introduced by the per-file refactor.
pub(crate) fn parse_multi_file_headers(
    sources: &[(String, String)],
    entry_path: &str,
) -> BoundModuleHeaders {
    let mut string_table = StringTable::new();
    let entry_file_path = PathBuf::from(entry_path);
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let prepare_context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: &entry_file_path,
        options: &options,
        style_directives: &style_directives,
    };

    let mut prepared_outputs = Vec::new();
    let mut span_builders = Vec::with_capacity(sources.len());
    let mut const_template_offset = 0usize;
    let mut runtime_fragment_offset = 0usize;

    for (source, path_str) in sources {
        let file_path = PathBuf::from(path_str);
        let mut span_builder = ExtendedSpanBuilder::new();
        let output = prepare_test_source_file(
            source,
            &file_path,
            &HeaderTestPrepareContext {
                source_id: SourceId::from_index(span_builders.len()),
                ..prepare_context
            },
            &mut string_table,
            const_template_offset,
            runtime_fragment_offset,
            &mut span_builder,
        )
        .expect("preparation should succeed");

        const_template_offset += output.const_template_count;
        runtime_fragment_offset += output.runtime_fragment_count;
        prepared_outputs.push(output);
        span_builders.push(span_builder);
    }

    prepare_and_bind_headers_result(
        prepared_outputs,
        &mut span_builders,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver,
        &mut string_table,
    )
    .expect("headers should parse")
}

/// Parse multiple files and return the full result together with collected warnings and the
/// string table so tests can inspect both success and failure paths.
fn parse_multi_file_headers_with_result(
    sources: &[(String, String)],
    entry_path: &str,
) -> (
    Result<BoundModuleHeaders, DiagnosticBag>,
    Vec<CompilerDiagnostic>,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let entry_file_path = PathBuf::from(entry_path);
    let external_package_registry = ExternalPackageRegistry::new();
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let prepare_context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: &entry_file_path,
        options: &options,
        style_directives: &style_directives,
    };

    let mut prepared_outputs = Vec::new();
    let mut span_builders = Vec::with_capacity(sources.len());
    let mut warnings = Vec::new();
    let mut diagnostic_bag = DiagnosticBag::new();
    let mut const_template_offset = 0usize;
    let mut runtime_fragment_offset = 0usize;

    for (source, path_str) in sources {
        let file_path = PathBuf::from(path_str);
        let mut span_builder = ExtendedSpanBuilder::new();
        match prepare_test_source_file(
            source,
            &file_path,
            &HeaderTestPrepareContext {
                source_id: SourceId::from_index(span_builders.len()),
                ..prepare_context
            },
            &mut string_table,
            const_template_offset,
            runtime_fragment_offset,
            &mut span_builder,
        ) {
            Ok(output) => {
                const_template_offset += output.const_template_count;
                runtime_fragment_offset += output.runtime_fragment_count;
                warnings.extend(output.warnings.clone());
                prepared_outputs.push(output);
            }
            Err(FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                warnings: file_warnings,
                diagnostic,
                ..
            })) => {
                warnings.extend(file_warnings);
                diagnostic_bag.push(diagnostic);
            }
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                panic!("multi-file header test hit infrastructure failure: {error:?}")
            }
        }
        span_builders.push(span_builder);
    }

    if diagnostic_bag.has_errors() {
        return (Err(diagnostic_bag), warnings, string_table);
    }

    let result = prepare_and_bind_headers_result(
        prepared_outputs,
        &mut span_builders,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver,
        &mut string_table,
    );

    (result, warnings, string_table)
}

fn empty_void_function_def(name: &str) -> ExternalFunctionDef {
    ExternalFunctionDef {
        name: name.to_owned(),
        parameters: Vec::new(),
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings::default(),
    }
}

fn registry_with_prelude_function_symbol(name: &'static str) -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "@test/prelude_symbol",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    let function_id = ExternalFunctionId::Synthetic(7_000);
    registry
        .register_function_at_path(
            package_id,
            ExternalSymbolPath::from_single(name),
            function_id,
            empty_void_function_def(name),
        )
        .expect("test function registration should not collide");
    registry
        .register_prelude_symbol(name, ExternalSymbolId::Function(function_id))
        .expect("prelude symbol registration should not collide");
    registry
}

fn registry_with_prelude_type_symbol(name: &'static str) -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "@test/prelude_type",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    let type_id = ExternalTypeId(7_100);
    registry
        .register_type_at_path(
            package_id,
            ExternalSymbolPath::from_single(name),
            type_id,
            ExternalTypeDef {
                name: name.to_owned(),
                package_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test type registration should not collide");
    registry
        .register_prelude_symbol(name, ExternalSymbolId::Type(type_id))
        .expect("prelude type registration should not collide");
    registry
}

fn assert_generic_dependency_name_collision(source: &str) {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        source_id: SourceId::COMPILATION_ROOT,
        entry_file_path: &file_path,
        options: &options,
        style_directives: &style_directives,
    };
    let mut span_builder = ExtendedSpanBuilder::new();
    let diagnostic = match prepare_test_source_file(
        source,
        &file_path,
        &context,
        &mut string_table,
        0,
        0,
        &mut span_builder,
    ) {
        Ok(_) => panic!("dependency names must reserve matching generic parameter names"),
        Err(FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
            diagnostic, ..
        })) => diagnostic,
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            panic!("generic-name collision became infrastructure failure: {error:?}")
        }
    };

    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidDeclaration {
                reason: InvalidDeclarationReason::GenericParameterNameCollision { .. },
                ..
            }
        ),
        "unexpected dependency-name diagnostic: {:?}",
        diagnostic.payload
    );
}

fn malformed_direct_selection_clause(range: DependencySelectionRange) -> RetainedDependencyClause {
    let provider = RetainedDependencyPath {
        span: SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start()),
        path: InternedPath::new(),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        dependency_shell_id: DependencyShellId::new(SourceId::COMPILATION_ROOT, 0),
    };
    RetainedDependencyClause {
        dependency: provider,

        binding: DependencyBindingSyntax::DirectSelections { range },
        export_mode: HeaderExportMode::Private,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedTypeAnchorSnapshot {
    span: SourceSpan,
    expected_text: String,
}

fn push_parsed_type_anchor(
    span: Option<SourceSpan>,
    expected_text: String,
    snapshots: &mut Vec<ParsedTypeAnchorSnapshot>,
) {
    snapshots.push(ParsedTypeAnchorSnapshot {
        span: span.expect("authored parsed type anchor should retain its span"),
        expected_text,
    });
}

fn collect_parsed_type_anchor_snapshots(
    parsed_type: &ParsedTypeRef,
    string_table: &StringTable,
    snapshots: &mut Vec<ParsedTypeAnchorSnapshot>,
) {
    match parsed_type {
        ParsedTypeRef::Inferred => {}
        ParsedTypeRef::Named { name, span } => {
            push_parsed_type_anchor(*span, string_table.resolve(*name).to_owned(), snapshots)
        }
        ParsedTypeRef::Qualified { path, span } => {
            push_parsed_type_anchor(*span, string_table.resolve(path[0]).to_owned(), snapshots)
        }
        ParsedTypeRef::Applied {
            base,
            arguments,
            span,
        } => {
            push_parsed_type_anchor(*span, "of".to_owned(), snapshots);
            collect_parsed_type_anchor_snapshots(base, string_table, snapshots);
            for argument in arguments {
                collect_parsed_type_anchor_snapshots(argument, string_table, snapshots);
            }
        }
        ParsedTypeRef::BuiltinBool { span }
        | ParsedTypeRef::BuiltinInt { span }
        | ParsedTypeRef::BuiltinFloat { span }
        | ParsedTypeRef::BuiltinString { span }
        | ParsedTypeRef::BuiltinChar { span } => {
            let expected_text = match parsed_type {
                ParsedTypeRef::BuiltinBool { .. } => "Bool",
                ParsedTypeRef::BuiltinInt { .. } => "Int",
                ParsedTypeRef::BuiltinFloat { .. } => "Float",
                ParsedTypeRef::BuiltinString { .. } => "String",
                ParsedTypeRef::BuiltinChar { .. } => "Char",
                _ => unreachable!("matched builtin type"),
            };
            push_parsed_type_anchor(*span, expected_text.to_owned(), snapshots);
        }
        ParsedTypeRef::Collection {
            element,
            span,
            fixed_capacity,
        } => {
            push_parsed_type_anchor(*span, "{".to_owned(), snapshots);
            collect_parsed_type_anchor_snapshots(element, string_table, snapshots);
            if let Some(capacity) = fixed_capacity {
                match capacity {
                    ParsedCollectionCapacity::Literal { value, span } => {
                        push_parsed_type_anchor(*span, value.to_string(), snapshots)
                    }
                    ParsedCollectionCapacity::BareConstant { name, span } => {
                        push_parsed_type_anchor(
                            *span,
                            string_table.resolve(*name).to_owned(),
                            snapshots,
                        )
                    }
                }
            }
        }
        ParsedTypeRef::Map { key, value, span } => {
            push_parsed_type_anchor(*span, "{".to_owned(), snapshots);
            collect_parsed_type_anchor_snapshots(key, string_table, snapshots);
            collect_parsed_type_anchor_snapshots(value, string_table, snapshots);
        }
        ParsedTypeRef::Optional { inner, span } => {
            push_parsed_type_anchor(*span, "?".to_owned(), snapshots);
            collect_parsed_type_anchor_snapshots(inner, string_table, snapshots);
        }
        ParsedTypeRef::This { .. } => {}
    }
}

fn snapshot_prepared_type_anchors(
    prepared: &FileFrontendPrepareOutput,
    string_table: &StringTable,
) -> (usize, usize, Vec<ParsedTypeAnchorSnapshot>) {
    let mut type_alias_count = 0;
    let mut inferred_count = 0;
    let mut anchors = Vec::new();
    for header in &prepared.headers {
        match &header.kind {
            HeaderKind::TypeAlias { target } => {
                type_alias_count += 1;
                collect_parsed_type_anchor_snapshots(target, string_table, &mut anchors);
            }
            HeaderKind::Constant { declaration } => {
                inferred_count += usize::from(matches!(
                    declaration.type_annotation,
                    ParsedTypeRef::Inferred
                ));
                collect_parsed_type_anchor_snapshots(
                    &declaration.type_annotation,
                    string_table,
                    &mut anchors,
                );
            }
            _ => {}
        }
    }
    (type_alias_count, inferred_count, anchors)
}
