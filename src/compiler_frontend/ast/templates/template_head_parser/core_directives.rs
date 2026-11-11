//! Core directive parsing and guardrails.
//!
//! WHAT:
//! - Handles compiler-owned core directives in template heads.
//! - Keeps structural helper parsing separate from generic style-handler logic.
//!
//! WHY:
//! - Core directives encode language semantics and structural helpers, so their
//!   control flow belongs in one dedicated module.

use super::child_decorators::{ChildDecoratorEdge, parse_child_decorator_directive};
use super::directive_args::{reject_directive_operand, reject_extra_directive_operand};
use super::head_parser::parse_directive_operand_expression_with_resources;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::ExpressionParseResources;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::styles::raw::configure_raw_style;
use crate::compiler_frontend::ast::templates::template::{
    BodyWhitespacePolicy, SlotDeclaration, SlotKey, Style, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, InvalidTemplateDirectiveReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::style_directives::{
    AFTER_DIRECTIVE_NAME, BEFORE_DIRECTIVE_NAME, CoreStyleDirectiveKind, INSERT_DIRECTIVE_NAME,
    SLOT_DIRECTIVE_NAME, StyleDirectiveKind,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{
    cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::grammar::{NumericLiteralKind, NumericLiteralSign};

/// Typed result for the connected core-directive family.
type CoreDirectiveResult<T> = Result<T, TemplateError>;

/// Parses compiler-owned structural helpers that own their trailing tokens.
///
/// A successful return leaves the cursor on the separator or body boundary, so
/// the head parser must not advance again.
#[allow(
    clippy::too_many_arguments,
    reason = "structural helper dispatch keeps the token stream, scope, mutable interner/build/string/path state, and the directive kind as separate borrows"
)]
pub(super) fn maybe_parse_structural_helper_directive(
    directive_kind: &StyleDirectiveKind,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    build_state: &mut TemplateBuildState,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> CoreDirectiveResult<bool> {
    if matches!(
        directive_kind,
        StyleDirectiveKind::Core(CoreStyleDirectiveKind::Slot)
    ) {
        let slot_name = string_table.intern(SLOT_DIRECTIVE_NAME);
        let declaration = parse_slot_declaration(
            slot_name,
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?;
        build_state.kind = TemplateType::SlotDefinition(declaration);
        return Ok(true);
    }

    if matches!(
        directive_kind,
        StyleDirectiveKind::Core(CoreStyleDirectiveKind::In)
    ) {
        let in_name = string_table.intern(INSERT_DIRECTIVE_NAME);
        let slot_name = parse_in_target(in_name, token_stream, string_table)?;
        build_state.kind = TemplateType::SlotInsert(SlotKey::named(slot_name));
        return Ok(true);
    }

    let decorator_edge = match directive_kind {
        StyleDirectiveKind::Core(CoreStyleDirectiveKind::Before) => ChildDecoratorEdge::Before,
        StyleDirectiveKind::Core(CoreStyleDirectiveKind::After) => ChildDecoratorEdge::After,
        _ => return Ok(false),
    };
    let directive_name = string_table.intern(match decorator_edge {
        ChildDecoratorEdge::Before => BEFORE_DIRECTIVE_NAME,
        ChildDecoratorEdge::After => AFTER_DIRECTIVE_NAME,
    });
    parse_child_decorator_directive(
        directive_name,
        decorator_edge,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        build_state,
        string_table,
        path_fork,
    )?;
    Ok(true)
}

/// Parses `$slot`, `$slot name`, `$slot 1` and an optional `= <compile-time String>`.
///
/// The caller leaves the cursor on the directive token. A successful return
/// leaves it after the declaration so the head parser continues with the
/// separator or body boundary.
#[allow(
    clippy::too_many_arguments,
    reason = "slot defaults reuse the head expression parser and keep its separate owners as explicit borrows"
)]
pub(crate) fn parse_slot_declaration(
    directive_name: StringId,
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> CoreDirectiveResult<SlotDeclaration> {
    reject_parenthesized_slot_syntax(directive_name, token_stream)?;

    let key = match token_stream.current_tag() {
        TokenTag::SYMBOL => {
            let name = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "slot name token had no string payload",
                    )
                })?;
            token_stream.advance();
            SlotKey::Named(name)
        }

        TokenTag::NUMERIC_LITERAL => {
            let position = positive_slot_position(directive_name, token_stream, string_table)?;
            token_stream.advance();
            SlotKey::Positional(position)
        }

        TokenTag::ASSIGN
        | TokenTag::TEMPLATE_CLOSE
        | TokenTag::START_TEMPLATE_BODY
        | TokenTag::COMMA
        | TokenTag::EOF => SlotKey::Default,

        _ => {
            return Err(templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::InvalidSlotTarget,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    token_stream.skip_newlines();
    let default = if token_stream.current_tag() == TokenTag::ASSIGN {
        token_stream.advance();
        token_stream.skip_newlines();
        let string_type = type_interner.environment().builtins().string;
        let mut expected_type =
            parse_expectation_for_type_id(string_type, type_interner.environment());
        let mut cast_target_context = cast_target_context_for_type_id(
            string_type,
            type_interner.environment(),
            string_table,
            path_fork,
        );
        Some(parse_slot_default(
            directive_name,
            ExpressionParseResources {
                token_stream,
                scope_context: context,
                type_interner,
                expression_builder,
                const_values,
                expected_type: &mut expected_type,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
                path_fork,
            },
        )?)
    } else {
        None
    };

    token_stream.skip_newlines();
    reject_slot_body(token_stream)?;

    Ok(SlotDeclaration::new(key, default))
}

/// Parses the required symbol target of `$in name`.
///
/// `$in` never accepts a default or a positional target. A successful return
/// leaves the cursor after the name.
pub(crate) fn parse_in_target(
    directive_name: StringId,
    token_stream: &mut AstCursor<'_>,
    string_table: &mut StringTable,
) -> CoreDirectiveResult<StringId> {
    reject_parenthesized_slot_syntax(directive_name, token_stream)?;

    let target = token_stream.current_tag();
    if target != TokenTag::SYMBOL {
        if slot_declaration_ends_at(target) {
            return Err(templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::MissingArgument,
                Some(token_stream.current_span()),
            )
            .into());
        }

        return Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::InvalidInsertTarget,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let name = token_stream
        .current_string_id_in(string_table)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "slot insert target had no string payload",
            )
        })?;
    token_stream.advance();
    token_stream.skip_newlines();
    Ok(name)
}

/// Advances past the directive token, rejecting the removed parenthesized forms.
fn reject_parenthesized_slot_syntax(
    directive_name: StringId,
    token_stream: &mut AstCursor<'_>,
) -> CoreDirectiveResult<()> {
    token_stream.advance();
    token_stream.skip_newlines();

    if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
        return Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::UnexpectedArguments,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Parses one optional compile-time `String` slot default.
fn parse_slot_default(
    directive_name: StringId,
    resources: ExpressionParseResources<'_, '_, '_>,
) -> CoreDirectiveResult<ConstValueId> {
    let ExpressionParseResources {
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        expected_type,
        cast_target_context,
        value_mode,
        string_table,
        path_fork,
    } = resources;
    if slot_declaration_ends_at(token_stream.current_tag()) {
        return Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::MissingArgument,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let fallback_span = token_stream.current_span();
    // A slot default is the directive's one operand. The parse keeps the caller's explicit
    // receiving type and stops before a second operand so this boundary owns that diagnostic.
    let expression =
        match parse_directive_operand_expression_with_resources(ExpressionParseResources {
            token_stream,
            scope_context,
            type_interner,
            expression_builder: &mut *expression_builder,
            const_values: &mut *const_values,
            expected_type,
            cast_target_context,
            value_mode,
            string_table,
            path_fork,
        }) {
            Ok(expression) => expression,
            // A slot default must be fully known here, so a body-root numeric
            // deferral is an invalid default rather than a retainable fragment.
            Err(ExpressionParseError::NumericDeferral(deferral)) => {
                return Err(templates::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidSlotDefault,
                    Some(deferral.span.unwrap_or(fallback_span)),
                )
                .into());
            }
            Err(error) => return Err(error.into()),
        };
    reject_extra_directive_operand(directive_name, token_stream)?;

    let expression = expression_builder.view_owned(expression)?;
    let span = expression.span().or(Some(fallback_span));
    let Some(value) = constant_string_value_id(expression, const_values) else {
        return Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::InvalidSlotDefault,
            span,
        )
        .into());
    };
    // A folded default expression (for example `[configured_label]`) carries synthetic-interface
    // provenance on its occurrence, not on the constant it resolved to. Stage this use through
    // the shared provenance-aware constant path so folding and public projection keep the
    // project dependency instead of reporting a portable fallback.
    Ok(const_values.with_occurrence_provenance(expression, value)?)
}

/// Reads the constant `String` id behind a compile-time expression.
///
/// Coercion wrappers are transparent while validating the payload, but the
/// returned id stays the expression's outermost constant. The shared constant
/// resolver unions provenance across the stored coercion chain, so unwrapping
/// to the inner `String` here would discard wrapper provenance that public
/// projection must keep. Template-valued constants are deliberately not
/// accepted: a slot default is a String, not a wrapper template.
fn constant_string_value_id(
    expression: ExpressionView<'_>,
    const_values: &ConstValueStore,
) -> Option<ConstValueId> {
    match expression.operation() {
        ExpressionOperationView::Constant(value) => {
            let mut resolved = value;
            loop {
                match const_values.payload(resolved)? {
                    ConstValuePayload::String(_) => return Some(value),
                    ConstValuePayload::Coerced(inner) => resolved = *inner,
                    _ => return None,
                }
            }
        }
        ExpressionOperationView::Coerce { value, .. } => {
            constant_string_value_id(value, const_values)
        }
        _ => None,
    }
}

/// Reads one authored positive whole-number slot position.
fn positive_slot_position(
    directive_name: StringId,
    token_stream: &AstCursor<'_>,
    string_table: &mut StringTable,
) -> CoreDirectiveResult<usize> {
    let literal = token_stream
        .current_numeric_literal_in(string_table)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "numeric slot target token had no payload",
            )
        })?;

    let is_positive_whole_number = literal.sign == NumericLiteralSign::Positive
        && literal.kind == NumericLiteralKind::WholeNumber
        && literal.fractional_digit_count == 0
        && literal.exponent_digit_count == 0;
    let position = is_positive_whole_number
        .then(|| {
            string_table
                .resolve(literal.normalized_text)
                .parse::<usize>()
                .ok()
        })
        .flatten()
        .filter(|position| *position > 0);
    match position {
        Some(position) => Ok(position),
        None => Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::InvalidSlotTarget,
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

/// Applies the slot body-colon rule: `$slot` declarations never open a body.
fn reject_slot_body(token_stream: &AstCursor<'_>) -> CoreDirectiveResult<()> {
    if token_stream.current_tag() == TokenTag::START_TEMPLATE_BODY {
        return Err(templates::invalid_template_structure(
            InvalidTemplateStructureReason::SlotInHead,
            Some(token_stream.current_span()),
        )
        .into());
    }
    Ok(())
}

/// Tokens that end a slot declaration without starting another head item.
fn slot_declaration_ends_at(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::TEMPLATE_CLOSE
            | TokenTag::START_TEMPLATE_BODY
            | TokenTag::COMMA
            | TokenTag::EOF
            | TokenTag::ELSE
            | TokenTag::END
            | TokenTag::CLOSE_CURLY
    )
}

pub(super) fn parse_core_style_directive(
    token_stream: &mut AstCursor,
    build_state: &mut TemplateBuildState,
    directive_name: &str,
    kind: CoreStyleDirectiveKind,
    string_table: &mut StringTable,
) -> CoreDirectiveResult<()> {
    // The registry owns the name; reuse its interned id instead of re-interning literals.
    let directive_name_id = string_table.intern(directive_name);
    match kind {
        CoreStyleDirectiveKind::Raw => {
            reject_directive_operand(directive_name_id, token_stream)?;
            configure_raw_style(build_state);
        }

        CoreStyleDirectiveKind::Fresh => {
            reject_directive_operand(directive_name_id, token_stream)?;
            // `$fresh` opts this template out of parent-applied direct-child
            // wrappers while still allowing local directives/wrappers in the same head.
            build_state.style.skip_parent_child_wrappers = true;
        }

        CoreStyleDirectiveKind::Note => {
            reject_directive_operand(directive_name_id, token_stream)?;
            build_state.kind = TemplateType::Comment;
            build_state.style = Style::default();
        }

        CoreStyleDirectiveKind::Slot
        | CoreStyleDirectiveKind::In
        | CoreStyleDirectiveKind::Before
        | CoreStyleDirectiveKind::After => {
            return Err(TemplateError::from(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Core style directive '{directive_name}' reached generic style parsing but should have been handled by structural helper dispatch."
                ),
            )));
        }
    }

    Ok(())
}

pub(super) fn mark_template_body_whitespace_style_controlled(build_state: &mut TemplateBuildState) {
    build_state.style.body_whitespace_policy = BodyWhitespacePolicy::StyleDirectiveControlled;
}
