//! Owned snapshots are requested only after a rewrite discovers changed edges.
use super::*;
impl<'a> ExpressionView<'a> {
    pub(crate) fn to_owned_completed_input(
        self,
        builder: &AstExpressionBuilder,
    ) -> Result<OwnedCompletedExpressionInput, CompilerError> {
        builder.store().check_owner_token(self.owner_token())?;
        // Every range and edge read below was checked when its row was appended, so these
        // expects guard the store invariant rather than user input.
        let owner = self.owner;
        let arguments = |range: ArgumentRange| {
            owner.arguments[range
                .bounds(owner.arguments.len())
                .expect("validated argument range")]
            .iter()
            .map(|value| ResolvedArgument {
                value: builder.own(value.value),
                target_param: value.target_param,
                parameter_slot: value.parameter_slot,
                access_mode: value.access_mode,
                passing_mode: value.passing_mode,
                span: value.span,
                target_span: value.target_span,
                marker_span: value.marker_span,
                following_separator_span: value.following_separator_span,
            })
            .collect()
        };
        let call = |id: CallId| {
            let c = &owner.calls[id.index()];
            CandidateCall {
                target: c.target,
                arguments: arguments(c.arguments),
                handling: c.handling.map(|id| owner.handling[id.index()].clone()),
                requires_external_float_validation: c.requires_external_float_validation,
                error_type: c.error_type,
                span: c.span,
            }
        };
        let fields = |range: ConstructorFieldRange| {
            owner.constructor_fields[range
                .bounds(owner.constructor_fields.len())
                .expect("validated constructor range")]
            .iter()
            .map(|value| ConstructorField {
                name: value.name,
                slot: value.slot,
                value: builder.own(value.value),
            })
            .collect()
        };
        use CompletedOperation as O;
        let operation = match self.row().operation.map_edges(|id| builder.own(id)) {
            ExpressionOp::Constant(v) => O::Constant(v),
            ExpressionOp::Reference(v) => O::Reference(v),
            ExpressionOp::Copy(id) => {
                let p = &owner.places[id.index()];
                O::Copy {
                    root: p.root,
                    root_type_id: p.root_type_id,
                    projections: owner.projections[p
                        .projections
                        .bounds(owner.projections.len())
                        .expect("validated place range")]
                    .iter()
                    .map(|projection| match *projection {
                        PlaceProjection::Field { field, result_type } => {
                            PlaceProjection::Field { field, result_type }
                        }
                        PlaceProjection::Index { index, result_type } => PlaceProjection::Index {
                            index: builder.own(index),
                            result_type,
                        },
                    })
                    .collect(),
                    type_id: p.type_id,
                    value_mode: p.value_mode,
                    span: p.span,
                }
            }
            ExpressionOp::FieldAccess { base, field } => O::FieldAccess { base, field },
            ExpressionOp::NumericUnary { operand, facts } => O::NumericUnary {
                operand,
                facts: owner.numeric[facts.index()],
            },
            ExpressionOp::NumericBinary { left, right, facts } => O::NumericBinary {
                left,
                right,
                facts: owner.numeric[facts.index()],
            },
            ExpressionOp::LogicalNot { operand } => O::LogicalNot { operand },
            ExpressionOp::LogicalBinary {
                left,
                right,
                operation,
            } => O::LogicalBinary {
                left,
                right,
                operation,
            },
            ExpressionOp::Compare { left, right, facts } => O::Compare {
                left,
                right,
                facts: owner.comparisons[facts.index()],
            },
            ExpressionOp::Range { start, end, facts } => O::Range {
                start,
                end,
                facts: owner.ranges[facts.index()],
            },
            ExpressionOp::Call(id) => O::Call(call(id)),
            ExpressionOp::MethodCall { receiver, call: id } => O::MethodCall {
                receiver,
                call: call(id),
            },
            ExpressionOp::CollectionBuiltin { receiver, call } => {
                let c = &owner.collection_calls[call.index()];
                O::CollectionBuiltin {
                    receiver,
                    operation: c.operation,
                    arguments: arguments(c.arguments),
                    span: c.span,
                }
            }
            ExpressionOp::MapBuiltin { receiver, call } => {
                let c = &owner.map_calls[call.index()];
                O::MapBuiltin {
                    receiver,
                    operation: c.operation,
                    receiver_requires_mutable: c.receiver_requires_mutable,
                    arguments: arguments(c.arguments),
                    span: c.span,
                }
            }
            ExpressionOp::Cast { source, facts } => O::Cast {
                source,
                facts: owner.casts[facts.index()].clone(),
            },
            ExpressionOp::HandleFallible { value, handling } => O::HandleFallible {
                value,
                handling: owner.handling[handling.index()].clone(),
            },
            ExpressionOp::PropagateOption { value } => O::PropagateOption { value },
            ExpressionOp::Coerce { value, operation } => O::Coerce { value, operation },
            ExpressionOp::Collection(range) => O::Collection(
                owner.children[range
                    .bounds(owner.children.len())
                    .expect("validated child range")]
                .iter()
                .map(|&id| builder.own(id))
                .collect(),
            ),
            ExpressionOp::Map(range) => O::Map(
                owner.map_entries[range
                    .bounds(owner.map_entries.len())
                    .expect("validated map range")]
                .iter()
                .map(|entry| MapEntry {
                    key: builder.own(entry.key),
                    value: builder.own(entry.value),
                })
                .collect(),
            ),
            ExpressionOp::Struct { fields: range } => O::Struct {
                fields: fields(range),
            },
            ExpressionOp::Choice { tag, fields: range } => O::Choice {
                tag,
                fields: fields(range),
            },
            ExpressionOp::ValueBlock(id) => O::ValueBlock(owner.value_blocks[id.index()].clone()),
            ExpressionOp::RuntimeTemplate(id) => {
                O::RuntimeTemplate(owner.runtime_templates[id.index()].clone())
            }
            ExpressionOp::RuntimeSlotApplication(id) => {
                O::RuntimeSlotApplication(owner.runtime_slots[id.index()].clone())
            }
            ExpressionOp::Template(id) => O::Template(owner.templates[id.index()].clone()),
        };
        Ok(OwnedCompletedExpressionInput {
            operation,
            success: self.success(),
            error_type: self.error_type(),
            span: self.span(),
            value_mode: self.value_mode(),
            classification: self.classification(),
            failure: self.failure_facts().cloned(),
            occurrence: self.occurrence_facts().cloned(),
        })
    }
}
