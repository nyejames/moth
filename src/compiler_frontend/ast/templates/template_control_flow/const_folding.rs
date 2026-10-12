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

use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::numeric_operators::range_iteration_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
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
    finished: bool,
    limit: usize,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
}

enum ConstRangeCursorKind {
    /// One shared `i128` cursor for signed and unsigned fixed integers.
    ///
    /// WHAT: the candidate/termination mechanics run once on `i128`; only the
    ///       emitted counter materialises at `scalar`.
    /// WHY: signed, unsigned and widened integer ranges share one stepping path
    ///      instead of owning parallel per-class successors.
    Integer {
        current: i128,
        end: i128,
        end_kind: RangeEndKind,
        step: i128,
        scalar: FixedScalar,
    },
    /// A fixed binary-float cursor that rounds every candidate to its own precision.
    ///
    /// WHY: D15 keeps the range's selected precision, so an `F16` counter advances
    ///      exactly as `F16` arithmetic does rather than widening to `F32`.
    FixedFloat {
        current: f64,
        end: f64,
        end_kind: RangeEndKind,
        step: f64,
        scalar: FixedScalar,
        precision: BinaryFloatPrecision,
    },
}

/// Inputs for the shared integer cursor constructor.
struct ConstRangeIntegerInput<'range> {
    scalar: FixedScalar,
    start: i128,
    end: i128,
    step: Option<(i128, Option<crate::compiler_frontend::source::SourceSpan>)>,
    maximum_step_magnitude: i128,
    range: &'range RangeLoopSpec,
    limit: usize,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
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
        let Some(scalar) = const_range_domain(start, end, step) else {
            return Err(invalid_range_bounds(span));
        };

        Self::new_fixed(scalar, (start, end, step), range, limit, span)
    }
    /// Builds the shared `i128` integer cursor for one exact fixed integer domain.
    ///
    /// WHAT: validates the step magnitude against the domain maximum,
    ///       normalises it to a positive magnitude and signs it by direction.
    ///       Only the emitted counter materialises at the source scalar.
    /// WHY: descending ranges subtract a magnitude without unary negation, the
    ///      inclusive maximum and descending zero endpoints terminate through
    ///      the in-range exit without ever computing an invalid successor.
    fn new_integer(input: ConstRangeIntegerInput<'_>) -> Result<Self, TemplateError> {
        let ConstRangeIntegerInput {
            scalar,
            start,
            end,
            step,
            maximum_step_magnitude,
            range,
            limit,
            span,
        } = input;
        let step_span = step.and_then(|(_, explicit)| explicit).or(span);
        let magnitude = match step {
            Some((value, _)) => value
                .checked_abs()
                .filter(|magnitude| *magnitude <= maximum_step_magnitude)
                .ok_or_else(|| invalid_range_bounds(step_span))?,
            None => 1,
        };
        if magnitude == 0 {
            return Err(invalid_range_bounds(step_span));
        }
        Ok(Self {
            kind: ConstRangeCursorKind::Integer {
                current: start,
                end,
                end_kind: range.end_kind,
                step: if start <= end { magnitude } else { -magnitude },
                scalar,
            },
            emitted_iterations: 0,
            finished: false,
            limit,
            span,
        })
    }
    fn new_fixed(
        scalar: FixedScalar,
        range_values: (FixedScalarValue, FixedScalarValue, Option<FixedScalarValue>),
        range: &RangeLoopSpec,
        limit: usize,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
    ) -> Result<Self, TemplateError> {
        let (start, end, step) = range_values;
        let end_kind = range.end_kind;
        let step_span = range.step.as_ref().and_then(|expression| expression.span);
        let kind = match scalar.class() {
            FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger => {
                let current =
                    fixed_integer_value(start).ok_or_else(|| invalid_range_bounds(span))?;
                let end = fixed_integer_value(end).ok_or_else(|| invalid_range_bounds(span))?;
                let maximum_step_magnitude = fixed_integer_max_magnitude(scalar)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let explicit_step = step
                    .map(|value| {
                        fixed_integer_value(value)
                            .ok_or_else(|| invalid_range_bounds(step_span.or(span)))
                    })
                    .transpose()?
                    .map(|magnitude| (magnitude, step_span));
                return Self::new_integer(ConstRangeIntegerInput {
                    scalar,
                    start: current,
                    end,
                    step: explicit_step,
                    maximum_step_magnitude,
                    range,
                    limit,
                    span,
                });
            }
            FixedScalarClass::BinaryFloat => {
                // D15: every candidate rounds to the range's own precision before the
                // endpoint and no-progress checks, so `F16` advances exactly as `F16`
                // arithmetic does instead of widening through an `F32` floor.
                let precision = scalar
                    .binary_float_precision()
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let current = fixed_float_value(start, precision)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let end =
                    fixed_float_value(end, precision).ok_or_else(|| invalid_range_bounds(span))?;
                if !current.is_finite() || !end.is_finite() {
                    return Err(invalid_range_bounds(span));
                }
                let step_magnitude = match step {
                    Some(value) => fixed_float_value(value, precision)
                        .map(f64::abs)
                        .filter(|value| *value != 0.0 && value.is_finite())
                        .ok_or_else(|| invalid_range_bounds(step_span.or(span)))?,
                    None => return Err(invalid_range_bounds(span)),
                };
                ConstRangeCursorKind::FixedFloat {
                    current,
                    end,
                    end_kind,
                    step: if current <= end {
                        step_magnitude
                    } else {
                        -step_magnitude
                    },
                    scalar,
                    precision,
                }
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
                let ascending = *step > 0;
                if !fixed_integer_range_contains(*current, *end, *end_kind, ascending) {
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

                // Only counter materialisation returns to the source scalar: the
                // candidate/termination mechanics above run once on `i128`.
                // The in-range exit below never computes an invalid successor,
                // so the inclusive maximum and descending zero terminate.
                let counter = fixed_integer_scalar(*scalar, *current)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
                self.emitted_iterations += 1;

                let next = current
                    .checked_add(*step)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
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

                let counter = FixedScalarValue::binary_float(*scalar, *current)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
                self.emitted_iterations += 1;

                // An inclusive endpoint has already been emitted; do not try to
                // advance past it, since a rounded stalled update could look in-range.
                if *current == *end && *end_kind == RangeEndKind::Inclusive {
                    self.finished = true;
                    return Ok(Some(counter));
                }

                // Round every candidate at the range's own precision before the
                // endpoint and no-progress checks, so narrow counters stall exactly
                // where their arithmetic does.
                let previous = *current;
                let next = precision.round(*current + *step);
                if !float_range_contains(next, *end, *end_kind, ascending) {
                    self.finished = true;
                } else {
                    if !next.is_finite() || next == previous {
                        return Err(invalid_range_bounds(self.span));
                    }
                    *current = next;
                }

                Ok(Some(counter))
            }
        }
    }
}

/// The integer payload of a fixed integer value widened to `i128`, exact for `U64`.
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

/// The exact counter domain of one range header, or `None` when the operands have no shared
/// exact numeric domain.
///
/// WHY: const folding, AST typing and HIR lowering share one selector; `Dec`, `Byte` and
///      integer/float mixtures have no range counter domain. `range_iteration_domain` rejects
///      every `Dec` operand before consulting the fixed operator table, so a number domain
///      cannot appear here.
fn const_range_domain(
    start: FixedScalarValue,
    end: FixedScalarValue,
    step: Option<FixedScalarValue>,
) -> Option<FixedScalar> {
    match range_iteration_domain(
        NumericScalar::Fixed(start.scalar()),
        NumericScalar::Fixed(end.scalar()),
        step.map(|value| NumericScalar::Fixed(value.scalar())),
    )? {
        NumericScalar::Fixed(scalar) => Some(scalar),
        NumericScalar::Number(_) => None,
    }
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
fn fixed_integer_max_magnitude(scalar: FixedScalar) -> Option<i128> {
    match scalar.class() {
        FixedScalarClass::SignedInteger => scalar
            .signed_range()
            .map(|(_, maximum)| i128::from(maximum)),
        FixedScalarClass::UnsignedInteger => scalar.unsigned_max().map(i128::from),
        FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
    }
}

fn invalid_range_bounds(
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> TemplateError {
    CompilerDiagnostic::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
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
fn const_numeric_expression(expression: &Expression) -> Result<FixedScalarValue, TemplateError> {
    match &expression.kind {
        ExpressionKind::FixedScalar(value) if value.scalar() != FixedScalar::Byte => Ok(*value),
        ExpressionKind::Coerced { value, .. } => const_numeric_expression(value),
        _ => Err(invalid_range_bounds(expression.span)),
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
) -> Result<&[Expression], TemplateError> {
    match &iterable.kind {
        ExpressionKind::Collection(items) => Ok(items),
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
    counter: FixedScalarValue,
    zero_based_index: usize,
    range_provenance: &SyntheticInterfaceProvenance,
) -> Vec<TemplateFoldBinding> {
    let mut fold_bindings = Vec::new();

    if let Some(item) = &bindings.item {
        let value = Expression::fixed_scalar(counter, item.value.span, ValueMode::ImmutableOwned);
        debug_assert_eq!(
            value.type_id, item.value.type_id,
            "fixed range cursor must produce the declared binding type"
        );
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value: value.with_synthetic_interface_provenance(range_provenance.clone()),
        });
    }

    if let Some(index) = &bindings.index {
        fold_bindings.push(iteration_index_binding(
            index,
            zero_based_index,
            range_provenance,
        ));
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
        fold_bindings.push(iteration_index_binding(
            index,
            zero_based_index,
            iterable_provenance,
        ));
    }

    fold_bindings
}

/// Builds the positional index binding for one const loop iteration.
///
/// WHY: range and collection loops share one I32 positional index domain, so both
///      iteration paths materialise the same counter type.
fn iteration_index_binding(
    index: &Declaration,
    zero_based_index: usize,
    provenance: &SyntheticInterfaceProvenance,
) -> TemplateFoldBinding {
    // Iteration indices stay within the const-loop expansion limit, well below
    // `i32::MAX`, so this checked conversion cannot fail in practice.
    let position = i32::try_from(zero_based_index).unwrap_or(i32::MAX);
    let value = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, i64::from(position))
            .expect("const loop positions fit I32"),
        index.value.span,
        ValueMode::ImmutableOwned,
    );
    TemplateFoldBinding {
        path: index.id,
        value: value.with_synthetic_interface_provenance(provenance.clone()),
    }
}
