//! Module constant normalization for HIR metadata preparation.
//!
//! WHAT: Normalizes module constant expressions by folding compile-time
//! templates and recursively processing collections and struct instances.
//! Returns normalized constants as HIR metadata, not runtime statements.
//!
//! WHY: Module constants are compile-time metadata that HIR exposes for
//! tooling and codegen decisions. They require separate normalization from
//! AST nodes because they must be fully foldable at compile time.
//!
//! ## Difference from AST Node Normalization
//!
//! **AST Node Normalization**:
//! - Mutates nodes in place
//! - Handles both constant and runtime templates
//! - Preserves runtime template structure for HIR lowering
//!
//! **Module Constant Normalization**:
//! - Returns new normalized expressions
//! - Only handles compile-time foldable templates
//! - Rejects non-constant templates as compiler errors
//! - Filters out `SlotInsertHelper` constants (composition-only)
//!
//! ## SlotInsertHelper Filtering
//!
//! `$insert(..)` helper constants only exist for AST template composition.
//! They don't have stable backend-facing value shapes, so HIR must not
//! receive them as module constants. They are filtered during normalization.

use super::finalizer::AstFinalizer;
use super::normalize_ast::TemplateNormalizationError;
use super::template_helpers::{
    FinalizedTemplateValue, TemplateValueFinalizationInputs, finalize_template_value,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateHelperKind, TemplateIrStore, TemplatePreparationMode, TemplatePreparationOutcome,
    TemplateTirPhase, TirView, prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;

impl AstFinalizer<'_, '_> {
    /// Normalizes module constants for HIR.
    ///
    /// WHAT: Folds compile-time templates in module constants and filters
    /// out composition-only helpers like `SlotInsertHelper`.
    ///
    /// WHY: Module constants are HIR metadata, not runtime statements. They
    /// must be fully foldable and have stable backend-facing shapes.
    pub(super) fn normalize_module_constants_for_hir(
        &self,
        string_table: &mut StringTable,
    ) -> Result<Vec<Declaration>, TemplateNormalizationError> {
        let mut normalized_constants =
            Vec::with_capacity(self.environment.lookups.module_constants.len());

        for declaration in &self.environment.lookups.module_constants {
            // `$insert(..)` helper constants only exist so AST template composition can
            // splice them into an immediate parent wrapper. They do not have a stable
            // backend-facing value shape, so HIR must not receive them as module consts.
            // Wrapper constants remain valid here even when their authored source used
            // slot-oriented composition structure, as long as the final constant value
            // classifies as `RenderableString` or `WrapperTemplate`.
            if self.is_helper_only_template_value(&declaration.value)? {
                continue;
            }

            normalized_constants.push(Declaration {
                id: declaration.id.to_owned(),
                value: self
                    .normalize_module_constant_expression(&declaration.value, string_table)?,
            });
        }

        Ok(normalized_constants)
    }

    /// Recursively normalizes a module constant expression.
    ///
    /// WHAT: Folds compile-time templates and recursively processes collections,
    /// struct instances, ranges, and result constructs.
    ///
    /// WHY: Module constants can contain nested structures that must all be
    /// normalized to ensure HIR receives fully folded metadata.
    ///
    /// Returns a new normalized expression (does not mutate the input).
    fn normalize_module_constant_expression(
        &self,
        expression: &Expression,
        string_table: &mut StringTable,
    ) -> Result<Expression, TemplateNormalizationError> {
        increment_ast_counter(AstCounter::ModuleConstantNormalizationExpressionsVisited);

        // Shorthand for the recursive case so each arm reads as a single step.
        let mut normalize_expr =
            |expr: &Expression| -> Result<Expression, TemplateNormalizationError> {
                self.normalize_module_constant_expression(expr, string_table)
            };

        if let ExpressionKind::Template(template) = &expression.kind {
            return normalize_module_constant_template_expression(
                expression,
                template,
                TemplateValueFinalizationInputs {
                    string_table,
                    template_const_loop_iteration_limit: self
                        .context
                        .template_const_loop_iteration_limit,
                    template_ir_store: &self.context.template_ir_store,
                },
            );
        }

        let mut normalized = expression.to_owned();
        normalized.kind = match &expression.kind {
            ExpressionKind::Collection(items) => ExpressionKind::Collection(
                items
                    .iter()
                    .map(normalize_expr)
                    .collect::<Result<Vec<_>, _>>()?,
            ),

            ExpressionKind::StructInstance(fields) => ExpressionKind::StructInstance(
                fields
                    .iter()
                    .map(|field| {
                        Ok(Declaration {
                            id: field.id.to_owned(),
                            value: normalize_expr(&field.value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, TemplateNormalizationError>>()?,
            ),

            ExpressionKind::Range(start, end) => ExpressionKind::Range(
                Box::new(normalize_expr(start)?),
                Box::new(normalize_expr(end)?),
            ),

            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { variant, value } => {
                ExpressionKind::FallibleCarrierConstruct {
                    variant: *variant,
                    value: Box::new(normalize_expr(value)?),
                }
            }

            ExpressionKind::OptionPropagation { value } => ExpressionKind::OptionPropagation {
                value: Box::new(normalize_expr(value)?),
            },

            ExpressionKind::Coerced { value, to_type } => ExpressionKind::Coerced {
                value: Box::new(normalize_expr(value)?),
                to_type: *to_type,
            },

            // Leaf expressions (literals, identifiers, operators, etc.) need no recursion.
            _ => expression.kind.to_owned(),
        };
        Ok(normalized)
    }
}

/// Normalizes one template-valued module constant through the shared fold owner.
///
/// WHAT: converts a folded template to `StringSlice` and rejects valid runtime
///       dependence before it can be mistaken for HIR constant metadata.
/// WHY: HIR's module-constant pool stores only `HirConstValue`; runtime handoffs
///      are executable AST expressions and therefore cannot cross this boundary.
pub(super) fn normalize_module_constant_template_expression(
    expression: &Expression,
    template: &Template,
    fold_inputs: TemplateValueFinalizationInputs<'_, '_>,
) -> Result<Expression, TemplateNormalizationError> {
    let finalization = finalize_template_value(
        template,
        fold_inputs,
        TemplatePreparationMode::ConstRequired,
    )?;
    let mut normalized = expression.to_owned();

    match finalization {
        FinalizedTemplateValue::Folded(folded, fold_provenance) => {
            normalized.diagnostic_type = DataType::StringSlice;
            normalized.kind = ExpressionKind::StringSlice(folded);
            normalized.synthetic_interface_provenance = normalized
                .synthetic_interface_provenance
                .union(&fold_provenance);
        }

        FinalizedTemplateValue::Helper(_) => {}

        FinalizedTemplateValue::Runtime(_) => {
            // Runtime handoffs are valid executable AST values, but HIR module
            // constants are compile-time metadata and accept only HirConstValue.
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::NonFoldableConstTemplate,
                template.location.to_owned(),
            )
            .into());
        }
    }

    Ok(normalized)
}

// --------------------------
//  Helper-only template filter
// --------------------------

impl AstFinalizer<'_, '_> {
    fn is_helper_only_template_value(
        &self,
        expression: &Expression,
    ) -> Result<bool, TemplateNormalizationError> {
        let ExpressionKind::Template(template) = &expression.kind else {
            return Ok(false);
        };

        let store = self.context.template_ir_store.borrow();
        let template_kind = effective_template_kind_from_store(template, &store)?;
        if !matches!(template_kind, TemplateType::SlotInsert(_)) {
            return Ok(false);
        }

        let reference = &template.tir_reference;
        let view = TirView::with_minimum_phase(
            &store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )?;
        let preparation = prepare_tir_view(&view, TemplatePreparationMode::Value)?;

        Ok(matches!(
            preparation.outcome,
            TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert)
        ))
    }
}

/// Reads the authoritative template kind from the module TIR store entry.
///
/// WHAT: resolves the template's TIR reference through the module store and
///       returns `TemplateIr.kind`.
/// WHY: `TemplateIr.kind` is the sole post-construction kind owner.
fn effective_template_kind_from_store(
    template: &Template,
    store: &TemplateIrStore,
) -> Result<TemplateType, TemplateNormalizationError> {
    store
        .get_template(template.tir_reference.root)
        .map(|template_ir| template_ir.kind.clone())
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Constant normalization template kind was not found in the module TIR store.",
            )
            .into()
        })
}
