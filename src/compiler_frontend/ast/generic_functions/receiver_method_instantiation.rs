//! Generic receiver-method instantiation from hidden receiver facts.
//!
//! WHAT: infers one concrete receiver-method instance, validates its declared trait bounds and
//!       recursion state, and produces the generated instance path, substituted signature and
//!       materialisation request.
//! WHY: ready calls and retained postfix chains share one instantiation owner. Ready calls provide
//!      their actual receiver argument; retained chains provide a known concrete receiver TypeId
//!      without fabricating an expression or place.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericFunctionInferenceInput, GenericFunctionInstantiationRequest,
    GenericFunctionTemplate, infer_generic_function_call, recursive_generic_function_instantiation,
    validate_generic_function_bound_evidence,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::ValueReceiverContext;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Receiver facts accepted by generic receiver-method inference.
///
/// WHAT: either the actual hidden receiver argument, or its already-known concrete type when a
///       retained chain has not produced the receiver expression yet.
/// WHY: receiver-method inference needs the same concrete receiver type in both paths, but only
///      ordinary ready calls have an expression to move into the final call.
// `Argument` is a one-shot receiver built at the call site, consumed by value by
// `instantiate_generic_receiver_method` and returned as `receiver_argument`. It is never stored in
// a collection or held across a body root, so it stays inline. Boxing it would add a heap
// allocation to every ready generic receiver call.
pub(crate) enum GenericReceiverMethodReceiver {
    Argument(CallArgument<OwnedExpr>),
    KnownType(TypeId),
}

/// Inputs for instantiating one generic receiver method.
///
/// WHAT: groups the template with the hidden-call receiver fact, authored arguments and the
///       body-emission collaborators needed to infer, validate and request the instance.
/// WHY: receiver-call syntax and cast evidence supply actual arguments, while retained postfix
///      chains supply their known concrete receiver type; both use this one instantiation owner.
pub(crate) struct GenericReceiverMethodInstantiationInput<'a, 'interner> {
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) receiver: GenericReceiverMethodReceiver,
    /// Authored arguments retaining their parser-assigned slots; inference accounts for hidden
    /// receiver slot zero without cloning or shifting these arguments.
    /// Empty for hidden call shapes such as cast evidence, where the receiver is the only fact.
    pub(crate) authored_arguments: &'a [CallArgument<OwnedExpr>],
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(crate) expression_builder: &'a AstExpressionBuilder,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// One concrete receiver-method instance ready for AST emission.
pub(crate) struct GenericReceiverMethodInstantiation {
    pub(crate) instance_path: PathId,
    pub(crate) signature: FunctionSignature,
    pub(crate) request: GenericFunctionInstantiationRequest,
    /// Present only when inference received the actual hidden receiver argument.
    pub(crate) receiver_argument: Option<CallArgument<OwnedExpr>>,
}

/// Infers and validates one generic receiver-method instance.
///
/// WHAT: supplies the receiver type as a type fact and borrows authored arguments with their
///       parser-assigned slots, then returns the generated path, substituted signature, request,
///       and (when present) the original receiver argument.
/// WHY: ready calls and retained chains share one inference owner without shifting or cloning
///      authored expression trees.
pub(crate) fn instantiate_generic_receiver_method(
    input: GenericReceiverMethodInstantiationInput<'_, '_>,
) -> Result<GenericReceiverMethodInstantiation, ExpressionParseError> {
    let GenericReceiverMethodInstantiationInput {
        template,
        receiver,
        authored_arguments,
        call_span,
        scope_context,
        type_interner,
        expression_builder,
        string_table,
        path_fork,
    } = input;

    let (receiver_argument, receiver_type_id, receiver_span) = match receiver {
        GenericReceiverMethodReceiver::Argument(receiver_argument) => {
            let receiver_type_id = expression_builder
                .view_owned(receiver_argument.value)?
                .require_single(ValueReceiverContext::ReceiverBase)?;
            let receiver_span = receiver_argument.span;
            (Some(receiver_argument), receiver_type_id, receiver_span)
        }
        GenericReceiverMethodReceiver::KnownType(receiver_type_id) => {
            (None, receiver_type_id, call_span)
        }
    };

    let (instance_path, signature, request) =
        infer_receiver_method_instance(ReceiverMethodInferenceInput {
            template,
            authored_arguments,
            receiver_type_id,
            receiver_span,
            call_span,
            scope_context,
            type_interner,
            expression_builder,
            string_table,
            path_fork,
        })?;

    Ok(GenericReceiverMethodInstantiation {
        instance_path,
        signature,
        request,
        receiver_argument,
    })
}

/// Inference facts for one receiver-method instance.
///
/// WHAT: borrows authored arguments with parser-owned parameter slots and carries the hidden
///       receiver type/span separately from its completed expression.
/// WHY: ready and retained call sites need identical receiver-first generic inference without
///      allocating shifted argument vectors or cloning argument payloads.
struct ReceiverMethodInferenceInput<'a, 'interner> {
    template: &'a GenericFunctionTemplate,
    authored_arguments: &'a [CallArgument<OwnedExpr>],
    receiver_type_id: TypeId,
    receiver_span: Option<SourceSpan>,
    call_span: Option<SourceSpan>,
    scope_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'interner>,
    expression_builder: &'a AstExpressionBuilder,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Infers, bound-validates and requests one receiver-method instance from borrowed type/routing
/// facts.
///
/// WHAT: the single owner of receiver-method type-argument inference, bound validation, recursion
///       rejection and materialisation-request construction.
/// WHY: ready and retained receiver-method paths apply the same rules through one shared inference
///      lane without constructing shifted CallArgument copies.
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
        authored_arguments,
        receiver_type_id,
        receiver_span,
        call_span,
        scope_context,
        type_interner,
        expression_builder,
        string_table,
        path_fork,
    } = input;

    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: authored_arguments,
        expression_builder,
        known_receiver_type_id: Some(receiver_type_id),
        known_receiver_span: receiver_span,
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
///       returns the generated instance path and the unchanged operand ID for the cast node.
/// WHY: a cast's evidence method is a hidden receiver call whose operand must keep its single
///      authored evaluation and whose template origin must never reach HIR. Inference borrows the
///      operand through its issuing builder and retains no expression view or detached graph.
#[allow(
    clippy::too_many_arguments,
    reason = "cast evidence keeps the template, source, span, scope and mutable state as separate inputs"
)]
pub(crate) fn instantiate_generic_cast_evidence_method(
    template: &GenericFunctionTemplate,
    source: OwnedExpr,
    call_span: Option<SourceSpan>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &AstExpressionBuilder,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(PathId, OwnedExpr), ExpressionParseError> {
    let receiver_argument = CallArgument::positional(source, CallAccessMode::Shared, call_span)
        .with_parameter_slot(ParameterSlot::new(0));
    let instantiation =
        instantiate_generic_receiver_method(GenericReceiverMethodInstantiationInput {
            template,
            receiver: GenericReceiverMethodReceiver::Argument(receiver_argument),
            authored_arguments: &[],
            call_span,
            scope_context,
            type_interner,
            expression_builder,
            string_table,
            path_fork,
        })?;
    let receiver_argument = instantiation.receiver_argument.ok_or_else(|| {
        CompilerError::compiler_error(
            "generic receiver instantiation omitted its actual receiver argument",
        )
    })?;

    // Template validation bodies are re-parsed per concrete instance, where the request is
    // recorded against the real requester evidence; the validated template keeps only the
    // resolved instance path, exactly like receiver-call syntax in the same context.
    if !scope_context.generic_template_validation {
        scope_context.record_generic_function_instantiation_request(instantiation.request);
    }

    Ok((instantiation.instance_path, receiver_argument.value))
}
