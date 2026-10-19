//! Function signature resolution for AST type resolution.

use super::ResolvedFunctionSignature;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionParameter, FunctionSignature, ParameterContract, ReturnChannel, ReturnSlot,
    UnresolvedFunctionSignature,
};
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionResult, resolve_parsed_type_annotation,
    reusable_type_id_for_parsed_ref,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverDeclarationReason, InvalidRouteOutputReason,
    InvalidThisUsageReason, InvalidTypeAnnotationReason, TypeAnnotationContext,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::ids::{FunctionTypeKey, GenericParameterListId, TypeId};
use crate::compiler_frontend::declaration_syntax::type_syntax::ParsedParameterContractSyntax;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Intern the callable contract from its resolved slots, without copying diagnostic syntax.
pub(crate) fn intern_resolved_function_signature(
    signature: &FunctionSignature,
    type_environment: &mut TypeEnvironment,
) -> TypeId {
    let parameters = signature
        .parameters
        .iter()
        .map(|parameter| parameter.type_id)
        .collect();
    let mut returns = Vec::with_capacity(signature.success_return_slot_count());
    let mut error_return = None;
    for return_slot in &signature.returns {
        // Signature resolution populates every slot before the callable contract is interned.
        let type_id = return_slot
            .type_id
            .expect("resolved callable return slot has no type ID");
        match return_slot.channel {
            ReturnChannel::Success => returns.push(type_id),
            ReturnChannel::Error => error_return = Some(type_id),
        }
    }
    type_environment.intern_function(FunctionTypeKey {
        parameters,
        returns: returns.into_boxed_slice(),
        error_return,
    })
}

// -------------------------------
//  Function signature resolution
// -------------------------------

/// Resolve a function signature and extract receiver metadata for method cataloging.
pub(crate) fn resolve_function_signature(
    function_path: &PathId,
    signature: &UnresolvedFunctionSignature,
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_resolution_context: &mut TypeResolutionContext<'_>,
    path_fork: &PathInternerFork,
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> TypeResolutionResult<ResolvedFunctionSignature> {
    let this_name = string_table.intern("this");
    let function_name_id = path_fork
        .component(*function_path)
        .unwrap_or_else(|| string_table.intern("<function>"));

    let function_span = type_resolution_context
        .declaration_table
        .get_by_path(function_path)
        .and_then(|declaration| declaration.binding_span);

    let mut resolved_parameters = Vec::with_capacity(signature.parameters.len());
    let mut receiver = None;

    // --------------------
    //  Resolve parameters
    // --------------------

    for (parameter_index, parameter) in signature.parameters.iter().enumerate() {
        let type_id = if let Some(type_syntax) = &parameter.type_syntax {
            let type_id = if let Some(type_id) = reusable_type_id_for_parsed_ref(
                type_syntax,
                Some(parameter.type_id),
                type_resolution_context,
            ) {
                Some(type_id)
            } else {
                resolve_parsed_type_annotation(
                    type_syntax,
                    parameter.binding_span,
                    type_resolution_context,
                    string_table,
                    scope_context,
                )?
                .type_id
            };
            type_id
                .filter(|type_id| *type_id != builtin_type_ids::NONE)
                .ok_or_else(|| {
                    CompilerDiagnostic::invalid_type_annotation(
                        TypeAnnotationContext::SignatureParameter,
                        InvalidTypeAnnotationReason::NoneNotAllowed,
                        parameter.binding_span,
                    )
                })?
        } else if parameter.type_id != builtin_type_ids::NONE
            && type_resolution_context
                .type_environment
                .get(parameter.type_id)
                .is_some()
        {
            parameter.type_id
        } else {
            return Err(CompilerDiagnostic::invalid_type_annotation(
                TypeAnnotationContext::SignatureParameter,
                InvalidTypeAnnotationReason::NoneNotAllowed,
                parameter.binding_span,
            ));
        };
        let contract = match &parameter.contract_syntax {
            ParsedParameterContractSyntax::Value => ParameterContract::Value,
            ParsedParameterContractSyntax::Wire => ParameterContract::Wire,
            ParsedParameterContractSyntax::Route { input, output_span } => {
                if !route_output_is_choice_or_generic(
                    type_id,
                    type_resolution_context.type_environment,
                ) {
                    return Err(CompilerDiagnostic::invalid_route_output(
                        InvalidRouteOutputReason::NotChoiceOrGeneric,
                        type_id,
                        output_span.or(parameter.binding_span),
                    ));
                }
                let input = input
                    .as_ref()
                    .map(|input_syntax| {
                        resolve_parsed_type_annotation(
                            input_syntax,
                            parameter.binding_span,
                            type_resolution_context,
                            string_table,
                            scope_context,
                        )
                        .map(|input| input.type_id)
                    })
                    .transpose()?
                    .flatten();
                ParameterContract::Route { input }
            }
        };
        let resolved_parameter = FunctionParameter {
            id: parameter.id,
            type_id,
            value_mode: parameter.value_mode,
            default: parameter.default,
            binding_span: parameter.binding_span,
            contract,
        };
        if path_fork.component(resolved_parameter.id) == Some(this_name) {
            if receiver.is_some() {
                return Err(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::DuplicateThis {
                        function_name: function_name_id,
                    },
                    parameter.binding_span,
                ));
            }

            if parameter_index != 0 {
                return Err(CompilerDiagnostic::invalid_this_usage(
                    InvalidThisUsageReason::NotFirstParameter {
                        function_name: function_name_id,
                    },
                    parameter.binding_span,
                ));
            }

            let receiver_key = receiver_key_for_resolved_parameter(
                function_name_id,
                resolved_parameter.type_id,
                generic_parameter_list_id,
                type_resolution_context.type_environment,
                parameter.binding_span,
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
        let type_id = if let Some(type_syntax) = &return_slot.type_syntax {
            if let Some(type_id) = reusable_type_id_for_parsed_ref(
                type_syntax,
                return_slot.type_id,
                type_resolution_context,
            ) {
                Some(type_id)
            } else {
                resolve_parsed_type_annotation(
                    type_syntax,
                    function_span,
                    type_resolution_context,
                    string_table,
                    scope_context,
                )?
                .type_id
            }
        } else {
            return_slot.type_id
        }
        .filter(|type_id| {
            *type_id != builtin_type_ids::NONE
                && type_resolution_context
                    .type_environment
                    .get(*type_id)
                    .is_some()
        })
        .ok_or_else(|| {
            CompilerDiagnostic::invalid_type_annotation(
                TypeAnnotationContext::SignatureReturn,
                InvalidTypeAnnotationReason::NoneNotAllowed,
                function_span,
            )
        })?;

        resolved_returns.push(ReturnSlot {
            type_id: Some(type_id),
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
                    unsupported_receiver_type_diagnostic(function_name_id, receiver_type_id, span)
                });
        }

        return Err(generic_receiver_type_diagnostic(
            function_name_id,
            receiver_type_id,
            span,
        ));
    }

    // Exact numeric scalars are builtin scalar values, like Bool, String and Char, even though
    // they have no receiver-method key of their own.
    if type_environment.fixed_scalar(receiver_type_id).is_some() {
        return Err(CompilerDiagnostic::invalid_receiver_declaration(
            InvalidReceiverDeclarationReason::BuiltinScalarType,
            span,
        ));
    }

    type_environment
        .receiver_key_for_type_id(receiver_type_id)
        .ok_or_else(|| {
            unsupported_receiver_type_diagnostic(function_name_id, receiver_type_id, span)
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
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::GenericReceiverType {
            function_name: function_name_id,
            type_id: receiver_type_id,
        },
        span,
    )
}

fn unsupported_receiver_type_diagnostic(
    function_name_id: StringId,
    receiver_type_id: TypeId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name: function_name_id,
            type_id: receiver_type_id,
        },
        span,
    )
}
fn route_output_is_choice_or_generic(type_id: TypeId, type_environment: &TypeEnvironment) -> bool {
    match type_environment.get(type_id) {
        Some(TypeDefinition::Choice(_)) | Some(TypeDefinition::GenericParameter(_)) => true,
        Some(TypeDefinition::GenericInstance(instance)) => type_environment
            .type_id_for_nominal_id(instance.base)
            .and_then(|base_type_id| type_environment.get(base_type_id))
            .is_some_and(|definition| matches!(definition, TypeDefinition::Choice(_))),
        _ => false,
    }
}
