//! Shared template folding helpers for AST finalization.
//!
//! WHAT: Provides common template folding utilities used by both AST node
//! normalization and module constant normalization.
//!
//! WHY: Consolidates duplicated template folding logic to ensure consistent
//! behavior across all normalization contexts.

use crate::compiler_frontend::ast::module_ast::finalization::normalize_ast::TemplateNormalizationError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateHelperKind, TemplateIrStore, TemplatePreparation, TemplatePreparationMode,
    TemplatePreparationOutcome, TemplateTirPhase, TirView, fold_prepared_template,
    prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use std::cell::RefCell;
use std::rc::Rc;

/// Exclusive finalization result for one prepared template value.
///
/// WHAT: pairs exactly one semantic outcome with the data needed by its owner.
/// WHY: a folded value, runtime proof and helper artifact must never be represented
///      as independent optional/disposition fields that can contradict each other.
pub(super) enum FinalizedTemplateValue {
    Folded(StringId, SyntheticInterfaceProvenance),
    Runtime(TemplatePreparation),
    Helper(TemplateHelperKind),
}

/// Prepares and finalizes one exact template value for its owning boundary.
pub(super) fn finalize_template_value(
    template: &Template,
    fold_inputs: TemplateValueFinalizationInputs<'_, '_>,
    preparation_mode: TemplatePreparationMode,
) -> Result<FinalizedTemplateValue, TemplateNormalizationError> {
    let reference = &template.tir_reference;

    if !reference.phase.is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "AST finalization template folding requires Composed-or-later TIR, but root {} is at phase {}.",
            reference.root, reference.phase
        ))
        .into());
    }

    let store_handle = Rc::clone(fold_inputs.template_ir_store);

    increment_ast_counter(AstCounter::TirFinalizationFoldAttempts);

    let store = store_handle.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;

    // Preparation validates and classifies the exact view before cache lookup
    // or folding. Its compact result is the sole final-value decision source.
    let preparation = prepare_tir_view(&view, preparation_mode)?;
    let fold_preparation = match preparation.outcome {
        TemplatePreparationOutcome::Helper(kind) => {
            increment_ast_counter(AstCounter::TirFinalizationFoldSuccesses);
            return Ok(FinalizedTemplateValue::Helper(kind));
        }
        TemplatePreparationOutcome::Runtime(_) => {
            return Ok(FinalizedTemplateValue::Runtime(preparation));
        }
        TemplatePreparationOutcome::Foldable => preparation,
    };

    let mut fold_context = make_fold_context(
        fold_inputs.string_table,
        fold_inputs.template_const_loop_iteration_limit,
    );
    let result = fold_prepared_template(&fold_preparation, view, &mut fold_context)?;
    let provenance = result.provenance;
    let folded = template_emission_to_string_id(result.emission, &mut fold_context)?;
    increment_ast_counter(AstCounter::TemplatesFoldedDuringFinalization);
    increment_ast_counter(AstCounter::TirFinalizationFoldSuccesses);
    Ok(FinalizedTemplateValue::Folded(folded, provenance))
}

fn template_emission_to_string_id(
    emission: TemplateEmission,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<StringId, TemplateNormalizationError> {
    match emission {
        TemplateEmission::NoOutput => Ok(fold_context.string_table.intern("")),
        TemplateEmission::Output(output) => Ok(output),
        TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
            Err(CompilerError::compiler_error(
                "Template loop-control signal escaped the nearest template loop during folding.",
            )
            .into())
        }
    }
}

/// Inputs for finalization-time template folding.
///
/// WHAT: bundles the string interner, loop policy and TIR ownership handle needed
/// to build the exact finalization view and run one fold operation.
/// WHY: finalization owns the module-store handle while the active `TirView`
/// carries structural authority through preparation and folding.
pub(super) struct TemplateValueFinalizationInputs<'store, 'strings> {
    pub(super) string_table: &'strings mut StringTable,
    pub(super) template_const_loop_iteration_limit: usize,
    pub(super) template_ir_store: &'store Rc<RefCell<TemplateIrStore>>,
}

/// Creates a narrow TIR fold context from finalization parameters.
///
pub(super) fn make_fold_context<'a>(
    string_table: &'a mut StringTable,
    template_const_loop_iteration_limit: usize,
) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table,
        template_const_loop_iteration_limit,
        bindings: Vec::new(),
    }
}
