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
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    FailureDisposition, ImplicitFailureSource,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, CompileTimeEvaluationErrorReason, DiagnosticLabelMessage,
    DiagnosticPayload, InvalidAssignmentTargetReason, InvalidCastReason,
    InvalidFallibleHandlingReason, InvalidReturnShapeReason,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalFunctionDef, ExternalFunctionLowerings,
    ExternalParameter, ExternalReturnSlot, ExternalSignatureType,
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
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::ExtendedSpanBuilder;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::{
    StyleDirectiveEffects, StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::tokenize_source_for_test;
use crate::compiler_frontend::tokenizer::tokens::{TemplateBodyMode, TokenizerEntryMode};
use crate::compiler_frontend::{AstBuildRequest, CompilerFrontend, FrontendBuildProfile};
use crate::projects::settings::Config;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::profile::NumericProfile;
use rustc_hash::FxHashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

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
        )
        .expect("project path resolver should build");

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
            options: Config::new(canonical_project_root).frontend_options(NumericProfile::STANDARD),
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
            &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
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

    fn ast_result(&mut self) -> Result<Ast, CompilerMessages> {
        // These fixtures have valid declaration shells. Body diagnostics must be returned by
        // the semantic stage, never routed through the success-only `ast`/`hir` helpers.
        let sorted = self.sorted_headers();
        self.frontend
            .with_compiler(|compiler| {
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
            })
            .map(|result| result.ast)
    }

    fn ast(&mut self) -> Ast {
        self.ast_result().expect("AST construction should succeed")
    }

    fn lower_ast_result(&mut self, ast: Ast) -> Result<HirModule, CompilerMessages> {
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
    fn failure_contract_result(&mut self) -> Result<HirModule, CompilerMessages> {
        let ast = self.ast_result()?;
        let lowered = self.frontend.with_compiler(|compiler| {
            compiler.generate_hir(
                ast,
                HirFunctionOriginLookup::default(),
                None,
                Default::default(),
            )
        })?;
        let hir = lowered.hir_module;
        let mut report = self
            .frontend
            .with_compiler(|compiler| compiler.check_borrows(&hir))?;
        crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
            &hir,
            &mut report,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &self.frontend.string_table))?;

        for function in &hir.functions {
            let diagnostic =
                crate::compiler_frontend::module_compilation::generated::builtin_failure_diagnostic(
                    &hir, &report, function.id,
                ).map_err(|error| {
                    CompilerMessages::from_error_ref(error, &self.frontend.string_table)
                })?;
            if let Some(diagnostic) = diagnostic {
                return Err(CompilerMessages::from_diagnostic_ref(
                    diagnostic,
                    &self.frontend.string_table,
                )
                .with_type_context_for_all_diagnostics(lowered.type_environment));
            }
        }
        Ok(hir)
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
            "Point = |\n    value Int,\n|\npoint = Point(1)\nloop 0 to 2 |i|:\n    io.line([: [point.value]])\n;\n",
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
                "add|left Int, right Int| -> Int:\n    return left + right\n;\n",
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

    // The lowered helper is the imported `add|left Int, right Int|`, not just "some function":
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

    let sorted = project.sorted_headers();
    let Err(messages) = project.frontend.with_compiler(|compiler| {
        compiler.headers_to_ast(
            AstBuildRequest {
                sorted,
                entry_file_path: &project.entry_file,
                root_role: ModuleRootRole::Normal,
                build_profile: FrontendBuildProfile::Dev,
                capacity_estimate: Default::default(),
                resolved_file_references: ResolvedFileReferenceTable::default(),
                build_config_values: Arc::new(Default::default()),
                module_origin: None,
            },
            #[cfg(feature = "timers")]
            None,
        )
    }) else {
        panic!("const host calls should fail during AST construction");
    };

    let first_diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("AST construction should return a diagnostic");
    assert!(
        first_diagnostic.primary_span.is_some(),
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
    let messages = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect_err("multiple mutable borrows should fail borrow checking");

    let first_diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("borrow checking should return a diagnostic");
    assert!(
        first_diagnostic.primary_span.is_some(),
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
            "enabled #= true\nchoose || -> Int:\n    if enabled:\n        return 1\n    ;\n;\nio.line([: [choose()]])\n",
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
            "enabled #= true\ndisabled #= false\nchoose_then || -> Int:\n    return if enabled:\n        return 1\n    else\n        then 2\n    ;\n;\nchoose_else || -> Int:\n    return if disabled:\n        then 1\n    else\n        return 2\n    ;\n;\nio.line([: [choose_then()]-[choose_else()]])\n",
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

fn initializer(node: &crate::compiler_frontend::ast::ast_nodes::AstNode) -> &Expression {
    let NodeKind::VariableDeclaration(declaration) = &node.kind else {
        panic!("expected declaration, got {:?}", node.kind);
    };
    &declaration.value
}

fn returned_value(node: &crate::compiler_frontend::ast::ast_nodes::AstNode) -> &Expression {
    let NodeKind::Return(values) = &node.kind else {
        panic!("expected success return, got {:?}", node.kind);
    };
    assert_eq!(values.len(), 1);
    &values[0]
}

fn catch_block(expression: &Expression) -> &ValueCatchBlock {
    let ExpressionKind::ValueBlock { block } = &expression.kind else {
        panic!("catch should remain an expression-owned value block");
    };
    let ValueBlock::Catch(block) = block.as_ref() else {
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
fn assert_catch_contract(
    expression: &Expression,
    has_implicit: bool,
    typed_error_id: Option<TypeId>,
    error_type_id: TypeId,
) -> &ValueCatchBlock {
    let block = catch_block(expression);
    let facts = &block.handled_value.failure_facts;
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
    assert_eq!(expression.type_id, block.handled_value.type_id);
    block
}

#[track_caller]
fn assert_fallible_reason(messages: &CompilerMessages, expected: InvalidFallibleHandlingReason) {
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "unexpected diagnostics: {messages:?}");
    assert!(
        matches!(
            &diagnostics[0].payload,
            DiagnosticPayload::InvalidFallibleHandling { reason } if reason == &expected
        ),
        "expected {expected:?}, got {:?}",
        diagnostics[0]
    );
    assert!(diagnostics[0].primary_span.is_some());
    assert_eq!(diagnostics[0].kind.code(), "MOTH-RULE-0051");
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
        panic!("carrier must be loaded from its producer local");
    };
    assert!(
        place.projections.is_empty(),
        "carrier must be a direct local load"
    );
    place.root
}

fn numeric_producer(hir: &HirModule, operator: NumericOperator) -> (&HirBlock, LocalId) {
    let producers = hir
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .statements
                .iter()
                .filter_map(move |statement| match &statement.kind {
                    HirStatementKind::NumericOp {
                        op,
                        failure_mode,
                        result,
                        ..
                    } if op.operator == operator => {
                        assert_eq!(*failure_mode, NumericFailureMode::ReturnError);
                        Some((block, result.local()))
                    }
                    _ => None,
                })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        producers.len(),
        1,
        "expected exactly one {operator:?} producer"
    );
    producers[0]
}

fn fallible_edges(hir: &HirModule, block: &HirBlock, carrier: LocalId) -> (BlockId, BlockId) {
    let HirTerminator::FallibleBranch {
        result,
        success_block,
        error_block,
    } = &block.terminator
    else {
        panic!("recoverable producer must branch before its payload is consumed");
    };
    assert_eq!(loaded_local(hir, result), carrier);
    for statement in &block.statements {
        if let HirStatementKind::Write { value, .. } = &statement.kind {
            match &hir.expressions.expression(*value).kind {
                HirExpressionKind::FallibleUnwrapSuccess { result }
                | HirExpressionKind::FallibleUnwrapError { result } => {
                    assert_ne!(
                        loaded_local(hir, result),
                        carrier,
                        "a producer must branch before either payload is assigned"
                    );
                }
                _ => {}
            }
        }
    }
    (*success_block, *error_block)
}

// Every producer has its own error adapter. Only that edge may unwrap its carrier,
// then assign the common handler's Error slot before entering the handler once.
fn error_adapter(hir: &HirModule, error_block: BlockId, carrier: LocalId) -> (BlockId, LocalId) {
    let adapter = hir_block(hir, error_block);
    let [statement] = adapter.statements.as_slice() else {
        panic!("error adapter must only initialise the handler's error slot");
    };
    let HirStatementKind::Write {
        target: HirWriteTarget::DefineLocal(error_local),
        value,
    } = &statement.kind
    else {
        panic!("error adapter must assign the shared error local");
    };
    let HirExpressionKind::FallibleUnwrapError { result } =
        &hir.expressions.expression(*value).kind
    else {
        panic!("error adapter must unwrap the producer's failure payload");
    };
    assert_eq!(loaded_local(hir, result), carrier);
    let HirTerminator::Jump { target, .. } = &adapter.terminator else {
        panic!("error adapter must enter the common handler");
    };
    assert_ne!(*target, error_block);
    (*target, *error_local)
}

#[test]
fn implicit_failure_private_chain_preserves_success_and_selects_error_materialisation() {
    let mut project = failure_project(
        "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
         first |left Int, right Int| -> Int:\n    return multiply(left, right)\n;\n\
         second |left Int, right Int| -> Int:\n    return first(left, right)\n;\n\
         safe_product |left Int, right Int| -> Int, Error!:\n    return second(left, right)\n;\n",
    );
    let ast = project.ast();
    let product = returned_value(&named_function_body(&project, &ast, "multiply")[0]);
    assert_eq!(product.type_id, builtin_type_ids::INT);
    assert!(product.failure_facts.checked_numeric_operation);
    assert!(product.failure_facts.summary.first_numeric.is_some());

    for name in ["first", "second", "safe_product"] {
        let value = returned_value(&named_function_body(&project, &ast, name)[0]);
        assert_eq!(value.type_id, builtin_type_ids::INT);
        assert!(matches!(value.kind, ExpressionKind::FunctionCall { .. }));
        assert!(value.failure_facts.summary.first_private_call.is_some());
        assert!(value.failure_facts.summary.first_typed.is_none());
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
    )
    .expect("private inferred failure summaries should converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
        let carrier = type_environment.fallible_carrier_slots(function.return_type);
        assert!(
            carrier.is_some(),
            "{name} must expose builtin Error through one carrier"
        );
        assert!(
            function_reaches_error_return(&hir, function.entry),
            "{name} must return the existing Error on its own failure edge",
        );
    }
    assert!(
        hir.blocks
            .iter()
            .any(|block| block.statements.iter().any(|statement| {
                matches!(
                    statement.kind,
                    HirStatementKind::NumericOp {
                        failure_mode: NumericFailureMode::ReturnError,
                        ..
                    }
                )
            })),
        "the private multiply helper must return overflow instead of trapping"
    );
}

#[test]
fn implicit_failure_lanes_and_builtin_catch_preserve_float_integrity_guards() {
    let mut project = failure_project(
        "@test/default measure\n\
         plain |value Float| -> String:\n\
             label String = cast measure(value)\n    return label\n;\n\
         escaping |value Float, left Int, right Int| -> String:\n\
             label String = cast measure(value)\n\
             product = left * right\n    return [: [label] [product]]\n;\n\
         caught |value Float| -> String:\n\
             label String = cast measure(value) * value catch then \"fallback\"\n\
             return label\n;\n\
         declared |value Float| -> String, Error!:\n\
             label String = cast measure(value)\n    return label\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::NativeFloat,
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(
                ExternalSignatureType::NativeFloat,
            )],
            error_return_type: None,
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
        .expect("float formatting and external validation must lower");
    let mut hir = lowered.hir_module;
    let mut type_environment = lowered.type_environment;
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("float integrity guards must pass borrow checking");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    )
    .expect("numeric failure summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
    )
    .expect("only implicit failures may join the private lane");
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("integrity guards and installed numeric lanes must pass borrow checking");

    for (name, expected_mode, escapes) in [
        ("plain", NumericFailureMode::Trap, false),
        ("escaping", NumericFailureMode::Trap, true),
        ("caught", NumericFailureMode::Trap, false),
        ("declared", NumericFailureMode::ReturnError, false),
    ] {
        let function = named_hir_function(&project, &hir, name);
        assert_eq!(
            report.analysis.public_call_summaries[&function.id].escapes_builtin_failure,
            escapes,
        );
        let mut formatting = Vec::new();
        let mut validation = Vec::new();
        for block_id in reachable_block_ids(&hir, function.entry) {
            for statement in &hir.blocks[block_id.0 as usize].statements {
                match &statement.kind {
                    HirStatementKind::FormatFloat { failure_mode, .. } => {
                        formatting.push(*failure_mode);
                    }
                    HirStatementKind::ValidateFloat { failure_mode, .. } => {
                        validation.push(*failure_mode);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            formatting,
            [expected_mode],
            "{name} must preserve its formatting contract"
        );
        assert_eq!(
            validation,
            [expected_mode],
            "{name} must preserve finite-value validation"
        );
    }
    for name in ["escaping", "caught"] {
        let function = named_hir_function(&project, &hir, name);
        assert!(
            function_numeric_ops_return_error(&hir, function.entry),
            "{name} must still recover ordinary checked arithmetic",
        );
    }
}

#[test]
fn implicit_failure_compound_writeback_and_range_update_join_private_lane() {
    let mut project = failure_project(
        "bump |amount U8| -> U8:\n\
             total ~U8 = 200\n\
             total += amount\n\
             return total\n;\n\
         safe_bump |amount U8| -> U8, Error!:\n\
             return bump(amount)\n;\n\
         walk |start Int, end Int| -> Int:\n\
             seen ~Int = 0\n\
             loop start to & end |value|:\n\
                 seen = copy value\n\
             ;\n\
             return seen\n;\n\
         safe_walk |start Int, end Int| -> Int, Error!:\n\
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
    )
    .expect("statement-owned failure summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
                hir.blocks[block_id.0 as usize]
                    .statements
                    .iter()
                    .any(|statement| {
                        matches!(
                statement.kind,
                HirStatementKind::RangeStepFailure {
                    cause: crate::compiler_frontend::hir::numeric::RangeStepFailureCause::ZeroStep,
                    failure_mode: NumericFailureMode::ReturnError,
                    ..
                }
            )
                    })
            }),
        "a zero step must return builtin Error through the private failure lane",
    );
}

#[test]
fn private_failure_lane_marks_carriers_as_temps_without_weakening_fresh_mutable_arguments() {
    let mut project = failure_project(
        "increment |value ~Int| -> Int:\n\
             value += 1\n\
             return value\n;\n\
         fresh |value Int| -> Int:\n\
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
    )
    .expect("private checked arithmetic summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
    )
    .expect("private checked arithmetic must install failure lanes");

    let mut introduced_carriers = 0;
    for local in hir.blocks.iter().flat_map(|block| &block.locals) {
        let origin = hir.side_table.local_origin(local.id);
        if let Some(original_origin) = original_origins.get(&local.id) {
            assert_eq!(
                origin, *original_origin,
                "lane installation must preserve existing origins, including fresh mutable roots"
            );
            continue;
        }

        introduced_carriers += 1;
        let origin = origin.expect("every lane-generated carrier must have a typed origin");
        assert_eq!(origin.kind, HirLocalOriginKind::CompilerTemp);
        assert_eq!(origin.call_span, None);
        assert_eq!(origin.argument_index, None);
        assert!(
            local.mutable,
            "carrier storage must remain writable scratch space"
        );
        assert!(
            type_environment.fallible_carrier_slots(local.ty).is_some(),
            "lane-generated storage must carry the checked success and error outcomes"
        );
    }
    assert!(
        introduced_carriers > 0,
        "checked arithmetic and the private call must introduce transport carriers"
    );
    project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("transport carriers must not introduce exclusive aliases of fresh mutable roots");
}

#[test]
fn private_failure_lane_preserves_unrelated_fatal_fallible_edge() {
    let mut project = failure_project(
        "bump |amount U8| -> U8:\n\
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
    )
    .expect("private failure summaries must converge");
    assert!(report.analysis.public_call_summaries[&function_id].escapes_builtin_failure);
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
    assert!(matches!(
        error.kind,
        HirExpressionKind::FallibleUnwrapError { .. }
    ));
    let return_type = hir
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .expect("private function must remain installed")
        .return_type;
    let (_, error_type) = type_environment
        .fallible_carrier_slots(return_type)
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
        let Some(carrier) = block
            .statements
            .iter()
            .find_map(|statement| match &statement.kind {
                HirStatementKind::CastOp {
                    result: Some(local),
                    ..
                } => Some(*local),
                _ => None,
            })
        else {
            continue;
        };
        let HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } = &block.terminator
        else {
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
                                HirExpressionKind::FallibleUnwrapSuccess { result }
                                    if loaded_local(hir, result) == carrier.local()
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
        for statement in &hir.blocks[block_id.0 as usize].statements {
            if let HirStatementKind::NumericOp { failure_mode, .. } = &statement.kind {
                if *failure_mode != NumericFailureMode::ReturnError {
                    return false;
                }
                found = true;
            }
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
         area |left Int, right Int, invalid Bool| -> Int, Failure!:\n\
             if invalid:\n        return! Failure(\"invalid\")\n    ;\n\
             return left * right\n;\n",
    );
    let messages = project
        .failure_contract_result()
        .expect_err("custom return! must not discharge numeric failure");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("custom contract diagnosis");
    let DiagnosticPayload::InvalidFallibleHandling {
        reason:
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                witness, ..
            },
    } = &diagnostic.payload
    else {
        panic!("expected custom error contract reason: {diagnostic:?}");
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert!(witness.call_spans.is_empty());
    assert!(witness.origin_span.is_some());
    assert!(diagnostic.primary_span.is_some());

    let mut project = failure_project(
        "Failure = | message String |\n\
         area |left Int, right Int| -> Int, Failure!:\n\
             result = left * right catch |err|:\n\
                 return! Failure(err.message)\n    ;\n    return result\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&named_function_body(&project, &ast, "area")[0]);
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
    assert!(expression.failure_facts.summary.first_implicit.is_none());
    project
        .lower_ast_result(ast)
        .expect("numeric catch mapping to a custom error must lower");
}

#[test]
fn implicit_failure_catch_covers_multiply_and_add_not_only_last_operand() {
    for protected in ["left * right + offset", "(left * right) + offset"] {
        let mut project = failure_project(&format!(
            "total |left Int, right Int, offset Int| -> Int:\n\
                 value = {protected} catch then 0\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&named_function_body(&project, &ast, "total")[0]);
        let block = assert_catch_contract(
            expression,
            true,
            None,
            builtin_error_type_id(&project, &ast),
        );
        assert!(matches!(
            block.handled_value.kind,
            ExpressionKind::Runtime(_)
        ));
        assert!(block.handled_value.failure_facts.checked_numeric_operation);
        assert!(
            block
                .handled_value
                .failure_facts
                .summary
                .first_numeric
                .is_some()
        );
        assert!(
            block
                .handled_value
                .failure_facts
                .summary
                .first_private_call
                .is_none()
        );
        assert!(expression.failure_facts.summary.first_implicit.is_none());
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
            "three || -> Int:\n    value = 1 + 2 {handler}\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&named_function_body(&project, &ast, "three")[0]);
        let block = assert_catch_contract(
            expression,
            false,
            None,
            builtin_error_type_id(&project, &ast),
        );
        assert!(matches!(block.handled_value.kind, ExpressionKind::Int(3)));
        assert!(block.handled_value.failure_facts.checked_numeric_operation);
        assert!(expression.failure_facts.summary.first_implicit.is_none());

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
            "three || -> Int:\n    value Int = 1 + 2 {handler}\n    return value\n;\n"
        ));
        let messages = project
            .ast_result()
            .err()
            .expect("an unreachable handler must still satisfy its receiving type");
        let diagnostic = messages
            .error_diagnostics()
            .next()
            .expect("handler type mismatch");
        assert_eq!(diagnostic.kind.code(), "MOTH-TYPE-0001");
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch { expected, found, .. }
                if expected == builtin_type_ids::INT && found == builtin_type_ids::STRING
        ));
        assert!(diagnostic.primary_span.is_some());
    }
}

#[test]
fn implicit_failure_hir_two_numeric_operations_sequence_success_and_share_one_handler() {
    let mut project = failure_project(
        "total |left Int, right Int, offset Int| -> Int:\n\
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
        .flat_map(|block| &block.statements)
        .filter(|statement| matches!(statement.kind, HirStatementKind::NumericOp { .. }))
        .count();
    assert_eq!(
        numeric_count, 2,
        "each protected operation must execute exactly once"
    );

    let (multiply, multiply_carrier) = numeric_producer(&hir, NumericOperator::Multiply);
    let (add, add_carrier) = numeric_producer(&hir, NumericOperator::Add);
    assert_ne!(multiply_carrier, add_carrier);
    let (multiply_success, multiply_error) = fallible_edges(&hir, multiply, multiply_carrier);
    let (add_success, add_error) = fallible_edges(&hir, add, add_carrier);
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
            HirTerminator::FallibleBranch {
                success_block,
                error_block,
                ..
            } => *success_block == add.id || *error_block == add.id,
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

    let (handler, error_local) = error_adapter(&hir, multiply_error, multiply_carrier);
    assert_eq!(
        error_adapter(&hir, add_error, add_carrier),
        (handler, error_local)
    );
    assert_ne!(handler, multiply.id);
    assert_ne!(handler, add.id);
    assert_ne!(error_local, multiply_carrier);
    assert_ne!(error_local, add_carrier);

    let add_left = add
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                operands: HirNumericOperands::Binary { left, .. },
                ..
            } => Some(left),
            _ => None,
        })
        .expect("addition must consume the multiplication success value");
    let add_left = hir.expressions.expression(*add_left);
    let HirExpressionKind::FallibleUnwrapSuccess { result } = &add_left.kind else {
        panic!("the first success edge must unwrap multiplication before adding");
    };
    assert_eq!(loaded_local(&hir, result), multiply_carrier);
    assert_eq!(add_left.ty, builtin_type_ids::INT);

    let success = hir_block(&hir, add_success);
    let (success_local, success_value) = success
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } if matches!(
                hir.expressions.expression(*value).kind,
                HirExpressionKind::FallibleUnwrapSuccess { .. }
            ) =>
            {
                Some((*local, value))
            }
            _ => None,
        })
        .expect("the final success edge must initialise the catch success slot");
    let success_value = hir.expressions.expression(*success_value);
    let HirExpressionKind::FallibleUnwrapSuccess { result } = &success_value.kind else {
        panic!("final success assignment must unwrap the addition carrier");
    };
    assert_eq!(loaded_local(&hir, result), add_carrier);
    assert_eq!(success_value.ty, builtin_type_ids::INT);
    assert_ne!(success_local, error_local);
    assert_ne!(success_local, multiply_carrier);
    assert_ne!(success_local, add_carrier);

    let handler_block = hir_block(&hir, handler);
    assert!(
        handler_block.statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
                if *local == success_local && matches!(hir.expressions.expression(*value).kind, HirExpressionKind::Int(0))
        )),
        "recovery must initialise the same success slot without reading a failed success payload"
    );
    let HirTerminator::Jump {
        target: success_merge,
        ..
    } = &success.terminator
    else {
        panic!("successful evaluation must reach the catch merge");
    };
    let HirTerminator::Jump {
        target: recovery_merge,
        ..
    } = &handler_block.terminator
    else {
        panic!("recovery must reach the catch merge");
    };
    assert_eq!(success_merge, recovery_merge);

    for (local, expected_type) in [
        (success_local, builtin_type_ids::INT),
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
    for (producer, carrier) in [(multiply, multiply_carrier), (add, add_carrier)] {
        let declaration = hir
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|declaration| declaration.id == carrier)
            .expect("numeric carrier must be declared");
        assert_ne!(declaration.ty, builtin_type_ids::INT);
        assert_ne!(declaration.ty, error_type);
        let HirTerminator::FallibleBranch { result, .. } = &producer.terminator else {
            panic!("numeric carrier must be tested before any slot is defined");
        };
        assert_eq!(hir.expressions.expression(*result).ty, declaration.ty);
    }
    for adapter_id in [multiply_error, add_error] {
        let adapter = hir_block(&hir, adapter_id);
        assert!(
            adapter.statements.iter().all(|statement| matches!(
                &statement.kind,
                HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
                    if *local == error_local
                        && hir.expressions.expression(*value).ty == error_type
                        && matches!(hir.expressions.expression(*value).kind, HirExpressionKind::FallibleUnwrapError { .. })
            )),
            "error edges must define only the Error slot, never the success slot"
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
                if matches!(hir.expressions.expression(*value).kind, HirExpressionKind::FallibleUnwrapSuccess { .. })
        )),
        "the handler must never consume a failed producer's success payload"
    );
}

#[test]
fn implicit_failure_nested_handler_catch_uses_its_own_error_continuation() {
    let mut project = failure_project(
        "recover |left Int, right Int| -> Int:\n\
             value = left * right catch:\n\
                 recovered = left // right catch then 7\n        then recovered\n\
             ;\n    return value\n;\n",
    );
    let hir = project.hir();
    let (multiply, multiply_carrier) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, multiply_error) = fallible_edges(&hir, multiply, multiply_carrier);
    let (outer_handler, outer_error_slot) = error_adapter(&hir, multiply_error, multiply_carrier);
    let (divide, divide_carrier) = numeric_producer(&hir, NumericOperator::IntegerDivide);
    assert_eq!(divide.id, outer_handler);
    let (_, divide_error) = fallible_edges(&hir, divide, divide_carrier);
    let (inner_handler, inner_error_slot) = error_adapter(&hir, divide_error, divide_carrier);
    assert_ne!(
        inner_handler, outer_handler,
        "nested handler failure must not restart outer recovery"
    );
    assert_ne!(
        inner_error_slot, outer_error_slot,
        "nested catches must not overwrite outer error slots"
    );
    assert!(hir_block(&hir, inner_handler).statements.iter().any(|statement| matches!(
        &statement.kind,
        HirStatementKind::Write { value, .. } if matches!(hir.expressions.expression(*value).kind, HirExpressionKind::Int(7))
    )));
}

#[test]
fn implicit_failure_catch_covers_private_operands_arguments_and_typed_call() {
    for protected in [
        "multiply(left, right) + offset",
        "load(multiply(left, right) + offset)",
        "load(left * right + offset)",
    ] {
        let mut project = failure_project(&format!(
            "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
             load |value Int| -> Int, Error!:\n    return value\n;\n\
             total |left Int, right Int, offset Int| -> Int:\n\
                 value = {protected} catch then 0\n    return value\n;\n"
        ));
        let ast = project.ast();
        let error_type_id = builtin_error_type_id(&project, &ast);
        let expression = initializer(&named_function_body(&project, &ast, "total")[0]);
        let typed_error = protected.starts_with("load(").then_some(error_type_id);
        let block = assert_catch_contract(expression, true, typed_error, error_type_id);
        if protected.contains("multiply(") {
            assert!(
                block
                    .handled_value
                    .failure_facts
                    .summary
                    .first_private_call
                    .is_some()
            );
        }
        assert!(expression.failure_facts.summary.first_implicit.is_none());
        assert!(expression.failure_facts.summary.first_typed.is_none());
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
        "Box = | value Int |\n\
         make |value Int| -> Box, Error!:\n    return Box(value)\n;\n\
         amount |this Box, extra Int| -> Int, Error!:\n    return extra\n;\n\
         total |left Int, right Int, offset Int| -> Int:\n\
             value = make(left * right).amount(offset * right) catch then 0\n\
             return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = initializer(&named_function_body(&project, &ast, "total")[0]);
    assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    assert!(expression.failure_facts.summary.first_implicit.is_none());
    assert!(expression.failure_facts.summary.first_typed.is_none());
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_catch_accepts_multiple_builtin_errors_without_erasing_code_zero() {
    let mut project = failure_project(
        "load |value Int| -> Int, Error!:\n\
             if value < 0:\n        return! Error(\"authored\", code = 0)\n    ;\n\
             return value\n;\n\
         total |left Int, right Int| -> Int:\n\
             value = load(left) + load(right) catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = initializer(&named_function_body(&project, &ast, "total")[0]);
    let block = assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    assert!(
        block
            .handled_value
            .failure_facts
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
    assert_eq!(error.type_id, error_type_id);
    let ExpressionKind::StructInstance(fields) = &error.kind else {
        panic!("authored Error identity and fields must be retained");
    };
    let code = fields
        .iter()
        .find(|field| {
            project
                .frontend
                .path_fork
                .component(field.id)
                .is_some_and(|component| project.frontend.string_table.resolve(component) == "code")
        })
        .expect("Error.code field");
    assert!(matches!(
        code.value.kind,
        ExpressionKind::FixedScalar(value)
            if Some(value) == FixedScalarValue::unsigned(FixedScalar::U32, 0)
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
    let expression = initializer(&named_function_body(&project, &ast, "recover")[0]);
    let block = catch_block(expression);
    let error_type_id = block
        .handled_value
        .failure_facts
        .summary
        .first_typed
        .expect("custom typed producer")
        .error_type_id;
    assert_ne!(error_type_id, builtin_error_type_id(&project, &ast));
    let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
    assert!(!block.handled_value.failure_facts.checked_numeric_operation);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("custom recovery handler");
    };
    assert_eq!(initializer(&body[0]).type_id, error_type_id);
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
    let expression = initializer(&named_function_body(&project, &ast, "recover")[0]);
    let block = catch_block(expression);
    let error_type_id = block
        .handled_value
        .failure_facts
        .summary
        .first_typed
        .expect("custom typed producer")
        .error_type_id;
    assert_ne!(error_type_id, builtin_error_type_id(&project, &ast));
    let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("custom recovery handler");
    };
    assert_eq!(initializer(&body[0]).type_id, error_type_id);
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_custom_catch_projects_private_compatibility_checks_into_hir() {
    for protected in ["multiply(left, right)", "forward(left, right)"] {
        let mut project = failure_project(&format!(
            "Failure = | message String |\n\
             multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
             forward |left Int, right Int| -> Int:\n    return multiply(left, right)\n;\n\
             load |value Int| -> Int, Failure!:\n    return value\n;\n\
             recover |left Int, right Int| -> Int:\n\
                 return load({protected}) catch then 0\n;\n"
        ));
        let ast = project.ast();
        let expression = returned_value(&named_function_body(&project, &ast, "recover")[0]);
        let block = catch_block(expression);
        let check = block
            .handled_value
            .failure_facts
            .deferred_custom_catch
            .expect("private call compatibility must be retained for exact callee convergence");
        let error_type_id = block
            .handled_value
            .failure_facts
            .summary
            .first_typed
            .expect("custom typed producer")
            .error_type_id;
        assert_eq!(check.error_type_id, error_type_id);
        assert!(check.catch_span.is_some());
        assert!(check.typed_producer_span.is_some());
        let private_call_span = block
            .handled_value
            .failure_facts
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
         helper_a |value Int| -> Int:\n    return value + 1\n;\n\
         helper_b |value Int| -> Int:\n    return value + 1\n;\n\
         load |value Int| -> Int, Failure!:\n    return value\n;\n\
         recover |value Int| -> Int:\n\
             return load(helper_a(value)) catch:\n\
                 inner = load(helper_b(value)) catch then 0\n\
                 then inner\n    ;\n;\n",
    );
    let ast = project.ast();
    let outer = catch_block(returned_value(
        &named_function_body(&project, &ast, "recover")[0],
    ));
    let FallibleHandling::Handler { body, .. } = &outer.handler else {
        panic!("outer recovery must retain its handler");
    };
    let inner = catch_block(initializer(&body[0]));
    let inner_check = inner
        .handled_value
        .failure_facts
        .deferred_custom_catch
        .expect("inner custom catch must defer private compatibility");
    let outer_check = outer
        .handled_value
        .failure_facts
        .deferred_custom_catch
        .expect("outer custom catch must defer private compatibility");
    let inner_private_span = inner
        .handled_value
        .failure_facts
        .summary
        .first_private_call
        .expect("inner helper call must retain its authored span")
        .span;
    let outer_private_span = outer
        .handled_value
        .failure_facts
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
    )
    .expect("both private helpers must converge before catch compatibility is diagnosed");
    let diagnostic =
        crate::compiler_frontend::module_compilation::generated::builtin_failure_diagnostic(
            &hir, &report, recover.id,
        )
        .expect("custom catch compatibility must produce a semantic diagnosis")
        .expect("the failing inner helper must make its custom catch incompatible");
    assert_eq!(diagnostic.primary_span, inner_check.catch_span);
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                error_type_id, typed_producer_span, implicit_producer_span,
            },
        } if error_type_id == inner_check.error_type_id
            && typed_producer_span == inner_check.typed_producer_span
            && implicit_producer_span == inner_private_span
    ));
}

#[test]
fn implicit_failure_catch_rejects_custom_numeric_and_distinct_typed_errors() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         load |value Int| -> Int, Failure!:\n    return value\n;\n\
         recover |left Int, right Int| -> Int:\n\
             return load(left) * right catch then 0\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("custom plus implicit cannot share a handler");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("compatibility diagnosis");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                typed_producer_span: Some(_),
                implicit_producer_span: Some(_),
                ..
            },
        }
    ));
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");

    for second_type in ["Other", "Error"] {
        let mut project = failure_project(&format!(
            "Failure = | message String |\nOther = | message String |\n\
             first |value Bool| -> Bool, Failure!:\n    return value\n;\n\
             second |value Bool| -> Bool, {second_type}!:\n    return value\n;\n\
             recover |left Bool, right Bool| -> Bool:\n\
                 return first(left) and second(right) catch then false\n;\n"
        ));
        let messages = project
            .ast_result()
            .err()
            .expect("distinct typed errors cannot share a handler");
        let diagnostic = messages
            .error_diagnostics()
            .next()
            .expect("compatibility diagnosis");
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidFallibleHandling {
                reason: InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                    first_error_type_id, second_error_type_id,
                    first_producer_span: Some(_), second_producer_span: Some(_),
                },
            } if first_error_type_id != second_error_type_id
        ));
        assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");
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
    let expression = initializer(&named_function_body(&project, &ast, "narrow")[0]);
    // Builtin arithmetic and conversion contribute implicit failure. The handler binds Error
    // without manufacturing a declared typed producer, and protects both operations.
    let block = assert_catch_contract(expression, true, None, error_type_id);
    let ExpressionKind::Cast(cast) = &block.handled_value.kind else {
        panic!("cast must own the complete arithmetic operand");
    };
    let u32_type = builtin_type_ids::fixed_scalar(FixedScalar::U32);
    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    assert_eq!(cast.source.type_id, u32_type);
    assert_eq!(cast.source_type_id, u32_type);
    assert_eq!(cast.target_type_id, u8_type);
    assert_eq!(expression.type_id, u8_type);
    assert!(cast.source.failure_facts.checked_numeric_operation);
    assert!(expression.failure_facts.summary.first_implicit.is_none());
    project
        .lower_ast_result(ast)
        .expect("cast catch must cover numeric operands and conversion");

    let mut project = failure_project(
        "widen |left Int, right Int| -> Float:\n\
             value Float = cast left * right catch then 0.0\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&named_function_body(&project, &ast, "widen")[0]);
    let block = assert_catch_contract(
        expression,
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    let ExpressionKind::Cast(cast) = &block.handled_value.kind else {
        panic!("infallible conversion must retain its pending numeric operand");
    };
    assert_eq!(cast.source_type_id, builtin_type_ids::INT);
    assert_eq!(cast.target_type_id, builtin_type_ids::FLOAT);
    let hir = project
        .lower_ast_result(ast)
        .expect("infallible cast must still recover operand failures");
    let (multiply, carrier) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, error) = fallible_edges(&hir, multiply, carrier);
    error_adapter(&hir, error, carrier);

    let mut project =
        failure_project("bad |values {Int}| -> Int:\n    return cast values catch then 0\n;\n");
    let messages = project
        .ast_result()
        .err()
        .expect("a handler must not manufacture cast evidence");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("missing cast evidence");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidCast {
            reason: InvalidCastReason::NoEvidence,
            ..
        }
    ));
    assert!(diagnostic.primary_span.is_some());
}

#[test]
fn implicit_failure_bound_catch_has_error_fields_unbound_catch_has_no_binding() {
    let mut project = failure_project(
        "bound |left Int, right Int| -> Int:\n\
             value = left * right catch |err|:\n\
                 saved Error = err\n        code U32 = err.code\n        message String = err.message\n\
                 then 0\n    ;\n    return value\n;\n\
         unbound |left Int, right Int| -> Int:\n\
             value = left * right catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let bound = initializer(&named_function_body(&project, &ast, "bound")[0]);
    let block = assert_catch_contract(bound, true, None, error_type_id);
    let FallibleHandling::Handler { error, body } = &block.handler else {
        panic!("bound handler");
    };
    assert!(error.is_some());
    assert_eq!(initializer(&body[0]).type_id, error_type_id);
    assert_eq!(
        initializer(&body[1]).type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
    );
    assert_eq!(initializer(&body[2]).type_id, builtin_type_ids::STRING);

    let unbound = initializer(&named_function_body(&project, &ast, "unbound")[0]);
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
            "recover |left Int, right Int, choose Bool| -> Int:\n\
                 value = left * right catch:\n        {handler}\n    ;\n    return value\n;\n"
        ));
        let ast = project.ast();
        let expression = initializer(&named_function_body(&project, &ast, "recover")[0]);
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
        "recover |left Int, right Int| -> Int:\n\
             value = left * right catch:\n        io.line(\"not a fallback\")\n    ;\n\
             return value\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a success receiver needs then or termination");
    assert_fallible_reason(
        &messages,
        InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
    );

    let mut project = failure_project(
        "recover |left Int, right Int| -> Int:\n\
             return left * right catch then 0, 1\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("numeric success has exactly one slot");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("arity diagnosis");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::TooManyReturnValues { expected_count: 1 },
        }
    ));

    let mut project = failure_project(
        "recover |left Int, right Int| -> Int:\n\
             value ~= 0\n    value = left * right catch then value\n    return value\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("the success target is unavailable in recovery");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("success target diagnosis");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidAssignmentTarget {
            reason: InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            ..
        }
    ));
}

#[test]
fn implicit_failure_multi_success_catch_preserves_slot_order_and_required_arity() {
    let mut project = failure_project(
        "pair |value Int| -> Int, String, Error!:\n    return value, \"ok\"\n;\n\
         recover |left Int, right Int| -> String:\n\
             number, label = pair(left * right) catch then 0, \"fallback\"\n    return label\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let NodeKind::MultiBind { targets, value } = &body[0].kind else {
        panic!("multi-success receiver must remain multi-bind");
    };
    let block = assert_catch_contract(
        value,
        true,
        Some(builtin_error_type_id(&project, &ast)),
        builtin_error_type_id(&project, &ast),
    );
    assert_eq!(
        block.result_type_ids,
        [builtin_type_ids::INT, builtin_type_ids::STRING]
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
            .map(|value| value.type_id)
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    project
        .lower_ast_result(ast)
        .expect("multi-success typed recovery must sequence numeric arguments");

    let mut project = failure_project(
        "pair |value Int| -> Int, String, Error!:\n    return value, \"ok\"\n;\n\
         recover |left Int, right Int| -> String:\n\
             number, label = pair(left * right) catch then 0\n    return label\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("multi-success fallback cannot leave a slot uninitialized");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("missing fallback slot");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidReturnShape {
            reason: InvalidReturnShapeReason::TooFewReturnValues {
                expected_count: 2,
                provided_count: 1
            },
        }
    ));
}

#[test]
fn implicit_failure_private_multi_success_catch_preserves_slot_order() {
    let mut project = failure_project(
        "pair |left Int, right Int| -> Int, String:\n    return left * right, \"ok\"\n;\n\
         recover |left Int, right Int| -> String:\n\
             number, label = pair(left, right) catch then 0, \"fallback\"\n    return label\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let NodeKind::MultiBind { targets, value } = &body[0].kind else {
        panic!("private multi-success catch must remain multi-bind");
    };
    let block = assert_catch_contract(value, true, None, builtin_error_type_id(&project, &ast));
    assert_eq!(
        block.result_type_ids,
        [builtin_type_ids::INT, builtin_type_ids::STRING]
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
            .map(|value| value.type_id)
            .collect::<Vec<_>>(),
        block.result_type_ids
    );
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_in_handler_goes_outward_not_back_into_its_own_catch() {
    let mut project = failure_project(
        "recover |left Int, right Int| -> Int, Error!:\n\
             value = left * right catch:\n        then left // right\n    ;\n    return value\n;\n",
    );
    let ast = project.ast();
    let expression = initializer(&named_function_body(&project, &ast, "recover")[0]);
    let block = assert_catch_contract(
        expression,
        true,
        None,
        builtin_error_type_id(&project, &ast),
    );
    assert!(
        block
            .handled_value
            .failure_facts
            .summary
            .first_numeric
            .is_some()
    );
    let handler_span = expression
        .failure_facts
        .summary
        .first_numeric
        .expect("handler arithmetic must remain pending")
        .span;
    assert!(handler_span.is_some());
    assert_eq!(
        expression
            .failure_facts
            .summary
            .first_implicit
            .unwrap()
            .span,
        handler_span
    );
    assert_eq!(
        expression.failure_facts.disposition,
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
    let (multiply_block, multiply_carrier) = numeric_producer(&hir, NumericOperator::Multiply);
    let (_, multiply_error) = fallible_edges(&hir, multiply_block, multiply_carrier);
    let (handler, _) = error_adapter(&hir, multiply_error, multiply_carrier);
    let (divide_block, divide_carrier) = numeric_producer(&hir, NumericOperator::IntegerDivide);
    assert_eq!(
        divide_block.id, handler,
        "handler arithmetic must execute inside the handler"
    );
    let (_, divide_error) = fallible_edges(&hir, divide_block, divide_carrier);
    let error_block = hir_block(&hir, divide_error);
    let HirTerminator::ReturnError(error) = &error_block.terminator else {
        panic!("handler failure must return outward, not re-enter its own handler");
    };
    assert!(matches!(
        &hir.expressions.expression(*error).kind,
        HirExpressionKind::FallibleUnwrapError { result } if loaded_local(&hir, result) == divide_carrier
    ));

    let mut project = failure_project(
        "Failure = | message String |\n\
         load |value Int| -> Int, Error!:\n    return value\n;\n\
         recover |left Int, right Int| -> Int, Failure!:\n\
             value = load(left) catch:\n        then left // right\n    ;\n    return value\n;\n",
    );
    let messages = project
        .failure_contract_result()
        .expect_err("handler failure still violates a custom contract");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("outward failure diagnosis");
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                witness, ..
            },
        } if witness.origin_span.is_some()
    ));
}

#[test]
fn implicit_failure_catch_keeps_comma_and_logical_line_boundaries() {
    let mut project = failure_project(
        "recover |left Int, right Int| -> Int, Int:\n\
             earlier = left * right\n\
             later = left + right catch then 0\n\
             return left * right, left + right catch:\n        then 0\n    ;\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "recover");
    let error_type_id = builtin_error_type_id(&project, &ast);
    let earlier = initializer(&body[0]);
    assert_eq!(
        earlier.failure_facts.disposition,
        FailureDisposition::Pending
    );
    assert!(earlier.failure_facts.summary.first_numeric.is_some());
    let later = initializer(&body[1]);
    assert_catch_contract(later, true, None, error_type_id);
    assert!(later.failure_facts.summary.first_implicit.is_none());
    let NodeKind::Return(values) = &body[2].kind else {
        panic!("expected two independent success slots");
    };
    assert_eq!(values.len(), 2);
    assert_eq!(
        values[0].failure_facts.disposition,
        FailureDisposition::Pending
    );
    assert!(values[0].failure_facts.summary.first_numeric.is_some());
    assert_catch_contract(&values[1], true, None, error_type_id);
    assert!(values[1].failure_facts.summary.first_implicit.is_none());
    project
        .lower_ast_result(ast)
        .expect("independently delimited numeric catches must lower");
}

#[test]
fn implicit_failure_catch_rejects_inner_argument_group_condition_template_and_constant() {
    let sources = [
        "consume |value Int| -> Int:\n    return value\n;\n\
         bad |left Int, right Int| -> Int:\n\
             return consume(left * right catch then 0)\n;\n",
        "bad |left Int, right Int| -> Int:\n\
             return (left * right catch then 0) + right\n;\n",
        "bad |left Int, right Int| -> Int:\n\
             if left * right > 0 catch then false:\n        return 1\n    else\n        return 0\n    ;\n;\n",
        "bad |left Int, right Int| -> String:\n\
             return [:value=[left * right catch then 0]]\n;\n",
        "value #= 1 * 2 catch then 0\n",
    ];
    for source in sources {
        let mut project = failure_project(source);
        let messages = project
            .ast_result()
            .err()
            .expect("catch remains a closed-receiver surface");
        assert_fallible_reason(
            &messages,
            InvalidFallibleHandlingReason::CatchOutsideBoundary,
        );
    }

    let mut project = failure_project(
        "bad |left Int, right Int| -> Int:\n\
             return left * right catch then\n        0\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("inline catch cannot cross a logical line");
    assert_fallible_reason(
        &messages,
        InvalidFallibleHandlingReason::InlineCatchMultiline,
    );
}

#[test]
fn implicit_failure_catch_remains_ineligible_for_literal_or_infallible_call() {
    for prefix in ["", "export:\n"] {
        let closing_export = if prefix.is_empty() { "" } else { ";\n" };
        for expression in ["3", "identity(value)"] {
            let mut project = failure_project(&format!(
                "{prefix}    identity |value Int| -> Int:\n        return value\n    ;\n{closing_export}\
                 bad |value Int| -> Int:\n    return {expression} catch then 0\n;\n"
            ));
            let messages = project
                .ast_result()
                .err()
                .expect("ordinary infallible values do not make catch eligible");
            assert_fallible_reason(&messages, InvalidFallibleHandlingReason::CatchOnNonFallible);
        }
    }
}

#[test]
fn implicit_failure_catch_preserves_propagation_and_optional_conflicts() {
    let mut project = failure_project(
        "load |value Int| -> Int, Error!:\n    return value\n;\n\
         bad |value Int| -> Int, Error!:\n    return load(value)! catch then 0\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("propagation and catch conflict on the same expression");
    assert_fallible_reason(
        &messages,
        InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict,
    );

    let mut project = failure_project(
        "maybe |value Int| -> Int?:\n    return value\n;\n\
         bad |value Int| -> Int?:\n    return maybe(value)? catch then 0\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("option propagation must not target catch");
    assert_fallible_reason(
        &messages,
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
        let messages = project
            .ast_result()
            .err()
            .expect("cast operand propagation must diagnose before debug finalization");
        assert_fallible_reason(&messages, expected_reason);
    }
}

#[test]
fn implicit_failure_compound_rhs_catch_does_not_consume_arithmetic_or_narrowing() {
    let mut project = failure_project(
        "load |amount U8| -> U8, Error!:\n    return amount\n;\n\
         update |amount U8| -> U8:\n\
             value ~U8 = 200\n    value += load(amount) catch then 0\n    return value\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "update");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    let ExpressionKind::Cast(conversion) = &value.kind else {
        panic!("narrow write-back requires a distinct checked conversion");
    };
    assert!(matches!(conversion.handling, CastHandling::StoreConversion));
    assert_eq!(
        conversion.target_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_eq!(
        conversion.source_type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_eq!(value.failure_facts.disposition, FailureDisposition::Pending);
    assert!(value.failure_facts.summary.first_typed.is_none());
    assert!(value.failure_facts.summary.first_numeric.is_some());
    assert!(value.failure_facts.checked_numeric_operation);
    assert!(conversion.source.failure_facts.checked_numeric_operation);
    assert_eq!(
        conversion.source.failure_facts.disposition,
        FailureDisposition::Pending
    );
    let ExpressionKind::Runtime(rpn) = &conversion.source.kind else {
        panic!("promoted arithmetic must be separate from protected RHS");
    };
    let rhs = rpn
        .items
        .iter()
        .find_map(|item| match item {
            ExpressionRpnItem::Operand(operand)
                if matches!(operand.kind, ExpressionKind::ValueBlock { .. }) =>
            {
                Some(operand)
            }
            _ => None,
        })
        .expect("compound arithmetic must retain one RHS catch operand");
    let error_type_id = builtin_error_type_id(&project, &ast);
    assert_catch_contract(rhs, false, Some(error_type_id), error_type_id);
    assert!(rhs.failure_facts.summary.first_implicit.is_none());
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
         safe_product |left Int, right Int| -> Int, Failure!:\n    return left * right\n;\n\
         recover |left Int, right Int| -> Int:\n\
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
    let expression = initializer(&named_function_body(&project, &ast, "recover")[0]);
    let block = assert_catch_contract(expression, true, Some(error_type_id), error_type_id);
    let FallibleHandling::Handler { body, .. } = &block.handler else {
        panic!("canonical Error handler");
    };
    assert_eq!(initializer(&body[0]).type_id, error_type_id);
    assert_catch_lowering_succeeds(&mut project, ast);

    let mut project = failure_project(
        "Failure = | message String, code U32 = 0 |\n\
         safe_product |left Int, right Int| -> Int, Failure!:\n    return left * right\n;\n",
    );
    let messages = project
        .failure_contract_result()
        .expect_err("same-shaped structs do not become builtin Error");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("nominal lookalike rejection");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction { .. },
        }
    ));
}

#[test]
fn implicit_failure_error_wrapper_struct_is_not_canonical() {
    // A struct that merely wraps builtin Error keeps its nominal identity: it must not
    // receive builtin treatment in the error slot, even though its only field is an Error.
    let mut project = failure_project(
        "Wrapped = | inner Error |\n\
         safe_product |left Int, right Int| -> Int, Wrapped!:\n    return left * right\n;\n",
    );
    let messages = project
        .failure_contract_result()
        .expect_err("a struct wrapping Error is not builtin Error");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("wrapper slot rejection");
    let DiagnosticPayload::InvalidFallibleHandling {
        reason:
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                witness, ..
            },
    } = &diagnostic.payload
    else {
        panic!("expected custom error contract reason, got {diagnostic:?}");
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert!(witness.origin_span.is_some());
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");
    assert!(diagnostic.primary_span.is_some());

    // The same wrapper must not unlock builtin mixing under catch either: a Wrapped-typed
    // producer alongside implicit failure stays a custom/implicit mix.
    let mut project = failure_project(
        "Wrapped = | inner Error |\n\
         load |value Int| -> Int, Wrapped!:\n    return value\n;\n\
         recover |left Int, right Int| -> Int:\n\
             return load(left) * right catch |err|:\n\
                 saved Wrapped = err\n        then 0\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("wrapper plus implicit cannot share a handler");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("wrapper catch diagnosis");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure { .. },
        }
    ));
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");
    assert!(diagnostic.primary_span.is_some());
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
                "bad || -> Int, Error!:\n    return {expression}{suffix}\n;\n"
            ));
            let messages = project
                .ast_result()
                .err()
                .expect("static invalid arithmetic remains a source diagnostic");
            let diagnostic = messages
                .error_diagnostics()
                .next()
                .expect("static numeric diagnosis");
            assert!(matches!(
                diagnostic.payload,
                DiagnosticPayload::CompileTimeEvaluationError { reason, .. } if reason == expected_reason
            ));
            assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0053");
            assert!(diagnostic.primary_span.is_some());
        }
    }
}

#[test]
fn implicit_failure_assertion_message_rejects_inferred_call_even_when_inactive() {
    for condition in ["ready", "true"] {
        let mut project = failure_project(&format!(
            "message |left Int, right Int| -> String:\n    value = left * right\n    return [value]\n;\n\
             validate |left Int, right Int, ready Bool|:\n\
                 assert({condition}, message(left, right))\n;\n"
        ));
        let messages = project
            .failure_contract_result()
            .expect_err("assertion message cannot escape by inferred private failure");
        assert_fallible_reason(
            &messages,
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        );
    }
}

#[test]
fn implicit_failure_assertion_message_rejects_template_range_runtime_step() {
    let mut project = failure_project(
        "validate |ready Bool, step Int|:\n\
             assert(ready, [loop 0 to 10 by step |i|: x])\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("template range checks cannot escape an assertion message");
    assert_fallible_reason(
        &messages,
        InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
    );
}

#[test]
fn implicit_failure_dormant_generic_assertion_message_rejects_private_failure() {
    for error_slot in ["", " -> Error!"] {
        let mut project = failure_project(&format!(
            "message |left Int, right Int| -> String:\n    value = left * right\n    return [value]\n;\n\
             export:\n    validate type T |marker T, left Int, right Int, ready Bool|{error_slot}:\n\
                 assert(ready, message(left, right))\n    ;\n;\n"
        ));
        let messages = project
            .ast_result()
            .err()
            .expect("dormant generic assertion messages must be validated before discard");
        assert_fallible_reason(
            &messages,
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
        );
    }
}

#[test]
fn implicit_failure_prepared_recovery_lowers_catch_with_outward_assertion() {
    let mut project = failure_project(
        "message |left Int, right Int| -> String:\n    value = left * right\n    return [value]\n;\n\
         validate |left Int, right Int, ready Bool|:\n\
             text = message(left, right) catch then \"fallback\"\n    assert(ready, text)\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "validate");
    let prepared = initializer(&body[0]);
    assert_catch_contract(prepared, true, None, builtin_error_type_id(&project, &ast));
    assert!(prepared.failure_facts.summary.first_implicit.is_none());
    let NodeKind::Assert { message, .. } = &body[1].kind else {
        panic!("prepared message must remain an ordinary assertion");
    };
    assert!(message.failure_facts.summary.first_implicit.is_none());
    assert!(message.failure_facts.summary.first_typed.is_none());
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_infallible_message_keeps_ordinary_assertion() {
    let mut project = failure_project(
        "message |value Int| -> String:\n    return [value]\n;\n\
         validate |value Int, ready Bool|:\n    assert(ready, message(value))\n;\n",
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
        "export:\n    product type T |marker T, left Int, right Int| -> Int:\n\
                 return left * right\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a dormant public generic cannot publish bare numeric failure");
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("expected dormant exported boundary witness, got {diagnostic:?}");
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert!(witness.call_spans.is_empty());
    assert_eq!(witness.elided_call_hops, 0);
    let producer_span = witness
        .origin_span
        .expect("direct numeric producer has an origin");
    assert!(diagnostic.primary_span.is_some());
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(producer_span));
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");

    let mut project = failure_project(
        "Failure = | message String |\n\
         product type T |marker T, left Int, right Int| -> Int, Failure!:\n\
             return left * right\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a dormant custom-error generic must discharge numeric failure");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("dormant custom contract diagnosis");
    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::InvalidFallibleHandling {
                reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                    witness, ..
                },
            } if witness.codes == vec![BuiltinErrorCode::IntOverflow]
                && witness.origin_span.is_some()
                && witness.call_spans.is_empty()
                && witness.elided_call_hops == 0
        ),
        "expected dormant custom-error boundary reason, got {diagnostic:?}"
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");

    for (prefix, error_slot) in [("export:\n", ", Error!"), ("", "")] {
        let closing_export = if prefix.is_empty() { "" } else { ";\n" };
        let mut project = failure_project(&format!(
            "{prefix}    product type T |marker T, left Int, right Int| -> Int{error_slot}:\n\
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
fn implicit_failure_range_loop_witnesses_carry_step_guard_codes() {
    fn boundary_codes(messages: &CompilerMessages) -> Vec<BuiltinErrorCode> {
        let diagnostic = messages
            .error_diagnostics()
            .next()
            .expect("range loop boundary diagnosis");
        match &diagnostic.payload {
            DiagnosticPayload::InvalidFallibleHandling {
                reason:
                    InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness }
                    | InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                        witness,
                        ..
                    },
            } => {
                assert!(witness.origin_span.is_some());
                witness.codes.clone()
            }
            _ => panic!("expected range loop boundary witness, got {diagnostic:?}"),
        }
    }

    let int_codes = vec![
        BuiltinErrorCode::IntOverflow,
        BuiltinErrorCode::InvalidRangeStep,
    ];
    let float_codes = vec![
        BuiltinErrorCode::FloatNonFinite,
        BuiltinErrorCode::InvalidRangeStep,
        BuiltinErrorCode::RangeStepNoProgress,
    ];
    for (scalar, end, zero, expected) in [
        ("Int", "10", "0", &int_codes),
        ("Float", "10.0", "0.0", &float_codes),
    ] {
        // Converged boundary: a custom error slot sees the HIR-projected contributor.
        let mut project = failure_project(&format!(
            "Failure = | message String |\n\
             walk |start {scalar}, step {scalar}| -> {scalar}, Failure!:\n\
                 loop start to {end} by step |value|:\n        return value\n    ;\n\
                 return {zero}\n;\n"
        ));
        let messages = project
            .failure_contract_result()
            .expect_err("custom contract must discharge range step failure");
        assert_eq!(&boundary_codes(&messages), expected, "converged {scalar}");

        // Dormant generic boundary: the finalizer reads the AST contributor directly.
        let mut project = failure_project(&format!(
            "export:\n    walk type T |marker T, start {scalar}, step {scalar}| -> {scalar}:\n\
                     loop start to {end} by step |value|:\n            return value\n        ;\n\
                     return {zero}\n    ;\n;\n"
        ));
        let messages = project
            .ast_result()
            .err()
            .expect("a dormant public generic cannot publish range step failure");
        assert_eq!(&boundary_codes(&messages), expected, "dormant {scalar}");
    }
}

#[test]
fn implicit_failure_dormant_complete_recursive_graph_without_numeric_origin_is_infallible() {
    let mut project = failure_project(
        "identity |value Int| -> Int:\n    return identity(value)\n;\n\
         export:\n    wrapper type T |marker T, value Int| -> Int:\n\
                 return identity(value)\n    ;\n;\n",
    );
    // Match concrete HIR convergence: stack exhaustion is fatal, not numeric failure.
    project
        .ast_result()
        .expect("a complete origin-free recursive graph is infallible");
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
            let ast = project
                .ast_result()
                .expect("closed private generic forwarding is infallible");
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
         multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
         private_product type U |marker U, left Int, right Int| -> Int:\n\
             return multiply(left, right)\n;\n\
         export:\n    product type T |marker T, left Int, right Int| -> Int:\n\
                 retained = identity(marker)\n        return private_product(retained, left, right)\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a real numeric failure cannot escape a dormant export");
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostics[0].payload
    else {
        panic!(
            "expected dormant numeric-origin witness, got {:?}",
            diagnostics[0]
        );
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert!(witness.origin_span.is_some());
    assert_eq!(witness.call_spans.len(), 2);
}

#[test]
fn implicit_failure_dormant_recursive_chain_with_numeric_origin_is_rejected() {
    let mut project = failure_project(
        "recursive |value Int, ready Bool| -> Int:\n\
             if ready:\n        return value * 2\n    ;\n    return recursive(value, ready)\n;\n\
         export:\n    wrapper type T |marker T, value Int, ready Bool| -> Int:\n\
                 return recursive(value, ready)\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a recursive graph still carries numeric origins");
    assert!(matches!(
        messages
            .error_diagnostics()
            .next()
            .expect("numeric boundary diagnostic")
            .payload,
        DiagnosticPayload::InvalidFallibleHandling {
            reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { .. },
        }
    ));
}

/// Extract the failure witness behind one boundary rejection.
///
/// WHAT: returns the structured witness of the single `MOTH-RULE-0051` boundary diagnostic,
///       whether it escapes through an exported slot or a custom error slot.
/// WHY: the recursive regression needs the codes, origin span and call hops themselves rather
///      than only the rendered message, and every boundary lane shares this shape.
fn boundary_failure_witness(
    messages: &CompilerMessages,
) -> crate::compiler_frontend::compiler_messages::BuiltinFailureWitness {
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1, "unexpected diagnostics: {messages:?}");
    let witness = match &diagnostics[0].payload {
        DiagnosticPayload::InvalidFallibleHandling {
            reason:
                InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness }
                | InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                    witness,
                    ..
                },
        } => witness,
        _ => panic!("expected builtin failure witness, got {:?}", diagnostics[0]),
    };
    assert_eq!(diagnostics[0].kind.code(), "MOTH-RULE-0051");
    // The diagnostic labels render each retained call hop plus the terminal origin, so the
    // structured witness and its labels must agree exactly.
    let mut expected_spans = witness.call_spans.clone();
    expected_spans.push(witness.origin_span);
    assert_eq!(
        diagnostics[0]
            .labels
            .iter()
            .map(|label| label.span)
            .collect::<Vec<_>>(),
        expected_spans,
        "labels must render each witness hop plus the origin"
    );
    (**witness).clone()
}

#[test]
fn implicit_failure_self_recursive_back_edge_keeps_overflow_origin_at_both_boundaries() {
    // WHAT: the recursive branch precedes the overflow producer in source order.
    // WHY: both witness walkers must skip only the back-edge and retain the available
    //      multiplication origin instead of reporting call-only hops with empty codes.
    const HELPER: &str = "helper |value Int, ready Bool| -> Int:\n\
         if ready:\n        return helper(value, ready)\n    ;\n    return value * 2\n;\n";
    // The concrete boundary exercises HIR summary convergence through a custom error slot.
    // The harness lowers plain exports without public origins, so the slot carries the
    // concrete contract here; the witness walker under test is the same HIR one.
    const FAILURE: &str = "Failure = | message String |\n";
    let mut project = failure_project(&format!(
        "{FAILURE}{HELPER}\
         boundary |value Int, ready Bool| -> Int, Failure!:\n    return helper(value, ready)\n;\n"
    ));
    let messages = project
        .failure_contract_result()
        .expect_err("escaping overflow must be rejected at the concrete boundary");
    let witness = boundary_failure_witness(&messages);
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);
    assert!(
        witness.origin_span.is_some(),
        "the retained origin points at the actual multiplication"
    );
    assert_eq!(witness.call_spans.len(), 1);
    assert_eq!(witness.elided_call_hops, 0);

    // The dormant generic boundary exercises the AST finalizer over the same helper shape.
    let mut project = failure_project(&format!(
        "{HELPER}\
         export:\n    wrapper type T |marker T, value Int, ready Bool| -> Int:\n\
                 return helper(value, ready)\n    ;\n;\n"
    ));
    let messages = project
        .ast_result()
        .err()
        .expect("escaping overflow must be rejected at the dormant generic boundary");
    let dormant = boundary_failure_witness(&messages);
    assert_eq!(dormant.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(dormant.origin, BuiltinFailureOriginKind::Operation);
    assert!(
        dormant.origin_span.is_some(),
        "the dormant lane retains the same multiplication origin"
    );
    assert_eq!(dormant.call_spans.len(), 1);
    assert_eq!(dormant.elided_call_hops, 0);

    // Repeated runs produce the same structured witness.
    let mut project = failure_project(&format!(
        "{FAILURE}{HELPER}\
         boundary |value Int, ready Bool| -> Int, Failure!:\n    return helper(value, ready)\n;\n"
    ));
    let messages = project
        .failure_contract_result()
        .expect_err("the repeated run must reject the same boundary");
    assert_eq!(boundary_failure_witness(&messages), witness);
}

#[test]
fn implicit_failure_mutual_recursive_back_edge_keeps_overflow_origin_at_both_boundaries() {
    const HELPERS: &str = "first |value Int, ready Bool| -> Int:\n\
         if ready:\n        return second(value, ready)\n    ;\n    return value * 2\n;\n\
         second |value Int, ready Bool| -> Int:\n    return first(value, ready)\n;\n";
    const FAILURE: &str = "Failure = | message String |\n";
    let mut project = failure_project(&format!(
        "{FAILURE}{HELPERS}\
         boundary |value Int, ready Bool| -> Int, Failure!:\n    return first(value, ready)\n;\n"
    ));
    let messages = project
        .failure_contract_result()
        .expect_err("escaping overflow must be rejected at the concrete boundary");
    let witness = boundary_failure_witness(&messages);
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);
    assert!(
        witness.origin_span.is_some(),
        "the retained origin points at the actual multiplication"
    );
    assert_eq!(witness.call_spans.len(), 1);
    assert_eq!(witness.elided_call_hops, 0);

    let mut project = failure_project(&format!(
        "{HELPERS}\
         export:\n    wrapper type T |marker T, value Int, ready Bool| -> Int:\n\
                 return first(value, ready)\n    ;\n;\n"
    ));
    let messages = project
        .ast_result()
        .err()
        .expect("escaping overflow must be rejected at the dormant generic boundary");
    let dormant = boundary_failure_witness(&messages);
    assert_eq!(dormant.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(dormant.origin, BuiltinFailureOriginKind::Operation);
    assert!(
        dormant.origin_span.is_some(),
        "the dormant lane retains the same multiplication origin"
    );
    assert_eq!(dormant.call_spans.len(), 1);
    assert_eq!(dormant.elided_call_hops, 0);

    let mut project = failure_project(&format!(
        "{FAILURE}{HELPERS}\
         boundary |value Int, ready Bool| -> Int, Failure!:\n    return first(value, ready)\n;\n"
    ));
    let messages = project
        .failure_contract_result()
        .expect_err("the repeated run must reject the same boundary");
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
         recurse |value Int| -> Int:\n    return recurse(value)\n;\n\
         boundary |value Int| -> Int, Failure!:\n    return recurse(value)\n;\n",
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
    project
        .ast_result()
        .expect("a declared success-only requirement cannot gain a private lane");
}

#[test]
fn implicit_failure_typed_map_catch_resolves_infallible_private_arguments_in_both_orders() {
    let helper = "key |value String| -> String:\n    return [value]\n;\n";
    let caller = "recover |value String| -> Int:\n\
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
        "key |value Int| -> Int:\n    return value * 2\n;\n\
         recover |value Int| -> Int:\n\
             values = {0 = 1}\n    return values.get(key(value)) catch then 0\n;\n",
    );
    let ast = project.ast();
    assert_catch_lowering_succeeds(&mut project, ast);
}

#[test]
fn implicit_failure_private_catch_routes_argument_failure_before_outer_call() {
    let mut project = failure_project(
        "double |value Int| -> Int:\n    return value * 2\n;\n\
         combine |left Int, right Int| -> Int:\n    return left + right\n;\n\
         recover |value Int| -> Int:\n\
             return combine(double(value), value * 1000000) catch then -1\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = returned_value(&named_function_body(&project, &ast, "recover")[0]);
    assert_catch_contract(expression, true, None, error_type_id);
    let hir = project
        .lower_ast_result(ast)
        .expect("failed private arguments must lower inside the outer catch");
    let recover = named_hir_function(&project, &hir, "recover");
    let reachable = reachable_block_ids(&hir, recover.entry);
    let argument_block = hir
        .blocks
        .iter()
        .filter(|block| reachable.contains(&block.id))
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call {
                        target: CallTarget::Local(_),
                        ..
                    }
                )
            })
        })
        .expect("the failing private argument must survive as a scalar call");
    let HirTerminator::FallibleBranch {
        result,
        error_block,
        ..
    } = &argument_block.terminator
    else {
        panic!("the private argument call must branch before the outer call consumes it");
    };
    let (handler, _) = error_adapter(&hir, *error_block, loaded_local(&hir, result));
    assert!(
        reachable.contains(&handler),
        "the argument failure must enter the shared catch handler"
    );

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
    )
    .expect("private failure summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
        if !block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::Local(_),
                    ..
                }
            )
        }) {
            continue;
        }
        let HirTerminator::FallibleBranch {
            result,
            error_block,
            ..
        } = &block.terminator
        else {
            panic!("every installed private call must branch before its success value is used");
        };
        let (error_handler, _) = error_adapter(&hir, *error_block, loaded_local(&hir, result));
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
        "double |value Int| -> Int:\n    return value * 2\n;\n\
         recover |value Int| -> Int:\n\
             return double(value) + value * 1000000 catch then -1\n;\n",
    );
    let ast = project.ast();
    let error_type_id = builtin_error_type_id(&project, &ast);
    let expression = returned_value(&named_function_body(&project, &ast, "recover")[0]);
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
            HirTerminator::FallibleBranch { .. }
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
    )
    .expect("private failure summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
        if !block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::Local(_),
                    ..
                }
            )
        }) {
            continue;
        }
        let HirTerminator::FallibleBranch {
            result,
            error_block,
            ..
        } = &block.terminator
        else {
            panic!("every installed private call must branch before its success value is used");
        };
        let (error_handler, _) = error_adapter(&hir, *error_block, loaded_local(&hir, result));
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
    let mut project = failure_project(
        "risky |amount U8| -> U8, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         export:\n    bump type T |marker T, extra U8| -> U8:\n\
                 total ~U8 = 250\n        total += risky(extra) catch then extra\n        return total\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("the write-back into the compound target must still escape");
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("expected exported write-back witness, got {diagnostic:?}");
    };
    // The promoted U8 arithmetic cannot overflow its wider domain, so the escaping failure is
    // the store conversion's range check into the declared target.
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntCastOutOfRange]);
    assert!(witness.call_spans.is_empty());
    let origin_span = witness
        .origin_span
        .expect("the write-back producer keeps its target span");
    let BuiltinFailureOriginKind::CompoundWriteBack { target } = witness.origin else {
        panic!(
            "expected compound write-back origin, got {:?}",
            witness.origin
        );
    };
    assert_eq!(
        target,
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "the write-back names the canonical compound target"
    );
    let mut project = failure_project(
        "risky |amount U8| -> U8, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         bump |extra U8| -> U8:\n    total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "bump");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    let ExpressionKind::Cast(conversion) = &value.kind else {
        panic!("the narrow back into the target must stay a checked conversion");
    };
    // The write-back contributor is recorded at the compound value span, which
    // the cast shares; the handled RHS catch keeps a distinct inner span.
    assert_eq!(
        value
            .failure_facts
            .implicit
            .iter()
            .find_map(|contributor| match contributor.source {
                ImplicitFailureSource::CompoundWriteBack { target, .. } =>
                    Some((contributor.span, target)),
                _ => None,
            }),
        Some((value.span, builtin_type_ids::fixed_scalar(FixedScalar::U8))),
        "the compound value owns the write-back contributor naming the target"
    );
    let ExpressionKind::Runtime(rpn) = &conversion.source.kind else {
        panic!("promoted arithmetic must be separate from protected RHS");
    };
    let rhs = rpn
        .items
        .iter()
        .find_map(|item| match item {
            ExpressionRpnItem::Operand(operand)
                if matches!(operand.kind, ExpressionKind::ValueBlock { .. }) =>
            {
                Some(operand)
            }
            _ => None,
        })
        .expect("compound arithmetic must retain one RHS catch operand");
    let rhs_span = rhs.span.expect("the handled RHS keeps its span");
    assert_ne!(
        origin_span, rhs_span,
        "the write-back origin must not point at the handled RHS"
    );
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(origin_span));
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::BuiltinWriteBackOrigin {
            target_type_id: target
        }),
        "the label names the write-back target instead of the handled arithmetic"
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0051");
}

#[test]
fn implicit_failure_uncaught_compound_update_reports_writeback_origin() {
    let mut project = failure_project(
        "export:\n    bump type T |marker T, x U8, y U8| -> U8:\n\
                 total ~U8 = x\n        total += y\n        return total\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("an unhandled compound update must be rejected");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("boundary diagnosis");
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("expected exported boundary witness, got {diagnostic:?}");
    };
    // Even without a catch, the escaping failure is the later write-back into
    // the target, not the promoted arithmetic that produced it.
    let BuiltinFailureOriginKind::CompoundWriteBack { target } = witness.origin else {
        panic!(
            "expected compound write-back origin, got {:?}",
            witness.origin
        );
    };
    assert_eq!(target, builtin_type_ids::fixed_scalar(FixedScalar::U8));
    assert!(witness.origin_span.is_some());
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::BuiltinWriteBackOrigin {
            target_type_id: target
        })
    );
    assert_eq!(diagnostic.labels[0].span, witness.origin_span);
}

#[test]
fn implicit_failure_plain_return_arithmetic_keeps_operation_origin() {
    let mut project = failure_project(
        "export:\n    bump type T |marker T, x Int, y Int| -> Int:\n\
                 return x + y\n    ;\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("unhandled arithmetic must be rejected");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("boundary diagnosis");
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("expected exported boundary witness, got {diagnostic:?}");
    };
    assert_eq!(witness.origin, BuiltinFailureOriginKind::Operation);
    assert!(witness.origin_span.is_some());
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::BuiltinFailureOrigin)
    );
    assert_eq!(diagnostic.labels[0].span, witness.origin_span);
}

#[test]
fn implicit_failure_custom_slot_keeps_writeback_origin() {
    let mut project = failure_project(
        "Failure = | message String |\n\
         risky |amount U8| -> U8, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         bump |extra U8| -> U8, Failure!:\n    total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n",
    );
    let messages = project
        .failure_contract_result()
        .expect_err("the write-back must not escape through a custom slot");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("custom slot diagnosis");
    let DiagnosticPayload::InvalidFallibleHandling {
        reason:
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                witness, ..
            },
    } = &diagnostic.payload
    else {
        panic!("expected custom-slot write-back witness, got {diagnostic:?}");
    };
    let BuiltinFailureOriginKind::CompoundWriteBack { target } = witness.origin else {
        panic!(
            "expected compound write-back origin, got {:?}",
            witness.origin
        );
    };
    assert_eq!(target, builtin_type_ids::fixed_scalar(FixedScalar::U8));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, witness.origin_span);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::BuiltinWriteBackOrigin {
            target_type_id: target
        })
    );
}
#[test]
fn implicit_failure_failed_compound_writeback_keeps_target_after_rhs_catch() {
    let mut project = failure_project(
        "risky |amount U8| -> U8, Error!:\n    wide U32 = amount + 200\n    return cast wide\n;\n\
         adjust |extra U8| -> U8:\n\
             total ~U8 = 250\n    total += risky(extra) catch then extra\n    return total\n;\n",
    );
    let ast = project.ast();
    let body = named_function_body(&project, &ast, "adjust");
    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("compound write-back must remain an assignment");
    };
    assert!(
        matches!(&value.kind, ExpressionKind::Cast(_)),
        "the narrow back into the target must stay a checked conversion outside the RHS catch"
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("RHS catch must lower while the write-back stays separate");
    let adjust = named_hir_function(&project, &hir, "adjust");
    let reachable = reachable_block_ids(&hir, adjust.entry);
    assert!(
        reachable
            .iter()
            .any(|block_id| hir.blocks[block_id.0 as usize]
                .statements
                .iter()
                .any(|statement| matches!(statement.kind, HirStatementKind::CastOp { .. }))),
        "a failing narrow back into the target must keep its checked conversion carrier"
    );

    // The RHS call targets a typed `Error!` callee, so it routes through its
    // carrier branch at lowering time and needs no private-lane record. Lane
    // installation must leave that routing untouched: no record appears, none
    // survives, and the write-back conversion keeps its own carrier.
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
        "a typed callee routes through its carrier branch, not the private lane"
    );
    let mut report = project
        .frontend
        .with_compiler(|compiler| compiler.check_borrows(&hir))
        .expect("protected RHS calls must pass borrow checking before lane installation");
    crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries(
        &hir,
        &mut report,
    )
    .expect("private failure summaries must converge");
    crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes(
        &mut hir,
        &report,
        &mut type_environment,
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
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call {
                        target: CallTarget::Local(_),
                        ..
                    }
                )
            })
        })
        .expect("the typed RHS call must survive lane installation");
    let HirTerminator::FallibleBranch {
        result,
        error_block,
        ..
    } = &typed_call.terminator
    else {
        panic!("the typed RHS call must branch into the RHS catch handler");
    };
    let (error_handler, _) = error_adapter(&hir, *error_block, loaded_local(&hir, result));
    assert!(
        reachable.contains(&error_handler),
        "the typed RHS call failure must enter the RHS catch handler"
    );
    assert!(
        reachable
            .iter()
            .any(|block_id| hir.blocks[block_id.0 as usize]
                .statements
                .iter()
                .any(|statement| matches!(statement.kind, HirStatementKind::CastOp { .. }))),
        "the write-back conversion must keep its checked carrier after installation"
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
    let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
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
    let messages = project
        .ast_result()
        .err()
        .expect("a statement boundary must not drop an argument's typed error");
    assert_fallible_reason(
        &messages,
        InvalidFallibleHandlingReason::UnhandledErrorReturn,
    );
}

#[test]
fn implicit_failure_zero_success_host_catch_lowers_pending_numeric_argument() {
    let mut project = failure_project(
        "@test/default consume\n\
         recover |left Int, right Int|:\n\
             consume(left * right) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
    );
    let host_id = Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "consume".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::NativeInt,
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![],
            error_return_type: Some(ExternalSignatureType::BuiltinError),
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("fallible host test signature must register");
    let ast = project.ast();
    let NodeKind::ExpressionStatement(expression) =
        &named_function_body(&project, &ast, "recover")[0].kind
    else {
        panic!("zero-success host recovery must remain an expression statement");
    };
    let error_type = builtin_error_type_id(&project, &ast);
    let block = assert_catch_contract(expression, true, Some(error_type), error_type);
    assert!(matches!(
        block.handled_value.kind,
        ExpressionKind::HandledFallibleHostFunctionCall { .. }
    ));
    assert!(
        block.result_type_ids.is_empty(),
        "host recovery must not invent success slots"
    );
    let hir = project
        .lower_ast_result(ast)
        .expect("numeric host arguments must support local recovery");
    let (multiply, carrier) = numeric_producer(&hir, NumericOperator::Multiply);
    let (success, error) = fallible_edges(&hir, multiply, carrier);
    let handler_slot = error_adapter(&hir, error, carrier);
    let host_block = hir_block(&hir, success);
    let host_carrier = host_block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call {
                target: CallTarget::External(id),
                result: Some(result),
                ..
            } if *id == host_id => Some(result.local()),
            _ => None,
        })
        .expect("the protected host call must execute only after its numeric argument succeeds");
    let (_, host_error) = fallible_edges(&hir, host_block, host_carrier);
    assert_eq!(
        error_adapter(&hir, host_error, host_carrier),
        handler_slot,
        "host and numeric failures must initialise the same handler Error slot"
    );
}

#[test]
fn implicit_failure_catches_keep_external_float_checks_on_function_boundary() {
    let mut project = failure_project(
        "@test/default measure\n\
         Failure = | message String |\n\
         load |value Float| -> Float, Failure!:\n    return value\n;\n\
         recover |value Float| -> Float:\n\
             result = load(measure(value)) catch then 0.0\n    return result\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::NativeFloat,
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(
                ExternalSignatureType::NativeFloat,
            )],
            error_return_type: None,
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
         load |value Float| -> Float, Error!:\n    return value\n;\n\
         recover |value Float| -> Float:\n\
             result = load(measure(value)) catch then 0.0\n    return result\n;\n",
    );
    Arc::make_mut(&mut project.frontend.external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "measure".to_owned(),
            parameters: vec![ExternalParameter {
                language_type: ExternalSignatureType::NativeFloat,
                access_kind: ExternalAccessKind::Shared,
            }],
            returns: vec![ExternalReturnSlot::fresh(
                ExternalSignatureType::NativeFloat,
            )],
            error_return_type: None,
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
            "recover |left Int, right Int| -> Int:\n\
                 items {Int} = {9}\n\
                 value = items.get(left * right) catch then 0\n    return value\n;\n",
            1,
            true,
        ),
        (
            "recover |left Int, right Int|:\n\
                 items ~{Int} = {9}\n\
                 ~items.set(0, left * right) catch:\n        io.line(\"fallback\")\n    ;\n;\n",
            1,
            false,
        ),
    ] {
        let mut project = failure_project(source);
        let ast = project.ast();
        let statement = &named_function_body(&project, &ast, "recover")[statement_index];
        let expression = if has_success {
            initializer(statement)
        } else {
            let NodeKind::ExpressionStatement(expression) = &statement.kind else {
                panic!("zero-success builtin recovery must remain an expression statement");
            };
            expression
        };
        let error_type = builtin_error_type_id(&project, &ast);
        let block = assert_catch_contract(expression, true, Some(error_type), error_type);
        assert!(matches!(
            block.handled_value.kind,
            ExpressionKind::HandledFallibleExpression { .. }
        ));
        assert_eq!(block.result_type_ids.len(), usize::from(has_success));
        let hir = project
            .lower_ast_result(ast)
            .expect("builtin catch must sequence numeric arguments");
        let (multiply, carrier) = numeric_producer(&hir, NumericOperator::Multiply);
        let (_, error) = fallible_edges(&hir, multiply, carrier);
        error_adapter(&hir, error, carrier);
    }
}

#[test]
fn implicit_failure_zero_success_receiver_and_builtin_catches_keep_typed_error_contracts() {
    for (source, statement_index) in [
        (
            "Box = | value Int |\n\
             touch |this Box| -> Error!:\n;\n\
             recover |box Box|:\n    box.touch() catch:\n        io.line(\"fallback\")\n    ;\n;\n",
            0,
        ),
        (
            "recover |value Int|:\n\
                 items ~{Int} = {0}\n\
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
        let block = assert_catch_contract(expression, false, Some(error_type_id), error_type_id);
        assert!(
            block.result_type_ids.is_empty(),
            "error-only handlers must not invent success slots"
        );
        project
            .lower_ast_result(ast)
            .expect("existing zero-success typed recovery must continue lowering");
    }

    let mut project = failure_project(
        "Box = | value Int |\n\
         touch |this Box| -> Error!:\n;\n\
         bad |box Box|:\n    box.touch()\n;\n",
    );
    let messages = project
        .ast_result()
        .err()
        .expect("a receiver statement cannot discard its typed error");
    assert_fallible_reason(
        &messages,
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
                 safe_product |left Int, right Int| -> Int, Error!:\n\
                     return multiply(left, right)\n;\n",
            ),
            (
                "src/helpers.moth",
                "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n",
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
    let value = returned_value(&named_function_body(&project, &ast, "safe_product")[0]);
    assert_eq!(value.type_id, builtin_type_ids::INT);
    assert!(matches!(value.kind, ExpressionKind::FunctionCall { name, .. } if name == helper_path));
    let private_call_span = value
        .failure_facts
        .summary
        .first_private_call
        .expect("same-module private call must retain its witness")
        .span;
    assert!(private_call_span.is_some());
    assert!(value.failure_facts.summary.first_typed.is_none());

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
    )
    .expect("canonical same-module call summary must converge");
    assert!(report.analysis.public_call_summaries[&helper.id].escapes_builtin_failure);
    assert!(!report.analysis.public_call_summaries[&caller.id].escapes_builtin_failure);
}
