//! Choice-variant pattern parsing.
//!
//! WHAT: parses `Variant =>` and `Variant(field) =>` patterns, resolving
//! variant names against the scrutinee choice declaration.
//! WHY: choice patterns have unique syntax (qualified names, payload captures)
//! and validation (exact field names, no reordering) that justify a dedicated file.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::deferred_feature_diagnostics::deferred_feature_reason_diagnostic;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

use rustc_hash::FxHashMap;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::types::ParsedChoicePattern;
use super::types::ParsedChoicePayloadCapture;

/// Boxed diagnostic result for all choice-pattern parsing functions.
///
/// WHAT: every function in this module returns errors as `Box<CompilerDiagnostic>`.
/// WHY: `CompilerDiagnostic` is large enough to trigger `clippy::result_large_err`;
/// boxing the error variant keeps the success path cheap and matches the
/// already-boxed `MatchHeaderResult` convention used by the surrounding
/// match-header parser.
type ChoicePatternResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Resolve a choice variant pattern to its deterministic variant index.
///
/// WHAT: accepts bare (`Ready`) or qualified (`Status::Ready`) variant names and
/// resolves them against the scrutinee choice metadata.
/// WHY: later lowering uses the stable variant index in `HirPattern::ChoiceVariant`,
/// while payload captures are materialized separately at arm entry.
pub fn parse_choice_variant_pattern(
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    choice_nominal_path: &InternedPath,
    variants: &[ChoiceVariant],
    string_table: &StringTable,
) -> ChoicePatternResult<ParsedChoicePattern> {
    // Choice patterns support exact variant names plus constructor-like payload captures.
    if let Some(diagnostic) = reject_deferred_pattern_lead_token(token_stream) {
        return Err(Box::new(diagnostic));
    }

    let choice_name_display = choice_display_name(choice_nominal_path, string_table);
    let (variant_name, variant_location) = parse_variant_name(
        token_stream,
        match_context,
        choice_nominal_path,
        &choice_name_display,
        string_table,
    )?;

    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        return Err(Box::new(deferred_feature_reason_diagnostic(
            DeferredFeatureReason::CaptureTaggedPattern,
            token_stream.current_location(),
        )));
    }

    let variant_index = resolve_variant_to_tag(
        variants,
        variant_name,
        &choice_name_display,
        &variant_location,
        string_table,
        choice_nominal_path,
    )?;

    let variant = &variants[variant_index];
    let captures =
        parse_choice_pattern_captures(token_stream, variant, &choice_name_display, string_table)?;

    Ok(ParsedChoicePattern {
        nominal_path: choice_nominal_path.to_owned(),
        variant: variant_name,
        tag: variant_index,
        captures,
        location: variant_location,
    })
}

/// Parse optional payload captures after a choice-variant name.
///
/// WHAT: handles `Err(message) =>` and `Success =>` forms, validating that
/// captures match the variant's payload metadata exactly.
/// WHY: separating capture parsing from name resolution keeps each function focused
/// and makes error messages specific to the payload layer.
fn parse_choice_pattern_captures(
    token_stream: &mut FileTokens,
    variant: &ChoiceVariant,
    _choice_name_display: &str,
    string_table: &StringTable,
) -> ChoicePatternResult<Vec<ParsedChoicePayloadCapture>> {
    match &variant.payload {
        ChoiceVariantPayload::Unit => {
            if token_stream.current_token_kind() == &TokenKind::OpenParenthesis {
                return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::UnitVariantHasPayload,
                    Some(variant.id),
                    None,
                    token_stream.current_location(),
                )));
            }

            Ok(Vec::new())
        }

        ChoiceVariantPayload::Record { fields } => {
            if token_stream.current_token_kind() != &TokenKind::OpenParenthesis {
                return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::PayloadVariantNeedsBindings,
                    Some(variant.id),
                    None,
                    token_stream.current_location(),
                )));
            }

            token_stream.advance();

            let mut captures = Vec::new();
            let mut seen_names: FxHashMap<StringId, SourceLocation> = FxHashMap::default();

            loop {
                token_stream.skip_newlines();

                if token_stream.current_token_kind() == &TokenKind::CloseParenthesis {
                    token_stream.advance();
                    break;
                }

                let capture_location = token_stream.current_location();

                // Wildcards are not yet supported in choice payload position.
                if token_stream.current_token_kind() == &TokenKind::Wildcard {
                    return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::WildcardNotSupported,
                        None,
                        None,
                        capture_location,
                    )));
                }

                let field_name = match token_stream.current_token_kind() {
                    TokenKind::Symbol(name) => *name,
                    _ => {
                        return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                            InvalidMatchPatternReason::CaptureBindingMustBeFieldName,
                            None,
                            None,
                            capture_location,
                        )));
                    }
                };
                token_stream.advance();

                let mut binding_name = field_name;
                let mut binding_location = capture_location.clone();

                // Parse optional `as <local_binding>` rename syntax.
                if token_stream.current_token_kind() == &TokenKind::As {
                    token_stream.advance();
                    binding_location = token_stream.current_location();

                    let after_as_token = token_stream.current_token_kind().to_owned();
                    binding_name = match after_as_token {
                        TokenKind::Symbol(name) => {
                            token_stream.advance();
                            name
                        }
                        TokenKind::End
                        | TokenKind::Eof
                        | TokenKind::CloseParenthesis
                        | TokenKind::Comma => {
                            return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                                InvalidMatchPatternReason::ExpectedLocalBindingAfterAs,
                                None,
                                None,
                                binding_location,
                            )));
                        }
                        _ => {
                            return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                                InvalidMatchPatternReason::AliasMustBeLocalBinding,
                                None,
                                None,
                                binding_location,
                            )));
                        }
                    };
                }

                // Reject named assignment: `Err(message = text) =>`
                if token_stream.current_token_kind() == &TokenKind::Assign {
                    return Err(Box::new(deferred_feature_reason_diagnostic(
                        DeferredFeatureReason::NamedPayloadPatternAssignment,
                        token_stream.current_location(),
                    )));
                }

                // Check duplicate capture binding name (uses the local alias when present).
                if seen_names.contains_key(&binding_name) {
                    return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::DuplicateCaptureBinding,
                        Some(variant.id),
                        None,
                        binding_location,
                    )));
                }
                seen_names.insert(binding_name, binding_location.clone());

                // Validate capture position and name against declaration metadata.
                let field_index = captures.len();
                let Some(field_decl) = fields.get(field_index) else {
                    return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::TooManyCaptureBindings,
                        Some(variant.id),
                        None,
                        capture_location,
                    )));
                };

                let expected_field_name = choice_payload_field_name(field_decl, string_table)?;
                if field_name != expected_field_name {
                    return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::CaptureBindingNameMismatch,
                        Some(variant.id),
                        None,
                        capture_location,
                    )));
                }

                captures.push(ParsedChoicePayloadCapture {
                    binding_name,
                    field_index,
                    type_id: field_decl.value.type_id,
                    location: capture_location,
                    binding_location,
                });

                // Advance past the separator or detect the end of the capture list.
                token_stream.skip_newlines();
                match token_stream.current_token_kind() {
                    TokenKind::Comma => {
                        token_stream.advance();
                        continue;
                    }
                    TokenKind::CloseParenthesis => {
                        token_stream.advance();
                        break;
                    }
                    TokenKind::OpenParenthesis => {
                        return Err(Box::new(deferred_feature_reason_diagnostic(
                            DeferredFeatureReason::NestedPayloadPattern,
                            token_stream.current_location(),
                        )));
                    }
                    _ => {
                        return Err(Box::new(CompilerDiagnostic::expected_token(
                            TokenKind::Comma,
                            Some(token_stream.current_token_kind().clone()),
                            token_stream.current_location(),
                        )));
                    }
                }
            }

            // Check for too few captures.
            if captures.len() != fields.len() {
                return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::TooFewCaptureBindings,
                    Some(variant.id),
                    None,
                    variant.location.clone(),
                )));
            }

            Ok(captures)
        }
    }
}

/// Resolve the leaf name of a choice payload field declaration.
///
/// WHAT: extracts the terminal identifier from a payload field's interned path.
/// WHY: payload fields are declarations with interned paths; during match-pattern
///      parsing we need the leaf string ID for capture-name validation.
///      This is an internal invariant: every payload field must have a leaf name.
fn choice_payload_field_name(
    field: &Declaration,
    string_table: &StringTable,
) -> ChoicePatternResult<StringId> {
    field.id.name().ok_or_else(|| {
        let missing_field_name_error = CompilerError::new(
            format!(
                "Choice payload field '{}' has no leaf name during match-pattern parsing",
                field.id.to_string(string_table)
            ),
            field.value.location.clone(),
            ErrorType::Compiler,
        );

        Box::new(missing_field_name_error.into())
    })
}

/// Parse a bare (`Ready`) or qualified (`Status::Ready`) variant name from the token stream.
///
/// WHY: separating token-level parsing from tag resolution keeps each function focused
/// and makes error messages specific to the syntactic layer they diagnose.
fn parse_variant_name(
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    choice_nominal_path: &InternedPath,
    _choice_name_display: &str,
    _string_table: &StringTable,
) -> ChoicePatternResult<(StringId, SourceLocation)> {
    let leading_token = token_stream.current_token_kind().to_owned();

    match leading_token {
        TokenKind::Symbol(first_name) => {
            let first_location = token_stream.current_location();
            token_stream.advance();

            if token_stream.current_token_kind() == &TokenKind::DoubleColon {
                if let Some(expected_choice_name) = choice_nominal_path.name()
                    && first_name != expected_choice_name
                    && !qualifier_resolves_to_choice(match_context, first_name, choice_nominal_path)
                {
                    return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee,
                        None,
                        choice_nominal_path.name(),
                        first_location,
                    )));
                }

                token_stream.advance();
                token_stream.skip_newlines();

                match token_stream.current_token_kind().to_owned() {
                    TokenKind::Symbol(qualified_variant_name) => {
                        let qualified_location = token_stream.current_location();
                        token_stream.advance();
                        Ok((qualified_variant_name, qualified_location))
                    }
                    _ => Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::ExpectedVariantNameAfterQualifier,
                        None,
                        None,
                        token_stream.current_location(),
                    ))),
                }
            } else {
                Ok((first_name, first_location))
            }
        }

        // Literal tokens are not valid as choice variant names.
        TokenKind::NumericLiteral(_)
        | TokenKind::BoolLiteral(_)
        | TokenKind::CharLiteral(_)
        | TokenKind::StringSliceLiteral(_)
        | TokenKind::Negative => Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::MustUseVariantNamesNotLiterals,
            None,
            None,
            token_stream.current_location(),
        ))),

        _ => Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::MustStartWithVariantName,
            None,
            None,
            token_stream.current_location(),
        ))),
    }
}

/// Check whether a leading qualifier symbol resolves to the scrutinee choice type.
///
/// WHAT: when the user writes `Status::Ready`, `Status` may be a module dependency namespace or
/// a local alias that points to the same choice declaration as the scrutinee.
/// WHY: this allows qualified variant names even when the qualifier name differs
/// from the choice's declared leaf name, as long as the symbol refers to the same type.
fn qualifier_resolves_to_choice(
    match_context: &ScopeContext,
    qualifier: StringId,
    choice_nominal_path: &InternedPath,
) -> bool {
    match_context
        .get_reference(&qualifier)
        .is_some_and(|declaration| &declaration.id == choice_nominal_path)
}

/// Look up a variant name in the declared choice variant list and return its positional tag.
///
/// WHY: separating semantic resolution from token parsing produces clearer control flow
/// and keeps the error-construction logic for unknown variants in one place.
fn resolve_variant_to_tag(
    variants: &[ChoiceVariant],
    variant_name: StringId,
    _choice_name_display: &str,
    variant_location: &SourceLocation,
    _string_table: &StringTable,
    choice_nominal_path: &InternedPath,
) -> ChoicePatternResult<usize> {
    let Some(variant_index) = variants
        .iter()
        .position(|variant| variant.id == variant_name)
    else {
        return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::UnknownVariant,
            Some(variant_name),
            choice_nominal_path.name(),
            variant_location.clone(),
        )));
    };

    Ok(variant_index)
}

/// Build a human-readable display name for a choice type from its nominal path.
///
/// WHAT: returns the leaf name of the choice (e.g. `"Result"` for `core::Result`),
/// falling back to `"<choice>"` if the path has no leaf segment.
/// WHY: diagnostic messages need a stable display string even when the nominal path
/// is synthetic or partially resolved.
fn choice_display_name(choice_nominal_path: &InternedPath, string_table: &StringTable) -> String {
    choice_nominal_path
        .name()
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| String::from("<choice>"))
}
