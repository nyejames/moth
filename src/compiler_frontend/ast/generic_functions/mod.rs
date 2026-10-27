//! AST-owned generic function and receiver-template model.
//!
//! WHAT: keeps parsed generic function and receiver-method bodies as immutable templates and
//! defines the concrete-call inference and instance-emission records for visible generic calls.
//! WHY: generic functions must be solved and instantiated before HIR lowering. The AST
//! stage owns that boundary so backends never receive unresolved generic parameters.

mod body_rules;
mod calls;
mod diagnostics;
mod instances;
mod materialisation;
pub(crate) mod numeric_origins;
mod receiver_method_instantiation;
mod templates;

pub(crate) use body_rules::{GenericFunctionBodyValidationInput, validate_generic_function_body};
pub(crate) use calls::{
    GenericCallExpectedContext, GenericFunctionCallParseInput, GenericFunctionInferenceInput,
    choose_retained_argument_destination, complete_deferred_generic_call,
    infer_generic_function_call, parse_generic_function_call_expression,
    substitute_function_signature, validate_generic_function_bound_evidence,
    validate_generic_function_template_call_expression, verify_completed_error_return,
    verify_retained_propagation_error,
};
pub(crate) use diagnostics::{
    GenericInstantiationDiagnosticContext, recursive_generic_function_instantiation,
    with_generic_instantiation_context,
};
pub(crate) use instances::{
    GenericFunctionInstance, GenericFunctionInstanceKey, GenericFunctionInstantiationRequest,
    GenericRequestRange, GenericRequestSlot, GenericRequestTicket, IfGenericRequestRanges,
};
pub(crate) use materialisation::{
    MaterialisedGenericAst, ModuleMaterialisationCapture, ModuleMaterialisationContext,
    ModuleMaterialisationEnvironmentInput, ModuleMaterialisationInput,
    ModuleMaterialisationPreparation, ModuleMaterialisationPreparationBuilder,
    ModuleMaterialisationPreparedBuilder, StableFunctionTarget,
    bootstrap_call_summary_from_signature,
};
pub(crate) use receiver_method_instantiation::{
    GenericReceiverMethodInstantiationInput, GenericReceiverMethodReceiver,
    instantiate_generic_cast_evidence_method, instantiate_generic_receiver_method,
};
pub(crate) use templates::{
    GenericFunctionBody, GenericFunctionTemplate, MaterialisedDonorContext,
};

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
