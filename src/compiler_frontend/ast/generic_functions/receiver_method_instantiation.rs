//! Generic receiver-method instantiation for hidden receiver calls.
//!
//! WHAT: infers one concrete receiver-method instance from the actual receiver-first arguments,
//!       validates the instance's declared trait bounds and recursion state, and produces the
//!       generated instance path, its substituted signature and its materialisation request.
//! WHY: receiver-call syntax and user-defined cast evidence both reach a generic evidence method
//!      through a hidden receiver call. Both must select the same generated instance before HIR,
//!      so the inference, bound and recursion rules live in one owner instead of a second copy.
//!      A hidden call without authored arguments borrows its one receiver argument instead of
//!      building an owned argument list, so cast evidence adds no argument allocation.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericFunctionInferenceInput, GenericFunctionInstantiationRequest,
    GenericFunctionTemplate, infer_generic_function_call, recursive_generic_function_instantiation,
    validate_generic_function_bound_evidence,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Inputs for instantiating one generic receiver method from its actual hidden-call arguments.
///
/// WHAT: groups the template with the hidden call's receiver argument, its authored arguments and
///       the body-emission collaborators needed to infer, validate and request the instance.
/// WHY: receiver-call syntax and cast evidence supply the same hidden call shape to the same
///      owner, so that shape belongs here rather than to either call site.
pub(crate) struct GenericReceiverMethodInstantiationInput<'a, 'interner> {
    pub(crate) template: &'a GenericFunctionTemplate,
    /// The hidden call's receiver argument, already in declaration-order slot zero.
    pub(crate) receiver_argument: CallArgument,
    /// Authored arguments retaining their parse-time slots, before the receiver shift. Empty for
    /// hidden call shapes such as cast evidence, where the receiver stays the only borrowed
    /// inference argument.
    pub(crate) authored_arguments: &'a [CallArgument],
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// One concrete receiver-method instance ready for AST emission.
pub(crate) struct GenericReceiverMethodInstantiation {
    pub(crate) instance_path: PathId,
    pub(crate) signature: FunctionSignature,
    pub(crate) request: GenericFunctionInstantiationRequest,
    /// The receiver argument handed in, returned so the call reuses one receiver expression
    /// instead of rebuilding an identical receiver AST for inference and for the call itself.
    pub(crate) receiver_argument: CallArgument,
}

/// Infers and validates one concrete receiver-method instance.
///
/// WHAT: solves the template's type arguments from the receiver-first arguments, validates the
///       instance's declared bounds and recursion state, and returns the generated instance path,
///       the substituted signature, the materialisation request and the receiver argument.
/// WHY: the generated instance is the only executable callable HIR may see for a generic
///      receiver method, so selection must happen in AST through the shared request lane.
pub(crate) fn instantiate_generic_receiver_method(
    input: GenericReceiverMethodInstantiationInput<'_, '_>,
) -> Result<GenericReceiverMethodInstantiation, ExpressionParseError> {
    let GenericReceiverMethodInstantiationInput {
        template,
        receiver_argument,
        authored_arguments,
        call_span,
        scope_context,
        type_interner,
        string_table,
        path_fork,
    } = input;

    // Cast evidence and receiver calls without authored arguments already carry the receiver in
    // declaration-order slot zero, so that one argument stays a borrowed stack value and no
    // argument list is allocated. Only a genuinely authored argument list pays for shifted copies,
    // because the shifted slots must sit next to the receiver in one contiguous argument slice.
    if authored_arguments.is_empty() {
        let (instance_path, signature, request) =
            infer_receiver_method_instance(ReceiverMethodInferenceInput {
                template,
                inference_arguments: std::slice::from_ref(&receiver_argument),
                call_span,
                scope_context,
                type_interner,
                string_table,
                path_fork,
            })?;

        return Ok(GenericReceiverMethodInstantiation {
            instance_path,
            signature,
            request,
            receiver_argument,
        });
    }

    let mut inference_arguments = Vec::with_capacity(authored_arguments.len() + 1);
    inference_arguments.push(receiver_argument);
    for argument in authored_arguments {
        let Some(parameter_slot) = argument.parameter_slot else {
            return Err(CompilerError::compiler_error(
                "Receiver call argument is missing its retained parameter slot",
            )
            .into());
        };
        inference_arguments.push(
            argument
                .clone()
                .with_parameter_slot(ParameterSlot::new(parameter_slot.index() + 1)),
        );
    }

    let (instance_path, signature, request) =
        infer_receiver_method_instance(ReceiverMethodInferenceInput {
            template,
            inference_arguments: &inference_arguments,
            call_span,
            scope_context,
            type_interner,
            string_table,
            path_fork,
        })?;

    // Inference populated slot zero with the receiver handed in above; the argument order of the
    // remaining slots is never read again, so the receiver leaves without shifting the list.
    let receiver_argument = inference_arguments.swap_remove(0);

    Ok(GenericReceiverMethodInstantiation {
        instance_path,
        signature,
        request,
        receiver_argument,
    })
}

/// Borrowed receiver-first arguments for one instance inference.
///
/// WHAT: carries the argument slice inference reads without taking ownership of any argument.
/// WHY: the allocation-free receiver path borrows its single stack argument, so the shared
///      inference core must read the receiver-first arguments through a slice.
struct ReceiverMethodInferenceInput<'a, 'interner> {
    template: &'a GenericFunctionTemplate,
    inference_arguments: &'a [CallArgument],
    call_span: Option<SourceSpan>,
    scope_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'interner>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Infers, bound-validates and requests one receiver-method instance from borrowed arguments.
///
/// WHAT: the single owner of receiver-method type-argument inference, bound validation, recursion
///       rejection and materialisation-request construction.
/// WHY: every receiver-method path must apply exactly the same rules, so the rules exist once and
///      read their arguments through a borrow instead of through an owned list.
fn infer_receiver_method_instance(
    input: ReceiverMethodInferenceInput<'_, '_>,
) -> Result<
    (
        PathId,
        FunctionSignature,
        GenericFunctionInstantiationRequest,
    ),
    ExpressionParseError,
> {
    let ReceiverMethodInferenceInput {
        template,
        inference_arguments,
        call_span,
        scope_context,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: inference_arguments,
        expected_context: GenericCallExpectedContext::None,
        call_span,
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
        path_fork,
    })?;
    let selected_evidence = validate_generic_function_bound_evidence(
        template,
        inference.key.type_arguments.as_ref(),
        scope_context,
        type_interner.environment(),
        path_fork,
        call_span,
    )?;

    if scope_context.is_generic_function_instantiation_active(&inference.key) {
        return Err(recursive_generic_function_instantiation(
            path_fork.component(template.function_path),
            call_span,
        )
        .into());
    }

    Ok((
        inference.instance_path,
        inference.signature,
        GenericFunctionInstantiationRequest {
            declaration_identity: template.declaration_identity.clone(),
            evidence: selected_evidence,
            key: inference.key,
            instance_path: inference.instance_path,
            call_span,
        },
    ))
}

/// Instantiates the generic receiver method named by one user-defined cast evidence method.
///
/// WHAT: treats the cast operand as the evidence method's receiver, infers the concrete instance
///       through the shared receiver-method owner, records its materialisation request, and
///       returns the generated instance path with the operand expression back for the cast node.
/// WHY: a cast's evidence method is a hidden receiver call whose operand must keep its single
///      authored evaluation and whose template origin must never reach HIR. The operand becomes
///      one stack-owned inference argument that is borrowed for inference and moves straight back
///      into the cast node, so no argument list or receiver AST copy is allocated.
pub(crate) fn instantiate_generic_cast_evidence_method(
    template: &GenericFunctionTemplate,
    source: Expression,
    call_span: Option<SourceSpan>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(PathId, Expression), ExpressionParseError> {
    let receiver_argument = CallArgument::positional(source, CallAccessMode::Shared, call_span)
        .with_parameter_slot(ParameterSlot::new(0));
    let instantiation =
        instantiate_generic_receiver_method(GenericReceiverMethodInstantiationInput {
            template,
            receiver_argument,
            authored_arguments: &[],
            call_span,
            scope_context,
            type_interner,
            string_table,
            path_fork,
        })?;

    // Template validation bodies are re-parsed per concrete instance, where the request is
    // recorded against the real requester evidence; the validated template keeps only the
    // resolved instance path, exactly like receiver-call syntax in the same context.
    if !scope_context.generic_template_validation {
        scope_context.record_generic_function_instantiation_request(instantiation.request);
    }

    Ok((
        instantiation.instance_path,
        instantiation.receiver_argument.value,
    ))
}
