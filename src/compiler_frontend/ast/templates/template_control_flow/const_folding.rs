//! Const loop folding mechanics for template control flow.
//!
//! WHAT: Drives compile-time numeric range iteration and collection iteration for
//!       const-required template loops, and builds the per-iteration fold bindings
//!       that template folding substitutes into body expressions.
//!
//! WHY: These helpers are owned by template control flow (the shape of a const
//!       loop header and its bindings), but they previously lived in the general
//!       template folding module. Moving them here keeps `template_folding.rs`
//!       focused on render-plan emission orchestration while giving const-loop
//!       mechanics a single, focused owner.

use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeEndKind, RangeLoopSpec};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;

/// One binding introduced by const template folding.
///
/// WHAT: maps a loop-bound or option-capture name to the compile-time expression
///       that should replace references to that name during folding.
/// WHY: folding substitutes these bindings into body expressions before passing
///       the resolved expressions to the constant folder or string coercion path.
#[derive(Clone)]
pub(crate) struct TemplateFoldBinding {
    pub(crate) path: PathId,
    pub(crate) value: Expression,
}

// -------------------------
//  Const Range Streaming
// -------------------------

/// Streaming driver for const numeric range loops.
///
/// WHAT: produces one counter at a time while enforcing the iteration limit
///       and hardened range edge-case rules, instead of preallocating a vector.
/// WHY: avoids O(N) upfront allocation for large or unbounded ranges, and keeps
///       validation (by-0, overflow, non-finite, non-progressing) near the range
///       shape rather than spread across separate vector-building helpers.
pub(crate) struct ConstRangeCursor {
    kind: ConstRangeCursorKind,
    emitted_iterations: usize,
    limit: usize,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
}

enum ConstRangeCursorKind {
    Int {
        current: i32,
        end: i32,
        end_kind: RangeEndKind,
        step: i32,
    },
    Float {
        current: f64,
        end: f64,
        end_kind: RangeEndKind,
        step: f64,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum ConstRangeIterationValue {
    Int(i32),
    Float(f64),
}

impl ConstRangeCursor {
    pub(crate) fn new(
        range: &RangeLoopSpec,
        limit: usize,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<Self, TemplateError> {
        let start = const_numeric_expression(&range.start)?;
        let end = const_numeric_expression(&range.end)?;
        let step = range
            .step
            .as_ref()
            .map(const_numeric_expression)
            .transpose()?;
        let step_span = range
            .step
            .as_ref()
            .and_then(|step_expression| step_expression.span);

        match (start, end, step) {
            (ConstNumericValue::Int(start), ConstNumericValue::Int(end), None) => Ok(Self {
                kind: ConstRangeCursorKind::Int {
                    current: start,
                    end,
                    end_kind: range.end_kind,
                    step: if start <= end { 1 } else { -1 },
                },
                emitted_iterations: 0,
                limit,
                span,
            }),

            (
                ConstNumericValue::Int(start),
                ConstNumericValue::Int(end),
                Some(ConstNumericValue::Int(step)),
            ) => {
                let step_magnitude = int_step_magnitude(step, step_span.or(span))?;

                if step_magnitude == 0 {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        span,
                    )
                    .into());
                }

                Ok(Self {
                    kind: ConstRangeCursorKind::Int {
                        current: start,
                        end,
                        end_kind: range.end_kind,
                        step: if start <= end {
                            step_magnitude
                        } else {
                            -step_magnitude
                        },
                    },
                    emitted_iterations: 0,
                    limit,
                    span,
                })
            }

            (start, end, step) => {
                let start = start.as_float();
                let end = end.as_float();

                if !start.is_finite() || !end.is_finite() {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        span,
                    )
                    .into());
                }

                let step_magnitude = match step {
                    None => {
                        return Err(CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                            span,
                        )
                        .into());
                    }
                    Some(step_value) => {
                        let magnitude = step_value.as_float().abs();
                        if magnitude == 0.0 || !magnitude.is_finite() {
                            return Err(CompilerDiagnostic::invalid_template_structure(
                                InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                                span,
                            )
                            .into());
                        }
                        magnitude
                    }
                };

                let step = if start <= end {
                    step_magnitude
                } else {
                    -step_magnitude
                };

                if start + step == start {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        span,
                    )
                    .into());
                }

                Ok(Self {
                    kind: ConstRangeCursorKind::Float {
                        current: start,
                        end,
                        end_kind: range.end_kind,
                        step,
                    },
                    emitted_iterations: 0,
                    limit,
                    span,
                })
            }
        }
    }
    pub(crate) fn iteration_count(&self) -> usize {
        self.emitted_iterations
    }

    pub(crate) fn next_counter(
        &mut self,
    ) -> Result<Option<ConstRangeIterationValue>, TemplateError> {
        match &mut self.kind {
            ConstRangeCursorKind::Int {
                current,
                end,
                end_kind,
                step,
            } => {
                let ascending = *step > 0;
                if !int_range_contains(*current, *end, *end_kind, ascending) {
                    return Ok(None);
                }

                if self.emitted_iterations >= self.limit {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                            limit: self.limit,
                        },
                        self.span,
                    )
                    .into());
                }

                let counter = ConstRangeIterationValue::Int(*current);
                self.emitted_iterations += 1;

                *current = current.checked_add(*step).ok_or_else(|| {
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        self.span,
                    )
                })?;

                Ok(Some(counter))
            }

            ConstRangeCursorKind::Float {
                current,
                end,
                end_kind,
                step,
            } => {
                let ascending = *step > 0.0;
                if !float_range_contains(*current, *end, *end_kind, ascending) {
                    return Ok(None);
                }

                if self.emitted_iterations >= self.limit {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                            limit: self.limit,
                        },
                        self.span,
                    )
                    .into());
                }

                let counter = ConstRangeIterationValue::Float(*current);
                self.emitted_iterations += 1;

                let previous = *current;
                *current += *step;

                if !current.is_finite() || *current == previous {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        self.span,
                    )
                    .into());
                }

                Ok(Some(counter))
            }
        }
    }
}

fn int_step_magnitude(
    step: i32,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Result<i32, TemplateError> {
    step.checked_abs().ok_or_else(|| {
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
            span,
        )
        .into()
    })
}

#[derive(Clone, Copy)]
enum ConstNumericValue {
    Int(i32),
    Float(f64),
}

impl ConstNumericValue {
    fn as_float(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
        }
    }
}

fn const_numeric_expression(expression: &Expression) -> Result<ConstNumericValue, TemplateError> {
    match &expression.kind {
        ExpressionKind::Int(value) => Ok(ConstNumericValue::Int(*value)),
        ExpressionKind::Float(value) => Ok(ConstNumericValue::Float(*value)),
        ExpressionKind::Coerced { value, .. } => const_numeric_expression(value),
        _ => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
            expression.span,
        )
        .into()),
    }
}

fn int_range_contains(current: i32, end: i32, end_kind: RangeEndKind, ascending: bool) -> bool {
    match (ascending, end_kind) {
        (true, RangeEndKind::Exclusive) => current < end,
        (true, RangeEndKind::Inclusive) => current <= end,
        (false, RangeEndKind::Exclusive) => current > end,
        (false, RangeEndKind::Inclusive) => current >= end,
    }
}

fn float_range_contains(current: f64, end: f64, end_kind: RangeEndKind, ascending: bool) -> bool {
    match (ascending, end_kind) {
        (true, RangeEndKind::Exclusive) => current < end,
        (true, RangeEndKind::Inclusive) => current <= end,
        (false, RangeEndKind::Exclusive) => current > end,
        (false, RangeEndKind::Inclusive) => current >= end,
    }
}

// -------------------------
//  Const Collection Items
// -------------------------

pub(crate) fn const_collection_items(
    iterable: &Expression,
) -> Result<Vec<Expression>, TemplateError> {
    match &iterable.kind {
        ExpressionKind::Collection(items) => Ok(items.to_owned()),
        ExpressionKind::Coerced { value, .. } => const_collection_items(value),
        _ => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopSourceNotConst,
            iterable.span,
        )
        .into()),
    }
}

// -------------------------
//  Iteration Bindings
// -------------------------

pub(crate) fn build_range_iteration_bindings(
    bindings: &LoopBindings,
    counter: ConstRangeIterationValue,
    zero_based_index: usize,
    range_provenance: &SyntheticInterfaceProvenance,
) -> Vec<TemplateFoldBinding> {
    let mut fold_bindings = Vec::new();

    if let Some(item) = &bindings.item {
        let value = match counter {
            ConstRangeIterationValue::Int(value) => {
                Expression::int(value, item.value.span, ValueMode::ImmutableOwned)
            }
            ConstRangeIterationValue::Float(value) => {
                Expression::float(value, item.value.span, ValueMode::ImmutableOwned)
            }
        }
        .with_synthetic_interface_provenance(range_provenance.clone());
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value,
        });
    }

    if let Some(index) = &bindings.index {
        fold_bindings.push(TemplateFoldBinding {
            path: index.id,
            value: Expression::int(
                zero_based_index as i32,
                index.value.span,
                ValueMode::ImmutableOwned,
            )
            .with_synthetic_interface_provenance(range_provenance.clone()),
        });
    }

    fold_bindings
}

pub(crate) fn build_collection_iteration_bindings(
    bindings: &LoopBindings,
    item_value: &Expression,
    zero_based_index: usize,
    iterable_provenance: &SyntheticInterfaceProvenance,
) -> Vec<TemplateFoldBinding> {
    let mut fold_bindings = Vec::new();

    if let Some(item) = &bindings.item {
        let mut value = item_value.to_owned();
        value.span = item.value.span;
        value.synthetic_interface_provenance = value
            .synthetic_interface_provenance
            .union(iterable_provenance);
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value,
        });
    }

    if let Some(index) = &bindings.index {
        fold_bindings.push(TemplateFoldBinding {
            path: index.id,
            value: Expression::int(
                zero_based_index as i32,
                index.value.span,
                ValueMode::ImmutableOwned,
            )
            .with_synthetic_interface_provenance(iterable_provenance.clone()),
        });
    }

    fold_bindings
}
