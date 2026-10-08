//! Structured template control-flow metadata and AST-stage helpers.
//!
//! Template control flow is parsed in the template head/body pipeline and
//! emitted directly as TIR `Conditional` and `Loop` nodes. The sibling modules
//! separate shared control-flow data from validation and const-loop mechanics.

mod const_eval;
mod const_folding;
mod types;
mod validation;

pub(crate) use const_eval::{
    collect_option_capture_binding_path, inline_source_consts_for_const_required_expression,
    inline_source_consts_for_const_required_if_condition, loop_body_const_evaluation_bindings,
};
#[cfg(test)]
pub(crate) use const_folding::ConstRangeIterationValue;
pub(crate) use const_folding::{
    ConstRangeCursor, TemplateFoldBinding, build_collection_iteration_bindings,
    build_range_iteration_bindings, const_collection_items,
};
pub(crate) use types::{
    TemplateBodyEmission, TemplateBodyParseMode, TemplateBranchSelector,
    TemplateControlFlowValidationMode, TemplateIfBodyParseInput, TemplateLoopBodyParseInput,
    TemplateLoopHeader,
};
pub(crate) use validation::validate_runtime_template_control_flow_slot_artifacts;
