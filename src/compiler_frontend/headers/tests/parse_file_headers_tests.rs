//! Header parsing regression tests.
//!
//! WHAT: validates top-level declaration classification, signature extraction, dependency edge
//!       generation, dependency normalization, and header-level diagnostics.
//! WHY: headers are the first compiler stage after tokenization; incorrect classification or
//!      dependency edges break everything downstream.

use super::*;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, DiagnosticBag, DiagnosticKind, DiagnosticPayload,
    InvalidChoiceVariantReason, InvalidDeclarationReason, InvalidDependencyClauseReason,
    InvalidFunctionSignatureReason, InvalidSignatureMemberReason, InvalidThisUsageReason,
    InvalidTypeAnnotationReason, ReservedNameOwner, RuleDiagnosticKind, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionReturnSyntax, FunctionSignatureSyntax, ReturnChannelSyntax, ReturnSlotSyntax,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalPackageRegistry, ExternalReturnAlias, ExternalSymbolId, ExternalSymbolPath,
    ExternalTypeDef, ExternalTypeId, external_success_returns,
};
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelectionRange, HeaderExportMode, RetainedDependencyClause,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    FilePathSyntax, FileTokens, SourceLocation, Token, TokenKind, TokenizerEntryMode,
};
use crate::compiler_frontend::traits::syntax::TraitThisUsage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
struct HeaderTestDiagnostics {
    diagnostics: Vec<CompilerDiagnostic>,
    string_table: StringTable,
}

struct HeaderTestPrepareContext<'a> {
    entry_file_path: &'a Path,
    options: &'a HeaderParseOptions,
    style_directives: &'a StyleDirectiveRegistry,
}

pub(crate) fn prepare_single_file(
    source: &str,
    file_path: &Path,
    entry_file_path: &Path,
    string_table: &mut StringTable,
) -> FileFrontendPrepareOutput {
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let interned_path = InternedPath::try_from_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        Some(FileId(0)),
    )
    .expect("tokenization should succeed");

    prepare_file_from_tokens(file_tokens, entry_file_path, &options, string_table, 0, 0)
        .expect("preparation should succeed")
}

fn prepare_test_source_file(
    source: &str,
    file_path: &Path,
    context: &HeaderTestPrepareContext<'_>,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let interned_path = InternedPath::try_from_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let file_tokens = match tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        context.style_directives,
        string_table,
        Some(FileId(0)),
    ) {
        Ok(file_tokens) => file_tokens,
        Err(diagnostic) => {
            return Err(FileFrontendPrepareFailure::Diagnosed(
                FileFrontendPrepareError {
                    warnings: Vec::new(),
                    diagnostic,
                },
            ));
        }
    };

    prepare_file_from_tokens(
        file_tokens,
        context.entry_file_path,
        context.options,
        string_table,
        const_template_offset,
        runtime_fragment_offset,
    )
}

#[test]
fn dependency_shell_without_retained_file_identity_fails_preparation() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let file_tokens = tokenize(
        "@core/math\n",
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    let error = match prepare_file_from_tokens(
        file_tokens,
        &file_path,
        &HeaderParseOptions::default(),
        &mut string_table,
        0,
        0,
    ) {
        Ok(_) => {
            panic!("a dependency shell without a retained file identity must fail preparation")
        }
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => panic!(
            "missing shell identity must not become a source diagnostic: {:?}",
            error.diagnostic.payload
        ),
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => error,
    };

    assert!(
        error
            .msg
            .contains("cannot be stamped without a retained source file identity"),
        "unexpected infrastructure error: {error:?}"
    );
}

fn prepare_tampered_path_clause(source: &str, file_path: &str) -> FileFrontendPrepareFailure {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from(file_path);
    let interned_path = InternedPath::try_from_filesystem_path(&file_path, &mut string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(0)),
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
    ) {
        Ok(_) => panic!("a tampered path handle must fail preparation"),
        Err(error) => error,
    }
}

fn expect_prepare_infrastructure(error: FileFrontendPrepareFailure, case: &str) {
    match error {
        FileFrontendPrepareFailure::Infrastructure(_) => {}
        FileFrontendPrepareFailure::Diagnosed(error) => panic!(
            "{case}: malformed path lookup must not fabricate a user diagnostic: {:?}",
            error.diagnostic.payload
        ),
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
    let file_tokens = tokenize(
        "@core/math sin\n",
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(0)),
    )
    .expect("tokenization should succeed");
    let other_path = InternedPath::from_single_str("other.moth", &mut string_table);
    let other_tokens = tokenize(
        "@other/path sin\n",
        &other_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(1)),
    )
    .expect("other file should tokenize");
    let swapped = FileTokens::new_with_identity(
        file_tokens.src_path,
        file_tokens.file_id,
        file_tokens.canonical_os_path,
        file_tokens.tokens,
        (*other_tokens.path_syntax).clone(),
    );

    expect_prepare_infrastructure(
        match prepare_file_from_tokens(
            swapped,
            &file_path,
            &HeaderParseOptions::default(),
            &mut string_table,
            0,
            0,
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
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: None,
        active_root_role,
    };
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        entry_file_path: file_path,
        options: &options,
        style_directives: &style_directives,
    };

    prepare_test_source_file(source, file_path, &context, string_table, 0, 0)
}

/// Test helper: run both header preparation and binding, returning the raw result.
fn prepare_and_bind_headers_result(
    prepared_outputs: Vec<FileFrontendPrepareOutput>,
    external_package_registry: &ExternalPackageRegistry,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    project_path_resolver: Option<&ProjectPathResolver>,
    string_table: &mut StringTable,
) -> Result<BoundModuleHeaders, DiagnosticBag> {
    let prepared = prepare_header_syntax(prepared_outputs, string_table)?;
    bind_module_headers(
        prepared,
        external_package_registry,
        external_dependency_resolution_table,
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        project_path_resolver,
        string_table,
    )
}

pub(crate) fn parse_single_file_headers(source: &str) -> BoundModuleHeaders {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(source, &file_path, &file_path, &mut string_table);

    prepare_and_bind_headers_result(
        vec![output],
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
    let output = prepare_single_file(source, &file_path, &file_path, &mut string_table);
    let warnings = output.warnings.clone();

    let headers = prepare_and_bind_headers_result(
        vec![output],
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
    let output = prepare_single_file(source, &file_path, &file_path, &mut string_table);

    let headers = prepare_and_bind_headers_result(
        vec![output],
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
    let file_tokens = tokenize(
        source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(FileId(0)),
    )
    .expect("tokenization should succeed");

    let prepare_result = prepare_file_from_tokens(
        file_tokens,
        &entry_file_path,
        &options,
        &mut string_table,
        0,
        0,
    );

    let output = match prepare_result {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            return Err(HeaderTestDiagnostics {
                diagnostics: vec![*error.diagnostic],
                string_table,
            });
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            panic!("header test fixture hit infrastructure failure: {error:?}")
        }
    };

    prepare_and_bind_headers_result(
        vec![output],
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver.as_ref(),
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
fn prepare_header_syntax_produces_retained_syntax_without_provider_inputs() {
    // WHAT: `prepare_header_syntax` must succeed with only a string table — no external package
    //       registry, resolution table, or project path resolver is supplied.
    // WHY: syntax preparation is provider-independent; it owns retained header/dependency shells,
    //      order-independent symbol facts, and statistics before provider interfaces exist.
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "const_a #Int = 1\nimport_b |x Int| -> Int:\n    return x\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    let prepared = prepare_header_syntax(vec![output], &mut string_table)
        .expect("header syntax preparation should succeed without provider inputs");

    // Retained declaration shells are present.
    assert!(
        !prepared.headers.is_empty(),
        "PreparedHeaderSyntax should retain parsed header shells"
    );
    // Order-independent symbol facts are present.
    assert!(
        !prepared.module_symbols.module_file_paths.is_empty(),
        "PreparedHeaderSyntax should carry module symbol facts"
    );
    // Root-activity and statistics metadata are populated.
    assert_eq!(
        prepared.const_fragment_count,
        prepared.top_level_const_fragments.len()
    );
    assert!(prepared.token_stats.total_tokens > 0);
    assert!(prepared.header_stats.functions >= 1);
    // No header binding environment exists yet — that is binding-phase output.
    assert!(
        prepared
            .module_symbols
            .source_package_public_exports
            .is_empty()
    );
}

#[test]
fn bind_module_headers_consumes_prepared_syntax_and_produces_binding_environment() {
    // WHAT: `bind_module_headers` consumes `PreparedHeaderSyntax` and produces
    //       `BoundModuleHeaders` with a completed header binding environment.
    // WHY: binding is the only phase that resolves retained dependency shells against provider
    //      interfaces. It must not retokenize or reparse — it consumes the retained output.
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "const_a #Int = 1\nimport_b |x Int| -> Int:\n    return x\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    let prepared = prepare_header_syntax(vec![output], &mut string_table)
        .expect("header syntax preparation should succeed");
    let header_count_before_binding = prepared.headers.len();
    let source_file = prepared.headers[0].source_file.to_owned();

    let bound = bind_module_headers(
        prepared,
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        None,
        &mut string_table,
    )
    .expect("header binding should succeed");

    // Binding preserves retained header shells — no retokenization or reparsing.
    assert_eq!(
        bound.headers.len(),
        header_count_before_binding,
        "binding must not add or remove header shells"
    );
    // Binding produces the header binding environment that preparation cannot.
    assert!(
        bound
            .binding_environment
            .file_visibility_by_source
            .contains_key(&source_file),
        "BoundModuleHeaders should carry a completed header binding environment"
    );
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

#[test]
fn malformed_children_wrapper_constant_initializer_reports_eof_delimiter_error() {
    let result = parse_single_file_headers_with_entry(
        "broken #= [$children([:<li>[$slot]</li>):\n<ul>[$slot]</ul>\n]\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "unterminated '$children(..)' wrapper templates should fail instead of hanging"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedEndOfFile { .. }
    )));
}

#[test]
fn legacy_import_clause_reports_dedicated_migration_with_flat_replacements() {
    let cases = [
        ("import @core/math\n", "@core/math"),
        ("import @core/math as maths\n", "@core/math as maths"),
        ("import @core/math { sin, cos }\n", "@core/math sin, cos"),
        (
            "import @core/math { sin as sine }\n",
            "@core/math sin as sine",
        ),
        (
            "export:\n    import @core/math { sin }\n;\n",
            "@core/math sin",
        ),
    ];

    for (source, expected_replacement) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors = expect_header_error(result, "legacy import syntax should be diagnosed");
        let diagnostic = &errors.diagnostics[0];
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause)
        );
        let DiagnosticPayload::LegacyDependencyClause {
            replacement: Some(replacement),
            ..
        } = diagnostic.payload
        else {
            panic!("expected a semantics-preserving migration replacement");
        };
        assert_eq!(
            errors.string_table.resolve(replacement),
            expected_replacement
        );
        assert!(
            diagnostic.primary_location.end_pos.char_column
                > diagnostic.primary_location.start_pos.char_column
                || diagnostic.primary_location.end_pos.line_number
                    > diagnostic.primary_location.start_pos.line_number
        );
    }
}

#[test]
fn legacy_filtered_or_nested_import_has_no_automatic_replacement() {
    for source in [
        "import @core/math as maths { sin }\n",
        "import @html { tables { row } }\n",
    ] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
            "ambiguous legacy import syntax should be diagnosed",
        );
        assert!(matches!(
            errors.diagnostics[0].payload,
            DiagnosticPayload::LegacyDependencyClause {
                replacement: None,
                ..
            }
        ));
    }
}

#[test]
fn legacy_quoted_path_and_config_import_have_no_automatic_replacement() {
    for (source, file_path) in [
        ("import @docs/\"my file.md\"\n", "src/@page.moth"),
        ("import @\"@tools\"\n", "src/@page.moth"),
        ("import @\"semi;colon\"\n", "src/@page.moth"),
        ("import @/\n", "src/@page.moth"),
        ("import @core/math\n", "config.moth"),
    ] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, file_path, file_path),
            "legacy syntax without a safe current clause should still be diagnosed",
        );
        assert!(matches!(
            errors.diagnostics[0].payload,
            DiagnosticPayload::LegacyDependencyClause {
                replacement: None,
                ..
            }
        ));
    }
}

#[test]
fn legacy_multiline_import_clause_reports_migration_with_flat_replacement() {
    let cases = [
        ("import\n    @core/math { sin }\n", "@core/math sin"),
        ("import\n\n\n    @core/math { sin }\n", "@core/math sin"),
        (
            "export:\n    import\n        @core/math { sin }\n;\n",
            "@core/math sin",
        ),
        ("import\n    @core/math as maths\n", "@core/math as maths"),
        (
            "import\n    @core/math { sin as sine }\n",
            "@core/math sin as sine",
        ),
    ];

    for (source, expected_replacement) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors =
            expect_header_error(result, "legacy multiline import syntax should be diagnosed");
        let diagnostic = &errors.diagnostics[0];
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause)
        );
        let DiagnosticPayload::LegacyDependencyClause {
            replacement: Some(replacement),
            ..
        } = diagnostic.payload
        else {
            panic!("expected a semantics-preserving migration replacement for: {source}");
        };
        assert_eq!(
            errors.string_table.resolve(replacement),
            expected_replacement
        );
    }
}

#[test]
fn legacy_dependency_comment_between_keyword_and_path_reports_migration() {
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(
            "import -- keep the old clause visible\n    @core/math { sin }\n",
            "src/@page.moth",
            "src/@page.moth",
        ),
        "a comment between import and the path must still be a legacy clause",
    );
    let DiagnosticPayload::LegacyDependencyClause {
        replacement: Some(replacement),
        ..
    } = errors.diagnostics[0].payload
    else {
        panic!("expected a replacement after comment trivia");
    };
    assert_eq!(errors.string_table.resolve(replacement), "@core/math sin");
}

#[test]
fn legacy_dependency_span_covers_import_through_closing_brace() {
    let source = "import @core/math { sin }\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
        "legacy import syntax should be diagnosed",
    );
    let location = &errors.diagnostics[0].primary_location;
    let close_brace = source
        .find('}')
        .expect("the fixture must include a closing brace");
    assert_eq!(location.start_pos.char_column, 1);
    assert_eq!(
        location.end_pos.char_column as usize,
        close_brace + 1,
        "the primary span must end at the closing brace, got {location:?}"
    );
}

#[test]
fn import_followed_by_unrelated_newline_statement_is_not_legacy_clause() {
    let headers = parse_single_file_headers("import\nvalue = 1\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::StartFunction)),
        "import followed by an ordinary statement must not be treated as a legacy clause"
    );
}

#[test]
fn import_is_an_ordinary_identifier() {
    let headers = parse_single_file_headers("import = 1\n");
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::StartFunction))
    );
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

#[test]
fn exported_untyped_constant_has_no_header_provided_dependencies() {
    let headers = parse_single_file_headers("theme #= navbar\n");
    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    assert!(
        constant_header.local_ordering_hints.is_empty(),
        "header-provided constant dependencies come from declared type syntax only"
    );
}

#[test]
fn exported_typed_constant_headers_are_parsed_and_follow_on_constant_stays_header() {
    let headers = parse_single_file_headers("page #String = [: world]\n\ntest #= [page: Hello ]\n");

    assert!(
        matches!(
            headers.headers.first().map(|header| &header.kind),
            Some(HeaderKind::Constant { .. })
        ),
        "first header should be parsed as a constant"
    );
    assert!(
        matches!(
            headers.headers.get(1).map(|header| &header.kind),
            Some(HeaderKind::Constant { .. })
        ),
        "follow-on 'test #= ...' should remain a constant header"
    );
}

#[test]
fn non_generic_headers_keep_generic_parameter_lists_empty() {
    let headers = parse_single_file_headers(
        "identity |value Int| -> Int:\n\
             return value\n\
         ;\n\
         Box = |\n\
             value Int,\n\
         |\n\
         Status :: Ready,\n\
         ;\n\
         Alias as Int\n",
    );

    for header in &headers.headers {
        match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => {
                assert!(
                    generic_parameters.parameters.is_empty(),
                    "non-generic declarations should keep generic parameter lists empty"
                );
            }
            _ => {}
        }
    }
}

#[test]
fn generic_declaration_headers_parse_parameter_lists() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "identity type T |value T| -> T:\n\
             return value\n\
         ;\n\
         Box type Item = |\n\
             value Item,\n\
         |\n\
         ResultShape type OkType, ErrType ::\n\
             Ok | value OkType |,\n\
             Err | error ErrType |,\n\
         ;\n",
    );

    let mut generic_parameter_counts = Vec::new();
    for header in &headers.headers {
        match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => generic_parameter_counts.push(generic_parameters.len()),
            _ => {}
        }
    }

    assert_eq!(generic_parameter_counts, vec![1, 1, 2]);
    assert_eq!(
        headers.module_symbols.generic_declarations_by_path.len(),
        3,
        "only declarations with generic parameters should be registered as generic declarations"
    );

    let generic_names = headers
        .module_symbols
        .generic_declarations_by_path
        .values()
        .flat_map(|metadata| {
            metadata
                .parameters
                .parameters
                .iter()
                .map(|parameter| string_table.resolve(parameter.name).to_owned())
        })
        .collect::<Vec<_>>();

    assert!(generic_names.contains(&"T".to_owned()));
    assert!(generic_names.contains(&"Item".to_owned()));
    assert!(generic_names.contains(&"OkType".to_owned()));
    assert!(generic_names.contains(&"ErrType".to_owned()));
}

#[test]
fn top_level_const_template_outside_entry_file_errors() {
    let result = parse_single_file_headers_with_entry(
        "#[html.head: [\"x\"]]\n",
        "src/lib.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "const templates outside the entry file should error"
    );
}

#[test]
fn top_level_const_template_tokens_keep_close_and_eof_for_ast_parser() {
    let headers = parse_single_file_headers("#[3]\n");

    let const_template_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");

    assert!(
        matches!(
            const_template_header
                .tokens
                .tokens
                .first()
                .map(|token| &token.kind),
            Some(TokenKind::TemplateHead)
        ),
        "const template token stream should start with template opener"
    );

    assert!(
        const_template_header
            .tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::TemplateClose)),
        "const template token stream should preserve template close token"
    );

    assert!(
        matches!(
            const_template_header
                .tokens
                .tokens
                .last()
                .map(|token| &token.kind),
            Some(TokenKind::Eof)
        ),
        "const template token stream should end with EOF sentinel"
    );
}

#[test]
fn top_level_const_template_uses_selected_dependency_alias_path() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@widgets content as panel\n#[panel]\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    let const_template_header = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");
    let hint_paths = const_template_header
        .local_ordering_hints
        .iter()
        .map(|hint| hint.path().to_portable_string(&string_table))
        .collect::<Vec<_>>();

    assert_eq!(hint_paths, vec!["widgets/content"]);
}

#[test]
fn top_level_const_template_collects_if_condition_dependency_refs() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "#[if show_banner:
            [if maybe_name is |name|:
                [name]
            ]
        ]\n",
    );

    let const_template_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::ConstTemplate { .. }))
        .expect("expected top-level const template header");
    let HeaderKind::ConstTemplate {
        condition_references,
        ..
    } = &const_template_header.kind
    else {
        panic!("expected const template header");
    };

    let names = condition_references
        .iter()
        .map(|reference| string_table.resolve(reference.name))
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["show_banner", "maybe_name"]);
}

#[test]
fn start_function_local_references_do_not_create_module_dependencies() {
    let headers = parse_single_file_headers(
        "value = 1\n\
         another = value + 1\n\
         io.line([: [another]])\n",
    );

    let start_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::StartFunction))
        .expect("expected start function header");

    assert!(
        start_header.local_ordering_hints.is_empty(),
        "local start-function symbols must not be tracked as inter-header/module dependencies"
    );
}

#[test]
fn loop_binding_symbols_remain_in_start_function_body() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "items = {1, 2, 3}\n\
         \n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n",
    );

    assert_eq!(
        headers.headers.len(),
        1,
        "loop-only top-level files should emit only the implicit start header"
    );
    assert!(matches!(headers.headers[0].kind, HeaderKind::StartFunction));

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(start_header, &string_table);
    let header_names = non_start_header_names(&headers, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "item"),
        "loop item binding should stay in the implicit start body token stream"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "index"),
        "loop index binding should stay in the implicit start body token stream"
    );
    assert!(
        start_header
            .tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Loop)),
        "start header should preserve the top-level loop statement tokens"
    );
    assert!(
        !header_names
            .iter()
            .any(|name| name == "item" || name == "index"),
        "loop binding names must never be elevated into headers"
    );
}

#[test]
fn top_level_expression_symbols_stay_in_implicit_start_body() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "func basic()\n\
         items = {1, 2, 3}\n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n\
         [basic]\n\
         basic()\n\
         items\n",
    );

    assert_eq!(
        headers.headers.len(),
        1,
        "dependency clauses and top-level expressions should still collapse into one start header here"
    );
    assert!(matches!(headers.headers[0].kind, HeaderKind::StartFunction));

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(start_header, &string_table);
    let header_names = non_start_header_names(&headers, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "basic"),
        "imported symbol usage in expression/template position should stay in start body"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "item")
            && start_symbols.iter().any(|symbol| symbol == "index"),
        "loop binding symbols inside top-level loops should remain start-body tokens"
    );
    assert!(
        start_header
            .tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::TemplateHead)),
        "runtime top-level templates should remain in the start-function token stream"
    );
    assert!(
        !header_names
            .iter()
            .any(|name| name == "basic" || name == "items" || name == "item" || name == "index"),
        "expression-position symbols must not be misclassified as top-level declaration headers"
    );
}

#[test]
fn compile_time_declarations_parse_as_headers_without_elevating_body_symbols() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "theme #= \"dark\"\n\
         items = {theme}\n\
         loop items |item, index|:\n\
             io.line([: [item]])\n\
         ;\n\
         [theme]\n\
         theme\n",
    );

    let header_names = non_start_header_names(&headers, &string_table);
    assert_eq!(
        header_names,
        vec![String::from("theme")],
        "the `theme #= ...` declaration should remain a real top-level constant header"
    );
    assert_eq!(
        headers.headers.len(),
        2,
        "expected one compile-time constant header plus the implicit start header"
    );
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Constant { .. })),
        "compile-time binding syntax should classify as a constant header"
    );

    let start_header = start_function_header(&headers);
    let start_symbols = symbol_tokens_in_header_body(start_header, &string_table);

    assert!(
        start_symbols.iter().any(|symbol| symbol == "theme"),
        "same-name symbol uses later in top-level expressions should stay in start body"
    );
    assert!(
        start_symbols.iter().any(|symbol| symbol == "item")
            && start_symbols.iter().any(|symbol| symbol == "index"),
        "loop-binding symbols in start-body statements must not become headers"
    );
    assert!(
        !header_names
            .iter()
            .any(|name| name == "items" || name == "item" || name == "index"),
        "only legitimate '#'-prefixed declarations should become headers"
    );
}

#[test]
fn function_without_arrow_has_zero_return_slots() {
    let headers = parse_single_file_headers("f||:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(signature.returns.is_empty());
}

#[test]
fn function_value_return_is_preserved_as_return_syntax_shell() {
    let headers = parse_single_file_headers("f|| -> Int:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::BuiltinInt { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}

#[test]
fn function_named_return_is_preserved_for_ast_resolution() {
    let headers = parse_single_file_headers("f|| -> Point:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::Named { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}

#[test]
fn function_parameter_default_stays_in_header_syntax_tokens() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("label |prefix String = \"item\"| -> String:\n;\n");
    let signature = first_function_signature(&headers);

    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");
    assert!(matches!(
        parameter.type_annotation,
        ParsedTypeRef::BuiltinString { .. }
    ));
    assert!(
        parameter.default_tokens.iter().any(|token| matches!(
            token.kind,
            TokenKind::StringSliceLiteral(id) if string_table.resolve(id) == "item"
        )),
        "header should capture default expression tokens without building an AST expression"
    );
}

#[test]
fn struct_field_default_stays_in_header_syntax_tokens() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "DEFAULT_WIDTH #= 80\nConfig = |\n    width Int = DEFAULT_WIDTH,\n|\n",
    );
    let struct_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("expected struct header");

    let HeaderKind::Struct { fields, .. } = &struct_header.kind else {
        panic!("expected Struct header kind");
    };
    let field = fields.first().expect("expected width field shell");

    assert!(matches!(
        field.type_annotation,
        ParsedTypeRef::BuiltinInt { .. }
    ));
    assert!(
        field.default_tokens.iter().any(|token| matches!(
            token.kind,
            TokenKind::Symbol(id) if string_table.resolve(id) == "DEFAULT_WIDTH"
        )),
        "header should preserve struct default tokens for AST-time constant resolution"
    );
}

#[test]
fn function_parameter_default_path_rows_use_the_file_owned_table() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n;\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected Function header kind");
    };
    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");

    let path_id = parameter
        .default_tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("expected a path token in the default");
    assert_eq!(
        function_header
            .tokens
            .path_syntax
            .try_path(path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/intro.md",
        "the file-owned table must resolve the default's path row"
    );
}

#[test]
fn struct_field_default_path_rows_use_the_file_owned_table() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "Config = |\n    path String = [: [@docs/intro.md] ],\n|\n",
    );
    let struct_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("expected struct header");

    let HeaderKind::Struct { fields, .. } = &struct_header.kind else {
        panic!("expected Struct header kind");
    };
    let field = fields.first().expect("expected one field shell");

    let path_id = field
        .default_tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("expected a path token in the default");
    assert_eq!(
        struct_header
            .tokens
            .path_syntax
            .try_path(path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/intro.md",
        "the file-owned table must resolve the field default's path row"
    );
}

#[test]
fn function_default_and_body_path_rows_stay_distinct() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/intro.md] ]| -> String:\n    io.line([: [@docs/body.md]])\n;\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected Function header kind");
    };
    let parameter = signature
        .parameters
        .first()
        .expect("expected one parameter shell");

    let default_path_id = parameter
        .default_tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("expected a path token in the default");
    assert_eq!(
        function_header
            .tokens
            .path_syntax
            .try_path(default_path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/intro.md",
        "the default's handle must not bind the body's path row"
    );

    let body_path_id = function_header
        .tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("expected a path token in the body");
    assert_eq!(
        function_header
            .tokens
            .path_syntax
            .try_path(body_path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/body.md",
        "the body's path row must stay distinct from the default's row"
    );
}

#[test]
fn retained_header_substreams_share_one_frozen_file_path_table() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/default.md] ]| -> String:\n;\nio.line([: [@docs/start.md]])\n",
    );
    let function_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected function header");
    let start_header = start_function_header(&headers);

    let FilePathSyntax::Shared(function_table) = &function_header.tokens.path_syntax else {
        panic!("prepared function header should receive the frozen file table");
    };
    let FilePathSyntax::Shared(start_table) = &start_header.tokens.path_syntax else {
        panic!("prepared start header should receive the frozen file table");
    };
    assert!(
        Arc::ptr_eq(function_table, start_table),
        "ordinary retained header substreams must share one immutable file-owned path table"
    );

    let HeaderKind::Function { signature, .. } = &function_header.kind else {
        panic!("expected function header");
    };
    let default_path_id = signature.parameters[0]
        .default_tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .expect("expected a default path token");
    let start_path_id = start_header
        .tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(path_id) => Some(path_id),
            _ => None,
        })
        .expect("expected a start-body path token");

    assert_eq!(
        function_table
            .try_path(default_path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/default.md"
    );
    assert_eq!(
        start_table
            .try_path(start_path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/start.md"
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn retained_header_substreams_do_not_count_copied_path_rows() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    parse_single_file_headers_with_table(
        "label |prefix String = [: [@docs/default.md] ]| -> String:\n;\nio.line([: [@docs/start.md]])\n",
    );

    log_frontend_counters();
    let observations = timing_session.finish();
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };

    assert_eq!(counter_value("path_syntax_row_count"), 2.0);
    assert_eq!(
        counter_value("persistent_generic_path_syntax_row_copy_count"),
        0.0
    );
    assert_eq!(counter_value("token_rescan_count"), 0.0);
}

#[test]
fn function_signature_rejects_void_return_syntax() {
    let source = format!("f|| {}{}:\n;\n", "-> ", "Void");
    let result = parse_single_file_headers_with_entry(&source, "src/@page.moth", "src/@page.moth");
    assert!(result.is_err(), "void return syntax must be rejected");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::VoidNotAllowed
        }
    )));
}

#[test]
fn function_signature_rejects_none_return_syntax() {
    let source = format!("f|| {}{}:\n;\n", "-> ", "None");
    let result = parse_single_file_headers_with_entry(&source, "src/@page.moth", "src/@page.moth");
    assert!(result.is_err(), "none return syntax must be rejected");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            reason: InvalidTypeAnnotationReason::NoneNotAllowed,
            ..
        }
    )));
}

#[test]
fn function_signature_preserves_unknown_symbolic_return_for_ast_resolution() {
    let headers = parse_single_file_headers("f|| -> MissingType:\n;\n");
    let signature = first_function_signature(&headers);

    assert!(matches!(
        signature.returns.as_slice(),
        [ReturnSlotSyntax {
            value: FunctionReturnSyntax {
                type_annotation: ParsedTypeRef::Named { .. },
                ..
            },
            channel: ReturnChannelSyntax::Success,
            ..
        }]
    ));
}

#[test]
fn trait_declaration_headers_parse_requirement_shells() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "DISPLAYABLE must:\n\
             display |This| -> String\n\
             reset |~This|\n\
             copy_value |This, other This| -> This\n\
         ;\n",
    );

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    let HeaderKind::Trait { declaration } = &trait_header.kind else {
        panic!("expected trait header kind");
    };

    assert_eq!(string_table.resolve(declaration.name), "DISPLAYABLE");
    assert_eq!(declaration.requirements.len(), 3);
    assert_eq!(
        declaration.requirements[0].this_usage,
        TraitThisUsage::Immutable
    );
    assert_eq!(
        declaration.requirements[1].this_usage,
        TraitThisUsage::Mutable
    );

    let copy_requirement = &declaration.requirements[2];
    assert!(matches!(
        copy_requirement.signature.parameters[1].type_annotation,
        ParsedTypeRef::This { .. }
    ));
    assert!(matches!(
        copy_requirement.signature.returns[0].value,
        FunctionReturnSyntax {
            type_annotation: ParsedTypeRef::This { .. },
            ..
        }
    ));
}

#[test]
fn duplicate_trait_declarations_are_structured_header_diagnostics() {
    let result = parse_single_file_headers_with_entry(
        "DISPLAYABLE must:\n    display |This| -> String\n;\n\
         DISPLAYABLE must:\n    render |This| -> String\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "duplicate trait declarations must be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicateDeclaration { .. }
    )));
}

#[test]
fn empty_marker_trait_declaration_is_a_valid_header() {
    let headers = parse_single_file_headers("MARKER must:\n;\n");

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    let HeaderKind::Trait { declaration } = &trait_header.kind else {
        panic!("expected trait header kind");
    };

    assert!(
        declaration.requirements.is_empty(),
        "marker traits should parse with no requirement shells"
    );
}

#[test]
fn trait_conformance_headers_parse_single_and_continued_trait_lists() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Card must DISPLAYABLE,\n    SERIALIZABLE\n");

    let conformance_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitConformance { .. }))
        .expect("expected trait conformance header");

    let HeaderKind::TraitConformance { conformance } = &conformance_header.kind else {
        panic!("expected trait conformance header kind");
    };

    let trait_names = conformance
        .traits
        .iter()
        .map(|trait_ref| string_table.resolve(trait_ref.name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(string_table.resolve(conformance.target.name), "Card");
    assert_eq!(trait_names, vec!["DISPLAYABLE", "SERIALIZABLE"]);
}

#[test]
fn builtin_type_conformance_headers_parse_as_trait_conformances() {
    let (headers, string_table) = parse_single_file_headers_with_table("Int must DISPLAYABLE\n");

    let conformance_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitConformance { .. }))
        .expect("expected builtin trait conformance header");

    let HeaderKind::TraitConformance { conformance } = &conformance_header.kind else {
        panic!("expected trait conformance header kind");
    };

    assert_eq!(string_table.resolve(conformance.target.name), "Int");
    assert_eq!(
        string_table.resolve(conformance.traits[0].name),
        "DISPLAYABLE"
    );
}

#[test]
fn trait_requirement_rejects_lowercase_this_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |this|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "lowercase this should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::ThisNotAllowed
        }
    )));
}

#[test]
fn trait_requirement_rejects_missing_this_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |value Int|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "trait requirements should start with This");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitReceiverMustBeThis
        }
    )));
}

#[test]
fn trait_requirement_rejects_mutable_this_after_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, ~This|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "mutable This is receiver-only");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitMutableThisOnlyFirstParameter
        }
    )));
}

#[test]
fn trait_requirement_rejects_composed_this_type_forms() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, values {This}|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "composed This forms are deferred");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            reason: InvalidTypeAnnotationReason::TraitThisMustBeDirect,
            ..
        }
    )));
}

#[test]
fn trait_requirement_rejects_method_bodies_and_reversed_mutability() {
    let method_body_result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This|:\n        return \"bad\"\n    ;\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let method_body_errors = expect_header_error(
        method_body_result,
        "trait requirements cannot have method bodies",
    );

    assert!(
        method_body_errors
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind
                == DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTokenInDeclaration))
    );

    let reversed_mutability_result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This ~|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reversed_mutability_errors = expect_header_error(
        reversed_mutability_result,
        "trait receiver mutability must be written as ~This",
    );

    assert!(
        reversed_mutability_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::ExpectedToken { .. }
            ))
    );
}

#[test]
fn trait_conformance_rejects_missing_trait_name() {
    let result =
        parse_single_file_headers_with_entry("Card must\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "conformance declarations require a trait name");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitConformanceMissingTrait,
            ..
        }
    )));
}

#[test]
fn trait_conformance_rejects_semicolon_terminator() {
    let result = parse_single_file_headers_with_entry(
        "Card must DISPLAYABLE;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "conformance declarations should be newline terminated",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitConformanceSemicolon,
            ..
        }
    )));
}

#[test]
fn trait_conformance_rejects_trailing_comma() {
    let result = parse_single_file_headers_with_entry(
        "Card must DISPLAYABLE,\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "trailing conformance commas should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedTrailingComma
    )));
}

#[test]
fn trait_declaration_and_reference_names_must_be_all_caps() {
    let declaration_result = parse_single_file_headers_with_entry(
        "Displayable must:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let declaration_errors = expect_header_error(
        declaration_result,
        "trait declarations should require all-caps names",
    );

    assert!(
        declaration_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );

    let conformance_result = parse_single_file_headers_with_entry(
        "Card must Displayable\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let conformance_errors = expect_header_error(
        conformance_result,
        "trait references should require all-caps names",
    );

    assert!(
        conformance_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );
}

#[test]
fn trait_incompatibility_headers_parse_single_and_continued_trait_lists() {
    let (headers, string_table) = parse_single_file_headers_with_table("A must not B,\n    C\n");

    let incompatibility_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitIncompatibility { .. }))
        .expect("expected trait incompatibility header");

    let HeaderKind::TraitIncompatibility { incompatibility } = &incompatibility_header.kind else {
        panic!("expected trait incompatibility header kind");
    };

    let trait_names = incompatibility
        .incompatible_traits
        .iter()
        .map(|trait_ref| string_table.resolve(trait_ref.name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(string_table.resolve(incompatibility.subject.name), "A");
    assert_eq!(trait_names, vec!["B", "C"]);
}

#[test]
fn trait_incompatibility_reuses_subject_trait_name_without_duplicate_error() {
    let headers = parse_single_file_headers(
        "A must:\n\
         ;
\
         A must not B\n",
    );

    let trait_headers = headers
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .count();
    let incompatibility_headers = headers
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::TraitIncompatibility { .. }))
        .count();

    assert_eq!(trait_headers, 1, "subject trait should parse once");
    assert_eq!(
        incompatibility_headers, 1,
        "incompatibility declaration should parse without duplicate-name error"
    );
}

#[test]
fn trait_incompatibility_rejects_missing_trait_name() {
    let result =
        parse_single_file_headers_with_entry("A must not\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "incompatibility declarations require at least one trait name",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitIncompatibilityMissingTrait,
            ..
        }
    )));
}

#[test]
fn trait_incompatibility_rejects_trailing_comma() {
    let result =
        parse_single_file_headers_with_entry("A must not B,\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "trailing incompatibility commas should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedTrailingComma
    )));
}

#[test]
fn trait_incompatibility_rejects_semicolon_terminator() {
    let result =
        parse_single_file_headers_with_entry("A must not B;\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "incompatibility declarations should be newline terminated",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitIncompatibilitySemicolon,
            ..
        }
    )));
}

#[test]
fn trait_incompatibility_subject_and_reference_names_must_be_all_caps() {
    let declaration_result = parse_single_file_headers_with_entry(
        "Displayable must not Other\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let declaration_errors = expect_header_error(
        declaration_result,
        "incompatibility subjects should require all-caps names",
    );

    assert!(
        declaration_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );

    let reference_result = parse_single_file_headers_with_entry(
        "DISPLAYABLE must not Other\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reference_errors = expect_header_error(
        reference_result,
        "incompatibility trait references should require all-caps names",
    );

    assert!(
        reference_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );
}

#[test]
fn trait_this_outside_trait_declaration_is_targeted() {
    let result = parse_single_file_headers_with_entry(
        "value This = 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "This outside trait declarations should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidThisUsage {
            reason: InvalidThisUsageReason::OutsideTraitDeclaration
        }
    )));
}

#[test]
fn function_signature_reports_missing_arrow_before_return_type() {
    let result = parse_single_file_headers_with_entry(
        "f|x Int| Int:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        result.is_err(),
        "missing arrow before return type must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingArrowOrColon { .. }
        }
    )));
}

#[test]
fn function_signature_reports_missing_colon_after_return_list() {
    let result =
        parse_single_file_headers_with_entry("f|| -> Int\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "missing ':' after return declarations must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingColonAfterReturns
        }
    )));
}

#[test]
fn function_signature_reports_missing_return_type_after_arrow_colon() {
    // An authored `->` immediately followed by `:` has no return type. The signature
    // parser owns this boundary and must report `MissingReturnType` at the colon rather
    // than the function name or parameter list.
    let result =
        parse_single_file_headers_with_entry("f|| -> :\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "an arrow immediately followed by ':' must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingReturnType
        }
    )));
}

#[test]
fn function_signature_reports_missing_return_type_after_arrow_newline() {
    // A newline immediately after `->` is a missing-return-type boundary, not a valid
    // empty return list. The signature parser reports `MissingReturnType` at the newline.
    let result =
        parse_single_file_headers_with_entry("f|| ->\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "an arrow immediately followed by a newline must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingReturnType
        }
    )));
}

#[test]
fn trait_requirement_reports_missing_return_type_after_arrow_colon() {
    // A trait requirement is bodyless, so an authored `->` followed by `:` is a missing
    // return type. The shared boundary predicate routes the trait-requirement parser to
    // `MissingTraitRequirementReturnType`, which never tells a bodyless requirement to add
    // the function-body `:` terminator. The diagnostic points at the first missing-type
    // boundary after the arrow, not at the requirement name or `This` receiver.
    let result = parse_single_file_headers_with_entry(
        "DISPLAYABLE must:\n    display |This| -> :\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        result.is_err(),
        "a trait requirement arrow followed by ':' must fail"
    );
    let errors = result.err().expect("expected parse errors");

    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidFunctionSignature {
                    reason: InvalidFunctionSignatureReason::MissingTraitRequirementReturnType
                }
            )
        })
        .expect("expected MissingTraitRequirementReturnType");

    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 23);
}

#[test]
fn trait_requirement_reports_missing_return_type_after_arrow_newline() {
    // A newline after `->` is also a missing-return-type boundary for a trait requirement.
    // The requirement-specific reason is used so the guidance never suggests the function
    // body `:` terminator.
    let result = parse_single_file_headers_with_entry(
        "DISPLAYABLE must:\n    display |This| ->\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        result.is_err(),
        "a trait requirement arrow followed by a newline must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingTraitRequirementReturnType
        }
    )));
}

#[test]
fn duplicate_top_level_function_names_error_during_header_parsing() {
    let result = parse_single_file_headers_with_entry(
        "simple_function |number Int| -> Int:\n\
             return number + 1\n\
         ;\n\
         \n\
         simple_function |value Int| -> Int:\n\
             return value + 2\n\
         ;\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "duplicate top-level function names should fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        )
    }));
}

#[test]
fn duplicate_header_detection_ignores_qualified_match_arms() {
    let mut string_table = StringTable::new();
    let source_file = InternedPath::from_single_str("src/@page.moth", &mut string_table);
    let status = string_table.intern("Status");
    let ready = string_table.intern("Ready");
    let write = string_table.intern("write");
    let location = SourceLocation::default();

    let mut token_stream = FileTokens::new(
        source_file,
        vec![
            Token::new(TokenKind::Symbol(status), location.clone()),
            Token::new(TokenKind::DoubleColon, location.clone()),
            Token::new(TokenKind::Symbol(ready), location.clone()),
            Token::new(TokenKind::FatArrow, location.clone()),
            Token::new(TokenKind::Symbol(write), location.clone()),
            Token::new(TokenKind::Eof, location),
        ],
    );
    token_stream.index = 1;

    assert!(
        !super::super::top_level_classifier::starts_duplicate_top_level_header_declaration(
            &token_stream
        ),
        "qualified match arms in the start body are not choice declarations"
    );
}

#[test]
fn choice_headers_parse_unit_variants_in_declaration_order() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Status :: Ready,\nBusy,\nIdle,\n;\n");
    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    let HeaderKind::Choice { variants, .. } = &choice_header.kind else {
        panic!("expected choice metadata");
    };

    assert_eq!(variants.len(), 3, "expected three parsed variants");
    assert_eq!(string_table.resolve(variants[0].id), "Ready");
    assert_eq!(string_table.resolve(variants[1].id), "Busy");
    assert_eq!(string_table.resolve(variants[2].id), "Idle");
}

#[test]
fn choice_headers_reject_duplicate_variants() {
    let result = parse_single_file_headers_with_entry(
        "Status :: Ready, Ready;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(result.is_err(), "duplicate choice variants must fail");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        )
    }));
}

#[test]
fn choice_headers_reject_invalid_payload_forms() {
    // Shorthand payload is invalid by design (not deferred).
    let payload_shorthand_result = parse_single_file_headers_with_entry(
        "Status :: Ready String;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        payload_shorthand_result.is_err(),
        "shorthand payload variants must be rejected"
    );
    let payload_errors = payload_shorthand_result
        .err()
        .expect("expected payload parse errors");
    assert!(payload_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidChoiceVariant {
            reason: InvalidChoiceVariantReason::PayloadShorthandNotSupported,
            ..
        }
    )));

    // Constructor-style declarations are invalid by design.
    let payload_paren_result = parse_single_file_headers_with_entry(
        "Status :: Ready(String);\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        payload_paren_result.is_err(),
        "constructor-style payload variants must be rejected"
    );
    let payload_paren_errors = payload_paren_result
        .err()
        .expect("expected constructor-style payload parse errors");
    assert!(
        payload_paren_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidChoiceVariant {
                    reason: InvalidChoiceVariantReason::ConstructorStyleNotSupported,
                    ..
                }
            ))
    );

    // Default values remain deferred.
    let defaults_result = parse_single_file_headers_with_entry(
        "Status :: Ready = true;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        defaults_result.is_err(),
        "choice variant defaults must fail"
    );
    let default_errors = defaults_result
        .err()
        .expect("expected default parse errors");
    assert!(default_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::DeferredFeature {
            reason: DeferredFeatureReason::ChoiceVariantDefaultValue
        }
    )));
}

#[test]
fn choice_headers_accept_record_payload_variants() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Status :: Pending |\n    RetryCount Int,\n|;\n");

    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    let HeaderKind::Choice { variants, .. } = &choice_header.kind else {
        panic!("expected choice metadata");
    };

    assert_eq!(variants.len(), 1, "expected one parsed variant");
    assert_eq!(
        string_table.resolve(variants[0].id),
        "Pending",
        "expected Pending variant"
    );
    match &variants[0].payload {
        ChoiceVariantPayloadSyntax::Record { fields } => {
            assert_eq!(fields.len(), 1, "expected one payload field");
            assert_eq!(
                fields[0].id.name_str(&string_table),
                Some("RetryCount"),
                "expected RetryCount field"
            );
        }
        other => panic!("expected Record payload, got {other:?}"),
    }
}

#[test]
fn header_parsing_emits_naming_warnings_for_non_camel_type_like_symbols() {
    let (headers, warnings) = parse_single_file_headers_with_warnings(
        "SITE_TITLE #= \"Moth\"\nStatus_type :: bad_variant;\n",
    );

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Choice { .. })),
        "fixture should still parse a choice header"
    );
    assert_eq!(
        warnings.len(),
        2,
        "expected warnings for choice name and variant only; uppercase constant should be allowed"
    );
    assert!(
        warnings
            .iter()
            .all(|warning| matches!(
                warning.kind,
                crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
                    crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
                )
            )),
        "expected naming convention warnings for choice name and variant only"
    );
}

#[test]
fn header_parsing_rejects_keyword_shadow_constant_name() {
    let result =
        parse_single_file_headers_with_entry("FALSE #= 1\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "keyword-shadow top-level constants must fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::Keyword,
            ..
        }
    )));
}

#[test]
fn trait_declarations_using_must_parse_as_trait_headers() {
    let headers = parse_single_file_headers("DISPLAYABLE must:\n    display |This| -> String\n;\n");

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Trait { .. })),
        "trait declarations using 'must:' should produce trait headers"
    );
}

#[test]
fn generic_type_aliases_are_rejected_during_header_parsing() {
    let result = parse_single_file_headers_with_entry(
        "Response type T as ResultShape of T, Error\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "generic type aliases should fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration)
            && matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::ParameterizedGenericTypeAlias,
                    ..
                }
            )
    }));
}

#[test]
fn entry_runtime_fragment_count_is_zero_with_no_templates() {
    let headers = parse_single_file_headers("x = 1\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "no top-level templates should yield runtime fragment count of 0"
    );
}

#[test]
fn entry_runtime_fragment_count_is_zero_for_const_only_templates() {
    // #[...] is a const (exported) template — it does not contribute to the runtime count.
    let headers = parse_single_file_headers("#[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "const templates should not increment the runtime fragment count"
    );
    assert_eq!(headers.const_fragment_count, 1);
}

#[test]
fn entry_runtime_fragment_count_reflects_runtime_template_count() {
    // [3] is a runtime template (no # prefix); one at top level should yield count 1.
    let headers = parse_single_file_headers("[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 1,
        "one runtime top-level template should yield runtime fragment count of 1"
    );
    assert!(headers.has_non_trivial_root_body);
}

#[test]
fn entry_runtime_fragment_count_accumulates_across_multiple_runtime_templates() {
    let headers = parse_single_file_headers("[1]\n[2]\n[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 3,
        "three runtime top-level templates should yield runtime fragment count of 3"
    );
}

#[test]
fn entry_runtime_fragment_count_ignores_assigned_templates() {
    let headers = parse_single_file_headers(
        "buffer ~= [:initial content]\nbuffer = [:updated content]\n[:fragment]\n",
    );

    assert_eq!(
        headers.entry_runtime_fragment_count, 1,
        "only the direct top-level template should count as an entry runtime fragment"
    );
}

#[test]
fn entry_runtime_fragment_count_is_zero_when_parsed_as_non_entry_file() {
    // An imported root with only declarations reports runtime fragment count 0.
    // WHY: only the active module root contributes runtime fragments.
    let headers = parse_single_file_headers_with_entry(
        "f || -> Int:\n    1\n;\n",
        "src/lib.moth",
        "src/@page.moth",
    )
    .expect("headers should parse");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "runtime_fragment_count must be 0 when the file is not the active root"
    );
}

#[test]
fn imported_module_root_discards_root_body_but_keeps_exportable_headers() {
    let headers = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\n[ Button(\"ignored\") ]\n",
        "src/@components.moth",
        "src/@page.moth",
    )
    .expect("imported module roots should parse their declaration surface");

    assert_eq!(
        headers
            .headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::StartFunction))
            .count(),
        0,
        "imported roots must not produce an implicit start header"
    );
    assert_eq!(headers.entry_runtime_fragment_count, 0);
    assert!(!headers.has_non_trivial_root_body);
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn imported_module_root_discards_const_root_fragments() {
    let headers = parse_single_file_headers_with_entry(
        "#[html.head: [\"ignored\"]]\n",
        "src/@components.moth",
        "src/@page.moth",
    )
    .expect("imported roots should skip const root fragments");

    assert!(headers.top_level_const_fragments.is_empty());
    assert!(
        headers
            .headers
            .iter()
            .all(|header| !matches!(header.kind, HeaderKind::ConstTemplate { .. }))
    );
}

#[test]
fn typed_constant_retains_local_ordering_hint_for_declared_type() {
    // WHY: the declared type creates a structural ordering constraint so that the type
    // is sorted before any constant that references it. Initializer-expression references
    // are collected later during binding; this check owns the declared type annotation.
    let (headers, string_table) =
        parse_single_file_headers_with_table("struct NavBar {}\ntheme #NavBar = default_navbar\n");

    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    assert!(
        !constant_header.local_ordering_hints.is_empty(),
        "typed constant must retain a local ordering hint for its declared type"
    );
    assert!(
        constant_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("NavBar")),
        "local ordering hint must reference the declared type name 'NavBar'"
    );
}

#[test]
fn struct_fields_retain_local_ordering_hints_for_named_field_types() {
    // WHY: struct fields whose types are user-defined names retain conservative hints that Stage 3
    // resolves so the named type is sorted before the struct that depends on it.
    let (headers, string_table) = parse_single_file_headers_with_table(
        "Point = |x Int, y Int|\nSpan = |start Point, end Point|\n",
    );

    let span_header = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Struct { .. })
                && header.tokens.src_path.name_str(&string_table) == Some("Span")
        })
        .expect("expected Span struct header");

    assert!(
        span_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("Point")),
        "Span must retain a local ordering hint for Point via its field type annotations"
    );
}

#[test]
fn function_error_return_retains_local_ordering_hint_for_named_type() {
    // WHY: final `T!` error slots are part of the declaration surface. Their named types must
    // participate in local declaration ordering before AST resolves function signatures.
    let (headers, string_table) = parse_single_file_headers_with_table(
        "AppError = |message String|\nparse || -> Int, AppError!:\n    return 1\n;\n",
    );

    let parse_header = headers
        .headers
        .iter()
        .find(|header| {
            matches!(header.kind, HeaderKind::Function { .. })
                && header.tokens.src_path.name_str(&string_table) == Some("parse")
        })
        .expect("expected parse function header");

    assert!(
        parse_header
            .local_ordering_hints
            .iter()
            .any(|dep| dep.path().name_str(&string_table) == Some("AppError")),
        "function error return slot must retain a local ordering hint for AppError"
    );
}

#[test]
fn constant_header_with_declared_type_captures_type_in_declaration() {
    // Confirms the header-stage contract: declared type annotation is present in the
    // Constant header's declaration, proving initializer resolution is deferred to AST.
    let headers = parse_single_file_headers("threshold #Int = 42\n");

    let constant_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("expected constant header");

    let HeaderKind::Constant { declaration, .. } = &constant_header.kind else {
        panic!("expected Constant header kind");
    };

    assert!(
        !matches!(declaration.type_annotation, ParsedTypeRef::Inferred),
        "declared type annotation on a typed constant must be resolved at the header stage, not left as Inferred"
    );
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
        entry_file_path: &entry_file_path,
        options: &options,
        style_directives: &style_directives,
    };

    let mut prepared_outputs = Vec::new();
    let mut const_template_offset = 0usize;
    let mut runtime_fragment_offset = 0usize;

    for (source, path_str) in sources {
        let file_path = PathBuf::from(path_str);
        let output = prepare_test_source_file(
            source,
            &file_path,
            &prepare_context,
            &mut string_table,
            const_template_offset,
            runtime_fragment_offset,
        )
        .expect("preparation should succeed");

        const_template_offset += output.const_template_count;
        runtime_fragment_offset += output.runtime_fragment_count;
        prepared_outputs.push(output);
    }

    prepare_and_bind_headers_result(
        prepared_outputs,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    )
    .expect("headers should parse")
}

#[test]
fn multi_file_parsing_aggregates_headers_const_fragments_and_runtime_count() {
    let sources = vec![
        (
            "[runtime1]\n#[const1]\n[runtime2]\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "helper_func || -> Int:\n    return 1\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let headers = parse_multi_file_headers(&sources, "src/@page.moth");

    // Entry file: 2 runtime templates + 1 const template + 1 start function = 2 headers
    // (const template + start function; runtime templates are inside start function)
    // Non-entry file: 1 function header
    assert!(
        headers.headers.len() >= 2,
        "expected headers from both files to be aggregated"
    );

    // Verify const fragment from entry file is preserved.
    assert_eq!(
        headers.top_level_const_fragments.len(),
        1,
        "expected one const fragment from entry file"
    );
    assert_eq!(
        headers.top_level_const_fragments[0].runtime_insertion_index, 1,
        "const fragment should be inserted after 1 runtime fragment (the one before it)"
    );

    // Verify runtime fragment count is correct for entry file.
    assert_eq!(
        headers.entry_runtime_fragment_count, 2,
        "expected 2 runtime fragments from entry file"
    );
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
        entry_file_path: &entry_file_path,
        options: &options,
        style_directives: &style_directives,
    };

    let mut prepared_outputs = Vec::new();
    let mut warnings = Vec::new();
    let mut diagnostic_bag = DiagnosticBag::new();
    let mut const_template_offset = 0usize;
    let mut runtime_fragment_offset = 0usize;

    for (source, path_str) in sources {
        let file_path = PathBuf::from(path_str);
        match prepare_test_source_file(
            source,
            &file_path,
            &prepare_context,
            &mut string_table,
            const_template_offset,
            runtime_fragment_offset,
        ) {
            Ok(output) => {
                const_template_offset += output.const_template_count;
                runtime_fragment_offset += output.runtime_fragment_count;
                warnings.extend(output.warnings.clone());
                prepared_outputs.push(output);
            }
            Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
                warnings.extend(error.warnings);
                diagnostic_bag.push(*error.diagnostic);
            }
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                panic!("multi-file header test hit infrastructure failure: {error:?}")
            }
        }
    }

    if diagnostic_bag.has_errors() {
        return (Err(diagnostic_bag), warnings, string_table);
    }

    let result = prepare_and_bind_headers_result(
        prepared_outputs,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        options.project_path_resolver.as_ref(),
        &mut string_table,
    );

    (result, warnings, string_table)
}

#[test]
fn multi_file_parsing_aggregates_warnings_from_all_files() {
    let sources = vec![
        (
            "Status_type :: bad_variant;\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "Helper_type :: other_variant;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(result.is_ok(), "expected successful header parsing");
    assert_eq!(
        warnings.len(),
        4,
        "expected four naming-convention warnings (two from each file)"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn multi_file_parsing_preserves_warnings_before_later_parse_error() {
    // The helper file emits naming warnings, then fails on a later duplicate declaration.
    // Those file-local warnings must still be merged even though the file contributes no output.
    let sources = vec![
        (
            "io.line([: [\"hello\"]])\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "Status_type :: bad_variant;\ndup ||:\n;\ndup ||:\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(
        result.is_err(),
        "expected header parsing to fail due to duplicate declaration"
    );

    assert_eq!(
        warnings.len(),
        2,
        "expected two naming-convention warnings from the failing helper file to be preserved"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn per_file_fork_merge_produces_correct_headers_and_warnings_for_multiple_files() {
    let sources = [
        (
            "FooA #= \"a\"\nBarA #= \"b\"\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
        (
            "FooB #= \"c\"\nBarB #= \"d\"\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    let headers = result.expect("headers should parse");

    // 4 constant headers + 1 start header = 5 headers
    assert_eq!(headers.headers.len(), 5, "expected 4 constants + 1 start");

    let constant_names: Vec<String> = headers
        .headers
        .iter()
        .filter_map(|header| match &header.kind {
            HeaderKind::Constant { .. } => header
                .tokens
                .src_path
                .name()
                .map(|n| string_table.resolve(n).to_owned()),
            _ => None,
        })
        .collect();

    assert!(constant_names.contains(&"FooA".to_owned()));
    assert!(constant_names.contains(&"BarA".to_owned()));
    assert!(constant_names.contains(&"FooB".to_owned()));
    assert!(constant_names.contains(&"BarB".to_owned()));

    // PascalCase top-level constant names should produce naming warnings.
    assert_eq!(
        warnings.len(),
        4,
        "expected four naming convention warnings for PascalCase constants"
    );
    assert!(
        warnings.iter().all(|warning| matches!(
            warning.kind,
            DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );
}

#[test]
fn per_file_fork_merge_remaps_non_identity_strings_across_multiple_files() {
    // Both files intern generated deferred-feature strings into their local suffixes.
    // Because the fork source is shared and frozen before the loop, the second merge must remap
    // that local ID past the first file's generated string in the module table.
    let sources = [
        (
            "Foo #= \"a\"\n#[public_surface_fragment]\n".to_owned(),
            "src/helper_a.moth".to_owned(),
        ),
        (
            "Bar #= \"b\"\n#[const_fragment]\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
    ];

    let (result, warnings, string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");

    assert!(
        result.is_err(),
        "expected header parsing to fail due to deferred header features"
    );

    // PascalCase constants produce naming warnings before the errors.
    assert_eq!(
        warnings.len(),
        2,
        "expected two naming convention warnings before errors"
    );

    let errors = result.err().expect("expected errors").into_diagnostics();
    assert_eq!(errors.len(), 2, "expected two deferred feature errors");

    let mut feature_names = Vec::new();
    for error in &errors {
        let DiagnosticPayload::DeferredFeature { reason } = &error.payload else {
            panic!("expected DeferredFeature payload, got {:?}", error.payload);
        };
        match reason {
            DeferredFeatureReason::NamedFeature { feature } => {
                feature_names.push(string_table.resolve(*feature).to_owned());
            }
            other => panic!("expected NamedFeature reason, got {:?}", other),
        }
    }

    assert!(
        feature_names
            .iter()
            .all(|feature| { feature == "top-level const templates in ordinary source files" })
    );
}

#[test]
fn dependency_only_file_contributes_file_dependency_clauses_and_module_file_paths() {
    use crate::compiler_frontend::headers::symbol_collection::build_module_symbols;

    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let helper_output = prepare_single_file(
        "@core/math\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let page_output = prepare_single_file(
        "value #= 1\n",
        &PathBuf::from("src/@page.moth"),
        &entry_file_path,
        &mut string_table,
    );

    let mut prepared_files = vec![helper_output, page_output];
    let module_symbols = build_module_symbols(&mut prepared_files, &mut string_table)
        .expect("module symbols should build");

    let helper_path = InternedPath::try_from_filesystem_path(
        &PathBuf::from("src/helper.moth"),
        &mut string_table,
    )
    .expect("test path should be UTF-8");
    let page_path =
        InternedPath::try_from_filesystem_path(&PathBuf::from("src/@page.moth"), &mut string_table)
            .expect("test path should be UTF-8");

    assert!(
        module_symbols.module_file_paths.contains(&helper_path),
        "dependency-only files must contribute to module_file_paths"
    );
    assert!(
        module_symbols.module_file_paths.contains(&page_path),
        "entry files must contribute to module_file_paths"
    );

    let helper_dependencies = module_symbols
        .file_dependency_clauses_by_source
        .get(&helper_path)
        .expect("dependency-only file clauses must be registered");

    assert_eq!(helper_dependencies.len(), 1);
    assert_eq!(
        helper_dependencies[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "core/math"
    );
    assert_eq!(
        helper_dependencies[0].export_mode,
        crate::compiler_frontend::headers::types::HeaderExportMode::Private
    );
}

#[test]
fn per_file_prepare_output_preserves_file_role_and_dependencies_on_output() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@core/math\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::Normal);
    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "core/math"
    );
}

#[test]
fn retained_js_provider_path_records_external_target() {
    let mut string_table = StringTable::new();
    let output = prepare_single_file(
        "@drawing.js as drawing\n",
        &PathBuf::from("src/@page.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );
    assert_eq!(output.file_dependency_clauses.len(), 1);
    match &output.file_dependency_clauses[0].dependency.target {
        crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::ExternalProvider {
            prefix_component_count,
            extension,
        } => {
            assert_eq!(*prefix_component_count, 1);
            assert_eq!(string_table.resolve(*extension), "js");
        }
        other => panic!("expected an external provider target, got {other:?}"),
    }
}

#[test]
fn explicit_extension_provider_requires_alias_or_selection() {
    let result =
        parse_single_file_headers_with_entry("@drawing.js\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "bare provider clauses should be rejected");
    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::ProviderRequiresBinding,
                ..
            }
        )
    }));
}

#[test]
fn dependency_clause_is_rejected_in_config_source() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("config.moth");
    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: None,
        active_root_role: ModuleRootRole::Normal,
    };
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        entry_file_path: &file_path,
        options: &options,
        style_directives: &style_directives,
    };
    let error = match prepare_test_source_file(
        "@core/math sin\n",
        &file_path,
        &context,
        &mut string_table,
        0,
        0,
    ) {
        Ok(_) => panic!("config dependency clause should be rejected"),
        Err(error) => error,
    };
    let FileFrontendPrepareFailure::Diagnosed(error) = error else {
        panic!("config dependency rejection must use a source diagnostic");
    };
    assert!(matches!(
        error.diagnostic.payload,
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
            ..
        }
    ));
}

#[test]
fn retained_dependency_shells_get_deterministic_ordinals_per_authored_clause() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@one a\n@two\nexport:\n    @one a\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        3,
        "every authored clause must keep its own retained shell even when it repeats a path"
    );

    let direct_selection = &output.file_dependency_clauses[0];
    let direct_selections = direct_selection
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selection.dependency.dependency_shell_id.ordinal, 0);
    assert_eq!(direct_selections.len(), 1);
    assert_eq!(direct_selection.export_mode, HeaderExportMode::Private);

    let bare = &output.file_dependency_clauses[1];
    assert_eq!(bare.dependency.dependency_shell_id.ordinal, 1);
    assert!(
        bare.selections(&output.dependency_selections)
            .expect("bare clause range should be valid")
            .is_empty()
    );
    assert_eq!(bare.export_mode, HeaderExportMode::Private);

    let public = &output.file_dependency_clauses[2];
    let public_selections = public
        .selections(&output.dependency_selections)
        .expect("public clause range should be valid");
    assert_eq!(public.dependency.dependency_shell_id.ordinal, 2);
    assert_eq!(public_selections.len(), 1);
    assert_eq!(
        public.export_mode,
        HeaderExportMode::Public,
        "the repeated public re-export keeps its own retained shell and visibility"
    );
}

#[test]
fn direct_selection_and_namespace_clauses_keep_provider_root_and_selection_shape() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@one a\n@one/a\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        2,
        "direct-selection and namespace clauses that flatten to the same path must both be retained"
    );

    let direct_selection = &output.file_dependency_clauses[0];
    let direct_selections = direct_selection
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selection.dependency.dependency_shell_id.ordinal, 0);
    assert_eq!(
        direct_selection
            .dependency
            .path
            .to_portable_string(&string_table),
        "one"
    );
    assert_eq!(string_table.resolve(direct_selections[0].source_name), "a");

    let bare = &output.file_dependency_clauses[1];
    assert!(
        bare.selections(&output.dependency_selections)
            .expect("bare clause range should be valid")
            .is_empty(),
        "the bare clause is a namespace binding"
    );
    assert_eq!(bare.dependency.dependency_shell_id.ordinal, 1);
    assert_eq!(
        bare.dependency.path.to_portable_string(&string_table),
        "one/a"
    );
    assert_ne!(
        direct_selection.dependency.location, bare.dependency.location,
        "each authored occurrence keeps its own source location"
    );
}

#[test]
fn imported_module_root_prepare_output_has_imported_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@mod.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "Button = | label String |\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ImportedModuleRoot);
    assert!(output.file_dependency_clauses.is_empty());
}

#[test]
fn entry_normal_module_root_file_is_assigned_active_module_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "greeting #= \"hello\"\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ActiveModuleRoot);
}

#[test]
fn api_only_active_roots_export_declarations_without_synthesizing_start() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        let mut string_table = StringTable::new();
        let file_path = PathBuf::from("src/styles/+package.moth");
        let output = prepare_active_root_with_role(
            "export:\n    theme #= \"dark\"\n;\n",
            &file_path,
            root_role,
            &mut string_table,
        )
        .expect("API-only roots should accept public declarations");

        assert_eq!(output.file_role, FileRole::ActiveApiOnlyModuleRoot);
        assert!(output.headers.iter().any(|header| {
            matches!(header.kind, HeaderKind::Constant { .. })
                && header.export_mode == HeaderExportMode::Public
        }));
        assert!(
            output
                .headers
                .iter()
                .all(|header| !matches!(header.kind, HeaderKind::StartFunction)),
            "API-only roots must not synthesize start"
        );
        assert_eq!(output.runtime_fragment_count, 0);
        assert_eq!(output.const_template_count, 0);
        assert!(!output.has_non_trivial_root_body);
    }
}

#[test]
fn api_only_active_roots_reject_every_root_activity_form() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        for source in ["value = 1\n", "[3]\n", "#[3]\n"] {
            let mut string_table = StringTable::new();
            let file_path = PathBuf::from("src/styles/+package.moth");
            let error = match prepare_active_root_with_role(
                source,
                &file_path,
                root_role,
                &mut string_table,
            ) {
                Ok(_) => panic!("API-only root activity should be rejected"),
                Err(FileFrontendPrepareFailure::Diagnosed(error)) => error,
                Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                    panic!(
                        "API-only root source rejection became infrastructure failure: {error:?}"
                    )
                }
            };

            assert_eq!(
                error.diagnostic.kind,
                DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement)
            );
        }
    }
}

#[test]
fn support_package_root_file_is_assigned_imported_module_root_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/styles/+package.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "theme #= \"dark\"\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::ImportedModuleRoot);
}

#[test]
fn ordinary_source_file_is_assigned_normal_role() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "value #= 1\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    assert_eq!(output.file_role, FileRole::Normal);
}

#[test]
fn support_package_root_file_accepts_an_export_block() {
    let headers = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\n",
        "src/styles/+package.moth",
        "src/@page.moth",
    )
    .expect("a `+*.moth` support-package root should be export-capable");

    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

// ------------------------------
//  Export block parsing tests
// ------------------------------

#[test]
fn export_outside_module_root_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\n",
        "src/helper.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "export block outside a module root should be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::ExportOutsideModuleRoot)));
}

#[test]
fn export_alone_is_rejected() {
    let result =
        parse_single_file_headers_with_entry("export\n", "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(result, "export without a block colon should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken {
                expected: TokenKind::Colon,
                ..
            }
        )
    }));
}

#[test]
fn empty_export_block_is_rejected() {
    let result =
        parse_single_file_headers_with_entry("export:\n;\n", "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(result, "empty export block should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn duplicate_export_blocks_are_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    first #= 1\n;\nexport:\n    second #= 2\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "duplicate export blocks should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateExportBlock)));
}

#[test]
fn legacy_inline_export_declaration_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export Button = | label String |\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "legacy inline export should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken {
                expected: TokenKind::Colon,
                ..
            }
        )
    }));
}

#[test]
fn export_dependency_path_parsed_as_public_surface_dependency() {
    let mut string_table = StringTable::new();
    let output = prepare_single_file(
        "export:\n    @button Button\n;\n",
        &PathBuf::from("src/@mod.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );

    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Public
    );
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "button"
    );
    let selections = output.file_dependency_clauses[0]
        .selections(&output.dependency_selections)
        .expect("retained public export selection range should be valid");
    assert_eq!(selections.len(), 1);
    assert_eq!(string_table.resolve(selections[0].source_name), "Button");
}

#[test]
fn export_block_requires_direct_selection_dependencies() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    @button\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "bare dependency paths in export blocks should be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn nested_export_blocks_are_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    export:\n        value #= 1\n    ;\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "nested export blocks should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn export_block_accepts_an_item_without_a_following_newline() {
    let headers = parse_single_file_headers_with_entry(
        "export: Button = | label String |\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    )
    .expect("export block should accept its first item after the colon");

    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Struct { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn legacy_export_path_syntax_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export @card Card, render as render_card\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "legacy export path syntax should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken {
                expected: TokenKind::Colon,
                ..
            }
        )
    }));
}

#[test]
fn export_bare_path_rejected_as_deferred_namespace_export() {
    let result =
        parse_single_file_headers_with_entry("export @layout\n", "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "bare namespace export should be rejected as deferred",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::ExpectedToken {
                expected: TokenKind::Colon,
                ..
            }
        )
    }));
}

#[test]
fn export_before_authored_declaration_marks_header_public() {
    let source = "export:\n    Button = | label String |\n    render |button Button| -> String:\n        return button.label\n    ;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let public_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| header.export_mode == HeaderExportMode::Public)
        .collect();

    assert_eq!(
        public_headers.len(),
        2,
        "expected two public headers: struct and function"
    );
}

#[test]
fn unmarked_authored_declarations_in_module_root_remain_private() {
    let source = "Button = | label String |\nrender |button Button| -> String:\n    return button.label\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let non_start_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::StartFunction))
        .collect();

    assert!(
        non_start_headers
            .iter()
            .all(|header| header.export_mode == HeaderExportMode::Private),
        "unmarked declarations in a module root should remain private"
    );
}

#[test]
fn duplicate_declaration_detection_works_with_exported_declarations() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n;\nButton = | title String |\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "duplicate declaration with export should still be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::DuplicateDeclaration { .. }
    )));
}

#[test]
fn export_before_constant_marks_header_public() {
    let source = "export:\n    theme #= \"dark\"\n    threshold #Int = 42\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let public_constants: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| {
            matches!(header.kind, HeaderKind::Constant { .. })
                && header.export_mode == HeaderExportMode::Public
        })
        .collect();

    assert_eq!(
        public_constants.len(),
        2,
        "expected two public constant headers"
    );
}

#[test]
fn export_before_type_alias_marks_header_public() {
    let source = "export:\n    UserId as Int\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let alias_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TypeAlias { .. }))
        .expect("expected type alias header");

    assert_eq!(alias_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_before_choice_marks_header_public() {
    let source = "export:\n    Status :: Ready, Failed | message String |;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    assert_eq!(choice_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_before_trait_declaration_marks_header_public() {
    let source = "export:\n    DISPLAY_TEXT must:\n        display |This| -> String\n    ;\n;\n";
    let headers = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth")
        .expect("headers should parse");

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    assert_eq!(trait_header.export_mode, HeaderExportMode::Public);
}

#[test]
fn export_trait_incompatibility_is_rejected() {
    let source = "export:\n    DISPLAY_TEXT must not TRY_DISPLAY_TEXT\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@page.moth");
    let errors = expect_header_error(result, "trait incompatibility must not be exported");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn export_before_trait_conformance_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    Label must DISPLAY_TEXT\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "trait conformance should not be exported");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn export_before_unsupported_runtime_statement_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    io.line([: [\"hello\"]])\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "export before runtime statement should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn receiver_methods_cannot_be_directly_exported() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    Button = | label String |\n    render |this Button| -> String:\n        return this.label\n    ;\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "receiver methods should not be direct exports");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverDeclaration)
    }));
}

#[test]
fn export_before_runtime_template_is_rejected() {
    let result = parse_single_file_headers_with_entry(
        "export:\n    [: hello ]\n;\n",
        "src/@mod.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "export before runtime template should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| diagnostic.kind
        == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget)));
}

#[test]
fn public_dependency_and_private_dependency_keep_distinct_retained_shells() {
    let mut string_table = StringTable::new();
    let output = prepare_single_file(
        "@button Button\nexport:\n    @button Button\n;\n",
        &PathBuf::from("src/@mod.moth"),
        &PathBuf::from("src/@page.moth"),
        &mut string_table,
    );

    assert_eq!(
        output.file_dependency_clauses.len(),
        2,
        "the private dependency and the public re-export each retain their own authored clause"
    );
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Private
    );
    assert_eq!(
        output.file_dependency_clauses[0]
            .dependency
            .dependency_shell_id
            .ordinal,
        0
    );
    assert_eq!(
        output.file_dependency_clauses[1].export_mode,
        HeaderExportMode::Public
    );
    assert_eq!(
        output.file_dependency_clauses[1]
            .dependency
            .dependency_shell_id
            .ordinal,
        1
    );
}

#[test]
fn capacity_references_extract_value_refs_without_treating_element_type_as_value_ref() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/test.moth");
    let source = "make |items ~{capacity MyType}| -> Int:
    return 1
;
";
    let output = prepare_single_file(source, &file_path, &file_path, &mut string_table);

    let headers = prepare_and_bind_headers_result(
        vec![output],
        &ExternalPackageRegistry::new(),
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    )
    .expect("headers should parse");

    let make_header = headers
        .headers
        .iter()
        .find(|h| {
            h.tokens
                .src_path
                .name_str(&string_table)
                .is_some_and(|n| n == "make")
        })
        .expect("make header should exist");

    let capacity_names: Vec<_> = make_header
        .capacity_references
        .iter()
        .map(|r| string_table.resolve(r.name))
        .collect();

    assert!(
        capacity_names.contains(&"capacity"),
        "bare capacity syntax should reference the capacity constant"
    );
    assert!(
        !capacity_names.contains(&"MyType"),
        "element type name must not be treated as a capacity value reference"
    );
}

// ------------------------------
//  Core cast trait name collision tests
// ------------------------------

#[test]
fn header_parsing_rejects_core_cast_trait_source_declaration() {
    let result = parse_single_file_headers_with_entry(
        "CASTABLE_TO_STRING must:\n    to_string |This| -> String\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "source declaration of a core cast trait name must be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
}

#[test]
fn header_parsing_allows_displayable_source_declaration() {
    let headers = parse_single_file_headers("DISPLAYABLE must:\n    display |This| -> String\n;\n");

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Trait { .. })),
        "DISPLAYABLE declarations must remain valid outside the core cast trait hardening slice"
    );
}

#[test]
fn header_parsing_rejects_selection_alias_to_core_cast_trait_name() {
    let sources = vec![
        (
            "USER_TRAIT must:\n    to_string |This| -> String\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
        (
            "@helper USER_TRAIT as CASTABLE_TO_STRING\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
    ];

    let (result, _warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");
    assert!(
        result.is_err(),
        "a dependency selection alias to a core cast trait name must be rejected"
    );

    let errors = result.err().expect("expected parse errors");
    assert!(errors.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
}

#[test]
fn header_parsing_rejects_module_public_surface_re_export_with_core_cast_trait_name() {
    let sources = vec![
        (
            "USER_TRAIT must:\n    to_string |This| -> String\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
        (
            "export:\n    @helper USER_TRAIT as CASTABLE_TO_STRING\n;\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
        ("@helper\n".to_owned(), "src/@page.moth".to_owned()),
    ];

    let (result, _warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");
    assert!(
        result.is_err(),
        "module public-surface re-export under a core cast trait name must be rejected"
    );

    let errors = result.err().expect("expected parse errors");
    assert!(errors.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
}

#[test]
fn missing_default_value_after_assign_points_at_member_boundary() {
    // An authored `=` for an ordinary parameter or struct field must be followed by a
    // default expression. Each case below reaches a distinct member/EOF boundary before
    // any expression token begins, so the shared member-default owner reports
    // `MissingDefaultValue` (MOTH-RULE-0028) pointing at that boundary, not a generic
    // unexpected-token or end-of-file failure.
    let cases: &[(&str, i32, i32)] = &[
        // function parameter ending at the closing pipe
        ("label |prefix String =| -> String:\n;\n", 0, 23),
        // struct field ending at a comma
        ("Config = |\n    width Int =,\n|\n", 1, 16),
        // struct field ending at the closing pipe
        ("Config = |\n    width Int =|\n", 1, 16),
        // newline immediately after the authored `=`
        ("label |prefix String =\n| -> String:\n;\n", 0, 22),
        // block end (`;`) immediately after the authored `=`
        ("label |prefix String =;\n", 0, 23),
        // end of file immediately after the authored `=`
        ("label |prefix String =", 0, 22),
    ];

    for (source, expected_line, expected_column) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors =
            expect_header_error(result, "an authored `=` with no value should be rejected");
        let diagnostic = errors
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.payload,
                    DiagnosticPayload::InvalidSignatureMember {
                        reason: InvalidSignatureMemberReason::MissingDefaultValue
                    }
                )
            })
            .expect("expected a MissingDefaultValue signature-member diagnostic");
        assert_eq!(
            diagnostic.kind.code(),
            "MOTH-RULE-0028",
            "MissingDefaultValue must keep the stable signature-member code"
        );
        assert_eq!(
            diagnostic.primary_location.start_pos.line_number, *expected_line,
            "MissingDefaultValue should point at the boundary line for: {source}"
        );
        assert_eq!(
            diagnostic.primary_location.start_pos.char_column, *expected_column,
            "MissingDefaultValue should point at the boundary column for: {source}"
        );
    }
}

#[test]
fn special_member_default_reasons_win_over_missing_default_value() {
    // Reactive parameters, trait requirements and choice payload fields keep their own
    // more specific default-value reasons even when no expression follows the authored
    // `=`, so they never fall through to `MissingDefaultValue`.
    let reactive = parse_single_file_headers_with_entry(
        "label |event $String =| -> String:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reactive_errors =
        expect_header_error(reactive, "reactive parameter defaults must be rejected");
    assert!(
        reactive_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidSignatureMember {
                    reason: InvalidSignatureMemberReason::ReactiveParameterDefaultValue
                }
            ))
    );

    let trait_requirement = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, value Int =|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let trait_errors = expect_header_error(
        trait_requirement,
        "trait requirement defaults must be rejected",
    );
    assert!(trait_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitRequirementDefaultValue
        }
    )));

    let choice = parse_single_file_headers_with_entry(
        "Response ::\n    Err |\n        message String =|,\n    Success,\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let choice_errors =
        expect_header_error(choice, "choice payload field defaults must be rejected");
    assert!(choice_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::ChoicePayloadDefaultValue
        }
    )));
}

#[test]
fn authored_default_expression_survives_newline_and_multiline_continuation() {
    // A default that begins with a real expression token before any boundary stays valid.
    // The early missing-default check only fires before the first expression token, so a
    // value followed by a newline member boundary and an operator-continued multiline
    // default both parse successfully.
    let (single_line_then_newline, _string_table) =
        parse_single_file_headers_with_table("label |prefix String = \"a\"\n| -> String:\n;\n");
    let signature = first_function_signature(&single_line_then_newline);
    assert!(
        signature.parameters.iter().any(|parameter| parameter
            .default_tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))),
        "a default that begins before a newline should be captured"
    );

    let (multiline, _string_table) = parse_single_file_headers_with_table(
        "label |prefix String = \"a\" +\n    \"b\"| -> String:\n;\n",
    );
    let multiline_signature = first_function_signature(&multiline);
    assert!(
        multiline_signature
            .parameters
            .iter()
            .any(|parameter| parameter
                .default_tokens
                .iter()
                .filter(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
                .count()
                == 2),
        "an operator-continued multiline default should fold both string literals"
    );
}

// ---------------------------------------------------------------------------
//  Provider-independent preparation vs provider-dependent prelude collisions
// ---------------------------------------------------------------------------

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

#[test]
fn prelude_symbol_declaration_prepared_without_registry_then_collides_at_binding() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    // Preparation takes no registry input, so a declaration that reuses a prelude symbol name
    // still parses into a retained declaration shell during provider-independent preparation.
    let output = prepare_single_file(
        "prelude_fn |x Int| -> Int:\n    return x\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "provider-independent preparation should retain the prelude-named declaration shell"
    );
    let expected_location = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected retained prelude-named function shell")
        .name_location
        .to_owned();

    let registry = registry_with_prelude_function_symbol("prelude_fn");
    let result = prepare_and_bind_headers_result(
        vec![output],
        &registry,
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    );
    let binding_error = match result {
        Ok(_) => panic!("binding should reject a declaration that collides with a prelude symbol"),
        Err(bag) => bag,
    };
    let diagnostic = binding_error
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.kind,
                DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName)
            )
        })
        .expect("binding should preserve the reserved builtin-name diagnostic");
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0027");
    assert_eq!(diagnostic.primary_location, expected_location);
}

#[test]
fn prelude_type_generic_parameter_prepared_without_registry_then_collides_at_binding() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    // A generic parameter reusing a prelude type name parses during provider-independent
    // preparation; the collision is provider-dependent and is validated during binding.
    let output = prepare_single_file(
        "Box type PreludeType = |\n    value PreludeType,\n|\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Struct { .. })),
        "provider-independent preparation should retain the generic declaration shell"
    );

    let registry = registry_with_prelude_type_symbol("PreludeType");
    let result = prepare_and_bind_headers_result(
        vec![output],
        &registry,
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    );
    let binding_error = match result {
        Ok(_) => panic!("binding should reject a generic parameter naming a prelude type"),
        Err(bag) => bag,
    };
    assert!(
        binding_error.diagnostics().iter().any(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::GenericParameterNameCollision { .. },
                    ..
                }
            )
        }),
        "prelude-type generic parameter collision should be diagnosed during binding"
    );
}

#[test]
fn direct_selection_does_not_reserve_provider_basename_for_generic_parameter() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@core/Math add\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "direct selections must not reserve the provider basename as a local namespace"
    );
}

#[test]
fn direct_selection_alias_still_reserves_effective_local_name_for_generic_parameter() {
    for source in [
        "@core/math add as Math\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        "identity type Math |value Math| -> Math:\n    return value\n;\n@core/math add as Math\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}

#[test]
fn namespace_provider_basename_reserves_generic_parameter_name_in_either_source_order() {
    for source in [
        "@core/Math\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        "identity type Math |value Math| -> Math:\n    return value\n;\n@core/Math\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}

#[test]
fn explicit_extension_namespace_stem_reserves_generic_parameter_name_in_either_source_order() {
    for source in [
        "@Drawing.js as Drawing\nidentity type Drawing |value Drawing| -> Drawing:\n    return value\n;\n",
        "identity type Drawing |value Drawing| -> Drawing:\n    return value\n;\n@Drawing.js as Drawing\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}

fn assert_generic_dependency_name_collision(source: &str) {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let options = HeaderParseOptions::default();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let context = HeaderTestPrepareContext {
        entry_file_path: &file_path,
        options: &options,
        style_directives: &style_directives,
    };
    let error =
        match prepare_test_source_file(source, &file_path, &context, &mut string_table, 0, 0) {
            Ok(_) => panic!("dependency names must reserve matching generic parameter names"),
            Err(FileFrontendPrepareFailure::Diagnosed(error)) => error,
            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                panic!("generic-name collision became infrastructure failure: {error:?}")
            }
        };

    assert!(
        matches!(
            &error.diagnostic.payload,
            DiagnosticPayload::InvalidDeclaration {
                reason: InvalidDeclarationReason::GenericParameterNameCollision { .. },
                ..
            }
        ),
        "unexpected dependency-name diagnostic: {:?}",
        error.diagnostic.payload
    );
}

#[test]
fn one_dependency_shell_and_selection_list_per_authored_clause() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@core/math sin, cos as cosine\n@core/io\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let clauses = &output.file_dependency_clauses;
    assert_eq!(
        clauses.len(),
        2,
        "the direct-selection clause owns both selections and the simple clause owns its namespace"
    );

    // Both selections of the direct-selection clause share one authored-clause shell.
    let selection_shell = clauses[0].dependency.dependency_shell_id;
    let direct_selections = clauses[0]
        .selections(&output.dependency_selections)
        .expect("direct-selection clause range should be valid");
    assert_eq!(direct_selections.len(), 2);
    assert_eq!(
        clauses[0].dependency.path.to_portable_string(&string_table),
        "core/math"
    );
    assert_eq!(
        string_table.resolve(direct_selections[0].source_name),
        "sin"
    );
    assert_eq!(
        string_table.resolve(direct_selections[1].source_name),
        "cos"
    );

    // The next authored clause receives the next shell ordinal regardless of the
    // clause's selected-name count.
    let simple = &clauses[1];
    assert_eq!(
        simple.dependency.dependency_shell_id.source,
        selection_shell.source
    );
    assert_eq!(simple.dependency.dependency_shell_id.ordinal, 1);
    assert!(
        simple
            .selections(&output.dependency_selections)
            .expect("simple clause range should be valid")
            .is_empty()
    );
    assert_eq!(
        simple.dependency.path.to_portable_string(&string_table),
        "core/io"
    );
}

#[test]
fn selected_name_duplicate_declaration_preserves_both_exact_spans() {
    let result = parse_single_file_headers_with_entry(
        "@core/math sin\nsin #= 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "a selected name must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let DiagnosticPayload::DuplicateDeclaration {
        first_location: Some(first_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the selected name to be the first location");
    };
    assert_eq!(first_location.start_pos.line_number, 0);
    assert_eq!(first_location.start_pos.char_column, 12);
    assert_eq!(first_location.end_pos.char_column, 14);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 3);
}

#[test]
fn selected_alias_duplicate_declaration_uses_the_alias_span() {
    let result = parse_single_file_headers_with_entry(
        "@core/math sin as local\nlocal #= 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "a selected alias must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let DiagnosticPayload::DuplicateDeclaration {
        first_location: Some(first_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the selected alias to be the first location");
    };
    assert_eq!(first_location.start_pos.line_number, 0);
    assert_eq!(first_location.start_pos.char_column, 19);
    assert_eq!(first_location.end_pos.char_column, 23);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 5);
}

#[test]
fn declaration_followed_by_selection_preserves_declaration_and_selection_spans() {
    let result = parse_single_file_headers_with_entry(
        "line #= 1\n@core/io line\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "a selection must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::ImportNameCollision { .. }
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "expected dependency-name collision diagnostic: {:?}",
                errors.diagnostics
            )
        });

    let DiagnosticPayload::ImportNameCollision {
        previous_location: Some(previous_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the declaration to be the previous location");
    };
    assert_eq!(previous_location.start_pos.line_number, 0);
    assert_eq!(previous_location.start_pos.char_column, 1);
    assert_eq!(previous_location.end_pos.char_column, 4);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 10);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 13);
}

#[test]
fn duplicate_selected_aliases_preserve_first_and_current_alias_spans() {
    let result = parse_single_file_headers_with_entry(
        "@core/io line as value\n@core/io debug as value\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "duplicate selected aliases must conflict");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::ImportNameCollision { .. }
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "expected dependency-name collision diagnostic: {:?}",
                errors.diagnostics
            )
        });

    let DiagnosticPayload::ImportNameCollision {
        previous_location: Some(previous_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the first selected alias to be the previous location");
    };
    assert_eq!(previous_location.start_pos.line_number, 0);
    assert_eq!(previous_location.start_pos.char_column, 18);
    assert_eq!(previous_location.end_pos.char_column, 22);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 19);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 23);
}

#[test]
fn direct_selection_empty_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(0, 0));
    let error = clause
        .selections(&[])
        .expect_err("empty direct-selection ranges must not become namespace bindings");
    assert!(error.msg.contains("empty selection range"));
}

#[test]
fn direct_selection_reversed_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(2, 1));
    let error = clause
        .selections(&[])
        .expect_err("reversed direct-selection ranges must fail closed");
    assert!(error.msg.contains("outside a table"));
}

#[test]
fn direct_selection_out_of_bounds_range_is_rejected_in_the_internal_error_lane() {
    let clause = malformed_direct_selection_clause(DependencySelectionRange::new(0, 1));
    let error = clause
        .selections(&[])
        .expect_err("out-of-bounds direct-selection ranges must fail closed");
    assert!(error.msg.contains("outside a table"));
}

fn malformed_direct_selection_clause(range: DependencySelectionRange) -> RetainedDependencyClause {
    let provider = RetainedDependencyPath {
        path: InternedPath::new(),
        path_syntax: crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
        target: crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::Source,
        location: SourceLocation::default(),
        dependency_shell_id: DependencyShellId::new(FileId(0), 0),
    };
    RetainedDependencyClause {
        dependency: provider,

        binding: DependencyBindingSyntax::DirectSelections { range },
        location: SourceLocation::default(),
        export_mode: HeaderExportMode::Private,
    }
}

#[test]
fn namespace_alias_duplicate_declaration_uses_the_alias_span() {
    let result = parse_single_file_headers_with_entry(
        "@core/io as io\nio #= 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "a namespace alias must conflict with a declaration");
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let DiagnosticPayload::DuplicateDeclaration {
        first_location: Some(first_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the namespace alias to be the first location");
    };
    assert_eq!(first_location.start_pos.line_number, 0);
    assert_eq!(first_location.start_pos.char_column, 13);
    assert_eq!(first_location.end_pos.char_column, 14);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 2);
}

#[test]
fn inferred_namespace_duplicate_declaration_uses_the_provider_path_span() {
    let result = parse_single_file_headers_with_entry(
        "@core/io\nio #= 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "an inferred namespace name must conflict with a declaration",
    );
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let DiagnosticPayload::DuplicateDeclaration {
        first_location: Some(first_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the provider path to be the first location");
    };
    assert_eq!(first_location.start_pos.line_number, 0);
    assert_eq!(first_location.start_pos.char_column, 1);
    assert_eq!(first_location.end_pos.char_column, 8);
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 1);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 1);
    assert_eq!(diagnostic.primary_location.end_pos.char_column, 2);
}

#[test]
fn inferred_namespace_provider_path_span_excludes_trailing_whitespace() {
    let result = parse_single_file_headers_with_entry(
        "@core/io   \nio #= 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "trailing whitespace must not enter the inferred namespace path span",
    );
    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::DuplicateDeclaration { .. }
            )
        })
        .expect("expected duplicate declaration diagnostic");

    let DiagnosticPayload::DuplicateDeclaration {
        first_location: Some(first_location),
        ..
    } = &diagnostic.payload
    else {
        panic!("expected the inferred namespace path to be the first location");
    };
    assert_eq!(first_location.start_pos.line_number, 0);
    assert_eq!(first_location.start_pos.char_column, 1);
    assert_eq!(first_location.end_pos.char_column, 8);
}

#[test]
fn retained_clause_uses_one_shell_for_the_provider_binding_index() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/helper.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let output = prepare_single_file(
        "@drawing.js draw, clear\n",
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let clauses = &output.file_dependency_clauses;
    assert_eq!(clauses.len(), 1);
    let shell = clauses[0].dependency.dependency_shell_id;
    assert!(
        clauses
            .iter()
            .all(|clause| clause.dependency.dependency_shell_id == shell)
    );
}
