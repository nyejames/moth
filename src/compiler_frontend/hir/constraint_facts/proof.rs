//! Bounded D16 proof policy for checked numeric operations.
//!
//! WHAT: the single owner of per-cause discharge from exact operand literals. Phase B
//!       `may_trap`/`may_fail` analysis and statement/function verification both ask this module
//!       whether an operation's failure causes are impossible.
//! WHY: acceptance must never depend on a backend or on the later numeric-proof side table. The
//!      policy therefore reads final HIR operands directly and proves only the causes named by the
//!      user contract: a nonzero literal divisor, a signed `//` divisor that is neither zero nor
//!      `-1`, and a non-negative literal exponent.
//!
//! Operand facts are deliberately small. A direct `FixedScalar` literal is exact. A `Load` of a
//! local is exact only when the same block defines it exactly once, before the operation, from a
//! direct fixed scalar and never writes it again. The block walk resets at every block entry and
//! invalidates a local on any later write, so there is no predecessor merge, loop induction or
//! cross-block fact. `Dec` literals never discharge a cause because a `Dec` scale can fail at any
//! divisor. Exact add/subtract/multiply results, binary-float finiteness and range guards are not
//! proved here.

use rustc_hash::FxHashMap;

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, fast_numeric_trap_codes, numeric_failure_codes,
};
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;

/// One block's forward local-literal facts, as seen at the current statement.
///
/// WHAT: locals defined exactly once so far by a direct fixed-scalar write. A second write of any
///       kind replaces the fact with `None` so it can never be used again in this block.
/// WHY: the oracle accepts a `Load` only from this exact definition shape. Keeping the three
///      states in one map makes "defined once" and "never rewritten" the same lookup.
#[derive(Debug, Default)]
pub(crate) struct BlockLiteralFacts {
    locals: FxHashMap<LocalId, Option<FixedScalarValue>>,
}

impl BlockLiteralFacts {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Drop every fact while keeping the map capacity for the next block walk.
    pub(crate) fn reset(&mut self) {
        self.locals.clear();
    }

    /// Resolve one operand to an exact fixed scalar, if the policy has a local fact for it.
    pub(crate) fn fixed_operand(
        &self,
        expressions: &HirExpressionStore,
        operand: HirValueId,
    ) -> Option<FixedScalarValue> {
        match &expressions.expression(operand).kind {
            HirExpressionKind::FixedScalar(value) => Some(*value),
            HirExpressionKind::Load(place)
                if expressions.projections(place.projections).is_empty() =>
            {
                self.locals.get(&place.root).copied().flatten()
            }
            _ => None,
        }
    }

    /// Record every binding this statement writes after its operands have been observed.
    ///
    /// WHAT: a first `DefineLocal` of a direct literal records the value; every direct write
    ///       disqualifies its local. Calls, map operations and projected writes clear every fact
    ///       in the block because final HIR no longer retains argument access modes or alias
    ///       roots: a mutable argument, an escaped reference or a write through an alias may
    ///       update any earlier binding.
    /// WHY: operand evaluation happens before the statement's own writes, so callers observe
    ///      operand facts first and then update the map with this method. Clearing at a possibly
    ///      reference-mutating statement is the only sound policy without retained borrow/alias
    ///      evidence; direct literals and literals defined later still prove independently.
    pub(crate) fn observe_statement(
        &mut self,
        expressions: &HirExpressionStore,
        statement: &HirStatement,
    ) {
        match &statement.kind {
            HirStatementKind::Write { target, value } => match target {
                HirWriteTarget::DefineLocal(local) => {
                    let literal = match &expressions.expression(*value).kind {
                        HirExpressionKind::FixedScalar(literal)
                            if !self.locals.contains_key(local) =>
                        {
                            Some(*literal)
                        }
                        _ => None,
                    };
                    self.locals.insert(*local, literal);
                }
                HirWriteTarget::AssignPlace(_) => {
                    // A projected or aliased place may update any binding whose reference
                    // escaped earlier in the block.
                    self.locals.clear();
                }
            },
            HirStatementKind::Call { .. } => {
                // Mutable `~` arguments have no retained access mode in final HIR; clear every
                // fact rather than trusting a literal a callee may overwrite.
                self.locals.clear();
            }
            HirStatementKind::MapOp { .. } => {
                // Map operations can mutate the receiver and any aliased map argument.
                self.locals.clear();
            }
            HirStatementKind::NumericOp { result, .. }
            | HirStatementKind::FastStoreConversion { result, .. }
            | HirStatementKind::LoopStepFailure { result, .. }
            | HirStatementKind::FormatFloat { result, .. }
            | HirStatementKind::ValidateFloat { result, .. } => {
                self.locals.insert(result.local(), None);
            }
            HirStatementKind::RangeCandidate {
                candidate_result,
                in_range_result,
                ..
            } => {
                self.locals.insert(candidate_result.local(), None);
                self.locals.insert(in_range_result.local(), None);
            }
            HirStatementKind::Expr(_) | HirStatementKind::PushRuntimeFragment { .. } => {}
        }
    }
}

/// Whether any trap cause of one operation remains after the bounded policy.
///
/// WHAT: the allocation-free predicate phase B and verification use for liveness and trap sites.
/// WHY: a `NumericFailureMode::Infallible` operation is already total; otherwise only the shared
///      per-cause table decides, so no caller reimplements operator-specific proof.
pub(crate) fn operation_has_remaining_trap_causes(
    expressions: &HirExpressionStore,
    literals: &BlockLiteralFacts,
    op: HirNumericOp,
    operands: &HirNumericOperands,
) -> bool {
    operation_trap_causes(op)
        .iter()
        .copied()
        .any(|cause| !cause_is_discharged(cause, op, operands, literals, expressions))
}

/// Iterate one operation's trap causes that the bounded policy does not discharge.
///
/// WHAT: the remaining causes in the canonical table order, without allocating a set.
/// WHY: diagnostics classify overflow, division, exponent and non-finite wording from the exact
///      causes, while liveness needs only the boolean above.
pub(crate) fn remaining_trap_causes<'a>(
    expressions: &'a HirExpressionStore,
    literals: &'a BlockLiteralFacts,
    op: HirNumericOp,
    operands: &'a HirNumericOperands,
) -> impl Iterator<Item = BuiltinErrorCode> + 'a {
    operation_trap_causes(op)
        .iter()
        .copied()
        .filter(move |cause| !cause_is_discharged(*cause, op, operands, literals, expressions))
}

/// The trap-cause table for one operation's selected semantics.
pub(crate) fn operation_trap_causes(op: HirNumericOp) -> &'static [BuiltinErrorCode] {
    if op.semantics.is_fast() {
        fast_numeric_trap_codes(op.operator, op.domain)
    } else {
        numeric_failure_codes(op.operator, op.domain)
    }
}

/// Whether record delivery already proves the statement cannot fail.
pub(crate) fn failure_mode_is_total(failure_mode: NumericFailureMode) -> bool {
    failure_mode == NumericFailureMode::Infallible
}

fn cause_is_discharged(
    cause: BuiltinErrorCode,
    op: HirNumericOp,
    operands: &HirNumericOperands,
    literals: &BlockLiteralFacts,
    expressions: &HirExpressionStore,
) -> bool {
    match cause {
        BuiltinErrorCode::DivideByZero => right_operand(operands)
            .and_then(|divisor| literals.fixed_operand(expressions, divisor))
            .is_some_and(|divisor| !fixed_scalar_is_zero(divisor)),
        BuiltinErrorCode::IntOverflow => {
            if op.operator != NumericOperator::IntegerDivide {
                return false;
            }
            // The ordinary code table lists `IntOverflow` for every integer `//`; truncating
            // division on an unsigned domain can never overflow, so that cause is impossible.
            let Some((minimum, _)) = op.domain.integer_range() else {
                return true;
            };
            if minimum >= 0 {
                return true;
            }
            right_operand(operands)
                .and_then(|divisor| literals.fixed_operand(expressions, divisor))
                .and_then(|divisor| divisor.as_i64())
                .is_some_and(|divisor| divisor != 0 && divisor != -1)
        }
        BuiltinErrorCode::InvalidExponent => {
            op.operator == NumericOperator::Power
                && right_operand(operands)
                    .and_then(|exponent| literals.fixed_operand(expressions, exponent))
                    .is_some_and(fixed_scalar_is_non_negative_integer)
        }
        // Binary-float finiteness depends on the other operand and the exact result, which this
        // bounded policy never computes.
        BuiltinErrorCode::FloatNonFinite => false,
        // The operation tables only use the four numeric causes above. A future cause has no
        // local proof until this owner learns it, so it remains live.
        _ => false,
    }
}

fn right_operand(operands: &HirNumericOperands) -> Option<HirValueId> {
    match operands {
        HirNumericOperands::Unary { .. } => None,
        HirNumericOperands::Binary { right, .. } => Some(*right),
    }
}

fn fixed_scalar_is_zero(value: FixedScalarValue) -> bool {
    if let Some(integer) = value.as_i64() {
        return integer == 0;
    }
    if let Some(integer) = value.as_u64() {
        return integer == 0;
    }
    // `-0.0 == 0.0` in IEEE comparison, so both signed zeros count as a zero divisor.
    value.as_f64().is_some_and(|float| float == 0.0)
}

fn fixed_scalar_is_non_negative_integer(value: FixedScalarValue) -> bool {
    value.as_i64().is_some_and(|integer| integer >= 0) || value.as_u64().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::datatypes::numeric_operators::NumericSemantics;
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
    use crate::compiler_frontend::hir::expressions::{HirExpression, ValueKind};
    use crate::compiler_frontend::hir::ids::RegionId;
    use crate::compiler_frontend::hir::places::HirPlace;
    use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatement};
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    fn store_with_literal(scalar: FixedScalar, value: i64) -> (HirExpressionStore, HirValueId) {
        let mut store = HirExpressionStore::default();
        let literal =
            FixedScalarValue::signed(scalar, value).expect("test literal fits signed scalar");
        let id = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::FixedScalar(literal),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    scalar,
                ),
                value_kind: ValueKind::Const,
                region: RegionId(0),
                span: None,
            })
            .expect("test expression store accepts a literal");
        (store, id)
    }

    fn store_with_literals(
        scalar: FixedScalar,
        left: i64,
        right: i64,
    ) -> (HirExpressionStore, HirValueId, HirValueId) {
        let mut store = HirExpressionStore::default();
        let append = |store: &mut HirExpressionStore, value: i64| {
            let literal =
                FixedScalarValue::signed(scalar, value).expect("test literal fits signed scalar");
            store
                .append_expression(HirExpression {
                    kind: HirExpressionKind::FixedScalar(literal),
                    ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                        scalar,
                    ),
                    value_kind: ValueKind::Const,
                    region: RegionId(0),
                    span: None,
                })
                .expect("test expression store accepts a literal")
        };
        let left = append(&mut store, left);
        let right = append(&mut store, right);
        (store, left, right)
    }

    fn operation(
        operator: NumericOperator,
        domain: NumericScalar,
        semantics: NumericSemantics,
    ) -> HirNumericOp {
        HirNumericOp {
            operator,
            domain,
            semantics,
        }
    }

    fn binary(right: HirValueId) -> HirNumericOperands {
        HirNumericOperands::Binary {
            left: HirValueId(u32::MAX),
            right,
        }
    }

    fn assert_split(
        store: &(HirExpressionStore, HirValueId, HirValueId),
        op: HirNumericOp,
        discharged: &[BuiltinErrorCode],
        remaining: &[BuiltinErrorCode],
    ) {
        let (_store, _left, right) = store;
        let facts = BlockLiteralFacts::new();
        let remaining_actual =
            remaining_trap_causes(&store.0, &facts, op, &binary(*right)).collect::<Vec<_>>();
        assert_eq!(remaining_actual, remaining);
        let discharged_actual = operation_trap_causes(op)
            .iter()
            .copied()
            .filter(|cause| !remaining_actual.contains(cause))
            .collect::<Vec<_>>();
        assert_eq!(discharged_actual, discharged);
        assert_eq!(
            operation_has_remaining_trap_causes(&store.0, &facts, op, &binary(*right)),
            !remaining.is_empty(),
            "remaining cause expectation must agree with the table"
        );
    }

    #[test]
    fn integer_division_by_a_nonzero_non_minus_one_literal_is_total() {
        let store = store_with_literals(FixedScalar::I32, 10, 2);
        assert_split(
            &store,
            operation(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::IntOverflow,
            ],
            &[],
        );
    }

    #[test]
    fn integer_division_by_zero_or_minus_one_keeps_the_matching_cause() {
        let zero = store_with_literals(FixedScalar::I32, 10, 0);
        assert_split(
            &zero,
            operation(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[],
            &[
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::IntOverflow,
            ],
        );

        let minus_one = store_with_literals(FixedScalar::I32, 10, -1);
        assert_split(
            &minus_one,
            operation(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[BuiltinErrorCode::DivideByZero],
            &[BuiltinErrorCode::IntOverflow],
        );
    }

    #[test]
    fn unsigned_truncating_division_only_needs_a_nonzero_divisor() {
        let mut store = HirExpressionStore::default();
        let append = |store: &mut HirExpressionStore, value: u64| {
            let literal =
                FixedScalarValue::unsigned(FixedScalar::U32, value).expect("test literal fits U32");
            store
                .append_expression(HirExpression {
                    kind: HirExpressionKind::FixedScalar(literal),
                    ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                        FixedScalar::U32,
                    ),
                    value_kind: ValueKind::Const,
                    region: RegionId(0),
                    span: None,
                })
                .expect("test expression store accepts a literal")
        };
        let left = append(&mut store, 10);
        let right = append(&mut store, 2);
        let facts = BlockLiteralFacts::new();
        assert!(!operation_has_remaining_trap_causes(
            &store,
            &facts,
            operation(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::U32),
                NumericSemantics::Ordinary,
            ),
            &HirNumericOperands::Binary { left, right },
        ));
    }

    #[test]
    fn remainder_discharges_only_the_zero_divisor_cause() {
        let store = store_with_literals(FixedScalar::I32, 10, 2);
        assert_split(
            &store,
            operation(
                NumericOperator::Remainder,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[BuiltinErrorCode::DivideByZero],
            &[],
        );
    }

    #[test]
    fn binary_float_division_never_proves_finiteness() {
        let mut store = HirExpressionStore::default();
        let append = |store: &mut HirExpressionStore, value: f64| {
            let literal =
                FixedScalarValue::binary_float(FixedScalar::F64, value).expect("finite F64");
            store
                .append_expression(HirExpression {
                    kind: HirExpressionKind::FixedScalar(literal),
                    ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                        FixedScalar::F64,
                    ),
                    value_kind: ValueKind::Const,
                    region: RegionId(0),
                    span: None,
                })
                .expect("test expression store accepts a literal")
        };
        let left = append(&mut store, 1.0);
        let right = append(&mut store, 2.0);
        let facts = BlockLiteralFacts::new();
        let op = operation(
            NumericOperator::Divide,
            NumericScalar::Fixed(FixedScalar::F64),
            NumericSemantics::Ordinary,
        );
        let operands = HirNumericOperands::Binary { left, right };
        assert_eq!(
            remaining_trap_causes(&store, &facts, op, &operands).collect::<Vec<_>>(),
            vec![BuiltinErrorCode::FloatNonFinite],
        );
        assert!(operation_has_remaining_trap_causes(
            &store, &facts, op, &operands
        ));
    }

    #[test]
    fn signed_zero_float_divisor_is_still_a_zero_divisor() {
        let mut store = HirExpressionStore::default();
        let left = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::FixedScalar(
                    FixedScalarValue::binary_float(FixedScalar::F64, 1.0).expect("finite F64"),
                ),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    FixedScalar::F64,
                ),
                value_kind: ValueKind::Const,
                region: RegionId(0),
                span: None,
            })
            .expect("literal row");
        let right = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::FixedScalar(
                    FixedScalarValue::binary_float(FixedScalar::F64, -0.0).expect("finite F64"),
                ),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    FixedScalar::F64,
                ),
                value_kind: ValueKind::Const,
                region: RegionId(0),
                span: None,
            })
            .expect("literal row");
        let facts = BlockLiteralFacts::new();
        let op = operation(
            NumericOperator::Divide,
            NumericScalar::Fixed(FixedScalar::F64),
            NumericSemantics::Ordinary,
        );
        let operands = HirNumericOperands::Binary { left, right };
        assert_eq!(
            remaining_trap_causes(&store, &facts, op, &operands).collect::<Vec<_>>(),
            operation_trap_causes(op).to_vec(),
        );
    }

    #[test]
    fn power_discharges_only_a_non_negative_exponent() {
        let positive = store_with_literals(FixedScalar::I32, 2, 3);
        assert_split(
            &positive,
            operation(
                NumericOperator::Power,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[BuiltinErrorCode::InvalidExponent],
            &[BuiltinErrorCode::IntOverflow],
        );

        let negative = store_with_literals(FixedScalar::I32, 2, -3);
        assert_split(
            &negative,
            operation(
                NumericOperator::Power,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Ordinary,
            ),
            &[],
            &[
                BuiltinErrorCode::IntOverflow,
                BuiltinErrorCode::InvalidExponent,
            ],
        );
    }

    #[test]
    fn fast_arithmetic_tables_are_the_only_fast_causes() {
        // Fast integer add has no trap cause at all.
        let store = store_with_literals(FixedScalar::I32, 1, 2);
        assert_split(
            &store,
            operation(
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Fast,
            ),
            &[],
            &[],
        );

        // Fast signed `//` by two is total even though the ordinary table separates the causes.
        let division = store_with_literals(FixedScalar::I32, 10, 2);
        assert_split(
            &division,
            operation(
                NumericOperator::IntegerDivide,
                NumericScalar::Fixed(FixedScalar::I32),
                NumericSemantics::Fast,
            ),
            &[
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::IntOverflow,
            ],
            &[],
        );
    }

    #[test]
    fn a_single_definition_literal_local_is_usable_until_any_rewrite() {
        let (mut store, literal) = store_with_literal(FixedScalar::I32, 2);
        let divisor_local = LocalId(7);
        let mut facts = BlockLiteralFacts::new();
        let define = HirStatement {
            id: crate::compiler_frontend::hir::ids::HirNodeId(0),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(divisor_local),
                value: literal,
            },
            span: None,
        };
        facts.observe_statement(&store, &define);
        let load = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(divisor_local)),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    FixedScalar::I32,
                ),
                value_kind: ValueKind::Place,
                region: RegionId(0),
                span: None,
            })
            .expect("load row");
        assert_eq!(
            facts.fixed_operand(&store, load),
            Some(FixedScalarValue::signed(FixedScalar::I32, 2).expect("fits I32"))
        );

        let rewrite = HirStatement {
            id: crate::compiler_frontend::hir::ids::HirNodeId(1),
            kind: HirStatementKind::NumericOp {
                op: operation(
                    NumericOperator::Add,
                    NumericScalar::Fixed(FixedScalar::I32),
                    NumericSemantics::Ordinary,
                ),
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Binary {
                    left: literal,
                    right: literal,
                },
                result: HirLocalDestination::Define(divisor_local),
            },
            span: None,
        };
        facts.observe_statement(&store, &rewrite);
        assert_eq!(facts.fixed_operand(&store, load), None);
    }

    #[test]
    fn a_call_clears_every_literal_fact_because_a_mutable_argument_may_overwrite_it() {
        let (mut store, literal) = store_with_literal(FixedScalar::I32, 2);
        let divisor_local = LocalId(11);
        let mut facts = BlockLiteralFacts::new();
        let define = HirStatement {
            id: crate::compiler_frontend::hir::ids::HirNodeId(0),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(divisor_local),
                value: literal,
            },
            span: None,
        };
        facts.observe_statement(&store, &define);
        let load = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(divisor_local)),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    FixedScalar::I32,
                ),
                value_kind: ValueKind::Place,
                region: RegionId(0),
                span: None,
            })
            .expect("load row");
        assert!(facts.fixed_operand(&store, load).is_some());

        let call = HirStatement {
            id: crate::compiler_frontend::hir::ids::HirNodeId(1),
            kind: HirStatementKind::Call {
                target: crate::compiler_frontend::external_packages::CallTarget::External(
                    crate::compiler_frontend::external_packages::ExternalFunctionId::IoPrint,
                ),
                args: crate::compiler_frontend::hir::expression_store::HirValueRange::empty(),
                results: Vec::new(),
            },
            span: None,
        };
        facts.observe_statement(&store, &call);
        assert_eq!(
            facts.fixed_operand(&store, load),
            None,
            "a call may mutate any binding through a retained mutable reference"
        );
    }

    #[test]
    fn a_second_definition_disqualifies_even_when_both_are_literals() {
        let (mut store, literal) = store_with_literal(FixedScalar::I32, 2);
        let divisor_local = LocalId(3);
        let mut facts = BlockLiteralFacts::new();
        for id in 0..2 {
            let define = HirStatement {
                id: crate::compiler_frontend::hir::ids::HirNodeId(id),
                kind: HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(divisor_local),
                    value: literal,
                },
                span: None,
            };
            facts.observe_statement(&store, &define);
        }
        let load = store
            .append_expression(HirExpression {
                kind: HirExpressionKind::Load(HirPlace::local(divisor_local)),
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    FixedScalar::I32,
                ),
                value_kind: ValueKind::Place,
                region: RegionId(0),
                span: None,
            })
            .expect("load row");
        assert_eq!(facts.fixed_operand(&store, load), None);
    }
}
