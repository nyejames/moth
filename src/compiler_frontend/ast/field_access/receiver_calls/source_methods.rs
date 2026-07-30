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
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, normalize_call_arguments,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext,
    expectations_from_receiver_method_signature, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::function_calls::{
    NamedArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::field_access::parse_chain::expression_from_postfix_node;
use crate::compiler_frontend::ast::field_access::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericFunctionInferenceInput, GenericFunctionInstantiationRequest,
    infer_generic_function_call, recursive_generic_function_instantiation,
    validate_generic_function_bound_evidence,
};
use crate::compiler_frontend::ast::receiver_methods::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReceiverCallReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

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
}

pub(super) struct SourceReceiverMethodCallInput<'a, 'interner> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) receiver_node: &'a AstNode,
    pub(super) member_name: StringId,
    pub(super) member_location: SourceLocation,
    pub(super) receiver_access_mode: ReceiverAccessMode,
    pub(super) authored_marker_location: Option<SourceLocation>,
    pub(super) scope_context: &'a ScopeContext,
    pub(super) source_method: SourceReceiverMethodTarget<'a>,
    pub(super) type_interner: &'a mut AstTypeInterner<'interner>,
    pub(super) string_table: &'a mut StringTable,
}

pub(super) fn parse_source_receiver_method_target_call_typed(
    input: SourceReceiverMethodCallInput<'_, '_>,
) -> Result<AstNode, ExpressionParseError> {
    let SourceReceiverMethodCallInput {
        token_stream,
        receiver_node,
        member_name,
        member_location,
        receiver_access_mode,
        authored_marker_location,
        scope_context,
        source_method,
        type_interner,
        string_table,
    } = input;

    if receiver_node.expression_is_const_record_value()? {
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::ConstRecordNoRuntimeCalls,
            None,
            Some(member_name),
            None,
            None,
            member_location,
        )
        .into());
    }

    if token_stream.peek_next_token() != Some(&TokenKind::OpenParenthesis) {
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::MustUseParentheses,
            None,
            Some(member_name),
            None,
            None,
            member_location,
        )
        .into());
    }

    token_stream.advance();

    let method_name = string_table.resolve(member_name).to_owned();
    validate_receiver_access(
        receiver_node,
        receiver_access_mode,
        &member_location,
        authored_marker_location.as_ref(),
        ReceiverAccessRequirement {
            requires_mutable: source_method.receiver_mutable(),
            diagnostic: ReceiverAccessDiagnostic::ReceiverMethod {
                method_name: member_name,
            },
        },
    )?;

    let initial_expectations =
        expectations_from_receiver_method_signature(&source_method.signature().parameters[1..]);
    let raw_args = parse_call_arguments_typed_with_expectations(
        token_stream,
        scope_context,
        type_interner,
        string_table,
        &initial_expectations,
        NamedArgumentSyntax::Supported {
            callee_name: Some(member_name),
        },
    )?;

    let (method_path, call_signature, generic_request) = match &source_method {
        SourceReceiverMethodTarget::Declared(method_entry) => {
            if let Some(template) =
                scope_context.lookup_generic_function_template(&method_entry.function_path)
            {
                let receiver_expr = expression_from_postfix_node(receiver_node)?;
                let receiver_access = if method_entry.receiver_mutable {
                    CallAccessMode::Mutable
                } else {
                    CallAccessMode::Shared
                };
                let receiver_arg = CallArgument::positional(
                    receiver_expr,
                    receiver_access,
                    member_location.clone(),
                );

                let mut inference_args = Vec::with_capacity(raw_args.len() + 1);
                inference_args.push(receiver_arg);
                inference_args.extend(raw_args.iter().cloned());

                let inference = infer_generic_function_call(GenericFunctionInferenceInput {
                    template,
                    raw_arguments: &inference_args,
                    expected_context: GenericCallExpectedContext::None,
                    call_location: member_location.clone(),
                    type_environment: type_interner.environment_mut_for_derived_types(),
                    string_table,
                })?;
                let selected_evidence = validate_generic_function_bound_evidence(
                    template,
                    inference.key.type_arguments.as_ref(),
                    scope_context,
                    type_interner.environment(),
                    member_location.clone(),
                )?;

                if scope_context.is_generic_function_instantiation_active(&inference.key) {
                    return Err(recursive_generic_function_instantiation(
                        template.function_path.name(),
                        member_location.clone(),
                    )
                    .into());
                }

                let request = GenericFunctionInstantiationRequest {
                    declaration_identity: template.declaration_identity.clone(),
                    evidence: selected_evidence,
                    key: inference.key,
                    instance_path: inference.instance_path.clone(),
                    call_location: member_location.clone(),
                };

                (inference.instance_path, inference.signature, Some(request))
            } else {
                (
                    method_entry.function_path.to_owned(),
                    method_entry.signature.to_owned(),
                    None,
                )
            }
        }

        SourceReceiverMethodTarget::TraitSurface(method) => {
            (method.method_path.clone(), method.signature.clone(), None)
        }
    };

    let expectations = expectations_from_receiver_method_signature(&call_signature.parameters[1..]);
    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::receiver_method(&method_name),
        &raw_args,
        &expectations,
        member_location.clone(),
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )?;
    let result_type_ids = receiver_result_type_ids_for_call(
        call_signature.success_return_type_ids(),
        call_signature.error_return_type_id(),
        token_stream,
        type_interner,
    )?;

    if let Some(request) = generic_request {
        scope_context.record_generic_function_instantiation_request(request);
    }

    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

    let receiver_expression = expression_from_postfix_node(receiver_node)?;
    let method_call_expression = Expression::method_call_with_typed_arguments(
        receiver_expression,
        method_path,
        normalize_call_arguments(&args),
        result_type_ids,
        type_interner.environment_mut_for_derived_types(),
        member_location.clone(),
    );

    Ok(AstNode {
        kind: NodeKind::ExpressionStatement(method_call_expression),
        scope: scope_context.scope.to_owned(),
        location: member_location,
    })
}
