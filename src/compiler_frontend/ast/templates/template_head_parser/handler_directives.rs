//! Handler-based style directive parsing.
//!
//! WHAT:
//! - Parses optional typed handler arguments.
//! - Normalizes argument values into `StyleDirectiveArgumentValue`.
//! - Applies handler effects and executes formatter factory callbacks.
//!
//! WHY:
//! - Project-owned directives and frontend handler directives share one execution
//!   contract, so this logic should be centralized and isolated from core directives.

use super::directive_args::parse_optional_parenthesized_expression;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveArgumentType, StyleDirectiveArgumentValue, StyleDirectiveEffects,
    StyleDirectiveHandlerSpec,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

/// Typed result shared by handler-directive parsing helpers.
type HandlerDirectiveResult<T> = Result<T, TemplateError>;

#[derive(Clone)]
struct ParsedHandlerDirectiveArgument {
    value: Option<StyleDirectiveArgumentValue>,
    error_location: SourceLocation,
}

pub(super) fn apply_handler_style_directive(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    build_state: &mut TemplateBuildState,
    directive_name: &str,
    handler_spec: &StyleDirectiveHandlerSpec,
    string_table: &mut StringTable,
) -> HandlerDirectiveResult<()> {
    let parsed_argument = parse_optional_handler_style_argument(
        token_stream,
        context,
        type_interner,
        directive_name,
        handler_spec.argument_type,
        string_table,
    )?;

    apply_style_directive_effects(build_state, handler_spec.effects);

    if let Some(factory) = handler_spec.formatter_factory {
        let formatter = factory(parsed_argument.value.as_ref()).map_err(|message| {
            let message_id = string_table.get_or_intern(message);
            TemplateError::from(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern(directive_name)),
                InvalidTemplateDirectiveReason::invalid_argument_with_detail(message_id),
                parsed_argument.error_location,
            ))
        })?;

        build_state.style.formatter = Some(formatter.clone());
    }

    Ok(())
}

fn apply_style_directive_effects(
    build_state: &mut TemplateBuildState,
    effects: StyleDirectiveEffects,
) {
    // Effects mutate semantic template style state. Formatter identity is set
    // separately by the optional formatter factory output.
    let style = &mut build_state.style;
    if let Some(style_id) = effects.style_id {
        style.id = style_id;
    }
    if let Some(policy) = effects.body_whitespace_policy {
        style.body_whitespace_policy = policy;
    }
    if let Some(suppress_child_templates) = effects.suppress_child_templates {
        style.suppress_child_templates = suppress_child_templates;
    }
    if let Some(skip_parent_wrappers) = effects.skip_parent_child_wrappers {
        style.skip_parent_child_wrappers = skip_parent_wrappers;
    }
}

/// Parses the optional handler argument and validates whether this directive
/// accepts one. Early exits keep the no-argument and invalid-argument cases
/// distinct from later normalization.
fn parse_optional_handler_style_argument(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    directive_name: &str,
    argument_type: Option<StyleDirectiveArgumentType>,
    string_table: &mut StringTable,
) -> HandlerDirectiveResult<ParsedHandlerDirectiveArgument> {
    let default_location = token_stream.current_location();
    let directive_name_id = string_table.intern(directive_name);

    let Some(expression) = parse_optional_parenthesized_expression(
        directive_name_id,
        token_stream,
        context,
        type_interner,
        string_table,
    )?
    else {
        return Ok(ParsedHandlerDirectiveArgument {
            value: None,
            error_location: default_location,
        });
    };

    let Some(argument_type) = argument_type else {
        return Err(CompilerDiagnostic::invalid_template_directive(
            Some(string_table.intern(directive_name)),
            InvalidTemplateDirectiveReason::DirectiveNotAllowedHere,
            default_location,
        )
        .into());
    };

    let argument_location = expression.location.clone();

    let argument_is_compile_time_constant = expression
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, &context.template_ir_store)
        })?
        .is_compile_time_value();

    if !argument_is_compile_time_constant {
        return Err(CompilerDiagnostic::invalid_template_directive(
            Some(string_table.intern(directive_name)),
            InvalidTemplateDirectiveReason::invalid_argument(),
            argument_location,
        )
        .into());
    }

    let normalized = normalize_provided_style_argument_value(
        expression,
        argument_type,
        directive_name,
        &argument_location,
        string_table,
    )?;

    Ok(ParsedHandlerDirectiveArgument {
        value: Some(normalized),
        error_location: argument_location,
    })
}

/// Normalizes a compile-time handler directive argument.
///
/// Early returns keep each rejected argument-kind branch close to its diagnostic.
fn normalize_provided_style_argument_value(
    expression: Expression,
    argument_type: StyleDirectiveArgumentType,
    directive_name: &str,
    argument_location: &SourceLocation,
    string_table: &mut StringTable,
) -> HandlerDirectiveResult<StyleDirectiveArgumentValue> {
    match argument_type {
        StyleDirectiveArgumentType::String => match expression.kind {
            ExpressionKind::StringSlice(text) => Ok(StyleDirectiveArgumentValue::String(
                string_table.resolve(text).to_owned(),
            )),
            _ => Err(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern(directive_name)),
                InvalidTemplateDirectiveReason::invalid_argument(),
                argument_location.clone(),
            )
            .into()),
        },

        StyleDirectiveArgumentType::Template => match expression.kind {
            ExpressionKind::Template(template) => Ok(StyleDirectiveArgumentValue::Template(
                Box::new(*template.to_owned()),
            )),
            _ => Err(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern(directive_name)),
                InvalidTemplateDirectiveReason::invalid_argument(),
                argument_location.clone(),
            )
            .into()),
        },

        StyleDirectiveArgumentType::Number => match expression.kind {
            ExpressionKind::Int(value) => Ok(StyleDirectiveArgumentValue::Number(value as f64)),
            ExpressionKind::Float(value) => Ok(StyleDirectiveArgumentValue::Number(value)),
            _ => Err(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern(directive_name)),
                InvalidTemplateDirectiveReason::invalid_argument(),
                argument_location.clone(),
            )
            .into()),
        },

        StyleDirectiveArgumentType::Bool => match expression.kind {
            ExpressionKind::Bool(value) => Ok(StyleDirectiveArgumentValue::Bool(value)),
            _ => Err(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern(directive_name)),
                InvalidTemplateDirectiveReason::invalid_argument(),
                argument_location.clone(),
            )
            .into()),
        },
    }
}
