//! Per-module frontend orchestration regression tests.
//!
//! WHAT: validates Stage 0/frontend boundary helpers such as message merging and parallel local-table
//!       file preparation.
//! WHY: these tests exercise infrastructure invariants that integration cases cannot inspect
//!      directly, while keeping test code out of the production orchestration module.

use super::super::prepared_source::PreparedSourceInput;
use super::merge_stage_messages;
use crate::builder_surface::SourceFileKindRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::compiler_errors::{CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, HeaderKind, HeaderParseOptions, PreparedHeaderSyntax,
    bind_module_headers, prepare_header_syntax,
};
use crate::compiler_frontend::paths::module_roots::{ModuleRootRecord, ModuleRootTable};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};
use crate::compiler_frontend::{
    FrontendBuildProfile, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};
use crate::projects::settings::Config;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn moth_prepared_input(
    source_path: PathBuf,
    source_code: &str,
    tokens: FileTokens,
) -> PreparedSourceInput {
    PreparedSourceInput::Moth {
        source_byte_len: source_code.len(),
        source_path,
        tokens: Box::new(tokens),
    }
}

/// Tokenize source text against a source file table and string table, then build a Moth
/// `PreparedSourceInput` carrying the retained token stream.
fn tokenized_moth_prepared_input(
    source_files: &SourceFileTable,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    source_path: PathBuf,
    source_code: &str,
) -> PreparedSourceInput {
    let tokens = CompilerFrontend::tokenize_source(
        source_files,
        style_directives,
        source_code,
        &source_path,
        TokenizerEntryMode::SourceFile,
        string_table,
    )
    .expect("test source should tokenize");
    moth_prepared_input(source_path, source_code, tokens)
}

fn source_byte_count(input_files: &[PreparedSourceInput]) -> usize {
    input_files
        .iter()
        .map(PreparedSourceInput::source_byte_len)
        .sum()
}

struct FrontendPreparationFixture {
    _temp_dir: tempfile::TempDir,
    frontend: CompilerFrontend,
    input_files: Vec<PreparedSourceInput>,
    entry_file_path: PathBuf,
}

fn frontend_preparation_fixture(file_sources: &[(&str, &str)]) -> FrontendPreparationFixture {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let mut canonical_paths = Vec::new();

    for (file_name, source) in file_sources {
        let path = temp_dir.path().join(file_name);
        fs::write(&path, source).expect("test source file should be written");
        let canonical = fs::canonicalize(&path).expect("test source file should canonicalize");
        canonical_paths.push(canonical);
    }

    let entry_file_path = canonical_paths
        .first()
        .expect("fixture should include at least one source file")
        .clone();

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        canonical_paths.iter().map(PathBuf::as_path),
        &entry_file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    // Tokenize each source file once so the retained token stream is available for header
    // preparation, mirroring the single Stage 0 lexical pass in production discovery.
    let style_directives = StyleDirectiveRegistry::built_ins();
    let input_files = canonical_paths
        .iter()
        .zip(file_sources)
        .map(|(canonical, (_, source))| {
            let tokens = CompilerFrontend::tokenize_source(
                &source_files,
                &style_directives,
                source,
                canonical,
                TokenizerEntryMode::SourceFile,
                &mut string_table,
            )
            .expect("fixture source should tokenize");
            moth_prepared_input(canonical.clone(), source, tokens)
        })
        .collect();

    let mut frontend = CompilerFrontend::new(
        &Config::new(temp_dir.path().to_path_buf()),
        string_table,
        style_directives,
        Arc::new(ExternalPackageRegistry::new()),
        None,
    );
    frontend.set_source_files(source_files);

    FrontendPreparationFixture {
        _temp_dir: temp_dir,
        frontend,
        input_files,
        entry_file_path,
    }
}

fn header_source_file_names(
    headers: &PreparedHeaderSyntax,
    string_table: &StringTable,
) -> Vec<String> {
    headers
        .headers
        .iter()
        .map(|header| {
            header
                .source_file
                .to_path_buf(string_table)
                .file_name()
                .expect("test logical source path should have a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn chunked_fixture_sources() -> Vec<(String, String)> {
    (0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT)
        .map(|index| {
            (
                format!("{index}.moth"),
                format!("value_{index} #= {index}\n"),
            )
        })
        .collect()
}

fn fixture_source_refs(file_sources: &[(String, String)]) -> Vec<(&str, &str)> {
    file_sources
        .iter()
        .map(|(file_name, source)| (file_name.as_str(), source.as_str()))
        .collect()
}

#[test]
fn merge_stage_messages_preserves_render_type_context_with_warnings() {
    let string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::Assignment,
        SourceLocation::default(),
    );
    let warning = CompilerDiagnostic::unreachable_match_arm(SourceLocation::default());
    let messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table.clone())
        .with_type_context_for_all_diagnostics(type_environment);

    let merged = merge_stage_messages(messages, &[warning], &string_table);
    let rendered_lines = format_terse_compiler_messages(&merged);

    assert_eq!(merged.render_type_contexts().len(), 1);
    assert_eq!(rendered_lines.len(), 2);
    assert!(
        rendered_lines[0].contains("expected Int, found String"),
        "errors should render before warnings; first line should be the type mismatch, got: {}",
        rendered_lines[0]
    );
    assert!(
        !rendered_lines[0].contains("TypeId("),
        "type names should be rendered, not raw type ids"
    );
}

#[test]
fn fused_preparation_merges_local_forks_and_resolves_source_and_generated_strings() {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_a = temp_dir.path().join("a.moth");
    let file_b = temp_dir.path().join("b.moth");
    // File A is the entry file with a runtime statement and a const template (which generates
    // a synthetic header name during header parsing).
    fs::write(&file_a, "alpha = 1\n#[hello]\n").unwrap();
    // File B is a normal source file with an exported constant declaration.
    fs::write(&file_b, "beta #= 2\n").unwrap();

    let canonical_a = fs::canonicalize(&file_a).unwrap();
    let canonical_b = fs::canonicalize(&file_b).unwrap();

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        &[&canonical_a, &canonical_b],
        &canonical_a,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let module_table_size_before = string_table.len();

    let mut frontend = CompilerFrontend::new(
        &Config::new(temp_dir.path().to_path_buf()),
        string_table,
        StyleDirectiveRegistry::built_ins(),
        Arc::new(ExternalPackageRegistry::new()),
        None,
    );
    frontend.set_source_files(source_files);

    let options = HeaderParseOptions {
        entry_file_id: frontend
            .source_files
            .get_by_canonical_path(&canonical_a)
            .map(|i| i.file_id),
        project_path_resolver: frontend.project_path_resolver.clone(),
        active_root_role: ModuleRootRole::Normal,
    };

    // Helper to prepare one file using the local-table variant and merge its delta back
    // into the module string table, returning the remapped output.
    let mut prepare_and_merge = |source_code: &str,
                                 source_path: &std::path::PathBuf,
                                 const_template_offset: usize,
                                 runtime_fragment_offset: usize| {
        // Tokenize against the module string table before forking, mirroring Stage 0 retention.
        let retained_tokens = CompilerFrontend::tokenize_source(
            &frontend.source_files,
            &frontend.style_directives,
            source_code,
            source_path,
            TokenizerEntryMode::SourceFile,
            &mut frontend.string_table,
        )
        .expect("test source should tokenize");

        let fork_source = frontend.string_table.fork_source();
        let (mut local_string_table, base_len) = fork_source.fork_for_module().into_parts();

        let result = {
            let prepare_context = FrontendFilePrepareContext {
                source_files: &frontend.source_files,
                style_directives: &frontend.style_directives,
                entry_file_path: &canonical_a,
                options: &options,
            };
            let input = FrontendFilePrepareInput {
                source: FrontendFilePrepareSource::Moth {
                    source_path: source_path.clone(),
                    tokens: Box::new(retained_tokens),
                },
                const_template_offset,
                runtime_fragment_offset,
            };

            CompilerFrontend::prepare_file_frontend_local(
                &prepare_context,
                input,
                &mut local_string_table,
            )
        };

        let remap = frontend
            .string_table
            .merge_delta_from(&local_string_table, base_len);
        match result {
            Ok(mut output) => {
                output.remap_string_ids(&remap);
                Ok(output)
            }
            Err(mut error) => {
                error.remap_string_ids(&remap);
                Err(error)
            }
        }
    };

    // Prepare file A (entry) — tokenization creates "alpha" and "hello"; header parsing
    // creates the synthetic "#const_template0" name for the const template.
    let output_a = prepare_and_merge("alpha = 1\n#[hello]\n", &canonical_a, 0, 0)
        .expect("file A preparation should succeed");

    // Prepare file B — tokenization creates "beta".
    let output_b = prepare_and_merge(
        "beta #= 2\n",
        &canonical_b,
        output_a.const_template_count,
        output_a.runtime_fragment_count,
    )
    .expect("file B preparation should succeed");

    // The module table should have grown: source strings (alpha, hello, beta) plus
    // header-generated strings (#const_template0 and possibly others).
    assert!(
        frontend.string_table.len() > module_table_size_before + 2,
        "module table should contain source strings plus header-generated strings"
    );

    // Aggregate the remapped outputs.
    let prepared_syntax =
        prepare_header_syntax(vec![output_a, output_b], &mut frontend.string_table)
            .expect("header syntax preparation should succeed");
    let headers = bind_module_headers(
        prepared_syntax,
        &frontend.external_package_registry,
        &ExternalImportResolutionTable::default(),
        &crate::compiler_frontend::public_interface::SourceProviderImportSet::default(),
        options.project_path_resolver.as_ref(),
        &mut frontend.string_table,
    )
    .expect("header binding should succeed");

    // Verify source text string "beta" resolves through the module table in file B headers.
    let beta_header = headers
        .headers
        .iter()
        .find(|h| h.tokens.src_path.name_str(&frontend.string_table) == Some("beta"));
    assert!(
        beta_header.is_some(),
        "beta header should exist with name resolvable through module table"
    );

    // Verify header-generated string "#const_template0" resolves correctly after merge.
    let const_template_header = headers
        .headers
        .iter()
        .find(|h| matches!(h.kind, HeaderKind::ConstTemplate { .. }));
    assert!(
        const_template_header.is_some(),
        "const template header should exist"
    );
    let const_template_name = const_template_header
        .unwrap()
        .tokens
        .src_path
        .name_str(&frontend.string_table)
        .expect("const template should have a name");
    assert_eq!(
        const_template_name, "#const_template0",
        "generated const template name should resolve through module table"
    );

    // Verify token symbols inside the const template also resolve.
    let hello_token = const_template_header
        .unwrap()
        .tokens
        .tokens
        .iter()
        .find_map(|t| match &t.kind {
            TokenKind::Symbol(id) if frontend.string_table.resolve(*id) == "hello" => Some(*id),
            _ => None,
        });
    assert!(
        hello_token.is_some(),
        "hello symbol inside const template should resolve through module table"
    );

    // Verify that beta and the const template have different global IDs, proving
    // non-identity remapping occurred for at least one file's local suffix.
    let beta_id = beta_header
        .unwrap()
        .tokens
        .src_path
        .name()
        .expect("beta should have a name ID");
    let const_template_id = const_template_header
        .unwrap()
        .tokens
        .src_path
        .name()
        .expect("const template should have a name ID");
    assert_ne!(
        beta_id, const_template_id,
        "beta and #const_template0 should have different global IDs after non-identity remapping"
    );
}

//  ----------------------------------------------------------------------
//  Phase boundary: preparation retains PreparedHeaderSyntax for semantic compilation
//  ----------------------------------------------------------------------

/// Demonstrate the provider-independent preparation/semantic phase boundary and guard against
/// semantic compilation regaining source or token inputs.
///
/// WHAT: `prepare_module` runs on a `ModulePreparationContext` that carries no provider-interface
///       values — only style directives and the project path resolver — and retains a
///       `PreparedModule` carrying `PreparedHeaderSyntax`, the module string table and source
///       identities. `compile_module_semantic` runs on a separately constructed
///       `FrontendModuleBuildContext` that owns the provider interfaces, and consumes only the
///       retained payload through HIR and borrow validation. The `PreparedModule` type carries no
///       source text or tokens, so the type system prevents semantic compilation from rerunning
///       file preparation, and the boundary lets Phase 5 schedule provider binding between the
///       two calls.
#[test]
fn prepare_module_retains_header_syntax_for_semantic_compilation() {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let entry_file = temp_dir.path().join("entry.moth");
    let source = "export:\n\
    identity type T |value T| -> T:\n\
        return value\n\
    ;\n\
;\n";
    fs::write(&entry_file, source).unwrap();
    let canonical_entry = fs::canonicalize(&entry_file).unwrap();

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        std::iter::once(canonical_entry.as_path()),
        &canonical_entry,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let style_directives = StyleDirectiveRegistry::built_ins();
    let input_files = vec![tokenized_moth_prepared_input(
        &source_files,
        &style_directives,
        &mut string_table,
        canonical_entry.clone(),
        source,
    )];

    // Fork a local module string table sharing the fixture base so retained token StringIds
    // stay valid, mirroring the production per-module fork.
    let local_table = string_table.fork_source().fork_for_module().into_parts().0;

    let source_byte_count = source_byte_count(&input_files);

    let external_packages = Arc::new(ExternalPackageRegistry::new());
    let resolution_table = ExternalImportResolutionTable::default();

    // Build a real project path resolver so the semantic phase can run AST, HIR and borrow
    // validation over the retained payload, exercising the full preparation/semantic boundary.
    let project_root = canonical_entry
        .parent()
        .expect("entry file should have a parent directory")
        .to_path_buf();
    let source_file_kinds = SourceFileKindRegistry::new();
    let project_path_resolver = ProjectPathResolver::new_with_module_roots(
        project_root.clone(),
        project_root.clone(),
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
        ModuleRootTable::from_records(vec![ModuleRootRecord::new(
            project_root.clone(),
            canonical_entry.clone(),
        )]),
    )
    .expect("project path resolver should build");

    // Phase 1: preparation is provider-independent. The preparation context carries no external
    // package registry, import resolution table or builder runtime packages — only style
    // directives and the project path resolver — so it can run before any provider interface
    // exists.
    let preparation_context = super::ModulePreparationContext {
        style_directives: &style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };

    // Single-file preparation tests use the same deterministic synthetic normal-module origin
    // the single-file compilation path constructs: the configured project name, the empty logical
    // module path and `ModuleRootRole::Normal`.
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("test-project"),
        std::path::Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic single-file module origin should construct");

    #[cfg(feature = "timers")]
    let prepared_result = preparation_context.prepare_module(
        stable_origin.clone(),
        input_files,
        &canonical_entry,
        local_table,
        source_byte_count,
        None,
    );
    #[cfg(not(feature = "timers"))]
    let prepared_result = preparation_context.prepare_module(
        stable_origin.clone(),
        input_files,
        &canonical_entry,
        local_table,
        source_byte_count,
    );
    let prepared = prepared_result.expect("module preparation should succeed");

    assert!(
        prepared
            .prepared_header_syntax
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "retained PreparedHeaderSyntax should carry the parsed public generic function declaration"
    );
    assert_eq!(
        prepared.source_files.iter().count(),
        1,
        "retained source identity table should carry the one source file"
    );
    assert_eq!(prepared.source_file_count, 1);
    assert_eq!(prepared.source_byte_count, source_byte_count);

    // Phase 2: semantic compilation is provider-dependent. A separately constructed
    // `FrontendModuleBuildContext` owns the provider interfaces, binds the retained
    // `PreparedHeaderSyntax`, then resolves dependencies, builds AST, lowers HIR and runs borrow
    // validation. It receives no `PreparedSourceInput`, source text or tokens.
    let source_provider_imports = Default::default();
    let compile_context = super::FrontendModuleBuildContext {
        config: &Config::new(temp_dir.path().to_path_buf()),
        build_profile: FrontendBuildProfile::Dev,
        project_path_resolver: Some(project_path_resolver),
        style_directives: &style_directives,
        external_packages: Arc::clone(&external_packages),
        external_import_resolution_table: &resolution_table,
        source_provider_imports: &source_provider_imports,
        source_provider_materialisations: &super::SourceProviderMaterialisationSet::default(),
        builder_runtime_packages: &[],
    };

    let generated_store =
        super::super::generated_worklist::BoundaryGeneratedFunctionStore::default();
    #[cfg(feature = "timers")]
    let semantic_result = compile_context.compile_module_semantic(
        prepared,
        &canonical_entry,
        None,
        generated_store.session(),
    );
    #[cfg(not(feature = "timers"))]
    let semantic_result = compile_context.compile_module_semantic(
        prepared,
        &canonical_entry,
        generated_store.session(),
    );
    let draft = semantic_result.expect("semantic compilation should succeed");

    let draft = match draft {
        super::ModuleCompilationOutcome::Success(draft) => draft,
        super::ModuleCompilationOutcome::Diagnosed(diagnostics) => panic!(
            "a generic free-function declaration should compile, not diagnose: {diagnostics:?}"
        ),
    };

    assert_eq!(
        draft.public_interface.module_origin, stable_origin,
        "the semantic draft should retain the module's stable origin"
    );
    assert!(
        !draft.module.executable.hir.functions.is_empty(),
        "the semantic draft should retain validated base HIR"
    );
    assert!(
        draft.string_table.len() > 0,
        "the semantic draft should retain its diagnostic render identities"
    );
    assert_eq!(
        draft.module.metadata.entry_point, canonical_entry,
        "semantic compilation should preserve the module entry point"
    );
    assert!(
        draft.module.metadata.warnings.is_empty(),
        "semantic compilation should not introduce warnings for a clean generic free-function declaration"
    );
    let materialisation_context = draft
        .module
        .metadata
        .materialisation_context
        .as_ref()
        .expect("a successful semantic module retains its materialisation context");
    let identity = GeneratedDeclarationIdentity::Public(OriginFunctionId::new_free(
        stable_origin,
        "identity".to_owned(),
    ));
    assert!(
        materialisation_context.contains_template(&identity),
        "the production semantic path should retain the declared generic free-function template"
    );
}

#[test]
fn api_only_root_roles_compile_public_interfaces_without_hir_start() {
    for root_role in [
        ModuleRootRole::Support,
        ModuleRootRole::ProjectPackageFacade,
    ] {
        compile_api_only_root_and_assert_boundary(root_role);
    }
}

fn compile_api_only_root_and_assert_boundary(root_role: ModuleRootRole) {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let entry_file = temp_dir.path().join("+package.moth");
    let source = "export:\n    answer #= 42\n;\n";
    fs::write(&entry_file, source).expect("test source should be written");
    let canonical_entry = fs::canonicalize(&entry_file).expect("test source should canonicalize");

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        std::iter::once(canonical_entry.as_path()),
        &canonical_entry,
        None,
        &mut string_table,
    )
    .expect("source file table should build");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let input_files = vec![tokenized_moth_prepared_input(
        &source_files,
        &style_directives,
        &mut string_table,
        canonical_entry.clone(),
        source,
    )];
    let local_table = string_table.fork_source().fork_for_module().into_parts().0;

    let project_root = canonical_entry
        .parent()
        .expect("entry file should have a parent")
        .to_path_buf();
    let project_path_resolver = ProjectPathResolver::new_with_module_roots(
        project_root.clone(),
        project_root.clone(),
        PreparedSourcePackageRoots::empty(),
        &SourceFileKindRegistry::new(),
        ModuleRootTable::from_records(vec![ModuleRootRecord::new(
            project_root,
            canonical_entry.clone(),
        )]),
    )
    .expect("project path resolver should build");
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("test-project"),
        std::path::Path::new("api"),
        root_role,
    )
    .expect("API-only stable origin should construct");
    let source_byte_count = source_byte_count(&input_files);
    let preparation_context = super::ModulePreparationContext {
        style_directives: &style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };
    #[cfg(feature = "timers")]
    let prepared_result = preparation_context.prepare_module(
        stable_origin.clone(),
        input_files,
        &canonical_entry,
        local_table,
        source_byte_count,
        None,
    );
    #[cfg(not(feature = "timers"))]
    let prepared_result = preparation_context.prepare_module(
        stable_origin.clone(),
        input_files,
        &canonical_entry,
        local_table,
        source_byte_count,
    );
    let prepared = prepared_result.expect("API-only module preparation should succeed");

    let external_packages = Arc::new(ExternalPackageRegistry::new());
    let resolution_table = ExternalImportResolutionTable::default();
    let source_provider_imports = Default::default();
    let compile_context = super::FrontendModuleBuildContext {
        config: &Config::new(temp_dir.path().to_path_buf()),
        build_profile: FrontendBuildProfile::Dev,
        project_path_resolver: Some(project_path_resolver),
        style_directives: &style_directives,
        external_packages,
        external_import_resolution_table: &resolution_table,
        source_provider_imports: &source_provider_imports,
        source_provider_materialisations: &super::SourceProviderMaterialisationSet::default(),
        builder_runtime_packages: &[],
    };
    let generated_store =
        super::super::generated_worklist::BoundaryGeneratedFunctionStore::default();
    #[cfg(feature = "timers")]
    let semantic_result = compile_context.compile_module_semantic(
        prepared,
        &canonical_entry,
        None,
        generated_store.session(),
    );
    #[cfg(not(feature = "timers"))]
    let semantic_result = compile_context.compile_module_semantic(
        prepared,
        &canonical_entry,
        generated_store.session(),
    );
    let outcome =
        semantic_result.expect("API-only semantic compilation should not fail internally");
    let draft = match outcome {
        super::ModuleCompilationOutcome::Success(draft) => draft,
        super::ModuleCompilationOutcome::Diagnosed(diagnostics) => {
            panic!("API-only declarations should compile without diagnostics: {diagnostics:?}")
        }
    };

    assert_eq!(draft.public_interface.module_origin, stable_origin);
    assert_eq!(draft.public_interface.export_bindings.len(), 1);
    assert_eq!(draft.module.executable.hir.start_function, None);
    assert!(draft.module.executable.hir.functions.is_empty());
    assert!(
        draft
            .module
            .executable
            .hir
            .function_origins
            .values()
            .all(|origin| !matches!(
                origin,
                crate::compiler_frontend::hir::functions::HirFunctionOrigin::EntryStart
            ))
    );
}

#[test]
fn file_preparation_strategy_uses_serial_for_tiny_modules() {
    assert_eq!(
        super::FilePreparationStrategy::for_module(
            super::FILE_PREPARATION_ALWAYS_SERIAL_FILE_COUNT,
            super::FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES * 2
        ),
        super::FilePreparationStrategy::Serial
    );
}

#[test]
fn file_preparation_strategy_uses_serial_for_medium_modules_below_byte_threshold() {
    assert_eq!(
        super::FilePreparationStrategy::for_module(
            5,
            super::FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES - 1
        ),
        super::FilePreparationStrategy::Serial
    );
}

#[test]
fn file_preparation_strategy_uses_chunked_parallel_for_large_modules() {
    assert_eq!(
        super::FilePreparationStrategy::for_module(
            super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT,
            1
        ),
        super::FilePreparationStrategy::ParallelChunked
    );
}

#[test]
fn file_preparation_strategy_uses_per_file_parallel_for_large_byte_medium_modules() {
    assert_eq!(
        super::FilePreparationStrategy::for_module(
            5,
            super::FILE_PREPARATION_MEDIUM_PARALLEL_MIN_BYTES
        ),
        super::FilePreparationStrategy::ParallelPerFile
    );
}

#[test]
fn chunk_planning_splits_eight_files_into_stable_source_order_chunks() {
    let plans =
        super::plan_file_preparation_chunks(super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT, 4);

    assert_eq!(
        plans,
        vec![
            super::FilePreparationChunkPlan {
                chunk_index: 0,
                file_range: 0..4,
            },
            super::FilePreparationChunkPlan {
                chunk_index: 1,
                file_range: 4..8,
            },
        ]
    );
}

#[test]
fn chunk_planning_is_bounded_by_thread_policy_and_minimum_chunk_size() {
    let single_thread_plans = super::plan_file_preparation_chunks(40, 1);
    let four_thread_plans = super::plan_file_preparation_chunks(40, 4);
    let uneven_plans = super::plan_file_preparation_chunks(9, 4);

    assert_eq!(single_thread_plans.len(), 2);
    assert_eq!(single_thread_plans[0].file_range, 0..20);
    assert_eq!(single_thread_plans[1].file_range, 20..40);

    assert_eq!(four_thread_plans.len(), 8);
    assert_eq!(four_thread_plans.first().unwrap().file_range, 0..5);
    assert_eq!(four_thread_plans.last().unwrap().file_range, 35..40);

    assert_eq!(uneven_plans.len(), 2);
    assert!(
        uneven_plans
            .iter()
            .all(|plan| plan.file_range.len() >= super::FILE_PREPARATION_MIN_CHUNK_SIZE)
    );
}

#[test]
fn serial_file_preparation_produces_deterministic_ordered_output() {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_a = temp_dir.path().join("a.moth");
    let file_b = temp_dir.path().join("b.moth");
    let file_c = temp_dir.path().join("c.moth");

    // File A is the entry file with a runtime template, a const template, and a declaration.
    fs::write(&file_a, "alpha = 1\n#[hello]\n[runtime]\n").unwrap();
    // File B is a normal source file with an exported constant (PascalCase produces a warning).
    fs::write(&file_b, "Beta #= 2\n").unwrap();
    // File C is a normal source file with another exported constant (PascalCase produces a warning).
    fs::write(&file_c, "Gamma #= 3\n").unwrap();

    let canonical_a = fs::canonicalize(&file_a).unwrap();
    let canonical_b = fs::canonicalize(&file_b).unwrap();
    let canonical_c = fs::canonicalize(&file_c).unwrap();

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        &[&canonical_a, &canonical_b, &canonical_c],
        &canonical_a,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let module_table_size_before = string_table.len();

    let mut frontend = CompilerFrontend::new(
        &Config::new(temp_dir.path().to_path_buf()),
        string_table,
        StyleDirectiveRegistry::built_ins(),
        Arc::new(ExternalPackageRegistry::new()),
        None,
    );
    frontend.set_source_files(source_files);

    let input_files = vec![
        tokenized_moth_prepared_input(
            &frontend.source_files,
            &frontend.style_directives,
            &mut frontend.string_table,
            canonical_a.clone(),
            "alpha = 1\n#[hello]\n[runtime]\n",
        ),
        tokenized_moth_prepared_input(
            &frontend.source_files,
            &frontend.style_directives,
            &mut frontend.string_table,
            canonical_b.clone(),
            "Beta #= 2\n",
        ),
        tokenized_moth_prepared_input(
            &frontend.source_files,
            &frontend.style_directives,
            &mut frontend.string_table,
            canonical_c.clone(),
            "Gamma #= 3\n",
        ),
    ];
    let source_byte_count = source_byte_count(&input_files);
    assert_eq!(
        super::FilePreparationStrategy::for_module(input_files.len(), source_byte_count),
        super::FilePreparationStrategy::Serial
    );

    let preparation_context = super::ModulePreparationContext {
        style_directives: &frontend.style_directives,
        project_path_resolver: frontend.project_path_resolver.clone(),
    };
    let (headers, warnings) = preparation_context
        .prepare_module_files(
            &mut frontend.string_table,
            &frontend.source_files,
            input_files,
            &canonical_a,
            ModuleRootRole::Normal,
            source_byte_count,
        )
        .expect("serial preparation should succeed");

    // The module table should have grown with strings from all three files plus
    // header-generated strings.
    assert!(
        frontend.string_table.len() > module_table_size_before + 4,
        "module table should contain source strings from all files plus header-generated strings"
    );

    // Verify deterministic header ordering: input order is preserved before aggregation.
    let header_source_names: Vec<_> = headers
        .headers
        .iter()
        .map(|h| {
            h.source_file
                .to_path_buf(&frontend.string_table)
                .file_name()
                .expect("test logical source path should have a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    let last_a = header_source_names
        .iter()
        .rposition(|name| name == "a.moth")
        .expect("file A headers should be present");
    let first_b = header_source_names
        .iter()
        .position(|name| name == "b.moth")
        .expect("file B headers should be present");
    let first_c = header_source_names
        .iter()
        .position(|name| name == "c.moth")
        .expect("file C headers should be present");
    assert!(
        last_a < first_b && first_b < first_c,
        "prepared headers should preserve input file order, got: {header_source_names:?}"
    );

    // Verify headers from all files exist and strings resolve.
    let beta_header = headers
        .headers
        .iter()
        .find(|h| h.tokens.src_path.name_str(&frontend.string_table) == Some("Beta"));
    assert!(beta_header.is_some(), "Beta header should exist");

    let gamma_header = headers
        .headers
        .iter()
        .find(|h| h.tokens.src_path.name_str(&frontend.string_table) == Some("Gamma"));
    assert!(gamma_header.is_some(), "Gamma header should exist");

    let const_template_header = headers
        .headers
        .iter()
        .find(|h| matches!(h.kind, HeaderKind::ConstTemplate { .. }));
    assert!(
        const_template_header.is_some(),
        "const template header should exist"
    );
    let const_template_name = const_template_header
        .unwrap()
        .tokens
        .src_path
        .name_str(&frontend.string_table)
        .expect("const template should have a name");
    assert_eq!(
        const_template_name, "#const_template0",
        "generated const template name should resolve through module table"
    );

    // Verify token symbols inside the const template resolve.
    let hello_token = const_template_header
        .unwrap()
        .tokens
        .tokens
        .iter()
        .find_map(|t| match &t.kind {
            TokenKind::Symbol(id) if frontend.string_table.resolve(*id) == "hello" => Some(*id),
            _ => None,
        });
    assert!(
        hello_token.is_some(),
        "hello symbol inside const template should resolve through module table"
    );

    // Verify runtime fragment count from entry file.
    assert_eq!(
        headers.entry_runtime_fragment_count, 1,
        "entry file should contribute exactly one runtime fragment"
    );

    // Verify const fragment from entry file.
    assert_eq!(
        headers.top_level_const_fragments.len(),
        1,
        "entry file should contribute exactly one const fragment"
    );

    // Verify warnings from multiple files are preserved deterministically.
    assert_eq!(
        warnings.len(),
        2,
        "expected two naming-convention warnings from Beta and Gamma"
    );
    assert!(
        warnings.iter().all(|w| matches!(
            w.kind,
            crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
                crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
            )
        )),
        "all warnings should be naming convention warnings"
    );

    // Verify non-identity remapping: Beta and the const template should have different
    // global IDs, proving at least one file's local suffix was remapped.
    let beta_id = beta_header
        .unwrap()
        .tokens
        .src_path
        .name()
        .expect("Beta should have a name ID");
    let const_template_id = const_template_header
        .unwrap()
        .tokens
        .src_path
        .name()
        .expect("const template should have a name ID");
    assert_ne!(
        beta_id, const_template_id,
        "Beta and #const_template0 should have different global IDs after non-identity remapping"
    );
}

#[test]
fn parallel_file_preparation_produces_deterministic_ordered_output() {
    let temp_dir = tempfile::tempdir().expect("should create temp dir");

    let mut canonical_paths = Vec::new();
    let mut sources = Vec::new();
    for index in 0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT {
        let path = temp_dir.path().join(format!("{index}.moth"));
        let source = format!("value_{index} #= {index}\n");
        fs::write(&path, &source).unwrap();
        let canonical = fs::canonicalize(&path).unwrap();
        sources.push(source);
        canonical_paths.push(canonical);
    }

    let entry_file_path = canonical_paths
        .first()
        .expect("test should create an entry file")
        .clone();

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        canonical_paths.iter().map(PathBuf::as_path),
        &entry_file_path,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let style_directives = StyleDirectiveRegistry::built_ins();
    let input_files = canonical_paths
        .iter()
        .zip(&sources)
        .map(|(canonical, source)| {
            tokenized_moth_prepared_input(
                &source_files,
                &style_directives,
                &mut string_table,
                canonical.clone(),
                source,
            )
        })
        .collect::<Vec<PreparedSourceInput>>();

    let mut frontend = CompilerFrontend::new(
        &Config::new(temp_dir.path().to_path_buf()),
        string_table,
        style_directives,
        Arc::new(ExternalPackageRegistry::new()),
        None,
    );
    frontend.set_source_files(source_files);

    let source_byte_count = source_byte_count(&input_files);
    assert_eq!(
        super::FilePreparationStrategy::for_module(input_files.len(), source_byte_count),
        super::FilePreparationStrategy::ParallelChunked
    );

    let preparation_context = super::ModulePreparationContext {
        style_directives: &frontend.style_directives,
        project_path_resolver: frontend.project_path_resolver.clone(),
    };
    let (headers, warnings) = preparation_context
        .prepare_module_files(
            &mut frontend.string_table,
            &frontend.source_files,
            input_files,
            &entry_file_path,
            ModuleRootRole::Normal,
            source_byte_count,
        )
        .expect("parallel preparation should succeed");

    assert!(warnings.is_empty(), "test declarations should not warn");

    let header_source_names: Vec<_> = headers
        .headers
        .iter()
        .map(|h| {
            h.source_file
                .to_path_buf(&frontend.string_table)
                .file_name()
                .expect("test logical source path should have a file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    let mut previous_file_last_header = None;
    for index in 0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT {
        let expected_name = format!("{index}.moth");
        let first_position = header_source_names
            .iter()
            .position(|name| name == &expected_name)
            .unwrap_or_else(|| panic!("{expected_name} headers should be present"));
        let last_position = header_source_names
            .iter()
            .rposition(|name| name == &expected_name)
            .expect("first position proves this file exists");

        if let Some(previous_file_last_header) = previous_file_last_header {
            assert!(
                previous_file_last_header < first_position,
                "parallel preparation should preserve file order, got: {header_source_names:?}"
            );
        }
        previous_file_last_header = Some(last_position);
    }
    assert_eq!(
        previous_file_last_header,
        Some(header_source_names.len() - 1),
        "all headers should belong to ordered file groups"
    );
}

#[test]
fn chunked_file_preparation_merges_in_source_order_after_out_of_order_completion() {
    let file_sources = chunked_fixture_sources();
    let file_source_refs = fixture_source_refs(&file_sources);
    let mut fixture = frontend_preparation_fixture(&file_source_refs);

    let options = HeaderParseOptions {
        entry_file_id: fixture
            .frontend
            .source_files
            .get_by_canonical_path(&fixture.entry_file_path)
            .map(|identity| identity.file_id),
        project_path_resolver: fixture.frontend.project_path_resolver.clone(),
        active_root_role: ModuleRootRole::Normal,
    };
    let fork_source = fixture.frontend.string_table.fork_source();
    let base_len = fork_source.base_len();
    let input_file_count = fixture.input_files.len();

    let mut chunks = {
        let prepare_context = FrontendFilePrepareContext {
            source_files: &fixture.frontend.source_files,
            style_directives: &fixture.frontend.style_directives,
            entry_file_path: &fixture.entry_file_path,
            options: &options,
        };

        super::ModulePreparationContext::prepare_module_file_chunks(
            std::mem::take(&mut fixture.input_files),
            &fork_source,
            &prepare_context,
            0,
            0,
            super::FilePreparationStrategy::ParallelChunked,
        )
    };
    chunks.reverse();

    let (headers, warnings) = super::ModulePreparationContext::merge_file_preparation_chunks(
        &mut fixture.frontend.string_table,
        chunks,
        input_file_count,
        base_len,
    )
    .expect("chunk merge should succeed");

    assert!(warnings.is_empty(), "test declarations should not warn");

    let header_source_names = header_source_file_names(&headers, &fixture.frontend.string_table);
    let mut previous_file_last_header = None;
    for index in 0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT {
        let expected_name = format!("{index}.moth");
        let first_position = header_source_names
            .iter()
            .position(|name| name == &expected_name)
            .unwrap_or_else(|| panic!("{expected_name} headers should be present"));
        let last_position = header_source_names
            .iter()
            .rposition(|name| name == &expected_name)
            .expect("first position proves this file exists");

        if let Some(previous_file_last_header) = previous_file_last_header {
            assert!(
                previous_file_last_header < first_position,
                "chunked preparation should preserve file order, got: {header_source_names:?}"
            );
        }
        previous_file_last_header = Some(last_position);
    }
}

#[test]
fn chunked_file_preparation_remaps_non_identity_later_chunks() {
    let file_sources = chunked_fixture_sources();
    let file_source_refs = fixture_source_refs(&file_sources);
    let mut fixture = frontend_preparation_fixture(&file_source_refs);
    let source_byte_count = source_byte_count(&fixture.input_files);
    let input_files = std::mem::take(&mut fixture.input_files);

    let preparation_context = super::ModulePreparationContext {
        style_directives: &fixture.frontend.style_directives,
        project_path_resolver: fixture.frontend.project_path_resolver.clone(),
    };
    let (headers, warnings) = preparation_context
        .prepare_module_files(
            &mut fixture.frontend.string_table,
            &fixture.frontend.source_files,
            input_files,
            &fixture.entry_file_path,
            ModuleRootRole::Normal,
            source_byte_count,
        )
        .expect("chunked preparation should succeed");

    assert!(warnings.is_empty(), "test declarations should not warn");

    let header_names: Vec<_> = headers
        .headers
        .iter()
        .filter_map(|header| {
            header
                .tokens
                .src_path
                .name_str(&fixture.frontend.string_table)
                .map(str::to_owned)
        })
        .collect();

    for index in 0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT {
        let expected_name = format!("value_{index}");
        assert!(
            header_names.contains(&expected_name),
            "expected remapped declaration `{expected_name}` in {header_names:?}"
        );
    }
}

#[test]
fn chunked_file_preparation_preserves_warning_source_order() {
    let file_sources: Vec<_> = (0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT)
        .map(|index| {
            (
                format!("{index}.moth"),
                format!("Value{index} #= {index}\n"),
            )
        })
        .collect();
    let file_source_refs = fixture_source_refs(&file_sources);
    let mut fixture = frontend_preparation_fixture(&file_source_refs);
    let source_byte_count = source_byte_count(&fixture.input_files);
    let input_files = std::mem::take(&mut fixture.input_files);

    let preparation_context = super::ModulePreparationContext {
        style_directives: &fixture.frontend.style_directives,
        project_path_resolver: fixture.frontend.project_path_resolver.clone(),
    };
    let (_headers, warnings) = preparation_context
        .prepare_module_files(
            &mut fixture.frontend.string_table,
            &fixture.frontend.source_files,
            input_files,
            &fixture.entry_file_path,
            ModuleRootRole::Normal,
            source_byte_count,
        )
        .expect("chunked preparation should succeed with warnings");

    let warning_names: Vec<_> = warnings
        .iter()
        .filter_map(|warning| match &warning.payload {
            DiagnosticPayload::IdentifierNamingConvention { name, .. } => {
                Some(fixture.frontend.string_table.resolve(*name).to_owned())
            }
            _ => None,
        })
        .collect();
    let expected_names: Vec<_> = (0..super::FILE_PREPARATION_ALWAYS_PARALLEL_FILE_COUNT)
        .map(|index| format!("Value{index}"))
        .collect();

    assert_eq!(
        warning_names, expected_names,
        "warnings should aggregate in source-file order"
    );
}

//  ----------------------------------------------------------------------
//  Malformed file-preparation payload rejection
//  ----------------------------------------------------------------------

/// Build a `PreparedFileResult` carrying a dummy error so the validation path can inspect
/// `file_index` without needing a full prepared output.
fn dummy_prepared_file_result(file_index: usize) -> super::PreparedFileResult {
    super::PreparedFileResult {
        file_index,
        result: Err(FileFrontendPrepareError {
            warnings: Vec::new(),
            diagnostic: Box::new(CompilerDiagnostic::unreachable_match_arm(
                SourceLocation::default(),
            )),
        }),
    }
}

/// Build a chunk with the given file range and one result per supplied file index.
///
/// Pass explicit `file_indexes` to create a wrong-index malformation or a length mismatch.
fn dummy_preparation_chunk(
    chunk_index: usize,
    file_range: std::ops::Range<usize>,
    file_indexes: Vec<usize>,
) -> super::FilePreparationChunk {
    super::FilePreparationChunk {
        chunk_index,
        file_range,
        local_string_table: StringTable::new(),
        results: file_indexes
            .into_iter()
            .map(dummy_prepared_file_result)
            .collect(),
    }
}

/// Merge malformed chunks through the real merge path and assert the boundary returns an
/// infrastructure `CompilerError` whose message contains `expected_fragment`.
fn assert_malformed_chunks_rejected(
    chunks: Vec<super::FilePreparationChunk>,
    module_file_count: usize,
    expected_fragment: &str,
) {
    let mut fixture = frontend_preparation_fixture(&[("a.moth", "x #= 1\n")]);
    let base_len = fixture.frontend.string_table.fork_source().base_len();

    let error_messages = match super::ModulePreparationContext::merge_file_preparation_chunks(
        &mut fixture.frontend.string_table,
        chunks,
        module_file_count,
        base_len,
    ) {
        Err(messages) => messages,
        Ok(_) => panic!("malformed chunk payload should be rejected, but merge succeeded"),
    };

    assert!(
        error_messages.has_errors(),
        "malformed chunks should produce at least one error diagnostic"
    );

    let infrastructure_error = error_messages.diagnostics.iter().find(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InfrastructureError { .. }
        )
    });
    let infrastructure_error = infrastructure_error
        .expect("malformed chunks should produce an infrastructure CompilerError");

    match &infrastructure_error.payload {
        DiagnosticPayload::InfrastructureError { msg, .. } => {
            assert!(
                msg.contains(expected_fragment),
                "error message `{msg}` should contain `{expected_fragment}`"
            );
        }
        _ => unreachable!("already matched InfrastructureError"),
    }
}

#[test]
fn merge_rejects_chunk_gap_in_file_indexes() {
    let chunk_zero = dummy_preparation_chunk(0, 0..4, (0..4).collect::<Vec<_>>());
    let chunk_one = dummy_preparation_chunk(1, 5..8, (5..8).collect::<Vec<_>>());

    assert_malformed_chunks_rejected(vec![chunk_zero, chunk_one], 8, "but expected 4");
}

#[test]
fn merge_rejects_chunk_overlap_in_file_indexes() {
    let chunk_zero = dummy_preparation_chunk(0, 0..4, (0..4).collect::<Vec<_>>());
    let chunk_one = dummy_preparation_chunk(1, 3..8, (3..8).collect::<Vec<_>>());

    assert_malformed_chunks_rejected(vec![chunk_zero, chunk_one], 8, "but expected 4");
}

#[test]
fn merge_rejects_wrong_internal_file_index_in_chunk() {
    let chunk = dummy_preparation_chunk(0, 0..4, vec![0, 1, 2, 7]);

    assert_malformed_chunks_rejected(vec![chunk], 4, "but expected 3");
}

#[test]
fn merge_rejects_missing_tail_coverage() {
    let chunk_zero = dummy_preparation_chunk(0, 0..4, (0..4).collect::<Vec<_>>());

    assert_malformed_chunks_rejected(vec![chunk_zero], 8, "cover 4 files but the module has 8");
}

#[test]
fn merge_rejects_chunk_range_past_module_tail() {
    let chunk = dummy_preparation_chunk(0, 0..5, (0..5).collect::<Vec<_>>());

    assert_malformed_chunks_rejected(vec![chunk], 4, "module has only 4 files");
}

#[test]
fn merge_rejects_reversed_chunk_range() {
    let chunk = dummy_preparation_chunk(0, 0..4, (0..4).collect::<Vec<_>>());
    let reversed = dummy_preparation_chunk(1, std::ops::Range { start: 4, end: 3 }, Vec::new());

    assert_malformed_chunks_rejected(vec![chunk, reversed], 4, "has reversed range");
}

#[test]
fn resolve_and_validate_active_root_rejects_mismatched_expected_origin() {
    // The active root maps to stable origin A in the source-origin table, but the expected
    // active origin passed by the caller is distinct origin B. Preparation must reject this
    // mismatch rather than trusting the loose expected-origin argument.
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let entry_path = temp_dir.path().join("@page.moth");
    fs::write(&entry_path, "").expect("test source file should be written");
    let canonical_entry = fs::canonicalize(&entry_path).expect("file should canonicalize");

    let mut string_table = StringTable::new();
    let source_files = SourceFileTable::build(
        std::iter::once(canonical_entry.clone()),
        &canonical_entry,
        None,
        &mut string_table,
    )
    .expect("source file table should build");

    let table_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("project-a"),
        std::path::Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("table origin should construct");
    let expected_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("project-b"),
        std::path::Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("expected origin should construct");

    let mut lookup = rustc_hash::FxHashMap::default();
    lookup.insert(canonical_entry.clone(), table_origin.clone());
    let source_module_origins =
        SourceModuleOriginTable::from_graph_ownership(&source_files, &lookup);

    let result = super::ModulePreparationContext::resolve_and_validate_active_root(
        &source_files,
        &source_module_origins,
        &expected_origin,
        &canonical_entry,
        &string_table,
    );

    let error_messages = match result {
        Err(messages) => messages,
        Ok(_) => panic!("a mismatched expected active origin must be rejected"),
    };
    assert!(
        error_messages.has_errors(),
        "an origin mismatch must produce at least one error diagnostic"
    );
    let mismatch_error = error_messages
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InfrastructureError { .. }
            )
        })
        .expect("an origin mismatch must produce an infrastructure error");
    match &mismatch_error.payload {
        DiagnosticPayload::InfrastructureError { msg, .. } => {
            assert!(
                msg.contains("does not match the expected active origin"),
                "error message `{msg}` should state the origin mismatch"
            );
        }
        _ => unreachable!("already matched InfrastructureError"),
    }
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn chunked_file_preparation_skips_identity_payload_remap() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, log_frontend_counters, reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();

    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let file_sources = chunked_fixture_sources();
    let file_source_refs = fixture_source_refs(&file_sources);
    let mut fixture = frontend_preparation_fixture(&file_source_refs);
    let source_byte_count = source_byte_count(&fixture.input_files);
    let input_files = std::mem::take(&mut fixture.input_files);

    let preparation_context = super::ModulePreparationContext {
        style_directives: &fixture.frontend.style_directives,
        project_path_resolver: fixture.frontend.project_path_resolver.clone(),
    };
    preparation_context
        .prepare_module_files(
            &mut fixture.frontend.string_table,
            &fixture.frontend.source_files,
            input_files,
            &fixture.entry_file_path,
            ModuleRootRole::Normal,
            source_byte_count,
        )
        .expect("chunked preparation should succeed");

    log_frontend_counters();
    let observations = timing_session.finish();

    // Each chunk forks the already-tokenized base string table, so both chunk merges are
    // identity remaps and neither chunk needs to rewrite prepared payload IDs.
    assert_counter_value(
        &observations.counters,
        "file_preparation_identity_remap_count",
        2.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_preparation_non_identity_remap_count",
        0.0,
    );
    assert_counter_value(
        &observations.counters,
        "file_prepare_output_remap_calls",
        0.0,
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
fn assert_counter_value(
    counters: &[crate::timing::BenchmarkObservationMetric],
    name: &str,
    expected: f64,
) {
    let actual = counters
        .iter()
        .find(|counter| counter.name == name)
        .map(|counter| counter.value)
        .unwrap_or(-1.0);

    assert_eq!(actual, expected, "counter `{name}` did not match");
}
