//! Body-local declaration parsing and lowering.
//!
//! WHAT: parses declarations that appear inside executable AST bodies, resolves their declared
//! type boundary, parses the initializer, and emits the resulting local `Declaration`.
//! WHY: top-level declaration discovery belongs to headers/environment construction; this module
//! owns only source-order body declarations and the coercion boundary between an initializer
//! expression and the declared local type.

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::const_values::binding::{
    bound_constant_initializer, resolve_const_expression_content,
};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::module_ast::environment::config_resolution::expression_for_resolved_build_config_value;
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::numeric_construction::{
    BindingContinuation, DeferredExpressionSuffix, NumericContinuation, NumericContinuationId,
    classify_recipe, register_binding_annotation,
};
use crate::compiler_frontend::ast::numeric_known_shape::directed_recipe_shape;
use crate::compiler_frontend::ast::statements::collections::{CurlyLiteral, new_curly_literal};
use crate::compiler_frontend::ast::statements::functions::{
    SignatureDefaultMode, SignatureTypeFallbackPolicy, signature_member_to_declaration,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionContextInputs, fold_collection_capacity,
    resolve_parsed_type_annotation,
};
use crate::compiler_frontend::ast::{
    ContextKind, ScopeContext,
    ast_nodes::Declaration,
    expressions::parse_expression::create_expression_with_trailing_newline_policy,
    statements::value_production::types::ParsedReceiverValue,
    statements::value_production::{ValueReceiverKind, try_parse_value_block_at_receiver},
};
use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data, choices_control_flow, names_declarations, project_config, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag, DiagnosticToken, InvalidCollectionTypeReason,
    InvalidConfigReason, InvalidDeclarationReason, InvalidFallibleHandlingReason,
    TypeMismatchContext, ValueReceiverContext,
};

use crate::compiler_frontend::build_config::BuildInputName;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    build_input_type_from_parsed, parsed_type_span,
};
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::r#struct::parse_struct_shell;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::signature_position::check_signature_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{TokenRange, TokenTag};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedCollectionContext, ExpectedCurlyLiteralContext, ExpectedType,
    cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// Body-local declaration parsing shares the AST body error lane.
///
/// A declaration initializer or signature default can require a temporary substream over a
/// frozen file table. If that retained-table invariant fails, the error must reach module
/// emission as `CompilerError`, not become an authored declaration diagnostic.
type DeclarationResult<T> = Result<T, ExpressionParseError>;

/// Returns `Some(capacity)` when the parsed type is a capacity-only shorthand `{N}`.
///
/// WHAT: detects shorthand collection annotations where the element type is inferred.
/// WHY: shorthand declarations need special handling to fold capacity first and
///      parse the initializer as a collection literal with capacity context.
fn capacity_only_shorthand(type_ref: &ParsedTypeRef) -> Option<&ParsedCollectionCapacity> {
    match type_ref {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity: Some(capacity),
            ..
        } if matches!(element.as_ref(), ParsedTypeRef::Inferred) => Some(capacity),
        _ => None,
    }
}
/// Inspect the first authored initializer token in canonical source coordinates.
fn initializer_starts_with_type_parameter(source: &AstCursor, range: Option<TokenRange>) -> bool {
    let Some(range) = range else {
        return false;
    };
    matches!(
        source
            .canonical_token_ref_at(range.start())
            .map(|token| token.tag()),
        Some(TokenTag::TYPE_PARAMETER_BRACKET)
    )
}

/// Classify a body-local constant initializer through the module's effective TIR views.
///
/// Both declaration paths use the shared module store so templates retain their exact view
/// identity after composition. Classification does not rebuild a scratch template view.
fn initializer_is_compile_time_constant(
    initializer: ExpressionView<'_>,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<bool, TemplateError> {
    initializer
        .const_value_kind_with_template_classifier(const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                expression_builder,
                const_values,
                Some(context.external_package_registry.as_ref()),
            )
        })
        .map(|kind| kind.is_compile_time_value())
}

/// Body-local declaration plus syntax-origin facts that are not stored on `Declaration`.
///
/// WHAT: carries whether the user authored the binding with `#`.
/// WHY: fixed-capacity type syntax accepts bare explicit constants only, so body
///      parsing must preserve the distinction between `#` constants and foldable
///      runtime immutable bindings while registering locals.
pub(crate) struct ResolvedDeclaration {
    pub(crate) declaration: Declaration,
    pub(crate) statement_kind: ResolvedDeclarationStatementKind,
    pub(crate) is_compile_time_binding: bool,
    pub(crate) binding_span: Option<SourceSpan>,
}

/// Statement-level shape that declaration parsing discovered alongside the binding value.
///
/// WHAT: distinguishes ordinary bindings from struct-definition statements.
pub(crate) enum ResolvedDeclarationStatementKind {
    Variable,
    StructDefinition(Vec<Declaration>),
}

/// Mutable interner tables shared by declaration lowering.
///
/// WHAT: groups the string and path tables that every declaration-lowering call needs together.
/// WHY: keeping them together keeps `resolve_declaration_syntax` under the argument-count lint
/// without changing lowering behavior.
pub(crate) struct DeclarationLoweringTables<'a> {
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Register one eligible pending numeric declaration and its ordered continuation.
///
/// WHAT: creates the body-owned origin, records the declaration's own exact annotation as a
///       receiving witness, and returns the continuation ID the statement draft records.
/// WHY: the binding must stay visible through its frame slot while no `Declaration` exists, and
///      the authored `=`/`~=` spelling decides the slot's mutability at completion.
#[allow(
    clippy::too_many_arguments,
    reason = "pending registration keeps interning tables, authored spans, the declared mode and the retained fragment as separate facts"
)]
fn register_pending_declaration(
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &AstExpressionBuilder,
    path_fork: &PathInternerFork,
    qualified_name: PathId,
    declaration_span: Option<SourceSpan>,
    binding_span: Option<SourceSpan>,
    value_mode: ValueMode,
    mutable: bool,
    items: Vec<ExpressionRpnItem>,
    annotation_type_id: Option<TypeId>,
) -> DeclarationResult<NumericContinuationId> {
    let name = path_fork.component(qualified_name).ok_or_else(|| {
        CompilerError::compiler_error("a body-local declaration has no terminal declaration name")
    })?;
    let directed_shape = directed_recipe_shape(
        context,
        type_interner.environment(),
        &items,
        expression_builder,
    )?;
    let recipe = classify_recipe(items, directed_shape);
    let facts = AuthoredBindingFacts {
        name,
        path: qualified_name,
        span: declaration_span,
        binding_span,
        value_mode,
        mutable,
    };
    let binding = context.declare_pending_numeric_binding(facts, recipe)?;
    if let Some(type_id) = annotation_type_id {
        // The declaration's own annotation is one exact receiving constraint on its origin, and it
        // becomes the exact reference shape every read of the binding sees.
        let mut state = context.numeric_state_mut();
        register_binding_annotation(
            &mut state,
            type_interner.environment(),
            binding,
            type_id,
            declaration_span,
        )?;
    }
    let continuation = context.register_numeric_continuation(NumericContinuation::Binding(
        BindingContinuation {
            binding,
            span: binding_span,
            scope: context.scope,
        },
    ))?;
    Ok(continuation)
}

/// Parse a new body-local declaration from the token stream.
///
/// Rejects module-only declaration headers before parsing, then resolves value declarations.
/// Outcome of parsing one body-local declaration.
///
/// WHAT: either a finished declaration, or an eligible numeric binding whose declaration node
///       resumes at the body root through one ordered continuation.
/// WHY: an unannotated numeric initializer has no selected static type yet, so the statement
///      cannot build its declaration node here without defaulting the origin early.
#[allow(
    clippy::large_enum_variant,
    reason = "each outcome is produced once and consumed by its caller, so boxing the Ready arm would add one heap allocation per declaration"
)]
pub(crate) enum NewDeclarationOutcome {
    Ready(ResolvedDeclaration),
    PendingNumeric(NumericContinuationId),
}

#[allow(
    clippy::too_many_arguments,
    reason = "declaration parsing keeps the cursor, symbol, scope and mutable state as separate borrows"
)]
pub(crate) fn new_declaration(
    token_stream: &mut AstCursor,
    symbol_id: StringId,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DeclarationResult<NewDeclarationOutcome> {
    // Functions and generic declaration headers belong to module-level declaration parsing.
    if matches!(
        token_stream.peek_next_tag(),
        Some(TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::TYPE)
    ) {
        return Err(names_declarations::invalid_declaration(
            InvalidDeclarationReason::DeclarationRequiresModuleTopLevel,
            Some(symbol_id),
            Some(token_stream.current_span()),
        )
        .into());
    }
    let declaration_name = string_table.resolve(symbol_id).to_owned();
    ensure_not_keyword_shadow_identifier(
        symbol_id,
        Some(token_stream.current_span()),
        string_table,
    )?;

    if is_reserved_builtin_symbol(&declaration_name) {
        return Err(names_declarations::invalid_declaration(
            InvalidDeclarationReason::ReservedBuiltinName,
            Some(symbol_id),
            Some(token_stream.current_span()),
        )
        .into());
    }
    // Capture the authored binding-name span before advancing past the name token.
    let binding_span = Some(token_stream.current_span());
    if let Some(diagnostic) =
        context.check_body_binding_visibility(symbol_id, binding_span, string_table)
    {
        return Err(diagnostic.into());
    }

    // Move past the name
    token_stream.advance();

    let qualified_name = path_fork
        .try_intern_child(context.scope, symbol_id)
        .expect("path table exhausted while creating declaration scope");

    if let Some(error) = check_signature_common_mistake(token_stream) {
        return Err(error.into());
    }

    // ----------------------------
    //  Parse declaration syntax
    // ----------------------------
    let mut span_builder = ExtendedSpanBuilder::new();
    let declaration_syntax = {
        let (syntax, next_index) = {
            let mut declaration_cursor = token_stream.declaration_cursor()?;
            let syntax = parse_declaration_syntax(
                &mut declaration_cursor,
                symbol_id,
                string_table,
                &mut span_builder,
            )?;
            (syntax, declaration_cursor.position())
        };
        token_stream.set_position(next_index)?;
        syntax
    };

    // Heuristic: a leading type-parameter pipe after the binding marker indicates
    // a struct or generic type definition, which uses type-like naming conventions.
    let naming_kind = if initializer_starts_with_type_parameter(
        token_stream,
        declaration_syntax.initializer_range,
    ) {
        IdentifierNamingKind::TypeLike
    } else {
        IdentifierNamingKind::ValueLike
    };

    if let Some(warning) = naming_warning_for_identifier(
        symbol_id,
        declaration_syntax.span,
        naming_kind,
        string_table,
    ) {
        context.emit_warning(warning);
    }
    let is_compile_time_binding = declaration_syntax.binding_mode.is_compile_time();
    let mut declaration = match resolve_declaration_syntax(
        declaration_syntax,
        qualified_name,
        binding_span,
        Some(token_stream),
        &mut *context,
        type_interner,
        expression_builder,
        const_values,
        DeclarationLoweringTables {
            string_table,
            path_fork,
        },
    )? {
        DeclarationLowering::Ready(declaration) => declaration,
        DeclarationLowering::PendingNumeric(continuation) => {
            return Ok(NewDeclarationOutcome::PendingNumeric(continuation));
        }
    };
    // The binding anchor is the declaration-name token captured on entry, not the
    // initializer value span. `resolve_declaration_syntax` leaves it empty for this
    // body-local path; restore the exact anchor here.
    declaration.binding_span = binding_span;
    if is_compile_time_binding && let Some(authored) = declaration.value.initializer() {
        let authored_is_template = matches!(
            expression_builder.view(authored)?.operation(),
            ExpressionOperationView::Template(_)
        );
        let content = resolve_const_expression_content(
            authored,
            expression_builder,
            const_values,
            type_interner.environment(),
            &*context,
            string_table,
        )?;
        const_values.bind_body_local(declaration.id, content)?;
        let bound = bound_constant_initializer(
            &declaration,
            authored,
            authored_is_template,
            content,
            expression_builder,
        )?;
        declaration.value = DeclarationValue::Initializer(bound);
    }
    let statement_kind = match &declaration.value {
        DeclarationValue::StructDefinition(params) => {
            ResolvedDeclarationStatementKind::StructDefinition(params.to_owned())
        }
        _ => ResolvedDeclarationStatementKind::Variable,
    };

    Ok(NewDeclarationOutcome::Ready(ResolvedDeclaration {
        declaration,
        statement_kind,
        is_compile_time_binding,
        binding_span,
    }))
}

/// Resolve a parsed declaration syntax into a fully typed `Declaration`.
///
/// This is the main lowering path for body-local value and struct declarations.
/// It resolves the declared type annotation, parses the initializer expression,
/// validates type compatibility, and applies contextual coercion.
/// Names the runtime storage position a declaration creates, or `None` when it is a constant.
///
/// Result of lowering one declaration initializer.
///
/// WHAT: a finished declaration, or an eligible numeric binding registered in the current frame
///       and construction domain whose declaration node resumes at the body root.
/// WHY: only a runtime body owns a construction domain; compile-time constant headers always
///       fold at their own boundary and can never return a pending outcome.
#[allow(
    clippy::large_enum_variant,
    reason = "each outcome is produced once and consumed by its caller, so boxing the Ready arm would add one heap allocation per declaration"
)]
pub(crate) enum DeclarationLowering {
    Ready(Declaration),
    PendingNumeric(NumericContinuationId),
}

#[allow(
    clippy::too_many_arguments,
    reason = "declaration lowering keeps the syntax, name, span, owner, scope and mutable state separate"
)]
pub fn resolve_declaration_syntax(
    declaration_syntax: DeclarationSyntax,
    qualified_name: PathId,
    binding_span: Option<SourceSpan>,
    source_owner: Option<&AstCursor>,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    tables: DeclarationLoweringTables<'_>,
) -> DeclarationResult<DeclarationLowering> {
    let DeclarationLoweringTables {
        string_table,
        path_fork,
    } = tables;
    let config_qualifier = declaration_syntax.config_qualifier.clone();
    let config_constant_context = matches!(context.kind, ContextKind::ConstantHeader);
    let has_config_resolution = context.shared.config_resolution.is_some();
    let config_resolution_context = config_constant_context && has_config_resolution;
    let source_build_config_contract_name = path_fork
        .component(qualified_name)
        .and_then(|name| BuildInputName::new(string_table.resolve(name)).ok());
    let has_source_build_config_contract = context
        .shared
        .source_build_config_contract_names
        .as_ref()
        .is_some_and(|names| {
            source_build_config_contract_name
                .as_ref()
                .is_some_and(|name| names.contains(name))
        });
    let source_build_config_context = config_constant_context && has_source_build_config_contract;
    if let Some(qualifier) = &declaration_syntax.config_qualifier
        && (config_resolution_context || source_build_config_context)
        && build_input_type_from_parsed(&qualifier.type_annotation).is_none()
    {
        return Err(project_config::invalid_config_reason(
            path_fork.component(qualified_name),
            InvalidConfigReason::ConfigQualifierUnsupportedType,
            parsed_type_span(&qualifier.type_annotation),
        )
        .into());
    }
    if let Some(qualifier) = &declaration_syntax.config_qualifier
        && !config_resolution_context
        && !source_build_config_context
    {
        return Err(project_config::invalid_config_reason(
            path_fork.component(qualified_name),
            InvalidConfigReason::ConfigQualifierInvalidPlacement,
            qualifier.qualifier_span,
        )
        .into());
    }

    // ----------------------------
    //  Validate constant-context constraints
    // ----------------------------
    let value_mode = declaration_syntax.value_mode();
    if declaration_syntax.binding_mode.is_mutable() && context.kind.is_constant_context() {
        return Err(names_declarations::invalid_declaration(
            InvalidDeclarationReason::ConstantCannotBeMutable,
            None,
            declaration_syntax.span,
        )
        .into());
    }

    // A runtime body-local binding is an eligible numeric origin: its initializer keeps
    // literal arithmetic and unresolved origins until the body root has recorded every
    // permitted use. Compile-time constants and non-runtime declaration contexts always
    // fold at this declaration boundary.
    let retain_pending_origin = !declaration_syntax.binding_mode.is_compile_time()
        && config_qualifier.is_none()
        && !context.kind.is_constant_context();

    // ----------------------------
    //  Resolve declared type
    // ----------------------------

    if let Some(capacity) = capacity_only_shorthand(&declaration_syntax.type_annotation) {
        let folded_capacity = fold_collection_capacity(
            capacity,
            Some(&*context),
            type_interner.environment_mut_for_derived_types(),
            const_values,
        )?;

        let mut initializer_stream = declaration_initializer_stream(
            source_owner,
            declaration_syntax.initializer_range,
            declaration_syntax.span,
        )?;

        // Shorthand requires an immediate collection literal initializer.
        if initializer_stream.current_tag() != TokenTag::OPEN_CURLY {
            return Err(calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                Some(initializer_stream.current_span()),
            )
            .into());
        }

        let collection_context = ExpectedCollectionContext::CapacityOnlyShorthand {
            fixed_capacity: folded_capacity,
        };
        let literal_span = Some(initializer_stream.current_span());
        let parsed_literal = new_curly_literal(
            &mut initializer_stream,
            ExpectedCurlyLiteralContext::Collection(collection_context),
            context,
            type_interner,
            expression_builder,
            const_values,
            &value_mode,
            string_table,
            path_fork,
            retain_pending_origin,
        )?;

        // The collection literal parser stops at the closing `}` without consuming it.
        // Advance past it so the remainder of the declaration validation sees EOF.
        if initializer_stream.current_tag() == TokenTag::CLOSE_CURLY {
            initializer_stream.advance();
        }

        let parsed_initializer = match parsed_literal {
            CurlyLiteral::Ready(expression) => expression,
            CurlyLiteral::Retained(deferred) => {
                if !retain_pending_origin {
                    let deferral =
                        ExpressionParseError::NumericDeferral(Box::new(NumericDeferral {
                            items: vec![ExpressionRpnItem::PendingNumericExpression {
                                expression: Box::new(deferred),
                                span: literal_span,
                            }],
                            expected: ExpectedType::Infer,
                            value_mode: value_mode.to_owned(),
                            span: literal_span,
                            suffix: DeferredExpressionSuffix::None,
                            complete_fragment: true,
                        }));
                    return Err(deferral.reject_at_boundary_without_numeric_retention(
                        "the collection initializer of a declaration",
                    ));
                }
                ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;
                let continuation = register_pending_declaration(
                    context,
                    type_interner,
                    expression_builder,
                    path_fork,
                    qualified_name,
                    declaration_syntax.span,
                    binding_span,
                    value_mode.to_owned(),
                    declaration_syntax.binding_mode.is_mutable(),
                    vec![ExpressionRpnItem::PendingNumericExpression {
                        expression: Box::new(deferred),
                        span: literal_span,
                    }],
                    None,
                )?;
                return Ok(DeclarationLowering::PendingNumeric(continuation));
            }
        };

        // Shorthand already rejected empty literals during parsing, but immutable
        // empty fixed collections are also invalid for explicit fixed annotations.
        // Post-parse validation for token consumption and constant folding.
        let initializer_is_compile_time_constant =
            if declaration_syntax.binding_mode.is_compile_time() {
                initializer_is_compile_time_constant(
                    expression_builder.view_owned(parsed_initializer)?,
                    context,
                    expression_builder,
                    const_values,
                )?
            } else {
                true
            };

        if declaration_syntax.binding_mode.is_compile_time()
            && !initializer_is_compile_time_constant
        {
            return Err(choices_control_flow::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                path_fork.component(qualified_name),
                declaration_syntax.span,
            )
            .into());
        }

        ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;

        return Ok(DeclarationLowering::Ready(Declaration {
            id: qualified_name,
            type_id: expression_builder
                .view_owned(parsed_initializer)?
                .require_single(ValueReceiverContext::Declaration)?,
            type_syntax: None,
            value_mode: value_mode.to_owned(),
            value: DeclarationValue::Initializer(parsed_initializer.id()),
            binding_span: None,
            config_qualifier,
        }));
    }

    let resolved_annotation = {
        let mut type_resolution_context =
            TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
                declaration_table: &context.top_level_declarations,
                visible_declaration_ids: context.visible_declaration_ids.as_ref(),
                visible_external_symbols: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_external_symbols),
                visible_source_bindings: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_source_names),
                visible_type_aliases: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_type_alias_names),
                resolved_type_aliases: context.resolved_type_aliases.as_deref(),
                generic_declarations_by_path: context.generic_declarations_by_path.as_deref(),
                resolved_struct_fields_by_path: context.resolved_struct_fields_by_path.as_deref(),
                type_environment: type_interner.environment_mut_for_derived_types(),
                const_values,
                visible_namespace_records: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_namespace_records),
                trait_environment: Some(context.trait_environment()),
                trait_evidence_environment: Some(context.trait_evidence_environment()),
                visible_trait_names: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_trait_names),
            })
            .with_active_generic_type_context(context.active_generic_type_context());
        let semantic_type = declaration_syntax.semantic_type();
        resolve_parsed_type_annotation(
            &semantic_type,
            declaration_syntax.span,
            &mut type_resolution_context,
            string_table,
            Some(context),
        )?
    };
    // Declaration-boundary config bootstrap permits required and optional contracts to omit `=`.
    // The config owner validates the contract and supplies a provider/default before the ordinary
    // constant fold; do not force a second initializer parser or invent a placeholder value here.
    if config_resolution_context
        && declaration_syntax.config_qualifier.is_some()
        && declaration_syntax.initializer_range.is_none()
    {
        return Ok(DeclarationLowering::Ready(Declaration {
            id: qualified_name,
            type_id: resolved_annotation
                .type_id
                .unwrap_or(builtin_type_ids::NONE),
            type_syntax: None,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::PendingConstant,
            binding_span: None,
            config_qualifier,
        }));
    }
    if source_build_config_context && declaration_syntax.config_qualifier.is_some() {
        let name = path_fork.component(qualified_name).ok_or_else(|| {
            CompilerError::compiler_error(
                "source #Config declaration has no terminal declaration name",
            )
        })?;
        let name_text = string_table.resolve(name);
        let input_name = BuildInputName::new(name_text).map_err(|_| {
            CompilerError::compiler_error(format!(
                "source #Config declaration has invalid build-input name '{name_text}'"
            ))
        })?;
        let resolved = context
            .shared
            .source_build_config_values
            .as_ref()
            .and_then(|values| values.get(&input_name))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "source #Config declaration '{name_text}' has no resolved boundary value"
                ))
            })?;
        let value = expression_for_resolved_build_config_value(
            resolved,
            declaration_syntax.config_qualifier.as_ref().map_or_else(
                || declaration_syntax.span,
                |qualifier| qualifier.qualifier_span,
            ),
            type_interner,
            expression_builder,
            const_values,
            string_table,
        )?;
        return Ok(DeclarationLowering::Ready(Declaration {
            id: qualified_name,
            type_id: expression_builder
                .view_owned(value)?
                .require_single(ValueReceiverContext::Declaration)?,
            type_syntax: None,
            value_mode: expression_builder.view_owned(value)?.value_mode(),
            value: DeclarationValue::Initializer(value.id()),
            binding_span: None,
            config_qualifier,
        }));
    }
    let mut initializer_stream = declaration_initializer_stream(
        source_owner,
        declaration_syntax.initializer_range,
        declaration_syntax.span,
    )?;

    let parsed_initializer = match initializer_stream.current_tag() {
        // A leading `|` opens a struct definition; parenthesized const records parse
        // through the ordinary expression grammar below.
        TokenTag::TYPE_PARAMETER_BRACKET => {
            // Struct field defaults must be compile-time foldable, so they are parsed
            // in a dedicated constant context.
            let constant_context = ScopeContext::new_constant(qualified_name, context);
            let mut field_warnings = DiagnosticBag::new();
            let field_syntax = {
                let (field_syntax, next_index) = {
                    let mut declaration_cursor = initializer_stream.declaration_cursor()?;
                    let field_syntax = parse_struct_shell(
                        &mut declaration_cursor,
                        string_table,
                        &mut field_warnings,
                        qualified_name,
                        path_fork,
                    )?;
                    (field_syntax, declaration_cursor.position())
                };
                initializer_stream.set_position(next_index)?;
                field_syntax
            };
            for warning in field_warnings.into_diagnostics() {
                context.emit_warning(warning);
            }
            let source_owner = source_owner.ok_or_else(|| {
                CompilerError::compiler_error(
                    "struct field defaults have no canonical source token owner",
                )
            })?;

            let mut params = Vec::with_capacity(field_syntax.len());
            for field in &field_syntax {
                params.push(signature_member_to_declaration(
                    field,
                    source_owner,
                    &constant_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    SignatureTypeFallbackPolicy::StrictCapacity,
                    SignatureDefaultMode::FoldNow,
                    path_fork,
                    TypeMismatchContext::StructFieldDefault,
                )?);
            }

            ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;
            // A struct definition is nominal declaration syntax, never a foldable initializer.
            if declaration_syntax.binding_mode.is_compile_time() {
                return Err(choices_control_flow::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
                    path_fork.component(qualified_name),
                    declaration_syntax.span,
                )
                .into());
            }
            return Ok(DeclarationLowering::Ready(Declaration {
                id: qualified_name,
                type_id: resolved_annotation
                    .type_id
                    .unwrap_or(builtin_type_ids::NONE),
                type_syntax: None,
                value_mode,
                value: DeclarationValue::StructDefinition(params),
                binding_span: None,
                config_qualifier,
            }));
        }

        _ => {
            // Keep the canonical annotation TypeId beside the expression so
            // compatibility and coercion use the semantic identity directly.
            //
            // Pass parse-time context only where syntax requires it, such as
            // `none` and empty collection literals. Other expressions resolve
            // their natural type before this declaration boundary validates and
            // coerces them.
            let declared_type_id = resolved_annotation.type_id;
            let mut expression_type = declared_type_id
                .map(|type_id| parse_expectation_for_type_id(type_id, type_interner.environment()))
                .unwrap_or(ExpectedType::Infer);
            let mut cast_target_context = declared_type_id
                .map(|type_id| {
                    cast_target_context_for_type_id(
                        type_id,
                        type_interner.environment(),
                        string_table,
                        &*path_fork,
                    )
                })
                .unwrap_or(CastTargetContext::None);

            // The declaration owns its initializer's receiving context. An annotation is the
            // only expected type. An omitted annotation infers one value from the initializer
            // and never borrows an enclosing receiver's types, such as an outer value block's
            // target or the function's result slots.
            let expression_expected_results =
                declared_type_id.map_or_else(Vec::new, |type_id| vec![type_id]);
            let mut expression_context = context.new_child_expression(expression_expected_results);

            // Body-local compile-time constants need the same constant-reference rules
            // as top-level constants, but top-level header constants must keep their
            // stronger `ConstantHeader` context for const-record coercion.
            if declaration_syntax.binding_mode.is_compile_time()
                && !context.kind.is_constant_context()
            {
                expression_context.kind = ContextKind::Constant;
            } else {
                expression_context.kind = context.kind.clone();
            }

            let expression = if let Some(value_block_result) = try_parse_value_block_at_receiver(
                &mut initializer_stream,
                &expression_context,
                type_interner,
                expression_builder,
                const_values,
                &expression_context.expected_result_type_ids,
                ValueReceiverKind::Declaration,
                string_table,
                path_fork,
            ) {
                match value_block_result {
                    Ok(ParsedReceiverValue::Complete(expression)) => expression,
                    // A value block whose nested bodies or slot types only the enclosing body
                    // root can finish keeps its authored block: this boundary either owns the
                    // completion pass (a body-local runtime binding) and registers the deferral
                    // exactly like a retained fragment initializer, or it owns none and rejects
                    // the position instead of dropping the retained bodies.
                    Ok(ParsedReceiverValue::Retained(retained)) => {
                        let deferral = retained.into_deferral(&value_mode);
                        if !retain_pending_origin {
                            return Err(deferral.reject_at_boundary_without_numeric_retention(
                                "the value-block initializer of a body-local declaration",
                            ));
                        }
                        let deferral = deferral.take_numeric_deferral(
                            "the value-block initializer of a body-local declaration",
                        )?;
                        ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;
                        let continuation = register_pending_declaration(
                            context,
                            type_interner,
                            expression_builder,
                            path_fork,
                            qualified_name,
                            declaration_syntax.span,
                            binding_span,
                            value_mode.to_owned(),
                            declaration_syntax.binding_mode.is_mutable(),
                            deferral.items,
                            declared_type_id,
                        )?;
                        return Ok(DeclarationLowering::PendingNumeric(continuation));
                    }
                    Ok(ParsedReceiverValue::NeedsSlotInference { .. }) => {
                        return Err(CompilerError::compiler_error(
                            "a declaration initializer receiver knows its slot types; a value \
                             block cannot ask for slot inference",
                        )
                        .into());
                    }
                    Err(error) => {
                        return Err(error.reject_at_boundary_without_numeric_retention(
                            "the value-block initializer of a body-local declaration",
                        ));
                    }
                }
            } else {
                let mut input = ExpressionParseInput::ordinary(
                    ExpressionParseResources {
                        token_stream: &mut initializer_stream,
                        scope_context: &expression_context,
                        type_interner,
                        expression_builder,
                        const_values,
                        expected_type: &mut expression_type,
                        cast_target_context: &mut cast_target_context,
                        value_mode: &value_mode,
                        string_table,
                        path_fork,
                    },
                    false,
                );
                if retain_pending_origin {
                    // The initializer is the position's whole authored syntax and it owns its
                    // deferred fragment, so completion may splice it verbatim at the body root.
                    input = input.with_numeric_deferral_owner();
                    if declared_type_id.is_none() {
                        // Only an unannotated origin needs literal-only recipes retained: its
                        // single static type is still being collected, so a later exact receiver
                        // may still type the raw leaves. An explicit annotation already
                        // establishes the literal materialisation at this declaration boundary
                        // through the ordinary destination rules (a whole literal materialises
                        // directly in F32/Byte/Dec without an integer intermediate), so retaining
                        // it would only make the body-root pass re-derive the same type.
                        input = input.with_retained_pending_numeric_fragment();
                    }
                }
                match create_expression_with_trailing_newline_policy(input) {
                    Ok(expression) => expression,
                    Err(error) => {
                        if !retain_pending_origin {
                            // A compile-time or otherwise non-runtime initializer folds at this
                            // declaration boundary; it can never retain an unresolved numeric
                            // origin, and the boundary names the rejected position.
                            return Err(error.reject_at_boundary_without_numeric_retention(
                                "a compile-time or non-runtime declaration initializer",
                            ));
                        }
                        let deferral = error
                            .take_numeric_deferral("the initializer of a body-local declaration")?;
                        ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;
                        let continuation = register_pending_declaration(
                            context,
                            type_interner,
                            expression_builder,
                            path_fork,
                            qualified_name,
                            declaration_syntax.span,
                            binding_span,
                            value_mode.to_owned(),
                            declaration_syntax.binding_mode.is_mutable(),
                            deferral.items,
                            declared_type_id,
                        )?;
                        return Ok(DeclarationLowering::PendingNumeric(continuation));
                    }
                }
            };

            if config_resolution_context && declaration_syntax.config_qualifier.is_some() {
                // Config bootstrap validates the authored fallback itself. Do not let the
                // declaration's explicit annotation coerce or reject the fallback first, because
                // an explicit provider must not mask a malformed authored default.
                expression
            } else if let Some(declared_type_id) = declared_type_id {
                // This is an explicit typed boundary: apply ordinary contextual coercions in
                // one shared path.
                coerce_expression_to_explicit_type_boundary(
                    expression,
                    declared_type_id,
                    type_interner.environment(),
                    TypeMismatchContext::Declaration,
                    expression_builder,
                )?
            } else {
                expression
            }
        }
    }
    .id();

    expression_builder
        .view(parsed_initializer)?
        .require_single(ValueReceiverContext::Declaration)?;

    // Body-local compile-time constants must fully fold after parsing and coercion.
    let initializer_is_compile_time_constant = initializer_is_compile_time_constant(
        expression_builder.view(parsed_initializer)?,
        context,
        expression_builder,
        const_values,
    )?;

    if declaration_syntax.binding_mode.is_compile_time() && !initializer_is_compile_time_constant {
        return Err(choices_control_flow::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
            path_fork.component(qualified_name),
            declaration_syntax.span,
        )
        .into());
    }

    // Defensive: ensure the initializer parser consumed all tokens.
    // If tokens remain, the parser stopped early (e.g. a newline broke the
    // expression before it was complete). This prevents silent truncation.
    initializer_stream.skip_newlines();
    if initializer_stream.current_tag() == TokenTag::ELSE
        && type_interner
            .environment()
            .option_inner_type(
                expression_builder
                    .view(parsed_initializer)?
                    .value_type_id()?,
            )
            .is_some()
    {
        return Err(choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
            Some(initializer_stream.current_span()),
        )
        .into());
    }

    ensure_initializer_stream_ends(&mut initializer_stream, string_table)?;

    // Reject immutable bindings initialized with an empty fixed collection literal.
    // Mutable bindings are allowed because the collection may be filled later.
    if !value_mode.is_mutable()
        && let Some(type_id) = resolved_annotation.type_id
    {
        let env = type_interner.environment();
        if env.collection_fixed_capacity(type_id).is_some()
            && let ExpressionOperationView::Collection(items) =
                expression_builder.view(parsed_initializer)?.operation()
            && items.len() == 0
        {
            return Err(calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection,
                expression_builder.view(parsed_initializer)?.span(),
            )
            .into());
        }
    }

    ast_log!("Created new ", Cyan #value_mode);
    Ok(DeclarationLowering::Ready(Declaration {
        id: qualified_name,
        type_id: expression_builder
            .view(parsed_initializer)?
            .value_type_id()?,
        type_syntax: None,
        value_mode,
        value: DeclarationValue::Initializer(parsed_initializer),
        binding_span: None,
        config_qualifier,
    }))
}

/// Requires a declaration initializer to end after its single expression.
///
/// WHAT: skips trailing newlines and reports any leftover token as an unexpected token.
/// WHY: the expression parser stops at a token it cannot continue with, and shorthand literals
///      bypass it entirely, so every declaration boundary must check the remaining tokens itself,
///      including one whose numeric fragment is retained for body completion.
fn ensure_initializer_stream_ends(
    initializer_stream: &mut AstCursor<'_>,
    string_table: &mut StringTable,
) -> DeclarationResult<()> {
    initializer_stream.skip_newlines();
    if initializer_stream.current_tag() != TokenTag::EOF {
        let found = initializer_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerError::compiler_error(format!(
                    "declaration initializer diagnostic token payload was malformed: {error:?}"
                ))
            })?
            .unwrap_or_else(|| DiagnosticToken::from_static_tag(initializer_stream.current_tag()));
        return Err(token_lexical::unexpected_token_from_tag(
            found,
            Some(initializer_stream.current_span()),
        )
        .into());
    }
    Ok(())
}

/// Build the bounded parser cursor for a declaration initializer.
///
/// Source-owned ranges stay on the canonical `AstCursor` view. A missing owner is an
/// invariant failure: every declaration initializer is parsed through a nested canonical cursor.
fn declaration_initializer_stream<'tokens>(
    source_owner: Option<&AstCursor<'tokens>>,
    initializer_range: Option<TokenRange>,
    declaration_span: Option<SourceSpan>,
) -> DeclarationResult<AstCursor<'tokens>> {
    let range = initializer_range
        .ok_or_else(|| CompilerError::compiler_error("declaration initializer range is missing"))?;
    let eof_span = declaration_span
        .map(SourceSpan::local)
        .unwrap_or_else(LocalSpan::source_start);

    let source_owner = source_owner.ok_or_else(|| {
        CompilerError::compiler_error("declaration initializer has no canonical source owner")
    })?;
    let cursor = source_owner.nested_cursor(range).map_err(|error| {
        ExpressionParseError::from(CompilerError::compiler_error(format!(
            "declaration initializer range cursor construction failed: {error:?}"
        )))
    })?;
    let source_id = cursor.source_id();
    Ok(cursor.with_synthetic_eof_span(SourceSpan::new(source_id, eof_span)))
}
#[cfg(test)]
#[path = "tests/declaration_tests.rs"]
mod declaration_tests;
