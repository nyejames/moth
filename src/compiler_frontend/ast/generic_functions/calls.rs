//! Generic free-function call inference.
//!
//! WHAT: infers concrete type arguments for visible generic function calls, substitutes the
//! template signature, and records a request for AST emission to materialize the concrete body in
//! the consuming module.
//! WHY: generic solving belongs in AST before HIR; backends must only see ordinary concrete
//! function calls.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallArgument, order_call_argument_refs_by_retained_slot,
};
use crate::compiler_frontend::ast::expressions::call_arguments::{
    ParsedCallArguments, parse_generic_call_arguments_typed,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, ExpectedParameterType,
    ParameterExpectation, expectations_from_user_parameters, generic_template_expectations,
    resolve_call_arguments, resolve_call_arguments_shape_and_access,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::expressions::function_calls::{
    ParsedFunctionCallExpression, RetainedCallHandlingInput, parse_retained_call_authored_handling,
    retained_pending_call_operand,
};
use crate::compiler_frontend::ast::generic_bounds::{
    BoundEvidenceSelection, evidence_for_type, evidence_target_is_visible,
    generated_evidence_pair_is_selected, generic_parameter_declares_bound,
};
use crate::compiler_frontend::ast::generic_functions::diagnostics::{
    cannot_infer_generic_function_arguments, conflicting_generic_function_argument,
    missing_generic_function_trait_evidence, recursive_generic_function_instantiation,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::{
    GenericErrorReturn, GenericNumericSignatureInput, link_generic_numeric_signature,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstanceKey, GenericFunctionInstantiationRequest, GenericFunctionTemplate,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredCallHandling, DeferredExpressionCompletion,
    DeferredExpressionServices, DeferredGenericCall, merged_receiver_type, numeric_witness_type,
};
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShape, materialise_known_shape};
use crate::compiler_frontend::ast::statements::fallible_handling::{
    FallibleCallSite, HandledFallibleCall, call_success_is_optional, non_fallible_handler_reason,
    parse_fallible_handling_suffix_for_call_expression,
    token_stream_starts_typed_propagation_suffix,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::generic_bindings::{BindingConflict, GenericTypeBindings};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::datatypes::{diagnostic_type_spelling, environment::TypeEnvironment};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use rustc_hash::FxHashMap;

/// Input bundle for generic call inference.
pub(crate) struct GenericFunctionCallParseInput<'a, 'b, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) context: &'a ScopeContext,
    pub(crate) expected_context: GenericCallExpectedContext<'a>,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Expected-result evidence available to generic free-function inference.
///
/// WHAT: distinguishes a direct receiving-site type from the absence of
/// expected-result evidence.
/// WHY: generic calls may infer from immediate declaration/return/then
/// boundaries, but not from later use or from an outer function parameter when
/// the generic call is nested inside another ordinary argument expression.
#[derive(Clone, Copy)]
pub(crate) enum GenericCallExpectedContext<'a> {
    ImmediateResult(&'a [TypeId]),
    None,
}

struct GenericFunctionCallFinishInput<'a, 'b, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    context: &'a ScopeContext,
    call: HandledFallibleCall,
    declaration_path: PathId,
    error_return_type_id: Option<TypeId>,
    value_required: bool,
    allow_boundary_catch: bool,
    warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    type_interner: &'a mut AstTypeInterner<'b>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Which lane a generic call is parsed in.
///
/// WHAT: an instance call infers, validates, resolves, and publishes an instantiation request; a
///       template-validation call checks only the template body's own call shapes.
/// WHY: validating a generic body must never instantiate it, so template calls never publish.
#[derive(Clone, Copy)]
enum GenericCallLane {
    Instance,
    TemplateValidation,
}

fn parse_generic_call_in_lane(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
    lane: GenericCallLane,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let parameter_expectations = generic_template_expectations(
        &input.template.signature.parameters,
        input.path_fork,
        input.type_interner.environment(),
    );
    let producer_start = input.context.numeric_state().forward_producer_checkpoint();
    let parsed_arguments = parse_generic_call_arguments_typed(
        input.token_stream,
        input.context,
        input.type_interner,
        input.string_table,
        input.path_fork.component(input.template.function_path),
        &parameter_expectations,
        input.path_fork,
    )?;
    let owned_producers =
        producer_start..input.context.numeric_state().forward_producer_checkpoint();

    let raw_arguments = match parsed_arguments {
        ParsedCallArguments::Ready(raw_arguments) => raw_arguments,
        ParsedCallArguments::Retained(arguments) => {
            return retain_generic_call(input, arguments, owned_producers, lane);
        }
    };

    let expression = match lane {
        GenericCallLane::Instance => parse_ready_instance_call(input, raw_arguments)?,
        GenericCallLane::TemplateValidation => validate_ready_template_call(input, raw_arguments)?,
    };
    Ok(ParsedFunctionCallExpression::Value(expression))
}

/// Retains one generic call whose argument values still depend on pending numeric resolution.
///
/// WHAT: links the callee's numeric origins into result shapes and an error slot, settles the
///       authored `!` against that error slot, reserves the instantiation ticket in the instance
///       lane, and stores the call as a pending operand for body completion.
/// WHY: the call's source position and request ticket must be fixed here, so the deferred
///      instantiation publishes in parse order once its numeric arguments resolve.
fn retain_generic_call(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
    arguments: Vec<DeferredCallArgument>,
    owned_producers: std::ops::Range<usize>,
    lane: GenericCallLane,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    let GenericFunctionCallParseInput {
        token_stream,
        template,
        context,
        expected_context,
        value_required,
        allow_boundary_catch,
        call_span,
        warnings,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let numeric_signature = link_generic_numeric_signature(GenericNumericSignatureInput {
        template,
        arguments: &arguments,
        expected_context,
        context,
        type_interner,
        path_fork,
        call_span,
        owned_producers,
        known_receiver_shape: None,
    })?;
    let expected_result_type_ids = match expected_context {
        GenericCallExpectedContext::ImmediateResult(type_ids) => {
            Some(type_ids.to_vec().into_boxed_slice())
        }
        GenericCallExpectedContext::None => None,
    };
    let success_result_type_ids = template.signature.success_return_type_ids();
    let authored_handling = parse_retained_call_authored_handling(RetainedCallHandlingInput {
        token_stream,
        context,
        type_interner,
        success_result_type_ids: &success_result_type_ids,
        error_return: numeric_signature.error_return,
        authored_failure_lane: context.source_call_has_private_failure_lane(template.function_path),
        value_required,
        call_span,
        allow_boundary_catch,
        warnings,
        string_table,
        path_fork,
    })?;

    // Reserved last so a parse failure above never strands a pending ticket.
    let ticket = match lane {
        GenericCallLane::Instance => Some(context.reserve_pending_generic_request()),
        GenericCallLane::TemplateValidation => None,
    };
    let call = DeferredCall::Generic(Box::new(DeferredGenericCall {
        template: template.clone(),
        arguments,
        call_span,
        authored_handling,
        result_shapes: numeric_signature.result_shapes,
        expected_result_type_ids,
        error_return: numeric_signature.error_return,
        ticket,
    }));
    Ok(retained_pending_call_operand(call, call_span))
}

fn parse_ready_instance_call(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
    raw_arguments: Vec<CallArgument>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallParseInput {
        token_stream,
        template,
        context,
        expected_context,
        value_required,
        allow_boundary_catch,
        call_span,
        warnings,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: &raw_arguments,
        expected_context,
        call_span,
        known_receiver_type_id: None,
        known_receiver_span: None,
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
        path_fork,
    })?;
    let selected_evidence = validate_generic_function_bound_evidence(
        template,
        inference.key.type_arguments.as_ref(),
        context,
        type_interner.environment(),
        path_fork,
        call_span,
    )?;

    if context.is_generic_function_instantiation_active(&inference.key) {
        return Err(recursive_generic_function_instantiation(
            path_fork.component(template.function_path),
            call_span,
        )
        .into());
    }
    let callee_name = path_fork
        .component(template.function_path)
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations =
        expectations_from_user_parameters(&inference.signature.parameters, path_fork);
    let type_check_context = type_interner.type_check_context();
    let arguments = resolve_call_arguments(
        CallDiagnosticContext::function(&callee_name),
        &raw_arguments,
        &expectations,
        call_span,
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
        },
    )
    .map_err(ExpressionParseError::from)?;

    let call = HandledFallibleCall {
        name: inference.instance_path,
        args: arguments,
        result_type_ids: inference.signature.success_return_type_ids(),
        call_span,
    };

    let expression = finish_generic_function_call(GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        declaration_path: template.function_path,
        error_return_type_id: inference.signature.error_return_type_id(),
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
        path_fork,
    })?;

    context.record_generic_function_instantiation_request(GenericFunctionInstantiationRequest {
        declaration_identity: template.declaration_identity.clone(),
        evidence: selected_evidence,
        key: inference.key,
        instance_path: inference.instance_path,
        call_span,
    });

    Ok(expression)
}

fn validate_ready_template_call(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
    raw_arguments: Vec<CallArgument>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallParseInput {
        token_stream,
        template,
        context,
        expected_context,
        value_required,
        allow_boundary_catch,
        call_span,
        warnings,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: &raw_arguments,
        expected_context,
        call_span,
        known_receiver_type_id: None,
        known_receiver_span: None,
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
        path_fork,
    })?;
    validate_generic_function_bound_evidence(
        template,
        inference.key.type_arguments.as_ref(),
        context,
        type_interner.environment(),
        path_fork,
        call_span,
    )?;

    let callee_name = path_fork
        .component(template.function_path)
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations =
        expectations_from_user_parameters(&inference.signature.parameters, path_fork);
    let arguments = resolve_call_arguments_shape_and_access(
        CallDiagnosticContext::function(&callee_name),
        &raw_arguments,
        &expectations,
        call_span,
        string_table,
        type_interner.environment(),
        path_fork,
    )
    .map_err(ExpressionParseError::from)?;

    let call = HandledFallibleCall {
        name: template.function_path,
        args: arguments,
        result_type_ids: inference.signature.success_return_type_ids(),
        call_span,
    };

    finish_generic_function_call(GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        declaration_path: template.function_path,
        error_return_type_id: inference.signature.error_return_type_id(),
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
        path_fork,
    })
}

pub(crate) fn parse_generic_function_call_expression(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    parse_generic_call_in_lane(input, GenericCallLane::Instance)
}

pub(crate) fn validate_generic_function_template_call_expression(
    input: GenericFunctionCallParseInput<'_, '_, '_>,
) -> Result<ParsedFunctionCallExpression, ExpressionParseError> {
    parse_generic_call_in_lane(input, GenericCallLane::TemplateValidation)
}

/// Completes a generic call retained while its numeric arguments were pending.
///
/// WHAT: completes the argument list through the body session, then infers, validates, and resolves
///       the call the same way the ready path does for its parse lane. The authored `!` decided at
///       parse time is applied, and the reserved instantiation request publishes last.
/// WHY: the call's routing was fixed at its source position, so completion supplies only concrete
///      argument values; the reserved ticket keeps the request in parse order.
pub(crate) fn complete_deferred_generic_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredGenericCall,
    session: &mut S,
) -> Result<Expression, ExpressionParseError> {
    let ticket = deferred.ticket;
    let completion = complete_reserved_generic_call(deferred, &mut *session);
    match (completion, ticket) {
        (Err(error), Some(ticket)) => {
            // A failed completion must not leave its reserved slot Pending: commit would then
            // report an abandoned request as a missing publication.
            let DeferredExpressionServices { context, .. } = session.session_services();
            context.abandon_pending_generic_request(ticket)?;
            Err(error)
        }
        (completion, _) => completion,
    }
}

/// Chooses one retained generic-call argument's destination from its linked numeric class.
///
/// WHAT: an argument still inferred takes its class's receiver witness when one exists, the same
///       destination a forward producer of that class takes. With no witness it stays inferred.
/// WHY: an exact witness already on the argument's class, such as an immediate result, must type
///      the literal. Completing the argument while it is inferred defaults the literal first.
pub(crate) fn choose_retained_argument_destination<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    argument: DeferredCallArgument,
    session: &mut S,
) -> Result<DeferredCallArgument, ExpressionParseError> {
    match argument {
        DeferredCallArgument::Retained {
            shape,
            mut expression,
        } => {
            if let Some(origin) = shape.origin_shape
                && matches!(expression.expected, ExpectedType::Infer)
            {
                let DeferredExpressionServices {
                    context,
                    type_interner,
                    ..
                } = session.session_services();
                let mut state = context.numeric_state_mut();
                let class = match state.shape(origin) {
                    KnownShape::Numeric(class) => Some(class),
                    _ => None,
                };
                let receiver = match class {
                    Some(class) => merged_receiver_type(&mut state, class)?,
                    None => None,
                };
                if let Some(type_id) = receiver
                    .and_then(|type_id| numeric_witness_type(type_id, type_interner.environment()))
                {
                    expression.expected = ExpectedType::NumericLiteral(type_id);
                }
            }
            Ok(DeferredCallArgument::Retained { shape, expression })
        }
        other => Ok(other),
    }
}

/// Runs the completion steps that may fail after a request ticket is reserved.
///
/// WHAT: everything up to and including request publication; the caller abandons the ticket on error.
/// WHY: publication is the only step that consumes the reserved slot, so a failure before it must
///      release the slot instead of leaving it Pending.
fn complete_reserved_generic_call<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    deferred: DeferredGenericCall,
    session: &mut S,
) -> Result<Expression, ExpressionParseError> {
    let DeferredGenericCall {
        template,
        arguments,
        call_span,
        authored_handling,
        expected_result_type_ids,
        error_return,
        ticket,
        ..
    } = deferred;

    let mut raw_arguments = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let argument = choose_retained_argument_destination(argument, &mut *session)?;
        raw_arguments.push(session.complete_call_argument(argument)?);
    }

    let DeferredExpressionServices {
        context,
        type_interner,
        string_table,
        path_fork,
    } = session.session_services();
    let expected_context = match expected_result_type_ids.as_deref() {
        Some(type_ids) => GenericCallExpectedContext::ImmediateResult(type_ids),
        None => GenericCallExpectedContext::None,
    };
    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template: &template,
        raw_arguments: &raw_arguments,
        expected_context,
        call_span,
        known_receiver_type_id: None,
        known_receiver_span: None,
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
        path_fork,
    })?;
    verify_completed_error_return(
        error_return,
        inference.signature.error_return_type_id(),
        context,
        type_interner,
    )?;
    if let Some(GenericErrorReturn::Retained(_)) = error_return {
        verify_retained_propagation_error(
            &authored_handling,
            inference.signature.error_return_type_id(),
            context,
            type_interner.environment(),
        )?;
    }
    let selected_evidence = validate_generic_function_bound_evidence(
        &template,
        inference.key.type_arguments.as_ref(),
        context,
        type_interner.environment(),
        path_fork,
        call_span,
    )?;

    let callee_name = path_fork
        .component(template.function_path)
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations =
        expectations_from_user_parameters(&inference.signature.parameters, path_fork);
    // A reserved ticket marks the instance lane; template-validation calls reserve none.
    let (call_name, arguments) = match ticket {
        Some(_) => {
            if context.is_generic_function_instantiation_active(&inference.key) {
                return Err(recursive_generic_function_instantiation(
                    path_fork.component(template.function_path),
                    call_span,
                )
                .into());
            }
            let type_check_context = type_interner.type_check_context();
            let resolved = resolve_call_arguments(
                CallDiagnosticContext::function(&callee_name),
                &raw_arguments,
                &expectations,
                call_span,
                CallArgumentResolutionContext {
                    string_table,
                    type_environment: type_check_context.type_environment,
                    compatibility_cache: type_check_context.compatibility_cache,
                    path_fork,
                },
            )
            .map_err(ExpressionParseError::from)?;
            (inference.instance_path, resolved)
        }
        None => {
            let resolved = resolve_call_arguments_shape_and_access(
                CallDiagnosticContext::function(&callee_name),
                &raw_arguments,
                &expectations,
                call_span,
                string_table,
                type_interner.environment(),
                path_fork,
            )
            .map_err(ExpressionParseError::from)?;
            (template.function_path, resolved)
        }
    };

    let call = HandledFallibleCall {
        name: call_name,
        args: arguments,
        result_type_ids: inference.signature.success_return_type_ids(),
        call_span,
    };
    let expression = finish_retained_generic_call(
        call,
        inference.signature.error_return_type_id(),
        authored_handling,
        template.function_path,
        context,
        type_interner,
    );

    if let Some(ticket) = ticket {
        context.publish_pending_generic_request(
            ticket,
            GenericFunctionInstantiationRequest {
                declaration_identity: template.declaration_identity.clone(),
                evidence: selected_evidence,
                key: inference.key,
                instance_path: inference.instance_path,
                call_span,
            },
        )?;
    }
    Ok(expression)
}

/// Checks that a completed generic call agrees with the error role linked when it was retained.
///
/// WHAT: `None` must complete to `None`; a `Concrete` link must complete to the same type; a
///       `Retained` link must materialise through its numeric origins to the completed error type.
/// WHY: the parse-time `!` decision was made from the linked error role, so a divergence means the
///      parse and completion lanes disagree about the callee's error slot and must not publish.
pub(crate) fn verify_completed_error_return(
    linked: Option<GenericErrorReturn>,
    completed: Option<TypeId>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
) -> Result<(), ExpressionParseError> {
    let agrees = match (linked, completed) {
        (None, None) => true,
        (Some(GenericErrorReturn::Concrete(linked_type)), Some(completed_type)) => {
            linked_type == completed_type
        }
        (Some(GenericErrorReturn::Retained(shape)), Some(completed_type)) => {
            let mut state = context.numeric_state_mut();
            let materialised = materialise_known_shape(
                &mut state,
                type_interner.environment_mut_for_derived_types(),
                shape,
            )?;
            materialised == completed_type
        }
        (_, _) => false,
    };
    if agrees {
        return Ok(());
    }
    Err(CompilerError::compiler_error(
        "generic call error return changed between retained parse and completion",
    )
    .into())
}

/// Checks an authored `!` on a retained generic call against the enclosing function's error slot.
///
/// WHAT: a retained error has no resolved type at parse time, so the propagated error's
///       compatibility is decided here, where the producer completes and its error type is known.
/// WHY: the parse-time `!` check compares only a concrete declared error, so a retained error that
///      skipped this check would propagate an error the enclosing function cannot return.
pub(crate) fn verify_retained_propagation_error(
    authored_handling: &DeferredCallHandling,
    completed_error_type_id: Option<TypeId>,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
) -> Result<(), ExpressionParseError> {
    let DeferredCallHandling::Propagate { span } = authored_handling else {
        return Ok(());
    };
    let Some(actual_error_type_id) = completed_error_type_id else {
        return Err(CompilerError::compiler_error(
            "a retained generic call completed without the error slot it was linked to",
        )
        .into());
    };
    let Some(expected_error_type_id) = context.expected_error_type else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
            *span,
        )
        .into());
    };
    if !is_postfix_error_compatible(
        expected_error_type_id,
        actual_error_type_id,
        type_environment,
    ) {
        return Err(CompilerDiagnostic::type_mismatch(
            expected_error_type_id,
            actual_error_type_id,
            TypeMismatchContext::ErrorReturn,
            *span,
        )
        .into());
    }
    Ok(())
}

/// Applies the authored `!` settled at parse time to a completed generic call.
///
/// WHAT: `Propagate` routes the call through the typed propagation producer; `Unhandled` keeps the
///       call's own error slot and private failure candidate exactly as the ready path does.
/// WHY: the parse-time decision already consumed the `!` token, so completion must not re-read it.
fn finish_retained_generic_call(
    call: HandledFallibleCall,
    error_return_type_id: Option<TypeId>,
    authored_handling: DeferredCallHandling,
    declaration_path: PathId,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
) -> Expression {
    let call_span = call.call_span;
    match authored_handling {
        DeferredCallHandling::Propagate { span } => call.into_expression(
            FallibleHandling::Propagate,
            span.or(call_span),
            type_interner.environment_mut_for_derived_types(),
        ),
        DeferredCallHandling::Unhandled => match error_return_type_id {
            None => {
                let call_path = call.name;
                let expression =
                    call.into_plain_expression(type_interner.environment_mut_for_derived_types());
                if context.source_call_has_private_failure_lane(declaration_path) {
                    expression.with_private_call_failure_candidate(call_path)
                } else {
                    expression
                }
            }
            Some(error_return_type_id) => {
                Expression::handled_fallible_function_call_with_typed_arguments(
                    call.name,
                    call.args,
                    call.result_type_ids,
                    FallibleExpressionHandling::Recover,
                    type_interner.environment_mut_for_derived_types(),
                    call.call_span,
                )
                .with_typed_error_producer(error_return_type_id, None)
            }
        },
    }
}

fn finish_generic_function_call(
    input: GenericFunctionCallFinishInput<'_, '_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        declaration_path,
        error_return_type_id,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let Some(error_return_type_id) = error_return_type_id else {
        if token_stream.current_tag() == TokenTag::BANG {
            let operand_is_optional = call_success_is_optional(
                call.result_type_ids.as_slice(),
                type_interner.environment(),
            );
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                non_fallible_handler_reason(token_stream.current_tag(), operand_is_optional),
                Some(token_stream.current_span()),
            )
            .into());
        }

        let call_path = call.name;
        let expression =
            call.into_plain_expression(type_interner.environment_mut_for_derived_types());
        return Ok(
            if context.source_call_has_private_failure_lane(declaration_path) {
                expression.with_private_call_failure_candidate(call_path)
            } else {
                expression
            },
        );
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
            string_table,
            path_fork,
        );
    }

    Ok(
        Expression::handled_fallible_function_call_with_typed_arguments(
            call.name,
            call.args,
            call.result_type_ids,
            FallibleExpressionHandling::Recover,
            type_interner.environment_mut_for_derived_types(),
            call.call_span,
        )
        .with_typed_error_producer(error_return_type_id, None),
    )
}

pub(crate) struct GenericFunctionInferenceInput<'a> {
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) raw_arguments: &'a [CallArgument],
    /// The hidden receiver's concrete type, derived from its actual argument for ready calls or
    /// supplied directly by a retained chain that has not produced the receiver expression yet.
    pub(crate) known_receiver_type_id: Option<TypeId>,
    /// Source span for the hidden receiver type fact, when present.
    pub(crate) known_receiver_span: Option<SourceSpan>,
    pub(crate) expected_context: GenericCallExpectedContext<'a>,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) type_environment: &'a mut TypeEnvironment,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

pub(crate) struct GenericFunctionInference {
    pub(crate) key: GenericFunctionInstanceKey,
    pub(crate) instance_path: PathId,
    pub(crate) signature: FunctionSignature,
}

struct GenericBindingEvidenceLocations {
    spans_by_parameter: FxHashMap<GenericParameterId, GenericEvidenceLocation>,
}

#[derive(Clone)]
struct GenericEvidenceLocation {
    span: Option<SourceSpan>,
}

struct GenericBindingEvidenceContext<'a> {
    template: &'a GenericFunctionTemplate,
    bindings: &'a mut GenericTypeBindings,
    evidence_locations: &'a mut GenericBindingEvidenceLocations,
    type_environment: &'a TypeEnvironment,
    string_table: &'a mut StringTable,
    path_fork: &'a PathInternerFork,
}

impl GenericBindingEvidenceLocations {
    fn new() -> Self {
        Self {
            spans_by_parameter: FxHashMap::default(),
        }
    }

    fn previous(&self, parameter_id: GenericParameterId) -> Option<GenericEvidenceLocation> {
        self.spans_by_parameter.get(&parameter_id).cloned()
    }

    fn record_first_bindings(
        &mut self,
        template: &GenericFunctionTemplate,
        bindings: &GenericTypeBindings,
        type_environment: &TypeEnvironment,
        span: Option<SourceSpan>,
    ) {
        let Some(parameter_list) =
            type_environment.generic_parameters(template.generic_parameter_list_id)
        else {
            return;
        };

        for parameter in &parameter_list.parameters {
            if bindings.get(parameter.id).is_some() {
                self.spans_by_parameter
                    .entry(parameter.id)
                    .or_insert(GenericEvidenceLocation { span });
            }
        }
    }
}

pub(crate) fn infer_generic_function_call(
    input: GenericFunctionInferenceInput<'_>,
) -> Result<GenericFunctionInference, ExpressionParseError> {
    let GenericFunctionInferenceInput {
        template,
        raw_arguments,
        known_receiver_type_id,
        known_receiver_span,
        expected_context,
        call_span,
        type_environment,
        string_table,
        path_fork,
    } = input;

    let expectations = expectations_from_user_parameters(&template.signature.parameters, path_fork);
    let receiver_slot_offset = if known_receiver_type_id.is_some() {
        1
    } else {
        0
    };
    let routed_arguments = order_call_argument_refs_by_retained_slot(
        raw_arguments,
        expectations.len(),
        receiver_slot_offset,
    )?;
    let mut bindings = GenericTypeBindings::new();
    let mut evidence_locations = GenericBindingEvidenceLocations::new();
    collect_call_argument_bindings(
        template,
        &routed_arguments,
        &expectations,
        known_receiver_type_id,
        known_receiver_span,
        &mut bindings,
        &mut evidence_locations,
        type_environment,
        string_table,
        path_fork,
    )?;

    let mut type_arguments =
        bindings.concrete_arguments_for(template.generic_parameter_list_id, type_environment);
    if type_arguments.is_none()
        && let Some(expected_result_type_ids) = expected_context
            .matching_success_results(template.signature.success_return_type_ids().len())
    {
        collect_expected_result_bindings(
            template,
            expected_result_type_ids,
            &mut bindings,
            &mut evidence_locations,
            type_environment,
            string_table,
            path_fork,
            call_span,
        )?;
        type_arguments =
            bindings.concrete_arguments_for(template.generic_parameter_list_id, type_environment);
    }

    let Some(type_arguments) = type_arguments else {
        let missing_parameters =
            missing_generic_parameter_names(template, &bindings, type_environment);
        return Err(cannot_infer_generic_function_arguments(
            path_fork.component(template.function_path),
            missing_parameters,
            call_span,
        )
        .into());
    };

    let mapping = type_environment
        .build_parameter_mapping(template.generic_parameter_list_id, &type_arguments)
        .ok_or_else(|| {
            cannot_infer_generic_function_arguments(
                path_fork.component(template.function_path),
                missing_generic_parameter_names(template, &bindings, type_environment),
                call_span,
            )
        })?;
    let signature = substitute_function_signature(&template.signature, &mapping, type_environment);
    let instance_path = generic_function_instance_path(
        template.function_path,
        type_arguments.as_ref(),
        path_fork,
        string_table,
    )?;

    Ok(GenericFunctionInference {
        key: GenericFunctionInstanceKey {
            function_path: template.function_path,
            type_arguments,
        },
        instance_path,
        signature,
    })
}

pub(crate) fn validate_generic_function_bound_evidence(
    template: &GenericFunctionTemplate,
    type_arguments: &[TypeId],
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
    call_span: Option<SourceSpan>,
) -> Result<Box<[BoundEvidenceSelection]>, ExpressionParseError> {
    let Some(parameter_list) =
        type_environment.generic_parameters(template.generic_parameter_list_id)
    else {
        return Ok(Box::new([]));
    };

    let trait_environment = context.trait_environment();
    let evidence_environment = context.trait_evidence_environment();

    let mut selected_evidence = Vec::new();
    for (parameter, concrete_type_id) in parameter_list.parameters.iter().zip(type_arguments) {
        for trait_id in &parameter.trait_bounds {
            let trait_is_visible = context.trait_id_is_visible(*trait_id);
            if context.generic_template_validation
                && trait_is_visible
                && generic_parameter_declares_bound(*concrete_type_id, *trait_id, type_environment)
            {
                // A template can forward its own bound into a nested generic request. The
                // concrete instance is reparsed later, where the requester selects the actual
                // evidence identity, so template validation must not manufacture a local row.
                continue;
            }
            // An exact generated pair the requester already transferred authorizes both halves of
            // its own visibility, because a materialised body may call a provider-owned trait's
            // evidence from a module that never named that trait. Every pair the requester did
            // not select still needs ordinary trait and target visibility, and either way the
            // real evidence row selected below remains mandatory.
            let evidence_is_visible = generated_evidence_pair_is_selected(
                *concrete_type_id,
                *trait_id,
                type_environment,
                context.shared.generated_evidence_pairs.as_ref(),
            ) || (trait_is_visible
                && evidence_target_is_visible(
                    *concrete_type_id,
                    type_environment,
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_source_names),
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_type_alias_names),
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_namespace_records),
                    context.shared.resolved_type_aliases.as_deref(),
                ));
            let evidence_selection = evidence_is_visible
                .then(|| {
                    evidence_for_type(
                        *concrete_type_id,
                        *trait_id,
                        type_environment,
                        trait_environment,
                        evidence_environment,
                    )
                })
                .flatten();

            if let Some(evidence_selection) = evidence_selection {
                selected_evidence.push(evidence_selection);
                continue;
            }

            let trait_name = trait_environment
                .get(*trait_id)
                .map(|definition| definition.name)
                .unwrap_or(
                    path_fork
                        .component(template.function_path)
                        .unwrap_or(parameter.name),
                );

            return Err(missing_generic_function_trait_evidence(
                path_fork.component(template.function_path),
                parameter.name,
                trait_name,
                *concrete_type_id,
                call_span,
            )
            .into());
        }
    }

    Ok(selected_evidence.into_boxed_slice())
}

impl<'a> GenericCallExpectedContext<'a> {
    fn matching_success_results(self, success_return_count: usize) -> Option<&'a [TypeId]> {
        match self {
            GenericCallExpectedContext::ImmediateResult(expected_result_type_ids)
                if expected_result_type_ids.len() == success_return_count =>
            {
                Some(expected_result_type_ids)
            }

            GenericCallExpectedContext::ImmediateResult(_) | GenericCallExpectedContext::None => {
                None
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "binding collection keeps borrowed routed arguments, the hidden receiver fact, expectations, and mutable evidence/type/string/path state as separate borrows"
)]
fn collect_call_argument_bindings(
    template: &GenericFunctionTemplate,
    routed_arguments: &[Option<&CallArgument>],
    expectations: &[ParameterExpectation],
    known_receiver_type_id: Option<TypeId>,
    known_receiver_span: Option<SourceSpan>,
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let mut evidence_context = GenericBindingEvidenceContext {
        template,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
        path_fork,
    };

    for (slot, argument) in routed_arguments.iter().enumerate() {
        let (argument_type_id, argument_span) = match (slot, known_receiver_type_id, argument) {
            (0, Some(receiver_type_id), _) => (receiver_type_id, known_receiver_span),
            (_, _, Some(argument)) => (argument.value.type_id, argument.span),
            _ => continue,
        };
        let ExpectedParameterType::Known(template_type_id) = expectations[slot].expected_type
        else {
            continue;
        };

        collect_binding_evidence(
            &mut evidence_context,
            template_type_id,
            argument_type_id,
            argument_span,
        )?;
    }

    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "expected-result binding collection keeps the template, expected types, and mutable binding/evidence/type/string/path state plus the call span as separate inputs"
)]
fn collect_expected_result_bindings(
    template: &GenericFunctionTemplate,
    expected_result_type_ids: &[TypeId],
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    let mut evidence_context = GenericBindingEvidenceContext {
        template,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
        path_fork,
    };

    for (template_return_type, expected_type) in template
        .signature
        .success_return_type_ids()
        .iter()
        .zip(expected_result_type_ids.iter())
    {
        collect_binding_evidence(
            &mut evidence_context,
            *template_return_type,
            *expected_type,
            span,
        )?;
    }

    Ok(())
}

fn collect_binding_evidence(
    context: &mut GenericBindingEvidenceContext<'_>,
    template_type_id: TypeId,
    concrete_type_id: TypeId,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    match context
        .type_environment
        .try_collect_type_parameter_bindings_typeid(
            template_type_id,
            concrete_type_id,
            &mut *context.bindings,
        ) {
        Ok(true) => {
            context.evidence_locations.record_first_bindings(
                context.template,
                &*context.bindings,
                context.type_environment,
                span,
            );
            Ok(())
        }
        Ok(false) => {
            // Structural non-match: no new binding evidence, and the staged walk
            // left the caller's binding map unchanged. This stays a non-binding
            // mismatch, not a repeated-parameter conflict.
            Ok(())
        }
        Err(conflict) => Err(binding_conflict_diagnostic(
            context.template,
            conflict,
            &*context.evidence_locations,
            context.type_environment,
            &mut *context.string_table,
            context.path_fork,
            span,
        )
        .into()),
    }
}
fn binding_conflict_diagnostic(
    template: &GenericFunctionTemplate,
    conflict: BindingConflict,
    evidence_locations: &GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
    span: Option<SourceSpan>,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let parameter_name = type_environment
        .generic_parameters(template.generic_parameter_list_id)
        .and_then(|list| {
            list.parameters
                .iter()
                .find(|parameter| parameter.id == conflict.parameter_id)
                .map(|parameter| parameter.name)
        })
        .unwrap_or_else(|| string_table.intern("<generic parameter>"));

    let previous = evidence_locations.previous(conflict.parameter_id);
    conflicting_generic_function_argument(
        path_fork.component(template.function_path),
        conflict,
        parameter_name,
        span,
        previous.as_ref().and_then(|evidence| evidence.span),
    )
}

pub(crate) fn substitute_function_signature(
    signature: &FunctionSignature,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> FunctionSignature {
    // Shared by generic call inference, concrete emission and generated requests.
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(
        FrontendCounter::CensusGenericCallAndMaterialisationSignatureSubstitutions,
    );
    FunctionSignature {
        parameters: signature
            .parameters
            .iter()
            .map(|parameter| substitute_declaration(parameter, mapping, type_environment))
            .collect(),
        returns: signature
            .returns
            .iter()
            .map(|slot| substitute_return_slot(slot, mapping, type_environment))
            .collect(),
    }
}

fn substitute_declaration(
    declaration: &Declaration,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> Declaration {
    let mut declaration = declaration.clone();
    let type_id = type_environment.substitute_type_id(declaration.value.type_id, mapping);
    declaration.value.type_id = type_id;
    declaration.value.diagnostic_type = diagnostic_type_spelling(type_id, type_environment);
    declaration
}

fn substitute_return_slot(
    slot: &ReturnSlot,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> ReturnSlot {
    let Some(type_id) = slot
        .type_id
        .map(|type_id| type_environment.substitute_type_id(type_id, mapping))
    else {
        return slot.clone();
    };

    let data_type = diagnostic_type_spelling(type_id, type_environment);
    let value = data_type;

    ReturnSlot {
        value,
        type_id: Some(type_id),
        channel: slot.channel,
    }
}

fn missing_generic_parameter_names(
    template: &GenericFunctionTemplate,
    bindings: &GenericTypeBindings,
    type_environment: &TypeEnvironment,
) -> Vec<StringId> {
    type_environment
        .generic_parameters(template.generic_parameter_list_id)
        .map(|list| {
            list.parameters
                .iter()
                .filter(|parameter| bindings.get(parameter.id).is_none())
                .map(|parameter| parameter.name)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn generic_function_instance_path(
    function_path: PathId,
    type_arguments: &[TypeId],
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<PathId, CompilerError> {
    // WHAT: builds an internal-only path for a concrete generic function instance.
    // WHY: instances are emitted into the consuming module's AST/HIR and are not
    //      user-visible, dependency-bindable, or namespace-exposed.
    let argument_suffix = type_arguments
        .iter()
        .map(|type_id| type_id.0.to_string())
        .collect::<Vec<_>>()
        .join("_");
    let component = string_table.intern(&format!("__generic_instance_{argument_suffix}"));
    path_fork
        .try_intern_child(function_path, component)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "path table exhausted while creating generic function instance",
            )
        })
}
