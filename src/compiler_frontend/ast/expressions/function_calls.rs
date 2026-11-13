//! Function-call parsing and typed producer completion.
//!
//! Calls finish explicit propagation immediately. Unhandled typed producers keep ordinary success
//! types and expression-owned failure facts until whole-expression catch completion decides their
//! delivery. Arguments retain their own pending contributors independently of the outer call: a
//! non-generic source call whose numeric-dependent arguments defer is retained as one pending
//! operand together with its stable callee identity, parameter expectations, ordered result slots
//! and authored operand `!`, and body completion resumes it exactly once. A visible external call
//! retains the same way: its host identity, parameter expectations, already-settled result slots,
//! float-integrity fact and authored operand `!` travel with the pending operand, and body
//! completion rebuilds the ordinary host constructor without a registry lookup or a second token
//! cursor. A `catch` is never consumed by a retained call: it protects the whole selected
//! expression, so the enclosing fragment owner retains it with the pending operand in place.
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{
    ExplicitConstantPayload, explicit_constant_operand, registered_external_const_eval_op,
};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ParameterExpectation,
    expectations_from_host_function, expectations_from_user_parameters, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::HandledFallibleHostFunctionCallInput;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstValueKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::GenericErrorReturn;
use crate::compiler_frontend::ast::numeric_construction::{
    CallCompletion, CompletedExpression, DeferredCall, DeferredCallArgument, DeferredCallHandling,
    DeferredExpressionCompletion, DeferredExpressionServices, DeferredSourceCall,
};
use crate::compiler_frontend::ast::statements::fallible_handling::{
    FUNCTION_CALL_STAGE, FallibleCallSite, FallibleHostCallSite, FallibleSuccessContract,
    HandledFallibleCall, HandledFallibleHostCall, ParsedFallibleSuffix,
    non_fallible_call_handler_diagnostic, parse_fallible_handling_suffix_for_call_expression,
    parse_fallible_handling_suffix_for_host_call_expression, parse_fragment_handling_suffix,
    token_stream_starts_typed_propagation_suffix,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::InvalidCallShapeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionDef, ExternalFunctionId, ExternalFunctionReturnContract, ExternalReturnSlot,
    ExternalReturnsView, ExternalSignatureType,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input bundle for `parse_function_call` to avoid long argument lists.
pub struct FunctionCallParseInput<'a, 'b, 'tokens> {
    pub token_stream: &'a mut AstCursor<'tokens>,
    pub id: &'a PathId,
    pub call_span: Option<SourceSpan>,
    pub context: &'a ScopeContext,
    pub signature: &'a FunctionSignature,
    /// Callee execution classification resolved at call typing.
    pub evaluation: CallableEvaluation,
    pub value_required: bool,
    pub allow_boundary_catch: bool,
    pub warnings: Option<&'a mut DiagnosticBag>,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub string_table: &'a mut StringTable,
    pub path_fork: &'a mut PathInternerFork,
}

/// One parsed call expression: complete at its receiving site, or retained for the body root.
///
/// WHAT: separates the finished call value from an operand item the enclosing fragment keeps for
///       body-root completion.
/// WHY: the enclosing expression buffer needs the exact operand shape; a retained call occupies
///      one operand position and must never be represented by a fabricated value.
pub(crate) enum ParsedFunctionCallExpression {
    /// The call completed with its validated argument list.
    Value(OwnedExpr),
    /// The call's contract is `-> !`: it has no value and is valid only as a standalone statement.
    Terminating(Box<NeverCall>),
    /// A retained operand item: the pending call itself, with its authored operand `!` when one
    /// was consumed. A `catch` is never consumed here; it stays with the enclosing fragment.
    RetainedOperand(ExpressionRpnItem),
}

/// Input bundle for external function calls.
pub struct ExternalFunctionCallParseInput<'a, 'b, 'tokens> {
    pub token_stream: &'a mut AstCursor<'tokens>,
    pub external_function_id: ExternalFunctionId,
    pub external_function: &'a ExternalFunctionDef,
    pub call_span: Option<SourceSpan>,
    pub context: &'a ScopeContext,
    pub value_required: bool,
    pub allow_boundary_catch: bool,
    pub warnings: Option<&'a mut DiagnosticBag>,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub string_table: &'a mut StringTable,
    pub path_fork: &'a mut PathInternerFork,
}

struct ParsedExternalFunctionCall {
    id: ExternalFunctionId,
    args: Vec<CallArgument<OwnedExpr>>,
    result_type_ids: Vec<TypeId>,
    requires_external_float_validation: bool,
    error_return_type_id: Option<TypeId>,
    span: Option<SourceSpan>,
}

/// One external-package call retained until its numeric-dependent arguments resolve.
///
/// WHAT: the stable host identity, the already-built parameter expectations and interned diagnostic
///       name, the ordered argument list, the result slots with the single float-integrity fact,
///       the contract-derived completion kind, and the authored call-site facts (operand `!`,
///       span).
/// WHY: the external ABI's parameter slots are concrete at the authored site, but argument
///      compatibility cannot run before the numeric origins resolve. Moving the expectations and
///      guard facts here lets body completion rebuild the ordinary host constructor exactly once
///      without cloning the definition, rebuilding expectations or searching the registry.
#[derive(Debug, Clone)]
pub(crate) struct DeferredExternalCall {
    pub(crate) function_id: ExternalFunctionId,
    pub(crate) callee_name: StringId,
    pub(crate) expectations: Vec<ParameterExpectation>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) requires_external_float_validation: bool,
    pub(crate) error_return_type_id: Option<TypeId>,
    pub(crate) call_span: Option<SourceSpan>,
    /// Whether the host signature's settled return contract completes as a value or terminates.
    ///
    /// This is the contract fact, settled from the registered definition at retention; completion
    /// reads it back instead of accepting a caller's preference. Zero success slots are also valid
    /// for ordinary no-value completion, so the slots alone cannot distinguish the two.
    pub(crate) completion: CallCompletion,
    /// The authored operand `!` consumed at the call's receiving site.
    pub(crate) authored_handling: DeferredCallHandling,
}

pub(crate) struct CallFinishContext<'a, 'b, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) context: &'a ScopeContext,
    /// Callee execution classification resolved at call typing.
    pub(crate) evaluation: CallableEvaluation,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
    pub(crate) warnings: Option<&'a mut DiagnosticBag>,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Parses a source-function call for expression position.
///
/// WHAT: resolves a source or visible external function call and returns the expression-owned
///       call payload, or the retained call record when a numeric-dependent argument needs
///       body-root completion.
/// WHY: call validation belongs here, while callers should consume the same expression contract
///      in statement and expression positions.
pub(crate) fn parse_function_call_expression(
    input: FunctionCallParseInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let FunctionCallParseInput {
        token_stream,
        id,
        call_span,
        context,
        signature,
        evaluation,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;

    // Required const bodies cannot call runtime source functions. The callee classification
    // was resolved at call typing, so the rejection stays at the authored call.
    if context.is_const_body() && evaluation != CallableEvaluation::Const {
        return Err(choices_control_flow::const_body_runtime_operation(
            path_fork.component(*id),
            call_span,
            context.const_declaration_span(),
        )
        .into());
    }

    // ------------------------
    //  Route to external call
    // ------------------------
    // External calls share the same argument parser, but they reject named targets until
    // external metadata carries stable public parameter names.
    if let Some((function_id, host_function)) = path_fork
        .component(*id)
        .and_then(|name| context.lookup_visible_external_function(name))
    {
        return parse_external_function_call_expression(ExternalFunctionCallParseInput {
            token_stream,
            external_function_id: function_id,
            external_function: host_function,
            call_span,
            context,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        });
    }

    // ------------------------
    //  Parse arguments
    // ------------------------
    // A numeric-dependent argument is retained in place: argument compatibility cannot run
    // before its type is selected, and the call must resume with the same slot routing.
    let parameter_expectations =
        expectations_from_user_parameters(&signature.parameters, path_fork);
    let raw_args = parse_call_arguments_typed_with_numeric_retention(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        &parameter_expectations,
        CallArgumentSyntax::Supported {
            callee_name: path_fork.component(*id),
        },
        path_fork,
    )?;

    let arguments = match raw_args {
        ParsedCallArguments::Ready(arguments) => arguments,
        ParsedCallArguments::Retained(arguments) => {
            return retain_numeric_source_call(RetainedSourceCallInput {
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
                function_path: id,
                signature,
                evaluation,
                expectations: parameter_expectations,
                arguments,
                call_span,
                value_required,
                allow_boundary_catch,
            });
        }
    };

    let args = resolve_user_function_call_arguments(
        id,
        arguments,
        &parameter_expectations,
        call_span,
        string_table,
        type_interner,
        expression_builder,
        const_values,
        path_fork,
    )?;

    // Required const calls may not depend on runtime state. The call-site check is skipped while
    // validating a const body: its parameters and locals are compile-time placeholders, and a
    // nested const call receives them when the outer call executes.
    if evaluation == CallableEvaluation::Const {
        validate_const_call_arguments(
            &args,
            id,
            call_span,
            context,
            expression_builder,
            const_values,
            path_fork,
        )?;
    }

    // A Never callee has no value. The statement owner lifts this outcome into `NeverCall`;
    // every value owner rejects it at the shared evaluation choke point.
    let Some(returns) = signature.return_contract.ordinary() else {
        let callee_name = path_fork.component(*id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "a resolved Never call has no terminal path component"
            )
        });
        reject_never_call_continuation(token_stream, callee_name, call_span)?;
        return Ok(ParsedFunctionCallExpression::Terminating(Box::new(
            NeverCall {
                target: NeverCallTarget::Source(*id),
                receiver: None,
                arguments: args
                    .into_iter()
                    .map(|argument| argument.map_value(OwnedExpr::id))
                    .collect(),
                name: callee_name,
                span: call_span,
            },
        )));
    };

    let call = HandledFallibleCall {
        name: id.to_owned(),
        result_type_ids: returns.success().to_vec(),
        args,
        call_span,
    };

    finish_function_call_expression(
        call,
        returns.error(),
        CallFinishContext {
            token_stream,
            context,
            evaluation,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        },
    )
    .map(ParsedFunctionCallExpression::Value)
}

/// Everything the call site must hand to the retained-call builder.
///
/// WHY: the retained record and its authored suffix share one site, so grouping them keeps the
///      long argument list in one place instead of threading each resource separately.
struct RetainedSourceCallInput<'a, 'b, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    warnings: Option<&'a mut DiagnosticBag>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
    function_path: &'a PathId,
    signature: &'a FunctionSignature,
    evaluation: CallableEvaluation,
    expectations: Vec<ParameterExpectation>,
    arguments: Vec<DeferredCallArgument>,
    call_span: Option<SourceSpan>,
    value_required: bool,
    allow_boundary_catch: bool,
}

/// Builds the retained record of one non-generic source call with numeric-dependent arguments.
///
/// WHAT: consumes the authored operand postfix `!` (when present) through the shared
///       fallible-handling fragment entry and retains parameter expectations, result facts and
///       ordered arguments in the construction-only call record.
/// WHY: the call site is the only place the authored operand handling and concrete signature are
///      available; recording only completion facts lets body completion resume the call exactly
///      once instead of reparsing the argument list or dropping the authored propagation. A
///      `catch` is never this site's to consume: it protects the whole selected expression (for
///      example `10 + f(pending) catch:`), so the token stays for the enclosing fragment owner,
///      which retains the complete fragment together with its handler.
fn retain_numeric_source_call(
    input: RetainedSourceCallInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let RetainedSourceCallInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
        function_path,
        signature,
        evaluation,
        expectations,
        arguments,
        call_span,
        value_required,
        allow_boundary_catch,
    } = input;

    let Some(returns) = signature.return_contract.ordinary() else {
        // Arguments still depend on numeric completion, but the call stores no success slots or
        // error channel. The record's own contract-derived completion fact makes completion
        // rebuild the exact call and hand it back as a terminating statement payload.
        let callee_name = path_fork.component(*function_path).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "a resolved Never call has no terminal path component"
            )
        });
        reject_never_call_continuation(token_stream, callee_name, call_span)?;
        let call = DeferredCall::Source(Box::new(DeferredSourceCall {
            function_path: function_path.to_owned(),
            success_result_type_ids: Vec::new(),
            error_return_type_id: None,
            evaluation,
            completion: CallCompletion::Never,
            expectations,
            arguments,
            call_span,
            authored_handling: DeferredCallHandling::Unhandled,
        }));
        return Ok(retained_pending_call_operand(call, call_span));
    };

    let error_return_type_id = returns.error();
    let error_return = error_return_type_id.map(GenericErrorReturn::Concrete);
    let success_result_type_ids = returns.success();
    // ------------------------
    //  Operand postfix `!`
    // ------------------------
    // The shared fragment entry owns the whole authored operand-suffix decision for a retained
    // call: the `!` legality checks and the non-fallible diagnostics come from the same site facts
    // the completed call path passes. The entry is only consulted when the operand's own `!` is
    // authored: a `catch` belongs to the enclosing expression owner, so a retained call never
    // consumes it and no second parser or protected-call payload exists here.
    let authored_handling = parse_retained_call_authored_handling(RetainedCallHandlingInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        success_result_type_ids,
        error_return,
        authored_failure_lane: context.source_call_has_private_failure_lane(*function_path),
        value_required,
        call_span,
        allow_boundary_catch,
        warnings,
        string_table,
        path_fork,
    })?;

    Ok(retained_pending_call_operand(
        DeferredCall::Source(Box::new(DeferredSourceCall {
            function_path: function_path.to_owned(),
            success_result_type_ids: success_result_type_ids.to_vec(),
            error_return_type_id,
            evaluation,
            completion: CallCompletion::Value,
            expectations,
            arguments,
            call_span,
            authored_handling,
        })),
        call_span,
    ))
}

/// The inputs of one retained call's authored operand `!`.
pub(crate) struct RetainedCallHandlingInput<'a, 'b, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) success_result_type_ids: &'a [TypeId],
    pub(crate) error_return: Option<GenericErrorReturn>,
    pub(crate) authored_failure_lane: bool,
    pub(crate) value_required: bool,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) allow_boundary_catch: bool,
    pub(crate) warnings: Option<&'a mut DiagnosticBag>,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Parses the authored operand `!` of one retained call into its deferred handling.
///
/// WHAT: returns `Unhandled` when the operand has no authored `!`; otherwise the fragment suffix
///       entry must resolve it as typed propagation.
/// WHY: source and generic retained calls share one operand-suffix decision. A `catch` is never
///      consumed here: it protects the whole selected expression and belongs to the enclosing
///      fragment owner.
pub(crate) fn parse_retained_call_authored_handling(
    input: RetainedCallHandlingInput<'_, '_, '_>,
) -> Result<DeferredCallHandling, ExpressionParseError> {
    let RetainedCallHandlingInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        success_result_type_ids,
        error_return,
        authored_failure_lane,
        value_required,
        call_span,
        allow_boundary_catch,
        warnings,
        string_table,
        path_fork,
    } = input;

    if token_stream.current_tag() != TokenTag::BANG {
        return Ok(DeferredCallHandling::Unhandled);
    }

    let propagation_span = Some(token_stream.current_span());
    let authored_suffix = parse_fragment_handling_suffix(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        FallibleSuccessContract::Known(success_result_type_ids),
        error_return,
        authored_failure_lane,
        value_required,
        call_span,
        FUNCTION_CALL_STAGE,
        allow_boundary_catch,
        warnings,
        string_table,
        path_fork,
    )?;
    let Some(ParsedFallibleSuffix::Ready(FallibleHandling::Propagate)) = authored_suffix else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "an authored operand `!` was not parsed as typed propagation",
        )
        .into());
    };

    Ok(DeferredCallHandling::Propagate {
        span: propagation_span,
    })
}

/// Places one retained call in its authored operand position.
pub(crate) fn retained_pending_call_operand(
    call: DeferredCall,
    span: Option<SourceSpan>,
) -> ParsedFunctionCallExpression {
    ParsedFunctionCallExpression::RetainedOperand(ExpressionRpnItem::PendingNumericCall {
        call: Box::new(call),
        span,
    })
}

/// Reject every continuation a Never call cannot support.
///
/// WHAT: reports postfix `!`, `?`, `catch` and member access after a call whose contract is
///       `-> !`.
/// WHY: the callee never produces a success, error, optional or receiver value, so these suffixes
///      have nothing to consume. Rejecting them at the call site keeps the targeted placement
///      diagnostic on the authored call instead of an unrelated fallible-handling error.
pub(crate) fn reject_never_call_continuation(
    token_stream: &AstCursor<'_>,
    callee_name: StringId,
    call_span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    let is_continuation = matches!(
        token_stream.current_tag(),
        TokenTag::BANG | TokenTag::QUESTION_MARK | TokenTag::CATCH | TokenTag::DOT
    );
    if !is_continuation {
        return Ok(());
    }

    Err(calls_data::invalid_call_shape(
        InvalidCallShapeReason::NeverCallHandled,
        Some(callee_name),
        call_span.or_else(|| Some(token_stream.current_span())),
    )
    .into())
}

pub(crate) fn finish_function_call_expression(
    call: HandledFallibleCall,
    error_return_type_id: Option<TypeId>,
    finish: CallFinishContext<'_, '_, '_>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let CallFinishContext {
        token_stream,
        context,
        evaluation,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork: _path_fork,
    } = finish;

    let Some(error_return_type_id) = error_return_type_id else {
        if token_stream.current_tag() == TokenTag::BANG {
            return Err(non_fallible_call_handler_diagnostic(
                token_stream,
                call.result_type_ids.as_slice(),
                TokenTag::BANG,
                type_interner.environment(),
            )
            .into());
        }

        let call_path = call.name;
        let expression = call.into_plain_expression(expression_builder, evaluation)?;
        return Ok(if context.source_call_has_private_failure_lane(call_path) {
            expression_builder.with_private_call_failure_candidate(expression, call_path)?
        } else {
            expression
        });
    };
    if token_stream_starts_typed_propagation_suffix(token_stream) {
        return parse_fallible_handling_suffix_for_call_expression(
            token_stream,
            context,
            FallibleCallSite {
                call,
                error_return_type_id,
                value_required,
                allow_boundary_catch,
            },
            warnings,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            _path_fork,
        );
    }

    let call = expression_builder.handled_fallible_function_call_with_typed_arguments(
        call.name,
        call.args,
        call.result_type_ids,
        error_return_type_id,
        FallibleExpressionHandling::Recover,
        call.call_span,
    )?;
    Ok(expression_builder.with_typed_error_producer(call, error_return_type_id, None)?)
}

/// Validates a non-generic source call against the expectations used to parse its arguments.
///
/// WHY: immediate and numeric-retained calls share this one validator so their diagnostics and
///      callee-name fallback cannot drift apart.
#[allow(
    clippy::too_many_arguments,
    reason = "user call argument resolution keeps the function name, owned arguments, expectations, span, string table, type interner, expression builder, const store, and path fork as separate inputs"
)]
pub(crate) fn resolve_user_function_call_arguments(
    function_name: &PathId,
    raw_args: Vec<CallArgument<OwnedExpr>>,
    expectations: &[ParameterExpectation],
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    path_fork: &PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    let callee_name = path_fork.component(*function_name);
    let type_check_context = type_interner.type_check_context();
    resolve_call_arguments(
        CallDiagnosticContext::function(CallDiagnosticName::PathComponent {
            component: callee_name,
            fallback: "<unknown>",
        }),
        raw_args,
        expectations,
        span,
        CallArgumentResolutionContext {
            expression_builder,
            const_values,
            string_table,
            type_environment: type_check_context.type_environment,
            path_fork,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .map_err(ExpressionParseError::from)
}

/// Enforce compile-time arguments for a required const call made outside a const body.
///
/// WHAT: requires every completed argument to be a compile-time value shape, allowing nested
///       const calls whose own arguments are constant.
/// WHY: a const declaration's own parameters and locals are placeholders that become concrete
///      only when the declaration executes; applying the call-site rule while validating the
///      declaration would reject valid const-to-const calls such as `inner(x)`.
pub(crate) fn validate_const_call_arguments(
    arguments: &[CallArgument<OwnedExpr>],
    callee_path: &PathId,
    call_span: Option<SourceSpan>,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    path_fork: &PathInternerFork,
) -> Result<(), ExpressionParseError> {
    if context.is_const_body() {
        return Ok(());
    }
    for argument in arguments {
        let kind = const_argument_kind(
            expression_builder.view_owned(argument.value)?,
            context,
            expression_builder,
            const_values,
        )?;
        if matches!(kind, ConstValueKind::NonConst) {
            let mut diagnostic = choices_control_flow::compile_time_evaluation_error(
                CompileTimeEvaluationErrorReason::ConstCallRequiresConstantArguments,
                path_fork.component(*callee_path),
                call_span,
            );
            if let Some(span) = context
                .top_level_declarations
                .get_by_path(callee_path)
                .and_then(|declaration| declaration.binding_span)
            {
                diagnostic =
                    diagnostic.with_label(Some(span), Some(LabelDraftMessage::PreviousDeclaration));
            }
            return Err(diagnostic.into());
        }
    }
    Ok(())
}

/// Classify one argument to a required const call.
///
/// WHAT: treats a call to another const function as a compile-time value when all of its own
///       arguments are compile-time values.
/// WHY: nested const calls are the normal composition mechanism, but the generic value-shape
///      classifier sees every call as non-constant until the executor folds it.
pub(crate) fn const_argument_kind(
    view: ExpressionView<'_>,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<ConstValueKind, ExpressionParseError> {
    // A registered infallible host call is compile-time executable even though the call's own
    // evaluation classification stays Runtime. Its arguments must still be compile-time values.
    // The call is matched first so the lookup only refines an operation this classifier already
    // knows is a call.
    if let ExpressionOperationView::Call(call) = view.operation()
        && registered_external_const_eval_op(view, Some(context.external_package_registry.as_ref()))
            .is_some()
    {
        for argument in call.arguments().iter() {
            if matches!(
                const_argument_kind(argument.value(), context, expression_builder, const_values)?,
                ConstValueKind::NonConst
            ) {
                return Ok(ConstValueKind::NonConst);
            }
        }
        return Ok(ConstValueKind::Composite);
    }

    let kind = view
        .const_value_kind_with_template_classifier(const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                expression_builder,
                const_values,
                Some(context.external_package_registry.as_ref()),
            )
        })
        .map_err(ExpressionParseError::from)?;
    if !matches!(kind, ConstValueKind::NonConst) {
        return Ok(kind);
    }
    const_composite_argument_kind(view, context, expression_builder, const_values)
}

/// Classify data construction whose leaves include required const calls.
///
/// WHAT: recursively accepts const calls nested inside a collection, map, nominal construction,
///       coercion or field access when every contained value is compile-time.
/// WHY: the generic value-shape classifier reports a composite containing a const call as
///      non-constant because the call itself is not yet folded. Treating the composite as runtime
///      would reject a valid argument such as `{inner()}` passed to another const function.
fn const_composite_argument_kind(
    view: ExpressionView<'_>,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<ConstValueKind, ExpressionParseError> {
    let all_constant = |values: &mut dyn Iterator<Item = ExpressionView<'_>>| {
        for value in values {
            if matches!(
                const_argument_kind(value, context, expression_builder, const_values)?,
                ConstValueKind::NonConst
            ) {
                return Ok(false);
            }
        }
        Ok::<bool, ExpressionParseError>(true)
    };

    match view.operation() {
        ExpressionOperationView::Call(call)
            if call.metadata().evaluation == CallableEvaluation::Const =>
        {
            let mut arguments = call.arguments().iter().map(|argument| argument.value());
            if all_constant(&mut arguments)? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        ExpressionOperationView::Collection(values) => {
            if all_constant(&mut values.iter())? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        ExpressionOperationView::Map(entries) => {
            let mut values = entries
                .iter()
                .flat_map(|entry| [entry.key(), entry.value()]);
            if all_constant(&mut values)? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        ExpressionOperationView::Struct { fields }
        | ExpressionOperationView::Choice { fields, .. }
        | ExpressionOperationView::RouteConstruct { fields, .. } => {
            if all_constant(&mut fields.iter().map(|field| field.value()))? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        ExpressionOperationView::Coerce { value, .. }
        | ExpressionOperationView::FieldAccess { base: value, .. }
        | ExpressionOperationView::Cast { source: value, .. }
        | ExpressionOperationView::LogicalNot { operand: value }
        | ExpressionOperationView::NumericUnary { operand: value, .. }
        | ExpressionOperationView::PropagateOption { value } => {
            const_argument_kind(value, context, expression_builder, const_values)
        }
        ExpressionOperationView::NumericBinary { left, right, .. }
        | ExpressionOperationView::LogicalBinary { left, right, .. }
        | ExpressionOperationView::Compare { left, right, .. }
        | ExpressionOperationView::Range {
            start: left,
            end: right,
            ..
        } => {
            let mut values = [left, right].into_iter();
            if all_constant(&mut values)? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        ExpressionOperationView::CollectionBuiltin {
            receiver,
            arguments,
            ..
        }
        | ExpressionOperationView::MapBuiltin {
            receiver,
            arguments,
            ..
        } => {
            let mut values =
                std::iter::once(receiver).chain(arguments.iter().map(|argument| argument.value()));
            if all_constant(&mut values)? {
                Ok(ConstValueKind::Composite)
            } else {
                Ok(ConstValueKind::NonConst)
            }
        }
        _ => Ok(ConstValueKind::NonConst),
    }
}

/// Parses an external-function call for expression position.
///
/// WHAT: returns the completed expression call contract after shared external-call argument and
///       signature validation, or the retained host-call operand when a numeric-dependent
///       argument needs body-root completion.
/// WHY: statement and expression parsing both consume the expression-owned call payload, avoiding
///      a statement-shaped call-node detour for host functions. A numeric-dependent argument
///      cannot be resolved at the call site, so its already-concrete host identity, parameter
///      expectations, diagnostic name, result slots and authored handling travel as one pending
///      operand instead of a deferral escaping past the call that must resume it.
pub(crate) fn parse_external_function_call_expression(
    input: ExternalFunctionCallParseInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let ExternalFunctionCallParseInput {
        token_stream,
        external_function_id,
        external_function,
        call_span,
        context,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;

    // ------------------------
    //  Parse raw arguments
    // ------------------------
    // External metadata does not expose public parameter names, so named arguments remain
    // intentionally unsupported. The shared retaining intake keeps each numeric-dependent
    // argument's authored fragment and slot routing for the body root, so the deferral never
    // escapes the call that owns it.
    let expectations = {
        let type_environment = type_interner.environment_mut_for_derived_types();
        expectations_from_host_function(external_function, type_environment)
    };
    let callee_name = string_table.intern(&external_function.name);
    if context.is_const_body() && external_function.const_eval.is_none() {
        return Err(choices_control_flow::const_body_runtime_operation(
            Some(callee_name),
            call_span,
            context.const_declaration_span(),
        )
        .into());
    }
    let raw_args = parse_call_arguments_typed_with_numeric_retention(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        &expectations,
        CallArgumentSyntax::UnsupportedCall {
            callee_name: Some(callee_name),
        },
        path_fork,
    )?;

    // Signature types are shared by ordinary and Never contracts; validate them once before
    // either lane settles its result facts.
    validate_external_signature_types_are_registered(
        external_function,
        &external_function.return_contract,
        context,
        call_span,
    )?;
    let ordinary_returns = external_function.return_contract.ordinary();

    let arguments = match raw_args {
        ParsedCallArguments::Ready(arguments) => arguments,
        ParsedCallArguments::Retained(arguments) => {
            let Some(returns) = ordinary_returns else {
                // Arguments still depend on numeric completion, but a Never call has no result
                // facts to settle. Completion rebuilds the same external call and hands it back
                // as a terminating statement payload.
                reject_never_call_continuation(token_stream, callee_name, call_span)?;
                return Ok(retained_pending_call_operand(
                    DeferredCall::External(Box::new(DeferredExternalCall {
                        function_id: external_function_id,
                        callee_name,
                        expectations,
                        arguments,
                        result_type_ids: Vec::new(),
                        requires_external_float_validation: false,
                        error_return_type_id: None,
                        call_span,
                        completion: CallCompletion::Never,
                        authored_handling: DeferredCallHandling::Unhandled,
                    })),
                    call_span,
                ));
            };
            // The host ABI's parameter and result slots are already concrete, so the signature,
            // error and float-integrity facts settle here and travel with the pending call.
            let facts = settle_external_call_facts(
                external_function_id,
                external_function,
                returns,
                call_span,
                context,
                type_interner,
                string_table,
            )?;
            return retain_numeric_external_call(RetainedExternalCallInput {
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
                external_function_id,
                callee_name,
                expectations,
                facts,
                arguments,
                call_span,
                value_required,
                allow_boundary_catch,
            });
        }
    };

    // ------------------------
    //  Resolve and validate arguments
    // ------------------------
    let type_check_context = type_interner.type_check_context();
    let mut args = resolve_call_arguments(
        CallDiagnosticContext::host_function(CallDiagnosticName::Interned(callee_name)),
        arguments,
        &expectations,
        call_span,
        CallArgumentResolutionContext {
            expression_builder,
            const_values,
            string_table,
            type_environment: type_check_context.type_environment,
            path_fork,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .map_err(ExpressionParseError::from)?;

    // A compile-time text call folds only over values, so an explicit `#` operand is read as
    // its constant rather than kept as a runtime read of the module binding.
    if external_function.const_eval.is_some() {
        for argument in &mut args {
            if let Some(value) = explicit_constant_operand(
                argument.value,
                ExplicitConstantPayload::Text,
                context,
                path_fork,
                expression_builder,
                const_values,
            )? {
                argument.value = value;
            }
        }
    }

    // ------------------------
    //  Validate signature and returns
    // ------------------------
    let Some(returns) = ordinary_returns else {
        // A Never callee has no value. The statement owner lifts this outcome into
        // `NeverCall`; every value owner rejects it at the shared evaluation choke point.
        reject_never_call_continuation(token_stream, callee_name, call_span)?;
        return Ok(ParsedFunctionCallExpression::Terminating(Box::new(
            NeverCall {
                target: NeverCallTarget::External(external_function_id),
                receiver: None,
                arguments: args
                    .into_iter()
                    .map(|argument| argument.map_value(OwnedExpr::id))
                    .collect(),
                name: callee_name,
                span: call_span,
            },
        )));
    };

    let facts = settle_external_call_facts(
        external_function_id,
        external_function,
        returns,
        call_span,
        context,
        type_interner,
        string_table,
    )?;

    finish_external_function_call_expression(
        ParsedExternalFunctionCall {
            id: facts.id,
            args,
            result_type_ids: facts.result_type_ids,
            requires_external_float_validation: facts.requires_external_float_validation,
            error_return_type_id: facts.error_return_type_id,
            span: facts.span,
        },
        CallFinishContext {
            token_stream,
            context,
            evaluation: CallableEvaluation::Runtime,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        },
    )
    .map(ParsedFunctionCallExpression::Value)
}

/// The parse-time signature, result and guard facts of one external call.
///
/// WHAT: the stable identity plus everything the registered definition settles before the
///       arguments complete: ordered success slots, the error slot when the callee is fallible,
///       the single float-integrity fact and the authored call span.
/// WHY: the completed and retained lanes need the same facts, so settling them once stops the
///      retained lane from re-deriving guard precision or re-looking-up the definition later.
struct ParsedExternalCallFacts {
    id: ExternalFunctionId,
    result_type_ids: Vec<TypeId>,
    requires_external_float_validation: bool,
    error_return_type_id: Option<TypeId>,
    span: Option<SourceSpan>,
}

/// Settles one ordinary external call's signature, result and guard facts from its registered
/// definition.
///
/// A Never contract is handled before this owner: it has no result facts to settle.
fn settle_external_call_facts(
    external_function_id: ExternalFunctionId,
    external_function: &ExternalFunctionDef,
    returns: ExternalReturnsView<'_>,
    call_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<ParsedExternalCallFacts, ExpressionParseError> {
    // ------------------------
    //  Validate signature and returns
    // ------------------------
    let builtin_error_type = resolve_builtin_error_type_typed(context, call_span, string_table)?;
    let type_environment = type_interner.environment_mut_for_derived_types();
    let result_type_ids = returns
        .success
        .iter()
        .filter_map(|slot| {
            slot.value_type
                .to_type_id(type_environment, builtin_error_type.type_id)
        })
        .collect::<Vec<_>>();
    validate_external_return_slots_are_visible(
        external_function,
        returns.success,
        &result_type_ids,
        call_span,
    )?;

    let error_return_type_id = returns
        .error
        .and_then(|error_type| error_type.to_type_id(type_environment, builtin_error_type.type_id));

    let error_return_type_id =
        if returns.error.is_some() {
            let Some(error_return_type_id) = error_return_type_id else {
                return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                "Fallible external function '{}' has no frontend-visible concrete error slot.",
                external_function.name
            ))
            .into());
            };

            Some(error_return_type_id)
        } else {
            None
        };

    // The float-integrity fact is settled once here, where the resolved success TypeId is
    // visible, so every later AST and HIR consumer carries one neutral boolean instead of
    // re-deriving it from the success type.
    let requires_external_float_validation =
        external_call_requires_float_validation(&result_type_ids, type_interner.environment());

    Ok(ParsedExternalCallFacts {
        id: external_function_id,
        result_type_ids,
        requires_external_float_validation,
        error_return_type_id,
        span: call_span,
    })
}

fn finish_external_function_call_expression(
    parsed_call: ParsedExternalFunctionCall,
    finish: CallFinishContext<'_, '_, '_>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let CallFinishContext {
        token_stream,
        context,
        evaluation: _,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork: _path_fork,
    } = finish;
    let ParsedExternalFunctionCall {
        id,
        args,
        result_type_ids,
        requires_external_float_validation,
        error_return_type_id,
        span,
    } = parsed_call;

    if let Some(error_type_id) = error_return_type_id {
        let call = HandledFallibleHostCall {
            name: id,
            args,
            result_type_ids,
            requires_external_float_validation,
            error_type_id,
            call_span: span,
        };

        if token_stream_starts_typed_propagation_suffix(token_stream) {
            return parse_fallible_handling_suffix_for_host_call_expression(
                token_stream,
                context,
                FallibleHostCallSite {
                    call,
                    value_required,
                    allow_boundary_catch,
                },
                warnings,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                _path_fork,
            );
        }

        let handled = expression_builder.handled_fallible_host_function_call_with_typed_arguments(
            HandledFallibleHostFunctionCallInput {
                id: call.name,
                args: call.args,
                result_type_ids: call.result_type_ids,
                requires_external_float_validation: call.requires_external_float_validation,
                error_type_id,
                handling: FallibleExpressionHandling::Recover,
                span: call.call_span,
            },
        )?;
        return Ok(expression_builder.with_typed_error_producer(handled, error_type_id, None)?);
    }

    if token_stream.current_tag() == TokenTag::BANG {
        return Err(non_fallible_call_handler_diagnostic(
            token_stream,
            result_type_ids.as_slice(),
            TokenTag::BANG,
            type_interner.environment(),
        )
        .into());
    }

    Ok(expression_builder.host_function_call_with_typed_arguments(
        id,
        args,
        result_type_ids,
        requires_external_float_validation,
        span,
    )?)
}

/// Everything the retained external-call builder needs at one authored call site.
///
/// WHY: the retained record and its authored operand suffix share one site, so grouping them keeps
///      the long argument list in one place instead of threading each resource separately.
struct RetainedExternalCallInput<'a, 'b, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'b>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    warnings: Option<&'a mut DiagnosticBag>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
    external_function_id: ExternalFunctionId,
    callee_name: StringId,
    expectations: Vec<ParameterExpectation>,
    facts: ParsedExternalCallFacts,
    arguments: Vec<DeferredCallArgument>,
    call_span: Option<SourceSpan>,
    value_required: bool,
    allow_boundary_catch: bool,
}

/// Builds the retained record of one visible external call with numeric-dependent arguments.
///
/// WHAT: consumes the authored operand postfix `!` (when present) through the shared
///       fallible-handling fragment entry and moves the already-built parameter expectations,
///       diagnostic name, settled result slots and ordered arguments into the retained record.
/// WHY: the call site owns the authored operand handling and concrete host expectations. Moving
///      them here lets body completion resume the call exactly once without cloning its definition,
///      rebuilding expectations, searching the registry or dropping the authored propagation.
///      A `catch` is never this site's to consume: it protects the whole
///      selected expression, so the token stays for the enclosing fragment owner, which retains
///      the complete fragment together with its handler.
fn retain_numeric_external_call(
    input: RetainedExternalCallInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let RetainedExternalCallInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
        external_function_id,
        callee_name,
        expectations,
        facts,
        arguments,
        call_span,
        value_required,
        allow_boundary_catch,
    } = input;

    // ------------------------
    //  Operand postfix `!`
    // ------------------------
    // The shared fragment entry owns the whole authored operand-suffix decision for a retained
    // external call: the `!` legality checks and the non-fallible diagnostics come from the same
    // site facts the completed host-call path passes. A visible external call has no authored
    // private failure lane of its own, so the failure-lane fact stays false and no synthetic error
    // slot is manufactured. The entry is only consulted when the operand's own `!` is authored: a
    // `catch` belongs to the enclosing expression owner, so a retained call never consumes it.
    let authored_handling = if token_stream.current_tag() == TokenTag::BANG {
        let propagation_span = Some(token_stream.current_span());
        let authored_suffix = parse_fragment_handling_suffix(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            FallibleSuccessContract::Known(&facts.result_type_ids),
            facts.error_return_type_id.map(GenericErrorReturn::Concrete),
            false,
            value_required,
            call_span,
            FUNCTION_CALL_STAGE,
            allow_boundary_catch,
            warnings,
            string_table,
            path_fork,
        )?;
        let Some(ParsedFallibleSuffix::Ready(FallibleHandling::Propagate)) = authored_suffix else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "an authored operand `!` was not parsed as typed propagation",
            )
            .into());
        };

        DeferredCallHandling::Propagate {
            span: propagation_span,
        }
    } else {
        DeferredCallHandling::Unhandled
    };

    Ok(ParsedFunctionCallExpression::RetainedOperand(
        ExpressionRpnItem::PendingNumericCall {
            call: Box::new(retained_external_call_record(
                external_function_id,
                callee_name,
                expectations,
                arguments,
                facts,
                authored_handling,
            )),
            span: call_span,
        },
    ))
}

/// Moves the call site's expectations and diagnostic identity into the construction-only record.
fn retained_external_call_record(
    function_id: ExternalFunctionId,
    callee_name: StringId,
    expectations: Vec<ParameterExpectation>,
    arguments: Vec<DeferredCallArgument>,
    facts: ParsedExternalCallFacts,
    authored_handling: DeferredCallHandling,
) -> DeferredCall {
    DeferredCall::External(Box::new(DeferredExternalCall {
        function_id,
        callee_name,
        expectations,
        arguments,
        result_type_ids: facts.result_type_ids,
        requires_external_float_validation: facts.requires_external_float_validation,
        error_return_type_id: facts.error_return_type_id,
        call_span: facts.span,
        completion: CallCompletion::Value,
        authored_handling,
    }))
}

/// Resumes one retained external call once the body root resolved its numeric-dependent arguments.
///
/// WHAT: completes every retained argument through the session's shared argument placement,
///       revalidates the whole list against the retained parameter expectations, and
///       rebuilds the exact host call the completed path builds, preserving its separate
///       success/error channels and authored propagation handling. The completion kind comes
///       from the record, which settled it from the host signature's return contract at the
///       authored site.
/// WHY: the host identity, expectations, result slots, float-integrity fact, return-contract
///      completion kind and authored operand `!` were settled at the call site and must not be
///      re-derived, re-parsed or dropped. The body root only supplies concrete argument values, so
///      completion needs neither the host definition nor another expectation-building pass,
///      registry lookup or token cursor.
pub(crate) fn complete_deferred_external_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredExternalCall,
    session: &mut S,
) -> Result<CompletedExpression, ExpressionParseError> {
    let DeferredExternalCall {
        function_id,
        callee_name,
        expectations,
        arguments,
        result_type_ids,
        requires_external_float_validation,
        error_return_type_id,
        call_span,
        completion,
        authored_handling,
    } = deferred;

    // Arguments complete before the session services are borrowed: both take the session mutably.
    let mut completed_arguments = Vec::with_capacity(arguments.len());
    for argument in arguments {
        completed_arguments.push(session.complete_call_argument(argument)?);
    }

    let DeferredExpressionServices {
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        ..
    } = session.session_services();
    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::host_function(CallDiagnosticName::Interned(callee_name)),
        completed_arguments,
        &expectations,
        call_span,
        CallArgumentResolutionContext {
            expression_builder,
            const_values,
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
        },
    )
    .map_err(ExpressionParseError::from)?;

    // The retained record settled this fact from the host signature's return contract; a Never
    // contract has no success or error slot. Completion keeps the stable external target and the
    // completed argument routing for statement lifting, exactly like the completed call path.
    if completion == CallCompletion::Never {
        return Ok(CompletedExpression::NeverCall(Box::new(NeverCall {
            target: NeverCallTarget::External(function_id),
            receiver: None,
            arguments: args
                .into_iter()
                .map(|argument| argument.map_value(OwnedExpr::id))
                .collect(),
            name: callee_name,
            span: call_span,
        })));
    }

    let Some(error_type_id) = error_return_type_id else {
        // The retention site already reported the authored operand `!` on an infallible callee,
        // and a `catch` was never the call site's to consume, so an infallible retained call
        // resumes as the plain host call.
        return Ok(CompletedExpression::Value(
            expression_builder.host_function_call_with_typed_arguments(
                function_id,
                args,
                result_type_ids,
                requires_external_float_validation,
                call_span,
            )?,
        ));
    };

    let call = HandledFallibleHostCall {
        name: function_id,
        args,
        result_type_ids,
        requires_external_float_validation,
        error_type_id,
        call_span,
    };
    match authored_handling {
        // A resumed call uses the same handled Recover shape as the completed call path;
        // whole-expression catch completion owns the delivery decision.
        DeferredCallHandling::Unhandled => {
            let handled = expression_builder
                .handled_fallible_host_function_call_with_typed_arguments(
                    HandledFallibleHostFunctionCallInput {
                        id: call.name,
                        args: call.args,
                        result_type_ids: call.result_type_ids,
                        requires_external_float_validation: call.requires_external_float_validation,
                        error_type_id,
                        handling: FallibleExpressionHandling::Recover,
                        span: call.call_span,
                    },
                )?;
            Ok(CompletedExpression::Value(
                expression_builder.with_typed_error_producer(handled, error_type_id, None)?,
            ))
        }
        // Replay the authored `!` through the same constructor the completed call path uses; the
        // retention site already validated the callee's error slot and the authored value
        // requirement, so no second decision exists here.
        DeferredCallHandling::Propagate { span } => {
            Ok(CompletedExpression::Value(call.into_expression(
                FallibleHandling::Propagate,
                span.or(call_span),
                expression_builder,
            )?))
        }
    }
}

/// Returns whether an external call's raw success value must pass HIR float validation.
///
/// WHAT: true when the resolved success channel is exactly one binary-float slot.
/// WHY: raw external boundaries may hand back any value, so every binary-float result needs
///      the HIR integrity guard at its resolved precision. The guard owns the single rounding
///      and finite check, so no JavaScript glue, package or lowering shape is exempted; every
///      other scalar, arity, or result shape stays outside this fact.
fn external_call_requires_float_validation(
    result_type_ids: &[TypeId],
    type_environment: &TypeEnvironment,
) -> bool {
    let [single] = result_type_ids else {
        return false;
    };

    NumericScalar::from_type_id(*single, type_environment)
        .is_some_and(NumericScalar::is_binary_float)
}

/// Verifies every external success-return slot has a frontend-visible semantic type.
///
/// Registration already rejects slots without a Moth value. An `Inferred` slot can still fail to
/// resolve a `TypeId`, which shortens the projected slot list.
fn validate_external_return_slots_are_visible(
    external_function: &ExternalFunctionDef,
    success_slots: &[ExternalReturnSlot],
    result_type_ids: &[TypeId],
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    if success_slots.len() != result_type_ids.len()
        || success_slots
            .iter()
            .any(|slot| !slot.value_type.represents_moth_value())
    {
        return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
            "External function '{}' declares a return slot that is not frontend-visible at {:?}.",
            external_function.name, span
        ))
        .into());
    }

    Ok(())
}

/// Ensures every type referenced in an external function signature is registered
/// in the external package registry.
///
/// Parameters belong to both contract variants. Success slots and the error slot are visited
/// only for an ordinary contract; a Never contract owns neither by construction.
fn validate_external_signature_types_are_registered(
    external_function: &ExternalFunctionDef,
    contract: &ExternalFunctionReturnContract,
    context: &ScopeContext,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    for parameter in &external_function.parameters {
        validate_external_signature_type_is_registered(
            external_function,
            &parameter.language_type,
            context,
            span,
        )?;
    }

    let Some(returns) = contract.ordinary() else {
        return Ok(());
    };

    for slot in returns.success {
        validate_external_signature_type_is_registered(
            external_function,
            &slot.value_type,
            context,
            span,
        )?;
    }

    if let Some(error_type) = returns.error {
        validate_external_signature_type_is_registered(
            external_function,
            error_type,
            context,
            span,
        )?;
    }

    Ok(())
}

/// Checks that a single external signature type is known to the frontend.
fn validate_external_signature_type_is_registered(
    external_function: &ExternalFunctionDef,
    signature_type: &ExternalSignatureType,
    context: &ScopeContext,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    match signature_type {
        ExternalSignatureType::Abi(_)
        | ExternalSignatureType::BuiltinError
        | ExternalSignatureType::StringContent => Ok(()),
        ExternalSignatureType::External(type_id) => {
            if context
                .external_package_registry
                .get_type_by_id(*type_id)
                .is_some()
            {
                return Ok(());
            }

            Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "External function '{}' references unknown external type {:?} at {:?}.",
                    external_function.name, type_id, span
                ),
            )
            .into())
        }
        ExternalSignatureType::Optional(inner) => {
            validate_external_signature_type_is_registered(external_function, inner, context, span)
        }
    }
}

#[cfg(test)]
#[path = "tests/cast_boundary_tests.rs"]
mod cast_boundary_tests;

#[cfg(test)]
#[path = "tests/external_call_float_selection_tests.rs"]
mod external_call_float_selection_tests;
