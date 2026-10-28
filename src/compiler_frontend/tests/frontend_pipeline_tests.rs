//! Stage-boundary regression tests for the frontend stage facade.
//!
//! WHAT: drives tokenization, header preparation, binding, declaration ordering, AST construction,
//!       HIR lowering and borrow validation one stage at a time, so a test can assert the
//!       intermediate value a stage produced rather than only the final outcome.
//! WHY:  stage-local unit tests can all pass while the handoff between two stages breaks. These
//!       tests own the handoffs: source identity surviving into diagnostics, declaration ordering
//!       surviving into lowering, and project-owned style directives reaching header parsing.
//!
//! This harness is NOT the canonical module compilation sequence and must not be read as a model
//! of it. [`module_compilation::compile_module`](crate::compiler_frontend::module_compilation)
//! is the one production owner: it also projects the public interface, completes generated
//! functions and converges their summaries, none of which happen here. Coverage for those belongs
//! to `public_interface/tests/`, `module_compilation/generated/tests/` and the build-system suites
//! that run the real `HirFunctionOriginLookup`.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::ConstValuePayload;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, ImplicitFailureSource,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidFallibleHandlingCode, InvalidFallibleHandlingView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag, InvalidFallibleHandlingReason, PreparedFailure,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType, ExternalAccessKind, ExternalFunctionDef,
    ExternalFunctionLowerings, ExternalParameter, ExternalReturnSlot, ExternalSignatureType,
};
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderParseOptions, bind_module_headers, prepare_file_from_tokens,
    prepare_header_syntax,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::failure_facts::{
    HirBuiltinFailureBoundary, HirBuiltinFailureSource,
};
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirFunctionOriginLookup};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureCause,
};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::{
    StyleDirectiveEffects, StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::{
    ast_first_error_type_display, tokenize_source_for_test,
};
use crate::compiler_frontend::tokenizer::tokens::{TemplateBodyMode, TokenizerEntryMode};
use crate::compiler_frontend::{AstBuildRequest, CompilerFrontend, FrontendBuildProfile};
use crate::projects::settings::Config;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use rustc_hash::FxHashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn freeze_prepared_failure(
    failure: PreparedFailure,
    source_files: Arc<SourceDatabase>,
    string_table: &StringTable,
) -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table.clone());
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, Some((source_files, None)));
        }
        PreparedFailure::Infrastructure(error) => {
            accumulator.append_infrastructure(error);
        }
    }
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

trait AstFailureResultExt<T> {
    fn expect_ast_failure(self, message: &str) -> CompilationResult<()>;
    fn expect_diagnosed(self, message: &str) -> CompilationResult<()>;
}

impl<T> AstFailureResultExt<T> for CompilationResult<T> {
    fn expect_ast_failure(self, message: &str) -> CompilationResult<()> {
        self.expect_diagnosed(message)
    }

    fn expect_diagnosed(self, message: &str) -> CompilationResult<()> {
        match self {
            Ok(CompilationOutcome::Diagnosed(reports)) => {
                Ok(CompilationOutcome::Diagnosed(reports))
            }
            Err(_) => panic!("{message}: expected diagnosed result, got infrastructure errors"),
            Ok(CompilationOutcome::Success { .. }) => {
                panic!("{message}: expected diagnosed result, got successful compilation")
            }
        }
    }
}

fn diagnosed_reports<T>(result: &CompilationResult<T>) -> &DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Err(_) => panic!("expected diagnosed compilation result, got infrastructure errors"),
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("expected diagnosed compilation result, got successful compilation")
        }
    }
}

fn single_ast_error_view<T>(result: &CompilationResult<T>) -> DiagnosticView<'_> {
    let mut errors = diagnosed_reports(result)
        .iter_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error);
    let (_, diagnostic) = errors
        .next()
        .expect("AST failure should contain an error diagnostic");
    assert!(
        errors.next().is_none(),
        "AST failure should contain one error"
    );
    diagnostic
}

#[track_caller]
fn assert_ast_diagnostic<'a, T>(
    result: &'a CompilationResult<T>,
    expected_code: &str,
    expected_reason_key: Option<&str>,
) -> DiagnosticView<'a> {
    let diagnostic = single_ast_error_view(result);
    assert_eq!(diagnostic.descriptor().code, expected_code);
    assert_eq!(diagnostic.reason_key(), expected_reason_key);
    diagnostic
}

fn diagnostic_label_spans(
    view: DiagnosticView<'_>,
) -> Vec<Option<crate::compiler_frontend::source::SourceSpan>> {
    view.durable_labels()
        .map(|label| match label.site {
            SiteRef::Absent => None,
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
        })
        .collect()
}

fn diagnostic_primary_span(
    view: DiagnosticView<'_>,
) -> Option<crate::compiler_frontend::source::SourceSpan> {
    match view.primary_site() {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}
fn fallible_reason_key(reason: &InvalidFallibleHandlingReason) -> &'static str {
    match reason {
        InvalidFallibleHandlingReason::CatchOutsideBoundary => {
            "invalid_fallible_handling.catch_outside_boundary"
        }
        InvalidFallibleHandlingReason::CatchOnNonFallible => {
            "invalid_fallible_handling.catch_on_non_fallible"
        }
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough => {
            "invalid_fallible_handling.catch_handler_can_fall_through"
        }
        InvalidFallibleHandlingReason::InlineCatchMultiline => {
            "invalid_fallible_handling.inline_catch_multiline"
        }
        InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict => {
            "invalid_fallible_handling.explicit_propagation_catch_conflict"
        }
        InvalidFallibleHandlingReason::OptionPropagationCatchConflict => {
            "invalid_fallible_handling.option_propagation_catch_conflict"
        }
        InvalidFallibleHandlingReason::AssertionMessageCannotEscape => {
            "invalid_fallible_handling.assertion_message_cannot_escape"
        }
        InvalidFallibleHandlingReason::UnhandledErrorReturn => {
            "invalid_fallible_handling.unhandled_error_return"
        }
        InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure { .. } => {
            "invalid_fallible_handling.custom_error_mixed_with_implicit_failure"
        }
        InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes { .. } => {
            "invalid_fallible_handling.incompatible_catch_error_types"
        }
        InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction { .. } => {
            "invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function"
        }
        InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { .. } => {
            "invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"
        }
        _ => panic!("missing stable reason-key assertion for {reason:?}"),
    }
}

#[track_caller]
fn assert_ast_fallible_reason<T>(
    failure: &CompilationResult<T>,
    expected: InvalidFallibleHandlingReason,
) -> DiagnosticView<'_> {
    let diagnostic = assert_ast_diagnostic(
        failure,
        "MOTH-RULE-0051",
        Some(fallible_reason_key(&expected)),
    );
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "fallible handling diagnostic should retain its primary source site"
    );
    diagnostic
}
fn assert_ast_compile_time_reason<T>(
    failure: &CompilationResult<T>,
    expected: CompileTimeEvaluationErrorReason,
) -> DiagnosticView<'_> {
    let reason_key = match expected {
        CompileTimeEvaluationErrorReason::DivideByZero => {
            "compile_time_evaluation_error.divide_by_zero"
        }
        CompileTimeEvaluationErrorReason::IntegerOverflow => {
            "compile_time_evaluation_error.integer_overflow"
        }
        CompileTimeEvaluationErrorReason::InvalidExponent => {
            "compile_time_evaluation_error.invalid_exponent"
        }
        _ => panic!("missing stable compile-time reason-key assertion for {expected:?}"),
    };
    let diagnostic = assert_ast_diagnostic(failure, "MOTH-RULE-0053", Some(reason_key));
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "compile-time evaluation diagnostic should retain its primary source site"
    );
    diagnostic
}

fn assert_ast_unhandled_builtin_export<T>(failure: &CompilationResult<T>) -> DiagnosticView<'_> {
    assert_ast_diagnostic(
        failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
    )
}

/// One materialised `I32` fixture value; panics when the fixture literal is out of range.
fn i32_value(value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range")
}

type TokenizedFile = (
    (SourceTokenOwner, PathId, Arc<PathSyntaxTable>),
    ExtendedSpanBuilder,
);

struct FrontendServices {
    options: crate::compiler_frontend::module_compilation::FrontendOptions,
    external_package_registry:
        Arc<crate::compiler_frontend::external_packages::ExternalPackageRegistry>,
    style_directives: StyleDirectiveRegistry,
    string_table: StringTable,
    path_fork: PathInternerFork,
    project_path_resolver: Option<ProjectPathResolver>,
    source_files: Arc<SourceDatabase>,
}

impl FrontendServices {
    /// Moves the fixture's source database out, so the frozen failure becomes its last owner.
    fn take_source_files(&mut self) -> Arc<SourceDatabase> {
        std::mem::replace(&mut self.source_files, Arc::new(SourceDatabase::empty()))
    }

    fn with_compiler<R>(&mut self, run: impl FnOnce(&mut CompilerFrontend<'_>) -> R) -> R {
        let mut compiler = CompilerFrontend::new(
            self.options.clone(),
            std::mem::take(&mut self.string_table),
            std::mem::replace(&mut self.path_fork, self.source_files.fork_path_interner()),
            &self.style_directives,
            &self.external_package_registry,
            self.project_path_resolver.as_ref(),
            &self.source_files,
        );
        let result = run(&mut compiler);
        self.string_table = compiler.string_table;
        self.path_fork = compiler.path_fork;
        result
    }
}

struct FrontendProject {
    _temp_dir: TempDir,
    entry_file: PathBuf,
    files: Vec<PathBuf>,
    frontend: FrontendServices,
}

impl FrontendProject {
    fn new(
        files: &[(&str, &str)],
        entry_relative_path: &str,
        style_directives: StyleDirectiveRegistry,
    ) -> Self {
        let temp_dir = tempfile::tempdir().expect("should create temp dir");
        let project_root = temp_dir.path().join("project");
        let entry_root = project_root.join("src");
        fs::create_dir_all(&entry_root).expect("should create project entry root");

        let mut canonical_files = Vec::with_capacity(files.len());
        for (relative_path, source) in files {
            let full_path = project_root.join(relative_path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).expect("should create parent directories");
            }
            fs::write(&full_path, source).expect("should write test source");
            canonical_files.push(
                fs::canonicalize(&full_path).expect("test source should canonicalize after write"),
            );
        }

        let canonical_project_root =
            fs::canonicalize(&project_root).expect("project root should canonicalize");
        let canonical_entry_root =
            fs::canonicalize(&entry_root).expect("entry root should canonicalize");
        let entry_file = fs::canonicalize(project_root.join(entry_relative_path))
            .expect("entry file should canonicalize");
        let resolver = ProjectPathResolver::new_with_module_roots(
            canonical_project_root.clone(),
            canonical_entry_root,
            PreparedSourcePackageRoots::default(),
            &crate::builder_surface::SourceFileKindRegistry::default(),
            ModuleRootTable::empty(),
        );

        let mut string_table = StringTable::new();
        let source_files = Arc::new(
            SourceDatabase::build(
                &canonical_files,
                &entry_file,
                Some(&resolver),
                &mut string_table,
            )
            .expect("source file table should build"),
        );

        let path_fork = source_files.fork_path_interner();
        let frontend = FrontendServices {
            options: Config::new(canonical_project_root).frontend_options(),
            string_table,
            path_fork,
            style_directives,
            external_package_registry: Arc::new(
                crate::compiler_frontend::external_packages::ExternalPackageRegistry::new(),
            ),
            project_path_resolver: Some(resolver),
            source_files,
        };

        Self {
            _temp_dir: temp_dir,
            entry_file,
            files: canonical_files,
            frontend,
        }
    }

    fn tokenize_all(&mut self) -> Vec<TokenizedFile> {
        let mut tokenized_files = Vec::with_capacity(self.files.len());
        self.frontend.with_compiler(|frontend| {
            for file in &self.files {
                let source = fs::read_to_string(file).expect("should read source file");
                let mut span_builder = ExtendedSpanBuilder::new();
                let source_file = frontend
                    .source_files
                    .get_by_canonical_path(file)
                    .expect("test source should have a registered identity")
                    .logical_path;
                tokenized_files.push(
                    tokenize_source_for_test(
                        frontend,
                        &source,
                        file,
                        TokenizerEntryMode::SourceFile,
                        &mut span_builder,
                    )
                    .map(|(owner, path_syntax)| ((owner, source_file, path_syntax), span_builder))
                    .expect("tokenization should succeed"),
                );
            }
        });
        tokenized_files
    }

    fn headers(&mut self) -> BoundModuleHeaders {
        let tokenized_files = self.tokenize_all();
        let entry_file_id = self
            .frontend
            .source_files
            .get_by_canonical_path(&self.entry_file)
            .map(|identity| identity.id);

        let options = HeaderParseOptions {
            entry_file_id,
            entry_file_role: None,
            active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        };

        let mut prepared_outputs = Vec::with_capacity(tokenized_files.len());
        let mut const_template_offset = 0usize;
        let mut runtime_fragment_offset = 0usize;
        for ((owner, source_file, path_syntax), mut span_builder) in tokenized_files {
            let output = prepare_file_from_tokens(
                owner,
                source_file,
                path_syntax,
                &self.entry_file,
                &options,
                &mut self.frontend.string_table,
                const_template_offset,
                runtime_fragment_offset,
                &mut span_builder,
                &mut self.frontend.path_fork,
            )
            .expect("header parsing should succeed");

            const_template_offset += output.const_template_count;
            runtime_fragment_offset += output.runtime_fragment_count;
            prepared_outputs.push(output);
        }

        let prepared_syntax = prepare_header_syntax(
            &mut prepared_outputs,
            &mut self.frontend.string_table,
            &mut |source, diagnostic| crate::compiler_frontend::compiler_messages::diagnostics::draft::validate_preparation_draft_spans(diagnostic, source),
            &mut self.frontend.path_fork,
        )
        .expect("header syntax preparation should succeed");
        bind_module_headers(
            prepared_syntax,
            self.frontend.external_package_registry.as_ref(),
            &ExternalImportResolutionTable::default(),
            &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
            self.frontend.project_path_resolver.as_ref(),
            self.frontend.source_files.as_ref(),
            &mut self.frontend.string_table,
            &mut self.frontend.path_fork,
        )
        .expect("header binding should succeed")
    }

    fn sorted_headers(&mut self) -> crate::compiler_frontend::module_dependencies::SortedHeaders {
        let headers = self.headers();
        self.frontend
            .with_compiler(|compiler| {
                compiler.sort_headers(headers, &ResolvedFileReferenceTable::default())
            })
            .expect("header sorting should succeed")
    }

    fn ast_result(&mut self) -> CompilationResult<Ast> {
        // These fixtures have valid declaration shells. Body diagnostics belong to the AST result
        // and are frozen only when a test inspects them.
        let sorted = self.sorted_headers();
        let result = self.frontend.with_compiler(|compiler| {
            compiler.headers_to_ast(
                AstBuildRequest {
                    sorted,
                    entry_file_path: &self.entry_file,
                    root_role: ModuleRootRole::Normal,
                    build_profile: FrontendBuildProfile::Dev,
                    capacity_estimate: Default::default(),
                    resolved_file_references: ResolvedFileReferenceTable::default(),
                    module_origin: None,
                    build_config_values: Arc::new(Default::default()),
                },
                #[cfg(feature = "timers")]
                None,
            )
        });
        match result {
            Ok(result) => Ok(CompilationOutcome::Success {
                value: result.ast,
                warnings: None,
            }),
            Err(failure) => match freeze_prepared_failure(
                failure,
                self.frontend.take_source_files(),
                &self.frontend.string_table,
            ) {
                Ok(CompilationOutcome::Diagnosed(reports)) => {
                    Ok(CompilationOutcome::Diagnosed(reports))
                }
                Err(errors) => Err(errors),
                Ok(CompilationOutcome::Success { .. }) => {
                    unreachable!("freezing a failure cannot produce a successful outcome")
                }
            },
        }
    }

    fn ast(&mut self) -> Ast {
        match self.ast_result() {
            Ok(CompilationOutcome::Success { value, .. }) => value,
            Err(errors) => panic!(
                "AST construction failed due to infrastructure errors: {}",
                errors.primary.message.as_ref()
            ),
            Ok(CompilationOutcome::Diagnosed(reports)) => panic!(
                "AST construction was diagnosed with {} report(s)",
                reports.iter().count()
            ),
        }
    }

    fn lower_ast_result(&mut self, ast: Ast) -> Result<HirModule, PreparedFailure> {
        self.frontend
            .with_compiler(|compiler| {
                compiler.generate_hir(
                    ast,
                    HirFunctionOriginLookup::default(),
                    None,
                    Default::default(),
                )
            })
            .map(|result| result.hir_module)
    }

    /// Exercise the summary owner's semantic contract checks without claiming backend delivery.
    fn failure_contract_result(&mut self) -> Result<HirModule, PreparedFailure> {
        let ast = self.ast();
        let lowered = self
            .frontend
            .with_compiler(|compiler| {
                compiler.generate_hir(
                    ast,
                    HirFunctionOriginLookup::default(),
                    None,
                    Default::default(),
                )
            })
            .expect("HIR lowering should succeed before checking its boundary contract");
        let type_environment = lowered.type_environment;
        let hir = lowered.hir_module;
        let mut report = self
            .frontend
            .with_compiler(|compiler| compiler.check_borrows(&hir))?;
        crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
            &hir,
            &mut report,
        );

        for function in &hir.functions {
            if let Some(diagnostic) =
                crate::compiler_frontend::module_compilation::generated::builtin_failure_diagnostic(
                    &hir,
                    &report,
                    function.id,
                )
            {
                return Err(PreparedFailure::capture_diagnosed(
                    DiagnosticBag::from(diagnostic),
                    self.frontend.string_table.clone(),
                    Some(&type_environment),
                    Some(Arc::new(self.frontend.path_fork.snapshot_table())),
                ));
            }
        }
        Ok(hir)
    }

    fn failure_contract_boundary_result(&mut self) -> CompilationResult<()> {
        match self.failure_contract_result() {
            Ok(_) => Ok(CompilationOutcome::Success {
                value: (),
                warnings: None,
            }),
            Err(failure) => freeze_prepared_failure(
                failure,
                self.frontend.take_source_files(),
                &self.frontend.string_table,
            ),
        }
    }

    fn hir(&mut self) -> crate::compiler_frontend::hir::module::HirModule {
        let ast = self.ast();
        self.lower_ast_result(ast)
            .expect("HIR lowering should succeed")
    }

    /// Lower to HIR and borrow-check it, returning both so tests can assert the exact
    /// relationship between the module and the facts derived from it.
    fn borrow_checked_hir(&mut self) -> (HirModule, BorrowCheckReport) {
        let hir = self.hir();
        let report = self
            .frontend
            .with_compiler(|compiler| compiler.check_borrows(&hir))
            .expect("borrow checking should succeed");
        (hir, report)
    }
}

/// The multiset of function origins in a lowered module, sorted for comparison.
fn function_origins(hir: &HirModule) -> Vec<HirFunctionOrigin> {
    let mut origins = hir
        .functions
        .iter()
        .map(|function| {
            *hir.function_origins
                .get(&function.id)
                .unwrap_or_else(|| panic!("function {:?} has no recorded origin", function.id))
        })
        .collect::<Vec<_>>();
    origins.sort_by_key(|origin| match origin {
        HirFunctionOrigin::EntryStart => 0,
        HirFunctionOrigin::Normal => 1,
    });
    origins
}

/// Assert the borrow report's side tables describe exactly the module that was checked.
///
/// WHAT: requires one summary per lowered function, no summary for a function that does not
///       exist, and statement facts keyed only by statements of this module.
/// WHY: `!statement_facts.is_empty()` passes for a report about a different module, or a report
///      that covered one function and skipped the rest.
#[track_caller]
fn assert_report_describes_module(hir: &HirModule, report: &BorrowCheckReport) {
    assert_eq!(
        report.stats.functions_analyzed,
        hir.functions.len(),
        "every lowered function must be analyzed"
    );

    let mut summarized = report
        .analysis
        .function_summaries
        .keys()
        .copied()
        .collect::<Vec<_>>();
    summarized.sort_by_key(|id| format!("{id:?}"));
    let mut lowered = hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    lowered.sort_by_key(|id| format!("{id:?}"));
    assert_eq!(
        summarized, lowered,
        "borrow summaries must cover exactly the lowered functions"
    );

    let module_statement_ids = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter().map(|statement| statement.id))
        .collect::<std::collections::HashSet<_>>();
    for fact_id in report.analysis.statement_facts.keys() {
        assert!(
            module_statement_ids.contains(fact_id),
            "statement fact {fact_id:?} does not belong to the checked module"
        );
    }
}

#[test]
fn compiles_single_file_program_through_borrow_check() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "Point = |\n    value I32,\n|\npoint = Point(1)\nloop 0 to 2 |i|:\n    io.line([: [point.value]])\n;\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let (hir, report) = project.borrow_checked_hir();

    // A program whose only executable code is at module root lowers to exactly one function:
    // the implicit entry start. An inequality would also accept a lowering that invented
    // extra functions or dropped the loop body into one.
    assert_eq!(
        function_origins(&hir),
        vec![HirFunctionOrigin::EntryStart],
        "module-root-only code lowers to exactly the implicit entry start"
    );
    assert_report_describes_module(&hir, &report);

    // Every statement of the entry block is reachable by construction, so each one must carry
    // a borrow fact.
    let entry_block = hir
        .blocks
        .iter()
        .find(|block| block.id == hir.functions[0].entry)
        .expect("the entry function's block should exist");
    assert!(
        !entry_block.statements.is_empty(),
        "the entry block should contain the lowered module-root statements"
    );
    for statement in &entry_block.statements {
        assert!(
            report.analysis.statement_fact(statement.id).is_some(),
            "entry-block statement {:?} has no borrow fact",
            statement.id
        );
    }
}

#[test]
fn compiles_multi_file_dependency_program_through_borrow_check() {
    let mut project = FrontendProject::new(
        &[
            (
                "src/@page.moth",
                "@helper add\nresult = add(1, 2)\nio.line([: [result]])\n",
            ),
            (
                "src/helper.moth",
                "add|left I32, right I32| -> I32:\n    return left + right\n;\n",
            ),
        ],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let (hir, report) = project.borrow_checked_hir();

    // The imported helper lowers beside the implicit entry start: exactly two functions, one
    // of each origin. `>= 2` would also accept a lowering that duplicated the helper.
    assert_eq!(
        function_origins(&hir),
        vec![HirFunctionOrigin::EntryStart, HirFunctionOrigin::Normal],
        "a two-file program lowers the entry start plus the imported helper"
    );
    assert_report_describes_module(&hir, &report);

    // The lowered helper is the imported `add|left I32, right I32|`, not just "some function":
    // it takes exactly the two declared parameters and carries a stable exported origin.
    let helper = hir
        .functions
        .iter()
        .find(|function| hir.function_origins.get(&function.id) == Some(&HirFunctionOrigin::Normal))
        .expect("the imported helper should lower to a Normal function");
    assert_eq!(
        helper.params.len(),
        2,
        "the imported helper declares two parameters"
    );
    // The origin side tables stay empty here because this pipeline harness lowers with an
    // empty `HirFunctionOriginLookup`; stable-origin joins are owned by the build-system
    // tests that run the real lookup.
    assert!(
        report.analysis.function_summaries.contains_key(&helper.id),
        "the imported helper must carry its own borrow summary"
    );
}
#[test]
fn frontend_diagnostics_preserve_source_spans() {
    let mut project = FrontendProject::new(
        &[("src/@page.moth", "bad #= io.line(\"runtime host call\")\n")],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("const host calls should fail during AST construction");
    let diagnostic = single_ast_error_view(&failure);
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "AST errors should preserve an authored source span"
    );

    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "data ~= [\"shared data\"]\nref1 ~= data\nref2 ~= data\nresult = [ref1, ref2]\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let hir = project.hir();
    let prepared_failure = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect_err("multiple mutable borrows should fail borrow checking");
    let messages = freeze_prepared_failure(
        prepared_failure,
        project.frontend.take_source_files(),
        &project.frontend.string_table,
    );

    let first_diagnostic = error_diagnostics(&messages)
        .next()
        .expect("borrow checking should return a diagnostic");
    assert!(
        !matches!(first_diagnostic.primary_site(), SiteRef::Absent),
        "borrow checker errors should preserve an authored source span"
    );
}

// -----------------------------------------------------------------------------
// Build-system style directive regression test
// -----------------------------------------------------------------------------

#[test]
fn html_style_directive_available_during_header_parsing() {
    // WHAT: project-owned style directives (like $html) must be visible during header-owned
    // parsing paths — specifically constant header expression parsing and template parsing.
    // This covers the docs-build failure mode where [$html: ...] templates in exported
    // constants could not be parsed because the directive registry was incomplete.
    let html_directive = StyleDirectiveSpec::handler(
        "html",
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::new(
            None,
            StyleDirectiveEffects {
                style_id: Some("html"),
                ..StyleDirectiveEffects::default()
            },
            None,
        ),
    );
    let directives = StyleDirectiveRegistry::merged(&[html_directive])
        .expect("merged directive registry should build");

    let mut project = FrontendProject::new(
        &[("src/@page.moth", "head #= [$html: <div>Hello</div>]\n")],
        "src/@page.moth",
        directives,
    );

    let ast = project.ast();

    let head_id = ast
        .const_values
        .iter_module_constant_views()
        .next()
        .expect("head constant should exist")
        .id;
    // [$html: <div>Hello</div>] has no runtime slots → folds to a string.
    assert!(
        ast.const_values.string_value(head_id).is_some(),
        "head should fold to a string when $html directive is available"
    );
}

// ---------------------------------
//  Static Bool control-flow handoff
// ---------------------------------
//
// Stage 4 owns static Bool `if` specialisation, so the AST that reaches HIR must contain a
// branch only where the condition is a genuine runtime test. The runtime case below records the
// shape that survives specialisation, while the static cases protect the Stage 4 handoff.

/// Count the `if` diamonds a lowered module actually contains.
fn branch_terminator_count(hir: &HirModule) -> usize {
    hir.blocks
        .iter()
        .filter(|block| matches!(block.terminator, HirTerminator::If { .. }))
        .count()
}

#[test]
fn runtime_bool_condition_lowers_one_branch_diamond() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "threshold ~= 4\nresult ~= 0\nif threshold > 1:\n    result = 1\nelse\n    result = 2\n;\nio.line([: [result]])\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let hir = project.hir();

    assert_eq!(
        branch_terminator_count(&hir),
        1,
        "a runtime Bool condition must keep its ordinary branch/merge shape"
    );
}

#[test]
fn compile_time_true_condition_reaches_hir_without_a_branch() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "enabled #= true\nresult ~= 0\nif enabled:\n    result = 1\nelse\n    result = 2\n;\nio.line([: [result]])\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let hir = project.hir();

    assert_eq!(
        branch_terminator_count(&hir),
        0,
        "a compile-time `true` condition must select the `then` branch before HIR"
    );
}

#[test]
fn compile_time_false_condition_without_else_lowers_no_branch_body() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "disabled #= false\nresult ~= 0\nif disabled:\n    result = 1\n;\nio.line([: [result]])\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let hir = project.hir();

    assert_eq!(
        branch_terminator_count(&hir),
        0,
        "a compile-time `false` condition with no `else` must produce an empty lexical-scope result"
    );
}

#[test]
fn terminality_observes_the_selected_branch() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "enabled #= true\nchoose || -> I32:\n    if enabled:\n        return 1\n    ;\n;\nio.line([: [choose()]])\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    // Terminality runs over the specialised active AST, so a function whose only active branch
    // returns is provably terminal and must not be rejected as a partial return.
    let hir = project.hir();

    assert_eq!(
        branch_terminator_count(&hir),
        0,
        "a compile-time `true` branch return must be terminal without a runtime branch"
    );
}

#[test]
fn selected_terminating_value_if_body_replaces_its_receiver_before_hir() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "enabled #= true\ndisabled #= false\nchoose_then || -> I32:\n    return if enabled:\n        return 1\n    else\n        then 2\n    ;\n;\nchoose_else || -> I32:\n    return if disabled:\n        then 1\n    else\n        return 2\n    ;\n;\nio.line([: [choose_then()]-[choose_else()]])\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let hir = project.hir();

    assert_eq!(
        branch_terminator_count(&hir),
        0,
        "terminating selected value bodies must not leave a branch or ownerless merge"
    );
}

#[test]
fn selected_statement_branch_keeps_its_immediate_scope_before_nested_control_flow() {
    let mut project = FrontendProject::new(
        &[(
            "src/@page.moth",
            "enabled #= false\nruntime ~= true\nif enabled:\n    then_value = 1\nelse\n    if runtime:\n        nested_value = 2\n    ;\n;\n",
        )],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let ast = project.ast();
    let body = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(_, _, body) => Some(body),
            _ => None,
        })
        .expect("entry function should exist");
    let selected = &body[1];
    let NodeKind::LexicalScope {
        body: selected_body,
    } = &selected.kind
    else {
        panic!("static outer if should become one lexical scope");
    };
    let nested = selected_body
        .first()
        .expect("selected else body should retain its nested runtime if");

    assert_ne!(
        selected.scope, nested.scope,
        "the selected scope must be the immediate else scope, not the nested if's child scope"
    );
}

// These tests protect failure facts crossing the AST-to-HIR boundary. Expression-local
// numeric producers lower to explicit recovery CFG; nested typed and inferred calls remain gated.

fn failure_project(source: &str) -> FrontendProject {
    FrontendProject::new(
        &[("src/@page.moth", source)],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    )
}

#[test]
fn synthetic_start_keeps_ast_builtin_error_slot_before_hir_narrowing() {
    for source in [
        "value = \"plain\"\n[:[value]]\n",
        "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
         value = load()!\n[:[value]]\n",
        "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
         value = load() catch then \"recovered\"\n[:[value]]\n",
        "left ~= 6\nright ~= 7\nvalue = left * right\n[:[value]]\n",
        "left ~= 6\nright ~= 7\nvalue = left * right catch then 0\n[:[value]]\n",
    ] {
        let mut project = failure_project(source);
        let ast = project.ast();
        let error_type = builtin_error_type_id(&project, &ast);
        let signature =
            ast.nodes
                .iter()
                .find_map(|node| match &node.kind {
                    NodeKind::Function(path, signature, _)
                        if project.frontend.path_fork.component(*path).is_some_and(
                            |component| project.frontend.string_table.resolve(component) == "start",
                        ) =>
                    {
                        Some(signature)
                    }
                    _ => None,
                })
                .expect("executable root must synthesize start");

        assert_eq!(signature.error_return_type_id(), Some(error_type));
        assert_eq!(signature.success_return_type_ids().len(), 1);
    }
}

fn named_function_body<'a>(
    project: &FrontendProject,
    ast: &'a Ast,
    name: &str,
) -> &'a [crate::compiler_frontend::ast::ast_nodes::AstNode] {
    ast.nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, _, body)
                if project
                    .frontend
                    .path_fork
                    .component(*path)
                    .is_some_and(|component| {
                        project.frontend.string_table.resolve(component) == name
                    }) =>
            {
                Some(body.as_slice())
            }
            _ => None,
        })
        .expect("authored function should exist")
}

fn expression_view(ast: &Ast, id: ExprId) -> ExpressionView<'_> {
    ast.expressions
        .view(id)
        .expect("fixture expression must exist in the AST expression store")
}

fn failure_facts_of(expression: ExpressionView<'_>) -> ExpressionFailureFacts {
    expression.failure_facts().cloned().unwrap_or_default()
}

fn handled_value<'a>(
    expression: ExpressionView<'a>,
    block: &ValueCatchBlock,
) -> ExpressionView<'a> {
    expression
        .bind(block.handled_value)
        .expect("catch handled value must exist in the expression store")
}

fn initializer<'a>(
    ast: &'a Ast,
    node: &crate::compiler_frontend::ast::ast_nodes::AstNode,
) -> ExpressionView<'a> {
    let NodeKind::VariableDeclaration(declaration) = &node.kind else {
        panic!("expected declaration, got {:?}", node.kind);
    };
    expression_view(
        ast,
        declaration
            .value
            .initializer()
            .expect("fixture initializer"),
    )
}

fn returned_value<'a>(
    ast: &'a Ast,
    node: &crate::compiler_frontend::ast::ast_nodes::AstNode,
) -> ExpressionView<'a> {
    let NodeKind::Return(values) = &node.kind else {
        panic!("expected success return, got {:?}", node.kind);
    };
    assert_eq!(values.len(), 1);
    expression_view(ast, values[0])
}

fn catch_block<'a>(expression: ExpressionView<'a>) -> &'a ValueCatchBlock {
    let ExpressionOperationView::ValueBlock(block) = expression.operation() else {
        panic!("catch should remain an expression-owned value block");
    };
    let ValueBlock::Catch(block) = block else {
        panic!("expected catch value block");
    };
    block
}

fn builtin_error_type_id(project: &FrontendProject, ast: &Ast) -> TypeId {
    let path =
        ast.nodes
            .iter()
            .find_map(|node| match &node.kind {
                NodeKind::StructDefinition(path, _)
                    if node.span.is_none()
                        && project.frontend.path_fork.component(*path).is_some_and(
                            |component| project.frontend.string_table.resolve(component) == "Error",
                        ) =>
                {
                    Some(*path)
                }
                _ => None,
            })
            .expect("frontend must register its canonical built-in Error definition");
    let nominal_id = ast
        .type_environment
        .nominal_id_for_path(&path)
        .expect("builtin Error nominal identity");
    ast.type_environment
        .type_id_for_nominal_id(nominal_id)
        .expect("builtin Error canonical type identity")
}

#[track_caller]
fn assert_catch_contract<'a>(
    expression: ExpressionView<'a>,
    has_implicit: bool,
    typed_error_id: Option<TypeId>,
    error_type_id: TypeId,
) -> &'a ValueCatchBlock {
    let block = catch_block(expression);
    let handled = handled_value(expression, block);
    let facts = failure_facts_of(handled);
    let summary = &facts.summary;
    assert_eq!(summary.first_implicit.is_some(), has_implicit);
    assert_eq!(
        summary.first_typed.map(|producer| producer.error_type_id),
        typed_error_id
    );
    assert!(
        summary.conflicting_typed.is_none(),
        "protected typed producers must be homogeneous"
    );
    assert_eq!(
        facts.disposition,
        FailureDisposition::HandledByCatch { error_type_id }
    );
    for witness in [
        summary.first_implicit,
        summary.first_numeric,
        summary.first_private_call,
    ]
    .into_iter()
    .flatten()
    {
        assert!(
            witness.span.is_some(),
            "compatibility witnesses must retain authored spans"
        );
    }
    assert_eq!(
        summary.first_implicit.is_some(),
        summary.first_numeric.is_some() || summary.first_private_call.is_some(),
    );
    if let Some(first) = summary.first_implicit {
        assert!(
            summary.first_numeric == Some(first) || summary.first_private_call == Some(first),
            "the first implicit witness must come from a numeric operation or private call",
        );
    }
    if let Some(producer) = summary.first_typed {
        assert!(producer.span.is_some());
        assert_eq!(producer.error_type_id, error_type_id);
    }
    assert!(
        has_implicit || typed_error_id.is_some() || facts.checked_numeric_operation,
        "catch must protect pending work or retain folded numeric eligibility",
    );
    assert!(
        !facts.has_pending_implicit(),
        "protected origins must not escape their catch"
    );
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("catch must retain its handler");
    };
    let first = body
        .first()
        .expect("catch handler must retain its recovery or terminal body");
    assert!(
        body.iter().all(|node| node.scope == first.scope),
        "handler nodes must share their lexical scope"
    );
    assert_eq!(expression.success_shape(), handled.success_shape());
    block
}

/// Error diagnostics retained by a diagnosed compilation result.
fn error_diagnostics<T>(result: &CompilationResult<T>) -> impl Iterator<Item = DiagnosticView<'_>> {
    diagnosed_reports(result)
        .iter_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error)
        .map(|(_, diagnostic)| diagnostic)
}

#[track_caller]
fn assert_fallible_reason(
    messages: &CompilationResult<()>,
    expected: InvalidFallibleHandlingReason,
) {
    let diagnostics = error_diagnostics(messages).collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "unexpected diagnostics: {messages:?}");
    let facts = diagnostics[0]
        .family_view::<InvalidFallibleHandlingView>()
        .expect("fallible diagnostic should retain its reason facts");
    assert_eq!(facts.reason.unit_reason(), Some(expected.clone()));
    assert!(
        !matches!(diagnostics[0].primary_site(), SiteRef::Absent),
        "fallible diagnostic should retain its primary span"
    );
    assert_eq!(diagnostics[0].descriptor().code, "MOTH-RULE-0051");
}

#[track_caller]
fn assert_catch_lowering_succeeds(project: &mut FrontendProject, ast: Ast) {
    let _module = project
        .lower_ast_result(ast)
        .expect("supported nested or inferred-failure producers must lower to HIR");
}

fn hir_block(hir: &HirModule, id: BlockId) -> &HirBlock {
    hir.blocks
        .iter()
        .find(|block| block.id == id)
        .expect("CFG block must exist")
}

fn loaded_local(hir: &HirModule, expression: &HirValueId) -> LocalId {
    let HirExpressionKind::Load(place) = &hir.expressions.expression(*expression).kind else {
        panic!("value must be loaded from its producer local");
    };
    assert!(
        place.projections.is_empty(),
        "value must be a direct local load"
    );
    place.root
}

fn numeric_producer(hir: &HirModule, operator: NumericOperator) -> (&HirBlock, LocalId) {
    let producers = hir
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                operation: HirFallibleOperation::NumericOp { op, .. },
                error_result,
                ..
            } if op.operator == operator => Some((block, *error_result)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        producers.len(),
        1,
        "expected exactly one {operator:?} producer"
    );
    producers[0]
}

fn fallible_edges(hir: &HirModule, block: &HirBlock, error_local: LocalId) -> (BlockId, BlockId) {
    let HirTerminator::Invoke {
        success,
        failure,
        success_results,
        error_result,
        ..
    } = &block.terminator
    else {
        panic!("recoverable producer must invoke before its values are consumed");
    };
    assert_eq!(*error_result, error_local);
    assert!(!success_results.contains(error_result));
    for statement in &block.statements {
        if let HirStatementKind::Write { value, .. } = &statement.kind
            && let HirExpressionKind::Load(place) = &hir.expressions.expression(*value).kind
        {
            assert_ne!(
                place.root, error_local,
                "failure value is defined only on the failure edge"
            );
            assert!(
                !success_results.contains(&place.root),
                "success values are defined only on the success edge"
            );
        }
    }
    (*success, *failure)
}

// Every producer transfers only its edge-local error into the common handler.
fn error_adapter(
    hir: &HirModule,
    error_block: BlockId,
    error_local: LocalId,
) -> (BlockId, LocalId) {
    let adapter = hir_block(hir, error_block);
    assert!(
        adapter.statements.is_empty(),
        "error adapter must not evaluate success values"
    );
    let HirTerminator::Jump { target, args } = &adapter.terminator else {
        panic!("error adapter must enter the common handler");
    };
    let [argument] = args.as_slice() else {
        panic!("error adapter must transfer exactly its error slot");
    };
    assert_eq!(argument.source, error_local);
    assert_ne!(*target, error_block);
    (*target, argument.destination)
}

#[test]
fn implicit_failure_private_chain_preserves_success_and_selects_error_materialisation() {
    let mut project = failure_project(
        "multiply |left I32, right I32| -> I32:\n    return left * right\n;\n\
         first |left I32, right I32| -> I32:\n    return multiply(left, right)\n;\n\
         second |left I32, right I32| -> I32:\n    return first(left, right)\n;\n\
         safe_product |left I32, right I32| -> I32, Error!:\n    return second(left, right)\n;\n",
    );
    let ast = project.ast();
    let product = returned_value(&ast, &named_function_body(&project, &ast, "multiply")[0]);
    assert_eq!(
        product.value_type_id().expect("multiply return has a type"),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert!(failure_facts_of(product).checked_numeric_operation);
    assert!(failure_facts_of(product).summary.first_numeric.is_some());

    for name in ["first", "second", "safe_product"] {
        let value = returned_value(&ast, &named_function_body(&project, &ast, name)[0]);
        assert_eq!(
            value
                .value_type_id()
                .expect("private call return has a type"),
            builtin_type_ids::fixed_scalar(FixedScalar::I32)
        );
        assert!(matches!(
            value.operation(),
            ExpressionOperationView::Call(_)
        ));
        assert!(failure_facts_of(value).summary.first_private_call.is_some());
        assert!(failure_facts_of(value).summary.first_typed.is_none());
    }

    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("private failure chain must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("ordinary numeric functions should pass borrow checking");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("private failure lane must install");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("installed failure edges must pass borrow checking");
    for (name, expected_escape) in [
        ("multiply", true),
        ("first", true),
        ("second", true),
        ("safe_product", false),
    ] {
        let function = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .and_then(|path| project.frontend.path_fork.component(path))
                    .is_some_and(|component| {
                        project.frontend.string_table.resolve(component) == name
                    })
            })
            .expect("authored function must lower");
        assert_eq!(
            report.analysis.public_call_summaries[&function.id].escapes_builtin_failure,
            expected_escape
        );
        assert_eq!(
            hir.function_failure_facts[&function.id].boundary,
            if expected_escape {
                HirBuiltinFailureBoundary::InferPrivate
            } else {
                HirBuiltinFailureBoundary::BuiltinErrorSlot
            },
        );
        let [contributor] = hir.function_failure_facts[&function.id]
            .contributors
            .as_slice()
        else {
            panic!("{name} must project its one escaping producer exactly once");
        };
        assert!(contributor.span.is_some());
        if name == "multiply" {
            assert!(matches!(
                contributor.source,
                HirBuiltinFailureSource::NumericOperation
            ));
            assert_eq!(contributor.codes, [BuiltinErrorCode::IntOverflow]);
        } else {
            assert!(matches!(
                contributor.source,
                HirBuiltinFailureSource::Call(CallTarget::Local(_))
            ));
        }
        assert_eq!(
            function.error_return,
            type_environment.type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                CanonicalBuiltinType::Error
            )),
            "{name} must expose builtin Error through its error channel"
        );
        assert!(
            function_reaches_error_return(&hir, function.entry),
            "{name} must return the existing Error on its own failure edge",
        );
    }
    assert!(
        hir.blocks.iter().any(|block| matches!(&block.terminator,
            HirTerminator::Invoke {
                operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::NumericOp { .. }, ..
            })),
        "the private multiply helper must return overflow instead of trapping"
    );
}

#[test]
fn implicit_failure_lanes_and_builtin_catch_preserve_float_integrity_guards() {
    let mut project = failure_project(
        "@test/default measure\n\
         plain |value F64| -> String:\n\
             label String = cast measure(value)\n    return label\n;\n\
         escaping |value F64, left I32, right I32| -> String:\n\
             label String = cast measure(value)\n\
             product = left * right\n    return [: [label] [product]]\n;\n\
         caught |value F64| -> String:\n\
             label String = cast measure(value) * value catch then \"fallback\"\n\
             return label\n;\n\
         declared |value F64| -> String, Error!:\n\
             label String = cast measure(value)\n    return label\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::F64)),
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::Fixed(FixedScalar::F64),
            ))],
            error_return_type: None,
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("float host signature must register");
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("float text conversion and external validation must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("float integrity guards must pass borrow checking");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("only implicit failures may join the private lane");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("integrity guards and installed numeric lanes must pass borrow checking");

    for (name, expected_invoke, escapes) in [
        ("plain", false, false),
        ("escaping", false, true),
        ("caught", false, false),
        ("declared", true, false),
    ] {
        let function = named_hir_function(&project, &hir, name);
        assert_eq!(
            report.analysis.public_call_summaries[&function.id].escapes_builtin_failure,
            escapes,
        );
        // External float integrity is independently guarded, even after numeric lane installation.
        let mut trapping_guards = 0;
        let mut invoking_guards = 0;
        for block_id in reachable_block_ids(&hir, function.entry) {
            let block = hir_block(&hir, block_id);
            for statement in &block.statements {
                if let HirStatementKind::ValidateFloat { failure_mode, .. } = &statement.kind {
                    assert_eq!(*failure_mode, NumericFailureMode::Trap);
                    trapping_guards += 1;
                }
            }
            if matches!(&block.terminator, HirTerminator::Invoke {
                operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::ValidateFloat { .. }, ..
            }) {
                invoking_guards += 1;
            }
        }
        assert_eq!(
            invoking_guards,
            usize::from(expected_invoke),
            "{name} finite validation channel"
        );
        assert_eq!(
            trapping_guards,
            usize::from(!expected_invoke),
            "{name} finite validation trap"
        );
    }
    let escaping = named_hir_function(&project, &hir, "escaping");
    assert!(
        function_numeric_ops_return_error(&hir, escaping.entry),
        "escaping arithmetic must retain its outward error edge"
    );
    let caught = named_hir_function(&project, &hir, "caught");
    let reachable = reachable_block_ids(&hir, caught.entry);
    let (producer, error_local) = reachable
        .iter()
        .find_map(|id| {
            let block = hir_block(&hir, *id);
            match &block.terminator {
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::NumericOp { op, .. },
                    error_result,
                    ..
                } if op.operator == NumericOperator::Multiply => Some((block, *error_result)),
                _ => None,
            }
        })
        .expect("caught arithmetic must retain its protected native numeric operation");
    let (_, failure) = fallible_edges(&hir, producer, error_local);
    let (handler, _) = error_adapter(&hir, failure, error_local);
    assert!(
        reachable_block_ids(&hir, caught.entry).contains(&handler),
        "caught arithmetic must still enter local recovery"
    );
}

#[test]
fn implicit_failure_compound_writeback_and_range_update_join_private_lane() {
    let mut project = failure_project(
        // The RHS is deliberately a wider U16 so `total += amount` stays a mixed-domain
        // write-back: only that genuinely wider RHS keeps the checked store conversion.
        "bump |amount U16| -> U8:\n\
             total ~U8 = 200\n\
             total += amount\n\
             return total\n;\n\
         safe_bump |amount U16| -> U8, Error!:\n\
             return bump(amount)\n;\n\
         walk |start I32, end I32| -> I32:\n\
             seen ~I32 = 0\n\
             loop start to & end |value|:\n\
                 seen = copy value\n\
             ;\n\
             return seen\n;\n\
         safe_walk |start I32, end I32| -> I32, Error!:\n\
             return walk(start, end)\n;\n",
    );
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("compound write-back and range updates must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("statement-owned checks must pass borrow checking");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("statement-owned checks must join the private failure lane");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("retargeted write-back and range edges must pass borrow checking");

    let bump = named_hir_function(&project, &hir, "bump");
    assert!(report.analysis.public_call_summaries[&bump.id].escapes_builtin_failure);
    assert!(
        store_conversion_returns_before_assign(&hir, bump.entry),
        "compound write-back must return the conversion error and assign only on success",
    );
    let walk = named_hir_function(&project, &hir, "walk");
    assert!(report.analysis.public_call_summaries[&walk.id].escapes_builtin_failure);
    assert!(
        function_numeric_ops_return_error(&hir, walk.entry),
        "required range updates must return through the private lane",
    );
    assert!(
        reachable_block_ids(&hir, walk.entry)
            .into_iter()
            .any(|block_id| {
                matches!(&hir_block(&hir, block_id).terminator,
                    HirTerminator::Invoke {
                        operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::RangeStepFailure {
                            cause: crate::compiler_frontend::hir::numeric::RangeStepFailureCause::ZeroStep,
                        }, ..
                    })
            }),
        "a zero step must return builtin Error through the private failure lane",
    );
}

#[test]
fn private_failure_lane_marks_edge_locals_as_temps_without_weakening_fresh_mutable_arguments() {
    let mut project = failure_project(
        "increment |value ~I32| -> I32:\n\
             value += 1\n\
             return value\n;\n\
         fresh |value I32| -> I32:\n\
             return increment(value + 1)\n;\n",
    );
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("fresh mutable arguments and private checked arithmetic must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let original_origins = hir
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .map(|local| (local.id, hir.side_table.local_origin(local.id)))
        .collect::<FxHashMap<_, _>>();
    assert!(
        original_origins.values().any(|origin| {
            origin.is_some_and(|origin| origin.kind == HirLocalOriginKind::CompilerFreshMutableArg)
        }),
        "the computed mutable argument must materialise an exclusive fresh root"
    );

    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("fresh mutable arguments must pass borrow checking before lane installation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("private checked arithmetic must install failure lanes");

    let mut introduced_errors = 0;
    for local in hir.blocks.iter().flat_map(|block| &block.locals) {
        let origin = hir.side_table.local_origin(local.id);
        if let Some(original_origin) = original_origins.get(&local.id) {
            assert_eq!(
                origin, *original_origin,
                "lane installation must preserve existing origins, including fresh mutable roots"
            );
            continue;
        }

        let origin = origin.expect("every lane-generated edge local must have a typed origin");
        assert_eq!(origin.kind, HirLocalOriginKind::CompilerTemp);
        assert_eq!(origin.call_span, None);
        assert_eq!(origin.argument_index, None);
        assert!(
            local.mutable,
            "lane destinations must remain writable scratch storage"
        );
        let error_destination = hir.blocks.iter().any(|block| {
            matches!(&block.terminator,
            HirTerminator::Invoke { error_result, .. } if *error_result == local.id)
        });
        let success_destination = hir.blocks.iter().any(|block| {
            matches!(&block.terminator,
            HirTerminator::Invoke { success_results, .. } if success_results.contains(&local.id))
        });
        assert!(
            error_destination || success_destination,
            "lane storage must be an edge destination"
        );
        assert!(
            !(error_destination && success_destination),
            "success and error destinations must be disjoint"
        );
        if error_destination {
            introduced_errors += 1;
            assert_eq!(
                Some(local.ty),
                type_environment.type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                    CanonicalBuiltinType::Error
                )),
                "lane-generated error storage must preserve the native Error type"
            );
        } else {
            assert_eq!(
                local.ty,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                "numeric update scratch must retain its success-slot type"
            );
        }
    }
    assert!(
        introduced_errors > 0,
        "checked arithmetic and private calls must introduce native error destinations"
    );
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("edge locals must not introduce exclusive aliases of fresh mutable roots");
}

#[test]
fn private_failure_lane_preserves_unrelated_fatal_fallible_edge() {
    let mut project = failure_project(
        // The wider U16 RHS keeps both compound stores genuine mixed-domain write-backs with
        // their own checked conversion traps.
        "bump |amount U16| -> U8:\n\
             total ~U8 = 200\n\
             total += amount\n\
             total += amount\n\
             return total\n;\n",
    );
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("compound stores must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let bump = named_hir_function(&project, &hir, "bump");
    let function_id = bump.id;
    let traps = reachable_block_ids(&hir, bump.entry)
        .into_iter()
        .filter(|block_id| {
            matches!(
                hir.blocks[block_id.0 as usize].terminator,
                HirTerminator::RuntimeFailure {
                    cause: Some(RuntimeFailureCause::StoreConversion { .. }),
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(traps.len(), 2);
    let fatal_block = traps[0];
    let conversion_block = traps[1];

    // Model a distinct fatal failure on a fallible edge. It has no write-back cause.
    hir.blocks[fatal_block.0 as usize].terminator = HirTerminator::RuntimeFailure {
        message: "Unrecoverable foreign fault".to_owned(),
        cause: None,
    };
    if let HirTerminator::RuntimeFailure { message, .. } =
        &mut hir.blocks[conversion_block.0 as usize].terminator
    {
        *message = "Rendered conversion message may change".to_owned();
    }
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("fatal fallible edge must be legal HIR");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    assert!(report.analysis.public_call_summaries[&function_id].escapes_builtin_failure);
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("an unrelated fatal fallible edge must not reject lane installation");

    assert!(
        matches!(
            hir.blocks[fatal_block.0 as usize].terminator,
            HirTerminator::RuntimeFailure { cause: None, .. }
        ),
        "a distinct fatal failure must not become recoverable"
    );
    let HirTerminator::ReturnError(error) = &hir.blocks[conversion_block.0 as usize].terminator
    else {
        panic!("the typed store conversion must return Error regardless of its rendered message");
    };
    let error = hir.expressions.expression(*error);
    assert!(matches!(error.kind, HirExpressionKind::Load(_)));
    let error_type = hir
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .expect("private function must remain installed")
        .error_return
        .expect("private failure lane must be fallible");
    assert_eq!(error.ty, error_type);
    assert_eq!(
        Some(error_type),
        type_environment.type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Error
        ),),
    );
}

fn named_hir_function<'a>(
    project: &FrontendProject,
    hir: &'a HirModule,
    name: &str,
) -> &'a crate::compiler_frontend::hir::functions::HirFunction {
    hir.functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .and_then(|path| project.frontend.path_fork.component(path))
                .is_some_and(|component| project.frontend.string_table.resolve(component) == name)
        })
        .expect("authored function must lower")
}

fn store_conversion_returns_before_assign(hir: &HirModule, entry: BlockId) -> bool {
    let mut found = false;
    for block_id in reachable_block_ids(hir, entry) {
        let block = &hir.blocks[block_id.0 as usize];
        let HirTerminator::Invoke {
            operation: HirFallibleOperation::CastOp { .. },
            success: success_block,
            failure: error_block,
            success_results,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let [success_local] = success_results.as_slice() else {
            return false;
        };
        if !matches!(
            hir.blocks[error_block.0 as usize].terminator,
            HirTerminator::ReturnError(_)
        ) {
            return false;
        }
        if hir.blocks[error_block.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, HirStatementKind::Write { .. }))
        {
            return false;
        }
        let assigned_on_success =
            hir.blocks[success_block.0 as usize]
                .statements
                .iter()
                .any(|statement| {
                    matches!(
                        &statement.kind,
                        HirStatementKind::Write { value, .. }
                            if matches!(
                                &hir.expressions.expression(*value).kind,
                                HirExpressionKind::Load(place)
                                    if place.root == *success_local
                            )
                    )
                });
        if !assigned_on_success {
            return false;
        }
        found = true;
    }
    found
}

fn function_numeric_ops_return_error(hir: &HirModule, entry: BlockId) -> bool {
    let mut found = false;
    for block_id in reachable_block_ids(hir, entry) {
        let block = hir_block(hir, block_id);
        for statement in &block.statements {
            if let HirStatementKind::NumericOp { failure_mode, .. } = &statement.kind
                && *failure_mode == NumericFailureMode::Trap
            {
                return false;
            }
        }
        if let HirTerminator::Invoke {
            operation: HirFallibleOperation::NumericOp { .. },
            failure,
            ..
        } = &block.terminator
        {
            if !function_reaches_error_return(hir, *failure) {
                return false;
            }
            found = true;
        }
    }
    found
}

fn reachable_block_ids(hir: &HirModule, entry: BlockId) -> Vec<BlockId> {
    let mut pending = vec![entry];
    let mut seen = Vec::new();
    while let Some(block_id) = pending.pop() {
        if seen.contains(&block_id) {
            continue;
        }
        seen.push(block_id);
        let Some(block) = hir.blocks.get(block_id.0 as usize) else {
            continue;
        };
        pending.extend(crate::compiler_frontend::hir::utils::terminator_targets(
            &block.terminator,
        ));
    }
    seen
}
fn function_reaches_error_return(hir: &HirModule, entry: BlockId) -> bool {
    let mut pending = vec![entry];
    let mut seen = Vec::new();
    while let Some(block_id) = pending.pop() {
        if seen.contains(&block_id) {
            continue;
        }
        seen.push(block_id);
        let Some(block) = hir.blocks.get(block_id.0 as usize) else {
            continue;
        };
        if matches!(block.terminator, HirTerminator::ReturnError(_)) {
            return true;
        }
        pending.extend(crate::compiler_frontend::hir::utils::terminator_targets(
            &block.terminator,
        ));
    }
    false
}

#[test]
fn implicit_failure_custom_contract_requires_explicit_mapping_even_with_other_error_return() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         area |left I32, right I32, invalid Bool| -> I32, Failure!:\n\
             if invalid:\n        return! Failure(\"invalid\")\n    ;\n\
             return left * right\n;\n",
    );
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("custom return! must not discharge numeric failure");
    let boundary_diagnostic = single_ast_error_view(&messages);
    let facts = boundary_diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("custom contract should retain fallible facts");
    assert_eq!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    );
    assert_eq!(boundary_diagnostic.descriptor().code, "MOTH-RULE-0051");
    assert!(!matches!(
        boundary_diagnostic.primary_site(),
        SiteRef::Absent
    ));
    assert_eq!(
        frozen_builtin_failure_labels(&messages),
        vec![(BuiltinFailureLabelRole::OperationOrigin, true)]
    );
    assert_eq!(frozen_elided_call_hops(&messages), Some(0));

    let mut project = failure_project(
        "Failure = | message String |\n\
         area |left I32, right I32| -> I32, Failure!:\n\
             result = left * right catch |err|:\n\
                 return! Failure(err.message)\n    ;\n    return result\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&ast, &named_function_body(&project, &ast, "area")[0]);
    let block = assert_catch_contract(
        expression,
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    let FallibleHandling::Handler { error, body } = &block.handler else {
        panic!("expected local mapping handler");
    };
    assert!(error.is_some());
    assert!(matches!(body[0].kind, NodeKind::ReturnError(_)));
    assert!(
        failure_facts_of(expression)
            .summary
            .first_implicit
            .is_none()
    );
    project
        .lower_ast_result(ast)
        .expect("numeric catch mapping to a custom error must lower");
}

#[test]
fn implicit_failure_catch_covers_multiply_and_add_not_only_last_operand() {
    for protected in ["left * right + offset", "(left * right) + offset"] {
        let mut project = failure_project(&format!(
            "total |left I32, right I32, offset I32| -> I32:\n\
                 value = {protected} catch then 0\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&ast, &named_function_body(&project, &ast, "total")[0]);
        let block = assert_catch_contract(
            expression,
            true,
            None,
            builtin_error_type_id(&project, &ast),
        );
        let handled = handled_value(expression, block);
        assert!(matches!(
            handled.operation(),
            ExpressionOperationView::NumericBinary { .. }
        ));
        let handled_facts = failure_facts_of(handled);
        assert!(handled_facts.checked_numeric_operation);
        assert!(handled_facts.summary.first_numeric.is_some());
        assert!(handled_facts.summary.first_private_call.is_none());
        assert!(
            failure_facts_of(expression)
                .summary
                .first_implicit
                .is_none()
        );
        let hir = project
            .lower_ast_result(ast)
            .expect("the complete arithmetic catch must lower");
        let total = named_hir_function(&project, &hir, "total");
        assert!(
            hir.function_failure_facts[&total.id]
                .contributors
                .is_empty()
        );
        numeric_producer(&hir, NumericOperator::Multiply);
        numeric_producer(&hir, NumericOperator::Add);
    }
}

#[test]
fn implicit_failure_folded_catch_preserves_eligibility_without_escaping_failure() {
    for handler in ["catch then 0", "catch |err|:\n        then 0\n    ;"] {
        let mut project = failure_project(&format!(
            "three || -> I32:\n    value = 1 + 2 {handler}\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&ast, &named_function_body(&project, &ast, "three")[0]);
        let block = assert_catch_contract(
            expression,
            false,
            None,
            builtin_error_type_id(&project, &ast),
        );
        let handled = handled_value(expression, block);
        assert!(matches!(
            handled.operation(),
            ExpressionOperationView::Constant(id)
                if matches!(
                    ast.const_values.payload(id),
                    Some(ConstValuePayload::FixedScalar(value)) if *value == i32_value(3)
                )
        ));
        assert!(failure_facts_of(handled).checked_numeric_operation);
        assert!(
            failure_facts_of(expression)
                .summary
                .first_implicit
                .is_none()
        );

        let hir = project
            .lower_ast_result(ast)
            .expect("folding must preserve an authored checked catch");
        let three = named_hir_function(&project, &hir, "three");
        assert!(
            hir.function_failure_facts[&three.id]
                .contributors
                .is_empty()
        );
    }
}

#[test]
fn implicit_failure_folded_catch_still_type_checks_handler_values() {
    for handler in [
        "catch then \"wrong\"",
        "catch |err|:\n        then \"wrong\"\n    ;",
    ] {
        let mut project = failure_project(&format!(
            "three || -> I32:\n    value I32 = 1 + 2 {handler}\n    return value\n;\n"
        ));
        let failure = project
            .ast_result()
            .expect_ast_failure("an unreachable handler must still satisfy its receiving type");
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
        assert!(
            !matches!(diagnostic.primary_site(), SiteRef::Absent),
            "handler type mismatch should retain its primary source site"
        );
    }
}

#[test]
fn implicit_failure_hir_two_numeric_operations_sequence_success_and_share_one_handler() {
    let mut project = failure_project(
        "total |left I32, right I32, offset I32| -> I32:\n\
             value = left * right + offset catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type = builtin_error_type_id(&project, &ast);
    let hir = project
        .lower_ast_result(ast)
        .expect("two-operation numeric recovery must lower");
    let numeric_count = hir
        .blocks
        .iter()
        .filter(|block| matches!(&block.terminator, HirTerminator::Invoke {
            operation: crate::compiler_frontend::hir::terminators::HirFallibleOperation::NumericOp { .. }, ..
        }))
        .count();
    assert_eq!(
        numeric_count, 2,
        "each protected operation must execute exactly once"
    );

    let (multiply, multiply_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
    let (add, add_error_local) = numeric_producer(&hir, NumericOperator::Add);
    assert_ne!(multiply_error_local, add_error_local);
    let (multiply_success, multiply_error) = fallible_edges(&hir, multiply, multiply_error_local);
    let (add_success, add_error) = fallible_edges(&hir, add, add_error_local);
    assert_eq!(
        multiply_success, add.id,
        "addition must execute only after multiplication succeeds"
    );
    assert_ne!(
        multiply_error, add_error,
        "each error payload needs its own adapter"
    );

    let predecessors = hir
        .blocks
        .iter()
        .filter(|block| match &block.terminator {
            HirTerminator::Invoke {
                success, failure, ..
            } => *success == add.id || *failure == add.id,
            HirTerminator::Jump { target, .. } => *target == add.id,
            _ => false,
        })
        .map(|block| block.id)
        .collect::<Vec<_>>();
    assert_eq!(
        predecessors,
        [multiply.id],
        "no other edge may reach the second operation"
    );

    let (handler, error_local) = error_adapter(&hir, multiply_error, multiply_error_local);
    assert_eq!(
        error_adapter(&hir, add_error, add_error_local),
        (handler, error_local)
    );
    assert_ne!(handler, multiply.id);
    assert_ne!(handler, add.id);
    assert_ne!(error_local, multiply_error_local);
    assert_ne!(error_local, add_error_local);

    let HirTerminator::Invoke {
        success_results: multiply_results,
        ..
    } = &multiply.terminator
    else {
        panic!("multiplication must define a native success destination");
    };
    let HirTerminator::Invoke {
        operation:
            HirFallibleOperation::NumericOp {
                operands: HirNumericOperands::Binary { left, .. },
                ..
            },
        success_results: add_results,
        ..
    } = &add.terminator
    else {
        panic!("addition must consume the multiplication success value");
    };
    assert_eq!(multiply_results.len(), 1);
    assert_eq!(add_results.len(), 1);
    assert_eq!(loaded_local(&hir, left), multiply_results[0]);
    assert_eq!(
        hir.expressions.expression(*left).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );

    let success = hir_block(&hir, add_success);
    let HirTerminator::Jump {
        target: success_merge,
        args: success_args,
    } = &success.terminator
    else {
        panic!("successful evaluation must transfer its slots to the catch merge");
    };
    let [success_argument] = success_args.as_slice() else {
        panic!("catch success must transfer exactly one slot");
    };
    let success_local = success_argument.destination;
    assert!(
        success
            .statements
            .iter()
            .any(|statement| matches!(&statement.kind,
        HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
            if *local == success_argument.source && loaded_local(&hir, value) == add_results[0])),
        "merge transfer must stage the native addition success value exactly once"
    );
    assert_ne!(success_local, error_local);
    assert_ne!(success_local, multiply_error_local);
    assert_ne!(success_local, add_error_local);

    let handler_block = hir_block(&hir, handler);
    let HirTerminator::Jump {
        target: recovery_merge,
        args: recovery_args,
    } = &handler_block.terminator
    else {
        panic!("recovery must transfer its slots to the catch merge");
    };
    let [recovery_argument] = recovery_args.as_slice() else {
        panic!("catch recovery must transfer exactly one slot");
    };
    assert_eq!(success_merge, recovery_merge);
    assert_eq!(recovery_argument.destination, success_local);
    assert!(
        handler_block.statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
                if *local == recovery_argument.source && matches!(
                    &hir.expressions.expression(*value).kind,
                    HirExpressionKind::FixedScalar(value) if *value == i32_value(0)
                )
        )),
        "recovery must produce zero without reading a failed success value"
    );

    for (local, expected_type) in [
        (
            success_local,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        ),
        (error_local, error_type),
    ] {
        let declaration = hir
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|declaration| declaration.id == local)
            .expect("catch slots must be declared");
        assert_eq!(declaration.ty, expected_type);
    }
    for producer in [multiply, add] {
        let HirTerminator::Invoke {
            success_results,
            error_result,
            ..
        } = &producer.terminator
        else {
            panic!("numeric producer must have native result channels");
        };
        assert!(!success_results.contains(error_result));
        for (local, expected_type) in success_results
            .iter()
            .map(|local| (*local, builtin_type_ids::fixed_scalar(FixedScalar::I32)))
            .chain(std::iter::once((*error_result, error_type)))
        {
            let declaration = hir
                .blocks
                .iter()
                .flat_map(|block| &block.locals)
                .find(|declaration| declaration.id == local)
                .expect("edge destination must be declared");
            assert_eq!(declaration.ty, expected_type);
        }
    }
    for (adapter_id, source) in [
        (multiply_error, multiply_error_local),
        (add_error, add_error_local),
    ] {
        assert_eq!(
            error_adapter(&hir, adapter_id, source),
            (handler, error_local)
        );
    }
    for success_block in [add, success] {
        assert!(success_block.statements.iter().all(|statement| !matches!(
            &statement.kind,
            HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), .. } if *local == error_local
        )), "success edges must not initialise the Error slot");
    }
    assert!(
        handler_block.statements.iter().all(|statement| !matches!(
            &statement.kind,
            HirStatementKind::Write { value, .. }
                if matches!(&hir.expressions.expression(*value).kind, HirExpressionKind::Load(place)
                    if multiply_results.contains(&place.root) || add_results.contains(&place.root))
        )),
        "the handler must never consume a failed producer's success payload"
    );
}

#[test]
fn implicit_failure_nested_handler_catch_uses_its_own_error_continuation() {
    let mut project = failure_project(
        "recover |left I32, right I32| -> I32:\n\
             value = left * right catch:\n\
                 recovered = left // right catch then 7\n        then recovered\n\
             ;\n    return value\n;\n",
    );
    let hir = project.hir();
    let (multiply, multiply_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, multiply_error) = fallible_edges(&hir, multiply, multiply_error_local);
    let (outer_handler, outer_error_slot) =
        error_adapter(&hir, multiply_error, multiply_error_local);
    let (divide, divide_error_local) = numeric_producer(&hir, NumericOperator::IntegerDivide);
    assert_eq!(divide.id, outer_handler);
    let (_, divide_error) = fallible_edges(&hir, divide, divide_error_local);
    let (inner_handler, inner_error_slot) = error_adapter(&hir, divide_error, divide_error_local);
    assert_ne!(
        inner_handler, outer_handler,
        "nested handler failure must not restart outer recovery"
    );
    assert_ne!(
        inner_error_slot, outer_error_slot,
        "nested catches must not overwrite outer error slots"
    );
    assert!(
        hir_block(&hir, inner_handler)
            .statements
            .iter()
            .any(|statement| matches!(
                &statement.kind,
                HirStatementKind::Write { value, .. } if matches!(
                    &hir.expressions.expression(*value).kind,
                    HirExpressionKind::FixedScalar(value) if *value == i32_value(7)
                )
            ))
    );
}

#[test]
fn implicit_failure_catch_covers_private_operands_arguments_and_typed_call() {
    for protected in [
        "multiply(left, right) + offset",
        "load(multiply(left, right) + offset)",
        "load(left * right + offset)",
    ] {
        let mut project = failure_project(&format!(
            "multiply |left I32, right I32| -> I32:\n    return left * right\n;\n\
             load |value I32| -> I32, Error!:\n    return value\n;\n\
             total |left I32, right I32, offset I32| -> I32:\n\
                 value = {protected} catch then 0\n    return value\n;\n"
        ));
        let ast = project.ast();
        let error_type_id = builtin_error_type_id(&project, &ast);
        let expression = initializer(&ast, &named_function_body(&project, &ast, "total")[0]);
        let typed_error = protected.starts_with("load(").then_some(error_type_id);
        let block = assert_catch_contract(expression, true, typed_error, error_type_id);
        if protected.contains("multiply(") {
            assert!(
                failure_facts_of(handled_value(expression, block))
                    .summary
                    .first_private_call
                    .is_some()
            );
        }
        assert!(
            failure_facts_of(expression)
                .summary
                .first_implicit
                .is_none()
        );
        assert!(failure_facts_of(expression).summary.first_typed.is_none());
        if protected.contains("multiply(") {
            assert_catch_lowering_succeeds(&mut project, ast);
        } else {
            project
                .lower_ast_result(ast)
                .expect("typed calls with pending numeric arguments must lower");
        }
    }
}

#[test]
fn implicit_failure_catch_covers_receiver_evaluation_and_method_arguments() {
    let mut project = failure_project(
        "Box = | value I32 |\n\
         make |value I32| -> Box, Error!:\n    return Box(value)\n;\n\
         amount |this Box, extra I32| -> I32, Error!:\n    return extra\n;\n\
         total |left I32, right I32, offset I32| -> I32:\n\
             value = make(left * right).amount(offset * right) catch then 0\n\
             return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = initializer(&ast, &named_function_body(&project, &ast, "total")[0]);
    assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    assert!(
        failure_facts_of(expression)
            .summary
            .first_implicit
            .is_none()
    );
    assert!(failure_facts_of(expression).summary.first_typed.is_none());
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_catch_accepts_multiple_builtin_errors_without_erasing_code_zero() {
    let mut project = failure_project(
        "load |value I32| -> I32, Error!:\n\
             if value < 0:\n        return! Error(\"authored\", code = 0)\n    ;\n\
             return value\n;\n\
         total |left I32, right I32| -> I32:\n\
             value = load(left) + load(right) catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = initializer(&ast, &named_function_body(&project, &ast, "total")[0]);
    let block = assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    assert!(
        failure_facts_of(handled_value(expression, block))
            .summary
            .first_numeric
            .is_some()
    );

    let load_body = named_function_body(&project, &ast, "load");
    let NodeKind::If(_, error_body, _, _) = &load_body[0].kind else {
        panic!("authored error branch must survive");
    };
    let NodeKind::ReturnError(error) = &error_body[0].kind else {
        panic!("code-zero value must remain an ordinary typed error return");
    };
    let error = expression_view(&ast, *error);
    assert_eq!(
        error.value_type_id().expect("authored Error has a type"),
        error_type_id
    );
    let ExpressionOperationView::Struct { fields } = error.operation() else {
        panic!("authored Error identity and fields must be retained");
    };
    let code = fields
        .iter()
        .find(|field| {
            project
                .frontend
                .path_fork
                .component(field.name())
                .is_some_and(|component| project.frontend.string_table.resolve(component) == "code")
        })
        .expect("Error.code field");
    let code_value = code.value();
    assert!(matches!(
        code_value.operation(),
        ExpressionOperationView::Constant(id)
            if matches!(
                ast.const_values.payload(id),
                Some(ConstValuePayload::FixedScalar(value))
                    if Some(*value) == FixedScalarValue::unsigned(FixedScalar::U32, 0)
            )
    ));
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_catch_accepts_one_custom_identity_without_numeric_failure() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         check |value Bool| -> Bool, Failure!:\n    return value\n;\n\
         recover |left Bool, right Bool| -> Bool:\n\
             value = check(left) and check(right) catch |err|:\n\
                 saved Failure = err\n        then false\n    ;\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&ast, &named_function_body(&project, &ast, "recover")[0]);
    let block = catch_block(expression);
    let error_type_id = failure_facts_of(handled_value(expression, block))
        .summary
        .first_typed
        .expect("custom typed producer")
        .error_type_id;
    assert_ne!(error_type_id, builtin_error_type_id(&project, &ast));
    let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
    assert!(!failure_facts_of(handled_value(expression, block)).checked_numeric_operation);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("custom recovery handler");
    };
    assert_eq!(
        initializer(&ast, &body[0])
            .value_type_id()
            .expect("custom Error binding has a type"),
        error_type_id
    );
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_custom_catch_accepts_known_infallible_private_operand() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         identity |value Bool| -> Bool:\n    return value\n;\n\
         check |value Bool| -> Bool, Failure!:\n    return value\n;\n\
         recover |left Bool, right Bool| -> Bool:\n\
             value = check(right) and identity(left) catch |err|:\n\
                 saved Failure = err\n        then false\n    ;\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&ast, &named_function_body(&project, &ast, "recover")[0]);
    let block = catch_block(expression);
    let error_type_id = failure_facts_of(handled_value(expression, block))
        .summary
        .first_typed
        .expect("custom typed producer")
        .error_type_id;
    assert_ne!(error_type_id, builtin_error_type_id(&project, &ast));
    let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("custom recovery handler");
    };
    assert_eq!(
        initializer(&ast, &body[0])
            .value_type_id()
            .expect("custom Error binding has a type"),
        error_type_id
    );
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_custom_catch_projects_private_compatibility_checks_into_hir() {
    for protected in ["multiply(left, right)", "forward(left, right)"] {
        let mut project = failure_project(&format!(
            "Failure = | message String |\n\
             multiply |left I32, right I32| -> I32:\n    return left * right\n;\n\
             forward |left I32, right I32| -> I32:\n    return multiply(left, right)\n;\n\
             load |value I32| -> I32, Failure!:\n    return value\n;\n\
             recover |left I32, right I32| -> I32:\n\
                 return load({protected}) catch then 0\n;\n"
        ));
        let ast = project.ast();
        let expression = returned_value(&ast, &named_function_body(&project, &ast, "recover")[0]);
        let block = catch_block(expression);
        let facts = failure_facts_of(handled_value(expression, block));
        let check = facts
            .deferred_custom_catch
            .expect("private call compatibility must be retained for exact callee convergence");
        let error_type_id = facts
            .summary
            .first_typed
            .expect("custom typed producer")
            .error_type_id;
        assert_eq!(check.error_type_id, error_type_id);
        assert!(check.catch_span.is_some());
        assert!(check.typed_producer_span.is_some());
        let private_call_span = facts
            .summary
            .first_private_call
            .expect("deferred compatibility must retain the private call witness")
            .span;
        assert!(private_call_span.is_some());
        let catch_span = check.catch_span;
        let typed_producer_span = check.typed_producer_span;

        let hir = project
            .lower_ast_result(ast)
            .expect("unresolved private compatibility must not diagnose before convergence");
        let recover = named_hir_function(&project, &hir, "recover");
        assert!(
            hir.function_failure_facts[&recover.id]
                .contributors
                .is_empty(),
            "protected private calls are compatibility candidates, not escaping producers"
        );
        let checks = &hir.function_failure_facts[&recover.id].deferred_custom_catches;
        let [check] = checks.as_slice() else {
            panic!("HIR must preserve the deferred custom catch check");
        };
        assert_eq!(check.error_type_id, error_type_id);
        assert_eq!(check.catch_span, catch_span);
        assert_eq!(check.typed_producer_span, typed_producer_span);
        assert!(matches!(
            check.candidates.as_slice(),
            [candidate] if matches!(candidate.source, HirBuiltinFailureSource::Call(_))
                && candidate.span == private_call_span
        ));
    }
}

#[test]
fn implicit_failure_custom_catch_reports_handler_compatibility_before_protected_work() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         helper_a |value I32| -> I32:\n    return value + 1\n;\n\
         helper_b |value I32| -> I32:\n    return value + 1\n;\n\
         load |value I32| -> I32, Failure!:\n    return value\n;\n\
         recover |value I32| -> I32:\n\
             return load(helper_a(value)) catch:\n\
                 inner = load(helper_b(value)) catch then 0\n\
                 then inner\n    ;\n;\n",
    );
    let ast = project.ast();
    let outer_expression = returned_value(&ast, &named_function_body(&project, &ast, "recover")[0]);
    let outer = catch_block(outer_expression);
    let FallibleHandling::Handler { body, .. } = &outer.handler else {
        panic!("outer recovery must retain its handler");
    };
    let inner_expression = initializer(&ast, &body[0]);
    let inner = catch_block(inner_expression);
    let outer_handled = handled_value(outer_expression, outer);
    let inner_handled = handled_value(inner_expression, inner);
    let inner_check = failure_facts_of(inner_handled)
        .deferred_custom_catch
        .expect("inner custom catch must defer private compatibility");
    let outer_check = failure_facts_of(outer_handled)
        .deferred_custom_catch
        .expect("outer custom catch must defer private compatibility");
    let inner_private_span = failure_facts_of(inner_handled)
        .summary
        .first_private_call
        .expect("inner helper call must retain its authored span")
        .span;
    let outer_private_span = failure_facts_of(outer_handled)
        .summary
        .first_private_call
        .expect("outer helper call must retain its authored span")
        .span;
    assert!(inner_check.catch_span.is_some());
    assert!(inner_check.typed_producer_span.is_some());
    assert!(inner_private_span.is_some());
    assert_ne!(inner_check.catch_span, outer_check.catch_span);
    assert_ne!(
        inner_check.typed_producer_span,
        outer_check.typed_producer_span
    );
    assert_ne!(inner_private_span, outer_private_span);

    let hir = project
        .lower_ast_result(ast)
        .expect("nested custom catches must defer compatibility until private summaries converge");
    let recover = named_hir_function(&project, &hir, "recover");
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("nested recovery must pass borrow checking");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    let diagnostic =
        crate::compiler_frontend::module_compilation::generated::builtin_failure_diagnostic(
            &hir, &report, recover.id,
        )
        .expect("the failing inner helper must make its custom catch incompatible");
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let view = compact
        .first()
        .expect("generated catch compatibility draft should compact");
    let facts = view
        .family_view::<InvalidFallibleHandlingView>()
        .expect("generated catch compatibility should retain fallible facts");
    assert_eq!(view.descriptor().code, "MOTH-RULE-0051");
    assert_eq!(
        diagnostic_primary_span(view),
        inner_check.catch_span,
        "generated catch compatibility should retain its primary span"
    );
    assert_eq!(
        facts.reason,
        InvalidFallibleHandlingCode::CustomErrorMixedWithImplicitFailure
    );
    assert_eq!(facts.second_error_type, None);
    let expected_error_type = format!("TypeId({})", inner_check.error_type_id.0);
    assert_eq!(
        view.types().display_for_render(
            facts
                .first_error_type
                .expect("mixed catch retains its typed error type"),
            &project.frontend.string_table,
            None,
        ),
        expected_error_type
    );

    let mut typed_producer = None;
    let mut implicit_producer = None;
    for label in view.durable_labels() {
        let span = match label.site {
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
            SiteRef::Absent => panic!("generated catch producer labels should retain spans"),
        };
        match label.message {
            DurableLabelMessage::TypedFailureProducer { error_type } => {
                typed_producer = Some((error_type, span));
            }
            DurableLabelMessage::ImplicitFailureProducer => implicit_producer = Some(span),
            _ => {}
        }
    }
    let (typed_error_type, typed_producer_span) =
        typed_producer.expect("generated catch should retain the typed producer label");
    assert_eq!(
        view.types()
            .display_for_render(typed_error_type, &project.frontend.string_table, None,),
        expected_error_type
    );
    assert_eq!(Some(typed_producer_span), inner_check.typed_producer_span);
    assert_eq!(
        implicit_producer, inner_private_span,
        "generated catch should retain the implicit producer span"
    );
}

#[test]
fn implicit_failure_catch_rejects_custom_numeric_and_distinct_typed_errors() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         load |value I32| -> I32, Failure!:\n    return value\n;\n\
         recover |left I32, right I32| -> I32:\n\
             return load(left) * right catch then 0\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("custom plus implicit cannot share a handler");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.custom_error_mixed_with_implicit_failure"),
    );
    assert!(
        !diagnostic_label_spans(diagnostic).is_empty(),
        "custom/implicit failure should retain producer labels"
    );
    for second_type in ["Other", "Error"] {
        let mut project = failure_project(&format!(
            "Failure = | message String |\nOther = | message String |\n\
             first |value Bool| -> Bool, Failure!:\n    return value\n;\n\
             second |value Bool| -> Bool, {second_type}!:\n    return value\n;\n\
             recover |left Bool, right Bool| -> Bool:\n\
                 return first(left) and second(right) catch then false\n;\n"
        ));
        let failure = project
            .ast_result()
            .expect_ast_failure("distinct typed errors cannot share a handler");
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0051",
            Some("invalid_fallible_handling.incompatible_catch_error_types"),
        );
        assert_eq!(
            diagnostic_label_spans(diagnostic).len(),
            2,
            "each incompatible error producer should remain labeled"
        );
    }
}

#[test]
fn implicit_failure_cast_catch_retains_operand_and_conversion_contracts() {
    let mut project = failure_project(
        "narrow |left U32, right U32| -> U8:\n\
             value U8 = cast left + right catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = initializer(&ast, &named_function_body(&project, &ast, "narrow")[0]);
    // Builtin arithmetic and conversion contribute implicit failure. The handler binds Error
    // without manufacturing a declared typed producer, and protects both operations.
    let block = assert_catch_contract(expression, true, None, error_type_id);
    let handled = handled_value(expression, block);
    let ExpressionOperationView::Cast {
        source,
        facts: cast,
    } = handled.operation()
    else {
        panic!("cast must own the complete arithmetic operand");
    };
    let u32_type = builtin_type_ids::fixed_scalar(FixedScalar::U32);
    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    assert_eq!(
        source.value_type_id().expect("cast source has a type"),
        u32_type
    );
    assert_eq!(cast.source_type_id, u32_type);
    assert_eq!(cast.target_type_id, u8_type);
    assert_eq!(
        expression.value_type_id().expect("cast catch has a type"),
        u8_type
    );
    assert!(failure_facts_of(source).checked_numeric_operation);
    assert!(
        failure_facts_of(expression)
            .summary
            .first_implicit
            .is_none()
    );
    project
        .lower_ast_result(ast)
        .expect("cast catch must cover numeric operands and conversion");

    let mut project = failure_project(
        "widen |left I32, right I32| -> F64:\n\
             value F64 = cast left * right catch then 0.0\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&ast, &named_function_body(&project, &ast, "widen")[0]);
    let block = assert_catch_contract(
        expression,
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    let handled = handled_value(expression, block);
    let ExpressionOperationView::Cast { facts: cast, .. } = handled.operation() else {
        panic!("infallible conversion must retain its pending numeric operand");
    };
    assert_eq!(
        cast.source_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(
        cast.target_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("infallible cast must still recover operand failures");
    let (multiply, producer_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, error) = fallible_edges(&hir, multiply, producer_error_local);
    error_adapter(&hir, error, producer_error_local);

    let mut project =
        failure_project("bad |values {I32}| -> I32:\n    return cast values catch then 0\n;\n");
    let failure = project
        .ast_result()
        .expect_ast_failure("a handler must not manufacture cast evidence");
    let diagnostic =
        assert_ast_diagnostic(&failure, "MOTH-RULE-0083", Some("invalid_cast.no_evidence"));
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "missing cast evidence should retain its primary source site"
    );
}

#[test]
fn implicit_failure_bound_catch_has_error_fields_unbound_catch_has_no_binding() {
    let mut project = failure_project(
        "bound |left I32, right I32| -> I32:\n\
             value = left * right catch |err|:\n\
                 saved Error = err\n        code U32 = err.code\n        message String = err.message\n\
                 then 0\n    ;\n    return value\n;\n\
         unbound |left I32, right I32| -> I32:\n\
             value = left * right catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let bound = initializer(&ast, &named_function_body(&project, &ast, "bound")[0]);
    let block = assert_catch_contract(bound, true, None, error_type_id);
    let FallibleHandling::Handler { error, body } = &block.handler else {
        panic!("bound handler");
    };
    assert!(error.is_some());
    assert_eq!(
        initializer(&ast, &body[0])
            .value_type_id()
            .expect("handler binding has a type"),
        error_type_id
    );
    assert_eq!(
        initializer(&ast, &body[1])
            .value_type_id()
            .expect("handler binding has a type"),
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
    );
    assert_eq!(
        initializer(&ast, &body[2])
            .value_type_id()
            .expect("handler binding has a type"),
        builtin_type_ids::STRING
    );

    let unbound = initializer(&ast, &named_function_body(&project, &ast, "unbound")[0]);
    let block = assert_catch_contract(unbound, true, None, error_type_id);
    let FallibleHandling::Handler { error, body } = &block.handler else {
        panic!("unbound handler");
    };
    assert!(
        error.is_none(),
        "unbound implicit recovery must not invent an observable error local"
    );
    assert!(matches!(body.as_slice(), [node] if matches!(node.kind, NodeKind::ThenValue(_))));
    project
        .lower_ast_result(ast)
        .expect("bound and unbound arithmetic recovery must lower");
}

#[test]
fn implicit_failure_handler_terminality_and_arity_are_checked_before_hir() {
    for handler in [
        "return 0",
        "assert(false, \"cannot recover\")",
        "if choose:\n            then 0\n        else\n            return 1\n        ;",
    ] {
        let mut project = failure_project(&format!(
            "recover |left I32, right I32, choose Bool| -> I32:\n\
                 value = left * right catch:\n        {handler}\n    ;\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&ast, &named_function_body(&project, &ast, "recover")[0]);
        assert_catch_contract(
            expression,
            true,
            None,
            builtin_error_type_id(&project, &ast),
        );
        project
            .lower_ast_result(ast)
            .expect("producing and terminal arithmetic handlers must lower");
    }

    let mut project = failure_project(
        "recover |left I32, right I32| -> I32:\n\
             value = left * right catch:\n        io.line(\"not a fallback\")\n    ;\n\
             return value\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a success receiver needs then or termination");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
    );

    let mut project = failure_project(
        "recover |left I32, right I32| -> I32:\n\
             return left * right catch then 0, 1\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("numeric success has exactly one slot");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.too_many_return_values"),
    );

    let mut project = failure_project(
        "recover |left I32, right I32| -> I32:\n\
             value ~= 0\n    value = left * right catch then value\n    return value\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("the success target is unavailable in recovery");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.unavailable_in_catch_recovery"),
    );
}

#[test]
fn implicit_failure_multi_success_catch_preserves_slot_order_and_required_arity() {
    let mut project = failure_project(
        "pair |value I32| -> I32, String, Error!:\n    return value, \"ok\"\n;\n\
         recover |left I32, right I32| -> String:\n\
             number, label = pair(left * right) catch then 0, \"fallback\"\n    return label\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let NodeKind::MultiBind { targets, value } = &body[0].kind else {
        panic!("multi-success receiver must remain multi-bind");
    };
    let block = assert_catch_contract(
        expression_view(&ast, *value),
        true,
        Some(builtin_error_type_id(&project, &ast)),
        builtin_error_type_id(&project, &ast),
    );
    assert_eq!(
        block.result_type_ids,
        [
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::STRING
        ]
    );
    assert_eq!(
        targets
            .iter()
            .map(|target| target.type_id)
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("multi-success fallback");
    };
    let NodeKind::ThenValue(values) = &body[0].kind else {
        panic!("fallback must produce both slots");
    };
    assert_eq!(
        values
            .expressions
            .iter()
            .map(|value| expression_view(&ast, *value)
                .value_type_id()
                .expect("fallback slot has a type"))
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    project
        .lower_ast_result(ast)
        .expect("multi-success typed recovery must sequence numeric arguments");

    let mut project = failure_project(
        "pair |value I32| -> I32, String, Error!:\n    return value, \"ok\"\n;\n\
         recover |left I32, right I32| -> String:\n\
             number, label = pair(left * right) catch then 0\n    return label\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("multi-success fallback cannot leave a slot uninitialized");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0055",
        Some("invalid_return_shape.too_few_return_values"),
    );
}

#[test]
fn implicit_failure_private_multi_success_catch_preserves_slot_order() {
    let mut project = failure_project(
        "pair |left I32, right I32| -> I32, String:\n    return left * right, \"ok\"\n;\n\
         recover |left I32, right I32| -> String:\n\
             number, label = pair(left, right) catch then 0, \"fallback\"\n    return label\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let NodeKind::MultiBind { targets, value } = &body[0].kind else {
        panic!("private multi-success catch must remain multi-bind");
    };
    let block = assert_catch_contract(
        expression_view(&ast, *value),
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    assert_eq!(
        block.result_type_ids,
        [
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::STRING
        ]
    );
    assert_eq!(
        targets
            .iter()
            .map(|target| target.type_id)
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("private multi-success fallback");
    };
    let NodeKind::ThenValue(values) = &body[0].kind else {
        panic!("fallback must produce both slots");
    };
    assert_eq!(
        values
            .expressions
            .iter()
            .map(|value| expression_view(&ast, *value)
                .value_type_id()
                .expect("fallback slot has a type"))
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_in_handler_goes_outward_not_back_into_its_own_catch() {
    let mut project = failure_project(
        "recover |left I32, right I32| -> I32, Error!:\n\
             value = left * right catch:\n        then left // right\n    ;\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&ast, &named_function_body(&project, &ast, "recover")[0]);
    let block = assert_catch_contract(
        expression,
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    let handled = handled_value(expression, block);
    assert!(failure_facts_of(handled).summary.first_numeric.is_some());
    let handler_span = failure_facts_of(expression)
        .summary
        .first_numeric
        .expect("handler arithmetic must remain pending")
        .span;
    assert!(handler_span.is_some());
    assert_eq!(
        failure_facts_of(expression)
            .summary
            .first_implicit
            .unwrap()
            .span,
        handler_span
    );
    assert_eq!(
        failure_facts_of(expression).disposition,
        FailureDisposition::Pending
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("handler failure must use the enclosing Error! boundary");
    let recover = named_hir_function(&project, &hir, "recover");
    let [contributor] = hir.function_failure_facts[&recover.id]
        .contributors
        .as_slice()
    else {
        panic!("only handler arithmetic must escape, exactly once");
    };
    assert!(matches!(
        contributor.source,
        HirBuiltinFailureSource::NumericOperation
    ));
    assert_eq!(contributor.span, handler_span);
    assert!(contributor.codes.contains(&BuiltinErrorCode::DivideByZero));
    let (multiply_block, multiply_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, multiply_error) = fallible_edges(&hir, multiply_block, multiply_error_local);
    let (handler, _) = error_adapter(&hir, multiply_error, multiply_error_local);
    let (divide_block, divide_error_local) = numeric_producer(&hir, NumericOperator::IntegerDivide);
    assert_eq!(
        divide_block.id, handler,
        "handler arithmetic must execute inside the handler"
    );
    let (_, divide_error) = fallible_edges(&hir, divide_block, divide_error_local);
    let error_block = hir_block(&hir, divide_error);
    let HirTerminator::ReturnError(error) = &error_block.terminator else {
        panic!("handler failure must return outward, not re-enter its own handler");
    };
    assert!(matches!(
        &hir.expressions.expression(*error).kind,
        HirExpressionKind::Load(place) if place.root == divide_error_local
    ));

    let mut project = failure_project(
        "Failure = | message String |\n\
         load |value I32| -> I32, Error!:\n    return value\n;\n\
         recover |left I32, right I32| -> I32, Failure!:\n\
             value = load(left) catch:\n        then left // right\n    ;\n    return value\n;\n",
    );
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("handler failure still violates a custom contract");
    let diagnostic = error_diagnostics(&messages)
        .next()
        .expect("outward failure diagnosis");
    let facts = diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("outward failure diagnosis should retain fallible facts");
    assert!(matches!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    ));
    assert!(diagnostic.durable_labels().any(|label| {
        matches!(
            label.message,
            DurableLabelMessage::BuiltinFailureOrigin
                | DurableLabelMessage::BuiltinWriteBackOrigin { .. }
        ) && !matches!(label.site, SiteRef::Absent)
    }));
}

#[test]
fn implicit_failure_catch_keeps_comma_and_logical_line_boundaries() {
    let mut project = failure_project(
        "recover |left I32, right I32| -> I32, I32:\n\
             earlier = left * right\n\
             later = left + right catch then 0\n\
             return left * right, left + right catch:\n        then 0\n    ;\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let error_type_id = builtin_error_type_id(&project, &ast);
    let earlier = initializer(&ast, &body[0]);
    assert_eq!(
        failure_facts_of(earlier).disposition,
        FailureDisposition::Pending
    );
    assert!(failure_facts_of(earlier).summary.first_numeric.is_some());
    let later = initializer(&ast, &body[1]);
    assert_catch_contract(later, true, None, error_type_id);
    assert!(failure_facts_of(later).summary.first_implicit.is_none());
    let NodeKind::Return(values) = &body[2].kind else {
        panic!("expected two independent success slots");
    };
    assert_eq!(values.len(), 2);
    let first = expression_view(&ast, values[0]);
    let second = expression_view(&ast, values[1]);
    assert_eq!(
        failure_facts_of(first).disposition,
        FailureDisposition::Pending
    );
    assert!(failure_facts_of(first).summary.first_numeric.is_some());
    assert_catch_contract(second, true, None, error_type_id);
    assert!(failure_facts_of(second).summary.first_implicit.is_none());
    project
        .lower_ast_result(ast)
        .expect("independently delimited numeric catches must lower");
}

#[test]
fn implicit_failure_catch_rejects_inner_argument_group_condition_template_and_constant() {
    let sources = [
        "consume |value I32| -> I32:\n    return value\n;\n\
         bad |left I32, right I32| -> I32:\n\
             return consume(left * right catch then 0)\n;\n",
        "bad |left I32, right I32| -> I32:\n\
             return (left * right catch then 0) + right\n;\n",
        "bad |left I32, right I32| -> I32:\n\
             if left * right > 0 catch then false:\n        return 1\n    else\n        return 0\n    ;\n;\n",
        "bad |left I32, right I32| -> String:\n\
             return [:value=[left * right catch then 0]]\n;\n",
        "value #= 1 * 2 catch then 0\n",
    ];
    for source in sources {
        let mut project = failure_project(source);
        let failure = project
            .ast_result()
            .expect_ast_failure("catch remains a closed-receiver surface");
        assert_ast_fallible_reason(
            &failure,
            InvalidFallibleHandlingReason::CatchOutsideBoundary,
        );
    }

    let mut project = failure_project(
        "bad |left I32, right I32| -> I32:\n\
             return left * right catch then\n        0\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("inline catch cannot cross a logical line");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::InlineCatchMultiline,
    );
}

#[test]
fn implicit_failure_catch_remains_ineligible_for_literal_or_infallible_call() {
    for prefix in ["", "export:\n"] {
        let closing_export = if prefix.is_empty() { "" } else { ";\n" };
        for expression in ["3", "identity(value)"] {
            let mut project = failure_project(&format!(
                "{prefix}    identity |value I32| -> I32:\n        return value\n    ;\n{closing_export}\
                 bad |value I32| -> I32:\n    return {expression} catch then 0\n;\n"
            ));
            let failure = project
                .ast_result()
                .expect_ast_failure("ordinary infallible values do not make catch eligible");
            assert_ast_fallible_reason(&failure, InvalidFallibleHandlingReason::CatchOnNonFallible);
        }
    }
}

#[test]
fn implicit_failure_catch_preserves_propagation_and_optional_conflicts() {
    let mut project = failure_project(
        "load |value I32| -> I32, Error!:\n    return value\n;\n\
         bad |value I32| -> I32, Error!:\n    return load(value)! catch then 0\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("propagation and catch conflict on the same expression");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict,
    );

    let mut project = failure_project(
        "maybe |value I32| -> I32?:\n    return value\n;\n\
         bad |value I32| -> I32?:\n    return maybe(value)? catch then 0\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("option propagation must not target catch");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::OptionPropagationCatchConflict,
    );

    for (producer, suffix, result_type, expected_reason) in [
        (
            "load |value U32| -> U32, Error!:\n    return value\n;\n",
            "!",
            "U8, Error!",
            InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict,
        ),
        (
            "load |value U32| -> U8?:\n    return 1\n;\n",
            "?",
            "U8?",
            InvalidFallibleHandlingReason::OptionPropagationCatchConflict,
        ),
    ] {
        let mut project = failure_project(&format!(
            "{producer}bad |left U32, right U32| -> {result_type}:\n\
                 return cast load(left){suffix} + right catch then 0\n;\n"
        ));
        let failure = project
            .ast_result()
            .expect_ast_failure("cast operand propagation must diagnose before debug finalization");
        assert_ast_fallible_reason(&failure, expected_reason);
    }
}

#[test]
fn implicit_failure_compound_rhs_catch_does_not_consume_arithmetic_or_narrowing() {
    let mut project = failure_project(
        // The wider U16 RHS keeps the compound store a genuinely mixed-domain write-back; the
        // fallback literal materialises at the callee's U16 success slot.
        "load |amount U8| -> U16, Error!:\n    return cast amount\n;\n\
         update |amount U8| -> U8:\n\
             value ~U8 = 200\n    value += load(amount) catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "update");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    let value = expression_view(&ast, *value);
    let ExpressionOperationView::Cast {
        source,
        facts: conversion,
    } = value.operation()
    else {
        panic!("narrow write-back requires a distinct checked conversion");
    };
    assert!(matches!(conversion.handling, CastHandling::StoreConversion));
    assert_eq!(
        conversion.target_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_eq!(
        conversion.source_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U16)
    );
    let value_facts = failure_facts_of(value);
    assert_eq!(value_facts.disposition, FailureDisposition::Pending);
    assert!(value_facts.summary.first_typed.is_none());
    assert!(value_facts.summary.first_numeric.is_some());
    assert!(value_facts.checked_numeric_operation);
    let source_facts = failure_facts_of(source);
    assert!(source_facts.checked_numeric_operation);
    assert_eq!(source_facts.disposition, FailureDisposition::Pending);
    let ExpressionOperationView::NumericBinary { left, right, .. } = source.operation() else {
        panic!("promoted arithmetic must be separate from protected RHS");
    };
    let rhs = [left, right]
        .into_iter()
        .find(|operand| matches!(operand.operation(), ExpressionOperationView::ValueBlock(_)))
        .expect("compound arithmetic must retain one RHS catch operand");
    let error_type_id = builtin_error_type_id(&project, &ast);
    assert_catch_contract(rhs, false, Some(error_type_id), error_type_id);
    assert!(failure_facts_of(rhs).summary.first_implicit.is_none());
    let hir = project
        .lower_ast_result(ast)
        .expect("existing typed RHS catch and numeric write-back must still lower");
    let update = named_hir_function(&project, &hir, "update");
    let write_back = hir.function_failure_facts[&update.id]
        .contributors
        .iter()
        .find_map(|producer| match &producer.source {
            HirBuiltinFailureSource::CompoundWriteBack { target, .. } => Some((producer, *target)),
            _ => None,
        })
        .expect("the numeric write-back must reach the function failure facts");
    assert_eq!(
        write_back.1,
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "the write-back contributor names the canonical compound target"
    );
    assert_eq!(
        write_back.0.codes,
        [BuiltinErrorCode::IntCastOutOfRange],
        "the narrow back into the target owns the cast range code"
    );
}

#[test]
fn implicit_failure_error_alias_is_canonical_but_lookalike_is_not() {
    let mut project = failure_project(
        "Failure as Error\n\
         safe_product |left I32, right I32| -> I32, Failure!:\n    return left * right\n;\n\
         recover |left I32, right I32| -> I32:\n\
             value = safe_product(left, right) + left catch |err|:\n\
                 saved Error = err\n        then 0\n    ;\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let signature = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _)
                if project
                    .frontend
                    .path_fork
                    .component(*path)
                    .is_some_and(|component| {
                        project.frontend.string_table.resolve(component) == "safe_product"
                    }) =>
            {
                Some(signature)
            }
            _ => None,
        })
        .expect("Error alias function");
    assert_eq!(
        signature.returns.last().expect("error slot").type_id,
        Some(error_type_id)
    );
    let expression = initializer(&ast, &named_function_body(&project, &ast, "recover")[0]);
    let block = assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("canonical Error handler");
    };
    assert_eq!(
        initializer(&ast, &body[0])
            .value_type_id()
            .expect("handler fallback has one success value"),
        error_type_id
    );
    assert_catch_lowering_succeeds(&mut project, ast);

    let mut project = failure_project(
        "Failure = | message String, code U32 = 0 |\n\
         safe_product |left I32, right I32| -> I32, Failure!:\n    return left * right\n;\n",
    );
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("same-shaped structs do not become builtin Error");
    let diagnostic = error_diagnostics(&messages)
        .next()
        .expect("nominal lookalike rejection");
    let facts = diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("nominal lookalike rejection should retain fallible facts");
    assert!(matches!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    ));
}

#[test]
fn implicit_failure_error_wrapper_struct_is_not_canonical() {
    // A struct that merely wraps builtin Error keeps its nominal identity: it must not
    // receive builtin treatment in the error slot, even though its only field is an Error.
    let mut project = failure_project(
        "Wrapped = | inner Error |\n\
         safe_product |left I32, right I32| -> I32, Wrapped!:\n    return left * right\n;\n",
    );
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("a struct wrapping Error is not builtin Error");
    assert_eq!(error_diagnostics(&messages).count(), 1);
    let boundary_diagnostic = single_ast_error_view(&messages);
    let facts = boundary_diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("wrapper slot rejection should retain fallible facts");
    assert_eq!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    );
    assert_eq!(boundary_diagnostic.descriptor().code, "MOTH-RULE-0051");
    assert!(!matches!(
        boundary_diagnostic.primary_site(),
        SiteRef::Absent
    ));
    assert_eq!(
        frozen_builtin_failure_labels(&messages),
        vec![(BuiltinFailureLabelRole::OperationOrigin, true)]
    );
    assert_eq!(frozen_elided_call_hops(&messages), Some(0));

    // The same wrapper must not unlock builtin mixing under catch either: a Wrapped-typed
    // producer alongside implicit failure stays a custom/implicit mix.
    let mut project = failure_project(
        "Wrapped = | inner Error |\n\
         load |value I32| -> I32, Wrapped!:\n    return value\n;\n\
         recover |left I32, right I32| -> I32:\n\
             return load(left) * right catch |err|:\n\
                 saved Wrapped = err\n        then 0\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("wrapper plus implicit cannot share a handler");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.custom_error_mixed_with_implicit_failure"),
    );
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "wrapper catch diagnosis should retain its primary source site"
    );
}

#[test]
fn implicit_failure_static_invalid_arithmetic_is_not_made_valid_by_catch_or_error_slot() {
    for (expression, expected_reason) in [
        ("1 // 0", CompileTimeEvaluationErrorReason::DivideByZero),
        ("1 % 0", CompileTimeEvaluationErrorReason::DivideByZero),
        (
            "2147483647 + 1",
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        ("2 ^ -1", CompileTimeEvaluationErrorReason::InvalidExponent),
    ] {
        for suffix in ["", " catch then 0"] {
            let mut project = failure_project(&format!(
                "bad || -> I32, Error!:\n    return {expression}{suffix}\n;\n"
            ));
            let failure = project
                .ast_result()
                .expect_ast_failure("static invalid arithmetic remains a source diagnostic");
            assert_ast_compile_time_reason(&failure, expected_reason);
        }
    }
}

#[test]
fn implicit_failure_assertion_message_rejects_inferred_call_even_when_inactive() {
    for condition in ["ready", "true"] {
        let mut project = failure_project(&format!(
            "message |left I32, right I32| -> String:\n    value = left * right\n    return [value]\n;\n\
             validate |left I32, right I32, ready Bool|:\n\
                 assert({condition}, message(left, right))\n;\n"
        ));
        let messages = project
            .failure_contract_boundary_result()
            .expect_diagnosed("assertion message cannot escape by inferred private failure");
        assert_fallible_reason(
            &messages,
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        );
    }
}

#[test]
fn implicit_failure_assertion_message_rejects_template_range_runtime_step() {
    let mut project = failure_project(
        "validate |ready Bool, step I32|:\n\
             assert(ready, [loop 0 to 10 by step |i|: x])\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("template range checks cannot escape an assertion message");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
    );
}

#[test]
fn implicit_failure_dormant_generic_assertion_message_rejects_private_failure() {
    for error_slot in ["", " -> Error!"] {
        let mut project = failure_project(&format!(
            "message |left I32, right I32| -> String:\n    value = left * right\n    return [value]\n;\n\
             export:\n    validate type T |marker T, left I32, right I32, ready Bool|{error_slot}:\n\
                 assert(ready, message(left, right))\n    ;\n;\n"
        ));
        let failure = project.ast_result().expect_ast_failure(
            "dormant generic assertion messages must be validated before discard",
        );
        assert_ast_fallible_reason(
            &failure,
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        );
    }
}

#[test]
fn implicit_failure_prepared_recovery_lowers_catch_with_outward_assertion() {
    let mut project = failure_project(
        "message |left I32, right I32| -> String:\n    value = left * right\n    return [value]\n;\n\
         validate |left I32, right I32, ready Bool|:\n\
             text = message(left, right) catch then \"fallback\"\n    assert(ready, text)\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "validate");
    let prepared = initializer(&ast, &body[0]);
    assert_catch_contract(prepared, true, None, builtin_error_type_id(&project, &ast));
    assert!(failure_facts_of(prepared).summary.first_implicit.is_none());
    let NodeKind::Assert { message, .. } = &body[1].kind else {
        panic!("prepared message must remain an ordinary assertion");
    };
    let message = expression_view(&ast, *message);
    assert!(failure_facts_of(message).summary.first_implicit.is_none());
    assert!(failure_facts_of(message).summary.first_typed.is_none());
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_infallible_message_keeps_ordinary_assertion() {
    let mut project = failure_project(
        "message |value I32| -> String:\n    return [value]\n;\n\
         validate |value I32, ready Bool|:\n    assert(ready, message(value))\n;\n",
    );
    let hir = project
        .failure_contract_result()
        .expect("infallible messages must pass inferred-call validation");
    assert!(
        hir.functions.iter().any(|function| {
            hir.side_table
                .function_name_path(function.id)
                .and_then(|path| project.frontend.path_fork.component(path))
                .is_some_and(|component| {
                    project.frontend.string_table.resolve(component) == "validate"
                })
        }),
        "an ordinary infallible message must retain its assertion function at HIR"
    );
}

#[test]
fn implicit_failure_dormant_generic_contracts_validate_before_instantiation() {
    let mut project = failure_project(
        "export:\n    product type T |marker T, left I32, right I32| -> I32:\n\
                 return left * right\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a dormant public generic cannot publish bare numeric failure");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
    );
    assert_eq!(
        diagnostic_label_spans(diagnostic),
        [Some(
            diagnostic_primary_span(diagnostic).expect("producer span")
        )],
        "a direct numeric producer should be the sole retained boundary label"
    );

    let mut project = failure_project(
        "Failure = | message String |\n\
         product type T |marker T, left I32, right I32| -> I32, Failure!:\n\
             return left * right\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a dormant custom-error generic must discharge numeric failure");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function"),
    );
    let labels = diagnostic_label_spans(diagnostic);
    assert_eq!(labels.len(), 1);
    assert!(labels[0].is_some(), "numeric origin should remain labeled");

    for (prefix, error_slot) in [("export:\n", ", Error!"), ("", "")] {
        let closing_export = if prefix.is_empty() { "" } else { ";\n" };
        let mut project = failure_project(&format!(
            "{prefix}    product type T |marker T, left I32, right I32| -> I32{error_slot}:\n\
                 return left * right\n    ;\n{closing_export}"
        ));
        let ast = project.ast();
        assert!(
            !ast.nodes.iter().any(|node| match &node.kind {
                NodeKind::Function(path, _, _) => {
                    project
                        .frontend
                        .path_fork
                        .component(*path)
                        .is_some_and(|component| {
                            project.frontend.string_table.resolve(component) == "product"
                        })
                }
                _ => false,
            }),
            "uninstantiated generic acceptance must not fabricate an executable body"
        );
    }
}

#[test]
fn implicit_failure_range_loop_witnesses_retain_origin_labels() {
    for (scalar, end, zero) in [("I32", "10", "0"), ("F64", "10.0", "0.0")] {
        // Converged boundary: a custom error slot sees the HIR-projected contributor.
        let mut project = failure_project(&format!(
            "Failure = | message String |\n\
             walk |start {scalar}, step {scalar}| -> {scalar}, Failure!:\n\
                 loop start to {end} by step |value|:\n        return value\n    ;\n\
                 return {zero}\n;\n"
        ));
        let messages = project
            .failure_contract_boundary_result()
            .expect_diagnosed("custom contract must discharge range step failure");
        let boundary_diagnostic = single_ast_error_view(&messages);
        let facts = boundary_diagnostic
            .family_view::<InvalidFallibleHandlingView>()
            .expect("range loop boundary should retain fallible facts");
        assert_eq!(
            facts.reason,
            InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
        );
        assert_eq!(boundary_diagnostic.descriptor().code, "MOTH-RULE-0051");
        assert!(!matches!(
            boundary_diagnostic.primary_site(),
            SiteRef::Absent
        ));
        assert_eq!(
            frozen_builtin_failure_labels(&messages),
            vec![(BuiltinFailureLabelRole::OperationOrigin, true)]
        );
        assert_eq!(frozen_elided_call_hops(&messages), Some(0));

        // Dormant generic boundary: the finalizer reads the AST contributor directly.
        let mut project = failure_project(&format!(
            "export:\n    walk type T |marker T, start {scalar}, step {scalar}| -> {scalar}:\n\
                     loop start to {end} by step |value|:\n            return value\n        ;\n\
                     return {zero}\n    ;\n;\n"
        ));
        let failure = project
            .ast_result()
            .expect_ast_failure("a dormant public generic cannot publish range step failure");
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0051",
            Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
        );
        let labels = diagnostic_label_spans(diagnostic);
        assert_eq!(
            labels.len(),
            1,
            "range step origin is the only boundary label"
        );
        assert!(
            labels[0].is_some(),
            "range step origin should remain labeled"
        );
    }
}

#[test]
fn implicit_failure_dormant_complete_recursive_graph_without_numeric_origin_is_infallible() {
    let mut project = failure_project(
        "identity |value I32| -> I32:\n    return identity(value)\n;\n\
         export:\n    wrapper type T |marker T, value I32| -> I32:\n\
                 return identity(value)\n    ;\n;\n",
    );
    // Match concrete HIR convergence: stack exhaustion is fatal, not numeric failure.
    project.ast();
}

#[test]
fn implicit_failure_dormant_private_generic_identity_and_transitive_chain_are_infallible() {
    for helpers in [
        "identity type U |value U| -> U:\n    return value\n;\n",
        "middle type U |value U| -> U:\n    return identity(value)\n;\n\
         identity type V |value V| -> V:\n    return value\n;\n",
    ] {
        let target = if helpers.starts_with("middle") {
            "middle"
        } else {
            "identity"
        };
        for helper_first in [true, false] {
            let wrapper = format!(
                "export:\n    forward type T |value T| -> T:\n        return {target}(value)\n    ;\n;\n"
            );
            let source = if helper_first {
                format!("{helpers}{wrapper}")
            } else {
                format!("{wrapper}{helpers}")
            };
            let mut project = failure_project(&source);
            let ast = project.ast();
            assert!(
                !ast.nodes.iter().any(|node| match &node.kind {
                    NodeKind::Function(path, _, _) => project
                        .frontend
                        .path_fork
                        .component(*path)
                        .is_some_and(
                            |name| project.frontend.string_table.resolve(name) == "forward"
                        ),
                    _ => false,
                }),
                "dormant proofs must not publish executable template bodies"
            );
        }
    }
}

#[test]
fn implicit_failure_dormant_private_numeric_helper_retains_active_numeric_witness() {
    let mut project = failure_project(
        "identity type U |value U| -> U:\n    return value\n;\n\
         multiply |left I32, right I32| -> I32:\n    return left * right\n;\n\
         private_product type U |marker U, left I32, right I32| -> I32:\n\
             return multiply(left, right)\n;\n\
         export:\n    product type T |marker T, left I32, right I32| -> I32:\n\
                 retained = identity(marker)\n        return private_product(retained, left, right)\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a real numeric failure cannot escape a dormant export");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
    );
    let labels = diagnostic_label_spans(diagnostic);
    assert_eq!(
        labels.len(),
        3,
        "both private call hops and the origin remain labeled"
    );
    assert!(
        labels.iter().all(Option::is_some),
        "the numeric origin and private call hops should retain their source spans"
    );
}

#[test]
fn implicit_failure_dormant_recursive_chain_with_numeric_origin_is_rejected() {
    let mut project = failure_project(
        "recursive |value I32, ready Bool| -> I32:\n\
             if ready:\n        return value * 2\n    ;\n    return recursive(value, ready)\n;\n\
         export:\n    wrapper type T |marker T, value I32, ready Bool| -> I32:\n\
                 return recursive(value, ready)\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a recursive graph still carries numeric origins");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0051",
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function"),
    );
    assert!(
        !diagnostic_label_spans(diagnostic).is_empty(),
        "recursive numeric origin should remain labeled"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct FrozenBuiltinFailureSite {
    role: BuiltinFailureLabelRole,
    span: SourceSpan,
}

#[derive(Debug, PartialEq, Eq)]
struct BoundaryBuiltinFailureWitness {
    sites: Vec<FrozenBuiltinFailureSite>,
}

/// Keep exact frozen label spans for the compact boundary witness.
fn boundary_failure_witness<T>(failure: &CompilationResult<T>) -> BoundaryBuiltinFailureWitness {
    let diagnostic = single_ast_error_view(failure);
    let facts = diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("builtin-failure diagnostic should retain fallible facts");
    assert_eq!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    );
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0051");
    assert!(
        !matches!(diagnostic.primary_site(), SiteRef::Absent),
        "the boundary should retain its primary source site"
    );
    assert_eq!(
        frozen_builtin_failure_labels(failure),
        vec![
            (BuiltinFailureLabelRole::Call, true),
            (BuiltinFailureLabelRole::OperationOrigin, true),
        ],
        "the boundary should retain the call hop and terminal operation origin"
    );
    assert_eq!(frozen_elided_call_hops(failure), Some(0));

    let sites = diagnostic
        .durable_labels()
        .map(|label| {
            let span = match label.site {
                SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
                SiteRef::Absent => panic!("builtin-failure labels should retain their spans"),
            };
            FrozenBuiltinFailureSite {
                role: durable_builtin_failure_label_role(label.message),
                span,
            }
        })
        .collect();

    BoundaryBuiltinFailureWitness { sites }
}

fn assert_recursive_witness_fixture_sites(
    witness: &BoundaryBuiltinFailureWitness,
    source: &str,
    expected_call_name: &str,
) {
    let [call, origin] = witness.sites.as_slice() else {
        panic!(
            "expected one call hop and one operation origin, got {:?}",
            witness.sites
        );
    };
    assert_eq!(call.role, BuiltinFailureLabelRole::Call);
    assert_eq!(origin.role, BuiltinFailureLabelRole::OperationOrigin);
    assert_eq!(call.span.source(), origin.span.source());

    let call_start = source
        .rfind(expected_call_name)
        .expect("the boundary call name should appear in the concrete fixture");
    let multiplication_start = source
        .find("value * 2")
        .expect("the multiplication origin should appear in the concrete fixture");
    let origin_start = multiplication_start + "value ".len();
    let mut expected_span_builder = ExtendedSpanBuilder::new();
    let expected_call = SourceSpan::new(
        call.span.source(),
        LocalSpan::exact(
            call_start as u32,
            expected_call_name.len() as u32,
            &mut expected_span_builder,
        )
        .expect("the fixture call name range should fit in a local span"),
    );
    let expected_origin = SourceSpan::new(
        origin.span.source(),
        LocalSpan::exact(
            origin_start as u32,
            "*".len() as u32,
            &mut expected_span_builder,
        )
        .expect("the fixture multiplication operator should fit in a local span"),
    );
    assert_eq!(
        call.span, expected_call,
        "the frozen call label should name the boundary callee"
    );
    assert_eq!(
        origin.span, expected_origin,
        "the frozen operation label should name the fixture multiplication operator"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinFailureLabelRole {
    Call,
    OperationOrigin,
    WriteBackOrigin,
}

fn durable_builtin_failure_label_role(message: DurableLabelMessage<'_>) -> BuiltinFailureLabelRole {
    match message {
        DurableLabelMessage::BuiltinFailureCall => BuiltinFailureLabelRole::Call,
        DurableLabelMessage::BuiltinFailureOrigin => BuiltinFailureLabelRole::OperationOrigin,
        DurableLabelMessage::BuiltinWriteBackOrigin { .. } => {
            BuiltinFailureLabelRole::WriteBackOrigin
        }
        other => panic!("unexpected label on a builtin-failure boundary: {other:?}"),
    }
}

/// The single frozen builtin-failure error's label roles, each with whether it keeps a span.
fn frozen_builtin_failure_labels<T>(
    failure: &CompilationResult<T>,
) -> Vec<(BuiltinFailureLabelRole, bool)> {
    single_ast_error_view(failure)
        .durable_labels()
        .map(|label| {
            (
                durable_builtin_failure_label_role(label.message),
                label.spanned,
            )
        })
        .collect()
}

/// The elided call-hop count the single frozen builtin-failure error retains.
fn frozen_elided_call_hops<T>(failure: &CompilationResult<T>) -> Option<usize> {
    single_ast_error_view(failure)
        .family_view::<InvalidFallibleHandlingView>()
        .expect("a builtin-failure boundary retains its fallible-handling facts")
        .count
}

#[test]
fn implicit_failure_self_recursive_back_edge_keeps_overflow_origin_at_both_boundaries() {
    // WHAT: the recursive branch precedes the overflow producer in source order.
    // WHY: both witness walkers must skip only the back-edge and retain the available
    //      multiplication origin instead of reporting only call hops.
    const HELPER: &str = "helper |value I32, ready Bool| -> I32:\n\
         if ready:\n        return helper(value, ready)\n    ;\n    return value * 2\n;\n";
    const FAILURE: &str = "Failure = | message String |\n";
    let concrete_source = format!(
        "{FAILURE}{HELPER}boundary |value I32, ready Bool| -> I32, Failure!:\n    return helper(value, ready)\n;\n"
    );

    // The concrete boundary exercises HIR summary convergence through a custom error slot.
    // The harness lowers plain exports without public origins, so the slot carries the
    // concrete contract here; the witness walker under test is the same HIR one.
    let mut project = failure_project(&concrete_source);
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("escaping overflow must be rejected at the concrete boundary");
    let witness = boundary_failure_witness(&messages);
    assert_recursive_witness_fixture_sites(&witness, &concrete_source, "helper");

    // The dormant generic boundary exercises the AST finalizer over the same helper shape.
    let mut project = failure_project(&format!(
        "{HELPER}export:\n    wrapper type T |marker T, value I32, ready Bool| -> I32:\nreturn helper(value, ready)\n    ;\n;\n"
    ));
    let failure = project
        .ast_result()
        .expect_ast_failure("escaping overflow must be rejected at the dormant generic boundary");
    assert_ast_unhandled_builtin_export(&failure);
    assert_eq!(
        frozen_builtin_failure_labels(&failure),
        [
            (BuiltinFailureLabelRole::Call, true),
            (BuiltinFailureLabelRole::OperationOrigin, true),
        ],
        "the dormant lane retains one call hop and the multiplication origin"
    );
    assert_eq!(frozen_elided_call_hops(&failure), Some(0));

    // Repeated runs produce the same compact witness.
    let mut project = failure_project(&concrete_source);
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("the repeated run must reject the same boundary");
    assert_eq!(boundary_failure_witness(&messages), witness);
}

#[test]
fn implicit_failure_mutual_recursive_back_edge_keeps_overflow_origin_at_both_boundaries() {
    const HELPERS: &str = "first |value I32, ready Bool| -> I32:\n\
         if ready:\n        return second(value, ready)\n    ;\n    return value * 2\n;\n\
         second |value I32, ready Bool| -> I32:\n    return first(value, ready)\n;\n";
    const FAILURE: &str = "Failure = | message String |\n";
    let concrete_source = format!(
        "{FAILURE}{HELPERS}boundary |value I32, ready Bool| -> I32, Failure!:\n    return first(value, ready)\n;\n"
    );
    let mut project = failure_project(&concrete_source);
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("escaping overflow must be rejected at the concrete boundary");
    let witness = boundary_failure_witness(&messages);
    assert_recursive_witness_fixture_sites(&witness, &concrete_source, "first");

    let mut project = failure_project(&format!(
        "{HELPERS}export:\n    wrapper type T |marker T, value I32, ready Bool| -> I32:\nreturn first(value, ready)\n    ;\n;\n"
    ));
    let failure = project
        .ast_result()
        .expect_ast_failure("escaping overflow must be rejected at the dormant generic boundary");
    assert_ast_unhandled_builtin_export(&failure);
    assert_eq!(
        frozen_builtin_failure_labels(&failure),
        [
            (BuiltinFailureLabelRole::Call, true),
            (BuiltinFailureLabelRole::OperationOrigin, true),
        ],
        "the dormant lane retains one call hop and the multiplication origin"
    );
    assert_eq!(frozen_elided_call_hops(&failure), Some(0));

    let mut project = failure_project(&concrete_source);
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("the repeated run must reject the same boundary");
    assert_eq!(boundary_failure_witness(&messages), witness);
}

#[test]
fn implicit_failure_origin_free_self_recursion_stays_infallible_at_concrete_boundary() {
    // The dormant generic lane already proves this in
    // `implicit_failure_dormant_complete_recursive_graph_without_numeric_origin_is_infallible`;
    // the concrete lane must agree through HIR summary convergence. The harness lowers plain
    // exports without public origins, so a custom error slot carries the concrete contract.
    let mut project = failure_project(
        "Failure = | message String |\n\
         recurse |value I32| -> I32:\n    return recurse(value)\n;\n\
         boundary |value I32| -> I32, Failure!:\n    return recurse(value)\n;\n",
    );
    project
        .failure_contract_result()
        .expect("origin-free recursion carries no recoverable numeric failure");
}

#[test]
fn implicit_failure_dormant_custom_error_generic_receiver_uses_closed_trait_requirement_contract() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         DISPLAY_TEXT must:\n    display |This| -> String\n;\n\
         render type T is DISPLAY_TEXT |value T| -> String, Failure!:\n\
             return value.display()\n;\n",
    );
    project.ast();
}

#[test]
fn implicit_failure_typed_map_catch_resolves_infallible_private_arguments_in_both_orders() {
    let helper = "key |value String| -> String:\n    return [value]\n;\n";
    let caller = "recover |value String| -> I32:\n\
                      set_values ~= {\"key\" = 1}\n\
                      ~set_values.set(key(value), 2) catch:\n    ;\n\
                      get_values = {\"key\" = 1}\n\
                      found = get_values.get(key(value)) catch then 0\n\
                      remove_values ~= {\"key\" = 1}\n\
                      removed = ~remove_values.remove(key(value)) catch then 0\n\
                      return removed\n;\n";
    for helper_first in [true, false] {
        let source = if helper_first {
            format!("{helper}{caller}")
        } else {
            format!("{caller}{helper}")
        };
        let mut project = failure_project(&source);
        project
            .failure_contract_result()
            .expect("infallible arguments retain supported typed-map catches");
    }
}

#[test]
fn implicit_failure_typed_map_catch_recovers_failing_private_argument() {
    let mut project = failure_project(
        "key |value I32| -> I32:\n    return value * 2\n;\n\
         recover |value I32| -> I32:\n\
             values = {0 = 1}\n    return values.get(key(value)) catch then 0\n;\n",
    );
    let ast = project.ast();
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_private_catch_routes_argument_failure_before_outer_call() {
    let mut project = failure_project(
        "double |value I32| -> I32:\n    return value * 2\n;\n\
         combine |left I32, right I32| -> I32:\n    return left + right\n;\n\
         recover |value I32| -> I32:\n\
             return combine(double(value), value * 1000000) catch then -1\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = returned_value(&ast, &named_function_body(&project, &ast, "recover")[0]);
    assert_catch_contract(expression, true, None, error_type_id);
    let hir = project
        .lower_ast_result(ast)
        .expect("failed private arguments must lower inside the outer catch");
    let recover = named_hir_function(&project, &hir, "recover");
    let reachable = reachable_block_ids(&hir, recover.entry);
    let double = named_hir_function(&project, &hir, "double");
    let (argument_block, argument_call, argument_results) = hir
        .blocks
        .iter()
        .filter(|block| reachable.contains(&block.id))
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call {
                        target: CallTarget::Local(id),
                        results,
                        ..
                    } if *id == double.id => Some((block, statement, results)),
                    _ => None,
                })
        })
        .expect("the pending private argument must retain its ordered native call results");
    assert_eq!(argument_results.len(), 1);
    let pending = hir
        .catch_protected_calls
        .iter()
        .find(|record| record.statement == argument_call.id)
        .expect("pending private argument must retain its catch routing before convergence");
    assert_eq!(pending.owner, recover.id);
    assert!(reachable.contains(&pending.handler.block));
    let HirTerminator::Invoke {
        operation: HirFallibleOperation::NumericOp { op, .. },
        failure,
        error_result,
        ..
    } = &argument_block.terminator
    else {
        panic!("later arithmetic must run only after the argument call produces its value");
    };
    assert_eq!(op.operator, NumericOperator::Multiply);
    let (handler, handler_error) = error_adapter(&hir, *failure, *error_result);
    assert_eq!(handler, pending.handler.block);
    assert_eq!(handler_error, pending.handler.error_local);

    // After summary convergence the recorded scalar call must split onto the
    // private lane with its error edge routed to the same catch handler.
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("failed private arguments must lower inside the outer catch");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    assert!(
        !hir.catch_protected_calls.is_empty(),
        "lowering must record the pending catch edge for the private argument call"
    );
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("protected private calls must pass borrow checking before lane installation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("private failure lane must install");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("installed failure edges must pass borrow checking");
    assert!(
        hir.catch_protected_calls.is_empty(),
        "installation must drain every pending catch edge"
    );
    let recover = named_hir_function(&project, &hir, "recover");
    let reachable = reachable_block_ids(&hir, recover.entry);
    let mut installed_handler = None;
    for block in &hir.blocks {
        if !reachable.contains(&block.id) {
            continue;
        }
        let HirTerminator::Invoke {
            operation:
                HirFallibleOperation::Call {
                    target: CallTarget::Local(_),
                    ..
                },
            failure,
            error_result,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let (error_handler, _) = error_adapter(&hir, *failure, *error_result);
        assert!(
            reachable.contains(&error_handler),
            "the installed call failure must enter the shared catch handler"
        );
        if let Some(expected) = installed_handler {
            assert_eq!(
                expected, error_handler,
                "every installed call must share the one catch handler"
            );
        } else {
            installed_handler = Some(error_handler);
        }
    }
    assert!(
        installed_handler.is_some(),
        "the installed private calls must keep their error edges"
    );
}

#[test]
fn implicit_failure_catch_abandons_later_temporaries_without_handler_reads() {
    let mut project = failure_project(
        "double |value I32| -> I32:\n    return value * 2\n;\n\
         recover |value I32| -> I32:\n\
             return double(value) + value * 1000000 catch then -1\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = returned_value(&ast, &named_function_body(&project, &ast, "recover")[0]);
    assert_catch_contract(expression, true, None, error_type_id);
    let hir = project
        .lower_ast_result(ast)
        .expect("abandoned temporaries must lower with one handler join");
    let recover = named_hir_function(&project, &hir, "recover");
    let reachable = reachable_block_ids(&hir, recover.entry);
    let mut branch_count = 0;
    for block_id in &reachable {
        if matches!(
            hir.blocks[block_id.0 as usize].terminator,
            HirTerminator::Invoke { .. }
        ) {
            branch_count += 1;
        }
    }
    assert!(
        branch_count >= 2,
        "both the private call and the later multiplication must branch, aborting the rest"
    );

    // After summary convergence the recorded call must split onto the private
    // lane with its error edge routed to the same catch handler, and no
    // pending record may survive.
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("abandoned temporaries must lower with one handler join");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    assert!(
        !hir.catch_protected_calls.is_empty(),
        "lowering must record the pending catch edge for the private call"
    );
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("protected private calls must pass borrow checking before lane installation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("private failure lane must install");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("installed failure edges must pass borrow checking");
    assert!(
        hir.catch_protected_calls.is_empty(),
        "installation must drain every pending catch edge"
    );
    let recover = named_hir_function(&project, &hir, "recover");
    let reachable = reachable_block_ids(&hir, recover.entry);
    let mut installed_handler = None;
    let mut installed_calls = 0;
    for block in &hir.blocks {
        if !reachable.contains(&block.id) {
            continue;
        }
        let HirTerminator::Invoke {
            operation:
                HirFallibleOperation::Call {
                    target: CallTarget::Local(_),
                    ..
                },
            failure,
            error_result,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let (error_handler, _) = error_adapter(&hir, *failure, *error_result);
        assert!(
            reachable.contains(&error_handler),
            "the installed call failure must enter the shared catch handler"
        );
        if let Some(expected) = installed_handler {
            assert_eq!(
                expected, error_handler,
                "every installed call must share the one catch handler"
            );
        } else {
            installed_handler = Some(error_handler);
        }
        installed_calls += 1;
    }
    assert!(
        installed_calls >= 1,
        "the installed private call must keep its routed error edge"
    );
}

#[test]
fn implicit_failure_rhs_caught_compound_update_reports_writeback_origin() {
    const ESCAPING_SOURCE: &str = "risky |amount U16| -> U16, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         export:\n    bump type T |marker T, extra U16| -> U8:\n\
                 total ~U8 = 250\n        total += risky(extra) catch then extra\n        return total\n    ;\n;\n";
    const AST_SOURCE: &str = "risky |amount U16| -> U16, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         bump |extra U16| -> U8:\n    total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n";
    let mut project = failure_project(ESCAPING_SOURCE);
    let failure = project
        .ast_result()
        .expect_ast_failure("the write-back into the compound target must still escape");
    assert_ast_unhandled_builtin_export(&failure);
    let labels = single_ast_error_view(&failure)
        .durable_labels()
        .collect::<Vec<_>>();
    assert_eq!(labels.len(), 1, "the write-back origin is the sole label");
    let DurableLabelMessage::BuiltinWriteBackOrigin { target_type } = labels[0].message else {
        panic!(
            "expected a write-back origin label, got {:?}",
            labels[0].message
        );
    };
    assert_eq!(
        ast_first_error_type_display(&failure, target_type),
        "U8",
        "the label names the write-back target instead of the handled arithmetic"
    );
    let SiteRef::Local(origin_span) = labels[0].site else {
        panic!("the write-back producer keeps its local target span");
    };
    let mut project = failure_project(AST_SOURCE);
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "bump");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    let value = expression_view(&ast, *value);
    let ExpressionOperationView::Cast { source, .. } = value.operation() else {
        panic!("the narrow back into the target must stay a checked conversion");
    };
    // The write-back contributor is recorded at the compound value span, which
    // the cast shares; the handled RHS catch keeps a distinct inner span.
    assert_eq!(
        failure_facts_of(value)
            .implicit
            .iter()
            .find_map(|contributor| match contributor.source {
                ImplicitFailureSource::CompoundWriteBack { target, .. } =>
                    Some((contributor.span, target)),
                _ => None,
            }),
        Some((
            value.span(),
            builtin_type_ids::fixed_scalar(FixedScalar::U8)
        )),
        "the compound value owns the write-back contributor naming the target"
    );
    let ExpressionOperationView::NumericBinary { left, right, .. } = source.operation() else {
        panic!("promoted arithmetic must be separate from protected RHS");
    };
    let rhs = [left, right]
        .into_iter()
        .find(|operand| matches!(operand.operation(), ExpressionOperationView::ValueBlock(_)))
        .expect("compound arithmetic must retain one RHS catch operand");
    assert_ne!(
        Some(origin_span),
        rhs.span(),
        "the write-back origin must not point at the handled RHS"
    );
}

#[test]
fn implicit_failure_uncaught_compound_update_reports_writeback_origin() {
    let mut project = failure_project(
        // `y` is a genuinely wider U16 so the uncaught write-back stays a mixed-domain
        // conversion with its own compound write-back producer.
        "export:\n    bump type T |marker T, x U8, y U16| -> U8:\n\
                 total ~U8 = x\n        total += y\n        return total\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("an unhandled compound update must be rejected");
    let diagnostic = assert_ast_unhandled_builtin_export(&failure);
    let labels = diagnostic_label_spans(diagnostic);
    assert_eq!(labels.len(), 1, "the write-back origin is the sole label");
    assert!(
        labels[0].is_some(),
        "the compound write-back origin should retain its source span"
    );
}

#[test]
fn implicit_failure_plain_return_arithmetic_keeps_operation_origin() {
    let mut project = failure_project(
        "export:\n    bump type T |marker T, x I32, y I32| -> I32:\n\
                 return x + y\n    ;\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("unhandled arithmetic must be rejected");
    let diagnostic = assert_ast_unhandled_builtin_export(&failure);
    let labels = diagnostic_label_spans(diagnostic);
    assert_eq!(labels.len(), 1, "the arithmetic origin is the sole label");
    assert!(
        labels[0].is_some(),
        "the arithmetic operation should retain its source span"
    );
}

#[test]
fn implicit_failure_custom_slot_keeps_writeback_origin() {
    let mut project = failure_project(
        // The wider U16 callee keeps the write-back a genuine mixed-domain conversion whose
        // producer must still name the custom-slot witness.
        "Failure = | message String |\n\
         risky |amount U16| -> U16, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         bump |extra U16| -> U8, Failure!:\n    total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n",
    );
    let messages = project
        .failure_contract_boundary_result()
        .expect_diagnosed("the write-back must not escape through a custom slot");
    assert_eq!(error_diagnostics(&messages).count(), 1);
    let boundary_diagnostic = single_ast_error_view(&messages);
    let facts = boundary_diagnostic
        .family_view::<InvalidFallibleHandlingView>()
        .expect("custom-slot rejection should retain fallible facts");
    assert_eq!(
        facts.reason,
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction
    );
    assert!(!matches!(
        boundary_diagnostic.primary_site(),
        SiteRef::Absent
    ));
    assert_eq!(
        frozen_builtin_failure_labels(&messages),
        vec![(BuiltinFailureLabelRole::WriteBackOrigin, true)]
    );
    let labels = boundary_diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    let DurableLabelMessage::BuiltinWriteBackOrigin { target_type } = labels[0].message else {
        panic!(
            "expected a write-back origin label, got {:?}",
            labels[0].message
        );
    };
    assert_eq!(ast_first_error_type_display(&messages, target_type), "U8");
}
#[test]
fn implicit_failure_failed_compound_writeback_keeps_target_after_rhs_catch() {
    let mut project = failure_project(
        // The wider U16 callee keeps the narrow back into the U8 target a genuine checked
        // conversion that stays separate from the RHS catch.
        "risky |amount U16| -> U16, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         adjust |extra U16| -> U8:\n\
             total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "adjust");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    let value = expression_view(&ast, *value);
    assert!(
        matches!(value.operation(), ExpressionOperationView::Cast { .. }),
        "the narrow back into the target must stay a checked conversion outside the RHS catch"
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("RHS catch must lower while the write-back stays separate");
    let adjust = named_hir_function(&project, &hir, "adjust");
    let reachable = reachable_block_ids(&hir, adjust.entry);
    assert!(
        reachable.iter().any(|block_id| matches!(
            &hir_block(&hir, *block_id).terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::CastOp { .. },
                ..
            }
        )),
        "a failing narrow back into the target must keep its checked conversion edge"
    );

    // The typed RHS routes through Invoke immediately and needs no private-lane record.
    // Installation must preserve that routing and the separate write-back conversion edge.
    let ast = project.ast();
    let lowered = project
        .frontend
        .with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })
        .expect("RHS catch must lower while the write-back stays separate");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    assert!(
        hir.catch_protected_calls.is_empty(),
        "a typed callee routes through Invoke, not the private lane"
    );
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("protected RHS calls must pass borrow checking before lane installation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
        project.frontend.external_package_registry.as_ref(),
    )
    .expect("private failure lane must install");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("installed failure edges must pass borrow checking");
    assert!(
        hir.catch_protected_calls.is_empty(),
        "installation must leave no pending catch edge behind"
    );
    let adjust = named_hir_function(&project, &hir, "adjust");
    let reachable = reachable_block_ids(&hir, adjust.entry);
    let typed_call = hir
        .blocks
        .iter()
        .filter(|block| reachable.contains(&block.id))
        .find(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation:
                        crate::compiler_frontend::hir::terminators::HirFallibleOperation::Call {
                            target: CallTarget::Local(_),
                            ..
                        },
                    ..
                }
            )
        })
        .expect("the typed RHS call must survive lane installation");
    let HirTerminator::Invoke {
        failure,
        error_result,
        ..
    } = &typed_call.terminator
    else {
        panic!("the typed RHS call must branch into the RHS catch handler");
    };
    let (error_handler, _) = error_adapter(&hir, *failure, *error_result);
    assert!(
        reachable.contains(&error_handler),
        "the typed RHS call failure must enter the RHS catch handler"
    );
    assert!(
        reachable.iter().any(|block_id| matches!(
            &hir_block(&hir, *block_id).terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::CastOp { .. },
                ..
            }
        )),
        "the write-back conversion must keep its checked edge after installation"
    );
}

#[test]
fn implicit_failure_zero_success_statement_catch_covers_host_argument_evaluation() {
    let mut project = failure_project(
        "message || -> String, Error!:\n    return \"ok\"\n;\n\
         recover ||:\n    io.line(message()) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let NodeKind::ExpressionStatement(expression) = &body[0].kind else {
        panic!("zero-success catch must remain a statement expression");
    };
    let error_type_id = builtin_error_type_id(&project, &ast);
    let block = assert_catch_contract(
        expression_view(&ast, *expression),
        false,
        Some(error_type_id),
        error_type_id,
    );
    assert!(block.result_type_ids.is_empty());
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("zero-success recovery body");
    };
    assert!(
        matches!(body.as_slice(), [node] if matches!(node.kind, NodeKind::ExpressionStatement(_)))
    );
    assert_catch_lowering_succeeds(&mut project, ast);

    let mut project = failure_project(
        "message || -> String, Error!:\n    return \"ok\"\n;\n\
         bad ||:\n    io.line(message())\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a statement boundary must not drop an argument's typed error");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::UnhandledErrorReturn,
    );
}

#[test]
fn implicit_failure_zero_success_host_catch_lowers_pending_numeric_argument() {
    let mut project = failure_project(
        "@test/default consume\n\
         recover |left I32, right I32|:\n\
             consume(left * right) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
    );
    let host_id = Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "consume".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![],
            error_return_type: Some(ExternalSignatureType::BuiltinError),
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("fallible host test signature must register");
    let ast = project.ast();
    let NodeKind::ExpressionStatement(expression) =
        &named_function_body(&project, &ast, "recover")[0].kind
    else {
        panic!("zero-success host recovery must remain an expression statement");
    };
    let expression = expression_view(&ast, *expression);
    let error_type = builtin_error_type_id(&project, &ast);
    let block = assert_catch_contract(expression, true, Some(error_type), error_type);
    assert!(matches!(
        handled_value(expression, block).operation(),
        ExpressionOperationView::Call(call)
            if call.handling().is_some()
                && matches!(call.metadata().target, ResolvedCallableTarget::Host(_))
    ));
    assert!(
        block.result_type_ids.is_empty(),
        "host recovery must not invent success slots"
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("numeric host arguments must support local recovery");
    let (multiply, producer_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
    let (success, error) = fallible_edges(&hir, multiply, producer_error_local);
    let handler_slot = error_adapter(&hir, error, producer_error_local);
    let host_block = hir_block(&hir, success);
    let HirTerminator::Invoke {
        operation:
            HirFallibleOperation::Call {
                target: CallTarget::External(id),
                ..
            },
        success_results,
        error_result: host_error_local,
        ..
    } = &host_block.terminator
    else {
        panic!("the protected host call must execute only after its numeric argument succeeds");
    };
    assert_eq!(*id, host_id);
    assert!(
        success_results.is_empty(),
        "error-only host must not manufacture a success slot"
    );
    let (_, host_error) = fallible_edges(&hir, host_block, *host_error_local);
    assert_eq!(
        error_adapter(&hir, host_error, *host_error_local),
        handler_slot,
        "host and numeric failures must initialise the same handler Error slot"
    );
}

#[test]
fn implicit_failure_catches_keep_external_float_checks_on_function_boundary() {
    let mut project = failure_project(
        "@test/default measure\n\
         Failure = | message String |\n\
         load |value F64| -> F64, Failure!:\n    return value\n;\n\
         recover |value F64| -> F64:\n\
             result = load(measure(value)) catch then 0.0\n    return result\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::F64)),
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::Fixed(FixedScalar::F64),
            ))],
            error_return_type: None,
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("float host signature must register");
    let ast = project.ast();
    let hir = project
        .lower_ast_result(ast)
        .expect("custom catch must not absorb injected float checks");
    let validation = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::ValidateFloat { failure_mode, .. } => Some(*failure_mode),
            _ => None,
        })
        .expect("external float result must still be boundary-validated");
    assert_eq!(validation, NumericFailureMode::Trap);

    let mut project = failure_project(
        "@test/default measure\n\
         load |value F64| -> F64, Error!:\n    return value\n;\n\
         recover |value F64| -> F64:\n\
             result = load(measure(value)) catch then 0.0\n    return result\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::F64)),
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::Fixed(FixedScalar::F64),
            ))],
            error_return_type: None,
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("float host signature must register");
    let ast = project.ast();
    let hir = project
        .lower_ast_result(ast)
        .expect("builtin Error catch must not absorb injected float checks");
    let validation = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::ValidateFloat { failure_mode, .. } => Some(*failure_mode),
            _ => None,
        })
        .expect("external float result must still be boundary-validated");
    assert_eq!(validation, NumericFailureMode::Trap);
}

#[test]
fn implicit_failure_builtin_catch_lowers_numeric_get_and_set_arguments() {
    for (source, statement_index, has_success) in [
        (
            "recover |left I32, right I32| -> I32:\n\
                 items {I32} = {9}\n\
                 value = items.get(left * right) catch then 0\n    return value\n;\n",
            1,
            true,
        ),
        (
            "recover |left I32, right I32|:\n\
                 items ~{I32} = {9}\n\
                 ~items.set(0, left * right) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
            1,
            false,
        ),
    ] {
        let mut project = failure_project(source);
        let ast = project.ast();
        let statement = &named_function_body(&project, &ast, "recover")[statement_index];
        let expression = if has_success {
            initializer(&ast, statement)
        } else {
            let NodeKind::ExpressionStatement(expression) = &statement.kind else {
                panic!("zero-success builtin recovery must remain an expression statement");
            };
            expression_view(&ast, *expression)
        };
        let error_type = builtin_error_type_id(&project, &ast);
        let block = assert_catch_contract(expression, true, Some(error_type), error_type);
        assert!(matches!(
            handled_value(expression, block).operation(),
            ExpressionOperationView::HandleFallible { .. }
        ));
        assert_eq!(block.result_type_ids.len(), usize::from(has_success));
        let hir = project
            .lower_ast_result(ast)
            .expect("builtin catch must sequence numeric arguments");
        let (multiply, producer_error_local) = numeric_producer(&hir, NumericOperator::Multiply);
        let (_, error) = fallible_edges(&hir, multiply, producer_error_local);
        error_adapter(&hir, error, producer_error_local);
    }
}

#[test]
fn implicit_failure_zero_success_receiver_and_builtin_catches_keep_typed_error_contracts() {
    for (source, statement_index) in [
        (
            "Box = | value I32 |\n\
             touch |this Box| -> Error!:\n;\n\
             recover |box Box|:\n    box.touch() catch:\n        io.line(\"fallback\")\n    ;\n;\n",
            0,
        ),
        (
            "recover |value I32|:\n\
                 items ~{I32} = {0}\n\
                 ~items.set(0, value) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
            1,
        ),
    ] {
        let mut project = failure_project(source);
        let ast = project.ast();
        let body = named_function_body(&project, &ast, "recover");
        let NodeKind::ExpressionStatement(expression) = &body[statement_index].kind else {
            panic!("error-only receiver recovery must remain a statement");
        };
        let error_type_id = builtin_error_type_id(&project, &ast);
        let block = assert_catch_contract(
            expression_view(&ast, *expression),
            false,
            Some(error_type_id),
            error_type_id,
        );
        assert!(
            block.result_type_ids.is_empty(),
            "error-only handlers must not invent success slots"
        );
        project
            .lower_ast_result(ast)
            .expect("existing zero-success typed recovery must continue lowering");
    }

    let mut project = failure_project(
        "Box = | value I32 |\n\
         touch |this Box| -> Error!:\n;\n\
         bad |box Box|:\n    box.touch()\n;\n",
    );
    let failure = project
        .ast_result()
        .expect_ast_failure("a receiver statement cannot discard its typed error");
    assert_ast_fallible_reason(
        &failure,
        InvalidFallibleHandlingReason::UnhandledErrorReturn,
    );
}

#[test]
fn implicit_failure_same_module_cross_file_call_keeps_canonical_private_failure_lane() {
    let mut project = FrontendProject::new(
        &[
            (
                "src/@page.moth",
                "@helpers multiply\n\
                 safe_product |left I32, right I32| -> I32, Error!:\n\
                     return multiply(left, right)\n;\n",
            ),
            (
                "src/helpers.moth",
                "multiply |left I32, right I32| -> I32:\n    return left * right\n;\n",
            ),
        ],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );
    let ast = project.ast();
    let helper_path = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, _, _)
                if project
                    .frontend
                    .path_fork
                    .component(*path)
                    .is_some_and(|component| {
                        project.frontend.string_table.resolve(component) == "multiply"
                    }) =>
            {
                Some(*path)
            }
            _ => None,
        })
        .expect("same-module helper must retain its canonical declaration path");
    let value = returned_value(
        &ast,
        &named_function_body(&project, &ast, "safe_product")[0],
    );
    assert_eq!(
        value
            .value_type_id()
            .expect("safe_product returns one success value"),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert!(matches!(
        value.operation(),
        ExpressionOperationView::Call(call)
            if matches!(
                call.metadata().target,
                ResolvedCallableTarget::Function(path) if path == helper_path
            )
    ));
    let facts = failure_facts_of(value);
    let private_call_span = facts
        .summary
        .first_private_call
        .expect("same-module private call must retain its witness")
        .span;
    assert!(private_call_span.is_some());
    assert!(facts.summary.first_typed.is_none());

    let hir = project
        .lower_ast_result(ast)
        .expect("same-module implicit calls must retain existing lowering");
    let helper = hir
        .functions
        .iter()
        .find(|function| hir.side_table.function_name_path(function.id) == Some(helper_path))
        .expect("canonical same-module helper must lower");
    let caller = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .and_then(|path| project.frontend.path_fork.component(path))
                .is_some_and(|component| {
                    project.frontend.string_table.resolve(component) == "safe_product"
                })
        })
        .expect("Error! caller must lower");
    let facts = &hir.function_failure_facts[&caller.id];
    assert_eq!(facts.boundary, HirBuiltinFailureBoundary::BuiltinErrorSlot);
    assert!(matches!(
        facts.contributors.as_slice(),
        [contributor] if matches!(
            contributor.source, HirBuiltinFailureSource::Call(CallTarget::Local(target))
                if target == helper.id && contributor.span == private_call_span
        )
    ));
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("same-module numeric helper must pass borrow validation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    );
    assert!(report.analysis.public_call_summaries[&helper.id].escapes_builtin_failure);
    assert!(!report.analysis.public_call_summaries[&caller.id].escapes_builtin_failure);
}
