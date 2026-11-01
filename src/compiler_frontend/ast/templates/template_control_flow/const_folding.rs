//! Const loop folding mechanics for template control flow.
//!
//! WHAT: Drives compile-time numeric range iteration and collection iteration for
//!       const-required template loops, and builds the per-iteration fold bindings
//!       that template folding substitutes into body expressions.
//!
//! WHY: const-loop header and binding mechanics have a focused owner, leaving
//! `template_folding.rs` responsible for fold orchestration and final value-boundary policy.

use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, OccurrenceFacts,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;

/// One binding introduced by const template folding.
///
/// WHAT: maps a loop-bound or option-capture name to the compile-time expression
///       that should replace references to that name during folding.
/// WHY: folding substitutes these bindings into body expressions before passing
///       the resolved expressions to the constant folder or string coercion path.
#[derive(Clone)]
pub(crate) struct TemplateFoldBinding {
    pub(crate) path: PathId,
    pub(crate) value: ExprId,
}

// -------------------------
//  Const Range Streaming
// -------------------------

/// Streaming driver for const numeric range loops.
///
/// WHAT: produces one counter at a time while enforcing the iteration limit and the hardened
///       range edge cases, instead of preallocating a vector.
/// WHY: avoids O(N) upfront allocation for large or unbounded ranges, and keeps validation
///      (zero step, non-finite and non-progressing candidates) near the range shape rather than
///      spread across separate vector-building helpers.
pub(crate) struct ConstRangeCursor {
    kind: ConstRangeCursorKind,
    emitted_iterations: usize,
    finished: bool,
    limit: usize,
    span: Option<SourceSpan>,
    /// The authored step position, or `None` when the step is the implicit `+1`.
    step_span: Option<SourceSpan>,
}

enum ConstRangeCursorKind {
    /// One shared `i128` cursor for every fixed integer counter domain.
    ///
    /// WHAT: the candidate and termination mechanics run once on `i128`; only the emitted
    ///       counter materialises at `scalar`.
    /// WHY: an integer step keeps its own signed width, independent of the counter, so exact
    ///      `i128` arithmetic covers signed minima, unsigned maxima and narrow counters on one
    ///      stepping path without magnitude normalisation or narrowing.
    Integer {
        current: i128,
        end: i128,
        end_kind: RangeEndKind,
        /// The authored signed step widened exactly, or the independent `+1` default. Never zero.
        step: i128,
        scalar: FixedScalar,
    },
    /// A fixed binary-float cursor that rounds every candidate to its own precision.
    ///
    /// WHY: the range's selected precision is kept, so an `F16` counter advances exactly as `F16`
    ///      arithmetic does rather than widening to `F32`.
    FixedFloat {
        current: f64,
        end: f64,
        end_kind: RangeEndKind,
        /// The authored step converted to the counter precision. Never zero.
        step: f64,
        scalar: FixedScalar,
        precision: BinaryFloatPrecision,
    },
}

/// The resolved const values of a range header's three operand slots.
///
/// WHAT: the start, end and optional step after transparent coercion unwrapping.
/// WHY: both counter classes read the same three slots, so one small record keeps the class
///      constructors from repeating the driver's operand plumbing.
struct ConstRangeOperands {
    start: FixedScalarValue,
    end: FixedScalarValue,
    /// The authored step. `None` keeps the independent `+1` default.
    step: Option<FixedScalarValue>,
}

impl ConstRangeCursor {
    pub(crate) fn new(
        range: &RangeLoopSpec,
        expressions: &AstExpressionBuilder,
        constants: &ConstValueStore,
        limit: usize,
        span: Option<SourceSpan>,
        step_span: Option<SourceSpan>,
    ) -> Result<Self, TemplateError> {
        let start = const_numeric_expression(expressions.view(range.start)?, constants)?;
        let end = const_numeric_expression(expressions.view(range.end)?, constants)?;
        let step = range
            .step
            .map(|id| const_numeric_expression(expressions.view(id)?, constants))
            .transpose()?;
        // The header selected its iteration domain from the bound types before any operand value
        // substitution, and those substitutions preserve operand types, so the spec's domain is
        // this cursor's exact counter domain.
        let NumericScalar::Fixed(scalar) = range.domain else {
            return Err(invalid_range_bounds(span));
        };
        let operands = ConstRangeOperands { start, end, step };
        let kind = match scalar.class() {
            FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger => {
                integer_cursor_kind(scalar, operands, range.end_kind, step_span, span)?
            }
            FixedScalarClass::BinaryFloat => {
                fixed_float_cursor_kind(scalar, operands, range.end_kind, step_span, span)?
            }
            // `Byte` has no range counter domain: the shared selector rejects it and
            // `const_numeric_expression` keeps it out of range operand materialisation.
            FixedScalarClass::Octet => return Err(invalid_range_bounds(span)),
        };

        Ok(Self {
            kind,
            emitted_iterations: 0,
            finished: false,
            limit,
            span,
            step_span,
        })
    }

    pub(crate) fn iteration_count(&self) -> usize {
        self.emitted_iterations
    }

    pub(crate) fn next_counter(&mut self) -> Result<Option<FixedScalarValue>, TemplateError> {
        if self.finished {
            return Ok(None);
        }

        match &mut self.kind {
            ConstRangeCursorKind::Integer {
                current,
                end,
                end_kind,
                step,
                scalar,
            } => {
                // The step sign is the only direction fact: the bounds never infer it and the
                // step is never normalised to a magnitude.
                let ascending = *step > 0;
                if !fixed_integer_range_contains(*current, *end, *end_kind, ascending) {
                    self.finished = true;
                    return Ok(None);
                }

                if self.emitted_iterations >= self.limit {
                    return Err(templates::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                            limit: self.limit,
                        },
                        self.span,
                    )
                    .into());
                }

                // Only counter materialisation returns to the source scalar: the
                // candidate/termination mechanics run once on `i128`.
                let counter = fixed_integer_scalar(*scalar, *current)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
                self.emitted_iterations += 1;

                // An inclusive endpoint is the last value, so no successor is computed past it.
                if *current == *end && *end_kind == RangeEndKind::Inclusive {
                    self.finished = true;
                    return Ok(Some(counter));
                }

                // `i128` holds the sum exactly for every fixed-width counter and signed step, so
                // a huge negative step simply overshoots the bound and the range stops.
                let next = *current + *step;
                if fixed_integer_range_contains(next, *end, *end_kind, ascending) {
                    *current = next;
                } else {
                    self.finished = true;
                }

                Ok(Some(counter))
            }

            ConstRangeCursorKind::FixedFloat {
                current,
                end,
                end_kind,
                step,
                scalar,
                precision,
            } => {
                // The signed step selects direction; a wrong-direction range is empty.
                let ascending = *step > 0.0;
                if !float_range_contains(*current, *end, *end_kind, ascending) {
                    self.finished = true;
                    return Ok(None);
                }

                if self.emitted_iterations >= self.limit {
                    return Err(templates::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                            limit: self.limit,
                        },
                        self.span,
                    )
                    .into());
                }

                let counter = FixedScalarValue::binary_float(*scalar, *current)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
                self.emitted_iterations += 1;

                // An inclusive endpoint has already been emitted; do not try to
                // advance past it, since a rounded stalled update could look in-range.
                if *current == *end && *end_kind == RangeEndKind::Inclusive {
                    self.finished = true;
                    return Ok(Some(counter));
                }

                // Add the signed step and round at the range's own precision before the
                // endpoint and no-progress checks, so narrow counters stall exactly where
                // their arithmetic does.
                let previous = *current;
                let next = precision.round(previous + *step);
                if !next.is_finite() || !float_range_contains(next, *end, *end_kind, ascending) {
                    self.finished = true;
                    return Ok(Some(counter));
                }
                // An in-range candidate that rounds back to the current counter would make the
                // range stall without ever reaching its endpoint.
                if next == previous {
                    return Err(invalid_range_step_no_progress(self.step_span));
                }
                *current = next;

                Ok(Some(counter))
            }
        }
    }
}

/// Builds the shared `i128` integer cursor kind for one exact fixed integer domain.
///
/// WHAT: widens the bounds and the authored step to `i128` without narrowing the step to the
///       counter, and rejects a const-known zero step before any bound is examined.
/// WHY: exact arithmetic preserves signed minima and unsigned maxima, and only the emitted
///      counter materialises at the counter scalar.
fn integer_cursor_kind(
    scalar: FixedScalar,
    operands: ConstRangeOperands,
    end_kind: RangeEndKind,
    step_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> Result<ConstRangeCursorKind, TemplateError> {
    let current = fixed_integer_value(operands.start).ok_or_else(|| invalid_range_bounds(span))?;
    let end = fixed_integer_value(operands.end).ok_or_else(|| invalid_range_bounds(span))?;
    let step_span = step_span.or(span);
    // The authored step keeps its own signed width and is never narrowed to the counter. The
    // omitted step is the independent signed `+1` default, not a counter-typed value.
    let step = match operands.step {
        Some(value) => fixed_integer_value(value).ok_or_else(|| invalid_range_bounds(step_span))?,
        None => 1,
    };
    // A const-known zero step fails even when the bounds would visit nothing.
    if step == 0 {
        return Err(invalid_range_step_is_zero(step_span));
    }

    Ok(ConstRangeCursorKind::Integer {
        current,
        end,
        end_kind,
        step,
        scalar,
    })
}

/// Builds the fixed binary-float cursor kind for one exact float counter domain.
///
/// WHAT: rounds every operand to the counter precision, rejects a missing, zero or non-finite
///       step and keeps the signed step unchanged.
/// WHY: candidates round at the counter's own precision, and the step's sign alone selects
///      direction rather than the bounds overriding it.
fn fixed_float_cursor_kind(
    scalar: FixedScalar,
    operands: ConstRangeOperands,
    end_kind: RangeEndKind,
    step_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> Result<ConstRangeCursorKind, TemplateError> {
    // Every candidate rounds to the range's own precision before the endpoint and no-progress
    // checks, so `F16` advances exactly as `F16` arithmetic does instead of widening through an
    // `F32` floor.
    let precision = scalar
        .binary_float_precision()
        .ok_or_else(|| invalid_range_bounds(span))?;
    let current =
        fixed_float_value(operands.start, precision).ok_or_else(|| invalid_range_bounds(span))?;
    let end =
        fixed_float_value(operands.end, precision).ok_or_else(|| invalid_range_bounds(span))?;
    if !current.is_finite() || !end.is_finite() {
        return Err(invalid_range_bounds(span));
    }
    let step_span = step_span.or(span);
    let step = match operands.step {
        // The step was converted to the counter precision before folding, so rounding here only
        // re-applies that owned conversion.
        Some(value) => {
            fixed_float_value(value, precision).ok_or_else(|| invalid_range_bounds(step_span))?
        }
        // A binary-float range always requires an explicit step.
        None => return Err(invalid_range_bounds(span)),
    };
    // `-0.0 == 0.0`, so a folded negative zero fails exactly like an authored integer zero, even
    // for equal endpoints that would otherwise visit nothing.
    if step == 0.0 {
        return Err(invalid_range_step_is_zero(step_span));
    }
    // Compile-time evaluation rejects non-finite constants, so this is a defensive bound check
    // rather than a source-reachable zero or no-progress case.
    if !step.is_finite() {
        return Err(invalid_range_bounds(step_span));
    }

    Ok(ConstRangeCursorKind::FixedFloat {
        current,
        end,
        end_kind,
        step,
        scalar,
        precision,
    })
}

/// The integer payload of a fixed integer value widened to `i128`, exact for every width
/// including `U64`.
fn fixed_integer_value(value: FixedScalarValue) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

/// The binary-float payload rounded once to the cursor precision.
fn fixed_float_value(value: FixedScalarValue, precision: BinaryFloatPrecision) -> Option<f64> {
    value.as_f64().map(|value| precision.round(value))
}

fn fixed_integer_scalar(scalar: FixedScalar, value: i128) -> Option<FixedScalarValue> {
    match scalar.class() {
        FixedScalarClass::SignedInteger => {
            FixedScalarValue::signed(scalar, i64::try_from(value).ok()?)
        }
        FixedScalarClass::UnsignedInteger => {
            FixedScalarValue::unsigned(scalar, u64::try_from(value).ok()?)
        }
        FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
    }
}

fn invalid_range_bounds(span: Option<SourceSpan>) -> TemplateError {
    templates::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
        span,
    )
    .into()
}

/// The const-time zero step of a range or collection loop.
///
/// WHY: a const-known zero cannot progress, and it is a step fact rather than a non-constant
///      bound fact, so it keeps its own reason at the authored step position.
fn invalid_range_step_is_zero(span: Option<SourceSpan>) -> TemplateError {
    templates::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateLoopStepIsZero,
        span,
    )
    .into()
}

/// A const float range step that rounds back to the current counter.
///
/// WHY: the range cannot reach its endpoint, and the authored step position owns the repair.
fn invalid_range_step_no_progress(span: Option<SourceSpan>) -> TemplateError {
    templates::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateLoopStepNoProgress,
        span,
    )
    .into()
}

fn fixed_integer_range_contains(
    current: i128,
    end: i128,
    end_kind: RangeEndKind,
    ascending: bool,
) -> bool {
    match (ascending, end_kind) {
        (true, RangeEndKind::Exclusive) => current < end,
        (true, RangeEndKind::Inclusive) => current <= end,
        (false, RangeEndKind::Exclusive) => current > end,
        (false, RangeEndKind::Inclusive) => current >= end,
    }
}

/// The exact fixed value of one const range operand.
///
/// WHAT: accepts a materialised fixed scalar through transparent coercion wrappers.
/// WHY: `Byte` and `Dec` have no range counter domain, so they take the ordinary
///      "bounds are not const" diagnostic instead of selecting a counter.
fn const_numeric_expression(
    expression: ExpressionView<'_>,
    constants: &ConstValueStore,
) -> Result<FixedScalarValue, TemplateError> {
    const_numeric_expression_value(expression, constants)?
        .ok_or_else(|| invalid_range_bounds(expression.span()))
}

/// The materialised fixed scalar behind one const operand, or `None` when it is not const.
///
/// WHAT: accepts a constant through transparent coercion wrappers.
/// WHY: range operands and collection steps both read one const numeric slot but own different
///      diagnostics, so value extraction stays free of the caller's reason.
fn const_numeric_expression_value(
    expression: ExpressionView<'_>,
    constants: &ConstValueStore,
) -> Result<Option<FixedScalarValue>, TemplateError> {
    match expression.operation() {
        ExpressionOperationView::Constant(id) => Ok(const_numeric_payload(id, constants)),
        ExpressionOperationView::Coerce { value, .. } => {
            const_numeric_expression_value(value, constants)
        }
        _ => Ok(None),
    }
}

fn const_numeric_payload(
    id: crate::compiler_frontend::ast::const_values::store::ConstValueId,
    constants: &ConstValueStore,
) -> Option<FixedScalarValue> {
    match constants.payload(id)? {
        ConstValuePayload::FixedScalar(value) if value.scalar() != FixedScalar::Byte => {
            Some(*value)
        }
        ConstValuePayload::Coerced(value) => const_numeric_payload(*value, constants),
        _ => None,
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
//  Const Collection Stepping
// -------------------------

/// The const-known signed integer step of one collection header.
///
/// WHAT: widens the authored step exactly and keeps its own type independent of the source.
/// WHY: the traversal only needs the exact value, and a runtime-only step must keep the
///      const-required diagnostic rather than being defaulted or narrowed to the source index.
///      `i128` holds every signed fixed-width step exactly, including `I64::MIN`, so no consumer
///      needs a signed `abs`.
pub(crate) fn const_collection_step(
    step: ExprId,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    span: Option<SourceSpan>,
) -> Result<i128, TemplateError> {
    let value = const_numeric_expression_value(expressions.view(step)?, constants)?
        .ok_or_else(|| invalid_collection_step(span))?;
    let signed = match value.scalar().class() {
        // The AST admits only signed fixed-width integer steps; anything else is a broken
        // invariant that still reports as a non-const step instead of a panic.
        FixedScalarClass::SignedInteger => value.as_i64(),
        FixedScalarClass::UnsignedInteger
        | FixedScalarClass::BinaryFloat
        | FixedScalarClass::Octet => None,
    }
    .ok_or_else(|| invalid_collection_step(span))?;
    Ok(i128::from(signed))
}

/// Streaming driver for const collection loops.
///
/// WHAT: visits the source's actual indices under the authored signed step and enforces the
///       visited-iteration limit.
/// WHY: enumeration would report wrong indices for a stride and would apply the limit to source
///      positions. Exact `i128` arithmetic also keeps an overshoot from ever materialising an
///      invalid index, including for `I64::MIN`.
pub(crate) struct ConstCollectionCursor {
    /// The next source index to visit, exact so an overshoot never wraps or narrows.
    next_index: i128,
    step: i128,
    /// The source length widened once; every bound test compares against it.
    source_len: i128,
    visited_iterations: usize,
    limit: usize,
    span: Option<SourceSpan>,
}

impl ConstCollectionCursor {
    pub(crate) fn new(
        source_len: usize,
        step: i128,
        limit: usize,
        step_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<Self, TemplateError> {
        // A const-known zero step fails even when the source is empty or the step would visit
        // nothing. Range and collection loops share the same const-time zero-step reason.
        if step == 0 {
            return Err(invalid_range_step_is_zero(step_span.or(span)));
        }
        let source_len = i128::try_from(source_len).map_err(|_| {
            crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "Const collection source length exceeded I128.",
            )
        })?;
        Ok(Self {
            // The step sign selects the start: positive steps begin at the first source index,
            // negative steps at the last. An empty source starts outside the bounds either way.
            next_index: if step > 0 { 0 } else { source_len - 1 },
            step,
            source_len,
            visited_iterations: 0,
            limit,
            span,
        })
    }

    pub(crate) fn next_index(&mut self) -> Result<Option<usize>, TemplateError> {
        if self.next_index < 0 || self.next_index >= self.source_len {
            return Ok(None);
        }
        if self.visited_iterations >= self.limit {
            return Err(templates::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                    limit: self.limit,
                },
                self.span,
            )
            .into());
        }

        let index = usize::try_from(self.next_index).map_err(|_| {
            crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "Const collection index left the source bounds.",
            )
        })?;
        self.visited_iterations += 1;
        // Exact `i128` addition: a huge step overshoots the bound and the next call stops without
        // constructing an invalid index.
        self.next_index += self.step;
        Ok(Some(index))
    }
}

fn invalid_collection_step(span: Option<SourceSpan>) -> TemplateError {
    templates::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateLoopStepNotConst,
        span,
    )
    .into()
}

// -------------------------
//  Const Collection Items
// -------------------------

pub(crate) fn const_collection_items(
    iterable: ExprId,
    expressions: &mut AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<Vec<ExprId>, TemplateError> {
    let iterable = expressions.view(iterable)?;
    let span = iterable.span();
    let mode = iterable.value_mode();
    let values = match iterable.operation() {
        ExpressionOperationView::Collection(items) => {
            return Ok(items.iter().map(|item| item.id()).collect());
        }
        ExpressionOperationView::Coerce { value, .. } => {
            let value = value.id();
            return const_collection_items(value, expressions, constants);
        }
        ExpressionOperationView::Constant(id) => const_collection_payload(id, constants),
        _ => None,
    };
    let Some(values) = values else {
        return Err(templates::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopSourceNotConst,
            span,
        )
        .into());
    };
    let mut items = Vec::with_capacity(values.len());
    for &value in values {
        let metadata = constants.metadata(value).ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "Missing const collection item.",
            )
        })?;
        items.push(
            expressions
                .constant(value, metadata.type_id, span, mode)?
                .id(),
        );
    }
    Ok(items)
}

fn const_collection_payload(
    id: crate::compiler_frontend::ast::const_values::store::ConstValueId,
    constants: &ConstValueStore,
) -> Option<&[crate::compiler_frontend::ast::const_values::store::ConstValueId]> {
    match constants.payload(id)? {
        ConstValuePayload::Collection(items) => Some(items),
        ConstValuePayload::Coerced(value) => const_collection_payload(*value, constants),
        _ => None,
    }
}

// -------------------------
//  Iteration Bindings
// -------------------------

pub(crate) fn build_range_iteration_bindings(
    bindings: &LoopBindings,
    counter: FixedScalarValue,
    zero_based_index: usize,
    range_provenance: &SyntheticInterfaceProvenance,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> Result<Vec<TemplateFoldBinding>, TemplateError> {
    let mut fold_bindings = Vec::with_capacity(2);
    if let Some(item) = &bindings.item {
        let constant = constants.insert_resolved(
            item.type_id,
            ConstValuePayload::FixedScalar(counter),
            item.binding_span,
        )?;
        let value = iteration_constant(expressions, constant, item, range_provenance.clone())?;
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value,
        });
    }
    if let Some(index) = &bindings.index {
        fold_bindings.push(iteration_index_binding(
            index,
            zero_based_index,
            range_provenance,
            expressions,
            constants,
        )?);
    }
    Ok(fold_bindings)
}

pub(crate) fn build_collection_iteration_bindings(
    bindings: &LoopBindings,
    item_value: ExprId,
    source_index: usize,
    iterable_provenance: &SyntheticInterfaceProvenance,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> Result<Vec<TemplateFoldBinding>, TemplateError> {
    let mut fold_bindings = Vec::with_capacity(2);
    if let Some(item) = &bindings.item {
        let mut input = expressions
            .view(item_value)?
            .to_completed_input(expressions)?;
        input.span = item.binding_span;
        let provenance = input.occurrence.as_ref().map_or_else(
            || iterable_provenance.clone(),
            |facts| facts.provenance.union(iterable_provenance),
        );
        input.occurrence = Some(OccurrenceFacts {
            provenance,
            diagnostic_spelling: None,
        });
        let value = expressions.commit_replacement(input)?;
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value: value.id(),
        });
    }
    if let Some(index) = &bindings.index {
        fold_bindings.push(iteration_index_binding(
            index,
            source_index,
            iterable_provenance,
            expressions,
            constants,
        )?);
    }
    Ok(fold_bindings)
}

fn iteration_constant(
    expressions: &mut AstExpressionBuilder,
    constant: crate::compiler_frontend::ast::const_values::store::ConstValueId,
    declaration: &Declaration,
    provenance: SyntheticInterfaceProvenance,
) -> Result<ExprId, TemplateError> {
    let mut input = expressions.completed_input(
        crate::compiler_frontend::ast::expressions::expression_store::ExpressionOp::Constant(
            constant,
        ),
        crate::compiler_frontend::ast::expressions::expression_store::SuccessShape::One(
            declaration.type_id,
        ),
        declaration.binding_span,
        ValueMode::ImmutableOwned,
    )?;
    input.occurrence = Some(OccurrenceFacts {
        provenance,
        diagnostic_spelling: None,
    });
    Ok(expressions.commit_replacement(input)?.id())
}

fn iteration_index_binding(
    index: &Declaration,
    position: usize,
    provenance: &SyntheticInterfaceProvenance,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> Result<TemplateFoldBinding, TemplateError> {
    let position = i32::try_from(position).map_err(|_| {
        crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Const loop position exceeded I32.",
        )
    })?;
    let scalar = FixedScalarValue::signed(FixedScalar::I32, i64::from(position))
        .expect("checked I32 position");
    let constant = constants.insert_resolved(
        index.type_id,
        ConstValuePayload::FixedScalar(scalar),
        index.binding_span,
    )?;
    let value = iteration_constant(expressions, constant, index, provenance.clone())?;
    Ok(TemplateFoldBinding {
        path: index.id,
        value,
    })
}
