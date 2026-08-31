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
mod templates;

pub(crate) use body_rules::{GenericFunctionBodyValidationInput, validate_generic_function_body};
pub(crate) use calls::{
    GenericCallExpectedContext, GenericFunctionCallParseInput, GenericFunctionInferenceInput,
    concrete_argument_mapping, infer_generic_function_call, parse_generic_function_call_expression,
    substitute_function_signature, validate_generic_function_bound_evidence,
    validate_generic_function_template_call_expression,
};
pub(crate) use diagnostics::{
    GenericInstantiationDiagnosticContext, recursive_generic_function_instantiation,
    with_generic_instantiation_context,
};
pub(crate) use instances::{
    GenericFunctionInstance, GenericFunctionInstanceKey, GenericFunctionInstantiationRequest,
    GenericRequestRange, IfGenericRequestRanges,
};
pub(crate) use materialisation::{
    MaterialisedGenericAst, ModuleMaterialisationContext, ModuleMaterialisationEnvironmentInput,
    ModuleMaterialisationInput, ModuleMaterialisationPreparation,
    ModuleMaterialisationPreparationBuilder, bootstrap_call_summary_from_signature,
};
pub(crate) use templates::{GenericFunctionBody, GenericFunctionTemplate};

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
