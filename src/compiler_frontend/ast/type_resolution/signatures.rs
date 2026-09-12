//! Function signature resolution for AST type resolution.

use super::{ResolvedFunctionSignature, resolve_named_signature_type};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionResult, resolve_diagnostic_type_to_type_id_checked,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverDeclarationReason, InvalidThisUsageReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterListId, TypeId};
use crate::compiler_frontend::datatypes::{ReceiverKey, diagnostic_type_spelling};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

// -------------------------------
//  Function signature resolution
// -------------------------------

/// Resolve a function signature and extract receiver metadata for method cataloging.
pub(crate) fn resolve_function_signature(
    function_path: &PathId,
    signature: &FunctionSignature,
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_resolution_context: &mut TypeResolutionContext<'_>,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> TypeResolutionResult<ResolvedFunctionSignature> {
    let this_name = string_table.intern("this");
    let function_name_id = path_fork
        .component(*function_path)
        .unwrap_or_else(|| string_table.intern("<function>"));

    let function_span = type_resolution_context
        .declaration_table
        .get_by_path(function_path)
        .and_then(|declaration| declaration.value.span);

    let mut resolved_parameters = Vec::with_capacity(signature.parameters.len());
    let mut receiver = None;

    // --------------------
    //  Resolve parameters
    // --------------------

    for (parameter_index, parameter) in signature.parameters.iter().enumerate() {
        let mut resolved_parameter = parameter.to_owned();

        resolved_parameter.value.diagnostic_type = resolve_named_signature_type(
            &parameter.value.diagnostic_type,
            parameter.value.span,
            type_resolution_context,
            string_table,
        )?;

        // Resolve the canonical TypeId for the parameter's data type so that
        // HIR lowering sees the correct type identity (not TypeId(0) from
        // Expression::new's builtin-only mapping).
        resolved_parameter.value.type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_parameter.value.diagnostic_type,
            type_resolution_context.type_environment,
            resolved_parameter.value.span,
        )?;
        if path_fork.component(resolved_parameter.id) == Some(this_name) {
            if receiver.is_some() {
                return Err(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::DuplicateThis {
                        function_name: function_name_id,
                    },
                    parameter.value.span,
                ));
            }

            if parameter_index != 0 {
                return Err(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::NotFirstParameter {
                        function_name: function_name_id,
                    },
                    parameter.value.span,
                ));
            }

            let receiver_key = receiver_key_for_resolved_parameter(
                function_name_id,
                resolved_parameter.value.type_id,
                generic_parameter_list_id,
                type_resolution_context.type_environment,
                parameter.value.span,
                path_fork,
                string_table,
            )?;

            receiver = Some(receiver_key);
        }

        resolved_parameters.push(resolved_parameter);
    }

    // -----------------
    //  Resolve returns
    // -----------------

    let mut resolved_returns = Vec::with_capacity(signature.returns.len());

    for return_slot in &signature.returns {
        let resolved_value = resolve_named_signature_type(
            &return_slot.value,
            function_span,
            type_resolution_context,
            string_table,
        )?;

        let type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_value,
            type_resolution_context.type_environment,
            function_span,
        )?;

        resolved_returns.push(ReturnSlot {
            value: resolved_value,
            type_id: Some(type_id),
            reactive_template: return_slot.reactive_template.clone(),
            channel: return_slot.channel,
        });
    }

    Ok(ResolvedFunctionSignature {
        receiver,
        signature: FunctionSignature {
            parameters: resolved_parameters,
            returns: resolved_returns,
        },
    })
}

fn receiver_key_for_resolved_parameter(
    function_name_id: StringId,
    receiver_type_id: TypeId,
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_environment: &TypeEnvironment,
    span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> TypeResolutionResult<ReceiverKey> {
    if let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(receiver_type_id)
    {
        if generic_receiver_arguments_align(
            instance.arguments.as_ref(),
            generic_parameter_list_id,
            type_environment,
        ) {
            return receiver_key_for_generic_instance_base(receiver_type_id, type_environment)
                .ok_or_else(|| {
                    unsupported_receiver_type_diagnostic(
                        function_name_id,
                        receiver_type_id,
                        type_environment,
                        span,
                        path_fork,
                        string_table,
                    )
                });
        }

        return Err(generic_receiver_type_diagnostic(
            function_name_id,
            receiver_type_id,
            type_environment,
            span,
            path_fork,
            string_table,
        ));
    }

    type_environment
        .receiver_key_for_type_id(receiver_type_id)
        .ok_or_else(|| {
            unsupported_receiver_type_diagnostic(
                function_name_id,
                receiver_type_id,
                type_environment,
                span,
                path_fork,
                string_table,
            )
        })
}

fn generic_receiver_arguments_align(
    receiver_arguments: &[TypeId],
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_environment: &TypeEnvironment,
) -> bool {
    let Some(generic_parameter_list_id) = generic_parameter_list_id else {
        return false;
    };

    let Some(method_parameters) = type_environment.generic_parameters(generic_parameter_list_id)
    else {
        return false;
    };

    if method_parameters.parameters.len() != receiver_arguments.len() {
        return false;
    }

    for (receiver_argument, method_parameter) in receiver_arguments
        .iter()
        .zip(method_parameters.parameters.iter())
    {
        let Some(TypeDefinition::GenericParameter(receiver_parameter)) =
            type_environment.get(*receiver_argument)
        else {
            return false;
        };

        if receiver_parameter.id != method_parameter.id {
            return false;
        }
    }

    true
}

fn receiver_key_for_generic_instance_base(
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<ReceiverKey> {
    let TypeDefinition::GenericInstance(instance) = type_environment.get(receiver_type_id)? else {
        return None;
    };

    let base_type_id = type_environment.type_id_for_nominal_id(instance.base)?;
    type_environment.receiver_key_for_type_id(base_type_id)
}

fn generic_receiver_type_diagnostic(
    function_name_id: StringId,
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let type_name = receiver_type_name(receiver_type_id, type_environment, path_fork, string_table);
    CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::GenericReceiverType {
            function_name: function_name_id,
            type_name,
        },
        span,
    )
}

fn unsupported_receiver_type_diagnostic(
    function_name_id: StringId,
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    span: Option<SourceSpan>,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let type_name = receiver_type_name(receiver_type_id, type_environment, path_fork, string_table);
    CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name: function_name_id,
            type_name,
        },
        span,
    )
}

fn receiver_type_name(
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
) -> StringId {
    let spelling = diagnostic_type_spelling(receiver_type_id, type_environment);
    let path_table = path_fork.snapshot_table();
    string_table.intern(&spelling.display_with_table(string_table, &path_table))
}
