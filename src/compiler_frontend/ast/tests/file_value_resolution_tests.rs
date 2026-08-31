//! Fold-level regression tests for value-position content file references.
//!
//! A direct `.mtf` or `.md` value in a constant initializer must reuse the synthetic `content`
//! declaration that preparation creates for that file. These tests intentionally compare the
//! folded string identity, rather than only its text, so a second content constant or a
//! re-interned value cannot pass.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{
    Ast, ContextKind, FileValueResolutionServices, ScopeContext, Stage0ResolutionFacts,
    TopLevelDeclarationTable,
};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, InvalidCompileTimePathReason,
    InvalidExpressionReason, PathKind, RuleDiagnosticKind, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::moth_template_prepare::prepare_moth_template_file;
use crate::compiler_frontend::headers::parse_file_headers::{
    HeaderParseOptions, bind_module_headers, prepare_file_from_tokens, prepare_header_syntax,
};
use crate::compiler_frontend::headers::plain_markdown_prepare::{
    PlainMarkdownPrepareInput, prepare_plain_markdown_file,
};
use crate::compiler_frontend::module_compilation::FrontendOptions;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget, ResourceSourceId,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::compiler_frontend::{AstBuildRequest, CompilerFrontend};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[test]
fn moth_file_value_from_module_root_folds_to_target_content_constant() {
    let (ast, string_table) = compile_fixture(&[("@page.moth", "intro #= @docs/intro.mtf\n")], &[]);

    assert_file_value_reuses_content(&ast, &string_table, "docs/intro.mtf/content");
}

#[test]
fn moth_file_value_from_same_module_source_folds_to_target_content_constant() {
    let (ast, string_table) = compile_fixture(
        &[
            ("@page.moth", ""),
            ("helper.moth", "intro #= @docs/intro.mtf\n"),
        ],
        &[],
    );

    assert_file_value_reuses_content(&ast, &string_table, "docs/intro.mtf/content");
}

#[test]
fn markdown_file_value_folds_to_target_content_constant() {
    let (ast, string_table) = compile_fixture(
        &[("@page.moth", "intro #= @docs/intro.md\n")],
        &[("docs/intro.md", "# Intro\n\nRendered markdown body.\n")],
    );

    assert_file_value_reuses_content(&ast, &string_table, "docs/intro.md/content");
}

#[test]
fn resource_file_value_folds_to_one_resource_piece_with_owner_relative_origin() {
    let owner_relative_path = portable_resource_path("vendor/drawing.js");
    let module_origin = test_module_origin();
    let (expression, module_resources, _) = resolve_file_value_fixture(
        "@vendor/drawing.js",
        PreparedFileReferenceClass::ResourceFile,
        ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
            source: ResourceSourceId::from_index(0),
            owner_relative_path: owner_relative_path.clone(),
        }),
    )
    .expect("resource value should resolve");

    let pieces = structural_string_pieces(expression);
    let [ConstStringPiece::Resource(resource)] = pieces.as_slice() else {
        panic!("resource value should fold to exactly one Resource piece: {pieces:?}");
    };
    let expected_origin = StableResourceOriginId::module_owned(module_origin, owner_relative_path);
    assert_eq!(
        module_resources
            .borrow()
            .try_origin(*resource)
            .expect("resource handle should be interned")
            .origin,
        expected_origin
    );
}

#[test]
fn bare_site_root_file_value_folds_to_one_site_root_piece() {
    let (expression, module_resources, _) = resolve_file_value_fixture(
        "@/",
        PreparedFileReferenceClass::SiteRoot,
        ResolvedFileReferenceOutcome::NoPhysicalTarget,
    )
    .expect("site root value should resolve");

    assert_eq!(
        structural_string_pieces(expression),
        [ConstStringPiece::SiteRoot],
    );
    assert!(
        module_resources.borrow().is_empty(),
        "site root should not intern a resource origin"
    );
}

#[test]
fn extensionless_file_value_reports_typed_diagnostic() {
    let result = resolve_file_value_fixture(
        "@docs/intro",
        PreparedFileReferenceClass::Extensionless,
        ResolvedFileReferenceOutcome::NoPhysicalTarget,
    );

    assert_invalid_expression_reason(result, InvalidExpressionReason::ExtensionlessFileValue);
}

#[test]
fn moth_file_value_reports_typed_no_value_diagnostic() {
    let result = resolve_file_value_fixture(
        "@helpers.moth",
        PreparedFileReferenceClass::SourceKindNoFileValue,
        ResolvedFileReferenceOutcome::NoPhysicalTarget,
    );

    assert_invalid_expression_reason(result, InvalidExpressionReason::MothFileHasNoValue);
}

#[test]
fn rooted_file_value_with_suffix_reports_only_root_slash_diagnostic() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let error = tokenize(
        "@/logo.svg",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect_err("a public root path cannot have a suffix");

    assert_eq!(
        error.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidPath)
    );
    assert_eq!(
        error.payload,
        DiagnosticPayload::InvalidPath {
            path_kind: PathKind::OnlyRootSlashSupported,
        }
    );
}

#[test]
fn resource_file_value_in_struct_field_default_resolves_to_one_resource_piece() {
    let (ast, string_table) = compile_fixture(
        &[(
            "@page.moth",
            "Config = |\n    drawing_url String = @vendor/drawing.js,\n|\nvalue #= Config()\n",
        )],
        &[],
    );

    let value_row = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(&string_table) == Some("value"))
        .expect("struct constructor constant should exist");
    let ConstValuePayload::Record(fields) = ast
        .const_values
        .payload(value_row.id)
        .expect("struct constructor value should be stored")
    else {
        panic!("struct constructor should fold to a record");
    };
    let field = fields
        .iter()
        .find(|field| field.name.name_str(&string_table) == Some("drawing_url"))
        .expect("resource field should be present in struct constructor");
    let ConstValuePayload::String(ConstStringValue::Pieces(pieces)) = ast
        .const_values
        .payload(field.value)
        .expect("resource field default should be stored")
    else {
        panic!("resource field default should fold to a structural string");
    };
    assert!(
        matches!(pieces.as_slice(), [ConstStringPiece::Resource(_)]),
        "struct field default should fold to exactly one Resource piece: {pieces:?}",
    );
}

#[test]
fn resource_file_value_in_function_parameter_default_resolves_through_stage_zero() {
    let (ast, string_table) = compile_fixture(
        &[(
            "@page.moth",
            "draw |drawing_url String = @vendor/drawing.js| -> String:\n    return \"ok\"\n;\n",
        )],
        &[],
    );
    let function = ast.nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Function(path, signature, _) if path.name_str(&string_table) == Some("draw") => {
            Some(signature)
        }
        _ => None,
    });
    let parameter = function
        .and_then(|signature| {
            signature
                .parameters
                .iter()
                .find(|parameter| parameter.id.name_str(&string_table) == Some("drawing_url"))
        })
        .expect("function parameter default should be retained in the AST signature");
    let ExpressionKind::StructuralString { pieces } = &parameter.value.kind else {
        panic!("function parameter default should fold to a structural string");
    };
    assert!(
        matches!(pieces.as_slice(), [ConstStringPiece::Resource(_)]),
        "function parameter default should fold to exactly one Resource piece: {pieces:?}",
    );
}

#[test]
fn content_file_value_behind_child_module_boundary_surfaces_stage0_diagnostic() {
    // Stage 0 owns the module-boundary verdict, so this fixture hands the value lane the settled
    // rejection row directly. The value site must surface Stage 0's diagnostic verbatim instead
    // of resolving, inventing a target or falling back to the eager rendered-path lane.
    let mut boundary_strings = StringTable::new();
    let result = resolve_file_value_fixture(
        "@child/existing.mtf",
        PreparedFileReferenceClass::ContentSource,
        ResolvedFileReferenceOutcome::Diagnostic(Box::new(
            CompilerDiagnostic::invalid_compile_time_path(
                InternedPath::from_single_str("child/existing.mtf", &mut boundary_strings),
                InvalidCompileTimePathReason::EscapesModuleBoundary,
                SourceLocation::default(),
            ),
        )),
    );

    assert_surfaced_stage0_boundary_diagnostic(
        result,
        InvalidCompileTimePathReason::EscapesModuleBoundary,
    );
}

#[test]
fn resource_file_value_behind_support_facade_surfaces_stage0_diagnostic() {
    let mut boundary_strings = StringTable::new();
    let result = resolve_file_value_fixture(
        "@support/existing.svg",
        PreparedFileReferenceClass::ResourceFile,
        ResolvedFileReferenceOutcome::Diagnostic(Box::new(
            CompilerDiagnostic::invalid_compile_time_path(
                InternedPath::from_single_str("support/existing.svg", &mut boundary_strings),
                InvalidCompileTimePathReason::EscapesModuleBoundary,
                SourceLocation::default(),
            ),
        )),
    );

    assert_surfaced_stage0_boundary_diagnostic(
        result,
        InvalidCompileTimePathReason::EscapesModuleBoundary,
    );
}

/// Build the smallest multi-file module that exercises Stage 0's resolved-reference handoff and
/// the complete header/dependency/AST pipeline. The fixture resolver is intentionally represented
/// by the resolved table below: this keeps the test focused on the AST contract and avoids any
/// filesystem work in value-position resolution.
fn compile_fixture(
    moth_files: &[(&str, &str)],
    markdown_files: &[(&str, &str)],
) -> (Ast, StringTable) {
    let templates = [("docs/intro.mtf", "shared body")];
    let entry_path = PathBuf::from("@page.moth");

    let all_paths = moth_files
        .iter()
        .map(|(path, _)| PathBuf::from(path))
        .chain(templates.iter().map(|(path, _)| PathBuf::from(path)))
        .chain(markdown_files.iter().map(|(path, _)| PathBuf::from(path)))
        .collect::<Vec<_>>();

    let mut string_table = StringTable::new();
    let source_files =
        SourceFileTable::build(all_paths.iter(), &entry_path, None, &mut string_table)
            .expect("fixture source identities should build");
    let file_id_for = |path: &str| {
        source_files
            .get_by_canonical_path(&PathBuf::from(path))
            .map(|identity| identity.file_id)
            .unwrap_or_else(|| panic!("fixture file {path} should have a source identity"))
    };

    let style_directives = StyleDirectiveRegistry::built_ins();
    let options = HeaderParseOptions {
        entry_file_id: Some(file_id_for("@page.moth")),
        project_path_resolver: None,
        active_root_role: ModuleRootRole::Normal,
    };
    let mut prepared_outputs =
        Vec::with_capacity(moth_files.len() + templates.len() + markdown_files.len());

    for (path, source) in moth_files {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        let file_tokens = tokenize(
            source,
            &interned_path,
            TokenizerEntryMode::SourceFile,
            &style_directives,
            &mut string_table,
            Some(file_id_for(path)),
        )
        .expect("Moth tokenization should succeed");

        prepared_outputs.push(
            prepare_file_from_tokens(file_tokens, &entry_path, &options, &mut string_table, 0, 0)
                .expect("Moth header preparation should succeed"),
        );
    }

    for (path, source) in templates {
        let path_buf = PathBuf::from(path);
        let interned_path = InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
            .expect("test path should be UTF-8");
        let entry_mode = TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template has a tokenizer entry mode");
        let file_tokens = tokenize(
            source,
            &interned_path,
            entry_mode,
            &style_directives,
            &mut string_table,
            Some(file_id_for(path)),
        )
        .expect("Moth template tokenization should succeed");

        let mut output = prepare_moth_template_file(file_tokens, &mut string_table)
            .expect("Moth template preparation should succeed");
        output
            .freeze_path_syntax(&string_table)
            .expect("prepared template should satisfy the path invariant");
        prepared_outputs.push(output);
    }

    for (path, source) in markdown_files {
        let path_buf = PathBuf::from(path);
        let mut output = prepare_plain_markdown_file(
            PlainMarkdownPrepareInput {
                source_code: source,
                source_file: InternedPath::try_from_filesystem_path(&path_buf, &mut string_table)
                    .expect("test path should be UTF-8"),
                file_id: Some(file_id_for(path)),
                canonical_os_path: None,
            },
            &mut string_table,
        );
        output
            .freeze_path_syntax(&string_table)
            .expect("prepared markdown should satisfy the path invariant");
        prepared_outputs.push(output);
    }

    // This is the Stage 0 fact consumed by AST file-value semantics. Every content path in this
    // fixture names the one prepared template, so there is exactly one target identity to publish.
    let mut resolved_references = ResolvedFileReferenceTable::new();
    let mut resource_source_index = 0;
    for output in &prepared_outputs {
        for reference in output.structural_file_references.iter() {
            let source_file = reference
                .source_file
                .expect("prepared rows carry a source FileId");
            let outcome = match reference.class {
                PreparedFileReferenceClass::ContentSource => {
                    let target_path =
                        PathBuf::from(reference.authored_path.to_portable_string(&string_table));
                    let target = source_files
                        .get_by_canonical_path(&target_path)
                        .map(|identity| identity.file_id)
                        .unwrap_or_else(|| panic!("content target {target_path:?} should exist"));
                    ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::ContentSource { source: target },
                    )
                }
                PreparedFileReferenceClass::ResourceFile => {
                    let authored_path = reference.authored_path.to_portable_string(&string_table);
                    let owner_relative_path = portable_resource_path(&authored_path);
                    let source = ResourceSourceId::from_index(resource_source_index);
                    resource_source_index += 1;
                    ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::ResourceSource {
                            source,
                            owner_relative_path,
                        },
                    )
                }
                PreparedFileReferenceClass::SiteRoot => {
                    ResolvedFileReferenceOutcome::NoPhysicalTarget
                }
                PreparedFileReferenceClass::SourceKindNoFileValue
                | PreparedFileReferenceClass::Extensionless => continue,
            };
            resolved_references
                .push(ResolvedFileReference {
                    source_file,
                    path_syntax: reference.path_syntax,
                    class: reference.class,
                    outcome,
                })
                .expect("fixture resolved rows should be unique");
        }
    }

    let prepared_syntax = prepare_header_syntax(prepared_outputs, &mut string_table)
        .expect("header syntax preparation should succeed");
    let external_package_registry = Arc::new(ExternalPackageRegistry::new());
    let headers = bind_module_headers(
        prepared_syntax,
        &external_package_registry,
        &ExternalImportResolutionTable::default(),
        &SourceProviderDependencySet::default(),
        None,
        &mut string_table,
    )
    .expect("header binding should succeed");

    let mut frontend = CompilerFrontend::new(
        FrontendOptions::default(),
        string_table,
        style_directives,
        external_package_registry,
        None,
    );
    frontend.set_source_files(source_files);
    let sorted = frontend
        .sort_headers(headers, &resolved_references)
        .expect("header sorting should succeed");
    let ast = frontend
        .headers_to_ast(
            AstBuildRequest {
                sorted,
                entry_file_path: &entry_path,
                root_role: ModuleRootRole::Normal,
                build_profile: FrontendBuildProfile::Dev,
                capacity_estimate: Default::default(),
                resolved_file_references: resolved_references,
                module_origin: Some(test_module_origin()),
            },
            #[cfg(feature = "timers")]
            None,
        )
        .expect("AST construction should succeed")
        .ast;

    (ast, frontend.string_table)
}

fn assert_file_value_reuses_content(
    ast: &Ast,
    string_table: &StringTable,
    content_constant_suffix: &str,
) {
    let content_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| {
            row.path.name_str(string_table) == Some("content")
                && row
                    .path
                    .to_portable_string(string_table)
                    .ends_with(content_constant_suffix)
        })
        .expect("synthetic content constant should exist")
        .id;
    let file_value_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(string_table) == Some("intro"))
        .expect("file-value constant should exist")
        .id;

    let content_string = ast
        .const_values
        .string_value(content_id)
        .expect("synthetic content should fold to a string");
    let file_value_string = ast
        .const_values
        .string_value(file_value_id)
        .expect("file-value initializer should fold to a string");
    assert_eq!(
        file_value_string, content_string,
        "file-value constant must reuse the synthetic content StringId"
    );
}
fn resolve_file_value_fixture(
    source: &str,
    class: PreparedFileReferenceClass,
    outcome: ResolvedFileReferenceOutcome,
) -> Result<
    (
        Expression,
        Rc<RefCell<ModuleResourceTable>>,
        StableModuleOriginIdentity,
    ),
    ExpressionParseError,
> {
    let mut string_table = StringTable::new();
    let source_path_buf = PathBuf::from("@page.moth");
    let source_files = SourceFileTable::build(
        std::iter::once(&source_path_buf),
        &source_path_buf,
        None,
        &mut string_table,
    )
    .expect("fixture source identity should build");
    let source_file = source_files
        .get_by_canonical_path(&source_path_buf)
        .expect("fixture source identity should be present")
        .file_id;
    let source_path = InternedPath::try_from_filesystem_path(&source_path_buf, &mut string_table)
        .expect("fixture source path should be UTF-8");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut token_stream = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        Some(source_file),
    )
    .expect("file-value fixture should tokenize");
    token_stream.freeze_path_syntax_for_test();
    let path_token_index = token_stream
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("file-value fixture should contain a path token");
    token_stream.index = path_token_index;
    let path_syntax = match token_stream.tokens[path_token_index].kind {
        TokenKind::Path(path_syntax) => path_syntax,
        _ => unreachable!("path token index was selected above"),
    };

    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file,
            path_syntax,
            class,
            outcome,
        })
        .expect("fixture resolved row should be unique");

    let module_origin = test_module_origin();
    let module_resources = Rc::new(RefCell::new(ModuleResourceTable::new()));
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        source_path,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_file_value_resolution(Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
            resolved_references,
            source_files,
        ))),
        module_resources: Rc::clone(&module_resources),
        module_origin: Some(module_origin.clone()),
    }))
    .with_declaring_file_id(Some(source_file));

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let expression = resolve_file_value(
        path_syntax,
        &token_stream,
        &context,
        &type_interner,
        &ValueMode::ImmutableOwned,
        &mut string_table,
    )?;

    Ok((expression, module_resources, module_origin))
}

fn structural_string_pieces(expression: Expression) -> Vec<ConstStringPiece> {
    match expression.kind {
        ExpressionKind::StructuralString { pieces } => pieces,
        other => panic!("expected structural string, got {other:?}"),
    }
}

fn assert_invalid_expression_reason(
    result: Result<
        (
            Expression,
            Rc<RefCell<ModuleResourceTable>>,
            StableModuleOriginIdentity,
        ),
        ExpressionParseError,
    >,
    expected_reason: InvalidExpressionReason,
) {
    let diagnostic = CompilerDiagnostic::from(
        result.expect_err("invalid file value should produce a diagnostic"),
    );
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidExpression)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidExpression {
            reason: expected_reason,
        }
    );
}

fn test_module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("file-value-tests"),
        String::new(),
        ModuleRootRole::Normal,
    )
}

fn assert_surfaced_stage0_boundary_diagnostic(
    result: Result<
        (
            Expression,
            Rc<RefCell<ModuleResourceTable>>,
            StableModuleOriginIdentity,
        ),
        ExpressionParseError,
    >,
    expected_reason: InvalidCompileTimePathReason,
) {
    let diagnostic = CompilerDiagnostic::from(
        result.expect_err("a boundary-rejected file value must not resolve"),
    );
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCompileTimePath)
    );
    match diagnostic.payload {
        DiagnosticPayload::InvalidCompileTimePath { reason, .. } => {
            assert_eq!(reason, expected_reason);
        }
        other => panic!("the value site must surface Stage 0's retained diagnostic, got {other:?}"),
    }
}

fn portable_resource_path(path: &str) -> PortableResourcePath {
    PortableResourcePath::from_relative_logical_path(Path::new(path))
        .expect("fixture resource path should be relative and portable")
}
