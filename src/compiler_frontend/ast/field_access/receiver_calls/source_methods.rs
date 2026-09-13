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
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidReceiverCallReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

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

struct GenericReceiverMethodInferenceInput<'a, 'interner> {
    template: &'a crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate,
    receiver_node: &'a AstNode,
    receiver_mutable: bool,
    raw_args: &'a [CallArgument],
    member_span: Option<SourceSpan>,
    scope_context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'interner>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

fn infer_generic_receiver_method_target<'a, 'interner>(
    input: GenericReceiverMethodInferenceInput<'a, 'interner>,
) -> Result<
    (
        PathId,
        FunctionSignature,
        GenericFunctionInstantiationRequest,
    ),
    ExpressionParseError,
> {
    let GenericReceiverMethodInferenceInput {
        template,
        receiver_node,
        receiver_mutable,
        raw_args,
        member_span,
        scope_context,
        type_interner,
        string_table,
        path_fork,
    } = input;
    let receiver_expr = expression_from_postfix_node(receiver_node)?;
    let receiver_access = if receiver_mutable {
        CallAccessMode::Mutable
    } else {
        CallAccessMode::Shared
    };
    let receiver_arg = CallArgument::positional(receiver_expr, receiver_access, member_span)
        .with_parameter_slot(ParameterSlot::new(0));

    let mut inference_args = Vec::with_capacity(raw_args.len() + 1);
    inference_args.push(receiver_arg);
    for argument in raw_args {
        let Some(parameter_slot) = argument.parameter_slot else {
            return Err(CompilerError::compiler_error(
                "Receiver call argument is missing its retained parameter slot",
            )
            .into());
        };
        inference_args.push(
            argument
                .clone()
                .with_parameter_slot(ParameterSlot::new(parameter_slot.index() + 1)),
        );
    }

    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: &inference_args,
        expected_context: GenericCallExpectedContext::None,
        call_span: member_span,
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
        member_span,
    )?;

    if scope_context.is_generic_function_instantiation_active(&inference.key) {
        return Err(recursive_generic_function_instantiation(
            path_fork.component(template.function_path),
            member_span,
        )
        .into());
    }

    let request = GenericFunctionInstantiationRequest {
        declaration_identity: template.declaration_identity.clone(),
        evidence: selected_evidence,
        key: inference.key,
        instance_path: inference.instance_path.clone(),
        call_span: member_span,
    };

    Ok((inference.instance_path, inference.signature, request))
}

pub(super) struct SourceReceiverMethodCallInput<'a, 'interner> {
    pub(super) token_stream: &'a mut FileTokens,
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
    input: SourceReceiverMethodCallInput<'_, '_>,
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

    if token_stream.peek_next_token() != Some(&TokenKind::OpenParenthesis) {
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

    let (method_path, call_signature, generic_request) = match &source_method {
        SourceReceiverMethodTarget::Declared(method_entry) => {
            if let Some(template) =
                scope_context.lookup_generic_function_template(&method_entry.function_path)
            {
                let (instance_path, signature, request) =
                    infer_generic_receiver_method_target(GenericReceiverMethodInferenceInput {
                        template,
                        receiver_node,
                        receiver_mutable: method_entry.receiver_mutable,
                        raw_args: &raw_args,
                        member_span,
                        scope_context,
                        type_interner,
                        string_table,
                        path_fork,
                    })?;

                (instance_path, signature, Some(request))
            } else {
                (
                    method_entry.function_path.to_owned(),
                    method_entry.signature.to_owned(),
                    None,
                )
            }
        }

        SourceReceiverMethodTarget::TraitSurface(method) => {
            if let Some(template) =
                scope_context.lookup_generic_function_template(&method.method_path)
            {
                let (instance_path, signature, request) =
                    infer_generic_receiver_method_target(GenericReceiverMethodInferenceInput {
                        template,
                        receiver_node,
                        receiver_mutable: method.receiver_mutable,
                        raw_args: &raw_args,
                        member_span,
                        scope_context,
                        type_interner,
                        string_table,
                        path_fork,
                    })?;
                (instance_path, signature, Some(request))
            } else {
                (method.method_path.clone(), method.signature.clone(), None)
            }
        }
    };

    let expectations = expectations_from_receiver_method_signature(
        &call_signature.parameters[1..],
        path_fork,
    );
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
        },
    )?;
    let result_type_ids = receiver_result_type_ids_for_call(
        call_signature.success_return_type_ids(),
        call_signature.error_return_type_id(),
        token_stream,
        type_interner,
    )?;

    if !scope_context.generic_template_validation
        && let Some(request) = generic_request
    {
        scope_context.record_generic_function_instantiation_request(request);
    }

    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

    let receiver_expression = expression_from_postfix_node(receiver_node)?;
    let method_call_expression = Expression::method_call_with_typed_arguments(
        receiver_expression,
        method_path,
        args,
        result_type_ids,
        type_interner.environment_mut_for_derived_types(),
        member_span,
    );

    Ok(AstNode {
        kind: NodeKind::ExpressionStatement(method_call_expression),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    })
}
