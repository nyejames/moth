//! AST constant semantic resolution.
//!
//! WHAT: parses and folds constant initializer expressions in header dependency order.
//! WHY: headers are already sorted by the dependency stage; AST owns expression semantics.
//! MUST NOT: rebuild dependency visibility.

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::statements::declarations::resolve_declaration_syntax;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAnnotation;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use std::sync::Arc;

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::add_ast_counter;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

/// WHAT: Carries all mutable/immutable context needed to parse one constant header.
/// WHY: Grouping these parameters keeps the resolver call sites explicit while avoiding
/// overly-wide function signatures that are harder to maintain.
pub(crate) struct ConstantHeaderParseContext<'a> {
    pub top_level_declarations: Rc<TopLevelDeclarationTable>,
    pub module_constants: &'a [Declaration],
    pub file_visibility: &'a FileVisibility,
    pub resolved_type_aliases: Rc<FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    pub generic_declarations_by_path: Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>,
    pub resolved_struct_fields_by_path: Rc<FxHashMap<InternedPath, Vec<Declaration>>>,
    pub choice_variant_shells_by_path: Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>,
    pub type_environment: &'a mut TypeEnvironment,
    pub nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,
    pub external_package_registry: &'a Arc<ExternalPackageRegistry>,
    pub style_directives: &'a StyleDirectiveRegistry,
    pub project_path_resolver: Option<ProjectPathResolver>,
    pub path_format_config: PathStringFormatConfig,
    pub template_const_loop_iteration_limit: usize,
    pub template_ir_store: Rc<RefCell<TemplateIrStore>>,
    pub build_profile: FrontendBuildProfile,
    pub warnings: &'a mut Vec<CompilerDiagnostic>,
    pub rendered_path_usages: Rc<RefCell<Vec<RenderedPathUsage>>>,
    pub string_table: &'a mut StringTable,
    pub trait_environment: Option<Rc<TraitEnvironment>>,
}

pub(crate) fn parse_constant_header_declaration(
    header: &Header,
    context: ConstantHeaderParseContext<'_>,
) -> Result<Declaration, ExpressionParseError> {
    // Destructure the context so each field can be moved into the builder
    // and resolver calls without borrow-checker conflicts.
    let ConstantHeaderParseContext {
        top_level_declarations,
        module_constants,
        file_visibility,
        resolved_type_aliases,
        generic_declarations_by_path,
        resolved_struct_fields_by_path,
        choice_variant_shells_by_path,
        type_environment,
        nominal_type_ids_by_path,
        external_package_registry,
        style_directives,
        project_path_resolver,
        path_format_config,
        template_const_loop_iteration_limit,
        template_ir_store,
        build_profile,
        warnings,
        rendered_path_usages,
        string_table,
        trait_environment,
    } = context;

    increment_ast_counter(AstCounter::ConstantResolutionContextsCreated);
    record_constant_pass_visibility_clone(file_visibility);

    let HeaderKind::Constant { declaration, .. } = &header.kind else {
        let error = CompilerError::compiler_error(
            "Constant header resolver called for a non-constant header.",
        );
        return Err(error.into());
    };

    // Derive the file scope from the canonical OS path when available,
    // falling back to the header's source file identity.
    let source_file_scope = header.canonical_source_file(string_table);

    // Constant headers are parsed while the AST environment is still being
    // assembled, so this context uses `ScopeContext::new` with explicit
    // visibility/alias services instead of the completed `AstModuleLookups`
    // package used by later body emission.
    let mut scope_context = ScopeContext::new(
        ContextKind::ConstantHeader,
        header.tokens.src_path.to_owned(),
        top_level_declarations,
        Arc::clone(external_package_registry),
        vec![],
        0,
        template_ir_store,
    )
    .with_style_directives(style_directives)
    .with_build_profile(build_profile)
    .with_project_path_resolver(project_path_resolver)
    .with_path_format_config(path_format_config)
    .with_template_const_loop_iteration_limit(template_const_loop_iteration_limit)
    .with_rendered_path_usage_sink(rendered_path_usages)
    // Keep full module declarations for path identity, but gate every file-local lookup through
    // the header-built visibility package so namespace bindings and aliases behave exactly like
    // they do in function/start body contexts.
    .with_file_visibility(Rc::new(file_visibility.clone()))
    // Type resolution support
    .with_resolved_type_aliases(Rc::clone(&resolved_type_aliases))
    .with_explicit_compile_time_constants(module_constants)
    .with_generic_declarations(Rc::clone(&generic_declarations_by_path))
    .with_resolved_struct_fields_by_path(Rc::clone(&resolved_struct_fields_by_path))
    .with_choice_variant_shells_by_path(Rc::clone(&choice_variant_shells_by_path))
    .with_nominal_type_ids_by_path(Rc::clone(&nominal_type_ids_by_path))
    .with_source_file_scope(source_file_scope);

    if let Some(trait_env) = trait_environment {
        scope_context = scope_context.with_trait_environment(trait_env);
    }

    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(type_environment, &mut compatibility_cache);

    let declaration_result = resolve_declaration_syntax(
        declaration.clone(),
        header.tokens.src_path.to_owned(),
        &header.tokens.path_syntax,
        &mut scope_context,
        &mut type_interner,
        string_table,
    );
    warnings.extend(scope_context.take_emitted_warnings());
    let declaration = declaration_result?;

    // After resolution, the initializer must be fully foldable at compile time.
    // Runtime expressions in constants are rejected here. Template payloads keep
    // their module-local reference, phase and overlay identity during classification.
    let initializer_is_compile_time_constant = declaration
        .value
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, &scope_context.template_ir_store)
        })
        .map(|kind| kind.is_compile_time_value())
        .map_err(ExpressionParseError::from)?;

    if !initializer_is_compile_time_constant {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
            declaration.id.name(),
            header.name_location.clone(),
        )
        .into());
    }

    Ok(declaration)
}

/// Record how many bound visibility entries this constant header copies.
///
/// WHY: the per-constant `FileVisibility` clone below is the dominant repeated
/// module-wide copy in const-heavy modules, and its cost scales with the visible
/// name set rather than with the constant's own initializer.
#[cfg(feature = "benchmark_counters")]
fn record_constant_pass_visibility_clone(file_visibility: &FileVisibility) {
    let entries = file_visibility.visible_declaration_paths.len()
        + file_visibility.visible_source_names.len()
        + file_visibility.visible_type_alias_names.len()
        + file_visibility.visible_trait_names.len()
        + file_visibility.visible_external_symbols.len()
        + file_visibility.visible_namespace_records.len();

    add_ast_counter(AstCounter::ConstantPassVisibilityEntriesCloned, entries);
}

#[cfg(not(feature = "benchmark_counters"))]
fn record_constant_pass_visibility_clone(_file_visibility: &FileVisibility) {}
