//! Control-flow and pattern diagnostic text renderers.
//!
//! WHAT: renders diagnostics for statement control flow, match patterns, and exhaustiveness.
//! WHY: these payloads are emitted by statement parsing and match validation, separate from
//! declaration/type/call validation.

use crate::compiler_frontend::compiler_messages::{
    InvalidControlFlowStatementReason, InvalidMatchPatternReason, NonExhaustiveMatchReason,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use super::named_value_or_default;

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
            "Value-producing blocks are only valid at declaration, assignment, return, multi-bind, catch, or `then` receiving sites.".to_string()
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

pub(crate) fn invalid_match_pattern_message(
    reason: InvalidMatchPatternReason,
    variant_name: Option<StringId>,
    string_table: &StringTable,
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
            "Relational match patterns are only supported for ordered scalar types: Int, Float, and Char.".to_string()
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
        InvalidMatchPatternReason::TooManyCaptureBindings => {
            format!("Too many capture bindings for variant {variant_text}.")
        }
        InvalidMatchPatternReason::CaptureBindingNameMismatch => {
            format!("Capture binding does not match payload field name in variant {variant_text}.")
        }
        InvalidMatchPatternReason::TooFewCaptureBindings => {
            format!("Too few capture bindings for variant {variant_text}.")
        }
        InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee => {
            "Match arm qualifier does not match the scrutinee choice.".to_string()
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
        InvalidMatchPatternReason::UnknownVariant => format!("Unknown variant {variant_text}."),
        InvalidMatchPatternReason::CaptureBindingShadowsVariable => {
            "Capture binding shadows an existing variable. Moth does not allow shadowing.".to_string()
        }
        InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee => {
            "`none =>` is only valid when matching an optional value.".to_string()
        }
        InvalidMatchPatternReason::OptionValuePatternRequiresEquality => {
            "Option value patterns require the option's inner type to support equality.".to_string()
        }
        InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee => {
            "A bare capture name is not allowed on an optional scrutinee. Use `|name|` to capture the present value.".to_string()
        }
        InvalidMatchPatternReason::OptionPresentCaptureOnNonOptional => {
            "`|name|` capture is only valid when matching an optional value.".to_string()
        }
        InvalidMatchPatternReason::EmptyOptionPresentCapture => {
            "Option present capture cannot be empty. Use `|name|` to capture the present value.".to_string()
        }
        InvalidMatchPatternReason::OptionPresentCaptureTypeAnnotation => {
            "Type annotations are not allowed inside `|...|`.".to_string()
        }
        InvalidMatchPatternReason::MissingClosingPipe => {
            "Expected `|` to close the option present capture.".to_string()
        }
        InvalidMatchPatternReason::ExpectedBindingInOptionPresentCapture => {
            "Expected a binding name inside `|...|`.".to_string()
        }
    }
}

pub(crate) fn non_exhaustive_match_message(
    reason: NonExhaustiveMatchReason,
    missing_variants: &[StringId],
    string_table: &StringTable,
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
        NonExhaustiveMatchReason::MissingOptionPatterns => {
            "Non-exhaustive option match. Add `none =>` or `|name| =>` to cover all cases."
                .to_string()
        }
    }
}
