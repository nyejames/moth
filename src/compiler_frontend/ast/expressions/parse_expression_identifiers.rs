//! Identifier-led expression parsing helpers.
//!
//! WHAT: parses identifier-led expression forms such as references, constructors, calls, and
//! namespace records.
//! WHY: identifier tokens fan out into the largest number of semantic cases and need isolated handling.

use super::choice_constructor::parse_choice_construct;
use super::constructor_views::ParsedConstructorExpression;
use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::expression_store::{AstExpressionBuilder, ExpressionOperationView};
use super::external_namespace_members::project_external_constant;
use super::function_calls::{
    ExternalFunctionCallParseInput, ParsedFunctionCallExpression,
    parse_external_function_call_expression,
};
use super::namespace_access::{NamespaceAccessInput, parse_namespace_access};
use super::parse_expression_dispatch::{
    ExpressionOperandInput, push_expression_operand, push_expression_operand_with_span,
    push_pending_numeric_binding, stream_is_boundary_leading,
};
use super::parse_expression_input::RouteArgumentContext;
use super::source_function_calls::{SourceCallableMemberInput, parse_source_callable_member};
use super::struct_instance::{StructConstructorParseInput, parse_struct_constructor_expression};
use crate::compiler_frontend::ast::ast_nodes::{DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::receiver_methods::free_function_receiver_method_call_error;
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::builtins::casts::traits::is_core_cast_trait_name;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, InvalidAssignmentTargetReason, InvalidCallShapeReason,
    InvalidTemplateSlotReason, NameNamespace,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

#[allow(
    clippy::too_many_arguments,
    reason = "identifier parsing keeps the token stream, scope, mutable rpn/interner/string/path state, and the catch and evidence policy flags as separate borrows"
)]
pub(super) fn parse_identifier_or_call(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expected_result_evidence_allowed: bool,
    route_argument_context: Option<RouteArgumentContext>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    // Fast path for reserved receiver keyword `this`.
    if token_stream.current_tag() == TokenTag::THIS {
        return parse_this_reference(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            expression,
            allow_boundary_catch,
            string_table,
            path_fork,
        );
    }

    // One identifier token can expand into several expression forms: a local/reference read,
    // struct construction, source or external function call, or namespace record access.
    if token_stream.current_tag() != TokenTag::SYMBOL {
        return Ok(());
    }
    let identifier = token_stream
        .current_string_id_in(string_table)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "symbol token had no string payload",
            )
        })?;

    if context.is_assignment_target_unavailable(identifier) {
        return Err(names_declarations::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(identifier),
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // ------------------------------------------------------------
    //  Local binding: reference, constructor, or function call
    // ------------------------------------------------------------
    if let Some(binding) =
        context.get_value_reference(&identifier, Some(token_stream.current_span()))?
    {
        // An eligible body-local numeric binding has no declaration, `TypeId` or default until
        // the body root selects its single static type. Retain the authored reference as a
        // construction-only operand item so the expression entry raises the body-root deferral
        // for the whole fragment instead of fabricating a value or reporting an absent name.
        if let Some(pending_binding) = binding.pending_binding() {
            if route_argument_context.is_some() {
                return Err(calls_data::invalid_call_shape(
                    InvalidCallShapeReason::RouteArgumentNotDirect,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }
            // A pending numeric binding is a runtime body-local value, and constants fold at
            // their declaration boundary, so a constant context can never consume it.
            if context.kind.is_constant_context() {
                return Err(ExpressionParseError::NonConstantReferenceInConstant(
                    choices_control_flow::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
                        Some(identifier),
                        Some(token_stream.current_span()),
                    ),
                ));
            }

            // Variant syntax names a choice type, and a pending numeric binding is a value.
            if token_stream.peek_next_tag() == Some(TokenTag::DOUBLE_COLON) {
                return Err(names_declarations::namespace_misuse(
                    identifier,
                    NameNamespace::Type,
                    NameNamespace::Value,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            let reference_span = Some(token_stream.current_span());
            token_stream.advance();
            push_pending_numeric_binding(
                context,
                expression,
                pending_binding,
                reference_span,
                false,
            )?;
            return Ok(());
        }

        // A ready local or shared declaration: the reference path needs its declaration, and the
        // pending case above is the only shape without one.
        let Some(declaration) = binding.ready_declaration() else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "a local binding reference carried neither a ready declaration nor a pending \
                 numeric binding",
            )
            .into());
        };
        if let Some(route_context) = route_argument_context
            && (token_stream.peek_next_tag() != Some(TokenTag::DOUBLE_COLON)
                || !route_context
                    .matches_output_type(declaration.type_id, type_interner.environment())
                || !context.is_source_choice_declaration(
                    declaration,
                    expression_builder,
                    const_values,
                )?)
        {
            return Err(calls_data::invalid_call_shape(
                InvalidCallShapeReason::RouteArgumentNotDirect,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }

        // Template slot inserts are only legal inside template bodies, constant
        // initializers, or constant headers.
        if let Some(initializer) = declaration.value.initializer()
            && let ExpressionOperationView::Template(template_value) =
                expression_builder.view(initializer)?.operation()
        {
            let is_slot_insert = context
                .template_ir_store
                .borrow()
                .get_template(template_value.tir_reference.root)
                .map(|template_ir| matches!(template_ir.kind, TemplateType::SlotInsert(_)))
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "Identifier template reference pointed at a missing same-store template.",
                    )
                })?;
            if is_slot_insert
                && !matches!(
                    context.kind,
                    ContextKind::Template | ContextKind::Constant | ContextKind::ConstantHeader
                )
            {
                return Err(templates::invalid_template_slot(
                    InvalidTemplateSlotReason::InsertOutsideParentSlot,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }
        }

        // Const records are field-access-only in runtime contexts.
        if declaration
            .value
            .is_const_record_value(expression_builder)?
            && token_stream.peek_next_tag() != Some(TokenTag::DOT)
            && !context.kind.is_constant_context()
        {
            return Err(names_declarations::const_record_used_as_value(
                identifier,
                Some(token_stream.current_span()),
            )
            .into());
        }

        // Struct constructors are parsed before constant-reference checks.
        // This keeps `x #= MyStruct(...)` on the constructor path so const
        // record coercion can validate field values instead of rejecting the
        // struct symbol itself as a non-constant reference.
        if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS)
            && let Some(struct_constructor) = context.source_struct_constructor(
                declaration,
                type_interner.environment(),
                expression_builder,
                const_values,
            )?
        {
            match parse_struct_constructor_expression(
                token_stream,
                StructConstructorParseInput {
                    struct_path: &struct_constructor.struct_path,
                    struct_name: identifier,
                    fields: struct_constructor.fields,
                    struct_value_mode: struct_constructor.struct_value_mode,
                    type_id: struct_constructor.type_id,
                },
                context,
                type_interner,
                expression_builder,
                const_values,
                path_fork,
                string_table,
            )? {
                ParsedConstructorExpression::Value(struct_instance) => {
                    push_expression_operand(
                        token_stream,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        expression,
                        allow_boundary_catch,
                        struct_instance,
                        path_fork,
                    )?;
                }
                // A constructor whose numeric-dependent arguments were retained takes its
                // authored operand position as one pending item; the enclosing fragment either
                // completes where it was parsed or defers to the body root with this item in
                // place, so the constructor resumes exactly once at its original position.
                ParsedConstructorExpression::RetainedOperand(item) => expression.push(item),
            }

            return Ok(());
        }

        // Only an immediate `::` or following symbol begins a choice constructor. A newline or
        // comment after the type name belongs to the existing separator diagnostic path.
        if context.is_source_choice_declaration(declaration, expression_builder, const_values)?
            && matches!(
                token_stream.peek_next_tag(),
                Some(TokenTag::DOUBLE_COLON | TokenTag::SYMBOL)
            )
        {
            match parse_choice_construct(
                token_stream,
                declaration,
                context,
                type_interner,
                expression_builder,
                const_values,
                path_fork,
                string_table,
                route_argument_context,
            )? {
                ParsedConstructorExpression::Value(choice_value) => {
                    push_expression_operand(
                        token_stream,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        expression,
                        allow_boundary_catch,
                        choice_value,
                        path_fork,
                    )?;
                }
                // A payload constructor whose numeric-dependent arguments were retained takes
                // its authored operand position as one pending item and resumes once at the root.
                ParsedConstructorExpression::RetainedOperand(item) => expression.push(item),
            }

            return Ok(());
        }

        if token_stream.peek_next_tag() == Some(TokenTag::DOUBLE_COLON) {
            return Err(names_declarations::namespace_misuse(
                identifier,
                NameNamespace::Type,
                NameNamespace::Value,
                Some(token_stream.current_span()),
            )
            .into());
        }

        // Type declarations live in the type namespace. Constructor and variant syntax above are
        // the only expression-position routes that may start from a nominal type name.
        if matches!(
            declaration.value,
            DeclarationValue::Shell(
                DeclarationShell::Struct | DeclarationShell::Choice | DeclarationShell::TypeAlias
            )
        ) {
            return Err(names_declarations::namespace_misuse(
                identifier,
                NameNamespace::Value,
                NameNamespace::Type,
                Some(token_stream.current_span()),
            )
            .into());
        }

        // Constant contexts reject non-constant local references. The unresolved
        // constant placeholder exemption is checked before TIR preparation so
        // placeholders that have not been folded yet are not rejected prematurely.
        // Template constness comes from preparation's exact effective view so composed
        // templates retain their module-local reference and overlays.
        if context.kind.is_constant_context()
            && !context.is_unresolved_constant_placeholder(declaration)
        {
            let is_compile_time_constant = match declaration.value.initializer() {
                Some(initializer) => expression_builder
                    .view(initializer)?
                    .const_value_kind_with_template_classifier(const_values, &mut |template| {
                        classify_template_from_effective_tir(
                            template,
                            &context.template_ir_store,
                            expression_builder,
                            const_values,
                            Some(context.external_package_registry.as_ref()),
                        )
                    })?
                    .is_compile_time_value(),
                // Imported constants carry their folded value instead of an initializer.
                None => context.is_explicit_compile_time_constant(declaration),
            };

            if !is_compile_time_constant {
                return Err(ExpressionParseError::NonConstantReferenceInConstant(
                    choices_control_flow::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
                        Some(identifier),
                        Some(token_stream.current_span()),
                    ),
                ));
            }
        }

        match context.source_callable_signature(declaration) {
            Some(signature) => {
                let generic_template = context.lookup_generic_function_template(&declaration.id);
                let call_span = Some(token_stream.current_span());
                let expression_is_boundary_leading = stream_is_boundary_leading(expression);

                parse_source_callable_member(SourceCallableMemberInput {
                    token_stream,
                    function_path: &declaration.id,
                    signature,
                    generic_template,
                    visible_name: identifier,
                    call_span,
                    context,
                    expression,
                    expression_is_boundary_leading,
                    allow_boundary_catch,
                    expected_result_evidence_allowed,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                })?;

                return Ok(());
            }

            None => {
                let reference_span = Some(token_stream.current_span());
                let reference_expression = reference_expression_from_declaration(
                    declaration,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    reference_span,
                )?;
                token_stream.advance();

                push_expression_operand_with_span(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    expression,
                    allow_boundary_catch,
                    ExpressionOperandInput {
                        operand: reference_expression,
                        wrapper_span: reference_span,
                    },
                    path_fork,
                )?;
                return Ok(()); // Will have moved onto the next token already
            }
        }
    }

    // ------------------------------------
    //  Namespace record access
    // ------------------------------------
    if let Some(record) = context
        .file_visibility
        .as_ref()
        .and_then(|fv| fv.visible_namespace_records.get(&identifier))
    {
        if token_stream.peek_next_tag() == Some(TokenTag::DOT) {
            return parse_namespace_access(NamespaceAccessInput {
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                expression,
                allow_boundary_catch,
                expected_result_evidence_allowed,
                root_name: identifier,
                root_record: record,
                path_fork,
                string_table,
            });
        }

        return Err(project_config::dependency_namespace_used_as_value(
            identifier,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // ------------------------------------
    //  External constant
    // ------------------------------------
    if let Some((_const_id, const_def)) = context.lookup_visible_external_constant(identifier) {
        let span = Some(token_stream.current_span());
        token_stream.advance();

        if context.kind.is_constant_context() && !const_def.value.is_scalar() {
            return Err(choices_control_flow::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext,
                Some(identifier),
                span,
            )
            .into());
        }

        let value_mode = ValueMode::ImmutableOwned;
        let const_expr = project_external_constant(
            const_def,
            identifier,
            span,
            value_mode,
            string_table,
            expression_builder,
            const_values,
        )?;

        push_expression_operand(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            expression,
            allow_boundary_catch,
            const_expr,
            path_fork,
        )?;
        return Ok(());
    }

    // ------------------------------------
    //  Host function call
    // ------------------------------------
    if let Some((function_id, host_function_definition)) =
        context.lookup_visible_external_function(identifier)
    {
        let call_span = Some(token_stream.current_span());
        // Host-call argument parsing starts at the opening parenthesis; lookup leaves the cursor
        // on the identifier itself.
        token_stream.advance();

        let parsed_call =
            parse_external_function_call_expression(ExternalFunctionCallParseInput {
                token_stream,
                external_function_id: function_id,
                external_function: host_function_definition,
                call_span,
                context,
                value_required: true,
                allow_boundary_catch: allow_boundary_catch
                    && expression.is_empty()
                    && fallible_catch_allowed_in_context(context),
                warnings: None,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            })?;

        match parsed_call {
            ParsedFunctionCallExpression::Value(function_call_expression) => {
                push_expression_operand(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    expression,
                    allow_boundary_catch,
                    function_call_expression,
                    path_fork,
                )?;
            }
            // A call whose numeric-dependent arguments were retained takes its authored operand
            // position as one pending item; the enclosing fragment either completes where it was
            // parsed or defers to the body root with this item in place, so the call resumes
            // exactly once at its original position.
            ParsedFunctionCallExpression::RetainedOperand(item) => {
                expression.push(item);
            }
        }

        return Ok(());
    }

    // Receiver methods cannot be called as free functions.
    if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS)
        && let Some(method_entry) = context.lookup_visible_receiver_method_by_name(identifier)
    {
        let diagnostic = free_function_receiver_method_call_error(
            identifier,
            method_entry,
            Some(token_stream.current_span()),
            string_table,
        );

        return Err(diagnostic.into());
    }

    // External types cannot be constructed with struct literal syntax.
    if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS)
        && context.lookup_visible_external_type(identifier).is_some()
    {
        return Err(choices_control_flow::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported,
            Some(identifier),
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Type aliases are type-namespace only.
    if context.is_visible_type_alias_name(identifier) {
        return Err(names_declarations::namespace_misuse(
            identifier,
            NameNamespace::Value,
            NameNamespace::Type,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Core cast trait names are reserved static contracts. They are valid in trait
    // declarations, conformances, and generic bounds, but never as ordinary values.
    if is_core_cast_trait_name(string_table.resolve(identifier)) {
        return Err(generics_traits_api::trait_name_used_as_type(
            identifier,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Err(
        names_declarations::unknown_value_name(identifier, Some(token_stream.current_span()))
            .into(),
    )
}

/// Parse a `this` reference inside a receiver method body.
///
/// WHAT: validates that `this` is in scope (i.e. the current function declared a receiver)
/// and emits a reference node identical to a normal local read.
/// WHY: `this` is a reserved keyword token, not an ordinary identifier, so it needs its own
/// parse path, but semantically it behaves like any other parameter reference.
#[allow(
    clippy::too_many_arguments,
    reason = "this-reference parsing keeps the token stream, scope, type interner, expression builder, const store, expression output, catch policy, string table, and path fork as separate borrows"
)]
fn parse_this_reference(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let this_id = string_table.intern("this");
    let receiver_declaration =
        context.resolve_this_reference(this_id, token_stream.current_span())?;
    // `this` names the reserved receiver parameter, which is never a pending numeric body local,
    // so a pending reference here is a broken invariant rather than an unresolved value.
    let receiver_declaration = receiver_declaration.ready_declaration().ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "`this` resolved to a pending numeric binding",
        )
    })?;
    let reference_span = Some(token_stream.current_span());
    let reference_expression = reference_expression_from_declaration(
        receiver_declaration,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        reference_span,
    )?;
    token_stream.advance();

    push_expression_operand_with_span(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        expression,
        allow_boundary_catch,
        ExpressionOperandInput {
            operand: reference_expression,
            wrapper_span: reference_span,
        },
        path_fork,
    )?;

    Ok(())
}
