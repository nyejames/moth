//! Typed value settlement, copying and publication for required const evaluation.
//!
//! The existing folder owns scalar and constructor semantics. Live aggregate members remain
//! in `storage`; copying and publication traverse them instead of reading stale shape payloads.
//! This module also owns the immutable projection and replacement helpers used at AST boundaries.

use super::{
    ProjectedConstant, ResolvedPlaceProjection,
    budget::{
        ExecutionBudget, MAX_PROJECTION_DEPTH, charge_new_value_text, register_precharged_value,
        value_own_allocation,
    },
    state::{ConstInterpreter, FrameBindings},
    storage::{
        AccessSite, AllocationId, LiveBinding, LivePlace, LiveStorage, LiveValue, Permission, StorageError,
    },
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetExhaustion, ConstBudgetUnit, ConstantFoldError,
    ConstantFoldRefusalReason, ConstantValueFold, FoldRequirement, FoldServices,
    fold_completed_content, fold_value_with_constants,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValue, ConstValueCandidate, ConstValueField, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CoercionOperation, ExprId, ExpressionOperationView, SuccessShape,
};
use crate::compiler_frontend::ast::statements::value_production::expression_build::commit_constant_result_block;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, types_borrows_targets,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::{DiagnosticPlace, InvalidMutableAccessReason};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};

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

fn value_invariant() -> ConstantFoldError {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "const execution encountered an invalid live value",
    )
    .into()
}

pub(super) fn retain_scratch<T>(
    budget: &mut super::budget::ExecutionBudget,
    count: usize,
) -> Result<(), ConstantFoldError> {
    budget.retain_values(count, budget.span)?;
    budget.retain_memory_bytes(count.saturating_mul(std::mem::size_of::<T>()), budget.span)
}

fn check_depth(depth: usize, span: Option<SourceSpan>) -> Result<(), ConstantFoldError> {
    if depth > MAX_PROJECTION_DEPTH {
        return Err(hard_limit_error(
            span,
            ConstBudgetCategory::Work,
            ConstBudgetUnit::ProjectionLevels,
            MAX_PROJECTION_DEPTH,
        ));
    }
    Ok(())
}

fn child_count(payload: &ConstValuePayload) -> usize {
    match payload {
        ConstValuePayload::Collection(items) => items.len(),
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            fields.len()
        }
        ConstValuePayload::Coerced(_) | ConstValuePayload::OptionSome(_) => 1,
        _ => 0,
    }
}

fn child_at(payload: &ConstValuePayload, index: usize) -> Option<ConstValueId> {
    match payload {
        ConstValuePayload::Collection(items) => items.get(index).copied(),
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            fields.get(index).map(|field| field.value)
        }
        ConstValuePayload::Coerced(inner) | ConstValuePayload::OptionSome(inner) if index == 0 => {
            Some(*inner)
        }
        _ => None,
    }
}

fn replace_children(
    payload: &mut ConstValuePayload,
    children: &[ConstValueId],
) -> Result<(), ConstantFoldError> {
    match payload {
        ConstValuePayload::Collection(items) => {
            if items.len() != children.len() {
                return Err(value_invariant());
            }
            items.copy_from_slice(children);
        }
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => {
            if fields.len() != children.len() {
                return Err(value_invariant());
            }
            for (field, &child) in fields.iter_mut().zip(children) {
                field.value = child;
            }
        }
        ConstValuePayload::Coerced(inner) | ConstValuePayload::OptionSome(inner) => {
            if children.len() != 1 {
                return Err(value_invariant());
            }
            *inner = children[0];
        }
        _ if !children.is_empty() => return Err(value_invariant()),
        _ => {}
    }
    Ok(())
}

/// Allocation identities are local to one ordered publication transaction.
#[derive(Default)]
struct FreezeMemo {
    allocations: FxHashMap<AllocationId, (ConstValueId, ConstValueId)>,
    occurrences: FxHashMap<(AllocationId, ConstValueId), ConstValueId>,
}

fn freeze_staged(
    candidate: &mut ConstValueCandidate<'_>,
    storage: &LiveStorage,
    budget: &mut ExecutionBudget,
    value: LiveValue,
    depth: usize,
    memo: &mut FreezeMemo,
) -> Result<ConstValueId, ConstantFoldError> {
    budget.step()?;
    check_depth(depth, budget.span)?;
    let Some(allocation) = value.allocation else {
        return Ok(value.shape);
    };
    if let Some(&(definition, shape)) = memo.allocations.get(&allocation) {
        if shape == value.shape {
            return Ok(definition);
        }
        if let Some(&id) = memo.occurrences.get(&(allocation, value.shape)) {
            return Ok(id);
        }
        let definition_payload = candidate.payload(definition).ok_or_else(value_invariant)?;
        let occurrence_payload = candidate.payload(value.shape).ok_or_else(value_invariant)?;
        if std::mem::discriminant(definition_payload) != std::mem::discriminant(occurrence_payload) {
            return Err(value_invariant());
        }
        retain_scratch::<ConstValue>(budget, 1)?;
        let metadata = candidate.metadata(value.shape).ok_or_else(value_invariant)?;
        budget.retain_memory_bytes(
            metadata.synthetic_interface_provenance.owned_allocation_bytes(),
            budget.span,
        )?;
        let metadata = metadata.clone();
        let id = candidate.insert_shared_occurrence(metadata, definition)?;
        retain_scratch::<((AllocationId, ConstValueId), ConstValueId)>(budget, 1)?;
        memo.occurrences.insert((allocation, value.shape), id);
        return Ok(id);
    }
    let payload = candidate.payload(value.shape).ok_or_else(value_invariant)?;
    let count = storage.children(allocation).len();
    if !matches!(payload, ConstValuePayload::Collection(_)) && count != child_count(payload) {
        return Err(value_invariant());
    }
    retain_scratch::<ConstValueId>(budget, count)?;
    let mut children = Vec::with_capacity(count);
    for child in storage.children(allocation) {
        children.push(freeze_staged(candidate, storage, budget, child, depth + 1, memo)?);
    }
    let payload = candidate.payload(value.shape).ok_or_else(value_invariant)?;
    let bytes = match payload {
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } =>
            fields.len().saturating_mul(std::mem::size_of::<ConstValueField>()),
        _ => 0,
    };
    retain_scratch::<ConstValue>(budget, 1)?;
    if !matches!(payload, ConstValuePayload::Collection(_)) {
        budget.retain_values(count, budget.span)?;
    }
    budget.retain_memory_bytes(bytes, budget.span)?;
    let payload = if matches!(payload, ConstValuePayload::Collection(_)) {
        ConstValuePayload::Collection(children)
    } else {
        let mut payload = payload.clone();
        replace_children(&mut payload, &children)?;
        payload
    };
    let metadata = candidate.metadata(value.shape).ok_or_else(value_invariant)?;
    budget.retain_memory_bytes(
        metadata.synthetic_interface_provenance.owned_allocation_bytes(),
        budget.span,
    )?;
    let metadata = metadata.clone();
    let definition = candidate.insert_value(metadata, payload)?;
    candidate.mark_shared_allocation(definition)?;
    retain_scratch::<(AllocationId, (ConstValueId, ConstValueId))>(budget, 1)?;
    memo.allocations.insert(allocation, (definition, value.shape));
    Ok(definition)
}

fn expanded_child_count(payload: &ConstValuePayload) -> usize {
    if matches!(payload, ConstValuePayload::Range { .. }) { 2 } else { child_count(payload) }
}

fn expanded_child_at(payload: &ConstValuePayload, index: usize) -> Option<ConstValueId> {
    match (payload, index) {
        (ConstValuePayload::Range { start, .. }, 0) => Some(*start),
        (ConstValuePayload::Range { end, .. }, 1) => Some(*end),
        _ => child_at(payload, index),
    }
}

/// Count the tree each postorder consumer will visit, without expanding repeated edges.
fn expanded_occurrences(
    candidate: &ConstValueCandidate<'_>,
    roots: &[ConstValueId],
    budget: &mut ExecutionBudget,
) -> Result<usize, ConstantFoldError> {
    let mut counts = FxHashMap::<ConstValueId, usize>::default();
    let mut pending = Vec::new();
    let mut held = 0;
    let result = (|| {
        for &root in roots {
            budget.reserve_scratch(&mut pending, 1, &mut held)?;
            pending.push((root, false));
            while let Some((id, finish)) = pending.pop() {
                if counts.contains_key(&id) {
                    continue;
                }
                let payload = candidate.payload(id).ok_or_else(value_invariant)?;
                if finish {
                    let mut count = 1usize;
                    for index in 0..expanded_child_count(payload) {
                        let child = expanded_child_at(payload, index).ok_or_else(value_invariant)?;
                        count = count.saturating_add(*counts.get(&child).ok_or_else(value_invariant)?);
                    }
                    budget.charge_scratch(&mut held, std::mem::size_of::<(ConstValueId, usize)>())?;
                    counts.insert(id, count);
                } else {
                    let count = expanded_child_count(payload);
                    budget.reserve_scratch(&mut pending, count.saturating_add(1), &mut held)?;
                    pending.push((id, true));
                    for index in 0..count {
                        pending.push((expanded_child_at(payload, index).ok_or_else(value_invariant)?, false));
                    }
                }
            }
        }
        Ok(roots.iter().fold(0usize, |sum, root| sum.saturating_add(counts[root])))
    })();
    budget.release_memory_bytes(held);
    result
}

impl ConstInterpreter<'_> {
    pub(super) fn bind_slot(
        &mut self,
        bindings: &mut FrameBindings,
        path: PathId,
        value: LiveValue,
        permission: Permission,
    ) -> Result<(), ConstantFoldError> {
        self.validate_live_value(value, self.budget.span)?;
        self.install_binding(bindings, path, value, permission)
    }

    /// Install a value whose access was checked before the receiver started.
    pub(super) fn install_binding(
        &mut self,
        bindings: &mut FrameBindings,
        path: PathId,
        value: LiveValue,
        permission: Permission,
    ) -> Result<(), ConstantFoldError> {
        if !bindings.contains_key(&path) {
            retain_scratch::<(PathId, LiveBinding)>(self.budget, 1)?;
        }
        let site = self.access_site(self.budget.span);
        let binding = self
            .state
            .storage
            .bind_slot(value, permission, site, self.budget)?;
        bindings.insert(path, binding);
        Ok(())
    }

    pub(super) fn read_place(
        &mut self,
        place: LivePlace,
        span: Option<SourceSpan>,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.budget.span = span;
        let site = self.access_site(span);
        self.state
            .storage
            .read(place, site, self.budget)
            .map_err(|error| self.storage_error(error, span))
    }

    pub(super) fn validate_live_value(
        &mut self,
        value: LiveValue,
        span: Option<SourceSpan>,
    ) -> Result<(), ConstantFoldError> {
        self.budget.span = span;
        self.state
            .storage
            .validate_value(value, self.budget)
            .map_err(|error| self.storage_error(error, span))
    }

    pub(super) fn acquire_shared_value(
        &mut self,
        value: LiveValue,
        span: Option<SourceSpan>,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.validate_live_value(value, span)?;
        let site = self.access_site(span);
        self.state.storage.share(value, site, self.budget)
    }

    pub(super) fn replace_live_place(
        &mut self,
        place: LivePlace,
        value: LiveValue,
        span: Option<SourceSpan>,
    ) -> Result<bool, ConstantFoldError> {
        self.budget.span = span;
        let site = self.access_site(span);
        self.state
            .storage
            .replace(place, value, site, self.budget)
            .map_err(|error| self.storage_error(error, span))
    }

    pub(super) fn access_site(&self, span: Option<SourceSpan>) -> AccessSite {
        AccessSite {
            span,
            body: self.state.current_body,
        }
    }

    pub(super) fn storage_error(
        &self,
        error: StorageError,
        use_span: Option<SourceSpan>,
    ) -> ConstantFoldError {
        let access = match error {
            StorageError::InvalidAccess(access) => access,
            StorageError::Execution(error) => return error,
        };
        let mut draft = types_borrows_targets::invalid_const_access(access.reason, use_span);
        // Fix each source domain independently. Donor unwind must not change
        // a caller-local acquisition into a span owned by the callee.
        draft.replace_primary_frozen_identity_handle(
            self.body_donor_identity(self.state.current_body),
        );
        for (site, label) in [
            (access.acquired, LabelDraftMessage::ConstReferenceAcquired),
            (
                access.invalidated,
                LabelDraftMessage::ConstInvalidatingWrite,
            ),
        ] {
            if let Some(site) = site {
                let index = draft.label_count();
                draft = draft.with_label(site.span, Some(label));
                if let Some(owner) = self.body_donor_identity(site.body) {
                    draft = draft.with_label_frozen_identity_handle(index, owner);
                }
            }
        }
        ConstantFoldError::Diagnostic(draft)
    }

    pub(super) fn mutable_access_error(
        &self,
        root: PathId,
        span: Option<SourceSpan>,
    ) -> ConstantFoldError {
        ConstantFoldError::Diagnostic(types_borrows_targets::invalid_mutable_access(
            DiagnosticPlace::Path(root),
            InvalidMutableAccessReason::ImmutablePlace,
            None,
            None,
            span,
        ))
    }

    pub(super) fn materialize_value(
        &mut self,
        shape: ConstValueId,
        permission: Permission,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.materialize_at(shape, permission, 0)
    }

    fn materialize_at(
        &mut self,
        shape: ConstValueId,
        permission: Permission,
        depth: usize,
    ) -> Result<LiveValue, ConstantFoldError> {
        let payload = self.constants.payload(shape).ok_or_else(value_invariant)?;
        let count = child_count(payload);
        let compound = matches!(
            payload,
            ConstValuePayload::Collection(_)
                | ConstValuePayload::Record(_)
                | ConstValuePayload::Choice { .. }
                | ConstValuePayload::Coerced(_)
                | ConstValuePayload::OptionSome(_)
        );
        if !compound {
            return Ok(LiveValue::leaf(shape, permission));
        }
        self.budget.step()?;
        check_depth(depth, self.budget.span)?;
        retain_scratch::<LiveValue>(self.budget, count)?;
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let child = child_at(
                self.constants.payload(shape).ok_or_else(value_invariant)?,
                index,
            )
            .ok_or_else(value_invariant)?;
            children.push(self.materialize_at(child, permission, depth + 1)?);
        }
        let mut result = self
            .state
            .storage
            .allocate(shape, children.into_iter(), self.budget)?;
        result.permission = permission;
        Ok(result)
    }

    pub(super) fn copy_live_value(
        &mut self,
        value: LiveValue,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.copy_at(value, 0, &mut FxHashMap::default())
    }

    fn copy_at(
        &mut self,
        value: LiveValue,
        depth: usize,
        copies: &mut FxHashMap<AllocationId, LiveValue>,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.budget.step()?;
        check_depth(depth, self.budget.span)?;
        self.validate_live_value(value, self.budget.span)?;
        let Some(allocation) = value.allocation else {
            return Ok(LiveValue::leaf(value.shape, Permission::Writable));
        };
        if let Some(&copy) = copies.get(&allocation) {
            let mut copy = copy;
            copy.shape = value.shape;
            return Ok(copy);
        }
        let count = self.state.storage.children(allocation).len();
        retain_scratch::<LiveValue>(self.budget, count)?;
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let child = self
                .state
                .storage
                .children(allocation)
                .nth(index)
                .ok_or_else(value_invariant)?;
            children.push(self.copy_at(child, depth + 1, copies)?);
        }
        let copy = self
            .state
            .storage
            .allocate(value.shape, children.into_iter(), self.budget)?;
        retain_scratch::<(AllocationId, LiveValue)>(self.budget, 1)?;
        copies.insert(allocation, copy);
        Ok(copy)
    }

    pub(super) fn freeze_live_value(
        &mut self,
        value: LiveValue,
    ) -> Result<ConstValueId, ConstantFoldError> {
        let mut roots = self.freeze_live_values(&[value])?;
        roots.pop().ok_or_else(value_invariant)
    }

    /// Validate all slots, then stage and publish their complete graph as one transaction.
    pub(super) fn freeze_live_values(
        &mut self,
        values: &[LiveValue],
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let mut held = 0;
        let validation = (|| {
            let mut pending = Vec::new();
            self.budget.reserve_scratch(&mut pending, values.len(), &mut held)?;
            pending.extend_from_slice(values);
            let mut seen = FxHashSet::default();
            while let Some(value) = pending.pop() {
                self.validate_live_value(value, self.budget.span)?;
                if let Some(allocation) = value.allocation
                    && !seen.contains(&allocation)
                {
                    self.budget.charge_scratch(&mut held, std::mem::size_of::<AllocationId>())?;
                    seen.insert(allocation);
                    let children = self.state.storage.children(allocation);
                    self.budget.reserve_scratch(&mut pending, children.len(), &mut held)?;
                    pending.extend(children);
                }
            }
            Ok::<_, ConstantFoldError>(())
        })();
        self.budget.release_memory_bytes(held);
        validation?;

        // Bring any earlier rows into the accounting range before staging. Extending this
        // contiguous range over the precharged staged rows is then infallible.
        let start = self.constants.value_count();
        charge_new_value_text(self.constants, self.string_table, start, self.budget)?;
        let storage = &self.state.storage;
        let budget = &mut *self.budget;
        let mut memo = FreezeMemo::default();
        let roots = self.constants.stage_values(|candidate| {
            let mut roots = Vec::with_capacity(values.len());
            for &value in values {
                roots.push(freeze_staged(candidate, storage, budget, value, 0, &mut memo)?);
            }
            let occurrences = expanded_occurrences(candidate, &roots, budget)?;
            budget.retain_values(occurrences, budget.span)?;
            Ok::<_, ConstantFoldError>(Some(roots))
        })?.ok_or_else(value_invariant)?;
        for index in start..self.constants.value_count() {
            register_precharged_value(self.constants, self.string_table, index, self.budget)?;
        }
        Ok(roots)
    }

    pub(super) fn fold_live_expression(
        &mut self,
        expression: ExprId,
        children: &[LiveValue],
        dec_depth: u32,
        bindings: &mut FrameBindings,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        let mut inline = [None; 2];
        if children.len() <= inline.len() {
            return self.fold_live_operands(
                expression,
                children,
                &mut inline[..children.len()],
                dec_depth,
                bindings,
            );
        }
        let mut bytes = 0;
        let result = (|| {
            self.budget.charge_scratch(
                &mut bytes,
                std::mem::size_of::<Option<ConstValueId>>() * children.len(),
            )?;
            let mut operands = vec![None; children.len()];
            self.fold_live_operands(expression, children, &mut operands, dec_depth, bindings)
        })();
        self.budget.release_memory_bytes(bytes);
        result
    }

    fn fold_live_operands(
        &mut self,
        expression: ExprId,
        children: &[LiveValue],
        operands: &mut [Option<ConstValueId>],
        dec_depth: u32,
        bindings: &mut FrameBindings,
    ) -> Result<Option<LiveValue>, ConstantFoldError> {
        self.budget.step()?;
        let span = self.builder.view(expression)?.span();
        for (operand, &child) in operands.iter_mut().zip(children) {
            self.validate_live_value(child, span)?;
            *operand = Some(child.shape);
        }
        // These completed constructors allocate one staged row and a known-size payload.
        // Charge before the folder builds them, then exclude the promoted row from the scan.
        let view = self.builder.view(expression)?;
        let constructor_slots = match view.operation() {
            ExpressionOperationView::Collection(_) => {
                Some((children.len(), std::mem::size_of::<ConstValueId>()))
            }
            ExpressionOperationView::Struct { .. } | ExpressionOperationView::Choice { .. } => {
                Some((children.len(), std::mem::size_of::<ConstValueField>()))
            }
            ExpressionOperationView::Range { .. } => Some((2, std::mem::size_of::<ConstValueId>())),
            ExpressionOperationView::Coerce {
                operation: CoercionOperation::ToType(_),
                ..
            } => Some((1, std::mem::size_of::<ConstValueId>())),
            _ => None,
        };
        let constructor_slots = if matches!(view.success_shape(), SuccessShape::One(_)) {
            constructor_slots
        } else {
            None
        };
        if let Some((count, slot_bytes)) = constructor_slots {
            self.budget.retain_values(count.saturating_add(1), span)?;
            self.budget.retain_memory_bytes(
                std::mem::size_of::<ConstValue>().saturating_add(count.saturating_mul(slot_bytes)),
                span,
            )?;
        }
        let value_count = self.constants.value_count();
        let result = fold_completed_content(
            expression,
            self.builder,
            self.constants,
            FoldServices::new(
                self.string_table,
                Some(self.scope),
                Some(self.scope.external_package_registry.as_ref()),
            )
            .recover_cast_failure(self.state.active_handler.body.is_some()),
            self.types,
            operands,
            dec_depth,
        )?;
        if constructor_slots.is_some()
            && matches!(&result, ConstantValueFold::Value(value) if value.index() == value_count)
        {
            register_precharged_value(self.constants, self.string_table, value_count, self.budget)?;
        }
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        let shape = match result {
            ConstantValueFold::Value(value) => value,
            ConstantValueFold::NotConstant => return Ok(None),
            ConstantValueFold::Refused(ConstantFoldRefusalReason::CastConversion {
                error,
                span,
                ..
            }) => {
                self.execute_builtin_handler(error.code, Some(error.message), span, bindings)?;
                return Ok(None);
            }
            ConstantValueFold::Refused(reason) => {
                return Err(ConstantFoldError::Diagnostic(reason.into_draft()));
            }
        };
        // Constructors retain their ordered live children. The folded row owns
        // type and provenance, not a snapshot of the current allocation graph.
        let value = match self.builder.view(expression)?.operation() {
            ExpressionOperationView::Struct { .. }
            | ExpressionOperationView::Choice { .. }
            | ExpressionOperationView::Collection(_)
            | ExpressionOperationView::Coerce { .. } => {
                self.state
                    .storage
                    .allocate(shape, children.iter().copied(), self.budget)?
            }
            _ => self.materialize_value(shape, Permission::Writable)?,
        };
        Ok(Some(value))
    }
}

/// Preflight an occurrence-specific row before the shared store clones its payload.
pub(super) fn attach_live_provenance(
    constants: &mut ConstValueStore,
    string_table: &StringTable,
    budget: &mut ExecutionBudget,
    value: ConstValueId,
    provenance: &SyntheticInterfaceProvenance,
) -> Result<ConstValueId, ConstantFoldError> {
    if provenance.is_empty() {
        return Ok(value);
    }
    let metadata = constants.metadata(value).ok_or_else(value_invariant)?;
    if provenance.is_subset_of(&metadata.synthetic_interface_provenance) {
        return Ok(value);
    }
    let (bytes, slots) = if constants.shared_allocation(value).is_some() {
        (
            std::mem::size_of::<ConstValue>().saturating_add(
                metadata.synthetic_interface_provenance.owned_allocation_bytes(),
            ),
            1,
        )
    } else {
        value_own_allocation(constants, string_table, value)
    };
    budget.retain_memory_bytes(
        bytes.saturating_add(provenance.owned_allocation_bytes()),
        budget.span,
    )?;
    budget.retain_values(slots, budget.span)?;
    let index = constants.value_count();
    let value = constants.with_synthetic_interface_provenance(value, provenance)?;
    register_precharged_value(constants, string_table, index, budget)?;
    Ok(value)
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
    ) -> Result<Option<ProjectedConstant>, ConstantFoldError> {
        let view = self.builder.view(expression)?;
        let ExpressionOperationView::FieldAccess { base, field } = view.operation() else {
            return Ok(None);
        };
        let ExpressionOperationView::Constant(base_value) = base.operation() else {
            return Ok(None);
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
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        self.budget.step()?;
        let value_count = self.constants.value_count();
        let value = match fold_value_with_constants(
            expression,
            self.builder,
            self.constants,
            FoldServices::new(
                self.string_table,
                Some(self.scope),
                Some(self.scope.external_package_registry.as_ref()),
            )
            .recover_cast_failure(self.state.active_handler.body.is_some()),
            self.types,
            FoldRequirement::Required,
            |_| None,
        )? {
            ConstantValueFold::Value(value) => value,
            ConstantValueFold::NotConstant => return Ok(None),
            ConstantValueFold::Refused(ConstantFoldRefusalReason::CastConversion {
                error,
                span,
                ..
            }) => {
                self.execute_builtin_handler(error.code, Some(error.message), span, bindings)?;
                return Ok(None);
            }
            ConstantValueFold::Refused(reason) => {
                return Err(ConstantFoldError::Diagnostic(reason.into_draft()));
            }
        };
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        Ok(Some(value))
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
) -> ConstantFoldError {
    ConstantFoldError::Budget(ConstBudgetExhaustion {
        category,
        unit,
        limit,
        setting,
        span,
        span_owner: None,
        call_chain: Vec::new(),
    })
}

pub(super) fn hard_limit_error(
    span: Option<SourceSpan>,
    category: ConstBudgetCategory,
    unit: ConstBudgetUnit,
    limit: usize,
) -> ConstantFoldError {
    limit_error(span, category, unit, limit as u64, None)
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

/// Check the callee, call site and returned values against the same ordered result contract
/// before any receiver or AST publication can observe the slots.
pub(super) fn validate_call_results(
    constants: &ConstValueStore,
    call_types: &[TypeId],
    return_types: &[TypeId],
    slots: &[LiveValue],
) -> Result<(), ConstantFoldError> {
    if call_types != return_types || call_types.len() != slots.len() {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "const function result slots disagreed with its call signature",
        )
        .into());
    }
    for (&value, &expected) in slots.iter().zip(call_types) {
        if constants
            .metadata(value.shape)
            .map(|metadata| metadata.type_id)
            != Some(expected)
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function returned a value whose stored type disagreed with its result slot",
            )
            .into());
        }
    }
    Ok(())
}

/// Publish checked call results at an actual expression replacement boundary.
pub(super) fn commit_call_result(
    builder: &mut AstExpressionBuilder,
    call: ExprId,
    slots: &[ConstValueId],
    scope: PathId,
    provenance: SyntheticInterfaceProvenance,
) -> Result<ExprId, ConstantFoldError> {
    let replacement = if let [value] = slots {
        let view = builder.view(call)?;
        let (span, value_mode) = (view.span(), view.value_mode());
        let type_id = view.success_shape().single().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function call folded before its single success slot was available",
            )
        })?;
        expression_with_value(builder, *value, type_id, span, value_mode)?
    } else {
        // `scope` is the executing body's, not necessarily the call site's. It is inert because
        // the block holds only `Constant` rows.
        commit_constant_result_block(builder, call, slots, scope, None)?.id()
    };
    let replacement =
        builder.with_synthetic_interface_provenance(builder.own(replacement), provenance)?;
    Ok(replacement.id())
}

/// Read an already materialized constant result block, such as a folded Host call.
pub(super) fn constant_result_block_slots(
    builder: &AstExpressionBuilder,
    expression: ExprId,
) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
    let ExpressionOperationView::ValueBlock(ValueBlock::LexicalScope(block)) =
        builder.view(expression)?.operation()
    else {
        return Ok(None);
    };
    let [
        AstNode {
            kind: NodeKind::ThenValue(values),
            ..
        },
    ] = block.body.as_slice()
    else {
        return Ok(None);
    };
    let mut slots = Vec::with_capacity(values.expressions.len());
    for &row in &values.expressions {
        let ExpressionOperationView::Constant(value) = builder.view(row)?.operation() else {
            return Ok(None);
        };
        slots.push(value);
    }
    Ok(Some(slots))
}
