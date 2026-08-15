//! Frontend stage-by-stage pipeline regression tests.
//!
//! WHAT: exercises tokenizer, header parsing, dependency sorting, AST construction, HIR lowering,
//! and borrow checking through the public frontend entrypoints.
//! WHY: Phase 4 needs coverage across stage boundaries so refactors cannot silently break the
//! compiler pipeline while unit tests still pass in isolation.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderParseOptions, bind_module_headers, prepare_file_from_tokens,
    prepare_header_syntax,
};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
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
            &Config::new(canonical_project_root),
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

    fn borrow_checked_hir(&mut self) -> BorrowCheckReport {
        let hir = self.hir();

        assert!(
            !hir.functions.is_empty(),
            "pipeline smoke test should produce at least one HIR function"
        );

        self.frontend
            .check_borrows(&hir)
            .expect("borrow checking should succeed")
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

    let report = project.borrow_checked_hir();

    assert!(report.stats.functions_analyzed >= 1);
    assert!(!report.analysis.statement_facts.is_empty());
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

    let report = project.borrow_checked_hir();

    assert!(report.stats.functions_analyzed >= 2);
    assert!(!report.analysis.value_facts.is_empty());
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
