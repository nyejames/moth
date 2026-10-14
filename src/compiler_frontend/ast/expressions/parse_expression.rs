//! AST expression parsing and expression-list helpers.
//!
//! Owns ordinary reduction followed by receiving-boundary recovery selection. Nested parses
//! retain producer facts but do not gain a catch boundary; the root rejects typed producers
//! still unhandled after its complete expression is known.

use super::call_arguments::ParsedCallArguments;
use super::error::{ExpressionParseError, NumericDeferral};
use super::eval_expression::{evaluate_expression, extract_expression_span};
use super::expression_rpn::{ExpressionRpnItem, single_operand};
use super::expression_store::{AstExpressionBuilder, ExprId, ExpressionView, OwnedExpr};
use super::failure_facts::{ExpressionFailureFacts, FailureDisposition, TypedErrorProducer};
use super::parse_expression_dispatch::{
    ExpressionDispatchState, ExpressionTokenStep, GroupFrame, close_innermost_group,
    dispatch_expression_token,
};
use super::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, ExpressionTrailingPolicy,
};
use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::field_access::{
    DeferredCallReceiver, GenericSuccessSlots, RetainedOperandContinuation, RetainedPostfixStep,
    RetainedTerminalCall, retained_operand_numeric_origin_shape, retained_operand_result_shape,
    retained_terminal_call,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::GenericErrorReturn;
use crate::compiler_frontend::ast::numeric_construction::{
    ConstructionResult, DeferredCall, DeferredCallArgument, DeferredCallHandling,
    DeferredExpression, DeferredExpressionSuffix, DeferredNumericExpression, TypedErrorFact,
    deferred_numeric_expression, register_direct_reference_witness, retained_known_shape,
    single_direct_pending_reference,
};
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShapeId, register_shape_witness};
use crate::compiler_frontend::ast::statements::fallible_handling::{
    EXPRESSION_STAGE, FallibleSuccessContract, constrain_protected_by_fallbacks,
    parse_completed_expression_catch, parse_fallible_handling_suffix_for_expression,
    parse_fragment_handling_suffix, raw_protected_result_shape, retain_pending_catch_handler,
    token_stream_starts_typed_propagation_suffix,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticToken};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::type_coercion::parse_context::is_numeric_literal_destination_type_id;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::ExpressionBoundaryDepth;

// WHAT: shared expression parser entry with configurable trailing-newline behavior.
// WHY: callers parsing comma-separated lists outside parentheses (for example
//      fallback/return lists) must preserve line boundaries between statements.
pub(crate) fn create_expression_with_trailing_newline_policy(
    mut input: ExpressionParseInput<'_, '_, '_>,
) -> Result<OwnedExpr, ExpressionParseError> {
    if !input.scope_context.defer_typed_error_validation {
        let deferred_context = input
            .scope_context
            .new_pending_typed_error_expression_context();
        let mut deferred_input = ExpressionParseInput::new(
            ExpressionParseResources {
                token_stream: input.token_stream,
                scope_context: &deferred_context,
                type_interner: input.type_interner,
                expression_builder: &mut *input.expression_builder,
                const_values: &mut *input.const_values,
                expected_type: input.expected_type,
                cast_target_context: input.cast_target_context,
                value_mode: input.value_mode,
                string_table: input.string_table,
                path_fork: input.path_fork,
            },
            input.trailing_policy,
        );
        deferred_input.stop_at_named_entry = input.stop_at_named_entry;
        deferred_input.numeric_deferral_owner = input.numeric_deferral_owner;
        deferred_input.retain_pending_numeric_fragment = input.retain_pending_numeric_fragment;
        let expression = create_expression_with_trailing_newline_policy(deferred_input)?;
        validate_completed_typed_errors(input.expression_builder.view_owned(expression)?)?;
        return Ok(expression);
    }

    // Keep the authored terminator until recovery has selected its expression.
    let skip_trailing_newlines = input.trailing_policy.skip_trailing_newlines;
    input.trailing_policy.skip_trailing_newlines = false;
    let fragment = match collect_expression_fragment(&mut input)? {
        ExpressionFragment::Value(value) => {
            if skip_trailing_newlines {
                input.token_stream.skip_newlines();
            }
            return Ok(value);
        }
        ExpressionFragment::Nodes(nodes) => nodes,
    };
    finish_collected_fragment(input, fragment, skip_trailing_newlines)
}

fn finish_collected_fragment(
    mut input: ExpressionParseInput<'_, '_, '_>,
    nodes: Vec<ExpressionRpnItem>,
    skip_trailing_newlines: bool,
) -> Result<OwnedExpr, ExpressionParseError> {
    // A fragment containing origins that only body completion can resolve is retained instead of
    // evaluated. The position's own receiving context travels with it so completion evaluates the
    // exact authored syntax at the resolved destination. An eligible origin boundary additionally
    // retains literal-only recipes, which a later exact receiver may still type.
    let retained = if input.retain_pending_numeric_fragment {
        crate::compiler_frontend::ast::numeric_construction::items_contain_pending(&nodes)
    } else {
        crate::compiler_frontend::ast::numeric_construction::items_contain_unresolvable_numeric(
            &nodes,
        )
    };
    if retained {
        // A direct use of one pending origin at an exact receiving boundary is that origin's
        // receiving constraint. A mention inside a computation is not: the boundary reaches only
        // the literal leaves, and the named operands stay forward-only.
        register_retained_fragment_witness(&input, &nodes)?;
        // An authored `catch` protects the fragment's whole value. The value does not exist where
        // the catch is authored, so the handler travels with the fragment and the body root
        // rebuilds the pair through the owning catch constructor.
        if input.token_stream.current_tag() == TokenTag::CATCH {
            return Err(retain_pending_fragment_catch(&mut input, nodes)?);
        }
        let suffix = consume_deferred_expression_suffix(input.token_stream)?;
        let span = extract_expression_span(input.expression_builder, &nodes)?;
        return Err(ExpressionParseError::NumericDeferral(Box::new(
            NumericDeferral {
                items: nodes,
                expected: *input.expected_type,
                value_mode: input.value_mode.to_owned(),
                span,
                suffix,
                complete_fragment: input.numeric_deferral_owner,
            },
        )));
    }

    let expression = evaluate_expression(
        input.scope_context,
        nodes,
        input.type_interner,
        input.expression_builder,
        input.const_values,
        input.expected_type,
        input.value_mode,
        input.string_table,
        input.path_fork,
    )?;

    let expression = complete_pending_expression(
        input.token_stream,
        input.scope_context,
        input.type_interner,
        input.expression_builder,
        input.const_values,
        expression,
        input.trailing_policy.allow_boundary_catch,
        input.string_table,
        input.path_fork,
    )?;

    if skip_trailing_newlines {
        input.token_stream.skip_newlines();
    }
    Ok(expression)
}

/// One already parsed prefix handed back into the shared expression parser.
///
/// WHAT: a completed subject contributes its retained ID, while a numeric-dependent subject
///       contributes its ordered transient fragment exactly as authored.
/// WHY: a tentative predicate must not be reparsed or discarded. A retained fragment's numeric
///      origins, forward-producer tickets and generic requests stay issued once, and its receiving
///      context travels with it into the resumed condition.
pub(crate) enum ExpressionParseSeed {
    /// A committed value reused by its identity, without rewalking the tokens that built it.
    Completed(OwnedExpr),
    /// A numeric-dependent fragment resumed before its body-root selection.
    Fragment(DeferredNumericExpression),
}

/// Bounded seeded resume: parse the remaining tokens in `[start, stop)` after a scrutinee prefix.
///
/// WHAT: locates the stop-token window exactly like `create_expression_until`, narrows the cursor
///       to it, pre-appends the seed (completed ID as one operand, or retained fragment items in
///       order), then collects/evaluates the combined fragment through the shared pipeline.
/// WHY: ordinary single-predicate subjects resume the central parser after classification, so the
///      subject parse registers forward producers and generic requests exactly once. The bounded
///      window keeps the `THEN`/`COLON` terminator for the receiver grammar.
pub(crate) fn create_expression_until_with_seed(
    input: ExpressionParseInput<'_, '_, '_>,
    stop_tokens: &[TokenTag],
    seed: ExpressionParseSeed,
) -> Result<OwnedExpr, ExpressionParseError> {
    let (seed_items, seed_expected, seed_value_mode) = match seed {
        ExpressionParseSeed::Completed(id) => {
            input.expression_builder.view_owned(id)?;
            (vec![ExpressionRpnItem::Operand(id.id())], None, None)
        }
        ExpressionParseSeed::Fragment(fragment) => {
            let DeferredNumericExpression {
                items,
                expected,
                value_mode,
                ..
            } = fragment;
            (items, Some(expected), Some(value_mode))
        }
    };
    if let Some(expected) = seed_expected {
        *input.expected_type = expected;
    }
    // The scrutinee and its resumed condition share one value mode (both parse under
    // `ImmutableOwned`); the seed carries it so a future divergence fails loudly here.
    if let Some(mode) = seed_value_mode {
        debug_assert_eq!(
            *input.value_mode, mode,
            "seeded resume value mode must match caller"
        );
    }
    create_expression_until_with_seeded_items(input, stop_tokens, seed_items)
}

fn create_expression_until_with_seeded_items(
    input: ExpressionParseInput<'_, '_, '_>,
    stop_tokens: &[TokenTag],
    seed_items: Vec<ExpressionRpnItem>,
) -> Result<OwnedExpr, ExpressionParseError> {
    if !input.scope_context.defer_typed_error_validation {
        let deferred_context = input
            .scope_context
            .new_pending_typed_error_expression_context();
        let deferred_input = ExpressionParseInput::new(
            ExpressionParseResources {
                token_stream: input.token_stream,
                scope_context: &deferred_context,
                type_interner: input.type_interner,
                expression_builder: &mut *input.expression_builder,
                const_values: &mut *input.const_values,
                expected_type: input.expected_type,
                cast_target_context: input.cast_target_context,
                value_mode: input.value_mode,
                string_table: input.string_table,
                path_fork: input.path_fork,
            },
            input.trailing_policy,
        );
        // Flag state travels explicitly: the deferred input starts clean.
        let mut deferred_input = deferred_input;
        deferred_input.stop_at_named_entry = input.stop_at_named_entry;
        deferred_input.numeric_deferral_owner = input.numeric_deferral_owner;
        deferred_input.retain_pending_numeric_fragment = input.retain_pending_numeric_fragment;
        let expression = create_expression_until_with_seeded_items_inner(
            deferred_input,
            stop_tokens,
            seed_items,
        )?;
        validate_completed_typed_errors(input.expression_builder.view_owned(expression)?)?;
        return Ok(expression);
    }
    create_expression_until_with_seeded_items_inner(input, stop_tokens, seed_items)
}

fn create_expression_until_with_seeded_items_inner(
    input: ExpressionParseInput<'_, '_, '_>,
    stop_tokens: &[TokenTag],
    seed_items: Vec<ExpressionRpnItem>,
) -> Result<OwnedExpr, ExpressionParseError> {
    // Empty stop list keeps the unbounded trailing policy with the seed prefix.
    if stop_tokens.is_empty() {
        let skip_trailing_newlines = input.trailing_policy.skip_trailing_newlines;
        let mut input = input;
        input.trailing_policy.skip_trailing_newlines = false;
        let mut collected = seed_items;
        collect_expression_fragment_into(&mut input, &mut collected)?;
        return finish_collected_fragment(input, collected, skip_trailing_newlines);
    }
    // Reuse the bounded window scan: locate the stop token, narrow the cursor, collect the
    // remainder onto the seed, then restore the cursor onto the stop token.
    let start_index = input.token_stream.position();
    let mut depth = ExpressionBoundaryDepth::default();
    while !input.token_stream.is_at_end() {
        if depth.is_top_level() && stop_tokens.contains(&input.token_stream.current_tag()) {
            break;
        }
        depth.step_tag(input.token_stream.current_tag());
        if input.token_stream.current_tag() == TokenTag::EOF {
            break;
        }
        input.token_stream.advance();
    }
    let end_index = input.token_stream.position();
    let end_tag =
        (end_index < input.token_stream.length()).then(|| input.token_stream.current_tag());
    let end_span = input.token_stream.span_at(end_index);
    input
        .token_stream
        .set_position(start_index)
        .expect("seeded expression boundary resume stays inside the active parser view");
    if end_index >= input.token_stream.length() {
        let expected_tokens = stop_tokens
            .iter()
            .map(|token| format!("{token:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let expected_delimiter = input.string_table.intern(&expected_tokens);
        return Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(expected_delimiter),
            Some(input.token_stream.current_span()),
        )
        .into());
    }
    let Some(end_tag) = end_tag else {
        let expected_tokens = stop_tokens
            .iter()
            .map(|token| format!("{token:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let expected_delimiter = input.string_table.intern(&expected_tokens);
        return Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(expected_delimiter),
            Some(input.token_stream.current_span()),
        )
        .into());
    };
    if !stop_tokens.contains(&end_tag) {
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(end_tag),
            end_span,
        )
        .into());
    }
    // An empty remainder means the scrutinee already spans the whole condition (e.g. a bare
    // `is`-less subject is impossible here — the seed stopped at `is` — but a defensive empty
    // window must not fabricate an empty fragment).
    let previous_limit = input.token_stream.set_limit(end_index)?;
    let mut inner_input = ExpressionParseInput::new(
        ExpressionParseResources {
            token_stream: input.token_stream,
            scope_context: input.scope_context,
            type_interner: input.type_interner,
            expression_builder: input.expression_builder,
            const_values: input.const_values,
            expected_type: input.expected_type,
            cast_target_context: input.cast_target_context,
            value_mode: input.value_mode,
            string_table: input.string_table,
            path_fork: input.path_fork,
        },
        ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: false,
            allow_boundary_catch: input.trailing_policy.allow_boundary_catch,
            allow_expected_result_evidence: input.trailing_policy.allow_expected_result_evidence,
        },
    );
    inner_input.numeric_deferral_owner = input.numeric_deferral_owner;
    inner_input.retain_pending_numeric_fragment = input.retain_pending_numeric_fragment;
    inner_input.stop_at_named_entry = input.stop_at_named_entry;
    let mut collected = seed_items;
    let early = collect_expression_fragment_into(&mut inner_input, &mut collected)?;
    // A lone `is:` guard evaluates the seeded scrutinee alone and returns it; the receiver
    // grammar still owns the stop token.
    if let Some(value) = early {
        input.token_stream.restore_limit(previous_limit);
        input.token_stream.set_position(end_index)?;
        return Ok(value);
    }
    let result = finish_collected_fragment(inner_input, collected, true);
    input.token_stream.restore_limit(previous_limit);
    input.token_stream.set_position(end_index)?;
    result
}

/// Register a retained fragment's exact receiving constraint on the origin it uses directly.
///
/// WHAT: the position's exact literal destination is the receiver. A position with no literal
///       destination registers nothing here; a non-numeric known parameter is registered by its
///       call argument owner through the same rule.
/// WHY: a direct use constrains its origin before body-root defaults run, and a computed fragment
///      never constrains its operands backwards.
fn register_retained_fragment_witness(
    input: &ExpressionParseInput<'_, '_, '_>,
    nodes: &[ExpressionRpnItem],
) -> ConstructionResult<()> {
    let Some(expected_type_id) = input.expected_type.literal_destination_type_id() else {
        return Ok(());
    };
    register_direct_receiver_witness(
        input.scope_context,
        input.type_interner.environment(),
        input.expression_builder,
        nodes,
        expected_type_id,
        false,
    )
}

/// Register the exact receiver a retained fragment's origin receives.
///
/// WHAT: a fragment that is exactly one pending binding reference registers its receiver on that
///       binding, through its known shape or its scalar class. A fragment that is exactly one
///       retained literal registers its receiver through the literal's known shape. A fragment that
///       is exactly one retained operand with a known result shape (a projection, a field, a method
///       result, or a catch-protected value) registers through that shape. A numeric receiver
///       registers only through a numeric origin shape. Any other fragment, including named
///       arithmetic, registers nothing.
/// WHY: a receiving boundary is the single static type source for its origin's numeric leaves, so
///      it must register before body-root defaults. `include_unhandled_success` says whether the
///      operand's unhandled fallible step contributes its success shape. A call argument sets it,
///      because its carrier leaves through the call's own error lane. This selects success metadata
///      only; ordinary typed validation still owns the legality of the unhandled carrier.
pub(crate) fn register_direct_receiver_witness(
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
    items: &[ExpressionRpnItem],
    receiver_type_id: TypeId,
    include_unhandled_success: bool,
) -> ConstructionResult<()> {
    if let Some(reference) = single_direct_pending_reference(items) {
        let mut state = scope_context.numeric_state_mut();
        register_direct_reference_witness(&mut state, environment, &reference, receiver_type_id);
        return Ok(());
    }
    // The operand shape is derived before the state borrow: a concrete result interns into that
    // state, so deriving it while the borrow is held would double-borrow.
    let shape = match retained_known_shape(items) {
        Some(shape) => Some(shape),
        None => receiver_operand_shape(
            scope_context,
            environment,
            expression_builder,
            items,
            receiver_type_id,
            include_unhandled_success,
        )?,
    };
    if let Some(shape) = shape {
        let span = match items.first() {
            Some(item) => item.source_span(expression_builder)?,
            None => None,
        };
        let mut state = scope_context.numeric_state_mut();
        register_shape_witness(&mut state, environment, shape, receiver_type_id, span);
    }
    Ok(())
}

/// The operand shape a retained single-operand fragment constrains its receiver through.
///
/// WHAT: only a single operand with a known result shape qualifies. A non-numeric receiver takes
///       any such shape; a numeric receiver takes only a numeric origin shape, which names its
///       origin class and never a concrete result.
/// WHY: operand shapes constrain aggregate receivers, and a numeric origin's class is the only
///      shape a scalar destination can use. The scalar leaves of a computed operand stay
///      forward-only.
fn receiver_operand_shape(
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
    items: &[ExpressionRpnItem],
    receiver_type_id: TypeId,
    include_unhandled_success: bool,
) -> ConstructionResult<Option<KnownShapeId>> {
    let Some(item) = single_operand(items) else {
        return Ok(None);
    };
    if is_numeric_literal_destination_type_id(receiver_type_id, environment) {
        return Ok(retained_operand_numeric_origin_shape(
            item,
            scope_context,
            environment,
            receiver_type_id,
            include_unhandled_success,
            expression_builder,
        ));
    }
    retained_operand_result_shape(
        item,
        scope_context,
        environment,
        include_unhandled_success,
        expression_builder,
    )
}

/// Retain one deferred fragment whose authored `catch` suffix protects its whole value.
///
/// WHAT: parses the handler through the shared fallible-handling suffix entry, pairs it with the
///       fragment as the protected value, and raises the deferral whose single item is the catch
///       attachment rebuilt at the body root.
/// WHY: the fragment's value does not exist where the catch is authored, so the handler cannot
///      attach yet; it rebuilds through the existing catch constructor, using the same site facts
///      the completed path derives — the trailing fallible pending call's signature when there is
///      one, and the builtin Error boundary the completed path synthesises for a value that is
///      infallible by authored shape otherwise.
fn retain_pending_fragment_catch(
    input: &mut ExpressionParseInput<'_, '_, '_>,
    items: Vec<ExpressionRpnItem>,
) -> Result<ExpressionParseError, ExpressionParseError> {
    let catch_span = Some(input.token_stream.current_span());
    let fragment_span = extract_expression_span(input.expression_builder, &items)?;
    let (fragment_success, error_return) =
        fragment_fallible_site(input.expression_builder, &items, input.scope_context)?;
    // An authored destination is the protected value's exact type, so it is the immediate receiving
    // type of the handler's single slot. It applies to a fragment with no resolved contract and to a
    // single-slot generic call whose arity is fixed while its slot types resolve at completion; a
    // later consumer of the value never supplies it.
    let destination_type_id = match &fragment_success {
        FragmentSuccess::Unresolved | FragmentSuccess::Pending { slot_count: 1 } => {
            input.expected_type.literal_destination_type_id()
        }
        _ => None,
    };
    let success_contract = match destination_type_id.as_ref() {
        Some(type_id) => FallibleSuccessContract::Known(std::slice::from_ref(type_id)),
        None => fragment_success.contract(),
    };
    // The authored catch is recovery for the assignment (or mutation) whose right-hand side this
    // fragment belongs to: the ordinary `catch` entry activates the pending target names before
    // parsing the handler, so the retained suffix entry must see the same context or a read of the
    // target inside the recovery is accepted instead of rejected.
    let catch_context = input
        .scope_context
        .activate_pending_catch_assignment_targets();
    let suffix = parse_fragment_handling_suffix(
        input.token_stream,
        &catch_context,
        input.type_interner,
        input.expression_builder,
        input.const_values,
        success_contract,
        error_return,
        true,
        true,
        fragment_span.or(catch_span),
        EXPRESSION_STAGE,
        input.trailing_policy.allow_boundary_catch,
        None,
        input.string_table,
        input.path_fork,
    )?;
    let Some(suffix) = suffix else {
        return Err(CompilerError::compiler_error(
            "a retained fragment's authored `catch` suffix was classified as absent",
        )
        .into());
    };
    let protected_shape = raw_protected_result_shape(
        &items,
        *input.expected_type,
        input.scope_context,
        input.type_interner.environment_mut_for_derived_types(),
        input.expression_builder,
    )?;
    let handled = deferred_numeric_expression(
        items,
        *input.expected_type,
        input.value_mode.to_owned(),
        fragment_span,
        DeferredExpressionSuffix::None,
    );
    let Some(payload) = retain_pending_catch_handler(handled, protected_shape, suffix) else {
        return Err(CompilerError::compiler_error(
            "a parsed `catch` suffix was classified as postfix propagation",
        )
        .into());
    };
    // The protected value's catch fallbacks join its numeric shape now, before body-root resolution
    // defaults the origins they share: a raw literal's class, or a retained generic call's result
    // slots.
    constrain_protected_by_fallbacks(
        &payload,
        &mut input.scope_context.numeric_state_mut(),
        input.type_interner.environment(),
        input.expression_builder,
        input.const_values,
    );
    Ok(ExpressionParseError::NumericDeferral(Box::new(
        NumericDeferral {
            items: vec![ExpressionRpnItem::PendingNumericExpression {
                expression: Box::new(DeferredExpression::CatchHandler(Box::new(payload))),
                span: catch_span,
            }],
            expected: *input.expected_type,
            value_mode: input.value_mode.to_owned(),
            span: fragment_span.or(catch_span),
            suffix: DeferredExpressionSuffix::None,
            complete_fragment: input.numeric_deferral_owner,
        },
    )))
}

/// The owned success contract of a retained fragment, read back through `contract` for the
/// suffix parse.
enum FragmentSuccess {
    Known(Vec<TypeId>),
    /// One concrete slot of a generic call, kept inline.
    Single(TypeId),
    /// A generic call whose slot types all resolve at completion, at its declared arity.
    Pending {
        slot_count: usize,
    },
    /// Per-slot types of a generic call with concrete and unresolved slots: `Some` for a concrete
    /// declared slot and `None` for a slot whose type resolves at completion. Never all `Some`
    /// (`Known`) or all `None` (`Pending`).
    Partial(Vec<Option<TypeId>>),
    Unresolved,
}

impl FragmentSuccess {
    fn contract(&self) -> FallibleSuccessContract<'_> {
        match self {
            Self::Known(types) => FallibleSuccessContract::Known(types.as_slice()),
            Self::Single(type_id) => FallibleSuccessContract::Known(std::slice::from_ref(type_id)),
            Self::Pending { slot_count } => FallibleSuccessContract::Pending {
                slot_count: *slot_count,
            },
            Self::Partial(slots) => FallibleSuccessContract::Partial(slots.as_slice()),
            Self::Unresolved => FallibleSuccessContract::Unresolved,
        }
    }
}

/// The fallible contract the shared handler parser needs for one retained fragment.
///
/// WHAT: when the fragment is exactly one retained terminal call (groups transparent), that
///       call's own signature supplies the success contract: `Known` for a concrete signature
///       (possibly empty for zero success slots). A generic call classifies its slots: `Known` once
///       all are concrete, `Pending` while all resolve at completion, `Partial` when some are
///       concrete. A chained method call is a
///       call too, once its operand and suffix are valid. Any other retained fragment has no
///       resolved contract yet (`Unresolved`, never zero success values). The error type is the
///       fragment's first typed producer in authored order, whether or not the terminal call
///       declares one; `None` leaves the builtin Error boundary to the shared parser.
/// WHY: the fragment is still infix at this boundary, so its result slots only exist after
///      evaluation; a trailing-operand read would give `f(pending) + 10` an empty contract and
///      `1 / f(pending)` the call's I32 instead of the fragment's F64 result. Success arity is
///      independent of the typed error, so an error-free `pair(1 + 2)` keeps its declared slots.
fn fragment_fallible_site(
    expression_builder: &AstExpressionBuilder,
    items: &[ExpressionRpnItem],
    scope_context: &ScopeContext,
) -> Result<(FragmentSuccess, Option<GenericErrorReturn>), CompilerError> {
    let success = match single_operand(items) {
        Some(item) => match retained_terminal_call(item) {
            Some(terminal) => terminal_call_success(terminal, scope_context),
            None => FragmentSuccess::Unresolved,
        },
        None => FragmentSuccess::Unresolved,
    };
    Ok((success, fragment_error_site(expression_builder, items)?))
}

/// The success contract of one retained terminal call, from its resolved or declared facts.
///
/// WHAT: a concrete signature gives its slot types, a declared-empty call is known to be empty, and
///       a generic call reads each declared slot whose type is already concrete, leaving the rest
///       unknown until completion.
/// WHY: a generic call's success slots resolve at completion, but a concrete declared slot of
///      the same signature is already a fixed receiving type. Reading only those concrete facts
///      keeps the arity and the concrete slots without resolving any generic parameter early.
fn terminal_call_success(
    call: RetainedTerminalCall<'_>,
    scope_context: &ScopeContext,
) -> FragmentSuccess {
    if let Some(types) = call.success_return_type_ids() {
        return FragmentSuccess::Known(types);
    }
    match call.generic_success_slots(scope_context) {
        GenericSuccessSlots::Single(type_id) => FragmentSuccess::Single(type_id),
        GenericSuccessSlots::Concrete(types) => FragmentSuccess::Known(types),
        GenericSuccessSlots::Unknown(slot_count) => FragmentSuccess::Pending { slot_count },
        GenericSuccessSlots::Mixed(slots) => FragmentSuccess::Partial(slots),
    }
}

/// The error slot of a larger retained fragment, from its own fallible calls.
///
/// WHAT: the first typed producer's error return in authored order, taken from the fragment's own
///       retained call slots and retained child syntax plus the already-built failure summaries of
///       Ready arguments and Ready receivers; `None` when the fragment carries no typed producer, so
///       the shared parser's boundary error stays the owner of that synthesis. A numeric-dependent
///       producer stays `Retained` until its shape completes. A second disagreeing producer keeps
///       the first here and is diagnosed by the ordinary compatibility owner at completion.
/// WHY: a Ready argument's `failure_facts` already carry its typed producer, and the completed
///      call constructor merges exactly those summaries; reading them keeps the parse-time
///      contract identical to the completed one, so `outer(g(), pending) catch |err| ...` binds
///      g's custom error instead of a fabricated boundary type.
fn fragment_error_site(
    expression_builder: &AstExpressionBuilder,
    items: &[ExpressionRpnItem],
) -> Result<Option<GenericErrorReturn>, CompilerError> {
    Ok(fragment_typed_error_summary(expression_builder, items)?
        .0
        .map(|fact| match fact {
            TypedErrorFact::Completed(producer) => {
                GenericErrorReturn::Concrete(producer.error_type_id)
            }
            TypedErrorFact::Pending { error_return, .. } => {
                GenericErrorReturn::Retained(error_return)
            }
        }))
}

/// The first and first-disagreeing typed producers of one retained fragment.
///
/// WHAT: the ordered walk `fragment_error_site` reads, returning both producers instead of only the
///       first error. A numeric-dependent producer is `Pending` with its error shape.
/// WHY: a deferred call argument stores this summary when it registers as a forward producer, so
///      its enclosing call replays the summary without the fragment's items.
pub(crate) fn fragment_typed_error_summary(
    expression_builder: &AstExpressionBuilder,
    items: &[ExpressionRpnItem],
) -> Result<(Option<TypedErrorFact>, Option<TypedErrorFact>), CompilerError> {
    /// Two producers share one error only when both are concrete and equal, or both retain the
    /// same shape. Any other pair stays a disagreement: a pending producer's type is not known
    /// until its shape completes, and the completion owner decides whether it really conflicts.
    fn known_same_error(left: TypedErrorFact, right: TypedErrorFact) -> bool {
        match (left, right) {
            (TypedErrorFact::Completed(left), TypedErrorFact::Completed(right)) => {
                left.error_type_id == right.error_type_id
            }
            (
                TypedErrorFact::Pending {
                    error_return: left, ..
                },
                TypedErrorFact::Pending {
                    error_return: right,
                    ..
                },
            ) => left == right,
            _ => false,
        }
    }

    fn record(
        fact: TypedErrorFact,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) {
        match first {
            None => *first = Some(fact),
            Some(existing) if !known_same_error(*existing, fact) && conflicting.is_none() => {
                *conflicting = Some(fact);
            }
            _ => {}
        }
    }

    /// Merge one already-built expression's pending producers; a handled child contributes none.
    fn collect_facts(
        facts: &ExpressionFailureFacts,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) {
        if facts.disposition != FailureDisposition::Pending {
            return;
        }
        if let Some(producer) = facts.summary.first_typed {
            record(TypedErrorFact::Completed(producer), first, conflicting);
        }
        if let Some(producer) = facts.summary.conflicting_typed {
            record(TypedErrorFact::Completed(producer), first, conflicting);
        }
    }

    fn collect_expression_facts(
        expression_builder: &AstExpressionBuilder,
        expression: ExprId,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) -> Result<(), CompilerError> {
        if let Some(facts) = expression_builder.view(expression)?.failure_facts() {
            collect_facts(facts, first, conflicting);
        }
        Ok(())
    }

    fn collect(
        expression_builder: &AstExpressionBuilder,
        items: &[ExpressionRpnItem],
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) -> Result<(), CompilerError> {
        for item in items {
            match item {
                ExpressionRpnItem::PendingNumericCall { call, .. } => {
                    collect_call(expression_builder, call, first, conflicting)?;
                }
                // A completed sibling operand already carries the failure summary the completed
                // call constructor merges, so an already-built typed producer must reach the
                // fragment's handler contract; a handled one contributes none.
                ExpressionRpnItem::Operand(expression) => {
                    collect_expression_facts(expression_builder, *expression, first, conflicting)?;
                }
                // A retained cast carries its own source fragment, so a fallible call inside the
                // cast supplies its error to the enclosing fragment's handler contract.
                ExpressionRpnItem::PendingNumericCast { source, .. } => {
                    collect(expression_builder, source, first, conflicting)?;
                }
                // A retained operand continuation keeps its operand and postfix steps, so a
                // pending method or builtin producer reaches the handler contract as the completed
                // chain exposes it. Other deferred expression kinds are not walked here.
                ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
                    if let DeferredExpression::OperandContinuation(continuation) =
                        expression.as_ref()
                    {
                        collect_operand_continuation(
                            expression_builder,
                            continuation,
                            first,
                            conflicting,
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Walk one retained operand continuation in authored order: its operand first, then each
    /// postfix step. A method or builtin step owns a producer only while its authored handling is
    /// unhandled, matching the completed step; field steps are never fallible. Each step's
    /// arguments are walked before its own producer, matching the order its completed call is
    /// built in.
    fn collect_operand_continuation(
        expression_builder: &AstExpressionBuilder,
        continuation: &RetainedOperandContinuation,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) -> Result<(), CompilerError> {
        collect(
            expression_builder,
            &continuation.operand,
            first,
            conflicting,
        )?;
        for step in &continuation.steps {
            match step {
                // The chained receiver is the preceding step's completed value, so only the
                // call's arguments and its own producer are new facts here.
                RetainedPostfixStep::Method(call) => {
                    collect_deferred_arguments(
                        expression_builder,
                        &call.arguments,
                        first,
                        conflicting,
                    )?;
                    if matches!(call.authored_handling, DeferredCallHandling::Unhandled) {
                        collect_error_return(
                            call.error_return(),
                            call.call_span,
                            first,
                            conflicting,
                        );
                    }
                }
                RetainedPostfixStep::Builtin(builtin) => {
                    match &builtin.arguments {
                        ParsedCallArguments::Ready(arguments) => {
                            for argument in arguments {
                                collect_expression_facts(
                                    expression_builder,
                                    argument.value,
                                    first,
                                    conflicting,
                                )?;
                            }
                        }
                        ParsedCallArguments::Retained(arguments) => {
                            collect_deferred_arguments(
                                expression_builder,
                                arguments,
                                first,
                                conflicting,
                            )?;
                        }
                    }
                    if let Some(error_type_id) = builtin.error_return_type_id
                        && matches!(builtin.authored_handling, DeferredCallHandling::Unhandled)
                    {
                        record(
                            TypedErrorFact::Completed(TypedErrorProducer {
                                span: builtin.member_span,
                                error_type_id,
                                builtin_name: Some(builtin.member_name),
                            }),
                            first,
                            conflicting,
                        );
                    }
                }
                RetainedPostfixStep::Field(_) | RetainedPostfixStep::DeclaredField(_) => {}
            }
        }
        Ok(())
    }

    /// Record one call's own producer: a concrete error is a completed producer, a retained one
    /// stays pending until its shape completes.
    fn collect_error_return(
        error_return: Option<GenericErrorReturn>,
        span: Option<SourceSpan>,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) {
        if let Some(error_return) = error_return {
            let fact = match error_return {
                GenericErrorReturn::Concrete(error_type_id) => {
                    TypedErrorFact::Completed(TypedErrorProducer {
                        span,
                        error_type_id,
                        builtin_name: None,
                    })
                }
                GenericErrorReturn::Retained(error_return) => {
                    TypedErrorFact::Pending { span, error_return }
                }
            };
            record(fact, first, conflicting);
        }
    }

    /// Walk a call's argument list: ready arguments contribute their facts, retained arguments
    /// their fragment items, and forwarded arguments the summary captured at registration.
    fn collect_deferred_arguments(
        expression_builder: &AstExpressionBuilder,
        arguments: &[DeferredCallArgument],
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) -> Result<(), CompilerError> {
        for argument in arguments {
            match argument {
                DeferredCallArgument::Ready(ready) => {
                    collect_expression_facts(expression_builder, ready.value, first, conflicting)?;
                }
                DeferredCallArgument::Retained { expression, .. } => {
                    collect(expression_builder, &expression.items, first, conflicting)?;
                }
                // A registered forward argument keeps its fragment items in the registry, so its
                // call replays the summary the fragment computed at registration.
                DeferredCallArgument::Forward { typed_errors, .. } => {
                    for fact in [typed_errors.0, typed_errors.1].into_iter().flatten() {
                        record(fact, first, conflicting);
                    }
                }
            }
        }
        Ok(())
    }

    fn collect_call(
        expression_builder: &AstExpressionBuilder,
        call: &DeferredCall,
        first: &mut Option<TypedErrorFact>,
        conflicting: &mut Option<TypedErrorFact>,
    ) -> Result<(), CompilerError> {
        collect_error_return(call.error_return(), call.call_span(), first, conflicting);
        collect_deferred_arguments(
            expression_builder,
            deferred_call_arguments(call),
            first,
            conflicting,
        )?;
        if let DeferredCall::Receiver(deferred) = call
            && let DeferredCallReceiver::Ready(receiver) = &deferred.receiver
        {
            collect_expression_facts(expression_builder, *receiver, first, conflicting)?;
        }
        Ok(())
    }

    fn deferred_call_arguments(call: &DeferredCall) -> &[DeferredCallArgument] {
        match call {
            DeferredCall::Source(deferred) => &deferred.arguments,
            DeferredCall::Receiver(deferred) => &deferred.arguments,
            DeferredCall::External(deferred) => &deferred.arguments,
            DeferredCall::Struct(deferred) => &deferred.arguments,
            DeferredCall::Choice(deferred) => &deferred.arguments,
            DeferredCall::Generic(deferred) => &deferred.arguments,
            DeferredCall::GenericNominal(deferred) => &deferred.arguments,
        }
    }

    let mut first = None;
    let mut conflicting = None;
    collect(expression_builder, items, &mut first, &mut conflicting)?;
    Ok((first, conflicting))
}

/// Consume the trailing handling that a deferred expression can carry.
///
/// WHAT: an authored postfix `!` is recorded on the retained expression; any other trailing
///       recovery (`catch`, `?`) needs the resolved value's result shape, so it is reported as a
///       precise unsupported position instead of being silently dropped.
/// WHY: numeric resolution happens at the body root, after the authored suffix tokens have been
///      consumed, so every suffix the resumed expression must apply has to be retained now.
fn consume_deferred_expression_suffix(
    token_stream: &mut AstCursor<'_>,
) -> Result<DeferredExpressionSuffix, ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::BANG => {
            let span = Some(token_stream.current_span());
            token_stream.advance();
            Ok(DeferredExpressionSuffix::Propagate { span })
        }
        TokenTag::CATCH | TokenTag::QUESTION_MARK => Err(ExpressionParseError::Infrastructure(
            Box::new(CompilerError::compiler_error(
                "numeric-dependent expression followed by catch/option propagation cannot be \
                     retained yet; the suffix needs the resolved result shape",
            )),
        )),
        _ => Ok(DeferredExpressionSuffix::None),
    }
}

/// Finish an existing expression at a receiving site without reparsing its primary.
///
/// Place parsing must first distinguish mutation from a call statement. Its non-mutation path
/// rejoins this owner so postfix propagation and whole-expression recovery have one meaning.
#[allow(
    clippy::too_many_arguments,
    reason = "completion keeps parser resources and the already-typed value separate"
)]
pub(crate) fn complete_pending_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    mut expression: OwnedExpr,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    if token_stream_starts_typed_propagation_suffix(token_stream) {
        let value_required = !expression_builder
            .view_owned(expression)?
            .success_types()
            .is_empty();
        expression = parse_fallible_handling_suffix_for_expression(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            expression,
            value_required,
            allow_boundary_catch,
            string_table,
            path_fork,
        )?;
    }
    if token_stream.current_tag() == TokenTag::CATCH {
        expression = parse_completed_expression_catch(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            expression,
            allow_boundary_catch,
            string_table,
            path_fork,
        )?;
    }
    if !context.defer_typed_error_validation {
        validate_completed_typed_errors(expression_builder.view_owned(expression)?)?;
    }
    Ok(expression)
}

pub(crate) fn validate_completed_typed_errors(
    expression: ExpressionView<'_>,
) -> Result<(), ExpressionParseError> {
    if let Some(diagnostic) = expression.unhandled_typed_error_diagnostic() {
        return Err(diagnostic.into());
    }
    Ok(())
}

/// One parsed expression before evaluation.
pub(super) enum ExpressionFragment {
    /// An infix fragment, including unresolved groups, still awaiting ordering, typing and folding.
    Nodes(Vec<ExpressionRpnItem>),
    /// A value returned early by a template or `is:` guard; it never reaches evaluation.
    Value(OwnedExpr),
}

// WHAT: collects the infix AST fragment without evaluating it.
// WHY: ordinary parentheses retain unresolved syntax so receiving and peer contexts can reach
//      literal leaves before ordering, operator typing and folding.
fn collect_expression_fragment(
    input: &mut ExpressionParseInput<'_, '_, '_>,
) -> Result<ExpressionFragment, ExpressionParseError> {
    let mut expression: Vec<ExpressionRpnItem> = Vec::new();
    match collect_expression_fragment_into(input, &mut expression)? {
        Some(value) => Ok(ExpressionFragment::Value(value)),
        None => Ok(ExpressionFragment::Nodes(expression)),
    }
}

fn collect_expression_fragment_into(
    input: &mut ExpressionParseInput<'_, '_, '_>,
    expression: &mut Vec<ExpressionRpnItem>,
) -> Result<Option<OwnedExpr>, ExpressionParseError> {
    ast_log!(
        "Parsing ",
        #input.value_mode,
        format!("{:?}", input.expected_type),
        " Expression"
    );

    // Collect the infix AST fragment as one flat stream. Each `(` pushes a `GroupOpen` marker and a
    // frame, and the matching `)` closes the innermost frame in place. `evaluate_expression` orders
    // deferred groups into one RPN stream, resolves the final type, and folds constants.
    let root_allow_boundary_catch = input.trailing_policy.allow_boundary_catch;
    let mut next_number_negative = false;
    let mut group_frames: Vec<GroupFrame> = Vec::new();
    while input.token_stream.position() < input.token_stream.length() {
        let token = input.token_stream.current_tag();
        let in_group = !group_frames.is_empty();
        if !in_group
            && input.stop_at_named_entry
            && expression
                .last()
                .is_some_and(ExpressionRpnItem::is_operand_shape)
            && token == TokenTag::SYMBOL
            && input.token_stream.peek_next_tag() == Some(TokenTag::ASSIGN)
        {
            break;
        }
        ast_log!("Parsing expression: ", #token);
        // Inside a group the `)` is always consumed, so a missing `)` reports as a delimiter error.
        // A group is never a receiving boundary and takes expected-result evidence from its own
        // parse, so only the root keeps the grammar's boundary and consumption policy.
        let mut dispatch_state = ExpressionDispatchState {
            expected_type: input.expected_type,
            cast_target_context: input.cast_target_context,
            value_mode: input.value_mode,
            consume_closing_parenthesis: input.trailing_policy.consume_closing_parenthesis
                || in_group,
            allow_boundary_catch: input.trailing_policy.allow_boundary_catch && !in_group,
            allow_expected_result_evidence: input.trailing_policy.allow_expected_result_evidence
                || in_group,
            expression,
            next_number_negative: &mut next_number_negative,
            retain_pending_numeric_fragment: input.retain_pending_numeric_fragment,
            group_frames: &mut group_frames,
            expression_builder: &mut *input.expression_builder,
            const_values: &mut *input.const_values,
        };
        let (close_group, early_value) = match dispatch_expression_token(
            token,
            input.token_stream,
            input.scope_context,
            input.type_interner,
            &mut dispatch_state,
            input.string_table,
            input.path_fork,
        )? {
            ExpressionTokenStep::Continue => (false, None),
            ExpressionTokenStep::Advance => {
                input.token_stream.advance();
                (false, None)
            }
            ExpressionTokenStep::CloseGroup => (true, None),
            // A break inside a group ends the group as one operand, and the parent re-dispatches
            // the same token. A value-template return has already consumed the group's `)`.
            ExpressionTokenStep::Break if dispatch_state.in_group() => (true, None),
            ExpressionTokenStep::Break => break,
            ExpressionTokenStep::Return(value) if dispatch_state.in_group() => (true, Some(value)),
            ExpressionTokenStep::Return(value) => return Ok(Some(value)),
        };
        if close_group {
            close_innermost_group(
                input.token_stream,
                input.scope_context,
                input.type_interner,
                &mut dispatch_state,
                input.string_table,
                input.path_fork,
                early_value,
                root_allow_boundary_catch,
            )?;
        }
    }

    if !group_frames.is_empty() {
        // The stream ended before a group's `)`, so that group has no closing delimiter.
        return Err(CompilerDiagnostic::missing_closing_delimiter(
            input.string_table.intern(")"),
            Some(input.token_stream.current_span()),
        )
        .into());
    }

    if input.trailing_policy.skip_trailing_newlines {
        input.token_stream.skip_newlines();
    }

    Ok(None)
}

// WHAT: parses an expression from a bounded token slice without consuming the stop token.
// WHY: some parent parsers need normal expression semantics while reserving a delimiter for the
//      surrounding grammar layer to inspect and consume itself.
pub(crate) fn create_expression_until(
    input: ExpressionParseInput<'_, '_, '_>,
    stop_tokens: &[TokenTag],
) -> Result<OwnedExpr, ExpressionParseError> {
    create_expression_until_with_policy(input, stop_tokens)
}

fn create_expression_until_with_policy(
    input: ExpressionParseInput<'_, '_, '_>,
    stop_tokens: &[TokenTag],
) -> Result<OwnedExpr, ExpressionParseError> {
    let allow_boundary_catch = input.trailing_policy.allow_boundary_catch;
    let allow_expected_result_evidence = input.trailing_policy.allow_expected_result_evidence;

    // Fast path: no stop tokens means an unbounded parse with the same boundary policy.
    if stop_tokens.is_empty() {
        let mut unbounded_input = input;
        unbounded_input.trailing_policy = ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: true,
            allow_boundary_catch,
            allow_expected_result_evidence,
        };

        return create_expression_with_trailing_newline_policy(unbounded_input);
    }

    // ------------------------
    //  Locate expression window
    // ------------------------
    let start_index = input.token_stream.position();
    // The boundary scan walks the live cursor and restores the entry position:
    // indexed reads re-scan segment prefixes on every step.
    let mut depth = ExpressionBoundaryDepth::default();
    while !input.token_stream.is_at_end() {
        if depth.is_top_level() && stop_tokens.contains(&input.token_stream.current_tag()) {
            break;
        }
        depth.step_tag(input.token_stream.current_tag());
        // A malformed payload is surfaced by typed readers at the consuming parser boundary.
        if input.token_stream.current_tag() == TokenTag::EOF {
            break;
        }
        input.token_stream.advance();
    }
    let end_index = input.token_stream.position();
    let end_tag =
        (end_index < input.token_stream.length()).then(|| input.token_stream.current_tag());
    let end_span = input.token_stream.span_at(end_index);
    input
        .token_stream
        .set_position(start_index)
        .expect("expression boundary resume stays inside the active parser view");

    // ------------------------
    //  Validate window bounds
    // ------------------------
    if end_index >= input.token_stream.length() {
        let formatted_stop_tokens: Vec<String> = stop_tokens
            .iter()
            .map(|token| format!("{token:?}"))
            .collect();
        let expected_tokens = formatted_stop_tokens.join(", ");

        let expected_delimiter = input.string_table.intern(&expected_tokens);
        return Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(expected_delimiter),
            Some(input.token_stream.current_span()),
        )
        .into());
    }

    let Some(end_tag) = end_tag else {
        let formatted_stop_tokens: Vec<String> = stop_tokens
            .iter()
            .map(|token| format!("{token:?}"))
            .collect();
        let expected_tokens = formatted_stop_tokens.join(", ");

        let expected_delimiter = input.string_table.intern(&expected_tokens);
        return Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(expected_delimiter),
            Some(input.token_stream.current_span()),
        )
        .into());
    };

    if end_index == start_index {
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(end_tag),
            end_span,
        )
        .into());
    }

    if !stop_tokens.contains(&end_tag) {
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(end_tag),
            end_span,
        )
        .into());
    }

    // ------------------------
    //  Parse within window
    // ------------------------
    let window_token_count = end_index.saturating_sub(start_index);
    add_ast_counter(AstCounter::BoundedExpressionTokenWindows, 1);
    add_ast_counter(
        AstCounter::BoundedExpressionTokenCopiesAvoided,
        window_token_count,
    );

    // Narrow the visible token stream so the inner parser stops at the stop token
    // without needing to copy the slice. The original limit is restored after parsing.
    let previous_limit = input.token_stream.set_limit(end_index)?;

    let mut inner_input = ExpressionParseInput::new(
        ExpressionParseResources {
            token_stream: input.token_stream,
            scope_context: input.scope_context,
            type_interner: input.type_interner,
            expression_builder: input.expression_builder,
            const_values: input.const_values,
            expected_type: input.expected_type,
            cast_target_context: input.cast_target_context,
            value_mode: input.value_mode,
            string_table: input.string_table,
            path_fork: input.path_fork,
        },
        ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: true,
            allow_boundary_catch,
            allow_expected_result_evidence,
        },
    );
    // The bounded window is the position's whole authored syntax, so the position's numeric
    // retention ownership travels into the inner parse exactly like the unbounded path. Without
    // this, a deferral raised inside a bounded parse would look like a swallowed sub-fragment and
    // its consumer could never retain the complete position.
    inner_input.numeric_deferral_owner = input.numeric_deferral_owner;
    inner_input.retain_pending_numeric_fragment = input.retain_pending_numeric_fragment;

    let result = create_expression_with_trailing_newline_policy(inner_input);

    // Restore the full stream and position the cursor on the stop token.
    // `input` was never moved, only its fields were reborrowed for `inner_input`,
    // so the token stream reference is still available here.
    input.token_stream.restore_limit(previous_limit);
    input.token_stream.set_position(end_index)?;
    result
}

#[cfg(test)]
#[path = "tests/bounded_expression_tests.rs"]
mod bounded_expression_tests;
