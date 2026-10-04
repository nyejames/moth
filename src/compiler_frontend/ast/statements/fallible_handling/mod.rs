//! Explicit fallible propagation and complete-expression recovery.
//!
//! Expression completion selects one receiving-boundary catch after ordinary reduction.
//! This owner resolves producer compatibility, types the optional binding and validates handler
//! success slots or termination. Call finishers consume only postfix propagation; neither
//! implicit failure facts nor handlers create a source-level carrier type.

mod catch_handler;
mod parser;
mod success_types;
mod validation;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

// --------------------------
//  Re-exports
// --------------------------

pub(crate) use parser::{
    CastCatchSite, FallibleCallSite, FallibleHostCallSite, HandledFallibleCall,
    HandledFallibleHostCall, compatible_expression_error_type, complete_catch_failure_fact,
    fallible_catch_allowed_in_context, parse_cast_catch_handling_suffix,
    parse_completed_expression_catch, parse_fallible_handling_suffix_for_call_expression,
    parse_fallible_handling_suffix_for_expression,
    parse_fallible_handling_suffix_for_host_call_expression, wrap_catch_expression,
};

const FUNCTION_CALL_STAGE: &str = "Function Call Parsing";
const EXPRESSION_STAGE: &str = "Expression Parsing";

/// Returns whether the current token starts explicit typed propagation.
///
/// Recovery is selected only by expression completion; call finishers consume postfix `!`.
pub(crate) fn token_stream_starts_typed_propagation_suffix(token_stream: &AstCursor) -> bool {
    token_stream.current_tag() == TokenTag::BANG
        || (token_stream.current_tag() == TokenTag::SYMBOL
            && token_stream.peek_next_tag() == Some(TokenTag::BANG))
}
/// Selects the precise reason for applying `!` or `catch` to a non-fallible operand.
///
/// WHAT: maps the authored handler (`!` vs `catch`) and whether the operand carries an
///       optional value to the matching `InvalidFallibleHandlingReason` case.
/// WHY: the old umbrella `NotResultExpression` reason hardcoded `!` wording and called every
///      carrier a result, so each construction site needs the exact handler and carrier pair.
pub(crate) fn non_fallible_handler_reason(
    handler_tag: TokenTag,
    operand_is_optional: bool,
) -> InvalidFallibleHandlingReason {
    match handler_tag {
        TokenTag::CATCH => {
            if operand_is_optional {
                InvalidFallibleHandlingReason::CatchOnOptional
            } else {
                InvalidFallibleHandlingReason::CatchOnNonFallible
            }
        }
        // `!` (Bang), including the receiver-call `Symbol` + `Bang` spelling.
        _ => {
            if operand_is_optional {
                InvalidFallibleHandlingReason::BangOnOptional
            } else {
                InvalidFallibleHandlingReason::BangOnNonFallible
            }
        }
    }
}

/// Returns true when a call's success return is a single optional slot.
///
/// WHAT: a non-fallible call is only an optional operand when it has exactly one success
///       slot whose type is `Option<_>`; multi-value or void returns are not optional.
/// WHY: the `!`/`catch` handler matrix distinguishes optional operands from plain
///      non-fallible ones, so every call construction site shares one carrier check.
pub(crate) fn call_success_is_optional(
    success_type_ids: &[TypeId],
    type_environment: &TypeEnvironment,
) -> bool {
    matches!(success_type_ids, [single] if type_environment.is_option(*single))
}

#[cfg(test)]
#[path = "../tests/fallible_handling_tests.rs"]
mod fallible_handling_tests;
