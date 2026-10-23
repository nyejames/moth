//! Shared match-arm header parsing for AST match syntax.
//!
//! WHAT: parses scrutinees through `is`, reusable `<pattern> [if guard]` headers,
//! and option/choice capture scopes. Body parsing stays with the caller.
//! WHY: statement, template, full match and value single-predicate forms share
//! pattern and capture construction. This file does not own `if` header scanning.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_until;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::field_access::{
    RetainedOperandResult, concrete_shape_type, retained_operand_result,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::{
    DeclaredChoiceSubject, declared_choice_payload_shape, declared_choice_subject,
    optional_subject_inner,
};
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::statements::condition_validation::ensure_match_guard_condition;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, PendingChoiceCapture, PendingChoicePattern, PendingOptionCapture,
    PendingOptionPattern, StatementExpression, deferred_numeric_expression,
};
use crate::compiler_frontend::ast::numeric_known_shape::KnownShapeId;
use crate::compiler_frontend::ast::statements::match_patterns::{
    ChoicePayloadCapture, MatchPattern, MatchSubject, OptionCaptureBinding, ParsedChoicePattern,
    parse_choice_variant_pattern, parse_non_choice_pattern, parse_option_capture_binding,
    parse_option_pattern,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, names_declarations,
};
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, InvalidMatchArmReason, InvalidMatchPatternReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// Parsed pattern header shared by full match arms and single-predicate value `if`.
///
/// WHAT: carries the normalized pattern plus any arm-local scope introduced by
/// captures.
/// WHY: inline `if value is Pattern then ... else ...` must use the same pattern
/// parser as full match arms without manufacturing a temporary `=>` body.
pub(crate) struct ParsedSinglePredicatePattern {
    pub(crate) pattern: DeferredArmPattern,
    pub(crate) arm_scope: ScopeContext,
}

/// Header facts needed by the caller that owns arm-body parsing.
pub(crate) struct ParsedMatchArmHeader {
    pub(crate) pattern: DeferredArmPattern,
    /// The authored guard: built, or retained when it still references a pending numeric origin.
    ///
    /// WHY: a guard is a boolean predicate over the arm scope, so a guard whose operands only the
    ///      body root can select keeps its authored fragment and is re-validated as `Bool` there.
    pub(crate) guard: Option<StatementExpression>,
    pub(crate) arm_scope: ScopeContext,
    pub(crate) matched_choice_variant: Option<StringId>,
    pub(crate) pattern_span: Option<SourceSpan>,
}

struct ParsedMatchPatternHeader {
    pattern: DeferredArmPattern,
    matched_choice_variant: Option<StringId>,
    pattern_span: Option<SourceSpan>,
    arm_scope: ScopeContext,
}

/// Two-lane result for all match-header parsing functions.
///
/// WHAT: preserves authored match diagnostics and retained-token infrastructure failures.
/// WHY: guard expressions and option capture parsing can consume a frozen prepared-file stream,
/// so an invalid retained table must reach the module boundary as `CompilerError`.
type MatchHeaderResult<T> = Result<T, ExpressionParseError>;

/// Parse a scrutinee expression that stops at the top-level `is` token.
///
/// WHAT: shared two-lane expression parse used by statement, template and value
/// receivers before they consume `is`; the position owns its complete fragment, so a
/// numeric-dependent scrutinee comes back as a retained statement expression.
/// WHY: each consumer previously reconstructed this `until([Is])` boundary, and a scrutinee that
///      references a pending numeric origin must keep its authored syntax until the body root
///      selects the subject type instead of fabricating a typed value now.
pub(crate) fn parse_scrutinee_until_is(
    token_stream: &mut AstCursor,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<StatementExpression> {
    let mut scrutinee_type = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        expected_type: &mut scrutinee_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
        path_fork,
    })
    .with_numeric_deferral_owner();

    match create_expression_until(input, &[TokenTag::IS]).map(|expression| expression.id()) {
        Ok(expression) => {
            expression_builder
                .view(expression)?
                .require_single(ValueReceiverContext::MatchScrutinee)?;
            Ok(StatementExpression::Built(expression))
        }
        Err(error @ ExpressionParseError::NumericDeferral(_)) => {
            // The scrutinee position owns its deferral, so the fragment is the whole authored
            // syntax and keeps its own `Infer` expectation: the subject type is whatever the
            // pending origin resolves to, never an eager default.
            let deferral = error.take_numeric_deferral("a match scrutinee")?;
            Ok(StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                deferral.expected,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            )))
        }
        Err(other) => Err(other),
    }
}

/// Build an option-present capture arm scope and pattern.
///
/// WHAT: clones the parent context and adds a `Declaration` for the inner payload
/// binding so the matched branch can reference the unwrapped value.
/// WHY: statement `if`, template `if`, full matches and value single predicates
/// share one capture-scope construction rule.
pub(crate) fn build_option_present_capture_scope_and_pattern(
    match_context: &ScopeContext,
    capture_name: StringId,
    binding_span: Option<SourceSpan>,
    inner_type_id: TypeId,
    pattern_span: Option<SourceSpan>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<(ScopeContext, MatchPattern)> {
    let mut arm_scope = match_context.clone();

    if arm_scope.get_reference(&capture_name).is_some() {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::CaptureBindingShadowsVariable,
            None,
            None,
            binding_span,
        )
        .into());
    }

    if let Some(diagnostic) =
        arm_scope.check_body_binding_visibility(capture_name, binding_span, string_table)
    {
        return Err(diagnostic.into());
    }

    let binding_path = path_fork
        .try_intern_child(arm_scope.scope, capture_name)
        .expect("path table exhausted while creating match capture scope");

    let declaration = Declaration {
        id: binding_path,
        type_id: inner_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span,
        config_qualifier: None,
    };

    let declaration_span = declaration.binding_span;
    arm_scope.add_var(declaration, declaration_span, path_fork);

    let pattern = MatchPattern::OptionPresentCapture {
        name: capture_name,
        binding_path,
        inner_type_id,
        span: pattern_span,
        binding_span,
    };

    Ok((arm_scope, pattern))
}

/// Parse one reusable match-arm header.
///
/// The caller supplies the subject (already typed, or pending numeric construction) and the
/// tokens that terminate a guard expression so the pattern parser stays independent from the body
/// grammar that follows it.
#[allow(
    clippy::too_many_arguments,
    reason = "arm header parsing keeps the subject, token stream, match scope, mutable interner/builder/constant/string/path state, and guard terminators as separate borrows"
)]
pub(crate) fn parse_match_arm_header(
    subject: &MatchSubject,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    guard_end_tokens: &[TokenTag],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<ParsedMatchArmHeader> {
    let ParsedMatchPatternHeader {
        pattern,
        matched_choice_variant,
        pattern_span,
        arm_scope,
    } = parse_match_pattern_header(
        subject,
        token_stream,
        match_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    reject_invalid_pattern_suffix(token_stream)?;

    let guard = parse_match_guard(
        token_stream,
        &arm_scope,
        type_interner,
        expression_builder,
        const_values,
        guard_end_tokens,
        string_table,
        path_fork,
    )?;

    Ok(ParsedMatchArmHeader {
        pattern,
        guard,
        arm_scope,
        matched_choice_variant,
        pattern_span,
    })
}

/// Parse one pattern after `if <scrutinee> is` for value-producing single-predicate `if`.
///
/// WHAT: reuses the same pattern resolution as full match arms, including choice
/// variant qualification and capture scope construction, and leaves the body
/// delimiter (`then` or `:`) for the caller.
/// WHY: inline and block single-predicate value `if` must not resolve variant
/// names as ordinary values; full match parsing is the owner of that semantics. A pending numeric
/// subject returns its authored pattern draft (`Retained`) so the value-block owner can
/// materialise it at the selected subject.
#[allow(
    clippy::too_many_arguments,
    reason = "single-predicate header parsing threads the subject, token stream, match scope, mutable interner/builder/constant/string/path state as separate borrows"
)]
pub(crate) fn parse_single_predicate_match_pattern(
    subject: &MatchSubject,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<ParsedSinglePredicatePattern> {
    let parsed = parse_match_pattern_header(
        subject,
        token_stream,
        match_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    // Colon starts the block body of a single-predicate value match. Full match
    // arms still reject it as legacy `pattern:` syntax through `parse_match_arm_header`.
    if token_stream.current_tag() != TokenTag::COLON {
        reject_invalid_pattern_suffix(token_stream)?;
    }

    Ok(ParsedSinglePredicatePattern {
        pattern: parsed.pattern,
        arm_scope: parsed.arm_scope,
    })
}
/// Parse one arm guard, retaining its authored fragment when it needs body-root completion.
///
/// WHAT: a guard that builds now is validated as `Bool` immediately; a guard referencing a pending
///       numeric origin keeps its complete fragment with a `Bool` receiving expectation.
/// WHY: the guard position is a boolean predicate, so completion evaluates the retained fragment
///      in that exact receiver and re-applies the same guard validation.
#[allow(
    clippy::too_many_arguments,
    reason = "guard parsing keeps the token stream, match scope, mutable interner/builder/constant/string/path state, and guard terminators as separate borrows"
)]
fn parse_match_guard(
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    guard_end_tokens: &[TokenTag],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<Option<StatementExpression>> {
    if token_stream.current_tag() != TokenTag::IF {
        return Ok(None);
    }

    token_stream.advance();
    token_stream.skip_newlines();

    let mut guard_type = ExpectedType::Infer;
    let guard_context =
        match_context.new_child_control_flow(ContextKind::Condition, string_table, path_fork);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &guard_context,
        type_interner,
        expression_builder,
        const_values,
        expected_type: &mut guard_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
        path_fork,
    })
    .with_numeric_deferral_owner();

    match create_expression_until(input, guard_end_tokens).map(|expression| expression.id()) {
        Ok(guard_expression) => {
            ensure_match_guard_condition(
                expression_builder.view(guard_expression)?,
                type_interner.environment(),
            )?;
            Ok(Some(StatementExpression::Built(guard_expression)))
        }
        Err(error @ ExpressionParseError::NumericDeferral(_)) => {
            let deferral = error.take_numeric_deferral("a match arm guard")?;
            Ok(Some(StatementExpression::Retained(
                deferred_numeric_expression(
                    deferral.items,
                    ExpectedType::Known(type_interner.environment().builtins().bool),
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                ),
            )))
        }
        Err(other) => Err(other),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "pattern header parsing threads the subject, token stream, match scope, mutable interner/builder/constant/string/path state as separate borrows"
)]
fn parse_match_pattern_header(
    subject: &MatchSubject,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<ParsedMatchPatternHeader> {
    let subject_type_id = match *subject {
        MatchSubject::Known(subject_type_id) => subject_type_id,
        MatchSubject::PendingChoice(declared) => {
            return parse_pending_choice_pattern_header(
                declared,
                token_stream,
                match_context,
                type_interner,
                string_table,
                path_fork,
            );
        }
        MatchSubject::PendingOption { inner } => {
            return parse_pending_option_pattern_header(
                inner,
                token_stream,
                match_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            );
        }
        // A pending numeric subject has no selected type yet, so choice/option pattern families
        // cannot be resolved; only the authored non-choice pattern draft can be produced now.
        MatchSubject::Pending => {
            let pattern = parse_non_choice_pattern(
                token_stream,
                MatchSubject::Pending,
                string_table,
                type_interner.environment(),
                expression_builder,
                const_values,
            )?;
            return Ok(ParsedMatchPatternHeader {
                pattern_span: deferred_pattern_span(&pattern),
                pattern,
                matched_choice_variant: None,
                arm_scope: match_context.clone(),
            });
        }
    };

    // Choice scrutinees resolve symbols to variants; all other scrutinees stay literal-only.
    let type_environment = type_interner.environment();
    let is_choice = matches!(
        type_environment.type_kind(subject_type_id),
        Some(TypeKind::Choice | TypeKind::GenericInstance)
    );

    let (pattern, matched_choice_variant, pattern_span, arm_scope) = if is_choice {
        let type_environment = type_interner.environment();
        let variants = choice_variants_for_type(subject_type_id, type_environment);
        let nominal_path = type_environment
            .nominal_path(subject_type_id)
            .cloned()
            .unwrap_or(match_context.scope);

        // Every bare symbol is resolved as a declared choice variant. Unknown
        // names receive the direct UnknownVariant diagnostic; no whole-value
        // capture syntax exists in full matches.
        let parsed = parse_choice_variant_pattern(
            token_stream,
            match_context,
            &nominal_path,
            &variants,
            string_table,
            path_fork,
        )?;
        let matched_choice_variant = Some(parsed.variant);
        let pattern_span = parsed.span;
        let (arm_scope, pattern) =
            build_arm_scope_with_choice_captures(match_context, parsed, string_table, path_fork)?;
        (
            DeferredArmPattern::Ready(pattern),
            matched_choice_variant,
            pattern_span,
            arm_scope,
        )
    } else {
        let option_inner_type_id = type_environment.option_inner_type(subject_type_id);
        if let Some(inner_type_id) = option_inner_type_id {
            // Optional scrutinees must use option-specific patterns.
            // Bare capture symbols are rejected because `|name|` is the only
            // valid capture form for optional values.
            if token_stream.current_tag() == TokenTag::SYMBOL
                && !option_pattern_constructor_like(token_stream)
            {
                return Err(choices_control_flow::invalid_match_pattern(
                    InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee,
                    None,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            let pattern = parse_option_pattern(
                token_stream,
                inner_type_id,
                string_table,
                type_environment,
                expression_builder,
                const_values,
            )?;
            let pattern_span = pattern.span();

            let (arm_scope, pattern) = if let MatchPattern::OptionPresentCapture {
                name,
                binding_span,
                inner_type_id: capture_inner_type_id,
                span: parsed_pattern_span,
                ..
            } = &pattern
            {
                build_option_present_capture_scope_and_pattern(
                    match_context,
                    *name,
                    *binding_span,
                    *capture_inner_type_id,
                    *parsed_pattern_span,
                    string_table,
                    path_fork,
                )?
            } else {
                (match_context.clone(), pattern)
            };

            (
                DeferredArmPattern::Ready(pattern),
                None,
                pattern_span,
                arm_scope,
            )
        } else {
            let pattern = parse_non_choice_pattern(
                token_stream,
                MatchSubject::Known(subject_type_id),
                string_table,
                type_environment,
                expression_builder,
                const_values,
            )?;
            let pattern_span = deferred_pattern_span(&pattern);
            (pattern, None, pattern_span, match_context.clone())
        }
    };

    Ok(ParsedMatchPatternHeader {
        pattern,
        matched_choice_variant,
        pattern_span,
        arm_scope,
    })
}

/// The authored span that identifies one arm pattern draft.
///
/// WHAT: the built pattern's own span, or the retained fragment's authored value span.
/// WHY: arm coverage facts and unreachable-arm warnings are keyed on the authored pattern span,
///      which exists before the subject type is selected.
pub(crate) fn deferred_pattern_span(pattern: &DeferredArmPattern) -> Option<SourceSpan> {
    match pattern {
        DeferredArmPattern::Ready(pattern) => pattern.span(),
        DeferredArmPattern::Retained(retained) => retained.value.span,
        DeferredArmPattern::PendingChoice(pattern) => pattern.span,
        DeferredArmPattern::PendingOption(pattern) => pattern.span(),
    }
}

fn reject_invalid_pattern_suffix(token_stream: &AstCursor) -> MatchHeaderResult<()> {
    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(names_declarations::deferred_feature_reason(
            DeferredFeatureReason::CaptureTaggedPattern,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() == TokenTag::AS {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::AsNotValid,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() == TokenTag::COLON {
        return Err(choices_control_flow::invalid_match_arm(
            InvalidMatchArmReason::LegacyColonSyntax,
            Some(token_stream.current_span()),
        )
        .into());
    }
    if token_stream.current_tag() == TokenTag::ARROW {
        return Err(choices_control_flow::invalid_match_arm(
            InvalidMatchArmReason::InvalidArrow,
            Some(token_stream.current_span()),
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
fn option_pattern_constructor_like(token_stream: &AstCursor) -> bool {
    token_stream
        .peek_next_tag()
        .is_some_and(|tag| matches!(tag, TokenTag::OPEN_PARENTHESIS | TokenTag::DOUBLE_COLON))
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
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<(ScopeContext, MatchPattern)> {
    let mut arm_scope = match_context.clone();
    let mut captures = Vec::with_capacity(parsed_pattern.captures.len());

    for capture in parsed_pattern.captures {
        let binding_name = capture.binding_name;

        // Enforce no-shadowing: the local binding name must not collide with any visible local.
        if let Some(_existing) = arm_scope.get_reference(&binding_name) {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::CaptureBindingShadowsVariable,
                None,
                None,
                capture.binding_span,
            )
            .into());
        }

        if let Some(diagnostic) = arm_scope.check_body_binding_visibility(
            binding_name,
            capture.binding_span,
            string_table,
        ) {
            return Err(diagnostic.into());
        }

        let binding_path = path_fork
            .try_intern_child(arm_scope.scope, binding_name)
            .expect("path table exhausted while creating choice capture scope");

        let declaration = Declaration {
            id: binding_path,
            type_id: capture.type_id,
            type_syntax: None,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: capture.binding_span,
            config_qualifier: None,
        };

        let declaration_span = declaration.binding_span;
        arm_scope.add_var(declaration, declaration_span, path_fork);
        captures.push(ChoicePayloadCapture {
            field_index: capture.field_index,
            type_id: capture.type_id,
            binding_path,
            span: capture.span,
            binding_span: capture.binding_span,
        });
    }

    Ok((
        arm_scope,
        MatchPattern::ChoiceVariant {
            nominal_path: parsed_pattern.nominal_path,
            tag: parsed_pattern.tag,
            captures,
            span: parsed_pattern.span,
        },
    ))
}

/// Classify a retained match subject from its authored fragment.
///
/// WHAT: a single retained operand whose known result is a concrete type is a known subject. One
///       whose known result is a declared generic choice shape keeps its declared owner, so its
///       variant names resolve at parse time; every other retained subject stays pending.
/// WHY: the shared retained operand owner follows groups and bindings, so a grouped `(origin)`
///      subject and a concrete `Delivery::Ready(number)` subject classify the same as their direct
///      forms. An instance's type arguments are selected only at body completion, but the variant
///      set and capture names are fixed by the declared owner, so the header resolves them early.
pub(crate) fn retained_match_subject(
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    items: &[ExpressionRpnItem],
    expression_builder: &AstExpressionBuilder,
) -> MatchSubject {
    let Some(item) = single_operand(items) else {
        return MatchSubject::Pending;
    };
    match retained_operand_result(item, scope_context, environment, false, expression_builder) {
        Some(RetainedOperandResult::Type(type_id)) => MatchSubject::Known(type_id),
        Some(RetainedOperandResult::Shape(shape)) => {
            match concrete_shape_type(scope_context, shape) {
                Some(type_id) => MatchSubject::Known(type_id),
                None => {
                    let state = scope_context.numeric_state();
                    declared_choice_subject(&state, environment, shape)
                        .map(MatchSubject::PendingChoice)
                        .or_else(|| {
                            optional_subject_inner(&state, shape)
                                .map(|inner| MatchSubject::PendingOption { inner })
                        })
                        .unwrap_or(MatchSubject::Pending)
                }
            }
        }
        None => MatchSubject::Pending,
    }
}

/// Parse a choice arm header whose subject is a retained generic choice instance.
///
/// WHAT: resolves variant names against the declared owner and keeps each capture as a pending
///       numeric binding whose payload field shape comes from that owner.
/// WHY: the instance's type arguments are selected at body completion, but the variant set and
///      the authored capture names are already fixed by the declared owner.
fn parse_pending_choice_pattern_header(
    declared: DeclaredChoiceSubject,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<ParsedMatchPatternHeader> {
    let type_environment = type_interner.environment();
    let variants = choice_variants_for_type(declared.declared_type_id, type_environment);
    let nominal_path = type_environment
        .nominal_path(declared.declared_type_id)
        .cloned()
        .ok_or_else(|| {
            CompilerError::compiler_error("a declared pending choice subject had no nominal path")
        })?;

    let parsed = parse_choice_variant_pattern(
        token_stream,
        match_context,
        &nominal_path,
        &variants,
        string_table,
        path_fork,
    )?;
    let matched_choice_variant = Some(parsed.variant);
    let pattern_span = parsed.span;
    let (arm_scope, pending) = build_pending_choice_arm_scope(
        match_context,
        declared,
        parsed,
        type_interner,
        string_table,
        path_fork,
    )?;

    Ok(ParsedMatchPatternHeader {
        pattern: DeferredArmPattern::PendingChoice(pending),
        matched_choice_variant,
        pattern_span,
        arm_scope,
    })
}

/// Build a retained choice arm's scope and pending pattern.
///
/// WHAT: adds each authored capture as a pending numeric binding whose shape is its declared
///       payload field, and keeps the variant identity for the body root to finish.
/// WHY: a capture's concrete type is the selected instance's field type, which only the body
///      root knows, so a capture must not pin the origin's width while the arm is parsed.
///
/// Validates:
/// - No capture name shadows an existing visible local (Moth no-shadowing rule).
fn build_pending_choice_arm_scope(
    match_context: &ScopeContext,
    subject: DeclaredChoiceSubject,
    parsed_pattern: ParsedChoicePattern,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<(ScopeContext, PendingChoicePattern)> {
    let arm_scope = match_context.clone();
    let mut captures = Vec::with_capacity(parsed_pattern.captures.len());

    for capture in parsed_pattern.captures {
        let binding_name = capture.binding_name;

        if arm_scope.get_reference(&binding_name).is_some() {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::CaptureBindingShadowsVariable,
                None,
                None,
                capture.binding_span,
            )
            .into());
        }

        if let Some(diagnostic) = arm_scope.check_body_binding_visibility(
            binding_name,
            capture.binding_span,
            string_table,
        ) {
            return Err(diagnostic.into());
        }

        let binding_path = path_fork
            .try_intern_child(arm_scope.scope, binding_name)
            .expect("path table exhausted while creating choice capture scope");

        let shape = {
            let mut numeric_state = match_context.numeric_state_mut();
            declared_choice_payload_shape(
                &mut numeric_state,
                type_interner.environment_mut_for_derived_types(),
                subject,
                parsed_pattern.tag,
                capture.field_index,
                capture.span,
            )?
        };
        arm_scope.declare_pending_pattern_capture(
            AuthoredBindingFacts {
                name: binding_name,
                path: binding_path,
                span: capture.binding_span,
                binding_span: capture.binding_span,
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            shape,
        )?;
        captures.push(PendingChoiceCapture {
            field_index: capture.field_index,
            binding_path,
            span: capture.span,
            binding_span: capture.binding_span,
        });
    }

    Ok((
        arm_scope,
        PendingChoicePattern {
            nominal_path: parsed_pattern.nominal_path,
            tag: parsed_pattern.tag,
            captures,
            span: parsed_pattern.span,
        },
    ))
}

/// Parse one arm header against a retained optional subject.
///
/// WHAT: `none` is a built absence pattern; `|name|` declares a pending payload capture over the
///       subject's retained inner shape; a value pattern is parsed as a non-choice value and is
///       finished at the body root.
/// WHY: the optional's inner shape is known at parse time, so a capture can be named before the
///      subject's concrete optional type is selected.
#[allow(
    clippy::too_many_arguments,
    reason = "optional pending header parsing threads the subject, token stream, match scope, mutable interner/builder/constant/string/path state as separate borrows"
)]
fn parse_pending_option_pattern_header(
    inner: KnownShapeId,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<ParsedMatchPatternHeader> {
    if token_stream.current_tag() == TokenTag::NONE_LITERAL {
        let span = Some(token_stream.current_span());
        token_stream.advance();
        return Ok(ParsedMatchPatternHeader {
            pattern: DeferredArmPattern::Ready(MatchPattern::OptionNone { span }),
            matched_choice_variant: None,
            pattern_span: span,
            arm_scope: match_context.clone(),
        });
    }

    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        let binding = parse_option_capture_binding(token_stream, string_table)?;
        let (arm_scope, capture) = build_pending_option_capture_scope(
            match_context,
            inner,
            binding,
            string_table,
            path_fork,
        )?;
        let pending = PendingOptionPattern::PresentCapture(capture);
        return Ok(ParsedMatchPatternHeader {
            pattern_span: pending.span(),
            pattern: DeferredArmPattern::PendingOption(pending),
            matched_choice_variant: None,
            arm_scope,
        });
    }

    // Optional scrutinees must use option-specific patterns; `|name|` is the only capture form.
    if token_stream.current_tag() == TokenTag::SYMBOL
        && !option_pattern_constructor_like(token_stream)
    {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let pending = match parse_non_choice_pattern(
        token_stream,
        MatchSubject::Pending,
        string_table,
        type_interner.environment(),
        expression_builder,
        const_values,
    )? {
        DeferredArmPattern::Ready(pattern) => PendingOptionPattern::PresentValue(pattern),
        DeferredArmPattern::Retained(retained) => PendingOptionPattern::RetainedValue(retained),
        DeferredArmPattern::PendingChoice(_) | DeferredArmPattern::PendingOption(_) => {
            return Err(CompilerError::compiler_error(
                "a pending non-choice parse returned a pending choice or option pattern",
            )
            .into());
        }
    };

    Ok(ParsedMatchPatternHeader {
        pattern_span: pending.span(),
        pattern: DeferredArmPattern::PendingOption(pending),
        matched_choice_variant: None,
        arm_scope: match_context.clone(),
    })
}

/// Build a retained optional arm's scope and pending capture.
///
/// WHAT: adds the authored payload capture as a pending numeric binding over the subject's inner
///       shape, and keeps its arm-local path for the body root to finish.
/// WHY: the capture's concrete type is the selected optional's inner type, which only the body
///      root knows, so the capture must not pin the origin's width while the arm is parsed.
///
/// Validates:
/// - The capture name does not shadow an existing visible local (Moth no-shadowing rule).
fn build_pending_option_capture_scope(
    match_context: &ScopeContext,
    inner: KnownShapeId,
    binding: OptionCaptureBinding,
    string_table: &StringTable,
    path_fork: &mut PathInternerFork,
) -> MatchHeaderResult<(ScopeContext, PendingOptionCapture)> {
    let arm_scope = match_context.clone();

    if arm_scope.get_reference(&binding.name).is_some() {
        return Err(choices_control_flow::invalid_match_pattern(
            InvalidMatchPatternReason::CaptureBindingShadowsVariable,
            None,
            None,
            binding.binding_span,
        )
        .into());
    }

    if let Some(diagnostic) =
        arm_scope.check_body_binding_visibility(binding.name, binding.binding_span, string_table)
    {
        return Err(diagnostic.into());
    }

    let binding_path = path_fork
        .try_intern_child(arm_scope.scope, binding.name)
        .expect("path table exhausted while creating option capture scope");

    arm_scope.declare_pending_pattern_capture(
        AuthoredBindingFacts {
            name: binding.name,
            path: binding_path,
            span: binding.binding_span,
            binding_span: binding.binding_span,
            value_mode: ValueMode::ImmutableOwned,
            mutable: false,
        },
        inner,
    )?;

    Ok((
        arm_scope,
        PendingOptionCapture {
            name: binding.name,
            binding_path,
            span: binding.span,
            binding_span: binding.binding_span,
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
                    payload: convert_choice_payload(&variant.payload),
                    span: variant.span,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn convert_choice_payload(payload: &ChoiceVariantPayloadDefinition) -> ChoiceVariantPayload {
    match payload {
        ChoiceVariantPayloadDefinition::Unit => ChoiceVariantPayload::Unit,
        ChoiceVariantPayloadDefinition::Record { fields } => ChoiceVariantPayload::Record {
            fields: fields
                .iter()
                .map(|field| Declaration {
                    id: field.name,
                    type_id: field.type_id,
                    type_syntax: None,
                    value_mode: ValueMode::ImmutableOwned,
                    value: DeclarationValue::Absent,
                    binding_span: field.span,
                    config_qualifier: None,
                })
                .collect(),
        },
    }
}
