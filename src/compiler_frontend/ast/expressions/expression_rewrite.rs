//! Immutable expression rewrites: unchanged rows are reused and changed roots promote scratch.
use super::expression_store::{
    AstExpressionBuilder, CandidateEdge, CompletedOperation, ExprId, ExpressionConstructionFailure,
    ExpressionView, OwnedCompletedExpressionInput, OwnedExpr, OwnedExprs, PlaceId, PlaceProjection,
    PlaceProjectionView, ScratchCompletedExpressionInput,
};
use rustc_hash::FxHashMap;

/// Preserves occurrence facts without copying the operation being replaced.
pub(crate) fn replacement_input(
    view: ExpressionView<'_>,
    operation: CompletedOperation<OwnedExpr>,
) -> OwnedCompletedExpressionInput {
    OwnedCompletedExpressionInput {
        operation,
        success: view.success_shape(),
        error_type: view.error_type(),
        span: view.span(),
        value_mode: view.value_mode(),
        classification: view.classification(),
        failure: view.failure_facts().cloned(),
        occurrence: view.occurrence_facts().cloned(),
    }
}

pub(crate) fn promote_replacement(
    builder: &mut AstExpressionBuilder,
    input: OwnedCompletedExpressionInput,
) -> Result<OwnedExpr, ExpressionConstructionFailure> {
    let mut validation = Ok(());
    input.operation.visit_edges(&mut |id| {
        if validation.is_ok() {
            validation = builder.check_owner(id);
        }
    });
    validation?;
    builder.construct(move |candidate| {
        let mut retained: FxHashMap<ExprId, _> = FxHashMap::default();
        let mut failure = None;
        input.operation.visit_edges(&mut |id| {
            if failure.is_some() || retained.contains_key(&id.id()) {
                return;
            }
            match candidate.view(id).and_then(|view| candidate.retain(view)) {
                Ok(id_in_prefix) => {
                    retained.insert(id.id(), CandidateEdge::Existing(id_in_prefix));
                }
                Err(error) => failure = Some(error),
            }
        });
        if let Some(error) = failure {
            return Err(error.into());
        }
        let operation = input.operation.map_edges(|id| {
            *retained
                .get(&id.id())
                .expect("every fixed and variable edge was checked above")
        });
        let root = candidate.append_completed(ScratchCompletedExpressionInput {
            operation,
            success: input.success,
            error_type: input.error_type,
            span: input.span,
            value_mode: input.value_mode,
            classification: input.classification,
            failure: input.failure,
            occurrence: input.occurrence,
        })?;
        Ok(CandidateEdge::New(root))
    })
}

pub(crate) fn rewrite_expression_children(
    builder: &mut AstExpressionBuilder,
    expression: OwnedExpr,
    replacements: OwnedExprs<'_, (ExprId, ExprId)>,
) -> Result<OwnedExpr, ExpressionConstructionFailure> {
    builder.check_owner(expression)?;
    let raw_replacements = validate_replacements(builder, replacements)?;
    builder.check_owned(expression)?;
    if raw_replacements.iter().all(|(old, new)| old == new) {
        return Ok(expression);
    }
    let mut input = builder
        .view_owned(expression)?
        .to_owned_completed_input(builder)?;
    let replacement_map: FxHashMap<_, _> = raw_replacements.iter().copied().collect();
    let raw_replacement = |id: ExprId| replacement_map.get(&id).copied().unwrap_or(id);
    match &mut input.operation {
        CompletedOperation::ValueBlock(block) => {
            block.visit_expression_ids_mut(&mut |id| *id = raw_replacement(*id));
            let mut result = Ok(());
            block.visit_place_ids_mut(&mut |place| {
                if result.is_ok() {
                    result = rewrite_place_indices(builder, *place, replacements).map(|id| {
                        *place = id;
                    });
                }
            });
            result?;
        }
        CompletedOperation::RuntimeTemplate(handoff) => {
            handoff.visit_expression_ids_mut(&mut |id| *id = raw_replacement(*id));
        }
        CompletedOperation::RuntimeSlotApplication(handoff) => {
            handoff.visit_expression_ids_mut(&mut |id| *id = raw_replacement(*id));
        }
        _ => {}
    }
    input.operation = input
        .operation
        .map_edges(|id| builder.own(raw_replacement(id.id())));
    promote_replacement(builder, input)
}

pub(crate) fn rewrite_place_indices(
    builder: &mut AstExpressionBuilder,
    place: PlaceId,
    replacements: OwnedExprs<'_, (ExprId, ExprId)>,
) -> Result<PlaceId, ExpressionConstructionFailure> {
    let replacements = validate_replacements(builder, replacements)?;
    if replacements.iter().all(|(old, new)| old == new) {
        return Ok(place);
    }
    let view = builder.place(place)?;
    if !view.projections().any(|projection| match projection {
        PlaceProjectionView::Index { index, .. } => replacements
            .iter()
            .any(|(old, new)| *old == index.id() && old != new),
        PlaceProjectionView::Field { .. } => false,
    }) {
        return Ok(place);
    }
    let mut record = view.metadata().clone();
    let replacements: FxHashMap<_, _> = replacements.iter().copied().collect();
    let projections: Vec<PlaceProjection> = view
        .projections()
        .map(|projection| match projection {
            PlaceProjectionView::Field { field, result_type } => {
                PlaceProjection::Field { field, result_type }
            }
            PlaceProjectionView::Index { index, result_type } => PlaceProjection::Index {
                index: replacements.get(&index.id()).copied().unwrap_or(index.id()),
                result_type,
            },
        })
        .collect();
    let span = record.span;
    record.projections = builder.append_place_projections(builder.own_slice(&projections), span)?;
    builder.append_place(record, span)
}

fn validate_replacements<'a>(
    builder: &AstExpressionBuilder,
    replacements: OwnedExprs<'a, (ExprId, ExprId)>,
) -> Result<&'a [(ExprId, ExprId)], ExpressionConstructionFailure> {
    let replacements = builder.check_owned_slice(replacements)?;
    for (old, new) in replacements {
        builder.view(*old)?;
        builder.view(*new)?;
    }
    Ok(replacements)
}
