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
use crate::compiler_frontend::ast::numeric_construction::PendingChoicePattern;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, names_declarations, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, DiagnosticToken, InvalidMatchPatternReason,
};
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::count_remaining_top_level_positional_items;

use rustc_hash::FxHashMap;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::types::{
    ChoicePayloadCapture, MatchPattern, ParsedChoicePattern, ParsedChoicePayloadCapture,
};

/// Typed result for choice-pattern parsing.
///
/// Diagnosed failures travel as drafts; retained-table and other compiler invariant failures
/// travel through the infrastructure lane without being reclassified.
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

    let (variant_name, variant_span, authored_qualifier) = parse_variant_name(
        token_stream,
        match_context,
        choice_nominal_path,
        path_fork,
        string_table,
    )?;

    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(names_declarations::deferred_feature_reason(
            DeferredFeatureReason::CaptureTaggedPattern,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let variant_index = resolve_variant_to_tag(
        match_context,
        variants,
        variant_name,
        authored_qualifier,
        variant_span,
        path_fork,
        choice_nominal_path,
        string_table,
    )?;

    let variant = &variants[variant_index];
    let captures = parse_choice_pattern_captures(token_stream, variant, string_table, path_fork)?;

    Ok(ParsedChoicePattern {
        nominal_path: *choice_nominal_path,
        variant: variant_name,
        tag: variant_index,
        captures,
        span: variant_span,
    })
}

/// Build one pending choice arm's pattern once its subject instance is selected.
///
/// WHAT: keeps the variant identity and capture bindings fixed at parse time, and reads each
///       captured field's type from the selected instance's pre-substituted variant view.
/// WHY: a pending generic subject names its variant from the declared owner, but a payload
///      field's type is only concrete once the origin selects the instance.
pub(in crate::compiler_frontend::ast::statements) fn finish_pending_choice_pattern(
    pattern: PendingChoicePattern,
    subject_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> ChoicePatternResult<MatchPattern> {
    let variant = type_environment
        .variants_for(subject_type_id)
        .and_then(|variants| variants.iter().find(|variant| variant.tag == pattern.tag))
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "a pending choice arm resumed against a subject without that variant",
            )
        })?;

    let mut captures = Vec::with_capacity(pattern.captures.len());
    for capture in pattern.captures {
        let field_type_id = match &variant.payload {
            ChoiceVariantPayloadDefinition::Record { fields } => {
                fields.get(capture.field_index).map(|field| field.type_id)
            }
            ChoiceVariantPayloadDefinition::Unit => None,
        }
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "a pending choice capture names a field its variant lacks",
            )
        })?;

        captures.push(ChoicePayloadCapture {
            field_index: capture.field_index,
            type_id: field_type_id,
            binding_path: capture.binding_path,
            span: capture.span,
            binding_span: capture.binding_span,
        });
    }

    Ok(MatchPattern::ChoiceVariant {
        nominal_path: pattern.nominal_path,
        tag: pattern.tag,
        captures,
        span: pattern.span,
    })
}

fn parse_choice_pattern_captures(
    token_stream: &mut AstCursor,
    variant: &ChoiceVariant,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> ChoicePatternResult<Vec<ParsedChoicePayloadCapture>> {
    match &variant.payload {
        ChoiceVariantPayload::Unit => {
            if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
                return Err(choices_control_flow::invalid_match_pattern(
                    InvalidMatchPatternReason::UnitVariantHasPayload,
                    Some(variant.id),
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            Ok(Vec::new())
        }

        ChoiceVariantPayload::Record { fields } => {
            if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
                return Err(choices_control_flow::invalid_match_pattern(
                    InvalidMatchPatternReason::PayloadVariantNeedsBindings,
                    Some(variant.id),
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            token_stream.advance();

            let mut captures = Vec::new();
            let mut seen_names: FxHashMap<StringId, Option<SourceSpan>> = FxHashMap::default();

            let closing_span = loop {
                token_stream.skip_newlines();

                if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
                    let closing_span = Some(token_stream.current_span());
                    token_stream.advance();
                    break closing_span;
                }

                let capture_span = Some(token_stream.current_span());
                // Wildcards are not supported in choice payload position.
                if token_stream.current_tag() == TokenTag::WILDCARD {
                    return Err(choices_control_flow::invalid_match_pattern(
                        InvalidMatchPatternReason::WildcardNotSupported,
                        None,
                        None,
                        capture_span,
                    )
                    .into());
                }

                if token_stream.current_tag() != TokenTag::SYMBOL {
                    return Err(choices_control_flow::invalid_match_pattern(
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
                        InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
                            "choice payload field symbol had no string payload",
                        )
                    })?;
                token_stream.advance();

                let mut binding_name = field_name;
                let mut binding_span = capture_span;

                if token_stream.current_tag() == TokenTag::AS {
                    token_stream.advance();
                    binding_span = Some(token_stream.current_span());
                    binding_name = match token_stream.current_tag() {
                        TokenTag::SYMBOL => token_stream
                            .current_string_id_in(string_table)?
                            .ok_or_else(|| {
                                InfrastructureFailure::internal(
                                    CompilerStage::AstConstruction,
                                    "choice payload alias symbol had no string payload",
                                )
                            })?,
                        TokenTag::END
                        | TokenTag::EOF
                        | TokenTag::CLOSE_PARENTHESIS
                        | TokenTag::COMMA => {
                            return Err(choices_control_flow::invalid_match_pattern(
                                InvalidMatchPatternReason::ExpectedLocalBindingAfterAs,
                                None,
                                None,
                                binding_span,
                            )
                            .into());
                        }
                        _ => {
                            return Err(choices_control_flow::invalid_match_pattern(
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
                    return Err(names_declarations::deferred_feature_reason(
                        DeferredFeatureReason::NamedPayloadPatternAssignment,
                        Some(token_stream.current_span()),
                    )
                    .into());
                }

                // Check duplicate capture binding name (uses the local alias when present).
                if seen_names.contains_key(&binding_name) {
                    return Err(choices_control_flow::invalid_match_pattern(
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
                    return Err(choices_control_flow::invalid_match_pattern_with_names(
                        InvalidMatchPatternReason::TooManyCaptureBindings,
                        Some(variant.id),
                        None,
                        Some((
                            fields.len(),
                            captures.len()
                                + 1
                                + count_remaining_top_level_positional_items(token_stream),
                        )),
                        payload_field_names(fields, path_fork, string_table)?,
                        capture_span,
                    )
                    .into());
                };

                let expected_field_name =
                    choice_payload_field_name(field_decl, path_fork, string_table)?;
                if field_name != expected_field_name {
                    return Err(choices_control_flow::invalid_match_pattern_with_names(
                        InvalidMatchPatternReason::CaptureBindingNameMismatch,
                        Some(variant.id),
                        None,
                        Some((fields.len(), captures.len() + 1)),
                        {
                            let mut names = payload_field_names(fields, path_fork, string_table)?;
                            names.push(field_name);
                            if binding_name != field_name {
                                names.push(binding_name);
                            }
                            names
                        },
                        capture_span,
                    )
                    .into());
                }

                captures.push(ParsedChoicePayloadCapture {
                    binding_name,
                    field_index,
                    type_id: field_decl.type_id,
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
                        let closing_span = Some(token_stream.current_span());
                        token_stream.advance();
                        break closing_span;
                    }
                    TokenTag::OPEN_PARENTHESIS => {
                        return Err(names_declarations::deferred_feature_reason(
                            DeferredFeatureReason::NestedPayloadPattern,
                            Some(token_stream.current_span()),
                        )
                        .into());
                    }
                    _ => {
                        let found = token_stream
                            .current_diagnostic_token(string_table)
                            .map_err(|error| {
                                InfrastructureFailure::malformed_token_projection(
                                    "choice payload separator diagnostic",
                                    error,
                                )
                            })?;
                        return Err(token_lexical::expected_token_from_projections(
                            DiagnosticToken::from_static_tag(TokenTag::COMMA),
                            found,
                            Some(token_stream.current_span()),
                        )
                        .into());
                    }
                }
            };

            // Check for too few captures.
            if captures.len() != fields.len() {
                return Err(choices_control_flow::invalid_match_pattern_with_names(
                    InvalidMatchPatternReason::TooFewCaptureBindings,
                    Some(variant.id),
                    None,
                    Some((fields.len(), captures.len())),
                    payload_field_names(fields, path_fork, string_table)?,
                    closing_span,
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
        ExpressionParseError::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Choice payload field '{}' has no leaf name during match-pattern parsing",
                    path_fork.render_portable(field.id, string_table, &mut Vec::new())
                ),
            )
            .into(),
        )
    })
}

/// WHAT: lists a variant's declared payload field names in declaration order.
/// WHY: capture diagnostics name the expected fields; a name mismatch appends the authored
/// capture and, when written, its `as` binding.
fn payload_field_names(
    fields: &[Declaration],
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> ChoicePatternResult<Vec<StringId>> {
    fields
        .iter()
        .map(|field| choice_payload_field_name(field, path_fork, string_table))
        .collect()
}

/// Parse a bare (`Ready`) or qualified (`Status::Ready`) variant name from the token stream.
///
/// WHY: separating token-level parsing from tag resolution keeps each function focused
/// and makes error messages specific to the syntactic layer they diagnose. A qualified arm
/// also returns its authored qualifier so later diagnostics use the author's spelling.
fn parse_variant_name(
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    choice_nominal_path: &PathId,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> ChoicePatternResult<(StringId, Option<SourceSpan>, Option<StringId>)> {
    match token_stream.current_tag() {
        TokenTag::SYMBOL => {
            let first_name = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "choice variant symbol had no string payload",
                    )
                })?;
            let first_span = Some(token_stream.current_span());
            token_stream.advance();

            if token_stream.current_tag() == TokenTag::DOUBLE_COLON {
                if let Some(gap) = token_stream.current_choice_variant_separator_gap() {
                    return Err(token_lexical::invalid_choice_variant_spacing(
                        gap,
                        Some(token_stream.current_span()),
                    )
                    .into());
                }

                let expected_choice_name = path_fork.component(*choice_nominal_path);
                if expected_choice_name.is_some_and(|expected| first_name != expected)
                    && !qualifier_resolves_to_choice(
                        match_context,
                        first_name,
                        first_span,
                        choice_nominal_path,
                    )?
                {
                    return Err(choices_control_flow::invalid_match_pattern(
                        InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee,
                        Some(first_name),
                        choice_visible_name(
                            match_context,
                            choice_nominal_path,
                            path_fork,
                            string_table,
                        ),
                        first_span,
                    )
                    .into());
                }

                token_stream.advance();

                if token_stream.current_tag() != TokenTag::SYMBOL {
                    return Err(choices_control_flow::invalid_match_pattern(
                        InvalidMatchPatternReason::ExpectedVariantNameAfterQualifier,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    )
                    .into());
                }
                let qualified_variant_name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
                            "qualified choice variant symbol had no string payload",
                        )
                    })?;
                let qualified_span = Some(token_stream.current_span());
                token_stream.advance();
                Ok((qualified_variant_name, qualified_span, Some(first_name)))
            } else {
                Ok((first_name, first_span, None))
            }
        }

        // Literal tokens are not valid as choice variant names.
        TokenTag::NUMERIC_LITERAL
        | TokenTag::BOOL_LITERAL
        | TokenTag::CHAR_LITERAL
        | TokenTag::STRING_SLICE_LITERAL
        | TokenTag::NEGATIVE => Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::MustUseVariantNamesNotLiterals,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into()),
        _ => Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::MustStartWithVariantName,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

/// Check whether a leading qualifier symbol resolves to the scrutinee choice type.
fn qualifier_resolves_to_choice(
    match_context: &ScopeContext,
    qualifier: StringId,
    qualifier_span: Option<SourceSpan>,
    choice_nominal_path: &PathId,
) -> ChoicePatternResult<bool> {
    match match_context.get_value_reference(&qualifier, qualifier_span)? {
        Some(reference) => Ok(reference
            .ready_declaration()
            .is_some_and(|declaration| declaration.id == *choice_nominal_path)),
        None => Ok(false),
    }
}

/// Look up a variant name in the declared choice variant list and return its positional tag.
///
/// WHY: separating semantic resolution from token parsing produces clearer control flow
/// and keeps the error-construction logic for unknown variants in one place.
fn resolve_variant_to_tag(
    match_context: &ScopeContext,
    variants: &[ChoiceVariant],
    variant_name: StringId,
    authored_qualifier: Option<StringId>,
    variant_span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
    choice_nominal_path: &PathId,
    string_table: &StringTable,
) -> ChoicePatternResult<usize> {
    let Some(variant_index) = variants
        .iter()
        .position(|variant| variant.id == variant_name)
    else {
        return Err(choices_control_flow::invalid_match_pattern_with_names(
            InvalidMatchPatternReason::UnknownVariant,
            Some(variant_name),
            authored_qualifier.or_else(|| {
                choice_visible_name(match_context, choice_nominal_path, path_fork, string_table)
            }),
            None,
            variants.iter().map(|variant| variant.id).collect(),
            variant_span,
        )
        .into());
    };

    Ok(variant_index)
}

/// Resolves the choice name as the current file spells it, falling back to the canonical name.
///
/// WHY: an aliased import should be named by the local spelling the author wrote.
fn choice_visible_name(
    match_context: &ScopeContext,
    choice_nominal_path: &PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Option<StringId> {
    let canonical = path_fork.component(*choice_nominal_path);
    match_context
        .visible_name_for_declaration(choice_nominal_path, canonical, string_table)
        .or(canonical)
}
