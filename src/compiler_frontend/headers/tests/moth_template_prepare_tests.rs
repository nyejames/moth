//! Moth template synthetic-header preparation tests.
//!
//! WHAT: verifies that `.mtf` files enter the frontend as one normal private
//! `content #String` constant with a structurally generated `$md` template initializer.

use super::prepare_moth_template_file;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
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
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderKind, HeaderParseOptions,
    bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::headers::types::{FileRole, HeaderExportMode};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::module_dependencies::resolve_module_dependencies;
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::pipeline::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicDiagnosticLocation, PublicExportDiagnosticProvenance, PublicSemanticInterface,
    SourceProviderDependency, SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginConstantId, OriginDeclarationId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

fn prepare_directly(source: &str) -> (FileFrontendPrepareOutput, StringTable) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.mtf", &mut string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("Moth template body should tokenize");

    let output = prepare_moth_template_file(file_tokens, &mut string_table)
        .expect("freshly tokenized Moth template should own a preparing path table");
    (output, string_table)
}

#[test]
fn preparation_preserves_invalid_path_table_lifecycle_as_compiler_error() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("invalid-lifecycle.mtf", &mut string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut file_tokens = tokenize(
        "# Heading",
        &source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should have a tokenizer entry mode"),
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("test Moth template should tokenize");
    let _path_syntax = file_tokens
        .take_preparing_path_syntax()
        .expect("fresh token stream should own its preparing path table");

    let error = match prepare_moth_template_file(file_tokens, &mut string_table) {
        Ok(_) => panic!("a deferred source token stream must fail through CompilerError"),
        Err(error) => error,
    };

    assert!(
        format!("{error:?}").contains("deferred stream"),
        "error should retain the path-table lifecycle failure: {error:?}"
    );
}

fn prepare_via_pipeline(
    source: &str,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let source_files = SourceFileTable::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let entry_file_path = PathBuf::from("src/@page.moth");
    let options = HeaderParseOptions::default();
    let context = FrontendFilePrepareContext {
        source_files: &source_files,
        style_directives: &style_directives,
        entry_file_path: entry_file_path.as_path(),
        options: &options,
    };
    let input_path = PathBuf::from("src/intro.mtf");
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate {
            source_code: source.to_owned(),
            source_path: input_path,
        },
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };
    let mut string_table = StringTable::new();

    CompilerFrontend::prepare_file_frontend_local(&context, input, &mut string_table)
}

fn ast_from_moth_template_source(source: &str) -> (Ast, StringTable) {
    let source_files = SourceFileTable::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let project_path = std::env::temp_dir();
    let project_path_resolver = ProjectPathResolver::new(
        project_path.clone(),
        project_path,
        crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
        &SourceFileKindRegistry::default(),
    )
    .expect("test project path resolver should build");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let options = HeaderParseOptions {
        entry_file_id: None,
        project_path_resolver: Some(project_path_resolver.clone()),
        active_root_role: crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
    };
    let context = FrontendFilePrepareContext {
        source_files: &source_files,
        style_directives: &style_directives,
        entry_file_path: entry_file_path.as_path(),
        options: &options,
    };
    let input_path = PathBuf::from("src/intro.mtf");
    let input = FrontendFilePrepareInput {
        source: FrontendFilePrepareSource::MothTemplate {
            source_code: source.to_owned(),
            source_path: input_path,
        },
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };
    let mut string_table = StringTable::new();
    let mut prepared_file =
        CompilerFrontend::prepare_file_frontend_local(&context, input, &mut string_table)
            .expect("Moth template source should prepare");
    prepared_file
        .freeze_path_syntax(&string_table)
        .expect("single-file Moth template output should satisfy the prepared-file invariant gate");

    let prepared_syntax = prepare_header_syntax(vec![prepared_file], &mut string_table)
        .expect("Moth template header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        Some(&project_path_resolver),
        &mut string_table,
    )
    .expect("Moth template headers should bind");
    let sorted_headers =
        resolve_module_dependencies(headers, &mut string_table).expect("headers should sort");
    let entry_dir = InternedPath::from_single_str("src/@page.moth", &mut string_table);

    let ast = Ast::new(
        AstBuildInput {
            headers: sorted_headers.headers,
            module_symbols: sorted_headers.module_symbols,
            binding_environment: sorted_headers.binding_environment,
            top_level_const_fragments: sorted_headers.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry: Arc::clone(&external_package_registry),
            style_directives: &style_directives,
            string_table: &mut string_table,
            entry_dir,
            build_profile: FrontendBuildProfile::Dev,
            project_path_resolver: Some(project_path_resolver),
            path_format_config: PathStringFormatConfig::default(),
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

    (ast, string_table)
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
    source_files: SourceFileTable,
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
        let source_files = SourceFileTable::build(
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
    ) -> Result<(Ast, StringTable), Box<CompilerDiagnostic>> {
        let (ast, string_table) = self.compile_module_ast(prepared_relative_paths)?;

        self.assert_ast_contains_moth_template_content(
            &ast,
            &string_table,
            moth_template_relative_path,
        );

        Ok((ast, string_table))
    }

    fn compile_moth_template_ast_with_providers(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
    ) -> Result<(Ast, StringTable), Box<CompilerDiagnostic>> {
        let (ast, string_table) = self.compile_module_ast_with_providers(
            prepared_relative_paths,
            source_provider_dependencies,
        )?;

        self.assert_ast_contains_moth_template_content(
            &ast,
            &string_table,
            moth_template_relative_path,
        );

        Ok((ast, string_table))
    }

    fn compile_module_ast(
        &self,
        prepared_relative_paths: &[&str],
    ) -> Result<(Ast, StringTable), Box<CompilerDiagnostic>> {
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
    ) -> Result<(Ast, StringTable), Box<CompilerDiagnostic>> {
        let (headers, mut string_table) = self.prepare_and_bind_headers_with_providers(
            prepared_relative_paths,
            source_provider_dependencies,
        )?;
        let sorted_headers = resolve_module_dependencies(headers, &mut string_table)
            .map_err(first_diagnostic_from_bag)?;
        let entry_dir =
            InternedPath::try_from_filesystem_path(&self.entry_file_path, &mut string_table)
                .expect("test path should be UTF-8");
        let style_directives = StyleDirectiveRegistry::built_ins();
        let external_package_registry = Arc::new(ExternalPackageRegistry::new());

        Ast::new(
            AstBuildInput {
                headers: sorted_headers.headers,
                module_symbols: sorted_headers.module_symbols,
                binding_environment: sorted_headers.binding_environment,
                top_level_const_fragments: sorted_headers.top_level_const_fragments,
            },
            AstBuildContext {
                root_role: ModuleRootRole::Normal,
                external_package_registry: Arc::clone(&external_package_registry),
                style_directives: &style_directives,
                string_table: &mut string_table,
                entry_dir,
                build_profile: FrontendBuildProfile::Dev,
                project_path_resolver: Some(self.project_path_resolver.clone()),
                path_format_config: PathStringFormatConfig::default(),
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
            messages
                .first_error()
                .cloned()
                .map(Box::new)
                .unwrap_or_else(|| panic!("AST failed without a diagnostic"))
        })
        .map(|build_result| (build_result.ast, string_table))
    }

    fn assert_ast_contains_moth_template_content(
        &self,
        ast: &Ast,
        string_table: &StringTable,
        moth_template_relative_path: &str,
    ) {
        let logical_moth_template_path = moth_template_relative_path
            .strip_prefix("src/")
            .unwrap_or(moth_template_relative_path);
        let content_suffix = format!("{logical_moth_template_path}/content");
        assert!(
            ast.module_constants.iter().any(|constant| {
                constant.id.name_str(string_table) == Some("content")
                    && constant
                        .id
                        .to_portable_string(string_table)
                        .ends_with(&content_suffix)
            }),
            "compiled AST should include Moth template content for {moth_template_relative_path}"
        );
    }

    fn prepare_and_bind_headers_for(
        &self,
        prepared_relative_paths: &[&str],
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
        ),
        Box<CompilerDiagnostic>,
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
        self.prepare_and_bind_headers_with_providers(
            prepared_relative_paths,
            &provider_dependencies,
        )
    }

    fn prepare_and_bind_headers_with_providers(
        &self,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
        ),
        Box<CompilerDiagnostic>,
    > {
        self.prepare_and_bind_headers_with_providers_with_table(
            prepared_relative_paths,
            source_provider_dependencies,
        )
        .map_err(|(diagnostic, _string_table)| diagnostic)
    }

    fn prepare_and_bind_headers_with_providers_with_table(
        &self,
        prepared_relative_paths: &[&str],
        source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<'_>,
    ) -> Result<
        (
            crate::compiler_frontend::headers::parse_file_headers::BoundModuleHeaders,
            StringTable,
        ),
        (Box<CompilerDiagnostic>, StringTable),
    > {
        let style_directives = StyleDirectiveRegistry::built_ins();
        let external_package_registry = Arc::new(ExternalPackageRegistry::new());
        let options = HeaderParseOptions {
            entry_file_id: None,
            project_path_resolver: Some(self.project_path_resolver.clone()),
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

        for relative_path in prepared_relative_paths {
            let source_path = self.source_path_for_fixture_path(relative_path);
            let source_code = fs::read_to_string(&source_path).expect("source should be readable");
            let source_kind = source_path
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(SourceFileKind::from_extension)
                .unwrap_or(SourceFileKind::Moth);

            let source = match source_kind {
                SourceFileKind::Moth => {
                    let tokens = CompilerFrontend::tokenize_source(
                        &self.source_files,
                        &style_directives,
                        &source_code,
                        &source_path,
                        TokenizerEntryMode::SourceFile,
                        &mut string_table,
                    )
                    .map_err(|error| match error {
                        FileFrontendPrepareFailure::Diagnosed(error) => {
                            (error.diagnostic, string_table.clone())
                        }
                        FileFrontendPrepareFailure::Infrastructure(error) => {
                            panic!("fixture tokenization hit infrastructure failure: {error:?}")
                        }
                    })?;
                    FrontendFilePrepareSource::Moth {
                        source_path,
                        tokens: Box::new(tokens),
                    }
                }
                SourceFileKind::MothTemplate => FrontendFilePrepareSource::MothTemplate {
                    source_code,
                    source_path,
                },
                SourceFileKind::PlainMarkdown => FrontendFilePrepareSource::PlainMarkdown {
                    source_code,
                    source_path,
                },
            };

            let input = FrontendFilePrepareInput {
                source,
                const_template_offset: 0,
                runtime_fragment_offset: 0,
            };

            let mut output =
                CompilerFrontend::prepare_file_frontend_local(&context, input, &mut string_table)
                    .map_err(|error| match error {
                    FileFrontendPrepareFailure::Diagnosed(error) => {
                        (error.diagnostic, string_table.clone())
                    }
                    FileFrontendPrepareFailure::Infrastructure(error) => {
                        panic!("fixture preparation hit infrastructure failure: {error:?}")
                    }
                })?;
            output
                .freeze_path_syntax(&string_table)
                .expect("fixture prepared output should satisfy the prepared-file invariant gate");
            prepared_files.push(output);
        }

        let prepared_syntax = prepare_header_syntax(prepared_files, &mut string_table)
            .map_err(|bag| (first_diagnostic_from_bag(bag), string_table.clone()))?;
        let headers = bind_module_headers(
            prepared_syntax,
            &external_package_registry,
            &ExternalImportResolutionTable::default(),
            source_provider_dependencies,
            Some(&self.project_path_resolver),
            &mut string_table,
        )
        .map_err(|bag| (first_diagnostic_from_bag(bag), string_table.clone()))?;

        Ok((headers, string_table))
    }

    fn compile_moth_template_ast_ok(
        &self,
        moth_template_relative_path: &str,
        prepared_relative_paths: &[&str],
    ) -> (Ast, StringTable) {
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
            Err(diagnostic) => *diagnostic,
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

fn first_diagnostic_from_bag(bag: DiagnosticBag) -> Box<CompilerDiagnostic> {
    Box::new(
        bag.into_diagnostics()
            .into_iter()
            .next()
            .expect("diagnostic bag should contain an error"),
    )
}

fn prepare_moth_source(
    source: &str,
    file_path: &Path,
    entry_file_path: &Path,
    string_table: &mut StringTable,
) -> FileFrontendPrepareOutput {
    let source_path = InternedPath::try_from_filesystem_path(file_path, string_table)
        .expect("test path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        Some(FileId(0)),
    )
    .expect("Moth source should tokenize");

    prepare_file_from_tokens(
        file_tokens,
        entry_file_path,
        &HeaderParseOptions::default(),
        string_table,
        0,
        0,
    )
    .expect("Moth header preparation should succeed")
}

fn content_constant(
    output: &FileFrontendPrepareOutput,
) -> &crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax {
    assert_eq!(output.headers.len(), 1);

    let HeaderKind::Constant { declaration, .. } = &output.headers[0].kind else {
        panic!("Moth template should produce a constant header");
    };

    declaration
}

fn initializer_kinds(output: &FileFrontendPrepareOutput) -> Vec<&TokenKind> {
    content_constant(output)
        .initializer_tokens
        .iter()
        .map(|token| &token.kind)
        .collect()
}

fn folded_content_value(ast: &Ast, string_table: &StringTable) -> String {
    let content = ast
        .module_constants
        .iter()
        .find(|constant| constant.id.name_str(string_table) == Some("content"))
        .expect("Moth template content constant should exist");

    let ExpressionKind::StringSlice(value) = &content.value.kind else {
        panic!(
            "Moth template content should fold to a string slice, got {:?}",
            content.value.kind
        );
    };

    string_table.resolve(*value).to_owned()
}

fn folded_content_contains(ast: &Ast, string_table: &StringTable, expected: &str) {
    let content = folded_content_value(ast, string_table);
    assert!(
        content.contains(expected),
        "folded content should contain {expected:?}, got {content:?}"
    );
}

fn folded_constant_value(ast: &Ast, string_table: &StringTable, name: &str) -> String {
    let constant = ast
        .module_constants
        .iter()
        .find(|constant| constant.id.name_str(string_table) == Some(name))
        .unwrap_or_else(|| panic!("module constant {name} should exist"));

    let ExpressionKind::StringSlice(value) = &constant.value.kind else {
        panic!(
            "module constant {name} should fold to a string slice, got {:?}",
            constant.value.kind
        );
    };

    string_table.resolve(*value).to_owned()
}

#[test]
fn moth_template_preparation_produces_private_content_constant() {
    let (output, string_table) = prepare_directly("# Heading");
    let header = &output.headers[0];
    let declaration = content_constant(&output);

    assert_eq!(output.file_role, FileRole::Normal);
    assert!(output.file_dependency_clauses.is_empty());
    assert!(output.top_level_const_fragments.is_empty());
    assert_eq!(output.runtime_fragment_count, 0);
    assert_eq!(output.const_template_count, 0);
    assert_eq!(header.export_mode, HeaderExportMode::Private);
    assert_eq!(
        header.tokens.src_path.to_portable_string(&string_table),
        "test.mtf/content"
    );
    assert_eq!(
        header.source_file.to_portable_string(&string_table),
        "test.mtf"
    );
    assert_eq!(header.tokens.canonical_os_path, output.canonical_os_path);
    assert_eq!(declaration.binding_mode, BindingMode::CompileTimeConstant);
    assert!(matches!(
        declaration.type_annotation,
        ParsedTypeRef::BuiltinString { .. }
    ));
}

#[test]
fn empty_moth_template_body_folds_to_empty_string() {
    let (ast, string_table) = ast_from_moth_template_source("");

    assert_eq!(folded_content_value(&ast, &string_table), "");
}

#[test]
fn simple_markdown_body_folds_like_markdown_template() {
    let (ast, string_table) = ast_from_moth_template_source("# Heading");

    assert_eq!(
        folded_content_value(&ast, &string_table),
        "<h1>Heading</h1>"
    );
}

#[test]
fn nested_moth_template_defaults_to_markdown_formatting() {
    let (ast, string_table) = ast_from_moth_template_source("[:# Nested]");

    assert_eq!(folded_content_value(&ast, &string_table), "<h1>Nested</h1>");
}

#[test]
fn explicit_nested_raw_directive_overrides_moth_template_markdown_default() {
    let (ast, string_table) = ast_from_moth_template_source("[$raw:# Nested]");

    assert_eq!(folded_content_value(&ast, &string_table), "# Nested");
}

#[test]
fn explicit_nested_non_formatter_directive_overrides_moth_template_markdown_default() {
    let (ast, string_table) = ast_from_moth_template_source("[$fresh:# Nested]");

    assert_eq!(folded_content_value(&ast, &string_table), "# Nested");
}

#[test]
fn moth_template_compile_time_if_folds_inside_content_constant() {
    let (ast, string_table) = ast_from_moth_template_source("[if true: visible]");

    folded_content_contains(&ast, &string_table, "visible");
}

#[test]
fn moth_template_compile_time_collection_loop_folds_inside_content_constant() {
    let (ast, string_table) =
        ast_from_moth_template_source(r#"[loop {"one", "two"} |item|: [item] ]"#);

    let content = folded_content_value(&ast, &string_table);
    assert!(
        content.contains("one") && content.contains("two"),
        "folded loop content should contain both collection items, got {content:?}"
    );
}

#[test]
fn empty_moth_template_body_generates_markdown_template_initializer() {
    let (output, string_table) = prepare_directly("");
    let kinds = initializer_kinds(&output);

    assert_eq!(kinds.len(), 4);
    assert!(matches!(kinds[0], TokenKind::TemplateHead));
    assert!(matches!(
        kinds[1],
        TokenKind::StyleDirective(id) if string_table.resolve(*id) == "md"
    ));
    assert!(matches!(kinds[2], TokenKind::StartTemplateBody));
    assert!(matches!(kinds[3], TokenKind::TemplateClose));
}

#[test]
fn simple_markdown_body_uses_original_body_token_location() {
    let (output, string_table) = prepare_directly("# Heading");
    let declaration = content_constant(&output);
    let body_token = declaration
        .initializer_tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
        .expect("body text should be preserved as a string literal token");

    assert_eq!(
        body_token.location.scope.to_portable_string(&string_table),
        "test.mtf"
    );
    assert!(matches!(
        &body_token.kind,
        TokenKind::StringSliceLiteral(id) if string_table.resolve(*id) == "# Heading"
    ));
}

#[test]
fn nested_templates_remain_structural_inside_markdown_initializer() {
    let (output, string_table) = prepare_directly("before [:inner] after");
    let declaration = content_constant(&output);
    let template_heads = declaration
        .initializer_tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .collect::<Vec<_>>();

    assert_eq!(template_heads.len(), 2);
    assert_eq!(
        template_heads[1]
            .location
            .scope
            .to_portable_string(&string_table),
        "test.mtf"
    );
    assert!(
        template_heads[1].location.start_pos.char_column > 0,
        "nested template opener should keep its original body position, not the synthetic start"
    );
}

#[test]
fn backslash_remains_body_text_inside_markdown_initializer() {
    let (output, string_table) = prepare_directly(r"before \n after");
    let declaration = content_constant(&output);

    assert!(declaration.initializer_tokens.iter().any(|token| matches!(
        &token.kind,
        TokenKind::StringSliceLiteral(id) if string_table.resolve(*id) == r"before \n after"
    )));
}

#[test]
fn unescaped_outer_close_diagnostic_flows_through_pipeline_preparation() {
    let Err(error) = prepare_via_pipeline("]") else {
        panic!("unescaped implicit Moth template close should fail during preparation");
    };
    let FileFrontendPrepareFailure::Diagnosed(error) = error else {
        panic!("unescaped template close must be a source diagnostic");
    };

    assert!(error.warnings.is_empty());
    assert_eq!(
        error.diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
    );
    assert!(matches!(
        &error.diagnostic.payload,
        DiagnosticPayload::UnescapedImplicitTemplateClose {
            source_kind: SourceFileKind::MothTemplate
        }
    ));
}

#[test]
fn double_dash_remains_body_text() {
    let (output, string_table) = prepare_directly("alpha -- still text\nbeta");
    let declaration = content_constant(&output);

    assert!(declaration.initializer_tokens.iter().any(|token| matches!(
        &token.kind,
        TokenKind::StringSliceLiteral(id)
            if string_table.resolve(*id) == "alpha -- still text\nbeta"
    )));
}

#[test]
fn declaration_like_text_remains_markdown_body_text() {
    let (output, string_table) = prepare_directly("@docs/intro\ncontent #String = value");
    let declaration = content_constant(&output);

    assert!(declaration.initializer_references.is_empty());
    assert!(declaration.initializer_tokens.iter().any(|token| matches!(
        &token.kind,
        TokenKind::StringSliceLiteral(id)
            if string_table.resolve(*id) == "@docs/intro\ncontent #String = value"
    )));
}

#[test]
fn module_root_export_syntax_can_target_moth_template_content() {
    let mut string_table = StringTable::new();
    let root_file_path = PathBuf::from("src/@mod.moth");
    let entry_path = PathBuf::from("src/@page.moth");

    let root_output = prepare_moth_source(
        "export:\n    @intro content as intro\n;\n",
        &root_file_path,
        &entry_path,
        &mut string_table,
    );

    assert_eq!(root_output.file_dependency_clauses.len(), 1);
    assert_eq!(
        root_output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Public
    );
    assert_eq!(
        root_output.file_dependency_clauses[0]
            .dependency
            .path
            .to_portable_string(&string_table),
        "intro"
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
    let (ast, string_table) = fixture
        .compile_moth_template_ast_ok("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    folded_content_contains(&ast, &string_table, "<p>");
}

#[test]
fn moth_template_header_visibility_contains_implicit_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[("src/intro.mtf", "[p]")]);
    let (headers, mut string_table) = fixture
        .prepare_and_bind_headers_for(&["@html/@mod.moth", "src/intro.mtf"])
        .expect("headers should parse");
    let moth_template_canonical_path = fixture.project_root_path().join("src/intro.mtf");
    let moth_template_logical_path = fixture
        .project_path_resolver
        .logical_path_for_canonical_file(&moth_template_canonical_path, &mut string_table)
        .expect("Moth template logical path should resolve");
    let moth_template_source =
        InternedPath::try_from_filesystem_path(&moth_template_logical_path, &mut string_table)
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
    let (ast, string_table) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/intro.mtf",
        ],
    );

    folded_content_contains(&ast, &string_table, "from root");
}

#[test]
fn moth_template_without_same_directory_root_sees_only_html_constants() {
    let fixture = MothTemplateScopeFixture::new(&[("src/docs/intro.mtf", "[collision]")]);
    let (ast, string_table) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &["@html/@mod.moth", "src/docs/intro.mtf"],
    );

    folded_content_contains(&ast, &string_table, "html");
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
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::ImportNameCollision {
            previous_location: Some(_),
            ..
        }
    ));
    assert_eq!(diagnostic.labels.len(), 2);
    assert!(
        diagnostic
            .labels
            .iter()
            .all(|label| !label.location.scope.is_empty())
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
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String("from html".to_owned()),
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
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String("from custom".to_owned()),
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

    let (ast, string_table) = fixture
        .compile_moth_template_ast_with_providers(
            "src/intro.mtf",
            &["src/intro.mtf"],
            &provider_dependencies,
        )
        .expect(".mtf body should see @html constant through provider interface");

    folded_content_contains(&ast, &string_table, "from html");
    folded_content_contains(&ast, &string_table, "from custom");
}

#[test]
fn provider_interface_collision_remaps_authored_declaration_location() {
    let fixture = MothTemplateScopeFixture::new(&[
        (
            "src/docs/@mod.moth",
            "export:\n    collision #= \"local\"\n;\n",
        ),
        ("src/docs/intro.mtf", "[collision]"),
    ]);

    let html_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("html"),
        "html".to_owned(),
        ModuleRootRole::Normal,
    );
    let collision_origin = OriginDeclarationId::Constant(OriginConstantId::new(
        html_origin.clone(),
        "collision".to_owned(),
    ));
    let html_interface = PublicSemanticInterface {
        module_origin: html_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            html_origin,
            "collision".to_owned(),
            collision_origin.clone(),
        )],
        export_diagnostic_provenance: vec![PublicExportDiagnosticProvenance {
            public_name: "collision".to_owned(),
            location: PublicDiagnosticLocation {
                scope_components: vec!["@html".to_owned(), "@mod.moth".to_owned()],
                start_line: 88,
                start_column: 5,
                end_line: 88,
                end_column: 10,
            },
        }],
        binding_exports: Vec::new(),
        declarations: vec![PublicDeclarationRecord {
            origin: collision_origin,
            semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                folded_value: PublicFoldedValue::String("from html".to_owned()),
            }),
        }],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let provider_dependencies = SourceProviderDependencySet::new(vec![SourceProviderDependency {
        kind:
            crate::compiler_frontend::public_interface::ProviderDependencyKind::ImplicitTemplate {
                package_prefix: "html",
            },
        interface: &html_interface,
    }])
    .expect("one implicit template provider should register");

    let (diagnostic, diagnostic_string_table) = match fixture
        .prepare_and_bind_headers_with_providers_with_table(
            &["src/docs/@mod.moth", "src/docs/intro.mtf"],
            &provider_dependencies,
        ) {
        Ok(_) => panic!("provider and same-directory constants should collide"),
        Err((diagnostic, string_table)) => (*diagnostic, string_table),
    };

    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Import(
            crate::compiler_frontend::compiler_messages::ImportDiagnosticKind::ImportNameCollision
        )
    ));
    let primary = diagnostic
        .labels
        .iter()
        .find(|label| label.style == DiagnosticLabelStyle::Primary)
        .expect("collision should have a primary label");
    let secondary = diagnostic
        .labels
        .iter()
        .find(|label| label.style == DiagnosticLabelStyle::Secondary)
        .expect("collision should preserve the provider declaration label");
    assert_eq!(primary.location.start_pos.line_number, 1);
    assert_eq!(secondary.location.start_pos.line_number, 88);
    assert_eq!(secondary.location.start_pos.char_column, 5);
    assert_eq!(
        secondary
            .location
            .scope
            .to_portable_string(&diagnostic_string_table),
        "@html/@mod.moth"
    );
    assert_ne!(
        primary.location.start_pos.line_number,
        secondary.location.start_pos.line_number
    );
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
    let (ast, string_table) = fixture
        .compile_moth_template_ast_ok("src/intro.mtf", &["@html/@mod.moth", "src/intro.mtf"]);

    folded_content_contains(&ast, &string_table, "green");
}

#[test]
fn root_supplied_content_constant_can_be_referenced_normally() {
    let fixture = MothTemplateScopeFixture::new(&[
        ("src/docs/@mod.moth", "export:\n    @other content\n;\n"),
        ("src/docs/other.mtf", "shared body"),
        ("src/docs/intro.mtf", "[content]"),
    ]);
    let (ast, string_table) = fixture.compile_moth_template_ast_ok(
        "src/docs/intro.mtf",
        &[
            "@html/@mod.moth",
            "src/docs/@mod.moth",
            "src/docs/other.mtf",
            "src/docs/intro.mtf",
        ],
    );

    folded_content_contains(&ast, &string_table, "shared body");
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
    let (ast, string_table) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/intro.mtf",
            "src/main.moth",
            "src/@page.moth",
        ])
        .expect("module using imported Moth template content should compile through AST");

    assert_eq!(
        folded_constant_value(&ast, &string_table, "from_intro"),
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
    let (ast, string_table) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/main.moth",
            "src/@page.moth",
            "src/intro.mtf",
        ])
        .expect("module using namespace-imported Moth template content should compile through AST");

    assert_eq!(
        folded_constant_value(&ast, &string_table, "from_intro"),
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

    let (headers, string_table) = fixture
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
    assert!(
        headers.top_level_const_fragments.is_empty(),
        "no top-level const fragments from non-entry files"
    );

    let moth_template_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|h| {
            h.source_file
                .to_portable_string(&string_table)
                .ends_with("intro.mtf")
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

    let (ast, ast_string_table) = fixture
        .compile_module_ast(&[
            "@html/@mod.moth",
            "src/intro.mtf",
            "src/main.moth",
            "src/@page.moth",
        ])
        .expect("module AST should build");

    let bd_function_nodes: Vec<_> = ast
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, NodeKind::Function(..))
                && node
                    .location
                    .scope
                    .to_portable_string(&ast_string_table)
                    .ends_with("intro.mtf")
        })
        .collect();

    assert!(
        bd_function_nodes.is_empty(),
        ".mtf file should not produce any AST function nodes"
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

/// Extracts the body tokens from a synthetic markdown-template initializer,
/// i.e. everything between `StartTemplateBody` and `TemplateClose`.
fn synthetic_template_body_tokens(
    output: &FileFrontendPrepareOutput,
) -> &[crate::compiler_frontend::tokenizer::tokens::Token] {
    let declaration = content_constant(output);
    let start_index = declaration
        .initializer_tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::StartTemplateBody))
        .expect("md template initializer should have StartTemplateBody");
    let close_index = declaration
        .initializer_tokens
        .iter()
        .rposition(|token| matches!(token.kind, TokenKind::TemplateClose))
        .expect("md template initializer should have TemplateClose");

    &declaration.initializer_tokens[start_index + 1..close_index]
}

#[test]
fn moth_template_synthetic_initializer_has_normal_markdown_template_shape() {
    let (output, string_table) = prepare_directly("# Heading\n\nParagraph.");
    let declaration = content_constant(&output);
    let kinds = initializer_kinds(&output);

    assert!(
        kinds.len() >= 4,
        "md template initializer should have wrapper tokens plus body tokens, got {kinds:?}"
    );
    assert!(
        matches!(kinds[0], TokenKind::TemplateHead),
        "first token should be TemplateHead, got {:?}",
        kinds[0]
    );
    assert!(
        matches!(
            kinds[1],
            TokenKind::StyleDirective(id) if string_table.resolve(*id) == "md"
        ),
        "second token should be $md style directive, got {:?}",
        kinds[1]
    );
    assert!(
        matches!(kinds[2], TokenKind::StartTemplateBody),
        "third token should be StartTemplateBody, got {:?}",
        kinds[2]
    );
    assert!(
        matches!(kinds[kinds.len() - 1], TokenKind::TemplateClose),
        "last token should be TemplateClose, got {:?}",
        kinds[kinds.len() - 1]
    );

    let body_tokens = synthetic_template_body_tokens(&output);
    assert!(
        !body_tokens.is_empty(),
        "body should contain the original markdown tokens"
    );
    assert!(
        declaration.initializer_references.is_empty(),
        "pure markdown body should not introduce symbol references"
    );
}

#[test]
fn moth_template_body_tokens_are_literal_template_body_text() {
    let source = "# Heading\n\nParagraph.";
    let (output, string_table) = prepare_directly(source);
    let body_tokens = synthetic_template_body_tokens(&output);

    assert!(
        !body_tokens.is_empty(),
        ".mtf body should tokenize into at least one body token"
    );

    let concatenated: String = body_tokens
        .iter()
        .map(|token| match &token.kind {
            TokenKind::StringSliceLiteral(id) => string_table.resolve(*id),
            other => panic!(".mtf body token should be literal text, got {other:?}"),
        })
        .collect();

    assert_eq!(
        concatenated, source,
        ".mtf body tokens should preserve the original source text as template body literals"
    );
}

#[test]
fn moth_template_folded_output_matches_authored_markdown_template() {
    let source = "# Heading";
    let (bd_ast, bd_string_table) = ast_from_moth_template_source(source);
    let bd_folded = folded_content_value(&bd_ast, &bd_string_table);

    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/content.moth");
    let entry_file_path = PathBuf::from("src/@page.moth");
    let prepared_file = prepare_moth_source(
        &format!("content #= [$md: {source}]"),
        &file_path,
        &entry_file_path,
        &mut string_table,
    );

    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let project_path = std::env::temp_dir();
    let project_path_resolver = ProjectPathResolver::new(
        project_path.clone(),
        project_path,
        crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
        &SourceFileKindRegistry::default(),
    )
    .expect("test project path resolver should build");

    let prepared_syntax = prepare_header_syntax(vec![prepared_file], &mut string_table)
        .expect("authored md header syntax should prepare");
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderDependencySet::default(),
        Some(&project_path_resolver),
        &mut string_table,
    )
    .expect("authored md headers should bind");
    let sorted_headers =
        resolve_module_dependencies(headers, &mut string_table).expect("headers should sort");
    let entry_dir = InternedPath::from_single_str("src/@page.moth", &mut string_table);

    let authored_ast = Ast::new(
        AstBuildInput {
            headers: sorted_headers.headers,
            module_symbols: sorted_headers.module_symbols,
            binding_environment: sorted_headers.binding_environment,
            top_level_const_fragments: sorted_headers.top_level_const_fragments,
        },
        AstBuildContext {
            root_role: ModuleRootRole::Normal,
            external_package_registry,
            style_directives: &StyleDirectiveRegistry::built_ins(),
            string_table: &mut string_table,
            entry_dir,
            build_profile: FrontendBuildProfile::Dev,
            project_path_resolver: Some(project_path_resolver),
            path_format_config: PathStringFormatConfig::default(),
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

    let authored_folded = folded_constant_value(&authored_ast, &string_table, "content");

    assert_eq!(
        bd_folded, authored_folded,
        "Moth template folded output should match authored $md template folded output"
    );
}
