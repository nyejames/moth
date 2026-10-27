//! Read-only views over stored expression records.
//!
//! WHAT: wraps each record family, such as arguments, constructor fields, map entries, calls,
//! places and operations, with its owning `ExpressionStore`. Child edges resolve to views.
//! WHY: consumers read through these views instead of indexing the typed side buffers, so range
//! bounds and validated-edge invariants stay inside the store module.

use super::*;
#[derive(Clone, Copy)]
pub(crate) struct ArgumentView<'a> {
    owner: &'a ExpressionStore,
    record: &'a ResolvedArgument,
}
impl<'a> ArgumentView<'a> {
    pub(crate) fn value(self) -> ExpressionView<'a> {
        self.owner
            .view(self.record.value)
            .expect("validated argument")
    }
    pub(crate) fn metadata(self) -> &'a ResolvedArgument {
        self.record
    }
}
#[derive(Clone, Copy)]
pub(crate) struct ConstructorFieldView<'a> {
    owner: &'a ExpressionStore,
    record: &'a ConstructorField,
}
impl<'a> ConstructorFieldView<'a> {
    pub(crate) fn value(self) -> ExpressionView<'a> {
        self.owner
            .view(self.record.value)
            .expect("validated constructor")
    }
    pub(crate) fn name(self) -> PathId {
        self.record.name
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MapEntryView<'a> {
    owner: &'a ExpressionStore,
    record: &'a MapEntry,
}
impl<'a> MapEntryView<'a> {
    pub(crate) fn key(self) -> ExpressionView<'a> {
        self.owner.view(self.record.key).expect("validated map key")
    }
    pub(crate) fn value(self) -> ExpressionView<'a> {
        self.owner
            .view(self.record.value)
            .expect("validated map value")
    }
}
#[derive(Clone, Copy)]
pub(crate) struct ExpressionListView<'a> {
    owner: &'a ExpressionStore,
    values: &'a [ExprId],
}
impl<'a> ExpressionListView<'a> {
    pub(crate) fn len(self) -> usize {
        self.values.len()
    }
    pub(crate) fn iter(self) -> impl ExactSizeIterator<Item = ExpressionView<'a>> + 'a {
        let owner = self.owner;
        self.values
            .iter()
            .map(move |&id| owner.view(id).expect("validated collection"))
    }
}
#[derive(Clone, Copy)]
pub(crate) struct ArgumentListView<'a> {
    owner: &'a ExpressionStore,
    values: &'a [ResolvedArgument],
}
impl<'a> ArgumentListView<'a> {
    pub(crate) fn len(self) -> usize {
        self.values.len()
    }
    pub(crate) fn iter(self) -> impl ExactSizeIterator<Item = ArgumentView<'a>> + 'a {
        let owner = self.owner;
        self.values
            .iter()
            .map(move |record| ArgumentView { owner, record })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct FieldListView<'a> {
    owner: &'a ExpressionStore,
    values: &'a [ConstructorField],
}
impl<'a> FieldListView<'a> {
    pub(crate) fn len(self) -> usize {
        self.values.len()
    }
    pub(crate) fn iter(self) -> impl ExactSizeIterator<Item = ConstructorFieldView<'a>> + 'a {
        let owner = self.owner;
        self.values
            .iter()
            .map(move |record| ConstructorFieldView { owner, record })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct MapListView<'a> {
    owner: &'a ExpressionStore,
    values: &'a [MapEntry],
}
impl<'a> MapListView<'a> {
    pub(crate) fn len(self) -> usize {
        self.values.len()
    }
    pub(crate) fn iter(self) -> impl ExactSizeIterator<Item = MapEntryView<'a>> + 'a {
        let owner = self.owner;
        self.values
            .iter()
            .map(move |record| MapEntryView { owner, record })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct CallView<'a> {
    owner: &'a ExpressionStore,
    record: &'a ResolvedCall,
}
impl<'a> CallView<'a> {
    pub(crate) fn metadata(self) -> &'a ResolvedCall {
        self.record
    }
    pub(crate) fn arguments(self) -> ArgumentListView<'a> {
        self.owner.arguments_view(self.record.arguments)
    }
    pub(crate) fn handling(self) -> Option<&'a HandlingRecord> {
        self.record
            .handling
            .map(|id| &self.owner.handling[id.index()])
    }
}
#[derive(Clone, Copy)]
pub(crate) struct PlaceView<'a> {
    owner: &'a ExpressionStore,
    record: &'a PlaceRecord,
}
pub(crate) enum PlaceProjectionView<'a> {
    Field {
        field: StringId,
        result_type: TypeId,
    },
    Index {
        index: ExpressionView<'a>,
        result_type: TypeId,
    },
}
impl<'a> PlaceView<'a> {
    pub(crate) fn metadata(self) -> &'a PlaceRecord {
        self.record
    }
    pub(crate) fn projections(self) -> impl ExactSizeIterator<Item = PlaceProjectionView<'a>> + 'a {
        let owner = self.owner;
        self.owner.projections[self
            .record
            .projections
            .bounds(self.owner.projections.len())
            .expect("validated place range")]
        .iter()
        .map(move |p| match *p {
            PlaceProjection::Field { field, result_type } => {
                PlaceProjectionView::Field { field, result_type }
            }
            PlaceProjection::Index { index, result_type } => PlaceProjectionView::Index {
                index: owner.view(index).expect("validated place index"),
                result_type,
            },
        })
    }
}
pub(crate) enum ExpressionOperationView<'a> {
    Constant(ConstValueId),
    Reference(PathId),
    Copy(PlaceView<'a>),
    FieldAccess {
        base: ExpressionView<'a>,
        field: StringId,
    },
    NumericUnary {
        operand: ExpressionView<'a>,
        facts: &'a ResolvedOperation,
    },
    NumericBinary {
        left: ExpressionView<'a>,
        right: ExpressionView<'a>,
        facts: &'a ResolvedOperation,
    },
    LogicalNot {
        operand: ExpressionView<'a>,
    },
    LogicalBinary {
        left: ExpressionView<'a>,
        right: ExpressionView<'a>,
        operation: LogicalOperator,
    },
    Compare {
        left: ExpressionView<'a>,
        right: ExpressionView<'a>,
        facts: &'a ResolvedOperation,
    },
    Range {
        start: ExpressionView<'a>,
        end: ExpressionView<'a>,
        facts: &'a ResolvedOperation,
    },
    Call(CallView<'a>),
    MethodCall {
        receiver: ExpressionView<'a>,
        call: CallView<'a>,
    },
    CollectionBuiltin {
        receiver: ExpressionView<'a>,
        facts: &'a CollectionCall,
        arguments: ArgumentListView<'a>,
    },
    MapBuiltin {
        receiver: ExpressionView<'a>,
        facts: &'a MapCall,
        arguments: ArgumentListView<'a>,
    },
    Cast {
        source: ExpressionView<'a>,
        facts: &'a CastFacts,
    },
    HandleFallible {
        value: ExpressionView<'a>,
        handling: &'a HandlingRecord,
    },
    PropagateOption {
        value: ExpressionView<'a>,
    },
    Coerce {
        value: ExpressionView<'a>,
        operation: CoercionOperation,
    },
    Collection(ExpressionListView<'a>),
    Map(MapListView<'a>),
    Struct {
        fields: FieldListView<'a>,
    },
    Choice {
        tag: u32,
        fields: FieldListView<'a>,
    },
    RouteConstruct {
        tag: u32,
        fields: FieldListView<'a>,
        future_input: bool,
    },
    ValueBlock(&'a ValueBlock),
    RuntimeTemplate(&'a OwnedRuntimeTemplateHandoff),
    RuntimeSlotApplication(&'a OwnedRuntimeSlotApplicationHandoff),
    Template(&'a Template),
}
impl ExpressionStore {
    fn arguments_view(&self, range: ArgumentRange) -> ArgumentListView<'_> {
        ArgumentListView {
            owner: self,
            values: &self.arguments[range
                .bounds(self.arguments.len())
                .expect("validated argument range")],
        }
    }
    fn fields_view(&self, range: ConstructorFieldRange) -> FieldListView<'_> {
        FieldListView {
            owner: self,
            values: &self.constructor_fields[range
                .bounds(self.constructor_fields.len())
                .expect("validated constructor range")],
        }
    }
    fn call_view(&self, id: CallId) -> CallView<'_> {
        CallView {
            owner: self,
            record: &self.calls[id.index()],
        }
    }
}
impl<'a> ExpressionView<'a> {
    pub(crate) fn value_type_id(self) -> Result<TypeId, InfrastructureFailure> {
        match self.row().success {
            SuccessShape::One(id) => Ok(id),
            _ => Err(invariant(
                "single value type requested from non-single producer",
            )),
        }
    }
    pub(crate) fn bind(self, id: ExprId) -> Result<ExpressionView<'a>, InfrastructureFailure> {
        self.owner.view(id)
    }
    pub(crate) fn operation(self) -> ExpressionOperationView<'a> {
        self.owner.operation_view(self.row().operation)
    }
    pub(crate) fn operation_id(self) -> ExpressionOp {
        self.row().operation
    }
    pub(crate) fn success(self) -> SuccessShape {
        self.row().success
    }
    pub(crate) fn to_completed_input(
        self,
        builder: &AstExpressionBuilder,
    ) -> Result<CompletedExpressionInput, InfrastructureFailure> {
        builder.store().check_owner_token(self.owner_token())?;
        Ok(CompletedExpressionInput {
            operation: self.operation_id().map_edges(|id| builder.own(id)),
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
impl ExpressionStore {
    pub(crate) fn place(&self, id: PlaceId) -> Result<PlaceView<'_>, InfrastructureFailure> {
        let record = self
            .places
            .get(id.index())
            .ok_or_else(|| invariant("place ID outside owner"))?;
        Ok(PlaceView {
            owner: self,
            record,
        })
    }
    fn operation_view(&self, operation: ExpressionOp) -> ExpressionOperationView<'_> {
        let o = self;
        let child = |id| o.view(id).expect("validated fixed edge");
        use ExpressionOperationView as V;
        match operation {
            ExpressionOp::Constant(id) => V::Constant(id),
            ExpressionOp::Reference(id) => V::Reference(id),
            ExpressionOp::Copy(id) => V::Copy(PlaceView {
                owner: o,
                record: &o.places[id.index()],
            }),
            ExpressionOp::FieldAccess { base, field } => V::FieldAccess {
                base: child(base),
                field,
            },
            ExpressionOp::NumericUnary { operand, facts } => V::NumericUnary {
                operand: child(operand),
                facts: &o.numeric[facts.index()],
            },
            ExpressionOp::NumericBinary { left, right, facts } => V::NumericBinary {
                left: child(left),
                right: child(right),
                facts: &o.numeric[facts.index()],
            },
            ExpressionOp::LogicalNot { operand } => V::LogicalNot {
                operand: child(operand),
            },
            ExpressionOp::LogicalBinary {
                left,
                right,
                operation,
            } => V::LogicalBinary {
                left: child(left),
                right: child(right),
                operation,
            },
            ExpressionOp::Compare { left, right, facts } => V::Compare {
                left: child(left),
                right: child(right),
                facts: &o.comparisons[facts.index()],
            },
            ExpressionOp::Range { start, end, facts } => V::Range {
                start: child(start),
                end: child(end),
                facts: &o.ranges[facts.index()],
            },
            ExpressionOp::Call(id) => V::Call(o.call_view(id)),
            ExpressionOp::MethodCall { receiver, call } => V::MethodCall {
                receiver: child(receiver),
                call: o.call_view(call),
            },
            ExpressionOp::CollectionBuiltin { receiver, call } => {
                let facts = &o.collection_calls[call.index()];
                V::CollectionBuiltin {
                    receiver: child(receiver),
                    facts,
                    arguments: o.arguments_view(facts.arguments),
                }
            }
            ExpressionOp::MapBuiltin { receiver, call } => {
                let facts = &o.map_calls[call.index()];
                V::MapBuiltin {
                    receiver: child(receiver),
                    facts,
                    arguments: o.arguments_view(facts.arguments),
                }
            }
            ExpressionOp::Cast { source, facts } => V::Cast {
                source: child(source),
                facts: &o.casts[facts.index()],
            },
            ExpressionOp::HandleFallible { value, handling } => V::HandleFallible {
                value: child(value),
                handling: &o.handling[handling.index()],
            },
            ExpressionOp::PropagateOption { value } => V::PropagateOption {
                value: child(value),
            },
            ExpressionOp::Coerce { value, operation } => V::Coerce {
                value: child(value),
                operation,
            },
            ExpressionOp::Collection(range) => V::Collection(ExpressionListView {
                owner: o,
                values: &o.children[range.bounds(o.children.len()).expect("validated children")],
            }),
            ExpressionOp::Map(range) => V::Map(MapListView {
                owner: o,
                values: &o.map_entries[range.bounds(o.map_entries.len()).expect("validated map")],
            }),
            ExpressionOp::Struct { fields } => V::Struct {
                fields: o.fields_view(fields),
            },
            ExpressionOp::Choice { tag, fields } => V::Choice {
                tag,
                fields: o.fields_view(fields),
            },
            ExpressionOp::RouteConstruct {
                tag,
                fields,
                future_input,
            } => V::RouteConstruct {
                tag,
                fields: o.fields_view(fields),
                future_input,
            },
            ExpressionOp::ValueBlock(id) => V::ValueBlock(&o.value_blocks[id.index()]),
            ExpressionOp::RuntimeTemplate(id) => {
                V::RuntimeTemplate(&o.runtime_templates[id.index()])
            }
            ExpressionOp::RuntimeSlotApplication(id) => {
                V::RuntimeSlotApplication(&o.runtime_slots[id.index()])
            }
            ExpressionOp::Template(id) => V::Template(&o.templates[id.index()]),
        }
    }
}
impl AstExpressionBuilder {
    pub(crate) fn place(&self, id: PlaceId) -> Result<PlaceView<'_>, InfrastructureFailure> {
        self.storage.place(id)
    }
    pub(crate) fn operation_view(
        &self,
        operation: ExpressionOp,
    ) -> Result<ExpressionOperationView<'_>, InfrastructureFailure> {
        self.visit_operation_children(operation, |_| ())?;
        Ok(self.storage.operation_view(operation))
    }
    pub(crate) fn visit_operation_children<'a>(
        &'a self,
        operation: ExpressionOp,
        mut visit: impl FnMut(ExpressionView<'a>),
    ) -> Result<(), InfrastructureFailure> {
        let owner = &self.storage;
        let mut error = None;
        owner.visit_operation_edges(operation, &mut |id| match owner.view(id) {
            Ok(view) => visit(view),
            Err(e) => error = Some(e),
        })?;
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(())
        }
    }
}
