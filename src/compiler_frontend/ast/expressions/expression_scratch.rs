//! Producer-local DAGs. Nothing borrowed from a candidate survives promotion.
//! Candidate handles carry the prefix owner token and serial, while promotion keeps a raw ID map.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CandidateSerial(u64);
#[derive(Clone, Copy)]
pub(crate) struct ScratchExprId<'p> {
    owner: OwnerToken,
    candidate: CandidateSerial,
    index: u32,
    prefix: std::marker::PhantomData<&'p AstExpressionBuilder>,
}
#[derive(Clone, Copy)]
pub(crate) struct ExistingExprId<'p> {
    owner: OwnerToken,
    candidate: CandidateSerial,
    id: ExprId,
    prefix: std::marker::PhantomData<&'p AstExpressionBuilder>,
}
#[derive(Clone, Copy)]
pub(crate) enum CandidateEdge<'p> {
    New(ScratchExprId<'p>),
    Existing(ExistingExprId<'p>),
}
pub(crate) type CandidateRoot<'p> = CandidateEdge<'p>;
pub(crate) struct CandidateCall<E> {
    pub target: ResolvedCallableTarget,
    pub arguments: Vec<ResolvedArgument<E>>,
    pub handling: Option<HandlingRecord>,
    pub requires_external_float_validation: bool,
    pub error_type: Option<TypeId>,
    pub span: Option<SourceSpan>,
}
pub(crate) enum CompletedOperation<E> {
    Constant(ConstValueId),
    Reference(PathId),
    Retained(ExpressionOp<E>),
    Copy {
        root: PathId,
        root_type_id: TypeId,
        projections: Vec<PlaceProjection<E>>,
        type_id: TypeId,
        value_mode: ValueMode,
        span: Option<SourceSpan>,
    },
    FieldAccess {
        base: E,
        field: StringId,
    },
    NumericUnary {
        operand: E,
        facts: ResolvedOperation,
    },
    NumericBinary {
        left: E,
        right: E,
        facts: ResolvedOperation,
    },
    LogicalNot {
        operand: E,
    },
    LogicalBinary {
        left: E,
        right: E,
        operation: LogicalOperator,
    },
    Compare {
        left: E,
        right: E,
        facts: ResolvedOperation,
    },
    Range {
        start: E,
        end: E,
        facts: ResolvedOperation,
    },
    Call(CandidateCall<E>),
    MethodCall {
        receiver: E,
        call: CandidateCall<E>,
    },
    CollectionBuiltin {
        receiver: E,
        operation: CollectionBuiltinOp,
        arguments: Vec<ResolvedArgument<E>>,
        span: Option<SourceSpan>,
    },
    MapBuiltin {
        receiver: E,
        operation: MapBuiltinOp,
        receiver_requires_mutable: bool,
        arguments: Vec<ResolvedArgument<E>>,
        span: Option<SourceSpan>,
    },
    Cast {
        source: E,
        facts: CastFacts,
    },
    HandleFallible {
        value: E,
        handling: HandlingRecord,
    },
    PropagateOption {
        value: E,
    },
    Coerce {
        value: E,
        operation: CoercionOperation,
    },
    Collection(Vec<E>),
    Map(Vec<MapEntry<E>>),
    Struct {
        fields: Vec<ConstructorField<E>>,
    },
    Choice {
        tag: u32,
        fields: Vec<ConstructorField<E>>,
    },
    RouteConstruct {
        tag: u32,
        fields: Vec<ConstructorField<E>>,
        future_input: bool,
    },
    ValueBlock(ValueBlock),
    RuntimeTemplate(OwnedRuntimeTemplateHandoff),
    RuntimeSlotApplication(OwnedRuntimeSlotApplicationHandoff),
    Template(Template),
}
pub(crate) struct ScratchCompletedExpressionInput<'p> {
    pub operation: CompletedOperation<CandidateEdge<'p>>,
    pub success: SuccessShape,
    pub error_type: Option<TypeId>,
    pub span: Option<SourceSpan>,
    pub value_mode: ValueMode,
    pub classification: ExpressionClassification,
    pub failure: Option<ExpressionFailureFacts>,
    pub occurrence: Option<OccurrenceFacts>,
}
pub(crate) struct ExpressionCandidate<'p> {
    prefix: &'p AstExpressionBuilder,
    serial: CandidateSerial,
    rows: Vec<Option<ScratchCompletedExpressionInput<'p>>>,
}
impl<'p> ExpressionCandidate<'p> {
    pub(crate) fn view(&self, id: OwnedExpr) -> Result<ExpressionView<'p>, CompilerError> {
        self.prefix.view_owned(id)
    }
    pub(crate) fn retain(
        &self,
        view: ExpressionView<'p>,
    ) -> Result<ExistingExprId<'p>, CompilerError> {
        if self.prefix.storage.owner != view.store().owner {
            return Err(invariant("foreign expression prefix"));
        }
        self.prefix.view(view.id)?;
        Ok(ExistingExprId {
            owner: self.prefix.storage.owner,
            candidate: self.serial,
            id: view.id,
            prefix: std::marker::PhantomData,
        })
    }
    fn validate_edge_owner(&self, edge: CandidateEdge<'p>) -> Result<(), CompilerError> {
        let (owner, candidate) = match edge {
            CandidateEdge::New(id) => (id.owner, id.candidate),
            CandidateEdge::Existing(id) => (id.owner, id.candidate),
        };
        if owner != self.prefix.storage.owner || candidate != self.serial {
            return Err(invariant("foreign candidate expression"));
        }
        Ok(())
    }
    fn validate_edge(&self, edge: CandidateEdge<'p>) -> Result<(), CompilerError> {
        self.validate_edge_owner(edge)?;
        match edge {
            CandidateEdge::New(id) => {
                if id.index as usize >= self.rows.len() {
                    return Err(invariant("unallocated scratch expression"));
                }
            }
            CandidateEdge::Existing(id) => {
                self.prefix.view(id.id)?;
            }
        }
        Ok(())
    }
    fn edge_type(&self, edge: CandidateEdge<'p>) -> Option<TypeId> {
        match edge {
            CandidateEdge::Existing(id) => match self.prefix.storage.rows[id.id.index()].success {
                SuccessShape::One(id) => Some(id),
                _ => None,
            },
            CandidateEdge::New(id) => match &self.rows[id.index as usize]
                .as_ref()
                .expect("live scratch row")
                .success
            {
                SuccessShape::One(id) => Some(*id),
                _ => None,
            },
        }
    }
    pub(crate) fn append_completed(
        &mut self,
        input: ScratchCompletedExpressionInput<'p>,
    ) -> Result<ScratchExprId<'p>, ExpressionConstructionFailure> {
        let mut valid = Ok(());
        input.operation.visit_edges(&mut |edge| {
            if valid.is_ok() {
                valid = self.validate_edge_owner(edge);
            }
        });
        valid?;
        valid = Ok(());
        input.operation.visit_edges(&mut |edge| {
            if valid.is_ok() {
                valid = self.validate_edge(edge);
            }
        });
        valid?;
        input.operation.validate_committed(
            &self.prefix.storage,
            &input.success,
            |edge| match edge {
                CandidateEdge::Existing(id) => id.id,
                CandidateEdge::New(id) => ExprId::at(self.prefix.len() + id.index as usize),
            },
        )?;
        if let Some((left, right, facts, kind)) = input.operation.resolved_edges() {
            let kind_valid = match kind {
                0 => matches!(facts.kind, ResolvedOperationKind::Numeric(_)),
                1 => matches!(
                    facts.kind,
                    ResolvedOperationKind::Comparison(_) | ResolvedOperationKind::NonNumeric
                ),
                _ => matches!(facts.kind, ResolvedOperationKind::Range { .. }),
            };
            if !kind_valid
                || !matches!(input.success, SuccessShape::One(id) if id == facts.result_type)
                || self.edge_type(left) != Some(facts.left_type)
                || right.is_some() != facts.right_type.is_some()
                || right.and_then(|edge| self.edge_type(edge)) != facts.right_type
            {
                return Err(
                    invariant("scratch resolved operation disagrees with edges or result").into(),
                );
            }
        }
        checked_extent(self.rows.len(), 1, self.prefix.limit, input.span, true)?;
        let id = ScratchExprId {
            owner: self.prefix.storage.owner,
            candidate: self.serial,
            index: self.rows.len() as u32,
            prefix: std::marker::PhantomData,
        };
        self.rows.push(Some(input));
        Ok(id)
    }
    pub(crate) fn append_replacement(
        &mut self,
        input: CompletedExpressionInput,
    ) -> Result<ScratchExprId<'p>, ExpressionConstructionFailure> {
        self.prefix.check_operation_owned(input.operation)?;
        let operation = input.operation.map_edges(|id| {
            CandidateEdge::Existing(ExistingExprId {
                owner: self.prefix.storage.owner,
                candidate: self.serial,
                id: id.id(),
                prefix: std::marker::PhantomData,
            })
        });
        self.append_completed(ScratchCompletedExpressionInput {
            operation: CompletedOperation::Retained(operation),
            success: input.success,
            error_type: input.error_type,
            span: input.span,
            value_mode: input.value_mode,
            classification: input.classification,
            failure: input.failure,
            occurrence: input.occurrence,
        })
    }
    fn prepare(self, root: CandidateRoot<'p>) -> Result<Promotion, ExpressionConstructionFailure> {
        self.validate_edge(root)?;
        if let CandidateEdge::Existing(id) = root {
            return Ok(Promotion::Existing(id.id));
        }
        let CandidateEdge::New(root) = root else {
            unreachable!()
        };
        let mut colors = vec![0u8; self.rows.len()];
        let mut order = Vec::new();
        let mut stack = vec![(root.index as usize, false)];
        while let Some((index, exit)) = stack.pop() {
            if exit {
                colors[index] = 2;
                order.push(index);
                continue;
            }
            if colors[index] == 2 {
                continue;
            }
            if colors[index] == 1 {
                return Err(invariant("cycle in expression candidate").into());
            }
            colors[index] = 1;
            stack.push((index, true));
            let row = self.rows[index]
                .as_ref()
                .ok_or_else(|| invariant("missing candidate row"))?;
            let mut error = None;
            row.operation.visit_edges(&mut |edge| {
                if let Err(e) = self.validate_edge(edge) {
                    error = Some(e);
                } else if let CandidateEdge::New(id) = edge {
                    stack.push((id.index as usize, false));
                }
            });
            if let Some(e) = error {
                return Err(e.into());
            }
        }
        let mut totals = SideCounts::default();
        for &index in &order {
            totals.add(self.rows[index].as_ref().unwrap())?;
        }
        totals.check(self.prefix, order.len())?;
        let mut mapping = vec![None; self.rows.len()];
        for (offset, &index) in order.iter().enumerate() {
            mapping[index] = Some(ExprId::at(self.prefix.len() + offset));
        }
        let final_root = mapping[root.index as usize].unwrap();
        let mut rows = self.rows;
        let mut inputs = Vec::with_capacity(order.len());
        for index in order {
            let row = rows[index].take().unwrap();
            let operation = row.operation.map_edges(|edge| {
                self.prefix.own(match edge {
                    CandidateEdge::Existing(id) => id.id,
                    CandidateEdge::New(id) => {
                        mapping[id.index as usize].expect("reachable edge mapped")
                    }
                })
            });
            inputs.push(OwnedCompletedExpressionInput {
                operation,
                success: row.success,
                error_type: row.error_type,
                span: row.span,
                value_mode: row.value_mode,
                classification: row.classification,
                failure: row.failure,
                occurrence: row.occurrence,
            });
        }
        Ok(Promotion::New {
            root: final_root,
            inputs,
        })
    }
}
pub(crate) struct OwnedCompletedExpressionInput {
    pub operation: CompletedOperation<OwnedExpr>,
    pub success: SuccessShape,
    pub error_type: Option<TypeId>,
    pub span: Option<SourceSpan>,
    pub value_mode: ValueMode,
    pub classification: ExpressionClassification,
    pub failure: Option<ExpressionFailureFacts>,
    pub occurrence: Option<OccurrenceFacts>,
}
enum Promotion {
    Existing(ExprId),
    New {
        root: ExprId,
        inputs: Vec<OwnedCompletedExpressionInput>,
    },
}
impl AstExpressionBuilder {
    pub(crate) fn construct(
        &mut self,
        produce: impl for<'p> FnOnce(
            &mut ExpressionCandidate<'p>,
        )
            -> Result<CandidateRoot<'p>, ExpressionConstructionFailure>,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        let candidate_serial = self
            .candidate_serial
            .checked_add(1)
            .ok_or_else(|| invariant("candidate serial exhausted"))?;
        let promotion = {
            let mut candidate = ExpressionCandidate {
                prefix: self,
                serial: CandidateSerial(candidate_serial),
                rows: Vec::new(),
            };
            let root = produce(&mut candidate)?;
            candidate.prepare(root)?
        };
        self.candidate_serial = candidate_serial;
        match promotion {
            Promotion::Existing(id) => Ok(self.own(id)),
            Promotion::New { root, inputs } => {
                for input in inputs {
                    self.append_owned_completed(input)?;
                }
                Ok(self.own(root))
            }
        }
    }
    pub(crate) fn append_owned_completed(
        &mut self,
        input: OwnedCompletedExpressionInput,
    ) -> Result<OwnedExpr, ExpressionConstructionFailure> {
        input.operation.validate_owned_edges(self)?;
        input
            .operation
            .validate_committed(&self.storage, &input.success, OwnedExpr::id)?;
        if let Some((left, right, facts, kind)) = input.operation.resolved_edges() {
            let kind_valid = match kind {
                0 => matches!(facts.kind, ResolvedOperationKind::Numeric(_)),
                1 => matches!(
                    facts.kind,
                    ResolvedOperationKind::Comparison(_) | ResolvedOperationKind::NonNumeric
                ),
                _ => matches!(facts.kind, ResolvedOperationKind::Range { .. }),
            };
            if !kind_valid
                || !matches!(input.success, SuccessShape::One(id) if id == facts.result_type)
                || self.view_owned(left)?.value_type_id()? != facts.left_type
                || right
                    .map(|id| self.view_owned(id).and_then(|v| v.value_type_id()))
                    .transpose()?
                    != facts.right_type
            {
                return Err(invariant("resolved operation disagrees with edges or result").into());
            }
        }
        let mut counts = SideCounts::default();
        counts.add_owned(&input)?;
        counts.check(self, 1)?;
        let operation = input.operation.install(self, input.span)?;
        self.append_completed(CompletedExpressionInput {
            operation,
            success: input.success,
            error_type: input.error_type,
            span: input.span,
            value_mode: input.value_mode,
            classification: input.classification,
            failure: input.failure,
            occurrence: input.occurrence,
        })
    }
}

#[path = "expression_scratch_operations.rs"]
mod operations;
use operations::SideCounts;

#[cfg(test)]
#[path = "tests/expression_scratch_tests.rs"]
pub(super) mod tests;
