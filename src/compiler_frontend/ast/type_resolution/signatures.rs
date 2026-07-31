//! Function signature resolution for AST type resolution.

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
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use super::{ResolvedFunctionSignature, resolve_named_signature_type};

// -------------------------------
//  Function signature resolution
// -------------------------------

/// Resolve a function signature and extract receiver metadata for method cataloging.
pub(crate) fn resolve_function_signature(
    function_path: &InternedPath,
    signature: &FunctionSignature,
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
) -> TypeResolutionResult<ResolvedFunctionSignature> {
    let this_name = string_table.intern("this");
    let _function_name = function_path.name_str(string_table).unwrap_or("<function>");
    let function_name_id = function_path
        .name()
        .unwrap_or_else(|| string_table.intern("<function>"));

    let function_location = type_resolution_context
        .declaration_table
        .get_by_path(function_path)
        .map(|declaration| declaration.value.location.clone())
        .unwrap_or_default();

    let mut resolved_parameters = Vec::with_capacity(signature.parameters.len());
    let mut receiver = None;

    // --------------------
    //  Resolve parameters
    // --------------------

    for (parameter_index, parameter) in signature.parameters.iter().enumerate() {
        let mut resolved_parameter = parameter.to_owned();

        resolved_parameter.value.diagnostic_type = resolve_named_signature_type(
            &parameter.value.diagnostic_type,
            &parameter.value.location,
            type_resolution_context,
            string_table,
        )?;

        // Resolve the canonical TypeId for the parameter's data type so that
        // HIR lowering sees the correct type identity (not TypeId(0) from
        // Expression::new's builtin-only mapping).
        resolved_parameter.value.type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_parameter.value.diagnostic_type,
            type_resolution_context.type_environment,
            &resolved_parameter.value.location,
        )?;

        if resolved_parameter.id.name() == Some(this_name) {
            if receiver.is_some() {
                return Err(Box::new(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::DuplicateThis {
                        function_name: function_name_id,
                    },
                    parameter.value.location.clone(),
                )));
            }

            if parameter_index != 0 {
                return Err(Box::new(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::NotFirstParameter {
                        function_name: function_name_id,
                    },
                    parameter.value.location.clone(),
                )));
            }

            let receiver_key = receiver_key_for_resolved_parameter(
                function_name_id,
                resolved_parameter.value.type_id,
                generic_parameter_list_id,
                type_resolution_context.type_environment,
                &parameter.value.location,
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
            &function_location,
            type_resolution_context,
            string_table,
        )?;

        let type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_value,
            type_resolution_context.type_environment,
            &function_location,
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
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
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
                        location,
                        string_table,
                    )
                });
        }

        return Err(generic_receiver_type_diagnostic(
            function_name_id,
            receiver_type_id,
            type_environment,
            location,
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
                location,
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
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
    string_table: &mut StringTable,
) -> Box<CompilerDiagnostic> {
    let type_name = receiver_type_name(receiver_type_id, type_environment, string_table);
    Box::new(CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::GenericReceiverType {
            function_name: function_name_id,
            type_name,
        },
        location.clone(),
    ))
}

fn unsupported_receiver_type_diagnostic(
    function_name_id: StringId,
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
    string_table: &mut StringTable,
) -> Box<CompilerDiagnostic> {
    let type_name = receiver_type_name(receiver_type_id, type_environment, string_table);
    Box::new(CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name: function_name_id,
            type_name,
        },
        location.clone(),
    ))
}

fn receiver_type_name(
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> StringId {
    let spelling = diagnostic_type_spelling(receiver_type_id, type_environment);
    string_table.intern(&spelling.display_with_table(string_table))
}
