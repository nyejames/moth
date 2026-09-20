//! Moth template synthetic-header preparation tests.
//!
//! WHAT: verifies that `.mtf` files enter the frontend as one normal private
//! `content #String` constant with compact directive and body-range facts.
use super::prepare_moth_template_file;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::{Ast, AstBuildContext, AstBuildInput};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticBag, DiagnosticKind,
    DiagnosticLabelStyle, DiagnosticPayload, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderKind,
    HeaderParseOptions, HeaderPreparationFailure, SourcePreparationDelta, bind_module_headers,
    prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::headers::types::{
    FileRole, HeaderExportMode, SyntheticContentPayload,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::{
    ContentSourceTargets, resolve_module_dependencies,
};
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::pipeline::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicSemanticInterface, SourceProviderDependency, SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginConstantId, OriginDeclarationId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceId, SourceKind, SourceRegistrationIndex,
};
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    TokenCursor, TokenRef, TokenTag, TokenizerEntryMode,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

fn prepare_directly(source: &str) -> (FileFrontendPrepareOutput, StringTable, ExtendedSpanBuilder) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.mtf", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("Moth template body should tokenize");
    let owner = SourceTokenOwner::new(lexed.tokens);
    let output = prepare_moth_template_file(
        owner,
        source_path,
        lexed.path_syntax,
        &mut string_table,
        &mut path_fork,
        &mut span_builder,
    )
    .expect("freshly tokenized Moth template should own a preparing path table");
    (output, string_table, span_builder)
}

#[test]
fn canonical_source_keeps_preparing_path_table_separate_until_preparation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("invalid-lifecycle.mtf", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        "# Heading",
        source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should have a tokenizer entry mode"),
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("test Moth template should tokenize");
    let owner = SourceTokenOwner::new(lexed.tokens);
    assert!(
        owner.tokens_ref().path_syntax_table().is_err(),
        "lexer output keeps the preparing path table separate from immutable source tokens"
    );
    let output = prepare_moth_template_file(
        owner,
        source_path,
        lexed.path_syntax,
        &mut string_table,
        &mut path_fork,
        &mut span_builder,
    )
    .expect("preparation should publish the canonical source owner");
    assert_eq!(output.file_id, SourceId::COMPILATION_ROOT);
}

#[test]
fn config_marked_moth_template_paths_stay_out_of_structural_references() {
    let (output, _, _span_builder) = prepare_directly("[@assets/missing.mtf #Config of String]");

    assert!(
        !output.path_syntax.table().paths().is_empty(),
        "fixture should retain the authored path row"
    );
    assert!(
        output.structural_file_references.references().is_empty(),
        "config-marked Moth-template paths must not reach Stage 0 file references"
    );
}

fn prepare_via_pipeline(source: &str) -> SourcePreparationDelta {
    let mut source_files = SourceDatabase::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let entry_file_path = PathBuf::from("src/@page.moth");
    let options = HeaderParseOptions::default();
    let input_path = PathBuf::from("src/intro.mtf");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_id = source_files
        .insert(
            input_path.clone(),
            SourceKind::Compiler(SourceFileKind::MothTemplate),
            &entry_file_path,
            None,
            &mut string_table,
        )
        .expect("test Moth template source identity should register");
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate {
            source_code: source,
        },
        source_id,
        span_builder: ExtendedSpanBuilder::new(),
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };
    let context = FrontendFilePrepareContext {
        source_files: &source_files,
        style_directives: &style_directives,
        entry_file_path: entry_file_path.as_path(),
        options: &options,
    };

    CompilerFrontend::prepare_file_frontend_local(
        &context,
        input,
        &mut string_table,
        &mut path_fork,
    )
}

fn ast_from_moth_template_source(source: &str) -> (Ast, StringTable, PathInternerFork) {
    let style_directives = StyleDirectiveRegistry::built_ins();
    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let input_path = PathBuf::from("src/intro.mtf");
    let source_root = input_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut source_file_kinds = SourceFileKindRegistry::new();
    source_file_kinds.register(
        SourceFileKind::MothTemplate.extension(),
        SourceFileKind::MothTemplate,
    );
    let project_path_resolver = ProjectPathResolver::new_with_module_roots(
        source_root.clone(),
        source_root,
        PreparedSourcePackageRoots::default(),
        &source_file_kinds,
        ModuleRootTable::empty(),
    )
    .expect("test project path resolver should build");
    let entry_file_path = input_path.clone();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    // One-row inventory, same constructor as the standalone Moth template service: authored
    // kind is registered here so preparation stamps `file_id` and binding applies template scope.
    let registration_index = SourceRegistrationIndex::from_rows(std::iter::once((
        input_path.as_path(),
        SourceKind::Compiler(SourceFileKind::MothTemplate),
    )));
    let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        &input_path,
        Some(&project_path_resolver),
        &mut string_table,
    )
    .expect("Moth template source identity should register");
    let entry_file_id = source_files
        .get_by_canonical_path(&input_path)
        .map(|identity| identity.id)
        .expect("standalone Moth template source identity was not registered");
    let options = HeaderParseOptions {
        entry_file_id: Some(entry_file_id),
        entry_file_role: None,
        active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
    };
    let context = FrontendFilePrepareContext {
        source_files: &source_files,
        style_directives: &style_directives,
        entry_file_path: entry_file_path.as_path(),
        options: &options,
    };
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate {
            source_code: source,
        },
        source_id: entry_file_id,
        span_builder: ExtendedSpanBuilder::new(),
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };
    let SourcePreparationDelta { result, .. } = CompilerFrontend::prepare_file_frontend_local(
        &context,
        input,
        &mut string_table,
        &mut path_fork,
    );
    let mut prepared_file = result.expect("Moth template source should prepare");
    assert_eq!(
        prepared_file.file_id, entry_file_id,
        "registered Moth template source should stamp its source identity"
    );
    prepared_file
        .freeze_path_syntax(&string_table, &mut path_fork)
        .expect("single-file Moth template output should satisfy the prepared-file invariant gate");

    let prepared_syntax = prepare_header_syntax(
        &mut [prepared_file],
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
        &mut path_fork,
    )
    .expect("Moth template header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        Some(&project_path_resolver),
        &source_files,
        &mut string_table,
        &mut path_fork,
    )
    .expect("Moth template headers should bind");
    let sorted_headers = resolve_module_dependencies(
        headers,
        &ContentSourceTargets::empty(),
        &mut string_table,
        &mut path_fork,
    )
    .expect("headers should sort");
    let entry_dir = path_fork
        .try_intern_portable_path("src/@page.moth", &mut string_table)
        .expect("test path fits");

    let ast = Ast::new(
        AstBuildInput {
            headers: sorted_headers.headers,
            source_token_owners: sorted_headers.source_token_owners,
            module_symbols: sorted_headers.module_symbols,
            binding_environment: sorted_headers.binding_environment,
            top_level_const_fragments: sorted_headers.top_level_const_fragments,
            source_build_config_contract_names: Arc::new(Default::default()),
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: Arc::clone(&external_package_registry),
            style_directives: &style_directives,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            entry_dir,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: None,
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
        },
    )
    .expect("Moth template content constant should build through AST")
    .ast;
    (ast, string_table, path_fork)
}

fn empty_provider_interface(prefix: &str) -> PublicSemanticInterface {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(prefix),
        format!("{prefix}/@mod.moth"),
        ModuleRootRole::Normal,
    );
    PublicSemanticInterface {
        module_origin,
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

struct MothTemplateScopeFixture {
    _temp_dir: TempDir,
    project_root: PathBuf,
    html_root_file: PathBuf,
    entry_file_path: PathBuf,
    project_path_resolver: ProjectPathResolver,
    source_files: SourceDatabase,
    base_string_table: StringTable,
}

impl MothTemplateScopeFixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let temp_dir = tempfile::tempdir().expect("test project root should be created");
        let project_root = temp_dir.path().join("project");
        let entry_root = project_root.join("src");
        let html_root = temp_dir.path().join("html_package");

        fs::create_dir_all(&entry_root).expect("entry root should be created");
        fs::create_dir_all(&html_root).expect("HTML source-backed package should be created");
        let project_root =
            fs::canonicalize(project_root).expect("project root should canonicalize");
        let entry_root = fs::canonicalize(entry_root).expect("entry root should canonicalize");
        let html_root = fs::canonicalize(html_root).expect("HTML root should canonicalize");
        let html_root_file = html_root.join("@mod.moth");

        // The miniature `@html` root deliberately includes non-constant exports so the
        // Moth template implicit scope proves it is filtering by source declaration kind.
        fs::write(
            &html_root_file,
            r#"export:
    p #String = "<p>"
    collision #= "html"
    html_defaults #= HtmlDefaults(color = "green")
    HtmlDefaults = | color String |
    render_html || -> String:
        return "runtime"
    ;
;
"#,
        )
        .expect("HTML source-backed package root should be written");

        let mut canonical_files = vec![html_root_file.clone()];
        for (relative_path, source) in files {
            let path = project_root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("source parent should be created");
            }
            fs::write(&path, source).expect("source file should be written");
            canonical_files.push(fs::canonicalize(path).expect("source path should canonicalize"));
        }

        let mut source_packages = SourcePackageRegistry::new();
        source_packages.register_filesystem_root("html", html_root.clone(), PackageOrigin::Builder);

        let mut source_file_kinds = SourceFileKindRegistry::new();
        source_file_kinds.register("mtf", SourceFileKind::MothTemplate);

        let module_roots = prepared_module_roots(&entry_root, &canonical_files);
        let mut prep_string_table = StringTable::new();
        let project_path_resolver = ProjectPathResolver::new_with_module_roots(
            project_root.clone(),
            entry_root.clone(),
            crate::build_system::create_project_modules::source_package_discovery::
                build_source_package_boundary_indexes(
                    &source_packages,
                    &source_file_kinds,
                    &crate::builder_surface::external_import_providers::registry::
                        ExternalImportProviderRegistry::default(),
                    &mut prep_string_table,
                )
                .expect("test source package boundary indexes should build")
                .prepared_source_package_roots(),
            &source_file_kinds,
            module_roots,
        )
        .expect("test project path resolver should build");

        let mut string_table = StringTable::new();
        let entry_file_path = entry_root.join("@page.moth");
        let source_files = SourceDatabase::build(
            canonical_files.iter(),
            &entry_file_path,
            Some(&project_path_resolver),
            &mut string_table,
        )
        .expect("source file identities should build");

        Self {
            _temp_dir: temp_dir,
            project_root,
            html_root_file,
            entry_file_path,
            project_path_resolver,
            source_files,
            base_string_table: string_table,
        }
    }

    fn compile_moth_template_ast(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
    ) -> Result<(Ast, StringTable, PathInternerFork), CompilerDiagnostic> {
        let (ast, string_table, path_fork) = self.compile_module_ast(prepared_relative_paths)?;

        self.assert_ast_contains_moth_template_content(
            &ast,
            &string_table,
            moth_template_relative_path,
        );

        Ok((ast, string_table, path_fork))
    }
    fn compile_moth_template_ast_with_providers(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
    ) -> Result<(Ast, StringTable, PathInternerFork), CompilerDiagnostic> {
        let (ast, string_table, path_fork) = self.compile_module_ast_with_providers(
            prepared_relative_paths,
            source_provider_dependencies,
        )?;

        self.assert_ast_contains_moth_template_content(
            &ast,
            &string_table,
            moth_template_relative_path,
        );

        Ok((ast, string_table, path_fork))
    }
    fn compile_module_ast(
        &self,
        prepared_relative_paths: &[&str],
    ) -> Result<(Ast, StringTable, PathInternerFork), CompilerDiagnostic> {
        let html_interface = empty_provider_interface("html");
        let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
            kind:
                crate::compiler_frontend::public_interface::ProviderDependencyKind::ImplicitTemplate {
                    package_prefix: "html",
                },
            interface: &html_interface,
        }])
        .expect("one implicit template provider should register");
        self.compile_module_ast_with_providers(prepared_relative_paths, &provider_dependencies)
    }

    fn compile_module_ast_with_providers(
        &self,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
    ) -> Result<(Ast, StringTable, PathInternerFork), CompilerDiagnostic> {
        let mut path_fork = self.source_files.fork_path_interner();
        let (headers, mut string_table) = self.prepare_and_bind_headers_with_providers(
            prepared_relative_paths,
            source_provider_dependencies,
            &mut path_fork,
        )?;
        let sorted_headers = resolve_module_dependencies(
            headers,
            &ContentSourceTargets::empty(),
            &mut string_table,
            &mut path_fork,
        )
        .map_err(|failure| {
            let messages = failure.into_messages(&string_table);
            if let Some(diagnostic) = messages.first_error() {
                return diagnostic.clone();
            }
            if let Some(error) = messages.infrastructure_error() {
                panic!("dependency sorting infrastructure failure: {}", error.msg)
            }
            panic!("dependency sorting failed without a diagnostic")
        })?;
        let entry_dir = path_fork
            .try_intern_filesystem_path(&self.entry_file_path, &mut string_table)
            .expect("test path should be UTF-8");
        let style_directives = StyleDirectiveRegistry::built_ins();
        let external_package_registry = Arc::new(ExternalPackageRegistry::new());

        Ast::new(
            AstBuildInput {
                source_token_owners: sorted_headers.source_token_owners,
                headers: sorted_headers.headers,
                module_symbols: sorted_headers.module_symbols,
                binding_environment: sorted_headers.binding_environment,
                top_level_const_fragments: sorted_headers.top_level_const_fragments,
                source_build_config_contract_names: Arc::new(Default::default()),
            },
            AstBuildContext {
                root_role: ModuleRootRole::Normal,
                external_package_registry: Arc::clone(&external_package_registry),
                style_directives: &style_directives,
                string_table: &mut string_table,
                path_fork: &mut path_fork,
                entry_dir,
                build_profile: FrontendBuildProfile::Dev,
                file_value_resolution: None,
                config_resolution: None,
                build_config_values: Arc::new(Default::default()),
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                capacity_estimate: Default::default(),
                #[cfg(feature = "timers")]
                timing_context: None,
                #[cfg(feature = "timers")]
                timing_metric_family:
                    crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
            },
        )
        .map_err(|messages| {
            if let Some(diagnostic) = messages.first_error() {
                return diagnostic.clone();
            }
            if let Some(error) = messages.infrastructure_error() {
                panic!("AST failed with infrastructure error: {}", error.msg)
            }
            panic!("AST failed without a diagnostic")
        })
        .map(|build_result| (build_result.ast, string_table, path_fork))
    }

    fn assert_ast_contains_moth_template_content(
        &self,
        ast: &Ast,
        string_table: &StringTable,
        moth_template_relative_path: &str,
    ) {
        let _ = (string_table, moth_template_relative_path);
        assert!(
            ast.const_values
                .iter_module_constant_views()
                .any(|row| *row.path != PathId::ROOT),
            "compiled AST should include Moth template content"
        );
    }

    fn prepare_and_bind_headers_for(
        &self,
        prepared_relative_paths: &[&str],
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
            PathInternerFork,
        ),
        CompilerDiagnostic,
    > {
        let html_interface = empty_provider_interface("html");
        let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
            kind:
                crate::compiler_frontend::public_interface::ProviderDependencyKind::ImplicitTemplate {
                    package_prefix: "html",
                },
            interface: &html_interface,
        }])
        .expect("one implicit template provider should register");
        let mut path_fork = self.source_files.fork_path_interner();
        self.prepare_and_bind_headers_with_providers(
            prepared_relative_paths,
            &provider_dependencies,
            &mut path_fork,
        )
        .map(|(headers, string_table)| (headers, string_table, path_fork))
    }

    fn prepare_and_bind_headers_with_providers(
        &self,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
        path_fork: &mut PathInternerFork,
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
        ),
        CompilerDiagnostic,
    > {
        self.prepare_and_bind_headers_with_providers_with_table(
            prepared_relative_paths,
            source_provider_dependencies,
            path_fork,
        )
        .map_err(|(diagnostic, _string_table)| diagnostic)
    }

    fn prepare_and_bind_headers_with_providers_with_table(
        &self,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
        path_fork: &mut PathInternerFork,
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
        ),
        (CompilerDiagnostic, StringTable),
    > {
        let style_directives = StyleDirectiveRegistry::built_ins();
        let external_package_registry = Arc::new(ExternalPackageRegistry::new());
        let options = HeaderParseOptions {
            entry_file_id: None,
            entry_file_role: None,
            active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        };
        let context = FrontendFilePrepareContext {
            source_files: &self.source_files,
            style_directives: &style_directives,
            entry_file_path: self.entry_file_path.as_path(),
            options: &options,
        };
        let mut string_table = self.base_string_table.clone();
        let mut prepared_files = Vec::new();
        let mut span_builders = Vec::new();

        for relative_path in prepared_relative_paths {
            let source_path = self.source_path_for_fixture_path(relative_path);
            let source_code = fs::read_to_string(&source_path).expect("source should be readable");
            let source_id = self
                .source_files
                .get_by_canonical_path(&source_path)
                .expect("fixture source should be registered")
                .id;
            let source_kind = source_path
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(SourceFileKind::from_extension)
                .unwrap_or(SourceFileKind::Moth);

            let mut span_builder = ExtendedSpanBuilder::new();
            let source = match source_kind {
                SourceFileKind::Moth => {
                    let (owner, path_syntax) = CompilerFrontend::tokenize_source(
                        &self.source_files,
                        &style_directives,
                        &source_code,
                        &source_path,
                        TokenizerEntryMode::SourceFile,
                        &mut string_table,
                        path_fork,
                        &mut span_builder,
                    )
                    .map_err(|error| match error {
                        FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                            diagnostic,
                            ..
                        }) => (diagnostic, string_table.clone()),
                        FileFrontendPrepareFailure::Infrastructure(error) => {
                            panic!("fixture tokenization hit infrastructure failure: {error:?}")
                        }
                    })?;
                    FrontendFilePrepareSource::Moth { owner, path_syntax }
                }
                SourceFileKind::MothTemplate => FrontendFilePrepareSource::MothTemplate {
                    source_code: source_code.as_str(),
                },
                SourceFileKind::PlainMarkdown => FrontendFilePrepareSource::PlainMarkdown {
                    source_code: source_code.as_str(),
                },
            };
            let input = FrontendFilePrepareInput {
                source,
                source_id,
                span_builder,
                const_template_offset: 0,
                runtime_fragment_offset: 0,
            };

            let SourcePreparationDelta {
                result,
                span_builder,
                ..
            } = CompilerFrontend::prepare_file_frontend_local(
                &context,
                input,
                &mut string_table,
                path_fork,
            );
            span_builders.push((source_id, span_builder));
            let mut output = result.map_err(|error| match error {
                FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                    diagnostic,
                    ..
                }) => (diagnostic, string_table.clone()),
                FileFrontendPrepareFailure::Infrastructure(error) => {
                    panic!("fixture preparation hit infrastructure failure: {error:?}")
                }
            })?;
            output
                .freeze_path_syntax(&string_table, path_fork)
                .expect("fixture prepared output should satisfy the prepared-file invariant gate");
            prepared_files.push(output);
        }

        let prepared_syntax = prepare_header_syntax(
            &mut prepared_files,
            &mut string_table,
            &mut |source, diagnostic| {
                let _ = span_builders
                    .iter_mut()
                    .find(|(id, _)| *id == source)
                    .expect("prepared source retains its original span builder");
                diagnostic.capture_preparation_span(source)
            },
            path_fork,
        )
        .map_err(|failure| match failure {
            HeaderPreparationFailure::Diagnosed(bag) => {
                (first_diagnostic_from_bag(bag), string_table.clone())
            }
            HeaderPreparationFailure::Infrastructure(error) => {
                panic!("aggregation fixture infrastructure failure: {error:?}")
            }
        })?;
        let headers = bind_module_headers(
            prepared_syntax,
            &external_package_registry,
            &ExternalImportResolutionTable::default(),
            source_provider_dependencies,
            Some(&self.project_path_resolver),
            &self.source_files,
            &mut string_table,
            path_fork,
        )
        .map_err(|failure| match failure {
            HeaderPreparationFailure::Diagnosed(bag) => {
                (first_diagnostic_from_bag(bag), string_table.clone())
            }
            HeaderPreparationFailure::Infrastructure(error) => {
                panic!("binding fixture infrastructure failure: {error:?}")
            }
        })?;

        Ok((headers, string_table))
    }

    fn compile_moth_template_ast_ok(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
    ) -> (Ast, StringTable, PathInternerFork) {
        self.compile_moth_template_ast(moth_template_relative_path, prepared_relative_paths)
            .expect("Moth template fixture should compile")
    }

    fn compile_moth_template_diagnostic(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
    ) -> CompilerDiagnostic {
        match self.compile_moth_template_ast(moth_template_relative_path, prepared_relative_paths) {
            Ok(_) => panic!("Moth template fixture should fail"),
            Err(diagnostic) => diagnostic,
        }
    }

    fn project_root_path(&self) -> &Path {
        &self.project_root
    }

    fn source_path_for_fixture_path(&self, relative_path: &str) -> PathBuf {
        if relative_path == "@html/@mod.moth" {
            return self.html_root_file.clone();
        }

        self.project_root_path().join(relative_path)
    }
}

fn prepared_module_roots(entry_root: &Path, files: &[PathBuf]) -> ModuleRootTable {
    let mut roots_by_directory = BTreeMap::<PathBuf, PathBuf>::new();

    for file in files {
        if !file.starts_with(entry_root) {
            continue;
        }

        let Some(file_name) = file.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with('@')
            || file.extension().and_then(|ext| ext.to_str()) != Some("moth")
        {
            continue;
        }

        let directory = file
            .parent()
            .expect("fixture source file should have a parent")
            .to_path_buf();
        roots_by_directory.insert(directory, file.clone());
    }

    let records = roots_by_directory
        .into_iter()
        .map(|(directory, root_file)| ModuleRootRecord::new(directory, root_file))
        .collect();

    ModuleRootTable::from_records(records)
}

fn first_diagnostic_from_bag(bag: DiagnosticBag) -> CompilerDiagnostic {
    bag.into_diagnostics()
        .into_iter()
        .next()
        .expect("diagnostic bag should contain an error")
}

fn prepare_moth_source(
    source: &str,
    file_path: &Path,
    entry_file_path: &Path,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> (FileFrontendPrepareOutput, ExtendedSpanBuilder) {
    let source_path = path_fork
        .try_intern_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("Moth source should tokenize");
    let owner = SourceTokenOwner::new(lexed.tokens);
    let output = prepare_file_from_tokens(
        owner,
        source_path,
        lexed.path_syntax,
        entry_file_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
        &mut span_builder,
        path_fork,
    )
    .expect("Moth header preparation should succeed");
    (output, span_builder)
}
fn content_header(
    output: &FileFrontendPrepareOutput,
) -> &crate::compiler_frontend::headers::types::Header {
    assert_eq!(output.headers.len(), 1);
    &output.headers[0]
}

fn content_constant(
    output: &FileFrontendPrepareOutput,
) -> &crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax {
    let HeaderKind::Constant { declaration, .. } = &content_header(output).kind else {
        panic!("Moth template should produce a constant header");
    };
    declaration
}

fn retained_body_tokens<'a>(output: &'a FileFrontendPrepareOutput) -> TokenCursor<'a> {
    let header = content_header(output);
    output
        .source_token_stream
        .as_ref()
        .expect("Moth template should retain its canonical source owner")
        .cursor(header.tokens)
        .expect("retained body range should resolve")
}

fn first_body_token_with_tag<'a>(
    output: &'a FileFrontendPrepareOutput,
    tag: TokenTag,
) -> Option<TokenRef<'a>> {
    let mut cursor = retained_body_tokens(output);
    while let Some(token) = cursor.advance() {
        if token.tag() == tag {
            return Some(token);
        }
    }
    None
}

fn body_has_tag(output: &FileFrontendPrepareOutput, tag: TokenTag) -> bool {
    first_body_token_with_tag(output, tag).is_some()
}

fn body_has_string(
    output: &FileFrontendPrepareOutput,
    string_table: &StringTable,
    expected: &str,
) -> bool {
    let mut cursor = retained_body_tokens(output);
    while let Some(token) = cursor.advance() {
        if token.tag() == TokenTag::STRING_SLICE_LITERAL
            && token
                .string_id()
                .is_some_and(|id| string_table.resolve(id) == expected)
        {
            return true;
        }
    }
    false
}

/// Fold one module constant identified by its final path component.
///
/// WHAT: resolves a named declaration through the same path fork domain that produced the AST
/// rows, instead of relying on row ordering.
fn folded_module_constant_value(
    ast: &Ast,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
    name: &str,
) -> Option<String> {
    let value_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| {
            path_fork
                .try_component(*row.path)
                .is_some_and(|component| string_table.resolve(component) == name)
        })
        .map(|row| row.id)?;
    let value = ast.const_values.string_value(value_id)?;
    Some(string_table.resolve(value).to_owned())
}

fn folded_content_value(
    ast: &Ast,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> String {
    folded_module_constant_value(ast, string_table, path_fork, "content")
        .expect("Moth template content constant should fold to a string")
}

fn folded_content_contains(
    ast: &Ast,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
    expected: &str,
) {
    let content = folded_content_value(ast, string_table, path_fork);
    assert!(
        content.contains(expected),
        "folded content should contain {expected:?}, got {content:?}"
    );
}

fn folded_constant_value(
    ast: &Ast,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
    name: &str,
) -> String {
    folded_module_constant_value(ast, string_table, path_fork, name)
        .unwrap_or_else(|| panic!("module constant {name} should fold to a string"))
}

#[test]
fn moth_template_preparation_produces_private_content_constant() {
    let (output, _string_table, _span_builder) = prepare_directly("# Heading");
    let header = content_header(&output);
    let declaration = content_constant(&output);
    assert_eq!(
        declaration.span, None,
        "the generated content declaration should not claim an authored source span"
    );
    assert!(
        declaration.initializer_range.is_none(),
        "Moth template declaration shells must not retain body token vectors"
    );
    assert!(
        matches!(
            header.synthetic_content_payload,
            Some(SyntheticContentPayload::MothTemplate {
                markdown_directive: _
            })
        ),
        "Moth template header should retain compact directive payload"
    );
    assert!(
        !header.tokens.is_empty(),
        "non-empty body should retain a body range"
    );

    assert_eq!(output.file_role, FileRole::Normal);
    assert!(output.file_dependency_clauses.is_empty());
    assert!(output.top_level_const_fragments.is_empty());
    let source_owner = output
        .source_token_stream
        .as_ref()
        .expect("prepared Moth template source retains its token owner");
    assert_ne!(output.source_file, PathId::ROOT);
    assert_ne!(header.declaration_path, PathId::ROOT);
    assert_eq!(source_owner.source(), output.file_id);
    assert_eq!(declaration.binding_mode, BindingMode::CompileTimeConstant);
    let ParsedTypeRef::BuiltinString { span } = &declaration.type_annotation else {
        panic!("expected builtin String annotation");
    };
    assert_eq!(*span, declaration.span);
}
#[test]
fn simple_markdown_body_uses_original_body_token_span() {
    let source = "# Heading";
    let (output, string_table, span_builder) = prepare_directly(source);
    let body_token = first_body_token_with_tag(&output, TokenTag::STRING_SLICE_LITERAL)
        .expect("body text should be preserved as a string literal token");
    let body_range = body_token
        .span()
        .resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert!(
        body_range.end() > body_range.start(),
        "body token should retain a resolved span"
    );
    assert_eq!(
        body_token.string_id().map(|id| string_table.resolve(id)),
        Some("# Heading")
    );
}
#[test]
fn nested_templates_remain_structural_inside_retained_body_range() {
    let source = "before [:inner] after";
    let (output, _string_table, span_builder) = prepare_directly(source);
    let mut cursor = retained_body_tokens(&output);
    let mut template_head_count = 0;
    let mut nested_span = None;
    while let Some(token) = cursor.advance() {
        if token.tag() == TokenTag::TEMPLATE_HEAD {
            template_head_count += 1;
            nested_span = Some(
                token
                    .span()
                    .resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT)),
            );
        }
    }
    assert_eq!(template_head_count, 1);
    let nested_range = nested_span.expect("nested template opener should be present");
    assert!(
        nested_range.start() > 0,
        "nested template opener should keep its original body position"
    );
}
#[test]
fn simple_markdown_body_folds_like_markdown_template() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("# Heading");

    assert_eq!(
        folded_content_value(&ast, &string_table, &path_fork),
        "<h1>Heading</h1>"
    );
}

#[test]
fn nested_moth_template_defaults_to_markdown_formatting() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("[:# Nested]");

    assert_eq!(
        folded_content_value(&ast, &string_table, &path_fork),
        "<h1>Nested</h1>"
    );
}

#[test]
fn explicit_nested_raw_directive_overrides_moth_template_markdown_default() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("[$raw:# Nested]");

    assert_eq!(
        folded_content_value(&ast, &string_table, &path_fork),
        "# Nested"
    );
}

#[test]
fn explicit_nested_non_formatter_directive_overrides_moth_template_markdown_default() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("[$fresh:# Nested]");

    assert_eq!(
        folded_content_value(&ast, &string_table, &path_fork),
        "# Nested"
    );
}

#[test]
fn moth_template_compile_time_if_folds_inside_content_constant() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("[if true: visible]");

    folded_content_contains(&ast, &string_table, &path_fork, "visible");
}

#[test]
fn moth_template_compile_time_collection_loop_folds_inside_content_constant() {
    let (ast, string_table, path_fork) =
        ast_from_moth_template_source(r#"[loop {"one", "two"} |item|: [item] ]"#);

    let content = folded_content_value(&ast, &string_table, &path_fork);
    assert!(
        content.contains("one") && content.contains("two"),
        "folded loop content should contain both collection items, got {content:?}"
    );
}
#[test]
fn empty_moth_template_body_retains_empty_body_range_and_directive_payload() {
    let (output, string_table, _span_builder) = prepare_directly("");
    let header = content_header(&output);
    let declaration = content_constant(&output);

    assert!(header.tokens.is_empty());
    assert!(declaration.initializer_range.is_none());
    let Some(SyntheticContentPayload::MothTemplate { markdown_directive }) =
        header.synthetic_content_payload
    else {
        panic!("expected Moth template payload");
    };
    assert_eq!(string_table.resolve(markdown_directive), "md");
}

#[test]
fn backslash_remains_body_text_inside_markdown_initializer() {
    let (output, string_table, _span_builder) = prepare_directly(r"before \n after");
    assert!(body_has_string(&output, &string_table, r"before \n after"));
}

#[test]
fn unescaped_outer_close_diagnostic_flows_through_pipeline_preparation() {
    let SourcePreparationDelta {
        result: Err(error),
        span_builder: _span_builder,
        ..
    } = prepare_via_pipeline("]")
    else {
        panic!("unescaped implicit Moth template close should fail during preparation");
    };
    let FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
        warnings,
        diagnostic,
        ..
    }) = error
    else {
        panic!("unescaped template close must be a source diagnostic");
    };

    assert!(warnings.is_empty());
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::UnescapedImplicitTemplateClose {
            source_kind: SourceFileKind::MothTemplate
        }
    ));
}

#[test]
fn double_dash_remains_body_text() {
    let (output, string_table, _span_builder) = prepare_directly("alpha -- still text\nbeta");
    assert!(body_has_string(
        &output,
        &string_table,
        "alpha -- still text\nbeta"
    ));
}

#[test]
fn declaration_like_text_remains_markdown_body_text() {
    let (output, string_table, _span_builder) =
        prepare_directly("@docs/intro\ncontent #String = value");
    let declaration = content_constant(&output);
    assert!(declaration.initializer_references.is_empty());
    assert!(body_has_string(
        &output,
        &string_table,
        "@docs/intro\ncontent #String = value",
    ));
}

#[test]
fn module_root_export_syntax_can_target_moth_template_content() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let root_file_path = PathBuf::from("src/@mod.moth");
    let entry_path = PathBuf::from("src/@page.moth");

    let (root_output, _span_builder) = prepare_moth_source(
        "export:\n    @intro content as intro\n;\n",
        &root_file_path,
        &entry_path,
        &mut string_table,
        &mut path_fork,
    );

    assert_eq!(root_output.file_dependency_clauses.len(), 1);
    assert_eq!(
        root_output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Public
    );
    assert_ne!(
        root_output.file_dependency_clauses[0].dependency.path,
        PathId::ROOT
    );
    let selections = root_output.file_dependency_clauses[0]
        .selections(&root_output.dependency_selections)
        .expect("retained template export selection range should be valid");
    assert_eq!(selections.len(), 1);
    assert_eq!(string_table.resolve(selections[0].source_name), "content");
    assert_eq!(
        selections[0]
            .local_alias
            .as_ref()
            .map(|alias| string_table.resolve(alias.name)),
        Some("intro")
    );
}

#[test]
fn moth_template_body_sees_flat_exported_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[p]")]);
    let (ast, string_table, path_fork) = fixture
        .compile_moth_template_ast_ok("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    folded_content_contains(&ast, &string_table, &path_fork, "<p>");
}

#[test]
fn moth_template_header_visibility_contains_implicit_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[p]")]);
    let (headers, mut string_table, mut path_fork) = fixture
        .prepare_and_bind_headers_for(&["@html/@mod.moth", "src/intro.mtf"])
        .expect("headers should parse");
    let moth_template_canonical_path = fixture.project_root_path().join("src/intro.mtf");
    let moth_template_logical_path = fixture
        .project_path_resolver
        .logical_path_for_canonical_file(&moth_template_canonical_path)
        .expect("Moth template logical path should resolve");
    let moth_template_source = path_fork
        .try_intern_filesystem_path(&moth_template_logical_path, &mut string_table)
        .expect("test path should be UTF-8");
    let visibility = headers
        .binding_environment
        .visibility_for(&moth_template_source)
        .expect("Moth template visibility should exist");
    let p_name = string_table.intern("p");

    assert!(
        visibility.visible_source_names.contains_key(&p_name),
        "Moth template visibility should include @html p; visible names: {:?}",
        visibility
            .visible_source_names
            .keys()
            .map(|name| string_table.resolve(*name).to_owned())
            .collect::<Vec<_>>()
    );
}

#[test]
fn moth_template_body_sees_exported_same_directory_root_constants() {
    let fixture = MothTemplateScopeFixture::new(&[
        (
            "src/docs/@mod.moth",
            "export:\n    local_label #= \"from root\"\n;\n",
        ),
        ("src/docs/intro.mtf", "[local_label]"),
    ]);
    let (ast, string_table, path_fork) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    folded_content_contains(&ast, &string_table, &path_fork, "from root");
}
#[test]
fn moth_template_without_same_directory_root_sees_only_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[("src/docs/intro.mtf", "[collision]")]);
    let (ast, string_table, path_fork) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &["@html/@mod.moth", "src/docs/intro.mtf"],
    );

    folded_content_contains(&ast, &string_table, &path_fork, "html");
}
#[test]
fn same_directory_root_constants_collide_with_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[
        (
            "src/docs/@mod.moth",
            "export:\n    collision #= \"local\"\n;\n",
        ),
        ("src/docs/intro.mtf", "[collision]"),
    ]);
    let diagnostic = fixture.compile_moth_template_diagnostic(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Import(
            crate::compiler_frontend::compiler_messages::ImportDiagnosticKind::ImportNameCollision
        )
    ));
    assert!(
        diagnostic.primary_span.is_some(),
        "collision primary span should remain separate from secondary labels"
    );
    assert_eq!(diagnostic.labels.len(), 1);
    assert!(
        diagnostic
            .labels
            .iter()
            .any(|label| label.style == DiagnosticLabelStyle::Secondary)
    );

    assert!(
        diagnostic.labels.iter().all(|label| label.span.is_some()),
        "collision labels should retain exact source spans"
    );
}

#[test]
fn exported_html_functions_are_not_visible_to_moth_template_body() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[render_html]")]);
    let diagnostic = fixture
        .compile_moth_template_diagnostic("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    assert!(
        !matches!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
        ),
        "non-constant filtering should fail during semantic lookup, got {diagnostic:?}"
    );
}

#[test]
fn moth_template_sees_capability_selected_provider_constants_without_provider_headers() {
    // Simulates the production path: source-backed providers are separate compiled modules whose
    // completed interfaces are available through SourceProviderDependencySet, but whose source
    // headers are not in the consumer module's prepared files. The `.mtf` implicit scope must
    // collect constant exports from every capability-selected provider, not only `@html`.
    let fixture =
        MothTemplateScopeFixture::new(&[("src/intro.mtf", "[test_constant] [custom_constant]")]);

    let html_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("html"),
        "html".to_owned(),
        ModuleRootRole::Normal,
    );

    let constant_origin = OriginDeclarationId::Constant(OriginConstantId::new(
        html_origin.clone(),
        "test_constant".to_owned(),
    ));

    let html_interface = PublicSemanticInterface {
        module_origin: html_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            html_origin.clone(),
            "test_constant".to_owned(),
            constant_origin.clone(),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: constant_origin,
            synthetic_interface_provenance: Default::default(),
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String(OwnedFoldedString::Text(
                    "from html".to_owned(),
                )),
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let custom_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("custom"),
        "custom".to_owned(),
        ModuleRootRole::Normal,
    );
    let custom_constant_origin = OriginDeclarationId::Constant(OriginConstantId::new(
        custom_origin.clone(),
        "custom_constant".to_owned(),
    ));
    let custom_interface = PublicSemanticInterface {
        module_origin: custom_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            custom_origin,
            "custom_constant".to_owned(),
            custom_constant_origin.clone(),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: custom_constant_origin,
            synthetic_interface_provenance: Default::default(),
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String(OwnedFoldedString::Text(
                    "from custom".to_owned(),
                )),
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let provider_dependencies =
        SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::ImplicitTemplate {
                package_prefix: "html",
            },
            interface: &html_interface,
        },
        SourceProviderDependency {
            kind: crate::compiler_frontend::public_interface::ProviderDependencyKind::ImplicitTemplate {
                package_prefix: "custom",
            },
            interface: &custom_interface,
        },
    ])
        .expect("two distinct implicit template providers should register");

    let (ast, string_table, path_fork) = fixture
        .compile_moth_template_ast_with_providers(
            "src/intro.mtf",
            &["src/intro.mtf"],
            &provider_dependencies,
        )
        .expect(".mtf body should see @html constant through provider interface");

    folded_content_contains(&ast, &string_table, &path_fork, "from html");
    folded_content_contains(&ast, &string_table, &path_fork, "from custom");
}
#[test]
fn moth_template_runtime_function_call_is_rejected_by_const_template_folding() {
    let fixture = MothTemplateScopeFixture::new(&[
        (
            "src/docs/@mod.moth",
            r#"export:
    render_local || -> String:
        return "runtime"
    ;
;
"#,
        ),
        ("src/docs/intro.mtf", "[render_local()]"),
    ]);
    let diagnostic = fixture.compile_moth_template_diagnostic(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    assert!(
        !matches!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
        ),
        "runtime function calls should fail through semantic const-template rules, got {diagnostic:?}"
    );
}

#[test]
fn moth_template_unknown_template_condition_is_rejected_by_const_template_folding() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[if show: visible]")]);
    let diagnostic = fixture
        .compile_moth_template_diagnostic("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    assert!(
        matches!(diagnostic.kind, DiagnosticKind::Rule(_)),
        "unknown Moth template conditions should use normal const diagnostics, got {diagnostic:?}"
    );
}

#[test]
fn exported_same_directory_functions_and_types_are_not_visible_to_moth_template_body() {
    let fixture = MothTemplateScopeFixture::new(&[
        (
            "src/docs/@mod.moth",
            r#"export:
    LocalType = | value String |
    render_local || -> String:
        return "runtime"
    ;
;
"#,
        ),
        ("src/docs/intro.mtf", "[render_local][LocalType]"),
    ]);
    let diagnostic = fixture.compile_moth_template_diagnostic(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    assert!(
        !matches!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
        ),
        "non-constant root exports should fail during semantic lookup, got {diagnostic:?}"
    );
}

#[test]
fn moth_template_const_record_field_access_folds_in_template_head() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[html_defaults.color]")]);
    let (ast, string_table, path_fork) = fixture
        .compile_moth_template_ast_ok("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    folded_content_contains(&ast, &string_table, &path_fork, "green");
}

#[test]
fn root_supplied_content_constant_can_be_referenced_normally() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/docs/@mod.moth", "export:\n    @other content\n;\n"),
        ("src/docs/other.mtf", "shared body"),
        ("src/docs/intro.mtf", "[content]"),
    ]);
    let (ast, string_table, path_fork) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/other.mtf",
            "src/docs/intro.mtf",
        ],
    );

    folded_content_contains(&ast, &string_table, &path_fork, "shared body");
}
#[test]
fn generated_self_content_is_not_visible_to_moth_template_body() {
    let fixture = MothTemplateScopeFixture::new(&[("src/docs/intro.mtf", "[content]")]);
    let diagnostic = fixture.compile_moth_template_diagnostic(
        "src/docs/intro.mtf",
        &["@html/@mod.moth", "src/docs/intro.mtf"],
    );

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::ConstantNotVisible,
                ..
            }
        ),
        "expected generated self content to be absent from body visibility, got {diagnostic:?}"
    );
}

#[test]
fn self_originating_content_reexport_is_excluded_from_moth_template_body_scope() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/docs/@mod.moth", "export:\n    @intro content\n;\n"),
        ("src/docs/intro.mtf", "[content]"),
    ]);
    let diagnostic = fixture.compile_moth_template_diagnostic(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::ConstantNotVisible,
                ..
            }
        ),
        "expected self content to be absent from dependency visibility, got {diagnostic:?}"
    );
}

#[test]
fn moth_dependency_binds_template_content_as_folded_string_constant() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/@page.moth", ""),
        (
            "src/main.moth",
            "@intro content as intro_content\nfrom_intro #String = intro_content\n",
        ),
        ("src/intro.mtf", "# Intro"),
    ]);
    let (ast, string_table, path_fork) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/intro.mtf",
            "src/main.moth",
            "src/@page.moth",
        ])
        .expect("module using imported Moth template content should compile through AST");

    assert_eq!(
        folded_constant_value(&ast, &string_table, &path_fork, "from_intro"),
        "<h1>Intro</h1>"
    );
}

#[test]
fn moth_namespace_dependency_binds_template_content_as_folded_string_constant() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/@page.moth", ""),
        (
            "src/main.moth",
            "@intro\nfrom_intro #String = intro.content\n",
        ),
        ("src/intro.mtf", "# Intro"),
    ]);
    let (ast, string_table, path_fork) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/main.moth",
            "src/@page.moth",
            "src/intro.mtf",
        ])
        .expect("module using namespace-imported Moth template content should compile through AST");

    assert_eq!(
        folded_constant_value(&ast, &string_table, &path_fork, "from_intro"),
        "<h1>Intro</h1>"
    );
}

#[test]
fn imported_bd_file_produces_no_runtime_or_start_behavior() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/@page.moth", ""),
        (
            "src/main.moth",
            "@intro\nfrom_intro #String = intro.content\n",
        ),
        ("src/intro.mtf", "# Heading"),
    ]);

    let (headers, mut string_table, path_fork) = fixture
        .prepare_and_bind_headers_for(&[
            "@html/@mod.moth",
            "src/intro.mtf",
            "src/main.moth",
            "src/@page.moth",
        ])
        .expect("headers should parse");

    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "module with empty entry should have no runtime fragments"
    );
    let content_name = string_table.intern("content");
    assert!(
        headers.top_level_const_fragments.is_empty(),
        "no top-level const fragments from non-entry files"
    );
    let moth_template_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|h| {
            h.declaration_path != PathId::ROOT
                && path_fork
                    .try_component(h.declaration_path)
                    .is_some_and(|component| component == content_name)
        })
        .collect();

    assert_eq!(
        moth_template_headers.len(),
        1,
        ".mtf file should contribute exactly one header"
    );
    assert!(
        matches!(moth_template_headers[0].kind, HeaderKind::Constant { .. }),
        ".mtf header should be a constant, got {:?}",
        moth_template_headers[0].kind
    );

    let (ast, ast_string_table, mut ast_path_fork) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/intro.mtf",
            "src/main.moth",
            "src/@page.moth",
        ])
        .expect("module AST should build");

    // Only a function whose declaration path descends from the imported `.mtf` file's own
    // path domain would violate the template's no-runtime contract. The entry root may
    // contribute its implicit `start`, and the `@html` provider module emits its own functions.
    let template_source_path = fixture.source_path_for_fixture_path("src/intro.mtf");
    let template_source_id = fixture
        .source_files
        .get_by_canonical_path(&template_source_path)
        .expect("imported template source should be registered")
        .id;
    let template_source = fixture
        .source_files
        .logical_path_in_fork(template_source_id, &mut ast_path_fork)
        .expect("template logical path should re-intern into the AST path fork");
    let template_function_nodes: Vec<_> = ast
        .nodes
        .iter()
        .filter(|node| match &node.kind {
            NodeKind::Function(name, ..) => ast_path_fork.starts_with(*name, template_source),
            _ => false,
        })
        .collect();
    assert!(
        template_function_nodes.is_empty(),
        "imported .mtf file should not produce any AST function nodes"
    );

    fixture.assert_ast_contains_moth_template_content(&ast, &ast_string_table, "src/intro.mtf");
}

#[test]
fn moth_template_dynamic_loop_condition_rejected_by_const_folding() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[loop show: visible]")]);
    let diagnostic = fixture
        .compile_moth_template_diagnostic("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    assert!(
        matches!(diagnostic.kind, DiagnosticKind::Rule(_)),
        "dynamic Moth template loop conditions should use normal const diagnostics, got {diagnostic:?}"
    );
}

#[test]
fn moth_template_external_prelude_call_rejected_by_const_folding() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[io.line([: [\"test\"]])]")]);
    let diagnostic = fixture
        .compile_moth_template_diagnostic("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    assert!(
        !matches!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
        ),
        "external prelude calls should fail through semantic const-template rules, got {diagnostic:?}"
    );
}

// -----------------------------------------------------------------------------
// TIR-backed construction alignment tests
// -----------------------------------------------------------------------------
//
// WHAT: prove that `.mtf` files reach the same normal template parsing and TIR
// construction path as authored `$md` templates, rather than a
// Moth template-specific old-authority object.

#[test]
fn moth_template_retains_body_range_and_normal_markdown_payload() {
    let (output, string_table, _span_builder) = prepare_directly("# Heading\n\nParagraph.");
    let header = content_header(&output);
    let declaration = content_constant(&output);

    assert!(
        declaration.initializer_range.is_none(),
        "Moth template declaration shell must not retain wrapper or body tokens"
    );
    assert!(
        body_has_tag(&output, TokenTag::STRING_SLICE_LITERAL),
        "body range should contain the original markdown tokens"
    );
    let Some(SyntheticContentPayload::MothTemplate { markdown_directive }) =
        header.synthetic_content_payload
    else {
        panic!("expected Moth template payload");
    };
    assert_eq!(string_table.resolve(markdown_directive), "md");
    assert!(
        !body_has_tag(&output, TokenTag::TEMPLATE_CLOSE),
        "canonical body range must not include a fabricated wrapper close"
    );
    assert!(
        declaration.initializer_references.is_empty(),
        "pure markdown body should not introduce symbol references"
    );
}

#[test]
fn empty_moth_template_body_folds_to_empty_string() {
    let (ast, string_table, path_fork) = ast_from_moth_template_source("");

    assert_eq!(
        folded_content_value(&ast, &string_table, &path_fork),
        "",
        "an empty Moth template body should fold to an empty string"
    );
}

#[test]
fn moth_template_folded_output_matches_authored_markdown_template() {
    let source = "# Heading";
    let (bd_ast, bd_string_table, bd_path_fork) = ast_from_moth_template_source(source);
    let bd_folded = folded_content_value(&bd_ast, &bd_string_table, &bd_path_fork);

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = PathBuf::from("src/content.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let (prepared_file, _) = prepare_moth_source(
        &format!("content #= [$md: {source}]"),
        &file_path,
        &entry_file_path,
        &mut string_table,
        &mut path_fork,
    );

    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let project_path = std::env::temp_dir();
    let project_path_resolver = ProjectPathResolver::new_with_module_roots(
        project_path.clone(),
        project_path,
        PreparedSourcePackageRoots::default(),
        &SourceFileKindRegistry::default(),
        ModuleRootTable::empty(),
    )
    .expect("test project path resolver should build");

    let prepared_syntax = prepare_header_syntax(
        &mut [prepared_file],
        &mut string_table,
        &mut |source, diagnostic| diagnostic.capture_preparation_span(source),
        &mut path_fork,
    )
    .expect("authored md header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        Some(&project_path_resolver),
        &crate::compiler_frontend::source::SourceDatabase::empty(),
        &mut string_table,
        &mut path_fork,
    )
    .expect("authored md headers should bind");
    let sorted_headers = resolve_module_dependencies(
        headers,
        &ContentSourceTargets::empty(),
        &mut string_table,
        &mut path_fork,
    )
    .expect("headers should sort");
    let entry_dir = path_fork
        .try_intern_portable_path("src/@page.moth", &mut string_table)
        .expect("test path fits");

    let authored_ast = Ast::new(
        AstBuildInput {
            headers: sorted_headers.headers,
            source_token_owners: sorted_headers.source_token_owners,
            module_symbols: sorted_headers.module_symbols,
            binding_environment: sorted_headers.binding_environment,
            top_level_const_fragments: sorted_headers.top_level_const_fragments,
            source_build_config_contract_names: Arc::new(Default::default()),
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry,
            style_directives: &StyleDirectiveRegistry::built_ins(),
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            entry_dir,
            build_profile: FrontendBuildProfile::Dev,
            file_value_resolution: None,
            config_resolution: None,
            build_config_values: Arc::new(Default::default()),
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            capacity_estimate: Default::default(),
            #[cfg(feature = "timers")]
            timing_context: None,
            #[cfg(feature = "timers")]
            timing_metric_family: crate::compiler_frontend::ast::AstTimingMetricFamily::Frontend,
        },
    )
    .expect("authored md template constant should build through AST")
    .ast;
    let authored_folded =
        folded_constant_value(&authored_ast, &string_table, &path_fork, "content");
    assert_eq!(
        bd_folded, authored_folded,
        "Moth template folded output should match authored $md template folded output"
    );
}
