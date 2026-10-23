//! Deferred-pattern diagnostics.
//!
//! WHAT: shared rejection logic for pattern syntax that is not yet supported.
//! WHY: centralising deferred-pattern checks ensures every parser entry point
//! rejects unsupported lead tokens with identical wording.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Reject match-pattern lead tokens that are unsupported or deferred.
///
/// WHAT: checks the current token against wildcard, negation, capture-tagged, and `as` patterns,
/// and returns a compact draft for each.
/// WHY: every parser entry point that begins a pattern should call this so
/// unsupported syntax is rejected with consistent wording and stable codes.
pub fn reject_deferred_pattern_lead_token(token_stream: &AstCursor) -> Option<DiagnosticDraft> {
    // These forms intentionally fail fast so unsupported syntax never drifts silently.
    match token_stream.current_tag() {
        TokenTag::WILDCARD => {
            return Some(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::WildcardNotSupported,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }

        TokenTag::NOT => {
            return Some(names_declarations::deferred_feature_reason(
                DeferredFeatureReason::NegatedMatchPattern,
                Some(token_stream.current_span()),
            ));
        }

        TokenTag::TYPE_PARAMETER_BRACKET => {
            return Some(names_declarations::deferred_feature_reason(
                DeferredFeatureReason::CaptureTaggedPattern,
                Some(token_stream.current_span()),
            ));
        }

        TokenTag::AS => {
            return Some(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::AsNotValid,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }

        _ => {}
    }

    None
}
