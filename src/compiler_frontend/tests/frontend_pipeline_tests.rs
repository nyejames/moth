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
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderParseOptions, bind_module_headers, prepare_file_from_tokens,
    prepare_header_syntax,
};
use crate::compiler_frontend::hir::functions::{HirFunctionOrigin, HirFunctionOriginLookup};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::{
    StyleDirectiveEffects, StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::tokenize_source_for_test;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, TemplateBodyMode, TokenizerEntryMode,
};
use crate::compiler_frontend::{CompilerFrontend, FrontendBuildProfile};
use crate::projects::settings::Config;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

struct FrontendProject {
    _temp_dir: TempDir,
    project_root: PathBuf,
    entry_file: PathBuf,
    files: Vec<PathBuf>,
    logical_paths: Vec<(PathBuf, InternedPath)>,
    frontend: CompilerFrontend,
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
        let resolver = ProjectPathResolver::new(
            canonical_project_root.clone(),
            canonical_entry_root,
            crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(
            ),
            &crate::builder_surface::SourceFileKindRegistry::default(),
        )
        .expect("project path resolver should build");

        let mut string_table = StringTable::new();
        let source_files = SourceFileTable::build(
            &canonical_files,
            &entry_file,
            Some(&resolver),
            &mut string_table,
        )
        .expect("source file table should build");
        let logical_paths = canonical_files
            .iter()
            .map(|canonical_file| {
                let logical_path = source_files
                    .get_by_canonical_path(canonical_file)
                    .expect("source file identity should exist")
                    .logical_path
                    .clone();
                (canonical_file.clone(), logical_path)
            })
            .collect::<Vec<_>>();

        let mut frontend = CompilerFrontend::new(
            Config::new(canonical_project_root).frontend_options(),
            string_table,
            style_directives,
            Arc::new(crate::compiler_frontend::external_packages::ExternalPackageRegistry::new()),
            Some(resolver),
        );
        frontend.set_source_files(source_files);

        Self {
            _temp_dir: temp_dir,
            project_root,
            entry_file,
            files: canonical_files,
            logical_paths,
            frontend,
        }
    }

    fn tokenize_all(&mut self) -> Vec<FileTokens> {
        let mut tokenized_files = Vec::with_capacity(self.files.len());

        for file in &self.files {
            let source = fs::read_to_string(file).expect("should read source file");
            tokenized_files.push(
                tokenize_source_for_test(
                    &mut self.frontend,
                    &source,
                    file,
                    TokenizerEntryMode::SourceFile,
                )
                .expect("tokenization should succeed"),
            );
        }

        tokenized_files
    }

    fn logical_path(&self, relative_path: &str) -> InternedPath {
        let canonical = fs::canonicalize(self.project_root.join(relative_path))
            .expect("fixture file should canonicalize");
        self.logical_paths
            .iter()
            .find_map(|(file, logical_path)| {
                if file == &canonical {
                    Some(logical_path.clone())
                } else {
                    None
                }
            })
            .expect("logical path should exist for fixture file")
    }

    fn headers(&mut self) -> BoundModuleHeaders {
        let tokenized_files = self.tokenize_all();
        let entry_file_id = self
            .frontend
            .source_files
            .get_by_canonical_path(&self.entry_file)
            .map(|identity| identity.file_id);

        let options = HeaderParseOptions {
            entry_file_id,
            project_path_resolver: self.frontend.project_path_resolver.clone(),
            active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        };

        let mut prepared_outputs = Vec::with_capacity(tokenized_files.len());
        let mut const_template_offset = 0usize;
        let mut runtime_fragment_offset = 0usize;

        for file_tokens in tokenized_files {
            let output = prepare_file_from_tokens(
                file_tokens,
                &self.entry_file,
                &options,
                &mut self.frontend.string_table,
                const_template_offset,
                runtime_fragment_offset,
            )
            .expect("header parsing should succeed");

            const_template_offset += output.const_template_count;
            runtime_fragment_offset += output.runtime_fragment_count;
            prepared_outputs.push(output);
        }

        let prepared_syntax =
            prepare_header_syntax(prepared_outputs, &mut self.frontend.string_table)
                .expect("header syntax preparation should succeed");
        bind_module_headers(
            prepared_syntax,
            &self.frontend.external_package_registry,
            &ExternalImportResolutionTable::default(),
            &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
            options.project_path_resolver.as_ref(),
            &mut self.frontend.string_table,
        )
        .expect("header binding should succeed")
    }

    fn sorted_headers(&mut self) -> crate::compiler_frontend::module_dependencies::SortedHeaders {
        let headers = self.headers();
        self.frontend
            .sort_headers(headers)
            .expect("header sorting should succeed")
    }

    fn ast(&mut self) -> crate::compiler_frontend::ast::Ast {
        let sorted = self.sorted_headers();
        self.frontend
            .headers_to_ast(
                sorted,
                &self.entry_file,
                ModuleRootRole::Normal,
                FrontendBuildProfile::Dev,
                Default::default(),
                #[cfg(feature = "timers")]
                None,
            )
            .expect("AST construction should succeed")
            .ast
    }

    fn hir(&mut self) -> crate::compiler_frontend::hir::module::HirModule {
        let ast = self.ast();
        self.frontend
            .generate_hir(ast, HirFunctionOriginLookup::default())
            .expect("HIR lowering should succeed")
            .hir_module
    }

    /// Lower to HIR and borrow-check it, returning both so tests can assert the exact
    /// relationship between the module and the facts derived from it.
    fn borrow_checked_hir(&mut self) -> (HirModule, BorrowCheckReport) {
        let hir = self.hir();
        let report = self
            .frontend
            .check_borrows(&hir)
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
fn frontend_diagnostics_preserve_string_table_context() {
    let mut project = FrontendProject::new(
        &[("src/@page.moth", "bad #= io.line(\"runtime host call\")\n")],
        "src/@page.moth",
        StyleDirectiveRegistry::built_ins(),
    );

    let sorted = project.sorted_headers();
    let Err(messages) = project.frontend.headers_to_ast(
        sorted,
        &project.entry_file,
        ModuleRootRole::Normal,
        FrontendBuildProfile::Dev,
        Default::default(),
        #[cfg(feature = "timers")]
        None,
    ) else {
        panic!("const host calls should fail during AST construction");
    };

    let first_diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("AST construction should return a diagnostic");
    let resolved_scope = first_diagnostic
        .primary_location
        .scope
        .to_portable_string(&messages.string_table);
    let expected_scope = project
        .logical_path("src/@page.moth")
        .to_portable_string(&messages.string_table);
    assert!(
        resolved_scope == expected_scope,
        "AST errors should preserve the logical source path in the returned StringTable, expected '{expected_scope}', got '{resolved_scope}'",
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
        .check_borrows(&hir)
        .expect_err("multiple mutable borrows should fail borrow checking");

    let first_diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("borrow checking should return a diagnostic");
    let resolved_scope = first_diagnostic
        .primary_location
        .scope
        .to_portable_string(&messages.string_table);
    let expected_scope = project
        .logical_path("src/@page.moth")
        .to_portable_string(&messages.string_table);
    assert!(
        resolved_scope == expected_scope,
        "borrow checker errors should preserve the logical source path in the returned StringTable, expected '{expected_scope}', got '{resolved_scope}'",
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

    let head = ast
        .module_constants
        .iter()
        .find(|c| c.id.name_str(&project.frontend.string_table) == Some("head"))
        .expect("head constant should exist");
    // [$html: <div>Hello</div>] has no runtime slots → folds to StringSlice.
    assert!(
        matches!(head.value.kind, ExpressionKind::StringSlice(_)),
        "head should fold to a string slice when $html directive is available, got {:?}",
        head.value.kind
    );
}

// ---------------------------------
//  Static Bool control-flow handoff
// ---------------------------------
//
// Stage 4 owns static Bool `if` specialisation, so the AST that reaches HIR must contain a
// branch only where the condition is a genuine runtime test. The frozen case below records the
// runtime shape that must survive specialisation. The `intended_` cases record the accepted
// contract the current compiler does not implement yet; they are enabled by the phase that adds
// the Stage 4 specialisation owner.

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
#[ignore = "intended Stage 4 static Bool `if` specialisation; enabled by the static-if phase of \
            docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md"]
fn intended_compile_time_true_condition_reaches_hir_without_a_branch() {
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
#[ignore = "intended Stage 4 static Bool `if` specialisation; enabled by the static-if phase of \
            docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md"]
fn intended_compile_time_false_condition_without_else_lowers_no_branch_body() {
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
        "a compile-time `false` condition with no `else` must produce an empty scoped result"
    );
}

#[test]
#[ignore = "intended Stage 4 terminality over specialised control flow; enabled by the static-if \
            phase of \
            docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md"]
fn intended_terminality_observes_the_selected_branch() {
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
