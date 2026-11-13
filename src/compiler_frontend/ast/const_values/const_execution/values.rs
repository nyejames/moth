//! Constant value helpers shared by const-body preparation and execution.
//!
//! WHAT: folds typed expressions to constant values, projects and updates field/index paths, and
//!       builds the constant replacements and budget/diagnostic errors used by the executor.
//! WHY: required calls, release rewriting and interpreted writes all need the same value-domain
//!      operations; one module keeps projection and error semantics consistent.
//! MUST NOT: execute statements, resolve aliases or decide eligibility.

use super::{
    ProjectedConstant, ResolvedPlaceProjection,
    budget::{ExecutionBudget, MAX_PROJECTION_DEPTH, charge_new_value_text},
    labels,
    state::{ConstInterpreter, FrameBindings},
};
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetExhaustion, ConstBudgetUnit, ConstantFoldError,
    ConstantFoldOutcome, FoldRequirement, FoldServices, fold_expression_with_constants,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValue, ConstValueField, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, SuccessShape,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) fn constant_index(constants: &ConstValueStore, value: ConstValueId) -> Option<usize> {
    match constants.payload(value)? {
        ConstValuePayload::FixedScalar(scalar) => scalar
            .as_u64()
            .or_else(|| scalar.as_i64().and_then(|value| u64::try_from(value).ok()))
            .and_then(|value| usize::try_from(value).ok()),
        ConstValuePayload::Number(_) => None,
        ConstValuePayload::Coerced(inner) => constant_index(constants, *inner),
        _ => None,
    }
}

/// Reports a signed index constant that cannot address a collection.
pub(super) fn fixed_scalar_is_negative(constants: &ConstValueStore, value: ConstValueId) -> bool {
    match constants.payload(value) {
        Some(ConstValuePayload::FixedScalar(scalar)) => {
            scalar.as_i64().is_some_and(|integer| integer.is_negative())
        }
        Some(ConstValuePayload::Coerced(inner)) => fixed_scalar_is_negative(constants, *inner),
        _ => false,
    }
}

/// Reads a value through resolved field/index projections.
pub(super) fn project_constant_value(
    constants: &ConstValueStore,
    value: ConstValueId,
    projections: &[ResolvedPlaceProjection],
    path_fork: &PathInternerFork,
) -> Option<ConstValueId> {
    let mut current = value;
    for projection in projections {
        current = match projection {
            ResolvedPlaceProjection::Field(field) => {
                constants.field_value(current, *field, path_fork)?
            }
            ResolvedPlaceProjection::Index(index) => match constants.payload(current)? {
                ConstValuePayload::Collection(items) => *items.get(*index)?,
                ConstValuePayload::Coerced(inner) => project_constant_value(
                    constants,
                    *inner,
                    std::slice::from_ref(projection),
                    path_fork,
                )?,
                _ => return None,
            },
        };
    }
    Some(current)
}

/// The owners one projected write reads, charges and records through.
///
/// WHY: the write helper needs the constant graph, the path component reader, the attempt budget
///      and the string table at every nesting level. One borrowed context keeps the recursive
///      signature readable and states that these four are the operation's whole environment.
pub(super) struct ProjectionWrite<'a> {
    pub(super) constants: &'a mut ConstValueStore,
    pub(super) path_fork: &'a PathInternerFork,
    pub(super) budget: &'a mut ExecutionBudget,
    pub(super) string_table: &'a mut StringTable,
}

/// Writes through resolved field/index projections by producing an updated immutable constant.
pub(super) fn assign_constant_projections(
    value: ConstValueId,
    projections: &[ResolvedPlaceProjection],
    replacement: ConstValueId,
    write: &mut ProjectionWrite<'_>,
) -> Result<ConstValueId, ConstantFoldError> {
    assign_constant_projections_at(value, projections, replacement, write, 0)
}

fn assign_constant_projections_at(
    value: ConstValueId,
    projections: &[ResolvedPlaceProjection],
    replacement: ConstValueId,
    write: &mut ProjectionWrite<'_>,
    depth: usize,
) -> Result<ConstValueId, ConstantFoldError> {
    if depth > MAX_PROJECTION_DEPTH {
        return Err(hard_limit_error(
            write.constants.metadata(value).and_then(|m| m.span),
            ConstBudgetCategory::Work,
            ConstBudgetUnit::ProjectionLevels,
            MAX_PROJECTION_DEPTH,
        ));
    }
    let Some((projection, rest)) = projections.split_first() else {
        return Ok(replacement);
    };
    let metadata = write
        .constants
        .metadata(value)
        .cloned()
        .ok_or_else(|| unsupported_error(labels::PROJECTED_WRITE, None, write.string_table))?;
    let borrowed_payload = write.constants.payload(value).ok_or_else(|| {
        unsupported_error(labels::PROJECTED_WRITE, metadata.span, write.string_table)
    })?;
    let slots = match borrowed_payload {
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            fields.len().saturating_add(1)
        }
        ConstValuePayload::Collection(items) => items.len().saturating_add(1),
        _ => 1,
    };
    let owned_slots = match borrowed_payload {
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => fields
            .len()
            .saturating_mul(std::mem::size_of::<ConstValueField>()),
        ConstValuePayload::Collection(items) => items
            .len()
            .saturating_mul(std::mem::size_of::<ConstValueId>()),
        _ => 0,
    };
    if !matches!(borrowed_payload, ConstValuePayload::Coerced(_)) {
        write.budget.retain_memory_bytes(
            std::mem::size_of::<ConstValue>().saturating_add(owned_slots),
            metadata.span,
        )?;
        write.budget.retain_values(slots, metadata.span)?;
    }
    let payload = borrowed_payload.clone();
    let payload = match (projection, payload) {
        (ResolvedPlaceProjection::Field(field), ConstValuePayload::Record(mut fields)) => {
            let entry = fields
                .iter_mut()
                .find(|entry| write.path_fork.component(entry.name) == Some(*field))
                .ok_or_else(|| {
                    unsupported_error(labels::PROJECTED_WRITE, metadata.span, write.string_table)
                })?;
            entry.value =
                assign_constant_projections_at(entry.value, rest, replacement, write, depth + 1)?;
            ConstValuePayload::Record(fields)
        }
        (
            ResolvedPlaceProjection::Field(field),
            ConstValuePayload::Choice {
                nominal_path,
                tag,
                mut fields,
            },
        ) => {
            let entry = fields
                .iter_mut()
                .find(|entry| write.path_fork.component(entry.name) == Some(*field))
                .ok_or_else(|| {
                    unsupported_error(labels::PROJECTED_WRITE, metadata.span, write.string_table)
                })?;
            entry.value =
                assign_constant_projections_at(entry.value, rest, replacement, write, depth + 1)?;
            ConstValuePayload::Choice {
                nominal_path,
                tag,
                fields,
            }
        }
        (ResolvedPlaceProjection::Index(index), ConstValuePayload::Collection(mut items)) => {
            let item = items.get_mut(*index).ok_or_else(|| {
                unsupported_error(labels::PROJECTED_WRITE, metadata.span, write.string_table)
            })?;
            *item = assign_constant_projections_at(*item, rest, replacement, write, depth + 1)?;
            ConstValuePayload::Collection(items)
        }
        (ResolvedPlaceProjection::Field(_), ConstValuePayload::Coerced(inner)) => {
            let updated =
                assign_constant_projections_at(inner, projections, replacement, write, depth + 1)?;
            return Ok(updated);
        }
        (ResolvedPlaceProjection::Index(_), ConstValuePayload::Coerced(inner)) => {
            let updated =
                assign_constant_projections_at(inner, projections, replacement, write, depth + 1)?;
            return Ok(updated);
        }
        _ => {
            return Err(unsupported_error(
                labels::PROJECTED_WRITE,
                metadata.span,
                write.string_table,
            ));
        }
    };
    let precharged_index = write.constants.value_count();
    let inserted = write
        .constants
        .insert_value(metadata, payload)
        .map_err(ConstantFoldError::from)?;
    write
        .budget
        .precharged_value_indices
        .insert(precharged_index);
    Ok(inserted)
}

impl ConstInterpreter<'_> {
    /// Projects a field from a constant record/choice base exposed by an earlier call fold.
    ///
    /// WHY: a required call can complete beneath a field access after expression parsing has
    /// already resolved that projection. The shared folder only projects explicit constants, so
    /// the executor completes this one remaining constant operation without a second expression
    /// vocabulary.
    pub(super) fn constant_field_projection(
        &self,
        expression: ExprId,
        bindings: &FrameBindings,
    ) -> Result<Option<ProjectedConstant>, ConstantFoldError> {
        let view = self.builder.view(expression)?;
        let ExpressionOperationView::FieldAccess { base, field } = view.operation() else {
            return Ok(None);
        };
        let base_value = match base.operation() {
            ExpressionOperationView::Constant(value) => value,
            ExpressionOperationView::Reference(path) => {
                let Some(value) = bindings.get(&path) else {
                    return Ok(None);
                };
                *value
            }
            _ => return Ok(None),
        };
        let Some(value) = self
            .constants
            .field_value(base_value, field, self.path_fork)
        else {
            return Ok(None);
        };
        let SuccessShape::One(result_type) = view.success() else {
            return Ok(None);
        };
        Ok(Some(ProjectedConstant {
            value,
            type_id: result_type,
            span: view.span(),
            value_mode: view.value_mode(),
        }))
    }

    /// Fold one completed typed expression, charging the attempt for the fold attempt.
    pub(super) fn fold_to_constant(
        &mut self,
        expression: ExprId,
        bindings: &FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        self.budget.step()?;
        let value_count = self.constants.value_count();
        let outcome = fold_expression_with_constants(
            expression,
            self.builder,
            self.constants,
            FoldServices::new(
                self.string_table,
                Some(self.scope),
                Some(self.scope.external_package_registry.as_ref()),
            ),
            self.types,
            FoldRequirement::Required,
            |path| bindings.get(&path).copied(),
        )?;
        let value: Option<ConstValueId> = match outcome {
            ConstantFoldOutcome::Folded(root) | ConstantFoldOutcome::NotConstant(root) => {
                match self.builder.view(root)?.operation() {
                    ExpressionOperationView::Constant(value) => Some(value),
                    _ => None,
                }
            }
            ConstantFoldOutcome::Refused { reason, .. } => {
                return Err(ConstantFoldError::Diagnostic(reason.into_draft()));
            }
        };
        if value.is_some() {
            charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        }
        Ok(value)
    }
}

pub(super) fn constant_operation(
    builder: &AstExpressionBuilder,
    expression: ExprId,
    string_table: &mut StringTable,
) -> Result<ConstValueId, ConstantFoldError> {
    let view = builder.view(expression)?;
    match view.operation() {
        ExpressionOperationView::Constant(value) => Ok(value),
        _ => Err(unsupported_error(
            labels::LOOP_BINDING,
            view.span(),
            string_table,
        )),
    }
}

pub(super) fn template_error(error: TemplateError) -> ConstantFoldError {
    match error {
        TemplateError::Diagnostic(diagnostic)
        | TemplateError::NonConstantReferenceInConstant(diagnostic) => {
            ConstantFoldError::Diagnostic(diagnostic)
        }
        TemplateError::Budget(exhaustion) => ConstantFoldError::Budget(exhaustion),
        TemplateError::Infrastructure(error) => ConstantFoldError::Infrastructure(error),
    }
}

pub(super) fn unsupported_error(
    operation: &'static str,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> ConstantFoldError {
    ConstantFoldError::Diagnostic(choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstFunctionExecutionUnsupported,
        Some(string_table.intern(operation)),
        span,
    ))
}

pub(super) fn limit_error(
    span: Option<SourceSpan>,
    category: ConstBudgetCategory,
    unit: ConstBudgetUnit,
    limit: u64,
    setting: Option<&'static str>,
    aggregate: bool,
) -> ConstantFoldError {
    ConstantFoldError::Budget(ConstBudgetExhaustion {
        category,
        unit,
        limit,
        setting,
        aggregate,
        span,
        call_span: None,
        call_chain: Vec::new(),
    })
}

pub(super) fn hard_limit_error(
    span: Option<SourceSpan>,
    category: ConstBudgetCategory,
    unit: ConstBudgetUnit,
    limit: usize,
) -> ConstantFoldError {
    limit_error(span, category, unit, limit as u64, None, false)
}

pub(super) fn expression_with_value(
    builder: &mut AstExpressionBuilder,
    value: ConstValueId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> Result<ExprId, ConstantFoldError> {
    Ok(builder.constant(value, type_id, span, value_mode)?.id())
}

/// Replace a call with a constant while retaining both authored and evaluated synthetic provenance.
///
/// WHY: HIR link facts accumulate the provenance of the expressions that survive into the module.
///      A folded helper may depend on project-context values that were only reachable through the
///      erased call. Dropping that provenance would let a facade appear not to expose them.
pub(super) fn expression_with_value_inheriting_provenance(
    builder: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
    value: ConstValueId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    inherited: SyntheticInterfaceProvenance,
) -> Result<ExprId, ConstantFoldError> {
    let evaluated = constants
        .metadata(value)
        .map(|metadata| metadata.synthetic_interface_provenance.clone())
        .unwrap_or_default();
    let replacement = builder.constant(value, type_id, span, value_mode)?.id();
    let replacement = builder.with_synthetic_interface_provenance(
        builder.own(replacement),
        inherited.union(&evaluated),
    )?;
    Ok(replacement.id())
}
