//! Declared/source receiver method dispatch.
//!
//! WHAT: looks up user-declared receiver methods (including generic instantiation)
//!       and parses the resulting call AST node, retaining the call together with its concrete
//!       receiver and authored handling when a numeric-dependent argument needs body-root
//!       completion.
//! WHY: source methods have distinct lookup rules from generic-bound and static
//!      trait-surface dispatch; keeping them in one file makes the generic-instantiation
//!      path explicit and local, and a retained call resumes through the same constructor the
//!      completed path uses instead of a second argument parse or a fabricated receiver.

use super::ParsedReceiverMethodCall;
use super::ReceiverAccessMode;
use super::shared::{TraitSurfaceReceiverMethod, receiver_result_type_ids_for_call};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_expectations,
    parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ParameterExpectation,
    expectations_from_receiver_method_signature, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::function_calls::{
    CallFinishContext, finish_function_call_expression,
};
use crate::compiler_frontend::ast::field_access::parse_chain::expression_from_postfix_node;
use crate::compiler_frontend::ast::field_access::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::{
    GenericErrorReturn, GenericNumericSignature,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, GenericFunctionTemplate,
    GenericReceiverMethodInstantiationInput, GenericReceiverMethodReceiver, GenericRequestTicket,
    choose_retained_argument_destination, instantiate_generic_receiver_method,
    verify_completed_error_return, verify_retained_propagation_error,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredCallHandling, DeferredExpressionCompletion,
    DeferredExpressionServices,
};
use crate::compiler_frontend::ast::numeric_known_shape::KnownShapeId;
use crate::compiler_frontend::ast::receiver_methods::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    FUNCTION_CALL_STAGE, FallibleSuccessContract, HandledFallibleCall, ParsedFallibleSuffix,
    parse_fragment_handling_suffix,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverCallReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

pub(in crate::compiler_frontend::ast::field_access) fn lookup_receiver_method<'a>(
    context: &'a ScopeContext,
    receiver_type_id: TypeId,
    member_name: StringId,
    type_environment: &TypeEnvironment,
) -> Option<&'a ReceiverMethodEntry> {
    let receiver_key = type_environment.receiver_key_for_type_id(receiver_type_id)?;
    context.lookup_receiver_method(&receiver_key, member_name)
}

pub(super) enum SourceReceiverMethodTarget<'a> {
    Declared(&'a ReceiverMethodEntry),
    TraitSurface(TraitSurfaceReceiverMethod),
}

impl SourceReceiverMethodTarget<'_> {
    pub(super) fn receiver_mutable(&self) -> bool {
        match self {
            SourceReceiverMethodTarget::Declared(entry) => entry.receiver_mutable,
            SourceReceiverMethodTarget::TraitSurface(method) => method.receiver_mutable,
        }
    }

    fn signature(&self) -> &FunctionSignature {
        match self {
            SourceReceiverMethodTarget::Declared(entry) => &entry.signature,
            SourceReceiverMethodTarget::TraitSurface(method) => &method.signature,
        }
    }

    fn method_path(&self) -> PathId {
        match self {
            SourceReceiverMethodTarget::Declared(entry) => entry.function_path,
            SourceReceiverMethodTarget::TraitSurface(method) => method.method_path,
        }
    }
}

/// Builds the hidden call's receiver argument for one receiver method call.
///
/// WHAT: materialises the receiver expression once in declaration-order slot zero, carrying the
///       receiver's declared mutability as its access mode.
/// WHY: generic instantiation and the final call share this single receiver expression; rebuilding
///      an identical receiver AST for either step would only add an avoidable source copy.
fn receiver_hidden_call_argument(
    receiver_node: &AstNode,
    receiver_mutable: bool,
    member_span: Option<SourceSpan>,
    expression_builder: &AstExpressionBuilder,
) -> Result<CallArgument<OwnedExpr>, ExpressionParseError> {
    let receiver_access = if receiver_mutable {
        CallAccessMode::Mutable
    } else {
        CallAccessMode::Shared
    };

    Ok(CallArgument::positional(
        expression_builder.own(expression_from_postfix_node(receiver_node)?),
        receiver_access,
        member_span,
    )
    .with_parameter_slot(ParameterSlot::new(0)))
}

pub(super) struct SourceReceiverMethodCallInput<'a, 'interner, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) receiver_node: &'a AstNode,
    pub(super) member_name: StringId,
    pub(super) member_span: Option<SourceSpan>,
    pub(super) receiver_access_mode: ReceiverAccessMode,
    pub(super) authored_marker_span: Option<SourceSpan>,
    /// Whether an authored boundary `catch` may attach at the chain entry that reached this call.
    pub(super) allow_boundary_catch: bool,
    pub(super) scope_context: &'a ScopeContext,
    pub(super) source_method: SourceReceiverMethodTarget<'a>,
    pub(super) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parses one already-resolved receiver method call.
///
/// WHAT: returns the built call node, or the pending operand item when a numeric-dependent
///       argument was retained together with the call's identity, receiver value and handling.
/// WHY: the enclosing operand position owns where a retained call resumes from, while a call whose
///      arguments are already concrete stays the ordinary completed node.
pub(super) fn parse_source_receiver_method_target_call_typed(
    input: SourceReceiverMethodCallInput<'_, '_, '_>,
) -> Result<ParsedReceiverMethodCall, ExpressionParseError> {
    let SourceReceiverMethodCallInput {
        token_stream,
        receiver_node,
        member_name,
        member_span,
        receiver_access_mode,
        authored_marker_span,
        allow_boundary_catch,
        scope_context,
        source_method,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;

    if receiver_node.expression_is_const_record_value(&*expression_builder)? {
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::ConstRecordNoRuntimeCalls,
            None,
            Some(member_name),
            None,
            None,
            member_span,
        )
        .into());
    }

    if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::MustUseParentheses,
            None,
            Some(member_name),
            None,
            None,
            member_span,
        )
        .into());
    }

    let member_span = member_span.or_else(|| Some(token_stream.current_span()));
    token_stream.advance();

    validate_receiver_access(
        receiver_node,
        &*expression_builder,
        path_fork,
        receiver_access_mode,
        member_span,
        authored_marker_span,
        ReceiverAccessRequirement {
            requires_mutable: source_method.receiver_mutable(),
            diagnostic: ReceiverAccessDiagnostic::ReceiverMethod {
                method_name: member_name,
            },
        },
    )?;

    let initial_expectations = expectations_from_receiver_method_signature(
        &source_method.signature().parameters[1..],
        path_fork,
    );
    let argument_syntax = CallArgumentSyntax::Supported {
        callee_name: Some(member_name),
    };
    // A generic receiver method needs its argument types to instantiate, so a numeric-dependent
    // argument there keeps propagating to its own owner; only an already-resolved (non-template)
    // method has the identity and signature a body-root resumption needs.
    let arguments_are_resumable = scope_context
        .lookup_generic_function_template(&source_method.method_path())
        .is_none();
    let argument_ownership = if arguments_are_resumable {
        parse_call_arguments_typed_with_numeric_retention(
            token_stream,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            &initial_expectations,
            argument_syntax,
            path_fork,
        )?
    } else {
        ParsedCallArguments::Ready(parse_call_arguments_typed_with_expectations(
            token_stream,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            &initial_expectations,
            argument_syntax,
            path_fork,
        )?)
    };
    let raw_args = match argument_ownership {
        ParsedCallArguments::Ready(arguments) => arguments,
        ParsedCallArguments::Retained(arguments) => {
            return retain_numeric_receiver_call(RetainedReceiverCallInput {
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                receiver_node,
                member_name,
                member_span,
                authored_marker_span,
                source_method: &source_method,
                expectations: initial_expectations,
                arguments,
                allow_boundary_catch,
            });
        }
    };

    let (method_path, call_signature, generic_request, receiver_expression) = match &source_method {
        SourceReceiverMethodTarget::Declared(method_entry) => {
            let receiver_argument = receiver_hidden_call_argument(
                receiver_node,
                method_entry.receiver_mutable,
                member_span,
                expression_builder,
            )?;
            match scope_context.lookup_generic_function_template(&method_entry.function_path) {
                Some(template) => {
                    let instantiation = instantiate_generic_receiver_method(
                        GenericReceiverMethodInstantiationInput {
                            template,
                            receiver: GenericReceiverMethodReceiver::Argument(receiver_argument),
                            authored_arguments: &raw_args,
                            call_span: member_span,
                            scope_context,
                            type_interner: &mut *type_interner,
                            expression_builder: &*expression_builder,
                            string_table: &mut *string_table,
                            path_fork: &mut *path_fork,
                        },
                    )?;
                    let receiver_argument = instantiation.receiver_argument.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "generic receiver instantiation omitted its actual receiver argument",
                        )
                    })?;
                    (
                        instantiation.instance_path,
                        instantiation.signature,
                        Some(instantiation.request),
                        receiver_argument.value,
                    )
                }
                None => (
                    method_entry.function_path.to_owned(),
                    method_entry.signature.to_owned(),
                    None,
                    receiver_argument.value,
                ),
            }
        }

        SourceReceiverMethodTarget::TraitSurface(method) => {
            let receiver_argument = receiver_hidden_call_argument(
                receiver_node,
                method.receiver_mutable,
                member_span,
                expression_builder,
            )?;
            match scope_context.lookup_generic_function_template(&method.method_path) {
                Some(template) => {
                    let instantiation = instantiate_generic_receiver_method(
                        GenericReceiverMethodInstantiationInput {
                            template,
                            receiver: GenericReceiverMethodReceiver::Argument(receiver_argument),
                            authored_arguments: &raw_args,
                            call_span: member_span,
                            scope_context,
                            type_interner: &mut *type_interner,
                            expression_builder: &*expression_builder,
                            string_table: &mut *string_table,
                            path_fork: &mut *path_fork,
                        },
                    )?;
                    let receiver_argument = instantiation.receiver_argument.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "generic receiver instantiation omitted its actual receiver argument",
                        )
                    })?;
                    (
                        instantiation.instance_path,
                        instantiation.signature,
                        Some(instantiation.request),
                        receiver_argument.value,
                    )
                }
                None => (
                    method.method_path,
                    method.signature.clone(),
                    None,
                    receiver_argument.value,
                ),
            }
        }
    };

    let expectations = if arguments_are_resumable {
        initial_expectations
    } else {
        expectations_from_receiver_method_signature(&call_signature.parameters[1..], path_fork)
    };
    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::receiver_method(CallDiagnosticName::Interned(member_name)),
        raw_args,
        &expectations,
        member_span,
        CallArgumentResolutionContext {
            string_table,
            expression_builder,
            const_values,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
        },
    )?;

    let method_call_expression = if let Some(error_return_type_id) =
        call_signature.error_return_type_id()
    {
        // Typed methods use the established handled-call shape, with their receiver in slot zero.
        // A raw MethodCall inside a carrier would bypass HIR's explicit error-channel call lowering.
        let receiver_span = expression_builder.view_owned(receiver_expression)?.span();
        let receiver_access = if source_method.receiver_mutable() {
            CallAccessMode::Mutable
        } else {
            CallAccessMode::Shared
        };
        let receiver_argument =
            CallArgument::positional(receiver_expression, receiver_access, receiver_span)
                .with_marker_span(authored_marker_span)
                .with_parameter_slot(ParameterSlot::new(0));
        let mut full_arguments = Vec::with_capacity(args.len() + 1);
        full_arguments.push(receiver_argument);
        for argument in args {
            let Some(parameter_slot) = argument.parameter_slot else {
                return Err(CompilerError::compiler_error(
                    "Receiver call argument is missing its retained parameter slot",
                )
                .into());
            };
            full_arguments
                .push(argument.with_parameter_slot(ParameterSlot::new(parameter_slot.index() + 1)));
        }
        let result_type_ids = call_signature.success_return_type_ids();
        let value_required = !result_type_ids.is_empty();
        finish_function_call_expression(
            HandledFallibleCall {
                name: method_path,
                args: full_arguments,
                result_type_ids,
                call_span: member_span,
            },
            Some(error_return_type_id),
            CallFinishContext {
                token_stream,
                context: scope_context,
                value_required,
                allow_boundary_catch: false,
                warnings: None,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            },
        )?
    } else {
        let result_type_ids = receiver_result_type_ids_for_call(
            call_signature.success_return_type_ids(),
            token_stream,
            type_interner,
        )?;
        let method_call_expression = expression_builder.method_call_with_typed_arguments(
            receiver_expression,
            method_path,
            args,
            result_type_ids,
            member_span,
        )?;
        let declared_requirement = matches!(
            &source_method,
            SourceReceiverMethodTarget::TraitSurface(method)
                if matches!(method.origin, super::shared::TraitSurfaceMethodOrigin::DeclaredRequirement)
        );
        let has_private_lane = !declared_requirement
            && scope_context.source_call_has_private_failure_lane(source_method.method_path());
        let failure_path = if scope_context.generic_template_validation {
            source_method.method_path()
        } else {
            method_path
        };
        if has_private_lane {
            expression_builder
                .with_private_call_failure_candidate(method_call_expression, failure_path)?
        } else {
            method_call_expression
        }
    };

    if !scope_context.generic_template_validation
        && let Some(request) = generic_request
    {
        scope_context.record_generic_function_instantiation_request(request);
    }
    Ok(ParsedReceiverMethodCall::Value(AstNode {
        kind: NodeKind::ExpressionStatement(method_call_expression.id()),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    }))
}

/// Everything the retention site must photograph before the token stream moves past the call.
///
/// WHAT: the authored receiver, resolved method target, intake expectations, retained arguments
///       and boundary catch permission, all read at the place that still owns them.
/// WHY: the retained record and the shared suffix decision share one site, so grouping them keeps
///      the receiver value, argument routing and handling decision in a single resumable record
///      instead of threading each resource separately.
struct RetainedReceiverCallInput<'a, 'interner, 'tokens> {
    token_stream: &'a mut AstCursor<'tokens>,
    scope_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'interner>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
    receiver_node: &'a AstNode,
    member_name: StringId,
    member_span: Option<SourceSpan>,
    authored_marker_span: Option<SourceSpan>,
    source_method: &'a SourceReceiverMethodTarget<'a>,
    expectations: Vec<ParameterExpectation>,
    arguments: Vec<DeferredCallArgument>,
    allow_boundary_catch: bool,
}

/// The authored receiver value of one retained receiver method call.
///
/// WHAT: distinguishes a receiver materialised at the authored call site from one produced by the
///       preceding retained postfix step of the same chain.
/// WHY: a chained call's receiver value does not exist where the call is retained, so the step
///      completion supplies it; the record never fabricates a receiver expression.
#[derive(Debug, Clone)]
pub(crate) enum DeferredCallReceiver {
    /// The receiver value was materialised once at the authored call site.
    Ready(ExprId),
    /// The receiver is the completed value of the preceding retained postfix step.
    Chained,
}

/// The generic-method facts of one declared-owner receiver call.
///
/// WHAT: names the template, the linked result and error shapes, and the ledger ticket reserved
///       for the instance request.
/// WHY: the concrete instance depends on the completed receiver type, so the authored site keeps
///      the template and linked numeric facts while completion resolves the instance.
#[derive(Debug, Clone)]
pub(crate) struct DeferredGenericReceiverMethod {
    pub(crate) template: GenericFunctionTemplate,
    pub(crate) result_shapes: Vec<KnownShapeId>,
    pub(crate) error_return: Option<GenericErrorReturn>,
    pub(crate) ticket: Option<GenericRequestTicket>,
}

/// The callee of one retained receiver method call.
///
/// WHAT: a concrete method keeps its resolved signature. A declared-owner generic method keeps
///       its template and linked numeric facts; the template owns the declared signature until
///       the instance completes.
/// WHY: one owner for the callee facts, so a generic record cannot hold a signature copy that
///      disagrees with its template.
// `Generic` keeps its payload inline: the template is larger than a concrete signature, and
// `DeferredReceiverCall` is already boxed by both `DeferredCall::Receiver` and
// `RetainedPostfixStep::Method`, so boxing the payload again would add a second heap allocation
// to every generic receiver call.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum DeferredReceiverCallee {
    /// A concrete method whose signature resolved at the authored site.
    Concrete(FunctionSignature),
    /// A declared-owner generic method whose instance completes once its receiver type is known.
    Generic(DeferredGenericReceiverMethod),
}

/// A concrete generic-method instance selected from a retained receiver type.
///
/// WHAT: pairs the instance request with the ledger ticket reserved for it at the authored site.
/// WHY: a retained receiver completes after its enclosing static-if ranges may have closed.
///      Publishing into the reserved slot keeps the request inside its authored range, so an
///      inactive branch tombstones it instead of appending an active request at completion.
#[derive(Debug, Clone)]
pub(crate) struct ReservedGenericRequest {
    pub(crate) ticket: GenericRequestTicket,
    pub(crate) request: GenericFunctionInstantiationRequest,
}

/// The resolved identity and authored call-site facts of one retained receiver method call.
///
/// WHAT: carries the already-resolved method identity and signature, the receiver value source,
///       the intake expectation view when its signature is unchanged, ordered arguments and call
///       handling plus the resolved failure-lane fact.
/// WHY: a numeric-dependent argument cannot be validated before its origin's single static type is
///      selected. Keeping the resolved call facts means the body root resumes the exact authored
///      receiver call through the ordinary constructor: no member lookup, no argument re-parse and
///      no generic instance search.
#[derive(Debug, Clone)]
pub(crate) struct DeferredReceiverCall {
    pub(crate) method_path: PathId,
    /// Visible member name of the call, kept for the resolved argument diagnostics.
    pub(crate) method_name: StringId,
    /// The callee. A concrete method keeps its resolved signature; a declared-owner generic
    /// method keeps its template, which owns the declared signature until instantiation.
    pub(crate) callee: DeferredReceiverCallee,
    pub(crate) receiver: DeferredCallReceiver,
    pub(crate) receiver_mutable: bool,
    pub(crate) authored_marker_span: Option<SourceSpan>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    /// Reuses intake expectations when the retained call already has a concrete signature.
    ///
    /// Generic receiver instantiation substitutes that signature, so its final expectations are
    /// built only after substitution and this field stays `None`.
    pub(crate) unchanged_expectations: Option<Vec<ParameterExpectation>>,
    pub(crate) call_span: Option<SourceSpan>,
    /// A concrete generic method instance selected from the retained receiver type, paired with
    /// the ledger slot reserved at its authored position. `None` for every other call, including
    /// template-validation lanes, which publish nothing.
    pub(crate) generic_request: Option<ReservedGenericRequest>,
    /// The private-failure-lane candidate of a non-fallible method, resolved at the call site
    /// exactly as the completed plain-call path resolves it.
    pub(crate) private_failure_candidate: Option<PathId>,
    pub(crate) authored_handling: DeferredCallHandling,
}

impl DeferredReceiverCall {
    /// Builds a declared-owner generic method record whose instance completes later.
    ///
    /// WHAT: keeps the template, the linked numeric shapes and the reserved ledger ticket. The
    ///       declared signature is the template's and is read until the instance completes.
    /// WHY: the concrete instance depends on the completed receiver type, so the authored site
    ///      cannot name it yet.
    #[allow(
        clippy::too_many_arguments,
        reason = "one declared-owner call's resolved facts are passed as separate arguments"
    )]
    pub(crate) fn chained_generic_method(
        template: GenericFunctionTemplate,
        linked: GenericNumericSignature,
        ticket: Option<GenericRequestTicket>,
        method_name: StringId,
        receiver_mutable: bool,
        authored_marker_span: Option<SourceSpan>,
        arguments: Vec<DeferredCallArgument>,
        call_span: Option<SourceSpan>,
        private_failure_candidate: Option<PathId>,
    ) -> Self {
        Self {
            method_path: template.function_path,
            method_name,
            callee: DeferredReceiverCallee::Generic(DeferredGenericReceiverMethod {
                template,
                result_shapes: linked.result_shapes,
                error_return: linked.error_return,
                ticket,
            }),
            receiver: DeferredCallReceiver::Chained,
            receiver_mutable,
            authored_marker_span,
            arguments,
            unchanged_expectations: None,
            call_span,
            generic_request: None,
            private_failure_candidate,
            authored_handling: DeferredCallHandling::Unhandled,
        }
    }

    /// The single result shape of a generic-method record with exactly one success slot.
    pub(crate) fn single_success_result_shape(&self) -> Option<KnownShapeId> {
        match self.generic_result_shapes()? {
            [shape] => Some(*shape),
            _ => None,
        }
    }

    /// The concrete callee signature, or `None` for a generic-method record.
    ///
    /// WHAT: a concrete record's resolved signature. A generic record's instance signature exists
    ///       only once its receiver type completes.
    /// WHY: exact success types must not come from a template whose generic parameters are unbound.
    pub(crate) fn concrete_signature(&self) -> Option<&FunctionSignature> {
        match &self.callee {
            DeferredReceiverCallee::Concrete(signature) => Some(signature),
            DeferredReceiverCallee::Generic(_) => None,
        }
    }

    /// The declared callee signature, read without completing the call.
    ///
    /// WHAT: a concrete record's resolved signature, or a generic record's template signature,
    ///       whose generic parameters stay unbound until completion.
    /// WHY: slot counts and the optional-operand decision need only the declared shape, and the
    ///      template states it exactly before the instance completes.
    pub(crate) fn declared_signature(&self) -> &FunctionSignature {
        match &self.callee {
            DeferredReceiverCallee::Concrete(signature) => signature,
            DeferredReceiverCallee::Generic(generic) => &generic.template.signature,
        }
    }

    /// The exact single success type of a concrete record. A generic record has no exact type
    /// until its instance completes, so it reports none here.
    pub(crate) fn single_success_return_type_id(&self) -> Option<TypeId> {
        self.concrete_signature()?.single_success_return_type_id()
    }

    /// The ordered result shapes of a generic-method record; `None` for a concrete record.
    pub(crate) fn generic_result_shapes(&self) -> Option<&[KnownShapeId]> {
        match &self.callee {
            DeferredReceiverCallee::Generic(generic) => Some(generic.result_shapes.as_slice()),
            DeferredReceiverCallee::Concrete(_) => None,
        }
    }

    /// The fallible error role: a generic record's linked role, otherwise the concrete signature's.
    pub(crate) fn error_return(&self) -> Option<GenericErrorReturn> {
        match &self.callee {
            DeferredReceiverCallee::Generic(generic) => generic.error_return,
            DeferredReceiverCallee::Concrete(signature) => signature
                .error_return_type_id()
                .map(GenericErrorReturn::Concrete),
        }
    }

    /// The ledger ticket reserved at this record's authored site, if any.
    ///
    /// WHAT: a declared-owner record's ticket and a concrete record's reserved request share one
    ///       slot in the ordered ledger.
    /// WHY: completion must publish into or abandon the same slot on both paths.
    fn reserved_ledger_ticket(&self) -> Option<GenericRequestTicket> {
        if let DeferredReceiverCallee::Generic(generic) = &self.callee {
            return generic.ticket;
        }
        self.generic_request
            .as_ref()
            .map(|reserved| reserved.ticket)
    }
}

/// Retains one receiver method call whose numeric-dependent arguments need body completion.
///
/// WHAT: consumes the authored operand postfix `!` (when present) through the shared
///       fallible-handling fragment entry and snapshots the resolved identity, signature, receiver
///       value and ordered argument list into the construction-only call record.
/// WHY: the call site is the only place the authored operand handling and the concrete signature
///      exist. A retained call can no longer read tokens, so recording the `!` decision here keeps
///      propagation exactly as authored. A `catch` is never this site's to consume: it protects the
///      whole selected expression (for example `10 + obj.f(pending) catch:`), so the token stays for
///      the enclosing fragment owner, which retains the complete fragment together with its
///      handler.
fn retain_numeric_receiver_call(
    input: RetainedReceiverCallInput<'_, '_, '_>,
) -> Result<ParsedReceiverMethodCall, ExpressionParseError> {
    let RetainedReceiverCallInput {
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        receiver_node,
        member_name,
        member_span,
        authored_marker_span,
        source_method,
        expectations,
        arguments,
        allow_boundary_catch,
    } = input;

    let signature = source_method.signature();
    let success_result_type_ids = signature.success_return_type_ids();
    let value_required = !success_result_type_ids.is_empty();

    // The resolved private-failure-lane fact of this call site: it is the method's own authored
    // failure eligibility, the same fact the completed plain-call path and the shared suffix entry
    // consume, and it is the exact candidate the completed plain-call path attaches.
    let declared_requirement = matches!(
        source_method,
        SourceReceiverMethodTarget::TraitSurface(method)
            if matches!(method.origin, super::shared::TraitSurfaceMethodOrigin::DeclaredRequirement)
    );
    let private_failure_lane = !declared_requirement
        && scope_context.source_call_has_private_failure_lane(source_method.method_path());

    // The shared fragment entry owns the whole authored operand-suffix decision for a retained
    // call: the `!` legality checks and the non-fallible diagnostics come from the same site facts
    // the completed call path passes. The entry is only consulted when the operand's own `!` is
    // authored: a `catch` belongs to the enclosing expression owner, so a retained call never
    // consumes it and no second parser or protected-call payload exists here.
    let authored_handling =
        match super::super::retained_postfix::authored_propagation_suffix(token_stream) {
            None => DeferredCallHandling::Unhandled,
            Some(propagation_span) => {
                let authored_suffix = parse_fragment_handling_suffix(
                    token_stream,
                    scope_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    FallibleSuccessContract::Known(success_result_type_ids.as_slice()),
                    signature
                        .error_return_type_id()
                        .map(GenericErrorReturn::Concrete),
                    private_failure_lane,
                    value_required,
                    member_span,
                    FUNCTION_CALL_STAGE,
                    allow_boundary_catch,
                    None,
                    string_table,
                    path_fork,
                )?;
                let Some(ParsedFallibleSuffix::Ready(FallibleHandling::Propagate)) =
                    authored_suffix
                else {
                    return Err(CompilerError::compiler_error(
                        "an authored operand `!` was not parsed as typed propagation",
                    )
                    .into());
                };

                DeferredCallHandling::Propagate {
                    span: propagation_span,
                }
            }
        };

    // The receiver value is materialised once, exactly as the completed path materialises it, so
    // the resumed call passes the one concrete receiver the authored call names.
    let receiver_expression = receiver_hidden_call_argument(
        receiver_node,
        source_method.receiver_mutable(),
        member_span,
        expression_builder,
    )?
    .value
    .id();

    // A fallible method owns its error channel: only the plain call carries the private lane.
    let private_failure_candidate = (private_failure_lane
        && signature.error_return_type_id().is_none())
    .then(|| source_method.method_path());
    let method_path = source_method.method_path();
    let receiver_mutable = source_method.receiver_mutable();
    let signature = match source_method {
        SourceReceiverMethodTarget::Declared(method) => method.signature.clone(),
        SourceReceiverMethodTarget::TraitSurface(method) => method.signature.clone(),
    };

    Ok(retained_receiver_call_operand(
        retained_receiver_call_record(
            method_path,
            member_name,
            signature,
            Some(expectations),
            receiver_expression,
            receiver_mutable,
            authored_marker_span,
            arguments,
            member_span,
            private_failure_candidate,
            authored_handling,
        ),
        member_span,
    ))
}

/// Builds the construction-only record of one retained receiver method call.
#[allow(
    clippy::too_many_arguments,
    reason = "the record names one call site's resolved facts as separate fields instead of hiding them in a navigation step"
)]
fn retained_receiver_call_record(
    method_path: PathId,
    method_name: StringId,
    signature: FunctionSignature,
    unchanged_expectations: Option<Vec<ParameterExpectation>>,
    receiver_expression: ExprId,
    receiver_mutable: bool,
    authored_marker_span: Option<SourceSpan>,
    arguments: Vec<DeferredCallArgument>,
    call_span: Option<SourceSpan>,
    private_failure_candidate: Option<PathId>,
    authored_handling: DeferredCallHandling,
) -> DeferredCall {
    DeferredCall::Receiver(Box::new(DeferredReceiverCall {
        method_path,
        method_name,
        callee: DeferredReceiverCallee::Concrete(signature),
        receiver: DeferredCallReceiver::Ready(receiver_expression),
        receiver_mutable,
        authored_marker_span,
        unchanged_expectations,
        arguments,
        call_span,
        private_failure_candidate,
        authored_handling,
        generic_request: None,
    }))
}

/// Places one retained receiver call in its authored operand position.
fn retained_receiver_call_operand(
    call: DeferredCall,
    span: Option<SourceSpan>,
) -> ParsedReceiverMethodCall {
    ParsedReceiverMethodCall::RetainedOperand(ExpressionRpnItem::PendingNumericCall {
        call: Box::new(call),
        span,
    })
}

/// Resumes one retained receiver method call once the body root resolved its arguments.
///
/// WHAT: takes the call's site-materialised receiver and rebuilds the exact call through the
///       shared value-supplied completion.
/// WHY: the body root only supplies the concrete argument values and the authored operand `!`
///      decision already recorded; the call's identity, evidence, access modes and argument
///      routing were resolved at the authored site and are never searched or parsed again.
pub(crate) fn complete_deferred_receiver_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredReceiverCall,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    // A generic method's ticket was reserved at the authored site, whether declared-owner or
    // concrete. Any failure before publication must release that slot so the ordered ledger never
    // keeps a pending request.
    let ticket = deferred.reserved_ledger_ticket();
    match complete_reserved_receiver_call(deferred, session) {
        Ok(expression) => Ok(expression),
        Err(error) => {
            if let Some(ticket) = ticket {
                session
                    .session_services()
                    .context
                    .abandon_pending_generic_request(ticket)?;
            }
            Err(error)
        }
    }
}

/// Completes one reserved receiver call once its receiver value is materialised.
///
/// WHAT: completes arguments in authored order. A generic method instantiates from the completed
///       receiver type, validates its linked error role and publishes its reserved request last.
///       A concrete call resolves its authored callee unchanged.
/// WHY: the receiver's type is only known at completion, so a generic method's instance cannot be
///      named at the authored site.
fn complete_reserved_receiver_call<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    deferred: DeferredReceiverCall,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredReceiverCall {
        method_path,
        method_name,
        callee,
        receiver,
        receiver_mutable,
        authored_marker_span,
        arguments,
        unchanged_expectations,
        call_span,
        generic_request,
        private_failure_candidate,
        authored_handling,
    } = deferred;
    // Every retained receiver call completes with one materialised receiver value: a site call
    // carries it from its authored site, and a chained step injects the preceding step's completed
    // value before resuming this same constructor.
    let DeferredCallReceiver::Ready(receiver_expression) = receiver else {
        return Err(CompilerError::compiler_error(
            "a chained retained receiver call completed without its preceding step value",
        )
        .into());
    };

    // Arguments complete before the session services are borrowed: both take the session mutably.
    // A generic method's retained arguments choose their template-owned destinations first, as a
    // generic free call does.
    let is_generic = matches!(callee, DeferredReceiverCallee::Generic(_));
    let mut completed_arguments = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let argument = if is_generic {
            choose_retained_argument_destination(argument, session)?
        } else {
            argument
        };
        completed_arguments.push(session.complete_call_argument(argument)?);
    }

    let DeferredExpressionServices {
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = session.session_services();
    let receiver_expression = expression_builder.own(receiver_expression);

    let (method_path, signature, publication, private_failure_candidate) = match callee {
        DeferredReceiverCallee::Generic(generic) => {
            let DeferredGenericReceiverMethod {
                template,
                result_shapes: _,
                error_return,
                ticket,
            } = generic;
            let receiver_type_id = expression_builder
                .view_owned(receiver_expression)?
                .require_single(ValueReceiverContext::ReceiverBase)?;
            let instantiation =
                instantiate_generic_receiver_method(GenericReceiverMethodInstantiationInput {
                    template: &template,
                    receiver: GenericReceiverMethodReceiver::KnownType(receiver_type_id),
                    authored_arguments: &completed_arguments,
                    call_span,
                    scope_context: context,
                    type_interner: &mut *type_interner,
                    expression_builder: &*expression_builder,
                    string_table: &mut *string_table,
                    path_fork: &mut *path_fork,
                })?;
            let completed_error = instantiation.signature.error_return_type_id();
            verify_completed_error_return(error_return, completed_error, context, type_interner)?;
            if matches!(error_return, Some(GenericErrorReturn::Retained(_))) {
                verify_retained_propagation_error(
                    &authored_handling,
                    completed_error,
                    context,
                    type_interner.environment(),
                )?;
            }
            let instance_path = instantiation.instance_path;
            // A validation lane keeps the template path exactly as the concrete path does.
            let private_failure_candidate = private_failure_candidate.map(|template_path| {
                if context.generic_template_validation {
                    template_path
                } else {
                    instance_path
                }
            });
            (
                instance_path,
                instantiation.signature,
                ticket.map(|ticket| ReservedGenericRequest {
                    ticket,
                    request: instantiation.request,
                }),
                private_failure_candidate,
            )
        }
        DeferredReceiverCallee::Concrete(signature) => (
            method_path,
            signature,
            generic_request,
            private_failure_candidate,
        ),
    };

    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::receiver_method(CallDiagnosticName::Interned(method_name)),
        completed_arguments,
        &unchanged_expectations.unwrap_or_else(|| {
            expectations_from_receiver_method_signature(&signature.parameters[1..], path_fork)
        }),
        call_span,
        CallArgumentResolutionContext {
            string_table,
            expression_builder,
            const_values,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
        },
    )?;

    let expression = receiver_call_expression(
        receiver_expression,
        method_path,
        &signature,
        args,
        receiver_mutable,
        authored_marker_span,
        call_span,
        private_failure_candidate,
        authored_handling,
        expression_builder,
    )?;
    // The reserved slot publishes only after every fallible completion step has succeeded.
    if let Some(ReservedGenericRequest { ticket, request }) = publication {
        context.publish_pending_generic_request(ticket, request)?;
    }
    Ok(expression)
}

/// Builds one completed receiver method call from its resolved callee and completed arguments.
///
/// WHAT: a plain call keeps the receiver as the method's receiver. A `!` call places the receiver
///       in implicit slot zero, shifts each authored argument one slot and replays the authored
///       handling exactly as the completed typed-method path does.
/// WHY: concrete and generic receiver calls share this tail, so both produce the same expression.
#[allow(
    clippy::too_many_arguments,
    reason = "each resolved call fact is a separate argument so the constructor stays flat"
)]
fn receiver_call_expression(
    receiver_expression: OwnedExpr,
    method_path: PathId,
    signature: &FunctionSignature,
    args: Vec<CallArgument<OwnedExpr>>,
    receiver_mutable: bool,
    authored_marker_span: Option<SourceSpan>,
    call_span: Option<SourceSpan>,
    private_failure_candidate: Option<PathId>,
    authored_handling: DeferredCallHandling,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    let result_type_ids = signature.success_return_type_ids();
    let Some(error_return_type_id) = signature.error_return_type_id() else {
        // Every receiver call here was resolved at its site without a `!`, so the success types are
        // the declared ones and the plain method call is the completed shape.
        let expression = expression_builder.method_call_with_typed_arguments(
            receiver_expression,
            method_path,
            args,
            result_type_ids,
            call_span,
        )?;
        return Ok(match private_failure_candidate {
            Some(candidate) => {
                expression_builder.with_private_call_failure_candidate(expression, candidate)?
            }
            None => expression,
        });
    };

    // The handled carrier keeps the receiver in implicit slot zero and shifts every authored
    // argument by one declaration-order slot, exactly as the completed typed-method path does.
    let receiver_span = expression_builder.view_owned(receiver_expression)?.span();
    let receiver_access = if receiver_mutable {
        CallAccessMode::Mutable
    } else {
        CallAccessMode::Shared
    };
    let receiver_argument =
        CallArgument::positional(receiver_expression, receiver_access, receiver_span)
            .with_marker_span(authored_marker_span)
            .with_parameter_slot(ParameterSlot::new(0));
    let mut full_arguments = Vec::with_capacity(args.len() + 1);
    full_arguments.push(receiver_argument);
    for argument in args {
        let Some(parameter_slot) = argument.parameter_slot else {
            return Err(CompilerError::compiler_error(
                "Receiver call argument is missing its retained parameter slot",
            )
            .into());
        };
        full_arguments
            .push(argument.with_parameter_slot(ParameterSlot::new(parameter_slot.index() + 1)));
    }

    let call = HandledFallibleCall {
        name: method_path,
        args: full_arguments,
        result_type_ids,
        call_span,
    };
    match authored_handling {
        // A resumed call keeps the ordinary unhandled-typed-producer shape: whole-expression catch
        // completion owns the delivery decision, exactly as the completed call path leaves it.
        DeferredCallHandling::Unhandled => {
            let expression = expression_builder
                .handled_fallible_function_call_with_typed_arguments(
                    call.name,
                    call.args,
                    call.result_type_ids,
                    error_return_type_id,
                    FallibleExpressionHandling::Recover,
                    call.call_span,
                )?;
            Ok(expression_builder.with_typed_error_producer(
                expression,
                error_return_type_id,
                None,
            )?)
        }
        // Replay the authored `!` through the same constructor the completed call path uses; the
        // retention site already validated the callee's error slot against the authored boundary.
        DeferredCallHandling::Propagate { span } => call.into_expression(
            FallibleHandling::Propagate,
            span.or(call_span),
            error_return_type_id,
            expression_builder,
        ),
    }
}
