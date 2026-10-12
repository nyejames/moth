//! Typed propagation, compatible producer selection and recovery-handler parsing.
//!
//! Postfix `!` leaves the enclosing function. Catch is selected by expression completion,
//! after operands, calls and operators have supplied their pending failure facts. It consumes
//! compatible producers once and retains handler failures outward in `ValueBlock::Catch`.
//! Success types remain ordinary values; failure facts do not change operand typing.

use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
    HandledFallibleHostFunctionCallInput,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::failure_classification::{
    explicit_propagation_catch_diagnostic, pending_body_failure_facts,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    DeferredCustomCatchCheck, ExpressionFailureFacts, FailureDisposition,
};
use crate::compiler_frontend::ast::expressions::option_propagation::finish_option_propagation;
use crate::compiler_frontend::ast::generic_functions::numeric_origins::GenericErrorReturn;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredExpression, DeferredExpressionSuffix, DeferredNumericExpression, StatementExpression,
};
use crate::compiler_frontend::ast::numeric_known_shape::KnownShapeId;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

use super::catch_handler::{
    CatchFallibleHandler, CatchFallibleHandlerSite, CatchHandlerBody, DeferredCatchHandler,
    FallibleSuccessContract, ParsedCatchHandler, ReadyCatchSite, RetainedCatchHandler,
    parse_catch_fallible_handler_typed, parse_catch_without_error_binding_typed,
    parse_inline_catch_fallible_handler_typed, parse_inline_catch_without_error_binding_typed,
};
use super::success_types::fallible_success_type_ids;
use super::{EXPRESSION_STAGE, FUNCTION_CALL_STAGE};

pub(crate) struct HandledFallibleCall {
    pub(crate) name: PathId,
    pub(crate) args: Vec<CallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) call_span: Option<SourceSpan>,
}

pub(crate) struct FallibleCallSite {
    pub(crate) call: HandledFallibleCall,
    pub(crate) error_return_type_id: TypeId,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
}

pub(crate) struct HandledFallibleHostCall {
    pub(crate) name: ExternalFunctionId,
    pub(crate) args: Vec<CallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) requires_external_float_validation: bool,
    pub(crate) error_type_id: TypeId,
    pub(crate) call_span: Option<SourceSpan>,
}

pub(crate) struct FallibleHostCallSite {
    pub(crate) call: HandledFallibleHostCall,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
}

struct FallibleHandlingSite<'a> {
    /// The protected value's success contract. `Known(&[])` has zero success slots. `Pending`
    /// keeps a generic call's declared arity while its slot types resolve. `Partial` types its
    /// concrete slots and keeps the rest unknown. `Unresolved` is never zero success values; its
    /// value-requirement checks are deferred to completion against the
    /// resolved site.
    success_contract: FallibleSuccessContract<'a>,
    /// The protected call's error return. A retained error resolves only at completion and never
    /// falls back to the builtin `Error`.
    error_return: GenericErrorReturn,
    value_required: bool,
    value_required_span: Option<SourceSpan>,
    compilation_stage: &'a str,
    allow_boundary_catch: bool,
}

impl HandledFallibleHostCall {
    pub(crate) fn into_expression(
        self,
        handling: FallibleHandling,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        let expression_handling = match &handling {
            FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
            FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
        };
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_host_function_call_with_typed_arguments(
                HandledFallibleHostFunctionCallInput {
                    id: self.name,
                    args: self.args,
                    result_type_ids: self.result_type_ids,
                    requires_external_float_validation: self.requires_external_float_validation,
                    error_type_id: self.error_type_id,
                    handling: expression_handling,
                    span: self.call_span,
                },
                type_environment,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        match handling {
            FallibleHandling::Propagate => function_call_expression,
            FallibleHandling::Handler { .. } => {
                wrap_catch_expression(function_call_expression, handling, result_type_ids)
            }
        }
    }

    /// Builds the value a `catch` on this call protects, plus the call's success slot types.
    ///
    /// WHY: a catch handler body that retained numeric continuations rebuilds at the body root,
    /// so the protected call value must be built here exactly as the catch site builds it.
    pub(crate) fn into_protected_expression(
        self,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> (Expression, Vec<TypeId>) {
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_host_function_call_with_typed_arguments(
                HandledFallibleHostFunctionCallInput {
                    id: self.name,
                    args: self.args,
                    result_type_ids: self.result_type_ids,
                    requires_external_float_validation: self.requires_external_float_validation,
                    error_type_id: self.error_type_id,
                    handling: FallibleExpressionHandling::Recover,
                    span: self.call_span,
                },
                type_environment,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        (function_call_expression, result_type_ids)
    }
}

impl HandledFallibleCall {
    pub(crate) fn into_plain_expression(
        self,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        Expression::function_call_with_typed_arguments(
            self.name,
            self.args,
            self.result_type_ids,
            type_environment,
            self.call_span,
        )
    }

    pub(crate) fn into_expression(
        self,
        handling: FallibleHandling,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        let expression_handling = match &handling {
            FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
            FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
        };
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_function_call_with_typed_arguments(
                self.name,
                self.args,
                self.result_type_ids,
                expression_handling,
                type_environment,
                self.call_span,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        match handling {
            FallibleHandling::Propagate => function_call_expression,
            FallibleHandling::Handler { .. } => {
                wrap_catch_expression(function_call_expression, handling, result_type_ids)
            }
        }
    }

    /// Builds the value a `catch` on this call protects, plus the call's success slot types.
    ///
    /// WHY: a catch handler body that retained numeric continuations rebuilds at the body root,
    /// so the protected call value must be built here exactly as the catch site builds it.
    pub(crate) fn into_protected_expression(
        self,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> (Expression, Vec<TypeId>) {
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_function_call_with_typed_arguments(
                self.name,
                self.args,
                self.result_type_ids,
                FallibleExpressionHandling::Recover,
                type_environment,
                self.call_span,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        (function_call_expression, result_type_ids)
    }
}

/// One parsed fallible-handling suffix: ready where it was parsed, or retained for the body root.
///
/// WHY: a catch handler body shares the enclosing function's numeric domain, so a body that
/// retained numeric continuations can only be rebuilt by that root. Every suffix consumer either
/// attaches the ready handling or hands the retained handler back with its protected value.
pub(crate) enum ParsedFallibleSuffix {
    Ready(FallibleHandling),
    Retained(Box<RetainedCatchHandler>),
}

/// Hands one parsed catch handler to the enclosing numeric body root.
///
/// WHAT: builds the pending-operand deferral whose item is this catch: the protected value exactly
///       as this site protects it, plus its fallible site and retained handler.
/// WHY: the catch attaches only at a boundary (`allow_boundary_catch`), so the retained item is the
///      position's whole authored syntax and the root rebuilds it through the owning constructor.
pub(crate) fn retain_catch_handler(
    handled: Expression,
    site: ReadyCatchSite,
    handler: RetainedCatchHandler,
    span: Option<SourceSpan>,
) -> ExpressionParseError {
    ExpressionParseError::NumericDeferral(Box::new(NumericDeferral {
        items: vec![ExpressionRpnItem::PendingNumericExpression {
            expression: Box::new(DeferredExpression::CatchHandler(Box::new(
                DeferredCatchHandler {
                    handled: StatementExpression::Built(handled),
                    ready_site: Some(site),
                    protected_shape: None,
                    handler,
                },
            ))),
            span,
        }],
        expected: ExpectedType::Infer,
        value_mode: ValueMode::ImmutableOwned,
        span,
        suffix: DeferredExpressionSuffix::None,
        complete_fragment: true,
    }))
}

/// Retains one parsed catch whose protected value still has to resolve.
///
/// WHAT: pairs the protected value's retained fragment with the suffix's parsed handler, ready or
///       retained, as the payload the enclosing body root rebuilds.
///       `protected_shape` is the numeric shape a raw literal fragment opened at retention, when it
///       has one.
/// WHY: a protected value that is itself numeric-dependent has no built expression at its site, so
///      the catch travels with the fragment and rebuilds in one place at the root; `ready_site`
///      stays `None` so completion derives the site from the resolved value exactly as the
///      completed-expression path derives it.
///
/// Returns `None` for a `!` propagation suffix: propagation on a retained call is the site's own
/// continuation, not a catch.
pub(crate) fn retain_pending_catch_handler(
    handled: DeferredNumericExpression,
    protected_shape: Option<KnownShapeId>,
    suffix: ParsedFallibleSuffix,
) -> Option<DeferredCatchHandler> {
    let handler = match suffix {
        ParsedFallibleSuffix::Ready(FallibleHandling::Propagate) => return None,
        ParsedFallibleSuffix::Ready(FallibleHandling::Handler { error, body }) => {
            RetainedCatchHandler {
                error,
                body: CatchHandlerBody::Ready(body),
            }
        }
        ParsedFallibleSuffix::Retained(handler) => *handler,
    };

    Some(DeferredCatchHandler {
        handled: StatementExpression::Retained(handled),
        ready_site: None,
        protected_shape,
        handler,
    })
}

/// One completed fallible carrier's protected recovery value and fallible site.
///
/// WHY: a completed-expression catch and a catch resumed at the body root protect a carrier
///      identically, so the recovery wrapper and its slot facts are derived in one place.
pub(super) struct CarriedCatchSite {
    expression: Expression,
    handled_type_id: TypeId,
    diagnostic_type: DataType,
    error_return_type_id: TypeId,
    success_result_type_ids: Vec<TypeId>,
}

impl CarriedCatchSite {
    /// The catch-protected value and its fallible site: the recovered handled-result view.
    pub(super) fn recovery(self) -> (Expression, TypeId, Vec<TypeId>) {
        let expression_span = self.expression.span;
        let recovered = Expression::handled_result_with_type_id(
            self.expression,
            FallibleExpressionHandling::Recover,
            self.handled_type_id,
            self.diagnostic_type,
            expression_span,
        )
        .with_typed_error_producer(self.error_return_type_id, None);

        (
            recovered,
            self.error_return_type_id,
            self.success_result_type_ids,
        )
    }

    /// The propagation-protected value: the propagating handled-result view of the carrier.
    pub(super) fn propagation(self) -> Expression {
        let expression_span = self.expression.span;
        Expression::handled_result_with_type_id(
            self.expression,
            FallibleExpressionHandling::Propagate,
            self.handled_type_id,
            self.diagnostic_type,
            expression_span,
        )
    }
}

/// The catch-site derivation of one completed expression.
///
/// WHAT: `Carrier` holds the derived recovery site of a fallible carrier; `NotCarrier` hands the
///       untouched expression back for the caller's own non-carrier derivation or diagnostic.
/// WHY: a non-carrier is an ordinary authored case rather than a failure, so it is a named
///      outcome instead of an `Err` payload that carries a whole expression.
pub(super) enum CarriedCatchOutcome {
    Carrier(CarriedCatchSite),
    NotCarrier(Expression),
}

/// Derives the recovery site of a completed fallible carrier.
///
/// WHAT: collapses the carrier's success slots into the handled result type, computes its
///       diagnostic spelling, and keeps the protected value with its slot facts.
/// WHY: a completed-expression catch and a catch resumed at the body root protect a carrier
///      identically, so the recovery facts are derived in one place. A value without carrier
///      slots is handed straight back to the caller's own diagnostic or derivation.
pub(super) fn carried_catch_site(
    expression: Expression,
    type_interner: &mut AstTypeInterner<'_>,
) -> CarriedCatchOutcome {
    let Some((_success_type_ids, error_return_type_id)) = type_interner
        .environment()
        .fallible_carrier_slots(expression.type_id)
    else {
        return CarriedCatchOutcome::NotCarrier(expression);
    };

    // The success-slot type IDs are computed separately because they drive the catch handler's
    // value-production target, not the propagation check.
    let success_result_type_ids =
        fallible_success_type_ids(expression.type_id, type_interner.environment());

    // Collapse single or zero success types into a concrete type for the resulting handled
    // expression. Multi-value successes become a tuple.
    let handled_type_id = match success_result_type_ids.as_slice() {
        [] => type_interner.environment().builtins().none,
        [single] => *single,
        multiple => type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(multiple.to_vec()),
    };

    let diagnostic_type = diagnostic_type_spelling(handled_type_id, type_interner.environment());

    CarriedCatchOutcome::Carrier(CarriedCatchSite {
        expression,
        handled_type_id,
        diagnostic_type,
        error_return_type_id,
        success_result_type_ids,
    })
}

/// Derives the catch site of a completed expression that is not a fallible carrier.
///
/// WHAT: selects the expression's compatible error producer and the success slot types of its
///       result kind, replacing the expression when the producer selection rewrote it.
/// WHY: both the completed-expression catch and a catch resumed at the body root attach to the
///      same derived site, so the derivation lives here instead of in two parser paths.
pub(super) fn completed_catch_attachment(
    mut expression: Expression,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<(Expression, TypeId, Vec<TypeId>), ExpressionParseError> {
    let error_type_id = compatible_expression_error_type(
        &mut expression,
        context,
        type_interner.environment(),
        string_table,
        span,
    )?;
    let result_type_ids = match &expression.kind {
        ExpressionKind::FunctionCall {
            result_type_ids, ..
        }
        | ExpressionKind::HostFunctionCall {
            result_type_ids, ..
        }
        | ExpressionKind::MethodCall {
            result_type_ids, ..
        }
        | ExpressionKind::CollectionBuiltinCall {
            result_type_ids, ..
        }
        | ExpressionKind::MapBuiltinCall {
            result_type_ids, ..
        }
        | ExpressionKind::HandledFallibleFunctionCall {
            result_type_ids, ..
        }
        | ExpressionKind::HandledFallibleHostFunctionCall {
            result_type_ids, ..
        } => result_type_ids.clone(),
        ExpressionKind::HandledFallibleExpression { value, .. }
            if type_interner
                .environment()
                .is_fallible_carrier(value.type_id) =>
        {
            fallible_success_type_ids(value.type_id, type_interner.environment())
        }
        _ if expression.type_id == type_interner.environment().builtins().none => Vec::new(),
        _ => vec![expression.type_id],
    };

    Ok((expression, error_type_id, result_type_ids))
}

/// Resolve one handler only after ordinary operators and nested producers have completed.
#[allow(
    clippy::too_many_arguments,
    reason = "completed recovery keeps the shared parser resources and selected expression separate"
)]
pub(crate) fn parse_completed_expression_catch(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: Expression,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    if !allow_boundary_catch || !fallible_catch_allowed_in_context(context) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOutsideBoundary,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Existing first-class carriers still use their established recovery representation.
    if type_interner
        .environment()
        .is_fallible_carrier(expression.type_id)
    {
        let value_required = expression.type_id != type_interner.environment().builtins().none;
        return parse_fallible_handling_suffix_for_expression(
            token_stream,
            context,
            type_interner,
            expression,
            value_required,
            true,
            string_table,
            path_fork,
        );
    }

    let (expression, error_type_id, result_type_ids) = completed_catch_attachment(
        expression,
        context,
        type_interner,
        string_table,
        Some(token_stream.current_span()),
    )?;
    let handling = parse_catch_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract: FallibleSuccessContract::Known(&result_type_ids),
            error_return: GenericErrorReturn::Concrete(error_type_id),
            value_required: !result_type_ids.is_empty(),
            value_required_span: expression.span,
            compilation_stage: EXPRESSION_STAGE,
            allow_boundary_catch: true,
        },
        None,
        string_table,
        path_fork,
    )?;
    let handling = match handling {
        ParsedFallibleSuffix::Ready(handling) => handling,
        // The handler body retained numeric continuations: protect the completed expression as
        // this site protects it and let the body root rebuild the pair.
        ParsedFallibleSuffix::Retained(handler) => {
            let handled_span = expression.span;
            return Err(retain_catch_handler(
                expression,
                ReadyCatchSite {
                    error_return_type_id: error_type_id,
                    success_result_type_ids: result_type_ids,
                },
                *handler,
                handled_span,
            ));
        }
    };
    let mut recovered = wrap_catch_expression(expression, handling, result_type_ids);
    complete_catch_failure_fact(&mut recovered, error_type_id, context)?;
    Ok(recovered)
}

pub(crate) fn complete_catch_failure_fact(
    expression: &mut Expression,
    error_type_id: TypeId,
    context: &ScopeContext,
) -> Result<(), ExpressionParseError> {
    let ExpressionKind::ValueBlock { block } = &mut expression.kind else {
        return Ok(());
    };
    let ValueBlock::Catch(catch) = block.as_mut() else {
        return Ok(());
    };
    catch.handled_value.failure_facts.disposition =
        FailureDisposition::HandledByCatch { error_type_id };
    expression.failure_facts = if let FallibleHandling::Handler { body, .. } = &catch.handler {
        let handler_facts = pending_body_failure_facts(body, &context.template_ir_store.borrow())?;
        if catch
            .handled_value
            .failure_facts
            .is_statically_infallible_catch()
        {
            // The handler remains semantically checked but has no executable failure edge.
            ExpressionFailureFacts::default()
        } else {
            handler_facts
        }
    } else {
        ExpressionFailureFacts::default()
    };
    Ok(())
}

pub(crate) fn compatible_expression_error_type(
    expression: &mut Expression,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    catch_span: Option<SourceSpan>,
) -> Result<TypeId, ExpressionParseError> {
    if let Some(diagnostic) =
        explicit_propagation_catch_diagnostic(expression, &context.template_ir_store.borrow())?
    {
        return Err(diagnostic.into());
    }

    let facts = &expression.failure_facts;
    let first = facts.summary.first_typed;
    if let Some(first) = first {
        if let Some(producer) = facts.summary.conflicting_typed {
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                    first_error_type_id: first.error_type_id,
                    second_error_type_id: producer.error_type_id,
                    first_producer_span: first.span,
                    second_producer_span: producer.span,
                },
                catch_span,
            )
            .into());
        }

        if facts.summary.first_implicit.is_some() {
            let builtin_error_type_id =
                resolve_builtin_error_type_typed(context, expression.span, string_table)?.type_id;
            if !is_postfix_error_compatible(
                builtin_error_type_id,
                first.error_type_id,
                type_environment,
            ) {
                if let Some(producer) = facts.summary.first_numeric {
                    return Err(CompilerDiagnostic::invalid_fallible_handling(
                        InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                            error_type_id: first.error_type_id,
                            typed_producer_span: first.span,
                            implicit_producer_span: producer.span,
                        },
                        catch_span,
                    )
                    .into());
                }

                let check = DeferredCustomCatchCheck {
                    catch_span,
                    error_type_id: first.error_type_id,
                    typed_producer_span: first.span,
                };
                let error_type_id = first.error_type_id;
                expression.failure_facts.deferred_custom_catch = Some(check);
                return Ok(error_type_id);
            }
        }
        return Ok(first.error_type_id);
    }

    if facts.has_authored_safety_catch_eligibility() {
        return Ok(
            resolve_builtin_error_type_typed(context, expression.span, string_table)?.type_id,
        );
    }

    Err(CompilerDiagnostic::invalid_fallible_handling(
        super::non_fallible_handler_reason(
            TokenTag::CATCH,
            type_environment.is_option(expression.type_id),
        ),
        catch_span,
    )
    .into())
}

#[allow(
    clippy::too_many_arguments,
    reason = "fallible suffix parsing keeps the token stream, scope, mutable interner/string/path state, the parsed expression, and the value/catch policy flags as separate borrows"
)]
pub(crate) fn parse_fallible_handling_suffix_for_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: Expression,
    value_required: bool,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let site = match carried_catch_site(expression, type_interner) {
        CarriedCatchOutcome::Carrier(site) => site,
        CarriedCatchOutcome::NotCarrier(expression) => {
            let type_environment = type_interner.environment();
            let operand_is_optional = type_environment.is_option(expression.type_id);
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                super::non_fallible_handler_reason(token_stream.current_tag(), operand_is_optional),
                Some(token_stream.current_span()),
            )
            .into());
        }
    };
    let site_span = site.expression.span;

    let propagation_span = (token_stream.current_tag() == TokenTag::BANG)
        .then(|| token_stream.current_postfix_operator_span());

    let handling = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract: FallibleSuccessContract::Known(&site.success_result_type_ids),
            error_return: GenericErrorReturn::Concrete(site.error_return_type_id),
            value_required,
            value_required_span: site_span,
            compilation_stage: EXPRESSION_STAGE,
            allow_boundary_catch,
        },
        None,
        string_table,
        path_fork,
    )?;

    match handling {
        None => Ok(site.expression),

        Some(ParsedFallibleSuffix::Ready(FallibleHandling::Propagate)) => {
            Ok(site.propagation().with_propagation_span(Some(
                propagation_span.expect("propagation handling must have a postfix span"),
            )))
        }

        Some(ParsedFallibleSuffix::Ready(handler @ FallibleHandling::Handler { .. })) => {
            let (handled, error_return_type_id, success_result_type_ids) = site.recovery();

            let mut recovered = wrap_catch_expression(handled, handler, success_result_type_ids);
            complete_catch_failure_fact(&mut recovered, error_return_type_id, context)?;
            Ok(recovered)
        }

        // The handler body retained numeric continuations: protect the carrier as its recovery
        // representation and let the body root rebuild the pair.
        Some(ParsedFallibleSuffix::Retained(handler)) => {
            let (handled, error_return_type_id, success_result_type_ids) = site.recovery();
            Err(retain_catch_handler(
                handled,
                ReadyCatchSite {
                    error_return_type_id,
                    success_result_type_ids,
                },
                *handler,
                site_span,
            ))
        }
    }
}

/// Returns whether `catch` handlers are syntactically permitted in the given scope context.
///
/// WHY: catch introduces a statement-like body block, so it is forbidden inside expression-only
/// contexts (conditions, templates, constants) where statements are not allowed.
pub(crate) fn fallible_catch_allowed_in_context(context: &ScopeContext) -> bool {
    !matches!(
        context.kind,
        ContextKind::Expression
            | ContextKind::Condition
            | ContextKind::Template
            | ContextKind::Constant
            | ContextKind::ConstantHeader
    )
}

fn parse_fallible_handling_suffix(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<ParsedFallibleSuffix>, ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::BANG => {
            parse_postfix_propagation(token_stream, context, site, type_interner.environment())
                .map(|handling| Some(ParsedFallibleSuffix::Ready(handling)))
        }
        TokenTag::CATCH => parse_catch_handling_suffix(
            token_stream,
            context,
            type_interner,
            site,
            warnings,
            string_table,
            path_fork,
        )
        .map(Some),
        _ => Ok(None),
    }
}

fn parse_postfix_propagation(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    site: FallibleHandlingSite<'_>,
    type_environment: &TypeEnvironment,
) -> Result<FallibleHandling, ExpressionParseError> {
    let propagation_span = token_stream.current_postfix_operator_span();
    token_stream.advance();

    let Some(expected_error_type_id) = context.expected_error_type else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
            Some(propagation_span),
        )
        .into());
    };

    // A retained error has no resolved type yet; its compatibility is checked where the producer
    // completes, so only a concrete error is compared here.
    if let GenericErrorReturn::Concrete(error_return_type_id) = site.error_return
        && !is_postfix_error_compatible(
            expected_error_type_id,
            error_return_type_id,
            type_environment,
        )
    {
        return Err(CompilerDiagnostic::type_mismatch(
            expected_error_type_id,
            error_return_type_id,
            TypeMismatchContext::ErrorReturn,
            Some(propagation_span),
        )
        .into());
    }

    Ok(FallibleHandling::Propagate)
}

/// Wraps value-producing catch recovery in the shared `ValueBlock` expression shape.
///
/// WHAT: only `catch` handlers become value blocks; postfix propagation stays an ordinary
/// handled fallible expression because it leaves the current function instead of recovering.
/// WHY: this keeps catch recovery on the same AST/HIR model as value `if` and match blocks
/// without changing statement-only catch behavior.
pub(crate) fn wrap_catch_expression(
    handled_expression: Expression,
    handler: FallibleHandling,
    result_type_ids: Vec<TypeId>,
) -> Expression {
    debug_assert!(matches!(handler, FallibleHandling::Handler { .. }));

    let span = handled_expression.span;
    let result_type_id = handled_expression.type_id;
    let diagnostic_type = handled_expression.diagnostic_type.to_owned();

    Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::Catch(ValueCatchBlock {
                handled_value: Box::new(handled_expression),
                handler,
                result_type_ids,
            })),
        },
        span,
        result_type_id,
        diagnostic_type,
        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
    )
}

fn parse_catch_handling_suffix(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedFallibleSuffix, ExpressionParseError> {
    if !site.allow_boundary_catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOutsideBoundary,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();
    let catch_context = context.activate_pending_catch_assignment_targets();

    match token_stream.current_tag() {
        // `catch then ...` — inline value-producing fallback with no error binding.
        TokenTag::THEN => parse_inline_catch_without_error_binding(
            token_stream,
            &catch_context,
            type_interner,
            site,
            string_table,
            path_fork,
        ),

        // `catch:` — no error binding, just a fallback body.
        TokenTag::COLON => parse_catch_without_error_binding(
            token_stream,
            &catch_context,
            type_interner,
            site,
            warnings,
            string_table,
            path_fork,
        ),

        // `catch |err|:` or `catch |err| then ...` — bind the error value before recovery.
        TokenTag::TYPE_PARAMETER_BRACKET => parse_catch_handler(
            token_stream,
            &catch_context,
            type_interner,
            site,
            warnings,
            string_table,
            path_fork,
        ),

        _ => Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

fn parse_inline_catch_without_error_binding(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedFallibleSuffix, ExpressionParseError> {
    let handler = parse_inline_catch_without_error_binding_typed(
        token_stream,
        context,
        type_interner,
        CatchFallibleHandlerSite {
            success_contract: site.success_contract,
            error_return: site.error_return,
            value_required: site.value_required,
            compilation_stage: site.compilation_stage,
            value_required_span: site.value_required_span,
        },
        string_table,
        path_fork,
    )?;

    Ok(match handler {
        ParsedCatchHandler::Ready(CatchFallibleHandler { error, body }) => {
            ParsedFallibleSuffix::Ready(FallibleHandling::Handler { error, body })
        }
        // The fallback still holds numeric origins: the handler travels to the body root with the
        // protected value instead of attaching here.
        ParsedCatchHandler::Retained(handler) => ParsedFallibleSuffix::Retained(handler),
    })
}

/// Delegates to `parse_catch_without_error_binding_typed` for `catch:` (no error binding).
fn parse_catch_without_error_binding(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedFallibleSuffix, ExpressionParseError> {
    let handler = parse_catch_without_error_binding_typed(
        token_stream,
        context,
        type_interner,
        CatchFallibleHandlerSite {
            success_contract: site.success_contract,
            error_return: site.error_return,
            value_required: site.value_required,
            compilation_stage: site.compilation_stage,
            value_required_span: site.value_required_span,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    Ok(match handler {
        ParsedCatchHandler::Ready(CatchFallibleHandler { error, body }) => {
            ParsedFallibleSuffix::Ready(FallibleHandling::Handler { error, body })
        }
        ParsedCatchHandler::Retained(handler) => ParsedFallibleSuffix::Retained(handler),
    })
}

/// Delegates to the block or inline binding parser for `catch |err|`.
fn parse_catch_handler(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedFallibleSuffix, ExpressionParseError> {
    let handler_site = CatchFallibleHandlerSite {
        success_contract: site.success_contract,
        error_return: site.error_return,
        value_required: site.value_required,
        compilation_stage: site.compilation_stage,
        value_required_span: site.value_required_span,
    };

    let handler = if next_catch_binding_is_inline(token_stream) {
        // The inline parser owns its own outcome: a fallback whose numeric origins are still
        // unresolved returns the retained handler for the body root.
        parse_inline_catch_fallible_handler_typed(
            token_stream,
            context,
            type_interner,
            handler_site,
            warnings,
            string_table,
            path_fork,
        )?
    } else {
        parse_catch_fallible_handler_typed(
            token_stream,
            context,
            type_interner,
            handler_site,
            warnings,
            string_table,
            path_fork,
        )?
    };

    Ok(match handler {
        ParsedCatchHandler::Ready(CatchFallibleHandler { error, body }) => {
            ParsedFallibleSuffix::Ready(FallibleHandling::Handler { error, body })
        }
        ParsedCatchHandler::Retained(handler) => ParsedFallibleSuffix::Retained(handler),
    })
}

fn next_catch_binding_is_inline(token_stream: &AstCursor) -> bool {
    let mut index = token_stream.position().checked_add(1);
    while let Some(probe) = index {
        if probe >= token_stream.length() {
            return false;
        }
        let Some(token) = token_stream.token_ref_at(probe) else {
            return false;
        };
        match token.tag() {
            TokenTag::TYPE_PARAMETER_BRACKET => {
                let mut lookahead = probe.checked_add(1);
                while let Some(next) = lookahead {
                    if next >= token_stream.length() {
                        return false;
                    }
                    let Some(token) = token_stream.token_ref_at(next) else {
                        return false;
                    };
                    match token.tag() {
                        TokenTag::NEWLINE => lookahead = next.checked_add(1),
                        tag => return tag == TokenTag::THEN,
                    }
                }
                return false;
            }

            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => return false,

            _ => index = probe.checked_add(1),
        }
    }
    false
}

// --------------------------
//  Catch handler call parsing
// --------------------------

pub(crate) fn parse_fallible_handling_suffix_for_call_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    handler_call: FallibleCallSite,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let FallibleCallSite {
        call,
        error_return_type_id,
        value_required,
        allow_boundary_catch,
    } = handler_call;

    let propagation_span = (token_stream.current_tag() == TokenTag::BANG)
        .then(|| token_stream.current_postfix_operator_span());

    let handling = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract: FallibleSuccessContract::Known(&call.result_type_ids),
            error_return: GenericErrorReturn::Concrete(error_return_type_id),
            value_required,
            value_required_span: call.call_span,
            compilation_stage: FUNCTION_CALL_STAGE,
            allow_boundary_catch,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    let Some(handling) = handling else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    };

    Ok(match handling {
        ParsedFallibleSuffix::Ready(handling) => call.into_expression(
            handling,
            propagation_span,
            type_interner.environment_mut_for_derived_types(),
        ),
        // The handler body retained numeric continuations: protect this call exactly as its catch
        // site protects it and let the body root rebuild the pair.
        ParsedFallibleSuffix::Retained(handler) => {
            let call_span = call.call_span;
            let (protected, success_result_type_ids) = call.into_protected_expression(
                propagation_span,
                type_interner.environment_mut_for_derived_types(),
            );
            return Err(retain_catch_handler(
                protected,
                ReadyCatchSite {
                    error_return_type_id,
                    success_result_type_ids,
                },
                *handler,
                call_span,
            ));
        }
    })
}

pub(crate) fn parse_fallible_handling_suffix_for_host_call_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    handler_call: FallibleHostCallSite,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let FallibleHostCallSite {
        call,
        value_required,
        allow_boundary_catch,
    } = handler_call;

    let propagation_span = (token_stream.current_tag() == TokenTag::BANG)
        .then(|| token_stream.current_postfix_operator_span());

    let handling = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract: FallibleSuccessContract::Known(&call.result_type_ids),
            error_return: GenericErrorReturn::Concrete(call.error_type_id),
            value_required,
            value_required_span: call.call_span,
            compilation_stage: FUNCTION_CALL_STAGE,
            allow_boundary_catch,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    let Some(handling) = handling else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    };

    Ok(match handling {
        ParsedFallibleSuffix::Ready(handling) => call.into_expression(
            handling,
            propagation_span,
            type_interner.environment_mut_for_derived_types(),
        ),
        // The handler body retained numeric continuations: protect this call exactly as its catch
        // site protects it and let the body root rebuild the pair.
        ParsedFallibleSuffix::Retained(handler) => {
            let call_span = call.call_span;
            let error_return_type_id = call.error_type_id;
            let (protected, success_result_type_ids) = call.into_protected_expression(
                propagation_span,
                type_interner.environment_mut_for_derived_types(),
            );
            return Err(retain_catch_handler(
                protected,
                ReadyCatchSite {
                    error_return_type_id,
                    success_result_type_ids,
                },
                *handler,
                call_span,
            ));
        }
    })
}

/// Applies one authored handling suffix that a deferral site consumed before the value existed.
///
/// WHAT: `None` leaves the value unchanged; an authored `!` rebuilds exactly the propagating
///       handled-result shape the parse-time carrier path builds, reporting the same non-fallible
///       operand, missing error-slot and error-return mismatch diagnostics at the authored suffix
///       span (or the value's own span when the suffix carried none); an authored `?` is applied by
///       the option-propagation owner's finisher, so both paths share one option rule.
/// WHY: a retained fragment is completed later than its suffix was consumed, so the suffix has to
///      be applied in one owner instead of every completion lane re-deriving postfix semantics.
pub(crate) fn apply_completed_handling_suffix(
    value: Expression,
    suffix: DeferredExpressionSuffix,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<Expression, ExpressionParseError> {
    let propagation_span = match suffix {
        DeferredExpressionSuffix::None => return Ok(value),
        DeferredExpressionSuffix::Propagate { span } => span.or(value.span),
        // The `?` token was consumed before the operand existed, so the authored optional
        // propagation replays through the shared finisher that owns the option rules.
        DeferredExpressionSuffix::OptionalPropagate { span } => {
            return finish_option_propagation(value, span, context, type_interner);
        }
    };

    let value_type_id = value.type_id;

    let CarriedCatchOutcome::Carrier(site) = carried_catch_site(value, type_interner) else {
        // Propagation needs a fallible carrier; the completed value has none, exactly as the
        // parse-time path reports for the same authored `!`.
        let operand_is_optional = type_interner.environment().is_option(value_type_id);
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            super::non_fallible_handler_reason(TokenTag::BANG, operand_is_optional),
            propagation_span,
        )
        .into());
    };

    let Some(expected_error_type_id) = context.expected_error_type else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
            propagation_span,
        )
        .into());
    };

    if !is_postfix_error_compatible(
        expected_error_type_id,
        site.error_return_type_id,
        type_interner.environment(),
    ) {
        return Err(CompilerDiagnostic::type_mismatch(
            expected_error_type_id,
            site.error_return_type_id,
            TypeMismatchContext::ErrorReturn,
            propagation_span,
        )
        .into());
    }

    Ok(site.propagation().with_propagation_span(propagation_span))
}

/// Reports the ordinary non-fallible handler reason for a producer with no error slot.
///
/// WHAT: the exact diagnostic a completed call reports when the authored `!`/`catch` meets a
///       producer whose signature carries no error slot, including its optional-operand wording.
/// WHY: the completed finishers and the retained-call lane must report one authored mistake in one
///      way, so the reason selection lives with the fallible owner instead of being re-derived.
pub(crate) fn non_fallible_call_handler_diagnostic(
    token_stream: &AstCursor,
    success_result_type_ids: &[TypeId],
    handler_tag: TokenTag,
    type_environment: &TypeEnvironment,
) -> CompilerDiagnostic {
    let operand_is_optional =
        super::call_success_is_optional(success_result_type_ids, type_environment);
    CompilerDiagnostic::invalid_fallible_handling(
        super::non_fallible_handler_reason(handler_tag, operand_is_optional),
        Some(token_stream.current_span()),
    )
}

/// Parses one boundary fallible-handling suffix whose protected value still has to resolve.
///
/// WHAT: consumes the authored `!` or `catch` suffix against the receiving site the fragment's
///       position supplies and returns it ready or retained (`None` when no suffix is authored).
///       A site without a declared error slot passes `None` and reports `authored_failure_lane`.
///       `success_contract` is `Known` when the fragment's success slots are resolved, `Pending`
///       while a generic call's arity is fixed, `Partial` when some slots are concrete, and
///       `Unresolved` when the selected
///       fragment has no contract yet: an unresolved contract is never zero success values, its
///       handler body parses through the provisional site (raw numeric fallbacks are retained and
///       slot typing is inferred from the computed result), and its value-requirement checks run
///       at completion against the resolved site.
/// WHY: a fragment that stays pending protects a value that does not exist yet, so its handler has
///      to travel to the body root with the fragment instead of attaching here. This is the single
///      suffix decision for retained positions: call sites map `Ready(Propagate)` to their own
///      deferred `!` metadata and feed `Ready(Handler)`/`Retained` through the catch conversion,
///      so no site keeps a second copy of the `!`/`catch` legality rules. An infallible callee keeps
///      the exact completed-path behaviour: `!` reports the ordinary non-fallible handler reason,
///      and `catch` recovers only when the protected value carries its own authored failure lane,
///      binding the same builtin error type the completed path derives.
#[allow(
    clippy::too_many_arguments,
    reason = "a fragment suffix names the receiving site's facts alongside the shared parser resources"
)]
pub(crate) fn parse_fragment_handling_suffix(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    success_contract: FallibleSuccessContract<'_>,
    error_return: Option<GenericErrorReturn>,
    authored_failure_lane: bool,
    value_required: bool,
    value_required_span: Option<SourceSpan>,
    compilation_stage: &str,
    allow_boundary_catch: bool,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<ParsedFallibleSuffix>, ExpressionParseError> {
    // This entry is its own receiving site: a retained `catch` here parses its handler while the
    // assignment's right-hand side is still being built, so the pending assignment targets must be
    // unavailable exactly as `parse_catch` makes them at its entry. Activating once here covers
    // every caller — source calls, receiver methods and the retained fragment path — instead of
    // each call site having to remember it.
    let context = &context.activate_pending_catch_assignment_targets();

    let Some(error_return) = error_return else {
        if token_stream.current_tag() != TokenTag::CATCH {
            // The callee declares no error slot: `!` is the ordinary non-fallible handler mistake,
            // exactly as the completed call path reports it for the same authored fact.
            if super::token_stream_starts_typed_propagation_suffix(token_stream) {
                return Err(non_fallible_call_handler_diagnostic(
                    token_stream,
                    success_contract.known().unwrap_or(&[]),
                    token_stream.current_tag(),
                    type_interner.environment(),
                )
                .into());
            }

            return Ok(None);
        }

        if !authored_failure_lane {
            // No declared error slot and no authored failure of its own: the ordinary non-fallible
            // catch reason, identical to the completed expression's compatibility check.
            return Err(non_fallible_call_handler_diagnostic(
                token_stream,
                success_contract.known().unwrap_or(&[]),
                TokenTag::CATCH,
                type_interner.environment(),
            )
            .into());
        }

        // The protected value's own authored failure lane supplies the error, so the handler binds
        // the builtin error type exactly as the completed path derives it.
        let error_type_id = resolve_builtin_error_type_typed(
            context,
            Some(token_stream.current_span()),
            string_table,
        )?
        .type_id;

        return parse_fallible_handling_suffix(
            token_stream,
            context,
            type_interner,
            FallibleHandlingSite {
                success_contract,
                error_return: GenericErrorReturn::Concrete(error_type_id),
                value_required,
                value_required_span,
                compilation_stage,
                allow_boundary_catch,
            },
            warnings,
            string_table,
            path_fork,
        );
    };

    parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract,
            error_return,
            value_required,
            value_required_span,
            compilation_stage,
            allow_boundary_catch,
        },
        warnings,
        string_table,
        path_fork,
    )
}

/// Input bundle for `parse_cast_catch_handling_suffix`.
///
/// WHAT: carries the success type and boundary permission for a `cast ... catch:` handler.
/// WHY: cast failure is not a `Result`-typed value, so the shared fallible-handling parser
///      needs a small, cast-specific site description.
pub(crate) struct CastCatchSite {
    pub(crate) success_type_id: TypeId,
    pub(crate) error_type_id: TypeId,
    pub(crate) value_required_span: Option<SourceSpan>,
    pub(crate) allow_boundary_catch: bool,
}

/// Parses a `catch` recovery suffix for a fallible `cast` expression and attaches it.
///
/// WHAT: reuses the shared catch-handler parser with a single success slot (the cast target) and
///      the cast failure error type, then attaches a ready handler to the protected cast or hands
///      a retained handler body to the enclosing numeric body root.
/// WHY: cast recovery uses the same surface syntax as fallible calls, but the error value is
///      supplied by the selected cast evidence rather than a Result carrier, and the cast
///      dispatch owns the protected expression.
pub(crate) fn parse_cast_catch_handling_suffix(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CastCatchSite,
    cast_expression: Expression,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let handler = parse_catch_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_contract: FallibleSuccessContract::Known(&[site.success_type_id]),
            error_return: GenericErrorReturn::Concrete(site.error_type_id),
            value_required: true,
            value_required_span: site.value_required_span,
            compilation_stage: EXPRESSION_STAGE,
            allow_boundary_catch: site.allow_boundary_catch,
        },
        None,
        string_table,
        path_fork,
    )?;

    let cast_span = cast_expression.span;

    match handler {
        ParsedFallibleSuffix::Ready(handler) => {
            let mut recovered =
                wrap_catch_expression(cast_expression, handler, vec![site.success_type_id]);
            complete_catch_failure_fact(&mut recovered, site.error_type_id, context)?;
            Ok(recovered)
        }

        // The handler body retained numeric continuations: protect the cast exactly as this site
        // protects it and let the body root rebuild the pair.
        ParsedFallibleSuffix::Retained(handler) => Err(retain_catch_handler(
            cast_expression,
            ReadyCatchSite {
                error_return_type_id: site.error_type_id,
                success_result_type_ids: vec![site.success_type_id],
            },
            *handler,
            cast_span,
        )),
    }
}
