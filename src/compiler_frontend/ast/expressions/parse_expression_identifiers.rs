//! Identifier-led expression parsing helpers.
//!
//! WHAT: parses identifier-led expression forms such as references, constructors, calls, and
//! namespace records.
//! WHY: identifier tokens fan out into the largest number of semantic cases and need isolated handling.

use super::choice_constructor::parse_choice_construct;
use super::error::ExpressionParseError;
use super::expression::{Expression, ExpressionKind};
use super::expression_rpn::ExpressionRpnItem;
use super::function_calls::{
    ExternalFunctionCallParseInput, parse_external_function_call_expression,
};
use super::namespace_access::{NamespaceAccessInput, parse_namespace_access};
use super::parse_expression_dispatch::{
    ExpressionOperandInput, push_expression_operand, push_expression_operand_at_location,
};
use super::source_function_calls::{SourceCallableMemberInput, parse_source_callable_member};
use super::struct_instance::{StructConstructorParseInput, parse_struct_constructor_expression};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::receiver_methods::free_function_receiver_method_call_error;
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::builtins::casts::traits::is_core_cast_trait_name;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, InvalidAssignmentTargetReason,
    InvalidTemplateSlotReason, InvalidThisUsageReason, NameNamespace,
};
use crate::compiler_frontend::external_packages::ExternalConstantValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) fn parse_identifier_or_call(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expected_result_evidence_allowed: bool,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    // Fast path for reserved receiver keyword `this`.
    if token_stream.current_token_kind() == &TokenKind::This {
        return parse_this_reference(
            token_stream,
            context,
            type_interner,
            expression,
            allow_boundary_catch,
            string_table,
        );
    }

    // One identifier token can expand into several expression forms: a local/reference read,
    // struct construction, source or external function call, or namespace record access.
    let TokenKind::Symbol(identifier) = token_stream.current_token_kind().to_owned() else {
        return Ok(());
    };

    if context.is_assignment_target_unavailable(identifier) {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(identifier),
            None,
            None,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    // ------------------------------------------------------------
    //  Local binding: reference, constructor, or function call
    // ------------------------------------------------------------
    if let Some(binding) = context.get_reference(&identifier) {
        // Template slot inserts are only legal inside template bodies, constant
        // initializers, or constant headers.
        if let ExpressionKind::Template(template_value) = &binding.value.kind {
            let is_slot_insert = context
                .template_ir_store
                .borrow()
                .get_template(template_value.tir_reference.root)
                .map(|template_ir| matches!(template_ir.kind, TemplateType::SlotInsert(_)))
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Identifier template reference pointed at a missing same-store template.",
                    )
                })?;
            if is_slot_insert
                && !matches!(
                    context.kind,
                    ContextKind::Template | ContextKind::Constant | ContextKind::ConstantHeader
                )
            {
                return Err(CompilerDiagnostic::invalid_template_slot(
                    InvalidTemplateSlotReason::InsertOutsideParentSlot,
                    None,
                    token_stream.current_location(),
                )
                .into());
            }
        }

        // Const records are field-access-only in runtime contexts.
        if binding.value.is_const_record_value()
            && token_stream.peek_next_token() != Some(&TokenKind::Dot)
            && !context.kind.is_constant_context()
        {
            return Err(CompilerDiagnostic::const_record_used_as_value(
                identifier,
                token_stream.current_location(),
            )
            .into());
        }

        // Struct constructors are parsed before constant-reference checks.
        // This keeps `x #= MyStruct(...)` on the constructor path so const
        // record coercion can validate field values instead of rejecting the
        // struct symbol itself as a non-constant reference.
        if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis)
            && let Some(struct_constructor) = context
                .source_struct_constructor(binding.as_declaration(), type_interner.environment())?
        {
            let struct_instance = parse_struct_constructor_expression(
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
                string_table,
            )?;

            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                expression,
                allow_boundary_catch,
                struct_instance,
            )?;

            return Ok(());
        }

        // Choice constructors are routed through their own parser.
        if token_stream.peek_next_token() == Some(&TokenKind::DoubleColon) {
            if context.is_source_choice_declaration(
                binding.as_declaration(),
                type_interner.environment(),
            )? {
                let choice_value = parse_choice_construct(
                    token_stream,
                    binding.as_declaration(),
                    context,
                    type_interner,
                    string_table,
                )?;
                push_expression_operand(
                    token_stream,
                    context,
                    type_interner,
                    string_table,
                    expression,
                    allow_boundary_catch,
                    choice_value,
                )?;

                return Ok(());
            }

            return Err(CompilerDiagnostic::namespace_misuse(
                identifier,
                NameNamespace::Type,
                NameNamespace::Value,
                token_stream.current_location(),
            )
            .into());
        }

        // Type declarations live in the type namespace. Constructor and variant syntax above are
        // the only expression-position routes that may start from a nominal type name.
        if context.is_nominal_type_declaration_path(&binding.id)
            && matches!(binding.value.kind, ExpressionKind::NoValue)
        {
            return Err(CompilerDiagnostic::namespace_misuse(
                identifier,
                NameNamespace::Value,
                NameNamespace::Type,
                token_stream.current_location(),
            )
            .into());
        }

        // Constant contexts reject non-constant local references. The unresolved
        // constant placeholder exemption is checked before TIR preparation so
        // placeholders that have not been folded yet are not rejected prematurely.
        // Template constness comes from preparation's exact effective view so composed
        // templates retain their module-local reference and overlays.
        if context.kind.is_constant_context() && !binding.is_unresolved_constant_placeholder() {
            let is_compile_time_constant = binding
                .value
                .const_value_kind_with_template_classifier(&mut |template| {
                    classify_template_from_effective_tir(template, &context.template_ir_store)
                })?
                .is_compile_time_value();

            if !is_compile_time_constant {
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
                    Some(identifier),
                    token_stream.current_location(),
                )
                .into());
            }
        }

        match context.source_callable_signature(binding.as_declaration()) {
            Some(signature) => {
                let generic_template = context.lookup_generic_function_template(&binding.id);
                let call_location = token_stream.current_location();

                parse_source_callable_member(SourceCallableMemberInput {
                    token_stream,
                    function_path: &binding.id,
                    signature,
                    generic_template,
                    visible_name: identifier,
                    call_location,
                    context,
                    expression,
                    allow_boundary_catch,
                    expected_result_evidence_allowed,
                    type_interner,
                    string_table,
                })?;

                return Ok(());
            }

            None => {
                let reference_location = token_stream.current_location();
                let reference_expression = reference_expression_from_declaration(
                    binding.as_declaration(),
                    context,
                    type_interner,
                    reference_location.clone(),
                );
                token_stream.advance();

                push_expression_operand_at_location(
                    token_stream,
                    context,
                    type_interner,
                    string_table,
                    expression,
                    allow_boundary_catch,
                    ExpressionOperandInput {
                        operand: reference_expression,
                        wrapper_location: reference_location,
                    },
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
        if token_stream.peek_next_token() == Some(&TokenKind::Dot) {
            return parse_namespace_access(NamespaceAccessInput {
                token_stream,
                context,
                type_interner,
                expression,
                allow_boundary_catch,
                expected_result_evidence_allowed,
                root_name: identifier,
                root_record: record,
                string_table,
            });
        }

        return Err(CompilerDiagnostic::dependency_namespace_used_as_value(
            identifier,
            token_stream.current_location(),
        )
        .into());
    }

    // ------------------------------------
    //  External constant
    // ------------------------------------
    if let Some((_const_id, const_def)) = context.lookup_visible_external_constant(identifier) {
        token_stream.advance();
        let location = token_stream.current_location();

        if context.kind.is_constant_context() && !const_def.value.is_scalar() {
            return Err(CompilerDiagnostic::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext,
                Some(identifier),
                location,
            )
            .into());
        }

        let value_mode = ValueMode::ImmutableOwned;
        let const_expr = match const_def.value {
            ExternalConstantValue::Float(value) => Expression::float(value, location, value_mode),
            ExternalConstantValue::Int(value) => Expression::int(value, location, value_mode),
            ExternalConstantValue::StringSlice(value) => {
                let string_id = string_table.intern(value);
                Expression::string_slice(string_id, location, value_mode)
            }
            ExternalConstantValue::Bool(value) => Expression::bool(value, location, value_mode),
        };

        push_expression_operand(
            token_stream,
            context,
            type_interner,
            string_table,
            expression,
            allow_boundary_catch,
            const_expr,
        )?;
        return Ok(());
    }

    // ------------------------------------
    //  Host function call
    // ------------------------------------
    if let Some((function_id, host_function_definition)) =
        context.lookup_visible_external_function(identifier)
    {
        if context.kind.is_constant_context() {
            return Err(CompilerDiagnostic::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ExternalFunctionCallInConstantContext,
                Some(identifier),
                token_stream.current_location(),
            )
            .into());
        }

        // External calls parse from metadata directly; do not synthesize fake parameter declarations.
        let call_location = token_stream.current_location();
        token_stream.advance();

        let function_call_expression =
            parse_external_function_call_expression(ExternalFunctionCallParseInput {
                token_stream,
                external_function_id: function_id,
                external_function: host_function_definition,
                call_location,
                context,
                value_required: true,
                allow_boundary_catch: allow_boundary_catch
                    && expression.is_empty()
                    && fallible_catch_allowed_in_context(context),
                warnings: None,
                type_interner,
                string_table,
            })?;

        push_expression_operand(
            token_stream,
            context,
            type_interner,
            string_table,
            expression,
            allow_boundary_catch,
            function_call_expression,
        )?;

        return Ok(());
    }

    // Receiver methods cannot be called as free functions.
    if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis)
        && let Some(method_entry) = context.lookup_visible_receiver_method_by_name(identifier)
    {
        let diagnostic = free_function_receiver_method_call_error(
            identifier,
            method_entry,
            token_stream.current_location(),
            string_table,
        );

        return Err(diagnostic.into());
    }

    // External types cannot be constructed with struct literal syntax.
    if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis)
        && context.lookup_visible_external_type(identifier).is_some()
    {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported,
            Some(identifier),
            token_stream.current_location(),
        )
        .into());
    }

    // Type aliases are type-namespace only.
    if context.is_visible_type_alias_name(identifier) {
        return Err(CompilerDiagnostic::namespace_misuse(
            identifier,
            NameNamespace::Value,
            NameNamespace::Type,
            token_stream.current_location(),
        )
        .into());
    }

    // Core cast trait names are reserved static contracts. They are valid in trait
    // declarations, conformances, and generic bounds, but never as ordinary values.
    if is_core_cast_trait_name(string_table.resolve(identifier)) {
        return Err(CompilerDiagnostic::trait_name_used_as_type(
            identifier,
            token_stream.current_location(),
        )
        .into());
    }

    Err(CompilerDiagnostic::unknown_value_name(identifier, token_stream.current_location()).into())
}

/// Parse a `this` reference inside a receiver method body.
///
/// WHAT: validates that `this` is in scope (i.e. the current function declared a receiver)
/// and emits a reference node identical to a normal local read.
/// WHY: `this` is a reserved keyword token, not an ordinary identifier, so it needs its own
/// parse path, but semantically it behaves like any other parameter reference.
fn parse_this_reference(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let this_id = string_table.intern("this");

    if context.is_assignment_target_unavailable(this_id) {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(this_id),
            None,
            None,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    let Some(receiver_declaration) = context.get_reference(&this_id) else {
        return Err(CompilerDiagnostic::invalid_this_usage(
            InvalidThisUsageReason::NotInReceiverMethod,
            token_stream.current_location(),
        )
        .into());
    };

    let reference_location = token_stream.current_location();
    let reference_expression = reference_expression_from_declaration(
        receiver_declaration.as_declaration(),
        context,
        type_interner,
        reference_location.clone(),
    );
    token_stream.advance();

    push_expression_operand_at_location(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        ExpressionOperandInput {
            operand: reference_expression,
            wrapper_location: reference_location,
        },
    )?;

    Ok(())
}
