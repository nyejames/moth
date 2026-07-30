//! AST-owned generic free-function model.
//!
//! WHAT: keeps parsed generic function bodies as immutable templates and defines the
//! concrete-call inference and instance-emission records for visible generic free functions.
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
};
pub(crate) use materialisation::{
    MaterialisedGenericAst, ModuleMaterialisationContext, bootstrap_call_summary_from_signature,
};
pub(crate) use templates::GenericFunctionTemplate;

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
