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
use crate::compiler_frontend::datatypes::numeric_operators::range_iteration_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::FloatPrecision;

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
    /// Boundary `Float` precision for mixed/int-widened float iteration.
    ///
    /// WHY: `Int` bounds and steps widen and `Float` counters accumulate at the
    ///      compilation boundary precision, so Bits32 counters advance exactly
    ///      as Bits32 literals do instead of drifting in f64.
    float_precision: FloatPrecision,
}
enum ConstRangeCursorKind {
    Int {
        current: i64,
        end: i64,
        end_kind: RangeEndKind,
        step: i64,
    },
    Integer {
        current: i128,
        end: i128,
        end_kind: RangeEndKind,
        step: i128,
        output: ConstRangeIntegerOutput,
    },
    Float {
        current: f64,
        end: f64,
        end_kind: RangeEndKind,
        step: f64,
    },
    FixedFloat {
        current: f64,
        end: f64,
        end_kind: RangeEndKind,
        step: f64,
        scalar: FixedScalar,
        precision: BinaryFloatPrecision,
    },
}

/// Which identity a shared integer cursor materialises at counter emission.
///
/// WHAT: the candidate/termination mechanics run once on `i128`; only the
///       emitted counter keeps the source identity (`Uint` payload versus a
///       fixed-width scalar).
/// WHY: `Uint` ranges reuse the existing unsigned candidate/termination logic
///      instead of owning a parallel stepping path.
#[derive(Clone, Copy)]
enum ConstRangeIntegerOutput {
    Uint,
    Fixed(FixedScalar),
}

/// Inputs for the shared integer cursor constructor.
struct ConstRangeIntegerInput<'range> {
    output: ConstRangeIntegerOutput,
    start: i128,
    end: i128,
    step: Option<(i128, Option<crate::compiler_frontend::source::SourceSpan>)>,
    maximum_step_magnitude: i128,
    range: &'range RangeLoopSpec,
    limit: usize,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    float_precision: FloatPrecision,
}
pub(crate) enum ConstRangeIterationValue {
    Int(i64),
    Uint(u64),
    Float(f64),
    Fixed(FixedScalarValue),
}

impl ConstRangeCursor {
    pub(crate) fn new(
        range: &RangeLoopSpec,
        limit: usize,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
        float_precision: FloatPrecision,
    ) -> Result<Self, TemplateError> {
        let start = const_numeric_expression(&range.start)?;
        let end = const_numeric_expression(&range.end)?;
        let step = range
            .step
            .as_ref()
            .map(const_numeric_expression)
            .transpose()?;
        let domain =
            const_range_domain(start, end, step).ok_or_else(|| invalid_range_bounds(span))?;
        if let NumericScalar::Fixed(scalar) = domain {
            return Self::new_fixed(
                scalar,
                (start, end, step),
                range,
                limit,
                span,
                float_precision,
            );
        }

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
                finished: false,
                limit,
                span,
                float_precision,
            }),
            (ConstNumericValue::Uint(start), ConstNumericValue::Uint(end), None) => {
                Self::new_integer(ConstRangeIntegerInput {
                    output: ConstRangeIntegerOutput::Uint,
                    start: i128::from(start),
                    end: i128::from(end),
                    step: None,
                    maximum_step_magnitude: i128::from(u64::MAX),
                    range,
                    limit,
                    span,
                    float_precision,
                })
            }

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
                    finished: false,
                    limit,
                    span,
                    float_precision,
                })
            }
            (
                ConstNumericValue::Uint(start),
                ConstNumericValue::Uint(end),
                Some(ConstNumericValue::Uint(step)),
            ) => Self::new_integer(ConstRangeIntegerInput {
                output: ConstRangeIntegerOutput::Uint,
                start: i128::from(start),
                end: i128::from(end),
                step: Some((i128::from(step), step_span)),
                maximum_step_magnitude: i128::from(u64::MAX),
                range,
                limit,
                span,
                float_precision,
            }),
            (start, end, step) => {
                // Widened `Int` bounds convert at the boundary precision (never via an
                // f64 intermediate under Bits32) and literal `Float` bounds round once,
                // so mixed ranges and counters start exactly where the boundary types them.
                let start = start
                    .to_float(float_precision)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let end = end
                    .to_float(float_precision)
                    .ok_or_else(|| invalid_range_bounds(span))?;
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
                        let magnitude = step_value
                            .to_float(float_precision)
                            .ok_or_else(|| invalid_range_bounds(step_span.or(span)))?
                            .abs();
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

                Ok(Self {
                    kind: ConstRangeCursorKind::Float {
                        current: start,
                        end,
                        end_kind: range.end_kind,
                        step,
                    },
                    emitted_iterations: 0,
                    finished: false,
                    limit,
                    span,
                    float_precision,
                })
            }
        }
    }
    /// Builds a shared `i128` integer cursor for fixed integers and `Uint`.
    ///
    /// WHAT: validates the step magnitude against the caller's maximum,
    ///       normalises it to a positive magnitude and signs it by direction.
    ///       Only the emitted counter keeps the source identity.
    /// WHY: descending ranges subtract a magnitude without unary negation, the
    ///      inclusive maximum and descending zero endpoints terminate through
    ///      the in-range exit without ever computing an invalid successor.
    fn new_integer(input: ConstRangeIntegerInput<'_>) -> Result<Self, TemplateError> {
        let ConstRangeIntegerInput {
            output,
            start,
            end,
            step,
            maximum_step_magnitude,
            range,
            limit,
            span,
            float_precision,
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
                output,
            },
            emitted_iterations: 0,
            finished: false,
            limit,
            span,
            float_precision,
        })
    }
    fn new_fixed(
        scalar: FixedScalar,
        range_values: (
            ConstNumericValue,
            ConstNumericValue,
            Option<ConstNumericValue>,
        ),
        range: &RangeLoopSpec,
        limit: usize,
        span: Option<crate::compiler_frontend::source::SourceSpan>,
        float_precision: FloatPrecision,
    ) -> Result<Self, TemplateError> {
        let (start, end, step) = range_values;
        let end_kind = range.end_kind;
        let step_span = range.step.as_ref().and_then(|expression| expression.span);
        let kind = match scalar.class() {
            FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger => {
                let current = start
                    .to_fixed_integer()
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let end = end
                    .to_fixed_integer()
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let maximum_step_magnitude = fixed_integer_max_magnitude(scalar)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let explicit_step = step
                    .map(|value| {
                        value
                            .to_fixed_integer()
                            .ok_or_else(|| invalid_range_bounds(step_span.or(span)))
                    })
                    .transpose()?
                    .map(|magnitude| (magnitude, step_span));
                return Self::new_integer(ConstRangeIntegerInput {
                    output: ConstRangeIntegerOutput::Fixed(scalar),
                    start: current,
                    end,
                    step: explicit_step,
                    maximum_step_magnitude,
                    range,
                    limit,
                    span,
                    float_precision,
                });
            }
            FixedScalarClass::BinaryFloat => {
                let (scalar, precision) =
                    fixed_float_domain(scalar).ok_or_else(|| invalid_range_bounds(span))?;
                let current = start
                    .to_fixed_float(precision)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                let end = end
                    .to_fixed_float(precision)
                    .ok_or_else(|| invalid_range_bounds(span))?;
                if !current.is_finite() || !end.is_finite() {
                    return Err(invalid_range_bounds(span));
                }
                let step_magnitude = match step {
                    Some(value) => value
                        .to_fixed_float(precision)
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
            FixedScalarClass::Octet => return Err(invalid_range_bounds(span)),
        };

        Ok(Self {
            kind,
            emitted_iterations: 0,
            finished: false,
            limit,
            span,
            float_precision,
        })
    }

    pub(crate) fn iteration_count(&self) -> usize {
        self.emitted_iterations
    }

    pub(crate) fn next_counter(
        &mut self,
    ) -> Result<Option<ConstRangeIterationValue>, TemplateError> {
        if self.finished {
            return Ok(None);
        }

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

                let next = i128::from(*current) + i128::from(*step);
                let next_is_in_range = match (ascending, *end_kind) {
                    (true, RangeEndKind::Exclusive) => next < i128::from(*end),
                    (true, RangeEndKind::Inclusive) => next <= i128::from(*end),
                    (false, RangeEndKind::Exclusive) => next > i128::from(*end),
                    (false, RangeEndKind::Inclusive) => next >= i128::from(*end),
                };

                if next_is_in_range {
                    *current = current.checked_add(*step).ok_or_else(|| {
                        CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                            self.span,
                        )
                    })?;
                } else {
                    self.finished = true;
                }

                Ok(Some(counter))
            }
            ConstRangeCursorKind::Integer {
                current,
                end,
                end_kind,
                step,
                output,
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

                // Only counter materialisation keeps the source identity: the
                // candidate/termination mechanics above run once on `i128`.
                // The in-range exit below never computes an invalid successor,
                // so the inclusive maximum and descending zero terminate.
                let counter = match output {
                    ConstRangeIntegerOutput::Uint => u64::try_from(*current)
                        .ok()
                        .map(ConstRangeIterationValue::Uint)
                        .ok_or_else(|| invalid_range_bounds(self.span))?,
                    ConstRangeIntegerOutput::Fixed(scalar) => {
                        fixed_integer_scalar(*scalar, *current)
                            .map(ConstRangeIterationValue::Fixed)
                            .ok_or_else(|| invalid_range_bounds(self.span))?
                    }
                };
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
                    .map(ConstRangeIterationValue::Fixed)
                    .ok_or_else(|| invalid_range_bounds(self.span))?;
                self.emitted_iterations += 1;

                if *current == *end && *end_kind == RangeEndKind::Inclusive {
                    self.finished = true;
                    return Ok(Some(counter));
                }

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

                // An inclusive endpoint has already been emitted; do not try to
                // advance past it, since a rounded stalled update could look in-range.
                if *current == *end && *end_kind == RangeEndKind::Inclusive {
                    self.finished = true;
                    return Ok(Some(counter));
                }

                // Round every accumulation at the boundary precision so Bits32 counters
                // follow f32 stepping (and stall exactly when f32 cannot advance).
                let previous = *current;
                let next = self.float_precision.round(*current + *step);
                if !float_range_contains(next, *end, *end_kind, ascending) {
                    self.finished = true;
                } else {
                    if !next.is_finite() || next == previous {
                        return Err(CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                            self.span,
                        )
                        .into());
                    }
                    *current = next;
                }

                Ok(Some(counter))
            }
        }
    }
}

fn int_step_magnitude(
    step: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Result<i64, TemplateError> {
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
    Int(i64),
    Uint(u64),
    Float(f64),
    Fixed(FixedScalarValue),
}

impl ConstNumericValue {
    fn numeric_scalar(self) -> NumericScalar {
        match self {
            Self::Int(_) => NumericScalar::Int,
            Self::Uint(_) => NumericScalar::Uint,
            Self::Float(_) => NumericScalar::Float,
            Self::Fixed(value) => NumericScalar::Fixed(value.scalar()),
        }
    }

    fn to_float(self, float_precision: FloatPrecision) -> Option<f64> {
        match self {
            // Widened `Int` bounds convert at the boundary precision (never via an
            // f64 intermediate under Bits32), while literal `Float` bounds round once.
            Self::Int(value) => Some(float_precision.round_int(value)),
            Self::Uint(value) => Some(float_precision.round_uint(value)),
            Self::Float(value) => Some(float_precision.round(value)),
            Self::Fixed(value) => value.as_f64().map(|value| float_precision.round(value)),
        }
    }

    fn to_fixed_integer(self) -> Option<i128> {
        let Self::Fixed(value) = self else {
            return None;
        };
        value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from))
    }

    fn to_fixed_float(self, precision: BinaryFloatPrecision) -> Option<f64> {
        let Self::Fixed(value) = self else {
            return None;
        };
        value.as_f64().map(|value| precision.round(value))
    }
}

fn const_range_domain(
    start: ConstNumericValue,
    end: ConstNumericValue,
    step: Option<ConstNumericValue>,
) -> Option<NumericScalar> {
    range_iteration_domain(
        start.numeric_scalar(),
        end.numeric_scalar(),
        step.map(ConstNumericValue::numeric_scalar),
    )
}

fn fixed_float_domain(scalar: FixedScalar) -> Option<(FixedScalar, BinaryFloatPrecision)> {
    match scalar {
        FixedScalar::F16 | FixedScalar::F32 => {
            Some((FixedScalar::F32, BinaryFloatPrecision::Binary32))
        }
        FixedScalar::F64 => Some((FixedScalar::F64, BinaryFloatPrecision::Binary64)),
        _ => None,
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

fn const_numeric_expression(expression: &Expression) -> Result<ConstNumericValue, TemplateError> {
    match &expression.kind {
        ExpressionKind::Int(value) => Ok(ConstNumericValue::Int(*value)),
        ExpressionKind::Uint(value) => Ok(ConstNumericValue::Uint(*value)),
        ExpressionKind::Float(value) => Ok(ConstNumericValue::Float(*value)),
        ExpressionKind::FixedScalar(value) => Ok(ConstNumericValue::Fixed(*value)),
        ExpressionKind::Coerced { value, .. } => const_numeric_expression(value),
        _ => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
            expression.span,
        )
        .into()),
    }
}

fn int_range_contains(current: i64, end: i64, end_kind: RangeEndKind, ascending: bool) -> bool {
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
            ConstRangeIterationValue::Uint(value) => {
                Expression::uint(value, item.value.span, ValueMode::ImmutableOwned)
            }
            ConstRangeIterationValue::Float(value) => {
                Expression::float(value, item.value.span, ValueMode::ImmutableOwned)
            }
            ConstRangeIterationValue::Fixed(value) => {
                let value =
                    Expression::fixed_scalar(value, item.value.span, ValueMode::ImmutableOwned);
                debug_assert_eq!(
                    value.type_id, item.value.type_id,
                    "fixed range cursor must produce the declared binding type"
                );
                value
            }
        }
        .with_synthetic_interface_provenance(range_provenance.clone());
        fold_bindings.push(TemplateFoldBinding {
            path: item.id,
            value,
        });
    }

    if let Some(index) = &bindings.index {
        // Iteration indices stay within the const-loop expansion limit, well
        // below `i64::MAX`, so this checked conversion cannot fail in practice.
        let index_value = i64::try_from(zero_based_index).unwrap_or(i64::MAX);
        fold_bindings.push(TemplateFoldBinding {
            path: index.id,
            value: Expression::int(index_value, index.value.span, ValueMode::ImmutableOwned)
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
        // Iteration indices stay within the const-loop expansion limit, well
        // below `i64::MAX`, so this checked conversion cannot fail in practice.
        let index_value = i64::try_from(zero_based_index).unwrap_or(i64::MAX);
        fold_bindings.push(TemplateFoldBinding {
            path: index.id,
            value: Expression::int(index_value, index.value.span, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(iterable_provenance.clone()),
        });
    }

    fold_bindings
}
