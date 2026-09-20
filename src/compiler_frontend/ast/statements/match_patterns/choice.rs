//! Choice-variant pattern parsing.
//!
//! WHAT: parses `Variant =>` and `Variant(field) =>` patterns, resolving
//! variant names against the scrutinee choice declaration.
//! WHY: choice patterns have unique syntax (qualified names, payload captures)
//! and validation (exact field names, no reordering) that justify a dedicated file.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::deferred_feature_diagnostics::deferred_feature_reason_diagnostic;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

use rustc_hash::FxHashMap;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::types::ParsedChoicePattern;
use super::types::ParsedChoicePayloadCapture;

/// Typed result for choice-pattern parsing.
///
/// Authored syntax failures remain `CompilerDiagnostic`; retained-table and other compiler
/// invariant failures travel through the infrastructure lane without being reclassified.
type ChoicePatternResult<T> = Result<T, ExpressionParseError>;

/// Resolve a choice variant pattern to its deterministic variant index.
///
/// WHAT: accepts bare (`Ready`) or qualified (`Status::Ready`) variant names and
/// resolves them against the scrutinee choice metadata.
/// WHY: later lowering uses the stable variant index in `HirPattern::ChoiceVariant`,
/// while payload captures are materialized separately at arm entry.
pub fn parse_choice_variant_pattern(
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    choice_nominal_path: &PathId,
    variants: &[ChoiceVariant],
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> ChoicePatternResult<ParsedChoicePattern> {
    // Choice patterns support exact variant names plus constructor-like payload captures.
    if let Some(diagnostic) = reject_deferred_pattern_lead_token(token_stream) {
        return Err(diagnostic.into());
    }

    let choice_name_display = choice_display_name(choice_nominal_path, path_fork, string_table);
    let (variant_name, variant_span) = parse_variant_name(
        token_stream,
        match_context,
        choice_nominal_path,
        &choice_name_display,
        path_fork,
        string_table,
    )?;

    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(deferred_feature_reason_diagnostic(
            DeferredFeatureReason::CaptureTaggedPattern,
            current_span(token_stream),
        )
        .into());
    }

    let variant_index = resolve_variant_to_tag(
        variants,
        variant_name,
        &choice_name_display,
        variant_span,
        path_fork,
        choice_nominal_path,
    )?;

    let variant = &variants[variant_index];
    let captures = parse_choice_pattern_captures(
        token_stream,
        variant,
        &choice_name_display,
        string_table,
        path_fork,
    )?;

    Ok(ParsedChoicePattern {
        nominal_path: *choice_nominal_path,
        variant: variant_name,
        tag: variant_index,
        captures,
        span: variant_span,
    })
}

fn parse_choice_pattern_captures(
    token_stream: &mut AstCursor,
    variant: &ChoiceVariant,
    _choice_name_display: &str,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> ChoicePatternResult<Vec<ParsedChoicePayloadCapture>> {
    match &variant.payload {
        ChoiceVariantPayload::Unit => {
            if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
                return Err(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::UnitVariantHasPayload,
                    Some(variant.id),
                    None,
                    current_span(token_stream),
                )
                .into());
            }

            Ok(Vec::new())
        }

        ChoiceVariantPayload::Record { fields } => {
            if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
                return Err(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::PayloadVariantNeedsBindings,
                    Some(variant.id),
                    None,
                    current_span(token_stream),
                )
                .into());
            }

            token_stream.advance();

            let mut captures = Vec::new();
            let mut seen_names: FxHashMap<StringId, Option<SourceSpan>> = FxHashMap::default();

            loop {
                token_stream.skip_newlines();

                if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
                    token_stream.advance();
                    break;
                }

                let capture_span = current_span(token_stream);
                // Wildcards are not yet supported in choice payload position.
                if token_stream.current_tag() == TokenTag::WILDCARD {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::WildcardNotSupported,
                        None,
                        None,
                        capture_span,
                    )
                    .into());
                }

                if token_stream.current_tag() != TokenTag::SYMBOL {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::CaptureBindingMustBeFieldName,
                        None,
                        None,
                        capture_span,
                    )
                    .into());
                }
                let field_name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "choice payload field symbol had no string payload",
                        )
                    })?;
                token_stream.advance();

                let mut binding_name = field_name;
                let mut binding_span = capture_span;

                if token_stream.current_tag() == TokenTag::AS {
                    token_stream.advance();
                    binding_span = current_span(token_stream);
                    binding_name = match token_stream.current_tag() {
                        TokenTag::SYMBOL => token_stream
                            .current_string_id_in(string_table)?
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "choice payload alias symbol had no string payload",
                                )
                            })?,
                        TokenTag::END
                        | TokenTag::EOF
                        | TokenTag::CLOSE_PARENTHESIS
                        | TokenTag::COMMA => {
                            return Err(CompilerDiagnostic::invalid_match_pattern(
                                InvalidMatchPatternReason::ExpectedLocalBindingAfterAs,
                                None,
                                None,
                                binding_span,
                            )
                            .into());
                        }
                        _ => {
                            return Err(CompilerDiagnostic::invalid_match_pattern(
                                InvalidMatchPatternReason::AliasMustBeLocalBinding,
                                None,
                                None,
                                binding_span,
                            )
                            .into());
                        }
                    };
                    if token_stream.current_tag() == TokenTag::SYMBOL {
                        token_stream.advance();
                    }
                }

                if token_stream.current_tag() == TokenTag::ASSIGN {
                    return Err(deferred_feature_reason_diagnostic(
                        DeferredFeatureReason::NamedPayloadPatternAssignment,
                        current_span(token_stream),
                    )
                    .into());
                }

                // Check duplicate capture binding name (uses the local alias when present).
                if seen_names.contains_key(&binding_name) {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::DuplicateCaptureBinding,
                        Some(variant.id),
                        None,
                        binding_span,
                    )
                    .into());
                }
                seen_names.insert(binding_name, binding_span);

                // Validate capture position and name against declaration metadata.
                let field_index = captures.len();
                let Some(field_decl) = fields.get(field_index) else {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::TooManyCaptureBindings,
                        Some(variant.id),
                        None,
                        capture_span,
                    )
                    .into());
                };

                let expected_field_name =
                    choice_payload_field_name(field_decl, path_fork, string_table)?;
                if field_name != expected_field_name {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::CaptureBindingNameMismatch,
                        Some(variant.id),
                        None,
                        capture_span,
                    )
                    .into());
                }

                captures.push(ParsedChoicePayloadCapture {
                    binding_name,
                    field_index,
                    type_id: field_decl.value.type_id,
                    span: capture_span,
                    binding_span,
                });

                // Advance past the separator or detect the end of the capture list.
                token_stream.skip_newlines();
                match token_stream.current_tag() {
                    TokenTag::COMMA => {
                        token_stream.advance();
                        continue;
                    }
                    TokenTag::CLOSE_PARENTHESIS => {
                        token_stream.advance();
                        break;
                    }
                    TokenTag::OPEN_PARENTHESIS => {
                        return Err(deferred_feature_reason_diagnostic(
                            DeferredFeatureReason::NestedPayloadPattern,
                            current_span(token_stream),
                        )
                        .into());
                    }
                    _ => {
                        let found = token_stream
                            .current_diagnostic_token(string_table)
                            .map_err(|error| {
                                CompilerDiagnostic::token_view_invariant_error(
                                    error,
                                    "choice payload separator diagnostic",
                                )
                            })?;
                        return Err(CompilerDiagnostic::expected_token_from_tags(
                            TokenTag::COMMA,
                            found,
                            current_span(token_stream),
                        )
                        .into());
                    }
                }
            }

            // Check for too few captures.
            if captures.len() != fields.len() {
                return Err(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::TooFewCaptureBindings,
                    Some(variant.id),
                    None,
                    variant.span,
                )
                .into());
            }

            Ok(captures)
        }
    }
}

/// Resolve the leaf name of a choice payload field declaration.
///
/// WHAT: extracts the terminal identifier from a payload field's interned path.
fn choice_payload_field_name(
    field: &Declaration,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> ChoicePatternResult<StringId> {
    path_fork.component(field.id).ok_or_else(|| {
        ExpressionParseError::Infrastructure(Box::new(CompilerError::compiler_error(format!(
            "Choice payload field '{}' has no leaf name during match-pattern parsing",
            path_fork.render_portable(field.id, string_table, &mut Vec::new())
        ))))
    })
}

/// Parse a bare (`Ready`) or qualified (`Status::Ready`) variant name from the token stream.
///
/// WHY: separating token-level parsing from tag resolution keeps each function focused
/// and makes error messages specific to the syntactic layer they diagnose.
fn parse_variant_name(
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    choice_nominal_path: &PathId,
    _choice_name_display: &str,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> ChoicePatternResult<(StringId, Option<SourceSpan>)> {
    match token_stream.current_tag() {
        TokenTag::SYMBOL => {
            let first_name = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    CompilerError::compiler_error("choice variant symbol had no string payload")
                })?;
            let first_span = current_span(token_stream);
            token_stream.advance();

            if token_stream.current_tag() == TokenTag::DOUBLE_COLON {
                let expected_choice_name = path_fork.component(*choice_nominal_path);
                if expected_choice_name.is_some_and(|expected| first_name != expected)
                    && !qualifier_resolves_to_choice(match_context, first_name, choice_nominal_path)
                {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee,
                        None,
                        expected_choice_name,
                        first_span,
                    )
                    .into());
                }

                token_stream.advance();
                token_stream.skip_newlines();

                if token_stream.current_tag() != TokenTag::SYMBOL {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::ExpectedVariantNameAfterQualifier,
                        None,
                        None,
                        current_span(token_stream),
                    )
                    .into());
                }
                let qualified_variant_name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "qualified choice variant symbol had no string payload",
                        )
                    })?;
                let qualified_span = current_span(token_stream);
                token_stream.advance();
                Ok((qualified_variant_name, qualified_span))
            } else {
                Ok((first_name, first_span))
            }
        }

        // Literal tokens are not valid as choice variant names.
        TokenTag::NUMERIC_LITERAL
        | TokenTag::BOOL_LITERAL
        | TokenTag::CHAR_LITERAL
        | TokenTag::STRING_SLICE_LITERAL
        | TokenTag::NEGATIVE => Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::MustUseVariantNamesNotLiterals,
            None,
            None,
            current_span(token_stream),
        )
        .into()),

        _ => Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::MustStartWithVariantName,
            None,
            None,
            current_span(token_stream),
        )
        .into()),
    }
}

/// Check whether a leading qualifier symbol resolves to the scrutinee choice type.
fn qualifier_resolves_to_choice(
    match_context: &ScopeContext,
    qualifier: StringId,
    choice_nominal_path: &PathId,
) -> bool {
    match_context
        .get_reference(&qualifier)
        .is_some_and(|declaration| declaration.id == *choice_nominal_path)
}

/// Look up a variant name in the declared choice variant list and return its positional tag.
///
/// WHY: separating semantic resolution from token parsing produces clearer control flow
/// and keeps the error-construction logic for unknown variants in one place.
fn resolve_variant_to_tag(
    variants: &[ChoiceVariant],
    variant_name: StringId,
    _choice_name_display: &str,
    variant_span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
    choice_nominal_path: &PathId,
) -> ChoicePatternResult<usize> {
    let Some(variant_index) = variants
        .iter()
        .position(|variant| variant.id == variant_name)
    else {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::UnknownVariant,
            Some(variant_name),
            path_fork.component(*choice_nominal_path),
            variant_span,
        )
        .into());
    };

    Ok(variant_index)
}
fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    Some(token_stream.current_span())
}
fn choice_display_name(
    choice_nominal_path: &PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    path_fork
        .component(*choice_nominal_path)
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| String::from("<choice>"))
}
