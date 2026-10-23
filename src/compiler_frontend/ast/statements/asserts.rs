//! Assert statement parsing.
//!
//! WHAT: parses the reserved `assert` statement through the shared call-argument contract,
//!       retaining numeric-dependent arguments until the body root can resolve them.
//! WHY: assertion placement, unrecoverable failure, and message control-flow policy are special;
//!      parentheses, separators, named routing, defaults, access and type validation are not.
//!
//! A condition or message that references a pending numeric origin is retained here as one
//! assertion continuation at its authored position. The body root completes the retained
//! fragments and calls [`finish_assert_statement`], the same finisher the completed parse path
//! uses, so defaults, slot routing, access modes, type diagnostics, message-effect policy and the
//! assertion node shape cannot diverge between the two paths.
//!
//! The message-effect classifier lives in `ast::expressions::assertion_message_effects`, where
//! its AST/TIR/runtime representation boundary and message-evaluation control-flow rules have a
//! truthful owner. This module remains responsible for the assertion statement contract only.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    assert_message_escape_diagnostic, assertion_condition_is_statically_true,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ExpectedAccessMode,
    ExpectedParameterType, ParameterDefault, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionConstructionFailure, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::failure_classification::pending_expression_failure_facts;
use crate::compiler_frontend::ast::generic_functions::GenericRequestRange;
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, DeferredCallArgument, NumericContinuation,
};
use crate::compiler_frontend::ast::statements::functions::ParameterContract;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::{
    InvalidFallibleHandlingReason, ValueReceiverContext,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Declaration-order slot of the assertion condition parameter.
const ASSERTION_CONDITION_SLOT: usize = 0;

/// Number of authored assertion parameters: the condition and the optional message.
const ASSERTION_PARAMETER_COUNT: usize = 2;

/// Continuation for one `assert` statement whose condition or message still needs numeric
/// resolution.
///
/// WHAT: the authored argument list (each slot completed with its routing, or retained with its
///       fragment and receiving constraint), the assertion's exact parameter expectations, and
///       the statement span and scope.
/// WHY: assertion argument compatibility cannot run before numeric selection, and a resumed
///      assertion must run the same finisher as the completed parse path without reparsing
///      tokens, rebuilding slot routing or reconstructing the message default.
#[derive(Debug)]
pub(crate) struct AssertionContinuation {
    pub(crate) arguments: Vec<DeferredCallArgument>,
    /// The authored condition/message expectations, in declaration order.
    pub(crate) expectations: [ParameterExpectation; ASSERTION_PARAMETER_COUNT],
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
    /// Original parse slots, kept separate from the later numeric-completion suffix.
    pub(crate) generic_request_range: GenericRequestRange,
}

/// Everything the assertion finisher needs from its authored site.
///
/// WHAT: the ordered resolved argument list, the authored parameter expectations, the statement
///       span and scope, and the request-sink position at which this assertion's own work began.
/// WHY: one finisher must serve both the completed parse path and the resumed body-root path, so
///      it takes exactly the facts the two sites already own instead of re-deriving either.
pub(crate) struct AssertionFinishInput<'a> {
    pub(crate) arguments: Vec<CallArgument<OwnedExpr>>,
    pub(crate) expectations: &'a [ParameterExpectation],
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
    /// Sink position at which this assertion's own work began.
    ///
    /// A statically true condition makes the message executable-inactive: requests since this
    /// position become validation-only candidates. Each caller passes a position that still
    /// brackets only this statement's suffix: the parse checkpoint on the completed path, or the
    /// resume-window start on the retained path. A sibling statement's requests keep their class.
    pub(crate) request_provenance: usize,
}

/// Validate one assertion argument list and build its AST node.
///
/// WHAT: the single assertion finisher: shared call validation, condition/message extraction,
///       message-effect and failure-fact checking, inactive-message request retention, and the
///       `Assert` node constructor.
/// WHY: the completed parse path and the resumed body-root path must not drift in defaults,
///      named/positional slots, access policy, type diagnostics or message control-flow policy.
pub(crate) fn finish_assert_statement(
    input: AssertionFinishInput<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<AstNode, ExpressionParseError> {
    let AssertionFinishInput {
        arguments,
        expectations,
        span,
        scope,
        request_provenance,
    } = input;
    for argument in &arguments {
        expression_builder
            .view_owned(argument.value)?
            .require_single(ValueReceiverContext::AssertionArgument)?;
    }

    let resolved_arguments = {
        let type_check_context = type_interner.type_check_context();
        resolve_call_arguments(
            CallDiagnosticContext::assertion(CallDiagnosticName::Borrowed("assert")),
            arguments,
            expectations,
            span,
            CallArgumentResolutionContext {
                string_table,
                type_environment: type_check_context.type_environment,
                path_fork,
                compatibility_cache: type_check_context.compatibility_cache,
                expression_builder,
                const_values,
            },
        )?
    };

    let mut resolved_arguments = resolved_arguments.into_iter();
    let condition = resolved_arguments
        .next()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a condition argument",
            )
        })?
        .value;
    let mut message = resolved_arguments
        .next()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a message argument",
            )
        })?
        .value;

    // The message is validated in full even when the condition is statically true: it must never
    // escape the enclosing function, and its failure facts must be complete before any inactive
    // work is pruned. Only the borrow for each check is held, never across fragment completion.
    if let Some(diagnostic) = assert_message_escape_diagnostic(
        expression_builder.view_owned(message)?,
        &context.template_ir_store.borrow(),
        &*expression_builder,
    )? {
        return Err(diagnostic.into());
    }
    let pending = pending_expression_failure_facts(
        expression_builder.view_owned(message)?,
        &context.template_ir_store.borrow(),
        &*expression_builder,
    )?;
    let mut facts = expression_builder
        .view_owned(message)?
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    facts.summary = pending.summary;
    facts.checked_numeric_operation = pending.checked_numeric_operation;
    facts.postfix_exit_span = pending.postfix_exit_span;
    message = expression_builder.with_failure_facts(message, facts)?;

    if assertion_condition_is_statically_true(
        expression_builder.view_owned(condition)?,
        const_values,
    ) {
        // Message failure validation may need concrete generic call summaries even though no
        // message work executes. Retain one validation-only candidate in each original slot;
        // finalization commits only candidates witnessed by surviving concrete assertions.
        context.retain_assertion_validation_requests_since(request_provenance);
    }

    Ok(AstNode {
        kind: NodeKind::Assert {
            condition: condition.id(),
            message: message.id(),
        },
        span,
        scope,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "assert statement keeps stream, body draft, scope and mutable parse resources apart"
)]
pub(crate) fn parse_assert_statement(
    token_stream: &mut AstCursor,
    ast: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let assert_span = Some(token_stream.current_span());
    let assert_name = string_table.intern("assert");
    let condition_name = string_table.intern("condition");
    let message_name = string_table.intern("message");
    let generic_request_checkpoint = context.generic_request_checkpoint();

    token_stream.advance(); // past `assert`

    let bool_type_id = type_interner.environment().builtins().bool;
    let string_type_id = type_interner.environment().builtins().string;
    let message_type_id = type_interner
        .environment_mut_for_derived_types()
        .intern_option(string_type_id);
    let default_message = const_values
        .insert_resolved(message_type_id, ConstValuePayload::OptionNone, assert_span)
        .map_err(ExpressionConstructionFailure::from)?;

    let expectations = [
        ParameterExpectation {
            name: Some(condition_name),
            expected_type: ExpectedParameterType::Known(bool_type_id),
            access_mode: ExpectedAccessMode::Shared,
            default_value: None,
            contract: ParameterContract::Value,
        },
        ParameterExpectation {
            name: Some(message_name),
            expected_type: ExpectedParameterType::Known(message_type_id),
            access_mode: ExpectedAccessMode::Shared,
            default_value: Some(ParameterDefault::Folded(default_message)),
            contract: ParameterContract::Value,
        },
    ];

    // An assertion argument is not a receiving site the ordinary expression path can resume, so a
    // numeric-dependent argument is retained here and resumed through the assertion continuation
    // instead of escaping as an unresolved fragment.
    let parsed_arguments = parse_call_arguments_typed_with_numeric_retention(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        &expectations,
        CallArgumentSyntax::Assertion {
            callee_name: Some(assert_name),
        },
        path_fork,
    )?;

    let arguments = match parsed_arguments {
        ParsedCallArguments::Ready(arguments) => arguments,
        ParsedCallArguments::Retained(arguments) => {
            // The authored suffix belongs to this statement and only this parse site can still
            // see it; a resumed statement has no tokens to re-read.
            reject_assert_fallible_suffix(token_stream)?;

            // These parse-time tickets still form this assertion's suffix. Retain their
            // validation class now, before later siblings reserve slots; numeric completion can
            // then finish each ticket without affecting unrelated requests.
            if retained_assert_condition_is_statically_true(
                &arguments,
                expression_builder,
                const_values,
            )? {
                context.retain_assertion_validation_requests_since(generic_request_checkpoint);
            }

            let continuation = context.register_numeric_continuation(
                NumericContinuation::Assertion(Box::new(AssertionContinuation {
                    arguments,
                    expectations,
                    span: assert_span,
                    scope: context.scope,
                    generic_request_range: GenericRequestRange::new(
                        generic_request_checkpoint,
                        context.generic_request_checkpoint(),
                    ),
                })),
            )?;
            ast.push_pending(continuation);
            return Ok(());
        }
    };

    let node = finish_assert_statement(
        AssertionFinishInput {
            arguments,
            expectations: expectations.as_slice(),
            span: assert_span,
            scope: context.scope,
            request_provenance: generic_request_checkpoint,
        },
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    // Reject `assert(...)!` and `assert(...) catch ...` — assert is not a fallible expression.
    reject_assert_fallible_suffix(token_stream)?;

    ast.push_node(node);
    Ok(())
}

/// Reject the authored fallible suffix of an assertion statement.
///
/// WHAT: reports the same non-fallible handling diagnostics the completed path has always
///       reported for `!` and `catch` after an assertion.
/// WHY: only the authored parse site can still see the suffix tokens, and a retained statement
///      must reject them before it records a continuation instead of re-reading tokens later.
fn reject_assert_fallible_suffix(token_stream: &AstCursor) -> Result<(), ExpressionParseError> {
    if token_stream.current_tag() == TokenTag::BANG {
        return Err(choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() == TokenTag::CATCH {
        return Err(choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnNonFallible,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Whether a retained argument list's condition slot is already the literal `true`.
///
/// WHAT: inspects only the condition slot's retained routing and value; a retained condition
///       fragment reports `false` because a `true` literal never defers.
/// WHY: parse-time validation retention must classify known-true messages before siblings reserve
///       tickets. A condition that becomes true only after completion uses the saved exact range.
fn retained_assert_condition_is_statically_true(
    arguments: &[DeferredCallArgument],
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<bool, ExpressionParseError> {
    for argument in arguments {
        if let DeferredCallArgument::Ready(ready) = argument
            && ready
                .parameter_slot
                .is_some_and(|slot| slot.index() == ASSERTION_CONDITION_SLOT)
            && assertion_condition_is_statically_true(expressions.view(ready.value)?, const_values)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "tests/assertion_message_tests.rs"]
mod assertion_message_tests;
