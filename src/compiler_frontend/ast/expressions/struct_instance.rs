//! Struct constructor expression parsing.
//!
//! WHAT: parses `StructName(...)` call-like syntax and lowers it to a typed
//! `Expression::struct_instance` with definition-order fields, generic inference,
//! default filling, and const-context coercion.
//! WHY: struct constructors share the same argument-resolution pipeline as function
//! calls, so keeping the struct-specific wrapper close to the generic nominal
//! inference and call-validation modules makes the shared machinery easy to follow.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    NamedArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, expectations_from_constructor_fields,
    resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::constructor_views::ConstructorField;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::generic_nominal_inference::{
    GenericNominalConstructorInput, GenericNominalTemplate, infer_generic_nominal_constructor,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

/// Input bundle for `parse_struct_constructor_expression`.
///
/// WHAT: carries the struct path, field declarations, value mode, and base type id so
/// the caller (identifier expression dispatch) can hand off a fully-resolved struct
/// identity without repeating lookup work.
pub(crate) struct StructConstructorParseInput<'a> {
    pub(crate) struct_path: &'a InternedPath,
    pub(crate) struct_name: StringId,
    pub(crate) fields: &'a [Declaration],
    pub(crate) struct_value_mode: &'a ValueMode,
    pub(crate) type_id: TypeId,
}

/// Parse `StructName(...)` and return a finalized struct instance expression.
///
/// WHAT:
/// - Parses constructor arguments (positional and named) using the shared call-argument model.
/// - Validates arity, named-target lookup, duplicate detection, positional-before-named ordering,
///   default filling, missing required-field detection, and per-field type compatibility.
/// - Fills trailing fields from struct defaults when arguments are omitted.
/// - Produces a canonical `Expression::struct_instance` with definition-order fields.
///
/// WHY:
/// - Constructor syntax is syntactically identical to function call syntax; sharing the same
///   argument-resolution machinery keeps the two forms consistent and avoids a parallel
///   resolution system.
/// - Const-record coercion for top-level compile-time constants is applied after resolution.
pub(super) fn parse_struct_constructor_expression(
    token_stream: &mut FileTokens,
    input: StructConstructorParseInput<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let StructConstructorParseInput {
        struct_path,
        struct_name,
        fields,
        struct_value_mode,
        type_id,
    } = input;

    let constructor_location = token_stream.current_location();
    let struct_name_display = string_table.resolve(struct_name).to_owned();

    // The stream is positioned on the struct symbol when called.
    // Advance past it to '(' so the shared call-argument parser can take over.
    token_stream.advance();

    if token_stream.current_token_kind() != &TokenKind::OpenParenthesis {
        return Err(CompilerError::compiler_error(
            "Struct constructor parser called without an opening parenthesis",
        )
        .into());
    }

    // ------------------------
    //  Parse raw arguments with constructor-field expectations
    // ------------------------
    let constructor_field_views = ConstructorField::from_struct_declarations(fields);
    let field_expectations = expectations_from_constructor_fields(&constructor_field_views);
    let raw_args = parse_call_arguments_typed_with_expectations(
        token_stream,
        context,
        type_interner,
        string_table,
        &field_expectations,
        NamedArgumentSyntax::Supported {
            callee_name: Some(struct_name),
        },
    )?;

    // ------------------------
    //  Resolve generic instance
    // ------------------------
    let (resolved_fields, generic_instance_key, instance_type_id) = if let Some(generic_decls) =
        &context.generic_declarations_by_path
        && let Some(metadata) = generic_decls.get(struct_path)
        && !metadata.parameters.is_empty()
    {
        let inference = infer_generic_nominal_constructor(
            GenericNominalConstructorInput {
                nominal_path: struct_path,
                display_name: &struct_name_display,
                metadata,
                template: GenericNominalTemplate::StructFields(&constructor_field_views),
                constructor_fields: Some(&constructor_field_views),
                raw_args: Some(&raw_args),
                location: constructor_location.clone(),
            },
            context,
            type_interner,
            string_table,
        )?;

        let resolved_fields = if let Some(instance_type_id) = inference.instance_type_id {
            let type_env = type_interner.environment();
            type_env
                .fields_for(instance_type_id)
                .map(|field_defs| {
                    ConstructorField::from_field_definitions_with_defaults(field_defs, fields)
                })
                .unwrap_or_else(|| constructor_field_views.clone())
        } else {
            constructor_field_views.clone()
        };

        (
            resolved_fields,
            inference.instance_key,
            inference.instance_type_id,
        )
    } else {
        (constructor_field_views, None, None)
    };

    // ------------------------
    //  Validate arguments against fields
    // ------------------------
    let expectations = expectations_from_constructor_fields(&resolved_fields);
    let type_check_context = type_interner.type_check_context();
    let resolved_args = resolve_call_arguments(
        CallDiagnosticContext::struct_constructor(&struct_name_display),
        &raw_args,
        &expectations,
        constructor_location.clone(),
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )?;

    // ------------------------
    //  Build fields with const checks
    // ------------------------
    let enforce_const_record = context.kind.allows_const_record_coercion();
    let mut struct_fields = Vec::with_capacity(fields.len());

    for (field, arg) in resolved_fields.iter().zip(resolved_args.iter()) {
        let mut value = arg.value.clone();

        if enforce_const_record {
            // Header-stage struct shells may carry placeholder references until AST environment
            // construction resolves constants in graph order.
            let is_placeholder_reference = if let ExpressionKind::Reference(path) = &value.kind {
                path.name().is_some_and(|name| {
                    context.get_reference(&name).is_some_and(|declaration| {
                        declaration
                            .as_declaration()
                            .is_unresolved_constant_placeholder()
                    })
                })
            } else {
                false
            };

            let value_is_compile_time_constant = if is_placeholder_reference {
                true
            } else {
                value
                    .const_value_kind_with_template_classifier(&mut |template| {
                        classify_template_from_effective_tir(template, &context.template_ir_store)
                    })?
                    .is_compile_time_value()
            };

            if !value_is_compile_time_constant {
                let field_name = field
                    .name
                    .name()
                    .unwrap_or_else(|| string_table.intern("<field>"));
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext,
                    Some(field_name),
                    value.location,
                )
                .into());
            }

            // Const records are data-only exports, so mutable ownership is removed to keep
            // constant semantics explicit in later stages.
            value.value_mode = ValueMode::ImmutableOwned;
        }

        struct_fields.push(Declaration {
            id: field.name.clone(),
            value,
        });
    }

    // Const records force immutable ownership; otherwise inherit the struct's declared mode.
    let instance_ownership = if enforce_const_record {
        ValueMode::ImmutableOwned
    } else {
        struct_value_mode.as_owned()
    };

    let struct_expr = Expression::struct_instance(
        struct_path.to_owned(),
        struct_fields,
        constructor_location,
        instance_ownership,
        enforce_const_record,
        generic_instance_key,
        instance_type_id.unwrap_or(type_id),
    );

    Ok(struct_expr)
}
