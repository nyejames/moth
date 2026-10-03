//! Conservative bounded-integer proof side table.
//!
//! WHAT: an optional, statement-keyed table of proven-safe checked integer operations and
//!       proven-safe fallible integer narrowings, computed once per validated HIR executable
//!       under one `NumericProfile`. Consumers (the scalar backends) elide redundant runtime
//!       range predicates only where a fact exists; every unproven operation keeps its check.
//! WHY: backend check elision needs conservative facts that pair with the exact immutable HIR
//!      they were derived from. HIR, AST evidence, public semantics and target capability gates
//!      stay unchanged: this is a read-only side analysis, never a source or HIR transformation.
//!
//! ## Soundness boundary
//!
//! Facts are closed integer intervals over `i128`, seeded from `NumericScalar::integer_range`
//! (including exact `U64`), fixed-scalar literals and infallible integer cast expressions.
//! `Byte`, `Float`, binary floats and exact `Dec` never prove. Blocks are walked
//! independently with one last-written-local interval cache: state resets at every block entry,
//! and every write, call or unknown statement drops the cache. There is no predecessor
//! dataflow, fixpoint, refinement or HIR mutation. Checked `i128` endpoint arithmetic that
//! cannot represent an intermediate means no proof, never wrapping.
//!
//! ## Identity contract
//!
//! Facts are keyed by the statement's own `HirNodeId` and carry no interned strings, paths or
//! dense local layouts, so a table needs no remap when its executable is published. The table
//! is computed inside the compiler services after HIR validation, before publication, and is
//! consumed only together with that same executable's HIR and profile.

use rustc_hash::FxHashSet;

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_operators::negation_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::ids::{HirNodeId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use moth_lexical::numeric::profile::NumericProfile;

#[cfg(test)]
mod tests;

/// Inclusive closed integer interval. `low <= high` always holds.
type Interval = (i128, i128);

/// Proven-safe integer operation and narrowing facts for one immutable HIR executable.
///
/// WHAT: sparse statement-keyed side table paired with its `NumericProfile`. The empty default
///       table retains every runtime check.
/// WHY: backends query per statement instead of re-deriving ranges, and a missing or
///      mismatched table degrades to the fully checked behavior.
#[derive(Debug, Default)]
pub(crate) struct NumericProofs {
    /// Numeric profile the facts were computed under. Mismatched queries return false.
    profile: NumericProfile,
    /// `NumericOp` statement ids whose relevant failures are impossible.
    safe_operations: FxHashSet<HirNodeId>,
    /// Fallible integer-to-integer `NumericConversion` `CastOp` statement ids whose range
    /// predicates can never fail.
    safe_narrowings: FxHashSet<HirNodeId>,
}

impl NumericProofs {
    /// Whether the checked integer operation recorded at `statement` cannot fail.
    ///
    /// Profile mismatch returns false: facts are only valid under the profile they were
    /// derived from.
    pub(crate) fn integer_operation_is_safe(
        &self,
        statement: HirNodeId,
        profile: NumericProfile,
    ) -> bool {
        self.profile == profile && self.safe_operations.contains(&statement)
    }

    /// Whether the fallible integer narrowing recorded at `statement` cannot fail.
    ///
    /// Profile mismatch returns false, matching the operation lookup.
    pub(crate) fn integer_narrowing_is_safe(
        &self,
        statement: HirNodeId,
        profile: NumericProfile,
    ) -> bool {
        self.profile == profile && self.safe_narrowings.contains(&statement)
    }
}

/// Computes the conservative proof table for one validated HIR module.
///
/// WHAT: walks every block independently under `profile`, deriving closed intervals for
///       checked integer operations and fallible integer narrowings.
/// WHY: base and generated sidecar compilation each call this once after HIR validation and
///      before publication, so every executable carries its own paired table.
pub(crate) fn analyse_numeric_proofs(
    hir: &HirModule,
    environment: &TypeEnvironment,
    profile: NumericProfile,
) -> NumericProofs {
    let mut analyser = BlockAnalyser {
        environment,
        profile,
        cache: None,
        facts: NumericProofs {
            profile,
            safe_operations: FxHashSet::default(),
            safe_narrowings: FxHashSet::default(),
        },
    };

    // Blocks are walked independently: value-specific state resets at every block entry,
    // including joins and loop headers. No predecessor state is ever consulted.
    for block in &hir.blocks {
        analyser.cache = None;
        for statement in &block.statements {
            analyser.visit_statement(statement);
        }
    }

    analyser.facts
}

/// One block walk's conservative state.
struct BlockAnalyser<'a> {
    environment: &'a TypeEnvironment,
    profile: NumericProfile,
    /// The single last-written-local interval cache. `None` means no value-specific fact.
    cache: Option<(LocalId, Interval)>,
    facts: NumericProofs,
}

impl BlockAnalyser<'_> {
    /// Visits one statement, updating the proof table and the interval cache.
    fn visit_statement(&mut self, statement: &HirStatement) {
        match &statement.kind {
            HirStatementKind::Assign { target, value } => {
                // Derive the source interval before any invalidation.
                let interval = self.expression_interval(value);
                match target {
                    // Supported direct-local assignments retain only the destination's current
                    // interval; the one-entry cache makes every earlier fact unreachable.
                    HirPlace::Local(local) => {
                        self.cache = interval.map(|interval| (*local, interval));
                    }
                    // Projection writes may reach any value behind the base place.
                    HirPlace::Field { .. } | HirPlace::Index { .. } => self.cache = None,
                }
            }
            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                // Operand facts are derived before any invalidation.
                if let Some(result_interval) = self.prove_operation(op, operands) {
                    self.facts.safe_operations.insert(statement.id);
                    if *failure_mode == NumericFailureMode::Trap {
                        // A safe trap operation writes the scalar success value directly, so
                        // the destination keeps its exact proven interval.
                        self.cache = Some((*result, result_interval));
                        return;
                    }
                }
                // ReturnError results are fallible carriers: narrower carrier-success facts
                // never propagate, and unproven operations conservatively drop the cache.
                self.cache = None;
            }
            HirStatementKind::CastOp { policy, source, .. } => {
                // Source facts are derived before invalidation.
                if let Some(source_interval) = self.expression_interval(source)
                    && self.cast_narrowing_is_provable(policy, source_interval)
                {
                    self.facts.safe_narrowings.insert(statement.id);
                }
                // CastOp statements lower fallible builtin casts, so the result local receives
                // a carrier; the success scalar only exists after the paired FallibleBranch
                // unwraps it. No value-specific fact survives the statement.
                self.cache = None;
            }
            // Calls, side-effect expressions, map operations, drops, float helpers and every
            // other unsupported statement conservatively drop value-specific facts.
            _ => self.cache = None,
        }
    }

    /// Proves one checked numeric operation safe, returning its exact result interval.
    ///
    /// `Power` never proves, and every non-integer domain (Float, binary floats, exact
    /// `Dec`) stays checked. The returned interval must fit the operation domain, which is
    /// what makes the checked operation's failure impossible.
    fn prove_operation(
        &self,
        op: &HirNumericOp,
        operands: &HirNumericOperands,
    ) -> Option<Interval> {
        let domain = op.domain.integer_range(self.profile)?;
        if !op.domain.is_integer() {
            return None;
        }

        let interval = match op.operator {
            NumericOperator::Add => {
                let (left, right) = self.binary_intervals(operands)?;
                (left.0.checked_add(right.0)?, left.1.checked_add(right.1)?)
            }
            NumericOperator::Subtract => {
                let (left, right) = self.binary_intervals(operands)?;
                (left.0.checked_sub(right.1)?, left.1.checked_sub(right.0)?)
            }
            NumericOperator::Multiply => {
                let (left, right) = self.binary_intervals(operands)?;
                let (mut low, mut high) = (i128::MAX, i128::MIN);
                for left_endpoint in [left.0, left.1] {
                    for right_endpoint in [right.0, right.1] {
                        let product = left_endpoint.checked_mul(right_endpoint)?;
                        low = low.min(product);
                        high = high.max(product);
                    }
                }
                (low, high)
            }
            NumericOperator::IntegerDivide => {
                let (left, right) = self.binary_intervals(operands)?;
                // A divisor interval that may contain zero can always fail.
                if right.0 <= 0 && 0 <= right.1 {
                    return None;
                }
                // Signed minimum divided by -1 overflows the domain (e.g. I64::MIN / -1).
                if domain.0 < 0
                    && left.0 <= domain.0
                    && domain.0 <= left.1
                    && right.0 <= -1
                    && -1 <= right.1
                {
                    return None;
                }
                // The divisor is single-signed here, so truncated division is monotone along
                // both axes and the four corners bound every quotient.
                let (mut low, mut high) = (i128::MAX, i128::MIN);
                for left_endpoint in [left.0, left.1] {
                    for right_endpoint in [right.0, right.1] {
                        let quotient = left_endpoint.checked_div(right_endpoint)?;
                        low = low.min(quotient);
                        high = high.max(quotient);
                    }
                }
                (low, high)
            }
            NumericOperator::Remainder => {
                let (left, right) = self.binary_intervals(operands)?;
                // A divisor interval that may contain zero can always fail.
                if right.0 <= 0 && 0 <= right.1 {
                    return None;
                }
                // Truncating remainder keeps the dividend's sign and stays strictly below the
                // divisor magnitude. The specified signed-minimum %-1 result is exact zero,
                // so no extra exclusion applies.
                let bound = right.0.abs().max(right.1.abs()).checked_sub(1)?;
                if left.0 >= 0 {
                    (0, bound)
                } else if left.1 <= 0 {
                    (-bound, 0)
                } else {
                    (-bound, bound)
                }
            }
            NumericOperator::Negate => {
                // Signed integer domains only: unsigned negation is rejected at the source
                // level even for known zero, and signed narrow domains promote to their
                // negation domain, so any other shape keeps its check.
                if negation_domain(op.domain) != Some(op.domain) {
                    return None;
                }
                let HirNumericOperands::Unary { operand } = operands else {
                    return None;
                };
                let (low, high) = self.expression_interval(operand)?;
                (high.checked_neg()?, low.checked_neg()?)
            }
            // Real division computes in float domains and power always retains its checks.
            NumericOperator::Divide | NumericOperator::Power => return None,
        };

        (domain.0 <= interval.0 && interval.1 <= domain.1).then_some(interval)
    }

    /// Derives both binary operand intervals before any invalidation.
    fn binary_intervals(&self, operands: &HirNumericOperands) -> Option<(Interval, Interval)> {
        let HirNumericOperands::Binary { left, right } = operands else {
            return None;
        };
        Some((
            self.expression_interval(left)?,
            self.expression_interval(right)?,
        ))
    }

    /// Whether one fallible integer-to-integer `NumericConversion` cast can never fail.
    ///
    /// Source and target domain facts stay owned by the immutable HIR policy; the proof only
    /// compares the derived source interval against the target's complete range.
    fn cast_narrowing_is_provable(
        &self,
        policy: &BuiltinCastPolicyId,
        source_interval: Interval,
    ) -> bool {
        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return false;
        };
        if !source.is_integer() || !target.is_integer() {
            return false;
        }
        let Some((target_low, target_high)) = target.integer_range(self.profile) else {
            return false;
        };
        source_interval.0 >= target_low && source_interval.1 <= target_high
    }

    /// Closed interval of one expression's runtime value, or `None` when it cannot bound one.
    ///
    /// Loads and copies use the current cache only for the exact direct local it names; every
    /// other value falls back to complete canonical integer type bounds.
    fn expression_interval(&self, expression: &HirExpression) -> Option<Interval> {
        match &expression.kind {
            // Literals are singleton exact values.
            HirExpressionKind::Int(value) => {
                let value = i128::from(*value);
                Some((value, value))
            }
            HirExpressionKind::FixedScalar(value) => {
                // Signed integers carry sign-extended i64 payloads; unsigned integers and Byte
                // carry zero-extended u64 payloads. Binary floats have no integer interval.
                let value = value
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| value.as_u64().map(i128::from))?;
                Some((value, value))
            }
            // Infallible integer-to-integer cast expressions preserve the source interval,
            // intersected with the target domain. Fallible casts never appear as expressions.
            HirExpressionKind::Cast { source, policy } => {
                let BuiltinCastPolicyId::NumericConversion { target, .. } = policy else {
                    return None;
                };
                let target_range = target.integer_range(self.profile)?;
                let source_interval = self.expression_interval(source)?;
                let low = source_interval.0.max(target_range.0);
                let high = source_interval.1.min(target_range.1);
                (low <= high).then_some((low, high))
            }
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => match place {
                HirPlace::Local(local) => match self.cache {
                    Some((cached_local, interval)) if cached_local == *local => Some(interval),
                    _ => self.canonical_interval(expression.ty),
                },
                HirPlace::Field { .. } | HirPlace::Index { .. } => {
                    self.canonical_interval(expression.ty)
                }
            },
            // Unwraps, constructions and every other value shape read complete canonical
            // integer bounds; non-integer types have no interval at all.
            _ => self.canonical_interval(expression.ty),
        }
    }

    /// Complete canonical integer range of one local or expression type.
    ///
    /// `Byte`, floats, binary floats and `Dec` have no bounded integer range and return
    /// `None`, keeping those domains excluded from every proof.
    fn canonical_interval(&self, type_id: TypeId) -> Option<Interval> {
        let scalar = NumericScalar::from_type_id(type_id, self.environment)?;
        if !scalar.is_integer() {
            return None;
        }
        scalar.integer_range(self.profile)
    }
}
