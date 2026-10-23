//! Scratch-to-store operation mapping.
//!
//! WHAT: installs completed scratch operations as stored `ExpressionOp`s, visits their edges and
//! tallies the side-buffer demand each operation adds.
//! WHY: the tally counts the same operation shapes that installation appends, so side-buffer
//! overflow reports a capacity diagnostic instead of growing past the limit.

use super::*;
impl<E: Copy> CompletedOperation<E> {
    pub(crate) fn visit_edges(&self, visit: &mut dyn FnMut(E)) {
        match self {
            Self::Retained(operation) => operation.visit_edges(visit),
            Self::Copy { projections, .. } => {
                for p in projections {
                    if let PlaceProjection::Index { index, .. } = p {
                        visit(*index);
                    }
                }
            }
            Self::FieldAccess { base, .. } => visit(*base),
            Self::NumericUnary { operand, .. } | Self::LogicalNot { operand } => visit(*operand),
            Self::NumericBinary { left, right, .. }
            | Self::LogicalBinary { left, right, .. }
            | Self::Compare { left, right, .. } => {
                visit(*left);
                visit(*right);
            }
            Self::Range { start, end, .. } => {
                visit(*start);
                visit(*end);
            }
            Self::Call(call) | Self::MethodCall { call, .. } => {
                if let Self::MethodCall { receiver, .. } = self {
                    visit(*receiver);
                }
                for arg in &call.arguments {
                    visit(arg.value);
                }
            }
            Self::CollectionBuiltin {
                receiver,
                arguments,
                ..
            }
            | Self::MapBuiltin {
                receiver,
                arguments,
                ..
            } => {
                visit(*receiver);
                for arg in arguments {
                    visit(arg.value);
                }
            }
            Self::Cast { source, .. } => visit(*source),
            Self::HandleFallible { value, .. }
            | Self::PropagateOption { value }
            | Self::Coerce { value, .. } => visit(*value),
            Self::Collection(values) => {
                for &value in values {
                    visit(value);
                }
            }
            Self::Map(entries) => {
                for entry in entries {
                    visit(entry.key);
                    visit(entry.value);
                }
            }
            Self::Struct { fields } | Self::Choice { fields, .. } => {
                for field in fields {
                    visit(field.value);
                }
            }
            Self::RouteConstruct { fields, .. } => {
                for field in fields {
                    visit(field.value);
                }
            }
            _ => (),
        }
    }
    pub(super) fn visit_committed_edges(&self, visit: &mut dyn FnMut(ExprId)) {
        match self {
            Self::ValueBlock(v) => v.visit_expression_ids(visit),
            Self::RuntimeTemplate(v) => v.visit_expression_ids(visit),
            Self::RuntimeSlotApplication(v) => v.visit_expression_ids(visit),
            _ => (),
        }
    }
    pub(super) fn validate_committed(
        &self,
        owner: &ExpressionStore,
        success: &SuccessShape,
        mut raw: impl FnMut(E) -> ExprId,
    ) -> Result<(), CompilerError> {
        let mut valid = Ok(());
        self.visit_committed_edges(&mut |id| {
            if valid.is_ok() {
                valid = owner.view(id).map(|_| ());
            }
        });
        valid?;
        if let Self::ValueBlock(value) = self {
            owner.validate_place_edges(|visit| value.visit_place_ids(visit))?;
        }
        if let SuccessShape::Multiple(range) = success {
            if range.len < 2 {
                return Err(invariant("invalid retained Multiple range"));
            }
            range.bounds(owner.result_types.len())?;
        }
        if let Self::Retained(operation) = self {
            owner.validate_operation(operation.map_edges(&mut raw), *success)?;
        }
        Ok(())
    }
    pub(super) fn resolved_edges(&self) -> Option<(E, Option<E>, &ResolvedOperation, u8)> {
        match self {
            Self::NumericUnary { operand, facts } => Some((*operand, None, facts, 0)),
            Self::NumericBinary { left, right, facts } => Some((*left, Some(*right), facts, 0)),
            Self::Compare { left, right, facts } => Some((*left, Some(*right), facts, 1)),
            Self::Range { start, end, facts } => Some((*start, Some(*end), facts, 2)),
            _ => None,
        }
    }
    pub(crate) fn map_edges<F>(self, mut map: impl FnMut(E) -> F) -> CompletedOperation<F> {
        fn args<E, F>(
            values: Vec<ResolvedArgument<E>>,
            map: &mut impl FnMut(E) -> F,
        ) -> Vec<ResolvedArgument<F>> {
            values
                .into_iter()
                .map(|a| ResolvedArgument {
                    value: map(a.value),
                    target_param: a.target_param,
                    parameter_slot: a.parameter_slot,
                    access_mode: a.access_mode,
                    passing_mode: a.passing_mode,
                    span: a.span,
                    target_span: a.target_span,
                    marker_span: a.marker_span,
                    following_separator_span: a.following_separator_span,
                })
                .collect()
        }
        fn call<E, F>(c: CandidateCall<E>, map: &mut impl FnMut(E) -> F) -> CandidateCall<F> {
            CandidateCall {
                target: c.target,
                arguments: args(c.arguments, map),
                handling: c.handling,
                requires_external_float_validation: c.requires_external_float_validation,
                error_type: c.error_type,
                span: c.span,
            }
        }
        fn fields<E, F>(
            values: Vec<ConstructorField<E>>,
            map: &mut impl FnMut(E) -> F,
        ) -> Vec<ConstructorField<F>> {
            values
                .into_iter()
                .map(|f| ConstructorField {
                    name: f.name,
                    slot: f.slot,
                    value: map(f.value),
                })
                .collect()
        }
        use CompletedOperation as O;
        match self {
            Self::Constant(v) => O::Constant(v),
            Self::Reference(v) => O::Reference(v),
            Self::Retained(operation) => O::Retained(operation.map_edges(&mut map)),
            Self::Copy {
                root,
                root_type_id,
                projections,
                type_id,
                value_mode,
                span,
            } => O::Copy {
                root,
                root_type_id,
                projections: projections
                    .into_iter()
                    .map(|p| match p {
                        PlaceProjection::Field { field, result_type } => {
                            PlaceProjection::Field { field, result_type }
                        }
                        PlaceProjection::Index { index, result_type } => PlaceProjection::Index {
                            index: map(index),
                            result_type,
                        },
                    })
                    .collect(),
                type_id,
                value_mode,
                span,
            },
            Self::FieldAccess { base, field } => O::FieldAccess {
                base: map(base),
                field,
            },
            Self::NumericUnary { operand, facts } => O::NumericUnary {
                operand: map(operand),
                facts,
            },
            Self::NumericBinary { left, right, facts } => O::NumericBinary {
                left: map(left),
                right: map(right),
                facts,
            },
            Self::LogicalNot { operand } => O::LogicalNot {
                operand: map(operand),
            },
            Self::LogicalBinary {
                left,
                right,
                operation,
            } => O::LogicalBinary {
                left: map(left),
                right: map(right),
                operation,
            },
            Self::Compare { left, right, facts } => O::Compare {
                left: map(left),
                right: map(right),
                facts,
            },
            Self::Range { start, end, facts } => O::Range {
                start: map(start),
                end: map(end),
                facts,
            },
            Self::Call(c) => O::Call(call(c, &mut map)),
            Self::MethodCall { receiver, call: c } => O::MethodCall {
                receiver: map(receiver),
                call: call(c, &mut map),
            },
            Self::CollectionBuiltin {
                receiver,
                operation,
                arguments,
                span,
            } => O::CollectionBuiltin {
                receiver: map(receiver),
                operation,
                arguments: args(arguments, &mut map),
                span,
            },
            Self::MapBuiltin {
                receiver,
                operation,
                receiver_requires_mutable,
                arguments,
                span,
            } => O::MapBuiltin {
                receiver: map(receiver),
                operation,
                receiver_requires_mutable,
                arguments: args(arguments, &mut map),
                span,
            },
            Self::Cast { source, facts } => O::Cast {
                source: map(source),
                facts,
            },
            Self::HandleFallible { value, handling } => O::HandleFallible {
                value: map(value),
                handling,
            },
            Self::PropagateOption { value } => O::PropagateOption { value: map(value) },
            Self::Coerce { value, operation } => O::Coerce {
                value: map(value),
                operation,
            },
            Self::Collection(values) => O::Collection(values.into_iter().map(map).collect()),
            Self::Map(values) => O::Map(
                values
                    .into_iter()
                    .map(|e| MapEntry {
                        key: map(e.key),
                        value: map(e.value),
                    })
                    .collect(),
            ),
            Self::Struct { fields: v } => O::Struct {
                fields: fields(v, &mut map),
            },
            Self::Choice { tag, fields: v } => O::Choice {
                tag,
                fields: fields(v, &mut map),
            },
            Self::RouteConstruct {
                tag,
                fields: v,
                future_input,
            } => O::RouteConstruct {
                tag,
                fields: fields(v, &mut map),
                future_input,
            },
            Self::ValueBlock(v) => O::ValueBlock(v),
            Self::RuntimeTemplate(v) => O::RuntimeTemplate(v),
            Self::RuntimeSlotApplication(v) => O::RuntimeSlotApplication(v),
            Self::Template(v) => O::Template(v),
        }
    }
}
impl CompletedOperation<OwnedExpr> {
    /// Rejects all foreign owners before inspecting indices or appending side records.
    pub(super) fn validate_owned_edges(
        &self,
        builder: &AstExpressionBuilder,
    ) -> Result<(), CompilerError> {
        let mut valid = Ok(());
        self.visit_edges(&mut |id| {
            if valid.is_ok() {
                valid = builder.check_owner(id);
            }
        });
        valid?;
        valid = Ok(());
        self.visit_edges(&mut |id| {
            if valid.is_ok() {
                valid = builder.view(id.id()).map(|_| ());
            }
        });
        valid
    }
    pub(in crate::compiler_frontend::ast::expressions) fn install(
        self,
        builder: &mut AstExpressionBuilder,
        span: Option<SourceSpan>,
    ) -> Result<ExpressionOp<OwnedExpr>, ExpressionConstructionFailure> {
        self.validate_owned_edges(builder)?;
        fn call(
            builder: &mut AstExpressionBuilder,
            c: CandidateCall<OwnedExpr>,
            span: Option<SourceSpan>,
        ) -> Result<CallId, ExpressionConstructionFailure> {
            let arguments = builder.append_arguments(c.arguments, span)?;
            let handling = c
                .handling
                .map(|v| builder.append_handling(v, span))
                .transpose()?;
            builder.append_call(
                ResolvedCall {
                    target: c.target,
                    arguments,
                    handling,
                    requires_external_float_validation: c.requires_external_float_validation,
                    error_type: c.error_type,
                    span: c.span,
                },
                span,
            )
        }
        use ExpressionOp as O;
        Ok(match self {
            Self::Constant(v) => O::Constant(v),
            Self::Reference(v) => O::Reference(v),
            Self::Retained(operation) => operation,
            Self::Copy {
                root,
                root_type_id,
                projections,
                type_id,
                value_mode,
                span: place_span,
            } => {
                let projections = builder.append_place_projections(projections, span)?;
                O::Copy(builder.append_place(
                    PlaceRecord {
                        root,
                        root_type_id,
                        projections,
                        type_id,
                        value_mode,
                        span: place_span,
                    },
                    span,
                )?)
            }
            Self::FieldAccess { base, field } => O::FieldAccess { base, field },
            Self::NumericUnary { operand, facts } => O::NumericUnary {
                operand,
                facts: builder.append_numeric_operation(facts, span)?,
            },
            Self::NumericBinary { left, right, facts } => O::NumericBinary {
                left,
                right,
                facts: builder.append_numeric_operation(facts, span)?,
            },
            Self::LogicalNot { operand } => O::LogicalNot { operand },
            Self::LogicalBinary {
                left,
                right,
                operation,
            } => O::LogicalBinary {
                left,
                right,
                operation,
            },
            Self::Compare { left, right, facts } => O::Compare {
                left,
                right,
                facts: builder.append_comparison_operation(facts, span)?,
            },
            Self::Range { start, end, facts } => O::Range {
                start,
                end,
                facts: builder.append_range_operation(facts, span)?,
            },
            Self::Call(c) => O::Call(call(builder, c, span)?),
            Self::MethodCall { receiver, call: c } => O::MethodCall {
                receiver,
                call: call(builder, c, span)?,
            },
            Self::CollectionBuiltin {
                receiver,
                operation,
                arguments,
                span: call_span,
            } => {
                let arguments = builder.append_arguments(arguments, span)?;
                O::CollectionBuiltin {
                    receiver,
                    call: builder.append_collection_call(
                        CollectionCall {
                            operation,
                            arguments,
                            span: call_span,
                        },
                        span,
                    )?,
                }
            }
            Self::MapBuiltin {
                receiver,
                operation,
                receiver_requires_mutable,
                arguments,
                span: call_span,
            } => {
                let arguments = builder.append_arguments(arguments, span)?;
                O::MapBuiltin {
                    receiver,
                    call: builder.append_map_call(
                        MapCall {
                            operation,
                            receiver_requires_mutable,
                            arguments,
                            span: call_span,
                        },
                        span,
                    )?,
                }
            }
            Self::Cast { source, facts } => O::Cast {
                source,
                facts: builder.append_cast(facts, span)?,
            },
            Self::HandleFallible { value, handling } => O::HandleFallible {
                value,
                handling: builder.append_handling(handling, span)?,
            },
            Self::PropagateOption { value } => O::PropagateOption { value },
            Self::Coerce { value, operation } => O::Coerce { value, operation },
            Self::Collection(v) => O::Collection(builder.append_children(v, span)?),
            Self::Map(v) => O::Map(builder.append_map_entries(v, span)?),
            Self::Struct { fields } => O::Struct {
                fields: builder.append_constructor_fields(fields, span)?,
            },
            Self::Choice { tag, fields } => O::Choice {
                tag,
                fields: builder.append_constructor_fields(fields, span)?,
            },
            Self::RouteConstruct {
                tag,
                fields,
                future_input,
            } => O::RouteConstruct {
                tag,
                fields: builder.append_constructor_fields(fields, span)?,
                future_input,
            },
            Self::ValueBlock(v) => O::ValueBlock(builder.append_value_block(v, span)?),
            Self::RuntimeTemplate(v) => {
                O::RuntimeTemplate(builder.append_runtime_template(v, span)?)
            }
            Self::RuntimeSlotApplication(v) => {
                O::RuntimeSlotApplication(builder.append_runtime_slot_application(v, span)?)
            }
            Self::Template(v) => O::Template(builder.append_template(v, span)?),
        })
    }
}
macro_rules! side_counts {
    ($($field:ident),* $(,)?) => {
        #[derive(Default)]
        pub(super) struct SideCounts {
            $($field: usize,)*
            span: Option<SourceSpan>,
        }
        impl SideCounts {
            pub(super) fn check(
                &self,
                builder: &AstExpressionBuilder,
                rows: usize,
            ) -> Result<(), ExpressionConstructionFailure> {
                checked_extent(builder.len(), rows, builder.limit, self.span, true)?;
                $(
                    checked_extent(
                        builder.storage.$field.len(),
                        self.$field,
                        builder.limit,
                        self.span,
                        false,
                    )?;
                )*
                Ok(())
            }
        }
    };
}
side_counts!(
    children,
    arguments,
    constructor_fields,
    map_entries,
    places,
    projections,
    calls,
    collection_calls,
    map_calls,
    numeric,
    comparisons,
    ranges,
    casts,
    handling,
    value_blocks,
    runtime_templates,
    runtime_slots,
    templates,
    failures,
    occurrences
);

impl SideCounts {
    pub(super) fn add(
        &mut self,
        input: &ScratchCompletedExpressionInput<'_>,
    ) -> Result<(), ExpressionConstructionFailure> {
        self.add_parts(
            &input.operation,
            input.failure.is_some(),
            input.occurrence.is_some(),
            input.span,
        )
    }
    pub(super) fn add_owned(
        &mut self,
        input: &OwnedCompletedExpressionInput,
    ) -> Result<(), ExpressionConstructionFailure> {
        self.add_parts(
            &input.operation,
            input.failure.is_some(),
            input.occurrence.is_some(),
            input.span,
        )
    }
    fn add_parts<E>(
        &mut self,
        op: &CompletedOperation<E>,
        failure: bool,
        occurrence: bool,
        span: Option<SourceSpan>,
    ) -> Result<(), ExpressionConstructionFailure> {
        self.span = span.or(self.span);
        macro_rules! add {
            ($field:ident,$amount:expr) => {
                self.$field = self.$field.checked_add($amount).ok_or_else(|| {
                    ExpressionConstructionFailure::Diagnostic(
                        crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::compiler_capacity_exceeded(
                            AstCapacityResource::ExpressionSideEntries,
                            span,
                        ),
                    )
                })?;
            };
        }
        add!(failures, usize::from(failure));
        add!(occurrences, usize::from(occurrence));
        match op {
            CompletedOperation::Copy { projections, .. } => {
                add!(places, 1);
                add!(projections, projections.len());
            }
            CompletedOperation::NumericUnary { .. } | CompletedOperation::NumericBinary { .. } => {
                add!(numeric, 1);
            }
            CompletedOperation::Compare { .. } => {
                add!(comparisons, 1);
            }
            CompletedOperation::Range { .. } => {
                add!(ranges, 1);
            }
            CompletedOperation::Call(call) | CompletedOperation::MethodCall { call, .. } => {
                add!(calls, 1);
                add!(arguments, call.arguments.len());
                add!(handling, usize::from(call.handling.is_some()));
            }
            CompletedOperation::CollectionBuiltin { arguments, .. } => {
                add!(collection_calls, 1);
                add!(arguments, arguments.len());
            }
            CompletedOperation::MapBuiltin { arguments, .. } => {
                add!(map_calls, 1);
                add!(arguments, arguments.len());
            }
            CompletedOperation::Cast { .. } => {
                add!(casts, 1);
            }
            CompletedOperation::HandleFallible { .. } => {
                add!(handling, 1);
            }
            CompletedOperation::Collection(v) => {
                add!(children, v.len());
            }
            CompletedOperation::Map(v) => {
                add!(map_entries, v.len());
            }
            CompletedOperation::Struct { fields } | CompletedOperation::Choice { fields, .. } => {
                add!(constructor_fields, fields.len());
            }
            CompletedOperation::RouteConstruct { fields, .. } => {
                add!(constructor_fields, fields.len());
            }
            CompletedOperation::ValueBlock(_) => {
                add!(value_blocks, 1);
            }
            CompletedOperation::RuntimeTemplate(_) => {
                add!(runtime_templates, 1);
            }
            CompletedOperation::RuntimeSlotApplication(_) => {
                add!(runtime_slots, 1);
            }
            CompletedOperation::Template(_) => {
                add!(templates, 1);
            }
            _ => (),
        }
        Ok(())
    }
}
