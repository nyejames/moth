//! Declared/source receiver method dispatch.
//!
//! WHAT: looks up user-declared receiver methods (including generic instantiation)
//!       and parses the resulting call AST node.
//! WHY: source methods have distinct lookup rules from generic-bound and static
//!      trait-surface dispatch; keeping them in one file makes the generic-instantiation
//!      path explicit and local.

use super::ReceiverAccessMode;
use super::shared::{TraitSurfaceReceiverMethod, receiver_result_type_ids_for_call};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext,
    expectations_from_receiver_method_signature, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::function_calls::{
    CallFinishContext, finish_function_call_expression,
};
use crate::compiler_frontend::ast::field_access::parse_chain::expression_from_postfix_node;
use crate::compiler_frontend::ast::field_access::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericReceiverMethodInstantiationInput, instantiate_generic_receiver_method,
};
use crate::compiler_frontend::ast::receiver_methods::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::fallible_handling::HandledFallibleCall;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReceiverCallReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

pub(super) fn lookup_receiver_method<'a>(
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
) -> Result<CallArgument, ExpressionParseError> {
    let receiver_access = if receiver_mutable {
        CallAccessMode::Mutable
    } else {
        CallAccessMode::Shared
    };

    Ok(CallArgument::positional(
        expression_from_postfix_node(receiver_node)?,
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
    pub(super) scope_context: &'a ScopeContext,
    pub(super) source_method: SourceReceiverMethodTarget<'a>,
    pub(super) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

pub(super) fn parse_source_receiver_method_target_call_typed(
    input: SourceReceiverMethodCallInput<'_, '_, '_>,
) -> Result<AstNode, ExpressionParseError> {
    let SourceReceiverMethodCallInput {
        token_stream,
        receiver_node,
        member_name,
        member_span,
        receiver_access_mode,
        authored_marker_span,
        scope_context,
        source_method,
        type_interner,
        string_table,
        path_fork,
    } = input;

    if receiver_node.expression_is_const_record_value()? {
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

    let method_name = string_table.resolve(member_name).to_owned();
    validate_receiver_access(
        receiver_node,
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
    let raw_args = parse_call_arguments_typed_with_expectations(
        token_stream,
        scope_context,
        type_interner,
        string_table,
        &initial_expectations,
        CallArgumentSyntax::Supported {
            callee_name: Some(member_name),
        },
        path_fork,
    )?;

    let (method_path, call_signature, generic_request, receiver_expression) = match &source_method {
        SourceReceiverMethodTarget::Declared(method_entry) => {
            let receiver_argument = receiver_hidden_call_argument(
                receiver_node,
                method_entry.receiver_mutable,
                member_span,
            )?;
            match scope_context.lookup_generic_function_template(&method_entry.function_path) {
                Some(template) => {
                    let instantiation = instantiate_generic_receiver_method(
                        GenericReceiverMethodInstantiationInput {
                            template,
                            receiver_argument,
                            authored_arguments: &raw_args,
                            call_span: member_span,
                            scope_context,
                            type_interner,
                            string_table,
                            path_fork,
                        },
                    )?;

                    (
                        instantiation.instance_path,
                        instantiation.signature,
                        Some(instantiation.request),
                        instantiation.receiver_argument.value,
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
            let receiver_argument =
                receiver_hidden_call_argument(receiver_node, method.receiver_mutable, member_span)?;
            match scope_context.lookup_generic_function_template(&method.method_path) {
                Some(template) => {
                    let instantiation = instantiate_generic_receiver_method(
                        GenericReceiverMethodInstantiationInput {
                            template,
                            receiver_argument,
                            authored_arguments: &raw_args,
                            call_span: member_span,
                            scope_context,
                            type_interner,
                            string_table,
                            path_fork,
                        },
                    )?;
                    (
                        instantiation.instance_path,
                        instantiation.signature,
                        Some(instantiation.request),
                        instantiation.receiver_argument.value,
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

    let expectations =
        expectations_from_receiver_method_signature(&call_signature.parameters[1..], path_fork);
    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::receiver_method(&method_name),
        &raw_args,
        &expectations,
        member_span,
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork,
            float_precision: scope_context.numeric_profile.float_precision,
        },
    )?;

    if !scope_context.generic_template_validation
        && let Some(request) = generic_request
    {
        scope_context.record_generic_function_instantiation_request(request);
    }

    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

    let method_call_expression = if let Some(error_return_type_id) =
        call_signature.error_return_type_id()
    {
        // Typed methods use the established handled-call shape, with their receiver in slot zero.
        // A raw MethodCall inside a carrier would bypass HIR's explicit error-channel call lowering.
        let receiver_span = receiver_expression.span;
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
        let method_call_expression = Expression::method_call_with_typed_arguments(
            receiver_expression,
            method_path,
            args,
            result_type_ids,
            type_interner.environment_mut_for_derived_types(),
            member_span,
        );
        let declared_requirement = matches!(
            &source_method,
            SourceReceiverMethodTarget::TraitSurface(method)
                if matches!(method.origin, super::shared::TraitSurfaceMethodOrigin::DeclaredRequirement)
        );
        if !declared_requirement
            && scope_context.source_call_has_private_failure_lane(source_method.method_path())
        {
            let failure_path = if scope_context.generic_template_validation {
                source_method.method_path()
            } else {
                method_path
            };
            method_call_expression.with_private_call_failure_candidate(failure_path)
        } else {
            method_call_expression
        }
    };

    Ok(AstNode {
        kind: NodeKind::ExpressionStatement(method_call_expression),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    })
}
