//! Compile-time template folding over immutable, owner-bound semantic expressions.
//!
//! Exact prepared TIR views retain structural authority. Bindings share committed IDs;
//! the ordinary expression folder owns isolated replacement construction and fold refusal.

use crate::compiler_frontend::ast::const_eval::{BindingFold, fold_expression_with_bindings};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateFoldBinding;
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, TemplateIrStore, TemplatePreparationMode, TemplateTirPhase, TirView,
    prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

pub(crate) struct TirFoldContext<'a> {
    pub string_table: &'a mut StringTable,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub template_const_loop_iteration_limit: usize,
    pub(crate) bindings: Vec<TemplateFoldBinding>,
    pub(crate) source_scope: Option<&'a ScopeContext>,
}

/// Structural no-output is distinct from an emitted empty string for parent wrappers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TemplateEmission {
    NoOutput,
    Output(ConstStringValue),
}

/// Provenance follows only the exact selected emission path, never an ambient accumulator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateFoldResult {
    pub(crate) emission: TemplateEmission,
    pub(crate) provenance: SyntheticInterfaceProvenance,
    pub(crate) projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
}

impl TemplateFoldResult {
    pub(crate) fn with_projection(
        emission: TemplateEmission,
        provenance: SyntheticInterfaceProvenance,
        projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
    ) -> Self {
        Self {
            emission,
            provenance,
            projection_pieces,
        }
    }
}

impl<'a> TirFoldContext<'a> {
    /// Creates a fold context with no active bindings or source scope.
    pub(crate) fn new(
        string_table: &'a mut StringTable,
        expression_builder: &'a mut AstExpressionBuilder,
        const_values: &'a mut ConstValueStore,
        template_const_loop_iteration_limit: usize,
    ) -> Self {
        Self {
            string_table,
            expression_builder,
            const_values,
            template_const_loop_iteration_limit,
            bindings: Vec::new(),
            source_scope: None,
        }
    }
}

impl TirFoldContext<'_> {
    pub(crate) fn push_bindings(
        &mut self,
        bindings: impl IntoIterator<Item = TemplateFoldBinding>,
    ) -> usize {
        let previous_len = self.bindings.len();
        self.bindings.extend(bindings);
        previous_len
    }

    pub(crate) fn restore_bindings(&mut self, previous_len: usize) {
        self.bindings.truncate(previous_len);
    }
}

pub(crate) fn resolve_fold_bindings_in_expression(
    expression: ExprId,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<ExprId, TemplateError> {
    let original = fold_context.expression_builder.view(expression)?;
    if let ExpressionOperationView::Reference(path) = original.operation()
        && let Some(binding) = fold_context
            .bindings
            .iter()
            .rev()
            .find(|binding| binding.path == path)
    {
        // A binding is already a completed value. Preserve optional wrappers and nested
        // template identity rather than evaluating or reconstructing the bound root.
        fold_context.expression_builder.view(binding.value)?;
        return Ok(binding.value);
    }
    let bindings = &fold_context.bindings;
    let resolved = if bindings.is_empty() {
        expression
    } else {
        fold_expression_with_bindings(
            expression,
            fold_context.expression_builder,
            fold_context.const_values,
            fold_context.string_table,
            BindingFold::speculative(fold_context.source_scope),
            |path| {
                bindings
                    .iter()
                    .rev()
                    .find(|binding| binding.path == path)
                    .map(|binding| binding.value)
            },
        )?
    };
    // Rendering a coerced template enters that nested value's own exact context.
    let view = fold_context.expression_builder.view(resolved)?;
    if resolved == expression
        && let ExpressionOperationView::Coerce { value, .. } = view.operation()
        && matches!(value.operation(), ExpressionOperationView::Template(_))
    {
        return Ok(value.id());
    }
    Ok(resolved)
}

pub(crate) fn selected_option_capture_payload_with_provenance(
    scrutinee: ExprId,
    pattern: &MatchPattern,
    store: &TemplateIrStore,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<(Option<TemplateFoldBinding>, SyntheticInterfaceProvenance), TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(scrutinee, fold_context)?;
    let view = fold_context.expression_builder.view(resolved)?;
    let provenance = expression_provenance(view);
    let payload = match view.operation() {
        ExpressionOperationView::Constant(id) => match fold_context.const_values.payload(id) {
            Some(ConstValuePayload::OptionNone) => return Ok((None, provenance)),
            Some(ConstValuePayload::OptionSome(value) | ConstValuePayload::Coerced(value)) => {
                let metadata = fold_context.const_values.metadata(*value).ok_or_else(|| {
                    CompilerError::compiler_error("Missing option capture constant.")
                })?;
                let value = *value;
                let type_id = metadata.type_id;
                let span = view.span();
                let mode = view.value_mode();
                fold_context
                    .expression_builder
                    .constant(value, type_id, span, mode)?
                    .id()
            }
            _ => return Err(option_capture_const_deferred_error(view).into()),
        },
        ExpressionOperationView::Coerce { value, .. } => {
            let constant = value
                .const_value_kind_with_template_classifier(
                    fold_context.const_values,
                    &mut |template| {
                        let reference = template.tir_reference;
                        let view = TirView::with_minimum_phase(
                            store,
                            reference.root,
                            reference.phase,
                            TemplateTirPhase::Composed,
                            reference.context,
                        )?;
                        Ok(prepare_tir_view(
                            &view,
                            fold_context.expression_builder,
                            fold_context.const_values,
                            TemplatePreparationMode::Value,
                        )?
                        .facts
                        .final_value_kind)
                    },
                )?
                .is_compile_time_value();
            if !constant {
                return Err(option_capture_const_deferred_error(view).into());
            }
            value.id()
        }
        _ => return Err(option_capture_const_deferred_error(view).into()),
    };
    let MatchPattern::OptionPresentCapture { binding_path, .. } = pattern else {
        return Err(CompilerError::compiler_error(
            "Template option-capture folding received a non-capture pattern.",
        )
        .into());
    };
    let provenance = provenance.union(&expression_provenance(
        fold_context.expression_builder.view(payload)?,
    ));
    Ok((
        Some(TemplateFoldBinding {
            path: *binding_path,
            value: payload,
        }),
        provenance,
    ))
}

fn expression_provenance(expression: ExpressionView<'_>) -> SyntheticInterfaceProvenance {
    expression
        .occurrence_facts()
        .map_or_else(SyntheticInterfaceProvenance::empty, |facts| {
            facts.provenance.clone()
        })
}

fn option_capture_const_deferred_error(expression: ExpressionView<'_>) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
        expression.span(),
    )
}

pub(crate) fn fold_conditional_loop_const_condition(
    condition: ExprId,
    fallback_span: Option<SourceSpan>,
    fold_context: &TirFoldContext<'_>,
) -> Result<bool, TemplateError> {
    fold_resolved_bool_condition(
        fold_context.expression_builder.view(condition)?,
        fallback_span,
        fold_context.const_values,
        InvalidTemplateStructureReason::TemplateLoopConditionNotConst,
    )
}

pub(crate) fn fold_bool_condition_with_provenance(
    condition: ExprId,
    fallback_span: Option<SourceSpan>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<(bool, SyntheticInterfaceProvenance), TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(condition, fold_context)?;
    let view = fold_context.expression_builder.view(resolved)?;
    let value = fold_resolved_bool_condition(
        view,
        fallback_span,
        fold_context.const_values,
        InvalidTemplateStructureReason::TemplateIfConditionNotConst,
    )?;
    Ok((value, expression_provenance(view)))
}

fn fold_resolved_bool_condition(
    condition: ExpressionView<'_>,
    fallback_span: Option<SourceSpan>,
    constants: &ConstValueStore,
    reason: InvalidTemplateStructureReason,
) -> Result<bool, TemplateError> {
    match condition.operation() {
        ExpressionOperationView::Constant(id) => {
            let mut payload = constants.payload(id);
            while let Some(ConstValuePayload::Coerced(value)) = payload {
                payload = constants.payload(*value);
            }
            if let Some(ConstValuePayload::Bool(value)) = payload {
                return Ok(*value);
            }
        }
        ExpressionOperationView::Coerce { value, .. } => {
            return fold_resolved_bool_condition(value, fallback_span, constants, reason);
        }
        _ => {}
    }
    Err(
        CompilerDiagnostic::invalid_template_structure(reason, condition.span().or(fallback_span))
            .into(),
    )
}
