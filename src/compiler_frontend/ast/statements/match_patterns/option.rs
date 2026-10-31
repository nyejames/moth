//! Option pattern parsing.
//!
//! WHAT: parses the `T?` match surface: `none =>` for absence, literal and
//! relational patterns for present values, and `|value|` for present capture.
//! WHY: option matching needs a narrow owner that preserves the compiler-owned
//! carrier model without exposing public `Option` constructors. A retained optional
//! subject shares the same capture and value syntax, and its arm is finished once the
//! body root selects the subject's inner type.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, DeferredExpressionCompletion, PendingOptionPattern,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, MatchSubject, ensure_pattern_compatible_with_subject, finish_retained_pattern,
    parse_non_choice_pattern,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidMatchPatternReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;

/// Result for the option pattern family.
///
/// Option parsing keeps diagnosed failures on the compact draft lane; infrastructure failures
/// remain typed.
type OptionPatternResult<T> =
    Result<T, crate::compiler_frontend::ast::expressions::error::ExpressionParseError>;

/// The authored parts of one `|name|` present capture, before its binding path is declared.
///
/// WHAT: the arm-local name, the span of the opening pipe and the span of the name token.
/// WHY: a known and a retained optional subject share this syntax; only the binding's type
///      and its declaration differ between them.
#[derive(Debug, Clone)]
pub(crate) struct OptionCaptureBinding {
    pub(crate) name: StringId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) binding_span: Option<SourceSpan>,
}

/// Parse a pattern for an optional scrutinee.
///
/// `none =>` is a presence check and does not require equality support from
/// the inner type. `|name|` matches any present value and binds the inner payload.
/// Literal present-value patterns compare the option payload for equality, so the
/// inner type must support runtime equality. Relational present-value patterns are
/// forwarded without additional validation.
pub fn parse_option_pattern(
    token_stream: &mut AstCursor,
    option_inner_type_id: TypeId,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> OptionPatternResult<MatchPattern> {
    if token_stream.current_tag() == TokenTag::NONE_LITERAL {
        let span = Some(token_stream.current_span());
        token_stream.advance();
        return Ok(MatchPattern::OptionNone { span });
    }

    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        return parse_option_present_capture(token_stream, option_inner_type_id, string_table);
    }

    // Option present-value checks reuse the non-choice parser's plain
    // diagnostic boundary without an intermediate representation. The inner type is the exact
    // known subject for this path, so the parser returns a ready pattern.
    let pattern = parse_non_choice_pattern(
        token_stream,
        MatchSubject::Known(option_inner_type_id),
        string_table,
        type_environment,
        expression_builder,
        const_values,
    )?;

    match pattern {
        DeferredArmPattern::Ready(pattern) => {
            wrap_option_present_value(pattern, option_inner_type_id, type_environment)
        }

        // A known-subject non-choice parse only returns a ready literal or relational pattern,
        // so any other variant is an invariant violation rather than user input.
        _ => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "parse_non_choice_pattern returned unexpected pattern variant for option",
        )
        .into()),
    }
}

/// Parse `|name|` present capture for a known optional scrutinee.
fn parse_option_present_capture(
    token_stream: &mut AstCursor,
    inner_type_id: TypeId,
    string_table: &mut StringTable,
) -> OptionPatternResult<MatchPattern> {
    let binding = parse_option_capture_binding(token_stream, string_table)?;

    // Binding path is filled in by the caller (branching.rs) once the arm scope is known.
    Ok(MatchPattern::OptionPresentCapture {
        name: binding.name,
        binding_path: PathId::ROOT,
        inner_type_id,
        span: binding.span,
        binding_span: binding.binding_span,
    })
}

/// Parse the authored `|name|` capture shared by known and retained optional subjects.
///
/// Validates:
/// - `||` is rejected.
/// - Multiple names are rejected.
/// - Type annotations inside `|...|` are rejected.
pub(crate) fn parse_option_capture_binding(
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
) -> OptionPatternResult<OptionCaptureBinding> {
    let span = Some(token_stream.current_span());
    token_stream.advance(); // consume opening '|'
    token_stream.skip_newlines();

    let name = match token_stream.current_tag() {
        TokenTag::TYPE_PARAMETER_BRACKET => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::EmptyOptionalPresentCapture,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
        TokenTag::SYMBOL => token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "option capture symbol is missing its payload",
                )
            })?,
        _ => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::ExpectedBindingInOptionalPresentCapture,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };
    let binding_span = Some(token_stream.current_span());
    token_stream.advance();

    // Reject type annotations such as `|name String|`.
    if token_stream.current_tag() == TokenTag::SYMBOL {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::OptionalPresentCaptureTypeAnnotation,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.skip_newlines();

    if token_stream.current_tag() != TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::MissingClosingPipe,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }
    token_stream.advance(); // consume closing '|'

    Ok(OptionCaptureBinding {
        name,
        span,
        binding_span,
    })
}

/// Wrap a ready non-choice pattern as an option present-value arm.
///
/// WHAT: a literal becomes an `OptionValue`; a relational pattern is forwarded unchanged.
/// WHY: literal patterns are scalar literals whose type must match the inner type, and every
///      scalar supports runtime equality, so a failing equality check is a compiler invariant.
/// MUST NOT: accept any other variant; the non-choice parser returns only these two for options.
fn wrap_option_present_value(
    pattern: MatchPattern,
    inner_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> OptionPatternResult<MatchPattern> {
    match pattern {
        MatchPattern::Literal(value) => {
            if !type_environment.supports_runtime_equality(inner_type_id) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "optional literal pattern reached an inner type without runtime equality",
                )
                .into());
            }

            let span = value.span;
            Ok(MatchPattern::OptionValue { value, span })
        }

        relational @ MatchPattern::Relational { .. } => Ok(relational),

        _ => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "option present-value pattern is neither a literal nor a relational pattern",
        )
        .into()),
    }
}

/// Finish a pending optional arm once the subject's concrete optional type is selected.
///
/// WHAT: a capture becomes an `OptionPresentCapture` over the selected inner type; a value
///       becomes an `OptionValue` or a forwarded relational pattern after the subject check.
/// WHY: a retained optional's inner shape is known at parse time, but the concrete types and
///      the literal's equality check wait for the body root.
pub(in crate::compiler_frontend::ast::statements) fn finish_pending_option_pattern<'env>(
    pending: PendingOptionPattern,
    subject_type_id: TypeId,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> OptionPatternResult<MatchPattern> {
    let inner_type_id = {
        if let Some(inner) = completion
            .session_services()
            .type_interner
            .environment()
            .option_inner_type(subject_type_id)
        {
            inner
        } else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "pending option arm finished against a non-option subject",
            )
            .into());
        }
    };

    match pending {
        PendingOptionPattern::PresentCapture(capture) => Ok(MatchPattern::OptionPresentCapture {
            name: capture.name,
            binding_path: capture.binding_path,
            inner_type_id,
            span: capture.span,
            binding_span: capture.binding_span,
        }),

        PendingOptionPattern::PresentValue(pattern) => {
            let environment = completion.session_services().type_interner.environment();
            ensure_pattern_compatible_with_subject(&pattern, subject_type_id, environment)?;
            wrap_option_present_value(pattern, inner_type_id, environment)
        }

        PendingOptionPattern::RetainedValue(retained) => {
            let value = completion.resolve_fragment(
                retained.value.items,
                ExpectedType::NumericLiteral(inner_type_id),
                &retained.value.value_mode,
            )?;
            let pattern = {
                let services = completion.session_services();
                finish_retained_pattern(
                    retained.relational,
                    services.expression_builder.view_owned(value)?,
                )?
            };
            let environment = completion.session_services().type_interner.environment();
            wrap_option_present_value(pattern, inner_type_id, environment)
        }
    }
}
