//! Parser-stage frontend test support.
//!
//! WHAT: turns single-file source snippets into ASTs or typed diagnostics.
//! WHY: parser and AST diagnostics tests need a stable frontend setup without depending on HIR
//!      lowering or borrow-checker helpers.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::{
    Ast, AstBuildContext, AstBuildInput, AstBuildResult, FileValueResolutionServices,
    Stage0ResolutionFacts,
};
use crate::compiler_frontend::compiler_errors::BoundaryFailure;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticSeverity, InvalidCompileTimePathReason, PreparedFailure,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderParseOptions, HeaderPreparationFailure, bind_module_headers, prepare_file_from_tokens,
    prepare_header_syntax,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, PreparedFileReferenceTable, ResolvedFileReference,
    ResolvedFileReferenceFailure, ResolvedFileReferenceOutcome, ResolvedFileReferenceTable,
    ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::test_support::TestSourceContext;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::TokenizeFailure;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub(crate) fn test_project_path_resolver() -> ProjectPathResolver {
    let cwd = std::env::temp_dir();

    ProjectPathResolver::new_with_module_roots(
        cwd.clone(),
        cwd,
        PreparedSourcePackageRoots::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        ModuleRootTable::empty(),
    )
    .expect("test path resolver should be valid")
}

/// Build deterministic Stage 0 facts for parser tests without reading the filesystem.
///
/// The single-file parser fixture has no project inventory, so file-value paths are represented
/// as missing-target diagnostics while site roots and source-kind-only paths keep their structural
/// outcomes. This exercises the same AST handoff as a prepared module.
fn test_file_value_resolution_services(
    source_file: SourceId,
    references: &PreparedFileReferenceTable,
    path_syntax: &PathSyntaxTable,
) -> Rc<FileValueResolutionServices> {
    let mut resolved_references = ResolvedFileReferenceTable::new();

    for reference in references.iter() {
        let outcome = match reference.class {
            PreparedFileReferenceClass::SiteRoot | PreparedFileReferenceClass::Extensionless => {
                ResolvedFileReferenceOutcome::NoPhysicalTarget
            }
            PreparedFileReferenceClass::SourceKindNoFileValue => {
                ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::IdentifiedSourceKind,
                )
            }
            PreparedFileReferenceClass::ContentSource
            | PreparedFileReferenceClass::ResourceFile => ResolvedFileReferenceOutcome::Diagnostic(
                ResolvedFileReferenceFailure::InvalidCompileTimePath {
                    path: path_syntax
                        .try_path(reference.path_syntax)
                        .expect("prepared reference should point into its path table")
                        .root,
                    reason: InvalidCompileTimePathReason::MissingTarget,
                    span: reference.span,
                },
            ),
        };

        resolved_references
            .push(ResolvedFileReference {
                source_file,
                path_syntax: reference.path_syntax,
                class: reference.class,
                outcome,
            })
            .expect("parser fixture should contain unique path handles");
    }

    Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
            resolved_references,
            SourceDatabase::empty().into(),
        ))),
        module_resources: Rc::new(RefCell::new(ModuleResourceTable::new())),
        module_origin: None,
        frozen_identity_handle: FrozenIdentityHandle::new(),
    })
}

fn capture_diagnostic_failure(
    diagnostic: DiagnosticDraft,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PreparedFailure {
    capture_diagnostic_bag_failure(DiagnosticBag::from(diagnostic), string_table, path_fork)
}

fn capture_diagnostic_bag_failure(
    bag: DiagnosticBag,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> PreparedFailure {
    PreparedFailure::capture_diagnosed(
        bag,
        string_table.clone(),
        None,
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    )
}

pub(crate) fn freeze_ast_diagnostic_failure(failure: PreparedFailure) -> BoundaryFailure {
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            let mut accumulator = BoundaryAccumulator::new();
            accumulator.append_prepared(batch, None);
            accumulator.finish(InstallMode::Frozen)
        }
        PreparedFailure::Infrastructure(error) | PreparedFailure::Mixed { error, .. } => {
            panic!("single-file AST construction infrastructure failure: {error:?}")
        }
    }
}

pub(crate) fn parse_single_file_ast_build_result(
    source: &str,
) -> Result<(AstBuildResult, PathInternerFork, StringTable), PreparedFailure> {
    parse_single_file_ast_build_result_with_registry(source, ExternalPackageRegistry::new())
}

pub(crate) fn parse_single_file_ast_build_result_with_registry(
    source: &str,
    registry: ExternalPackageRegistry,
) -> Result<(AstBuildResult, PathInternerFork, StringTable), PreparedFailure> {
    let mut source_context = TestSourceContext::new("@page.moth");

    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let external_package_registry = Arc::new(registry);
    let file_path = source_context.path().to_path_buf();
    let project_path_resolver = test_project_path_resolver();

    let options = HeaderParseOptions {
        entry_file_id: None,
        entry_file_role: None,
        active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
    };

    let lexed = match source_context.tokenize(
        source,
        &mut path_fork,
        &style_directives,
        TokenizerEntryMode::SourceFile,
    ) {
        Ok(lexed) => lexed,
        Err(TokenizeFailure::Diagnosed(diagnostic)) => {
            let (string_table, _) = source_context.preparation_parts();
            return Err(capture_diagnostic_failure(
                diagnostic.into(),
                string_table,
                &path_fork,
            ));
        }
        Err(TokenizeFailure::Infrastructure(error)) => {
            panic!("parse fixture tokenization encountered infrastructure failure: {error:?}")
        }
    };
    let owner = SourceTokenOwner::new(lexed.tokens);
    let path_syntax = lexed.path_syntax;
    let (string_table, span_builder) = source_context.preparation_parts();

    let output = match prepare_file_from_tokens(
        owner,
        lexed.logical_path,
        path_syntax,
        &file_path,
        &options,
        string_table,
        0,
        0,
        span_builder,
        &mut path_fork,
    ) {
        Ok(output) => output,
        Err(crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Diagnosed(
            crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareError {
                diagnostic, ..
            },
        )) => return Err(capture_diagnostic_failure(diagnostic.into(), string_table, &path_fork)),
        Err(crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Infrastructure(
            error,
        )) => panic!("single-file test preparation hit infrastructure failure: {error:?}"),
    };
    let source_file_id = output.file_id;
    let file_value_resolution = test_file_value_resolution_services(
        source_file_id,
        &output.structural_file_references,
        output.path_syntax.table(),
    );

    let prepared_syntax = prepare_header_syntax(
        &mut [output],
        string_table,
        &mut |source, diagnostic| {
            assert_eq!(source, source_file_id);
            diagnostic.capture_preparation_span(source)
        },
        &mut path_fork,
    )
    .map_err(|failure| {
        let bag = match failure {
            HeaderPreparationFailure::Diagnosed(bag) => bag,
            HeaderPreparationFailure::Infrastructure(error) => {
                panic!("single-file test aggregation hit infrastructure failure: {error:?}")
            }
        };
        capture_diagnostic_bag_failure(bag, string_table, &path_fork)
    })?;

    let headers = bind_module_headers(
        prepared_syntax,
        external_package_registry.as_ref(),
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        Some(&project_path_resolver),
        &crate::compiler_frontend::source::SourceDatabase::empty(),
        string_table,
        &mut path_fork,
    )
    .map_err(|failure| {
        let bag = match failure {
            HeaderPreparationFailure::Diagnosed(bag) => bag,
            HeaderPreparationFailure::Infrastructure(error) => {
                panic!("single-file test binding hit infrastructure failure: {error:?}")
            }
        };
        capture_diagnostic_bag_failure(bag, string_table, &path_fork)
    })?;

    let sorted = match resolve_module_dependencies(
        headers,
        &ContentSourceTargets::empty(),
        string_table,
        &mut path_fork,
    ) {
        Ok(sorted) => sorted,
        Err(PreparedFailure::Diagnosed(batch)) => {
            return Err(PreparedFailure::Diagnosed(batch));
        }
        Err(PreparedFailure::Infrastructure(error)) => {
            panic!("dependency sorting infrastructure failure: {error:?}")
        }
        Err(PreparedFailure::Mixed { error, .. }) => {
            panic!("dependency sorting infrastructure failure alongside diagnostics: {error:?}")
        }
    };

    let entry_path = path_fork
        .try_intern_portable_path("@page.moth", string_table)
        .expect("test entry path fits");
    let build_result = Ast::new(
        AstBuildInput {
            headers: sorted.headers,
            source_token_owners: sorted.source_token_owners,
            module_symbols: sorted.module_symbols,
            binding_environment: sorted.binding_environment,
            top_level_const_fragments: sorted.top_level_const_fragments,
            source_build_config_contract_names: Arc::new(Default::default()),
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry,
            style_directives: &style_directives,
            path_fork: &mut path_fork,
            string_table,
            entry_dir: entry_path,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: Some(file_value_resolution),
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
        },
    )?;

    Ok((build_result, path_fork, std::mem::take(string_table)))
}

pub(crate) fn parse_single_file_ast_result(
    source: &str,
) -> Result<(Ast, PathInternerFork, StringTable), PreparedFailure> {
    parse_single_file_ast_build_result(source)
        .map(|(build_result, path_fork, string_table)| (build_result.ast, path_fork, string_table))
}

pub(crate) fn parse_single_file_ast(source: &str) -> (Ast, PathInternerFork, StringTable) {
    parse_single_file_ast_result(source).expect("source should parse into AST")
}

pub(crate) fn parse_single_file_ast_diagnostic(source: &str) -> BoundaryFailure {
    match parse_single_file_ast_result(source) {
        Ok(_) => panic!("source should fail during frontend parsing"),
        Err(failure) => freeze_ast_diagnostic_failure(failure),
    }
}

pub(crate) fn first_ast_error_view(failure: &BoundaryFailure) -> DiagnosticView<'_> {
    let (_, diagnostic) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("AST failure should contain an error diagnostic");
    diagnostic
}

pub(crate) fn ast_first_error_type_display(failure: &BoundaryFailure, id: TypeDisplayId) -> String {
    let (report, diagnostic) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("AST failure should contain an error diagnostic");
    let context = DiagnosticRenderContext::for_report(report);
    diagnostic
        .types()
        .display_for_render(id, context.string_table, context.path_table())
}

pub(crate) fn ast_first_error_rendered_message(failure: &BoundaryFailure) -> String {
    let (report, diagnostic) = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("AST failure should contain an error diagnostic");
    let render = diagnostic
        .family()
        .render
        .expect("a produced AST diagnostic has a family renderer");
    render(&diagnostic, DiagnosticRenderContext::for_report(report)).message
}

#[track_caller]
pub(crate) fn assert_ast_diagnostic<'a>(
    failure: &'a BoundaryFailure,
    expected_code: &str,
    expected_reason_key: Option<&str>,
) -> DiagnosticView<'a> {
    let diagnostic = first_ast_error_view(failure);
    assert_eq!(
        diagnostic.descriptor().code,
        expected_code,
        "AST failure's first error should be {expected_code}"
    );
    assert_eq!(diagnostic.reason_key(), expected_reason_key);
    diagnostic
}

pub(crate) fn ast_error_primary_span(diagnostic: DiagnosticView<'_>) -> Option<SourceSpan> {
    match diagnostic.primary_site() {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}

pub(crate) fn ast_error_label_spans(diagnostic: DiagnosticView<'_>) -> Vec<Option<SourceSpan>> {
    diagnostic
        .durable_labels()
        .map(|label| match label.site {
            SiteRef::Absent => None,
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
        })
        .collect()
}

/// Tokenizes one source file through a [`CompilerFrontend`] instance for test suites.
///
/// WHAT: provides a test-support helper that calls the real `tokenize_source` implementation.
/// WHY: tokenization-only tests need access to the tokenizer without taking ownership of the
///      frontend's string table, and this keeps test-only entry points out of production code.
///      The caller owns the span builder for the tokenized source and keeps it alive wherever its
///      assertions still resolve spans; this helper only borrows it for the lexical pass.
pub(crate) fn tokenize_source_for_test(
    frontend: &mut CompilerFrontend<'_>,
    source_code: &str,
    module_path: &std::path::Path,
    tokenizer_entry_mode: TokenizerEntryMode,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<(SourceTokenOwner, Arc<PathSyntaxTable>), DiagnosticDraft> {
    let (owner, path_syntax) = CompilerFrontend::tokenize_source(
        frontend.source_files.as_ref(),
        frontend.style_directives,
        source_code,
        module_path,
        tokenizer_entry_mode,
        &mut frontend.string_table,
        &mut frontend.path_fork,
        span_builder,
    )
    .map_err(|error| match error {
        crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Diagnosed(
            crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareError {
                diagnostic, ..
            },
        ) => DiagnosticDraft::from(diagnostic),
        crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure::Infrastructure(
            error,
        ) => panic!("tokenization test hit infrastructure failure: {error:?}"),
    })?;
    Ok((owner, path_syntax))
}
