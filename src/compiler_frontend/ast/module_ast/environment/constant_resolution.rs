//! AST constant semantic resolution.
//!
//! WHAT: owns [`ConstantResolutionSession`], the single module-scoped session that parses and
//! folds every top-level constant initializer in header dependency order.
//! WHY: headers are already sorted by the dependency stage, so the whole pass reads one
//! unchanging view of the module. Building that view once means a module with many constants
//! prepares its side tables, canonical file scopes and compatibility cache a fixed number of
//! times instead of once per constant.
//! MUST NOT: rebuild dependency visibility, or hold the declaration table across a constant. The
//! environment builder commits each resolved constant into the table in place, which requires
//! sole ownership between calls.
//!
//! Body-local `#` constants are unrelated to this session. They stay on the normal lexical
//! `ScopeContext` built during body emission.

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
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The module view every constant in the pass reads.
///
/// WHY: separating this from the per-constant call keeps the session constructor honest about
/// what is genuinely module-wide, and keeps `resolve_constant_header` down to the state that
/// really does change between constants.
pub(crate) struct ConstantResolutionSessionInput {
    pub resolved_type_aliases: Rc<FxHashMap<InternedPath, ResolvedTypeAnnotation>>,
    pub generic_declarations_by_path: Rc<FxHashMap<InternedPath, GenericDeclarationMetadata>>,
    pub resolved_struct_fields_by_path: Rc<FxHashMap<InternedPath, Vec<Declaration>>>,
    pub choice_variant_shells_by_path: Rc<FxHashMap<InternedPath, Vec<ChoiceVariant>>>,
    pub nominal_type_ids_by_path: Rc<FxHashMap<InternedPath, TypeId>>,
    pub trait_environment: Rc<TraitEnvironment>,
    pub external_package_registry: Arc<ExternalPackageRegistry>,
    pub style_directives: StyleDirectiveRegistry,
    pub project_path_resolver: Option<ProjectPathResolver>,
    pub path_format_config: PathStringFormatConfig,
    pub template_const_loop_iteration_limit: usize,
    pub template_ir_store: Rc<RefCell<TemplateIrStore>>,
    pub build_profile: FrontendBuildProfile,
    pub rendered_path_usages: Rc<RefCell<Vec<RenderedPathUsage>>>,
}

/// State that changes between constants, supplied by the environment builder per call.
pub(crate) struct ConstantHeaderInput<'a> {
    /// Current declaration table. The builder commits each resolved constant into it, so the
    /// session takes a fresh handle per constant rather than retaining one.
    pub top_level_declarations: Rc<TopLevelDeclarationTable>,
    /// Paths of the constants resolved so far, shared rather than copied into the scope frame.
    pub resolved_constant_paths: Rc<FxHashSet<InternedPath>>,
    pub file_visibility: &'a Arc<FileVisibility>,
    pub type_environment: &'a mut TypeEnvironment,
    pub warnings: &'a mut Vec<CompilerDiagnostic>,
}

/// One module-owned session for the dependency-ordered top-level constant pass.
pub(crate) struct ConstantResolutionSession {
    module_view: ConstantResolutionSessionInput,

    /// Canonical scope path of each source file that declares a constant.
    ///
    /// WHY: interning the canonical file path needs the string table and is identical for every
    /// constant declared in the same file.
    source_file_scopes: FxHashMap<InternedPath, InternedPath>,

    /// One compatibility cache for the whole pass.
    ///
    /// The cache is keyed purely on canonical `TypeId` pairs and the pass only ever adds new
    /// types to the module `TypeEnvironment`, so entries stay valid until the session is dropped.
    compatibility_cache: TypeCompatibilityCache,
}

impl ConstantResolutionSession {
    pub(crate) fn new(module_view: ConstantResolutionSessionInput) -> Self {
        Self {
            module_view,
            source_file_scopes: FxHashMap::default(),
            compatibility_cache: TypeCompatibilityCache::new(),
        }
    }

    /// Parse and fold one constant header initializer.
    ///
    /// The returned declaration is fully resolved. The caller commits it to the declaration
    /// table and the module constant list before resolving the next header.
    pub(crate) fn resolve_constant_header(
        &mut self,
        header: &Header,
        input: ConstantHeaderInput<'_>,
        string_table: &mut StringTable,
    ) -> Result<Declaration, ExpressionParseError> {
        let ConstantHeaderInput {
            top_level_declarations,
            resolved_constant_paths,
            file_visibility,
            type_environment,
            warnings,
        } = input;

        let HeaderKind::Constant { declaration, .. } = &header.kind else {
            let error = CompilerError::compiler_error(
                "Constant header resolver called for a non-constant header.",
            );
            return Err(error.into());
        };

        let mut scope_context = self.constant_header_scope(
            header,
            top_level_declarations,
            resolved_constant_paths,
            file_visibility,
            string_table,
        );

        let mut type_interner =
            AstTypeInterner::new(type_environment, &mut self.compatibility_cache);

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

        increment_ast_counter(AstCounter::ConstantsResolved);

        Ok(declaration)
    }

    /// Build the scope one constant header is parsed in.
    ///
    /// Constant headers are parsed while the AST environment is still being assembled, so this
    /// scope carries explicit visibility and alias services instead of the completed
    /// `AstModuleLookups` package used by later body emission. Everything except the declaration
    /// table and the scope path is a shared handle prepared once by the session.
    fn constant_header_scope(
        &mut self,
        header: &Header,
        top_level_declarations: Rc<TopLevelDeclarationTable>,
        resolved_constant_paths: Rc<FxHashSet<InternedPath>>,
        file_visibility: &Arc<FileVisibility>,
        string_table: &mut StringTable,
    ) -> ScopeContext {
        increment_ast_counter(AstCounter::ConstantResolutionContextsCreated);

        let module_view = &self.module_view;
        let source_file_scope = self
            .source_file_scopes
            .entry(header.source_file.to_owned())
            .or_insert_with(|| header.canonical_source_file(string_table));

        ScopeContext::new(
            ContextKind::ConstantHeader,
            header.tokens.src_path.to_owned(),
            top_level_declarations,
            Arc::clone(&module_view.external_package_registry),
            vec![],
            0,
            Rc::clone(&module_view.template_ir_store),
        )
        .with_style_directives(&module_view.style_directives)
        .with_build_profile(module_view.build_profile)
        .with_project_path_resolver(module_view.project_path_resolver.clone())
        .with_path_format_config(module_view.path_format_config.clone())
        .with_template_const_loop_iteration_limit(module_view.template_const_loop_iteration_limit)
        .with_rendered_path_usage_sink(Rc::clone(&module_view.rendered_path_usages))
        // Keep full module declarations for path identity, but gate every file-local lookup
        // through the header-built visibility package so namespace bindings and aliases behave
        // exactly like they do in function/start body contexts.
        .with_file_visibility(Arc::clone(file_visibility))
        .with_source_file_scope(source_file_scope.to_owned())
        .with_resolved_type_aliases(Rc::clone(&module_view.resolved_type_aliases))
        .with_explicit_compile_time_constants(resolved_constant_paths)
        .with_generic_declarations(Rc::clone(&module_view.generic_declarations_by_path))
        .with_resolved_struct_fields_by_path(Rc::clone(&module_view.resolved_struct_fields_by_path))
        .with_choice_variant_shells_by_path(Rc::clone(&module_view.choice_variant_shells_by_path))
        .with_nominal_type_ids_by_path(Rc::clone(&module_view.nominal_type_ids_by_path))
        .with_trait_environment(Rc::clone(&module_view.trait_environment))
    }
}
