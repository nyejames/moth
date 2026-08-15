//! Shared match-arm header parsing for AST match syntax.
//!
//! WHAT: parses the reusable `<pattern> [if guard]` portion of a match arm,
//! resolves arm-local capture bindings, and leaves body parsing to the caller.
//! WHY: full statement matches and inline single-predicate value forms need
//! identical pattern, guard, and capture-scope semantics without sharing
//! statement-body parsing or constructing temporary `MatchArm` bodies.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::condition_validation::ensure_match_guard_condition;
use crate::compiler_frontend::ast::statements::if_headers::build_option_present_capture_scope_and_pattern;
use crate::compiler_frontend::ast::statements::match_patterns::{
    ChoicePayloadCapture, MatchPattern, ParsedChoicePattern, parse_choice_variant_pattern,
    parse_non_choice_pattern, parse_option_pattern,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::deferred_feature_diagnostics::deferred_feature_reason_diagnostic;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidMatchArmReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// Parsed pattern header shared by full match arms and inline single-predicate value `if`.
///
/// WHAT: carries the normalized pattern plus any arm-local scope introduced by
/// captures.
/// WHY: inline `if value is Pattern then ... else ...` must use the same pattern
/// parser as full match arms without manufacturing a temporary `=>` body.
pub(crate) struct ParsedSinglePredicatePattern {
    pub(crate) pattern: MatchPattern,
    pub(crate) arm_scope: ScopeContext,
}

/// Header facts needed by the caller that owns arm-body parsing.
pub(crate) struct ParsedMatchArmHeader {
    pub(crate) pattern: MatchPattern,
    pub(crate) guard: Option<Expression>,
    pub(crate) arm_scope: ScopeContext,
    pub(crate) matched_choice_variant: Option<StringId>,
    pub(crate) pattern_location: SourceLocation,
}

struct ParsedMatchPatternHeader {
    pattern: MatchPattern,
    matched_choice_variant: Option<StringId>,
    pattern_location: SourceLocation,
    arm_scope: ScopeContext,
}

/// Two-lane result for all match-header parsing functions.
///
/// WHAT: preserves authored match diagnostics and retained-token infrastructure failures.
/// WHY: guard expressions and option capture parsing can consume a frozen prepared-file stream,
/// so an invalid retained table must reach the module boundary as `CompilerError`.
type MatchHeaderResult<T> = Result<T, ExpressionParseError>;

/// Parse one reusable match-arm header.
///
/// The caller supplies the tokens that terminate a guard expression so the
/// pattern parser stays independent from the body grammar that follows it.
pub(crate) fn parse_match_arm_header(
    scrutinee: &Expression,
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    guard_end_tokens: &[TokenKind],
    string_table: &mut StringTable,
) -> MatchHeaderResult<ParsedMatchArmHeader> {
    let ParsedMatchPatternHeader {
        pattern,
        matched_choice_variant,
        pattern_location,
        arm_scope,
    } = parse_match_pattern_header(
        scrutinee,
        token_stream,
        match_context,
        type_interner,
        string_table,
    )?;

    reject_invalid_pattern_suffix(token_stream)?;

    let guard = parse_match_guard(
        token_stream,
        &arm_scope,
        type_interner,
        guard_end_tokens,
        string_table,
    )?;

    Ok(ParsedMatchArmHeader {
        pattern,
        guard,
        arm_scope,
        matched_choice_variant,
        pattern_location,
    })
}

/// Parse one pattern after `if <scrutinee> is` for inline value-producing `if`.
///
/// WHAT: reuses the same pattern resolution as full match arms, including choice
/// variant qualification and capture scope construction.
/// WHY: inline single-predicate value `if` must not resolve variant names as
/// ordinary values; full match parsing is the owner of that semantics.
pub(crate) fn parse_single_predicate_match_pattern(
    scrutinee: &Expression,
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MatchHeaderResult<ParsedSinglePredicatePattern> {
    let parsed = parse_match_pattern_header(
        scrutinee,
        token_stream,
        match_context,
        type_interner,
        string_table,
    )?;

    reject_invalid_pattern_suffix(token_stream)?;

    Ok(ParsedSinglePredicatePattern {
        pattern: parsed.pattern,
        arm_scope: parsed.arm_scope,
    })
}

/// Parse an optional `if <condition>` guard before the caller-owned separator.
///
/// WHY: guard parsing is self-contained (token check, expression parse, validation)
/// and extracting it keeps arm body parsers focused on their own body grammar.
fn parse_match_guard(
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    guard_end_tokens: &[TokenKind],
    string_table: &mut StringTable,
) -> MatchHeaderResult<Option<Expression>> {
    if token_stream.current_token_kind() != &TokenKind::If {
        return Ok(None);
    }

    token_stream.advance();
    token_stream.skip_newlines();

    let mut guard_type = ExpectedType::Infer;
    let guard_context = match_context.new_child_control_flow(ContextKind::Condition, string_table);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &guard_context,
        type_interner,
        expected_type: &mut guard_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let guard_expression = create_expression_until(input, guard_end_tokens)?;
    let type_environment = type_interner.environment();
    ensure_match_guard_condition(&guard_expression, type_environment)?;

    Ok(Some(guard_expression))
}

fn parse_match_pattern_header(
    scrutinee: &Expression,
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> MatchHeaderResult<ParsedMatchPatternHeader> {
    // Choice scrutinees resolve symbols to variants; all other scrutinees stay literal-only.
    let type_environment = type_interner.environment();
    let is_choice = matches!(
        type_environment.type_kind(scrutinee.type_id),
        Some(TypeKind::Choice | TypeKind::GenericInstance)
    );

    let (pattern, matched_choice_variant, pattern_location, arm_scope) = if is_choice {
        let type_environment = type_interner.environment();
        let variants = choice_variants_for_type(scrutinee.type_id, type_environment);
        let nominal_path = type_environment
            .nominal_path(scrutinee.type_id)
            .cloned()
            .unwrap_or_else(|| match_context.scope.clone());

        // Every bare symbol is resolved as a declared choice variant. Unknown
        // names receive the direct UnknownVariant diagnostic; no whole-value
        // capture syntax exists in full matches.
        let parsed = parse_choice_variant_pattern(
            token_stream,
            match_context,
            &nominal_path,
            &variants,
            string_table,
        )?;
        let matched_choice_variant = Some(parsed.variant);
        let pattern_location = parsed.location.clone();
        let (arm_scope, pattern) = build_arm_scope_with_choice_captures(
            match_context,
            parsed,
            type_environment,
            string_table,
        )?;
        (pattern, matched_choice_variant, pattern_location, arm_scope)
    } else {
        let option_inner_type_id = type_environment.option_inner_type(scrutinee.type_id);
        if let Some(inner_type_id) = option_inner_type_id {
            // Optional scrutinees must use option-specific patterns.
            // Bare capture symbols are rejected because `|name|` is the only
            // valid capture form for optional values.
            if let TokenKind::Symbol(_) = token_stream.current_token_kind()
                && !option_pattern_constructor_like(token_stream)
            {
                return Err(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee,
                    None,
                    None,
                    token_stream.current_location(),
                )
                .into());
            }

            let pattern =
                parse_option_pattern(token_stream, inner_type_id, string_table, type_environment)?;
            let location = pattern.location().to_owned();

            let (arm_scope, pattern) = if let MatchPattern::OptionPresentCapture {
                name,
                binding_location,
                inner_type_id: capture_inner_type_id,
                location: pattern_location,
                ..
            } = &pattern
            {
                build_option_present_capture_scope_and_pattern(
                    match_context,
                    *name,
                    binding_location,
                    *capture_inner_type_id,
                    pattern_location,
                    type_interner,
                    string_table,
                )?
            } else {
                (match_context.clone(), pattern)
            };

            (pattern, None, location, arm_scope)
        } else {
            let pattern = parse_non_choice_pattern(
                token_stream,
                scrutinee.type_id,
                string_table,
                type_environment,
            )?;
            let location = pattern.location().to_owned();
            (pattern, None, location, match_context.clone())
        }
    };

    Ok(ParsedMatchPatternHeader {
        pattern,
        matched_choice_variant,
        pattern_location,
        arm_scope,
    })
}

fn reject_invalid_pattern_suffix(token_stream: &FileTokens) -> MatchHeaderResult<()> {
    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        return Err(deferred_feature_reason_diagnostic(
            DeferredFeatureReason::CaptureTaggedPattern,
            token_stream.current_location(),
        )
        .into());
    }

    if token_stream.current_token_kind() == &TokenKind::As {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::AsNotValid,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    if token_stream.current_token_kind() == &TokenKind::Colon {
        return Err(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::LegacyColonSyntax,
            token_stream.current_location(),
        )
        .into());
    }

    if token_stream.current_token_kind() == &TokenKind::Arrow {
        return Err(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::InvalidArrow,
            token_stream.current_location(),
        )
        .into());
    }

    Ok(())
}

/// Returns true when a symbol-shaped option pattern looks constructor-like.
///
/// WHY: bare capture symbols are rejected for optional scrutinees, but constructor-like
/// syntax should continue into the pattern parser so it receives the more specific
/// unsupported-pattern diagnostic instead of being mistaken for a capture.
fn option_pattern_constructor_like(token_stream: &FileTokens) -> bool {
    token_stream
        .tokens
        .get(token_stream.index + 1)
        .is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::OpenParenthesis | TokenKind::DoubleColon
            )
        })
}

/// Build a choice arm scope and final pattern with fully resolved capture binding paths.
///
/// WHAT: clones the parent match context and adds `Declaration` entries for each parsed capture.
/// WHY: captures must be visible in both the guard and the body, but must not leak to other arms.
///
/// Validates:
/// - No capture name shadows an existing visible local (Moth no-shadowing rule).
fn build_arm_scope_with_choice_captures(
    match_context: &ScopeContext,
    parsed_pattern: ParsedChoicePattern,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> MatchHeaderResult<(ScopeContext, MatchPattern)> {
    let mut arm_scope = match_context.clone();
    let mut captures = Vec::with_capacity(parsed_pattern.captures.len());

    for capture in parsed_pattern.captures {
        let binding_name = capture.binding_name;

        // Enforce no-shadowing: the local binding name must not collide with any visible local.
        if let Some(_existing) = arm_scope.get_reference(&binding_name) {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::CaptureBindingShadowsVariable,
                None,
                None,
                capture.binding_location.clone(),
            )
            .into());
        }

        let binding_name_str = string_table.resolve(binding_name).to_owned();
        let binding_path = arm_scope.scope.join_str(&binding_name_str, string_table);

        let declaration = Declaration {
            id: binding_path.clone(),
            value: Expression::new(
                ExpressionKind::NoValue,
                capture.binding_location.clone(),
                capture.type_id,
                diagnostic_type_spelling(capture.type_id, type_environment),
                ValueMode::ImmutableOwned,
            ),
        };

        let binding_location = declaration.value.location.clone();
        arm_scope.add_var(declaration, binding_location);
        captures.push(ChoicePayloadCapture {
            field_index: capture.field_index,
            type_id: capture.type_id,
            binding_path,
            location: capture.location,
        });
    }

    Ok((
        arm_scope,
        MatchPattern::ChoiceVariant {
            nominal_path: parsed_pattern.nominal_path,
            tag: parsed_pattern.tag,
            captures,
            location: parsed_pattern.location,
        },
    ))
}

/// Fetch choice variants for a type, converting environment definitions into
/// AST-facing `ChoiceVariant` shapes.
///
/// WHAT: queries `TypeEnvironment` for variant metadata and maps payload
/// definitions into the local `ChoiceVariant` / `ChoiceVariantPayload` types.
/// WHY: match parsing needs AST-level variant info (names, field types, locations)
/// to validate choice arms, but it must not depend on the internal `TypeEnvironment`
/// representation beyond this boundary.
fn choice_variants_for_type(type_id: TypeId, env: &TypeEnvironment) -> Vec<ChoiceVariant> {
    env.variants_for(type_id)
        .map(|variants| {
            variants
                .iter()
                .map(|variant| ChoiceVariant {
                    id: variant.name,
                    payload: convert_choice_payload(&variant.payload, env),
                    location: variant.location.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_choice_payload(
    payload: &ChoiceVariantPayloadDefinition,
    type_environment: &TypeEnvironment,
) -> ChoiceVariantPayload {
    match payload {
        ChoiceVariantPayloadDefinition::Unit => ChoiceVariantPayload::Unit,
        ChoiceVariantPayloadDefinition::Record { fields } => ChoiceVariantPayload::Record {
            fields: fields
                .iter()
                .map(|field| Declaration {
                    id: field.name.clone(),
                    value: Expression::new(
                        ExpressionKind::NoValue,
                        field.location.clone(),
                        field.type_id,
                        diagnostic_type_spelling(field.type_id, type_environment),
                        ValueMode::ImmutableOwned,
                    ),
                })
                .collect(),
        },
    }
}
