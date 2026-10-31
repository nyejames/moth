//! Control-flow and pattern diagnostic text renderers.
//!
//! WHAT: renders diagnostics for statement control flow, match patterns, and exhaustiveness.
//! WHY: these payloads are emitted by statement parsing and match validation, separate from
//! declaration/type/call validation.

use crate::compiler_frontend::compiler_messages::{
    InvalidControlFlowStatementReason, InvalidMatchGuardReason, InvalidMatchPatternReason,
    NonExhaustiveMatchReason,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};

use super::{
    DiagnosticRenderContext, RenderedPayload, closest_name_suggestion,
    format_available_variants_hint, named_value_or_default,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidControlFlowStatementView, InvalidFallibleHandlingCode, InvalidFallibleHandlingView,
    InvalidMatchGuardView, InvalidMatchPatternView, NonExhaustiveMatchView,
    UnreachableMatchArmView, invalid_match_pattern_capture_counts,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;

/// The fixed message for an unreachable match arm.
pub(crate) const UNREACHABLE_MATCH_ARM_MESSAGE: &str = "Unreachable match arm";

pub(crate) fn invalid_control_flow_statement_message(
    reason: InvalidControlFlowStatementReason,
) -> String {
    match reason {
        InvalidControlFlowStatementReason::ElseOutsideIfOrMatch => {
            "Unexpected use of 'else' keyword. It can only be used inside an if statement or match statement.".to_string()
        }
        InvalidControlFlowStatementReason::ElseIfUnsupported => {
            "Ordinary if statements do not support 'else if'. Use a standalone 'else' body with a nested if statement when another condition is needed.".to_string()
        }
        InvalidControlFlowStatementReason::BreakOutsideLoop => {
            "Break statements can only be used inside loops.".to_string()
        }
        InvalidControlFlowStatementReason::ContinueOutsideLoop => {
            "Continue statements can only be used inside loops.".to_string()
        }
        InvalidControlFlowStatementReason::ReturnOutsideFunction => {
            "Return statements can only be used inside functions.".to_string()
        }
        InvalidControlFlowStatementReason::ReturnBangOutsideErrorFunction => {
            "return! can only be used inside functions that declare an error return slot.".to_string()
        }
        InvalidControlFlowStatementReason::ExpectedColonAfterCondition => {
            "Expected ':' after the condition to open a new scope.".to_string()
        }
        InvalidControlFlowStatementReason::ExpectedConditionAfterIf => {
            "Expected a condition after 'if'.".to_string()
        }
        InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch => {
            "Unexpected end of file in match statement.".to_string()
        }
        InvalidControlFlowStatementReason::CaseRequiredBeforeElse => {
            "Match statements require at least one pattern arm before 'else =>'.".to_string()
        }
        InvalidControlFlowStatementReason::DuplicateElseArm => {
            "Match statement can only have one 'else =>' arm.".to_string()
        }
        InvalidControlFlowStatementReason::ExpectedFatArrow => {
            "Expected '=>' in match arm.".to_string()
        }
        InvalidControlFlowStatementReason::InlineValueIfMultiline => {
            "Inline value-producing 'if' must fit on a single logical line.".to_string()
        }
        InvalidControlFlowStatementReason::InlineValueIfElseThen => {
            "Inline value-producing 'if' cannot use 'else then'.".to_string()
        }
        InvalidControlFlowStatementReason::ValueIfMissingElse => {
            "Value-producing 'if' requires an 'else' branch.".to_string()
        }
        InvalidControlFlowStatementReason::ValueIfBranchFallsThrough => {
            "Every reachable branch of a value-producing 'if' must produce a value or terminate.".to_string()
        }
        InvalidControlFlowStatementReason::ValueIfNoProducingPath => {
            "Value-producing 'if' has no reachable path that produces a value.".to_string()
        }
        InvalidControlFlowStatementReason::ValueBlockOutsideReceiver => {
            "Value-producing blocks are only valid at declaration, assignment, return, multi-bind, or catch receiving sites.".to_string()
        }
        InvalidControlFlowStatementReason::ValueIfOptionNonePredicate => {
            "Optional `none` checks are statement-only here. Use `if option is |value| ... else ...` for value recovery.".to_string()
        }
        InvalidControlFlowStatementReason::ValueIfOptionLiteralPredicate => {
            "Inline value-producing optional checks must use `|value|`; literal option matching belongs in full `if option is:` matches.".to_string()
        }
        InvalidControlFlowStatementReason::ExpectedValueAfterThen => {
            "Expected a value after 'then'.".to_string()
        }
        InvalidControlFlowStatementReason::ExpectedValueAfterElse => {
            "Expected a value after 'else'.".to_string()
        }
    }
}

/// WHAT: renders an invalid control-flow statement from its compact reason code.
/// WHY: the compact view carries the reason `invalid_control_flow_statement_message` consumes.
pub(crate) fn render_invalid_control_flow_statement(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidControlFlowStatementView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_control_flow_statement_message(facts.reason),
        guidance: Vec::new(),
    }
}

fn invalid_fallible_handling_message(
    reason: InvalidFallibleHandlingCode,
    first_error_type: Option<&str>,
    second_error_type: Option<&str>,
    count: Option<usize>,
) -> String {
    match reason {
        InvalidFallibleHandlingCode::EntryHasIncompatibleError => {
            let produced_error =
                first_error_type.expect("entry error reason stores its produced type");
            let entry_error =
                second_error_type.expect("entry error reason stores its boundary type");
            format!(
                "This `!` propagation produces `{produced_error}!`, which is incompatible with the entry's built-in `start` `{entry_error}!` boundary. Handle `{produced_error}` locally with `catch` or recover explicitly."
            )
        }
        InvalidFallibleHandlingCode::FunctionHasMultipleSuccessReturns => {
            let count = count.expect("multiple-success-return reason stores its count");
            format!(
                "This `?` propagation is inside a function with {count} authored success return slots, but requires exactly one optional success return slot. Inspect the option locally or use a single optional success return slot."
            )
        }
        InvalidFallibleHandlingCode::IncompatibleCatchErrorTypes => {
            let first_error =
                first_error_type.expect("incompatible catch reason stores its first error type");
            let second_error =
                second_error_type.expect("incompatible catch reason stores its second error type");
            format!(
                "This catch protects incompatible typed error producers: `{first_error}!` and `{second_error}!`. Split the work into separately handled expressions or convert the errors explicitly."
            )
        }
        InvalidFallibleHandlingCode::CustomErrorMixedWithImplicitFailure => {
            let error_type =
                first_error_type.expect("mixed catch reason stores its typed error type");
            format!(
                "This catch mixes a `{error_type}!` producer with implicit built-in failure (`Error`). Split the work into separately handled expressions or convert the errors explicitly."
            )
        }
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction => {
            let error_type =
                first_error_type.expect("custom error function reason stores its error type");
            let mut message = format!(
                "Implicit built-in failure cannot escape through the custom `{error_type}!` slot. Recover locally with `catch` or convert the failure explicitly to `{error_type}` with `catch` and `return!`. Alternatively, change the final error slot to `Error!`. Use an assertion only if unrecoverable failure is an explicit invariant choice."
            );
            append_elided_builtin_failure_hops(
                &mut message,
                count.expect("builtin failure reason stores its elided-hop count"),
            );
            message
        }
        InvalidFallibleHandlingCode::UnhandledBuiltinFailureInExportedFunction => {
            let mut message = "This exported function has no error slot but implicit built-in failure may escape. Recover locally with catch or declare a final Error! return slot.".to_owned();
            message.push_str(
                " Use an assertion only if unrecoverable failure is an explicit invariant choice.",
            );
            append_elided_builtin_failure_hops(
                &mut message,
                count.expect("builtin failure reason stores its elided-hop count"),
            );
            message
        }
        _ => reason
            .unit_reason()
            .expect("unit fallible code has its matching unit reason")
            .message()
            .to_owned(),
    }
}

/// WHAT: renders fallible-handling facts and type data from their compact row.
/// WHY: the code selects which optional types and count feed `invalid_fallible_handling_message`.
pub(crate) fn render_invalid_fallible_handling(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidFallibleHandlingView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let first_error_type = facts.first_error_type.map(|type_id| {
        diagnostic
            .types()
            .display_for_render(type_id, context.string_table, context.path_table())
    });
    let second_error_type = facts.second_error_type.map(|type_id| {
        diagnostic
            .types()
            .display_for_render(type_id, context.string_table, context.path_table())
    });
    RenderedPayload {
        message: invalid_fallible_handling_message(
            facts.reason,
            first_error_type.as_deref(),
            second_error_type.as_deref(),
            facts.count,
        ),
        guidance: Vec::new(),
    }
}

fn append_elided_builtin_failure_hops(message: &mut String, elided_call_hops: usize) {
    if elided_call_hops != 0 {
        use std::fmt::Write;

        let _ = write!(
            message,
            " Witness omits {elided_call_hops} additional private call hop(s)."
        );
    }
}

pub(crate) fn invalid_match_pattern_message(
    reason: InvalidMatchPatternReason,
    variant_name: Option<StringId>,
    scrutinee_name: Option<StringId>,
    capture_counts: Option<(usize, usize)>,
    names: &[StringId],
    string_table: &dyn StringTableResolver,
) -> String {
    let variant_text = named_value_or_default(variant_name, string_table, "this variant");

    match reason {
        InvalidMatchPatternReason::WildcardNotSupported => {
            "Wildcard pattern '_' is not supported in Moth. Use 'else =>' for a catch-all arm.".to_string()
        }
        InvalidMatchPatternReason::AsNotValid => {
            "`as` is not valid in match patterns. It is only supported in choice payload captures.".to_string()
        }
        InvalidMatchPatternReason::NegativeLiteralNotNumeric => {
            "Negative literal patterns must be numeric literals, for example '-1' or '-3.2'.".to_string()
        }
        InvalidMatchPatternReason::LiteralTypeUnsupported => {
            "Literal match patterns currently support only literal int, float, bool, char, and string values.".to_string()
        }
        InvalidMatchPatternReason::ScrutineeTypeUnsupportedForRelational => {
            "Relational match patterns are only supported for ordered scalar types: integer, binary-float and Dec types, Byte and Char.".to_string()
        }
        InvalidMatchPatternReason::UnitVariantHasPayload => {
            format!("Unit variant {variant_text} cannot have payload captures. Use '<variant> =>' without parentheses.")
        }
        InvalidMatchPatternReason::PayloadVariantNeedsBindings => {
            format!("Payload variant {variant_text} requires capture bindings. Expected '{variant_text}(...) =>'.")
        }
        InvalidMatchPatternReason::CaptureBindingMustBeFieldName => {
            "Capture binding must be a field name.".to_string()
        }
        InvalidMatchPatternReason::ExpectedLocalBindingAfterAs => {
            "Expected local binding name after `as` in choice payload pattern.".to_string()
        }
        InvalidMatchPatternReason::AliasMustBeLocalBinding => {
            "Choice payload alias must be a local binding name.".to_string()
        }
        InvalidMatchPatternReason::DuplicateCaptureBinding => {
            format!("Duplicate capture binding in pattern for variant {variant_text}.")
        }
        InvalidMatchPatternReason::TooManyCaptureBindings
        | InvalidMatchPatternReason::TooFewCaptureBindings => capture_count_message(
            &variant_text,
            capture_counts.unwrap_or_default(),
            names,
            string_table,
        ),
        InvalidMatchPatternReason::CaptureBindingNameMismatch => capture_name_mismatch_message(
            &variant_text,
            capture_counts.unwrap_or_default(),
            names,
            string_table,
        ),
        InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee => {
            qualifier_mismatch_message(variant_name, scrutinee_name, string_table)
        }
        InvalidMatchPatternReason::ExpectedVariantNameAfterQualifier => {
            "Expected a variant name after '::' in this match pattern.".to_string()
        }
        InvalidMatchPatternReason::MustUseVariantNamesNotLiterals => {
            "Choice match arms must use variant names, not raw literals.".to_string()
        }
        InvalidMatchPatternReason::MustStartWithVariantName => {
            "Choice match arms must start with a declared variant name.".to_string()
        }
        InvalidMatchPatternReason::UnknownVariant => {
            unknown_variant_message(variant_name, scrutinee_name, names, string_table)
        }
        InvalidMatchPatternReason::CaptureBindingShadowsVariable => {
            "Capture binding shadows an existing variable. Moth does not allow shadowing.".to_string()
        }
        InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee => {
            "`none =>` is only valid when matching an optional value.".to_string()
        }
        InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee => {
            "A bare capture name is not allowed on an optional scrutinee. Use `|name|` to capture the present value.".to_string()
        }
        InvalidMatchPatternReason::OptionalPresentCaptureOnNonOptional => {
            "`|name|` capture is only valid when matching an optional value.".to_string()
        }
        InvalidMatchPatternReason::EmptyOptionalPresentCapture => {
            "An optional-present capture cannot be empty. Use `|name|` to capture the present value."
                .to_string()
        }
        InvalidMatchPatternReason::OptionalPresentCaptureTypeAnnotation => {
            "Type annotations are not allowed inside `|...|`.".to_string()
        }
        InvalidMatchPatternReason::MissingClosingPipe => {
            "Expected `|` to close the optional-present capture.".to_string()
        }
        InvalidMatchPatternReason::ExpectedBindingInOptionalPresentCapture => {
            "Expected a binding name inside `|...|`.".to_string()
        }
    }
}

/// WHAT: describes a payload capture count that does not match the choice variant.
/// WHY: listing the declared field names shows which captures the variant takes.
fn capture_count_message(
    variant_text: &str,
    counts: (usize, usize),
    names: &[StringId],
    string_table: &dyn StringTableResolver,
) -> String {
    let (expected, provided) = counts;
    let field_word = if expected == 1 { "field" } else { "fields" };
    let expected_names = names.get(..expected).unwrap_or_default();
    if expected_names.is_empty() {
        return format!(
            "Variant {variant_text} has {expected} payload {field_word}, but this pattern captures {provided}."
        );
    }
    let list = quoted_name_list(expected_names, string_table);
    format!(
        "Variant {variant_text} has {expected} payload {field_word}, {list}, but this pattern captures {provided}."
    )
}

/// WHAT: describes a payload capture whose name differs from the field at its position.
/// WHY: naming the expected field and the alias form shows both ways to fix the arm. The
/// names list holds the declared fields, then the authored capture, then its `as` binding
/// when one was written, so the alias suggestion keeps the binding the arm body uses.
fn capture_name_mismatch_message(
    variant_text: &str,
    counts: (usize, usize),
    names: &[StringId],
    string_table: &dyn StringTableResolver,
) -> String {
    let (expected_count, provided) = counts;
    let (Some(expected), Some(authored)) = (
        names.get(provided.saturating_sub(1)).copied(),
        names.get(expected_count).copied(),
    ) else {
        return format!("Capture does not match a payload field in variant {variant_text}.");
    };
    let binding = names.get(expected_count + 1).copied().unwrap_or(authored);
    let expected = string_table.resolve(expected);
    let authored = string_table.resolve(authored);
    let binding = string_table.resolve(binding);
    format!(
        "Capture '{authored}' does not match payload field '{expected}' in variant {variant_text}. Use '{expected}' in this position, or write '{expected} as {binding}' to bind it under a different name."
    )
}

/// WHAT: explains a match arm qualifier that names a different choice than the scrutinee.
/// WHY: naming both choices shows which variant set the arm should draw from.
fn qualifier_mismatch_message(
    qualifier: Option<StringId>,
    scrutinee: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    match (qualifier, scrutinee) {
        (Some(qualifier), Some(scrutinee)) => {
            let qualifier = string_table.resolve(qualifier);
            let scrutinee = string_table.resolve(scrutinee);
            format!(
                "Match arm uses qualifier '{qualifier}', but the scrutinee is '{scrutinee}'. Use a '{scrutinee}' variant or omit the qualifier."
            )
        }
        _ => "Match arm qualifier does not match the scrutinee choice.".to_string(),
    }
}

/// WHAT: explains a match arm variant that the scrutinee choice does not declare.
/// WHY: a close spelling and the declared variant list usually point straight at the fix.
fn unknown_variant_message(
    variant_name: Option<StringId>,
    scrutinee_name: Option<StringId>,
    available_variants: &[StringId],
    string_table: &dyn StringTableResolver,
) -> String {
    let Some(variant_name) = variant_name else {
        return "Unknown variant.".to_string();
    };
    let variant_spelling = string_table.resolve(variant_name);
    let variant_text = quoted_variant_text(variant_spelling, scrutinee_name, string_table);
    let did_you_mean = closest_name_suggestion(variant_spelling, available_variants, string_table)
        .map(|suggested| {
            let suggested = quoted_variant_text(&suggested, scrutinee_name, string_table);
            format!(" Did you mean {suggested}?")
        })
        .unwrap_or_default();
    let available = format_available_variants_hint(available_variants, string_table);
    format!("Unknown variant {variant_text}.{did_you_mean}{available}")
}

/// WHAT: quotes a variant name, qualified by its scrutinee choice when that choice is known.
/// WHY: qualified spellings match what authors write in match arms.
fn quoted_variant_text(
    variant: &str,
    scrutinee: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    match scrutinee {
        Some(choice) => format!("'{}::{variant}'", string_table.resolve(choice)),
        None => format!("'{variant}'"),
    }
}

/// WHAT: joins quoted names into an English list such as `'a', 'b' and 'c'`.
/// WHY: capture diagnostics name every expected field in declaration order.
fn quoted_name_list(names: &[StringId], string_table: &dyn StringTableResolver) -> String {
    let quoted = names
        .iter()
        .map(|name| format!("'{}'", string_table.resolve(*name)))
        .collect::<Vec<_>>();
    match quoted.split_last() {
        Some((last, [])) => last.to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// WHAT: renders an invalid match pattern from its reason and the facts its message reads.
/// WHY: capture-count and unknown-variant messages need the packed counts and the declared name list.
pub(crate) fn render_invalid_match_pattern(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidMatchPatternView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_match_pattern_message(
            facts.reason,
            facts.variant_name,
            facts.scrutinee_name,
            invalid_match_pattern_capture_counts(facts),
            diagnostic.string_ids(),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn render_invalid_match_guard(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidMatchGuardView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = match facts.reason {
        InvalidMatchGuardReason::SideEffectingPrelude => {
            "Match guards must be pure Bool expressions and cannot lower to side-effect statements."
        }
    };
    RenderedPayload {
        message: message.to_owned(),
        guidance: Vec::new(),
    }
}

pub(crate) fn non_exhaustive_match_message(
    reason: NonExhaustiveMatchReason,
    missing_variants: &[StringId],
    string_table: &dyn StringTableResolver,
) -> String {
    match reason {
        NonExhaustiveMatchReason::MissingElseArm => {
            "Choice matches with guarded arms must include an explicit 'else =>' arm.".to_string()
        }
        NonExhaustiveMatchReason::MissingVariants => {
            let variants = missing_variants
                .iter()
                .map(|variant| string_table.resolve(*variant).to_string())
                .collect::<Vec<_>>()
                .join(", ");

            format!("Non-exhaustive choice match. Missing variants: [{variants}].")
        }
        NonExhaustiveMatchReason::GuardedArmsRequireElse => {
            "Choice matches with guarded arms must include an explicit 'else =>' arm in Alpha."
                .to_string()
        }
        NonExhaustiveMatchReason::MissingOptionalPatterns => {
            "Non-exhaustive optional-value match. Add `none =>` or `|name| =>` to cover all cases."
                .to_string()
        }
    }
}

/// WHAT: renders an unreachable match arm from its compact record, which carries no facts.
/// WHY: the message is fixed, so the record stores nothing beyond its family.
pub(crate) fn render_unreachable_match_arm(
    _diagnostic: &DiagnosticView<'_>,
    _facts: &UnreachableMatchArmView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: UNREACHABLE_MATCH_ARM_MESSAGE.to_owned(),
        guidance: Vec::new(),
    }
}

/// WHAT: renders a non-exhaustive match from its compact reason and missing variant names.
/// WHY: the record stores the reason and the missing variant names that
/// `non_exhaustive_match_message` uses.
pub(crate) fn render_non_exhaustive_match(
    diagnostic: &DiagnosticView<'_>,
    facts: &NonExhaustiveMatchView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: non_exhaustive_match_message(
            facts.reason,
            diagnostic.string_ids(),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}
