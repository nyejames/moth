//! Conservative bounded-integer proof side table.
//!
//! WHAT: an optional, operation-keyed table of proven-safe checked integer operations and
//!       proven-safe fallible integer narrowings, computed once per validated HIR executable
//!       from its exact numeric domains. The scalar backends use it to elide redundant range
//!       predicates. Without a fact, they retain every otherwise-required check.
//! WHY: backend check elision needs conservative facts that pair with the exact immutable HIR
//!      they were derived from. HIR, AST evidence, public semantics and target capability gates
//!      stay unchanged: this is a read-only side analysis, never a source or HIR transformation.
//!
//! ## Soundness boundary
//!
//! Facts are closed integer intervals over `i128`, seeded from `NumericScalar::integer_range`
//! (including exact `U64`), fixed-scalar literals and infallible integer cast expressions.
//! `Byte`, binary floats and exact `Dec` never prove. Blocks are otherwise walked independently
//! with one last-written-local interval cache, and every write, call or unknown statement drops
//! the cache.
//!
//! One bounded predecessor fact crosses a block edge. When an `Invoke` whose operation is a
//! proven integer `NumericOp` or a proven integer narrowing has exactly one success result, and
//! its success block has exactly one incoming CFG edge over every terminator target and is not a
//! function entry, that block's walk starts with the result local at the operation's conservative
//! enclosing result interval. The block is entered only through that edge, the edge transfer writes
//! exactly that local with the proven interval, and nothing executes before the block's first
//! statement.
//! Seeds are consumed when their block is walked, and a block already visited in dense block
//! order never receives one. Everything else still starts from no value-specific fact: several
//! predecessors, a loop header, no fact travels on a failure edge, an unproven operation or a seed
//! recorded after the block's walk. There is no general predecessor dataflow, fixpoint,
//! refinement, join or HIR mutation. Checked `i128` endpoint arithmetic that cannot represent an
//! intermediate means no proof, never wrapping.
//!
//! Fast-semantics operations never publish an operation fact: the table is the ordinary
//! checked-operation contract, and both scalar backends select fast delivery before consulting
//! it. A proven fast statement's result interval still enters the single-local cache, because a
//! fast result whose checked mathematical interval fits the result domain cannot wrap and is
//! therefore the mathematical value.
//!
//! ## Identity contract
//!
//! Facts are keyed by the statement or Invoke operation's own `HirNodeId` and carry no interned
//! strings, paths or dense local layouts, so a table needs no remap when its executable is
//! published. Compiler services compute it from final validated HIR after private-lane
//! installation and convergence. Backends consume it only with that executable's HIR.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, checked_result_range, negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{HirNumericOp, HirNumericOperands};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::for_each_terminator_target;

#[cfg(test)]
mod tests;

/// Inclusive closed integer interval. `low <= high` always holds.
type Interval = (i128, i128);

/// Proven-safe integer operation and narrowing facts for one immutable HIR executable.
///
/// WHAT: sparse operation-keyed side table paired with its executable.
/// WHY: backends query an operation rather than re-deriving ranges. A missing fact retains
///      every otherwise-required check without changing mandatory HIR totality.
#[derive(Debug, Default)]
pub(crate) struct NumericProofs {
    /// Ordinary checked `NumericOp` statement or Invoke ids whose relevant failures are
    /// impossible. Fast-semantics operations never publish here; their result interval only feeds
    /// the analysis cache.
    safe_operations: FxHashSet<HirNodeId>,
    /// Fallible integer-to-integer `NumericConversion` Invoke ids whose range predicates
    /// can never fail.
    safe_narrowings: FxHashSet<HirNodeId>,
}

impl NumericProofs {
    /// Whether the checked integer operation with this node identity cannot fail.
    pub(crate) fn integer_operation_is_safe(&self, operation: HirNodeId) -> bool {
        self.safe_operations.contains(&operation)
    }

    /// Whether the fallible integer narrowing with this Invoke node identity cannot fail.
    pub(crate) fn integer_narrowing_is_safe(&self, operation: HirNodeId) -> bool {
        self.safe_narrowings.contains(&operation)
    }
}

/// Computes the conservative proof table for one validated HIR module.
///
/// WHAT: walks every block in `hir.blocks` order, deriving closed intervals from each
///       statement's exact numeric domains for checked integer operations and fallible integer
///       narrowings. A proven Invoke supplies its sole success block's entry interval.
/// WHY: base and generated sidecar compilation each compute this from final validated HIR after
///      private-lane installation and convergence, before publication. Post-compaction block ids
///      are dense, so the incoming-edge table can index by `BlockId.0`.
pub(crate) fn analyse_numeric_proofs(
    hir: &HirModule,
    environment: &TypeEnvironment,
) -> NumericProofs {
    let incoming_edges = count_incoming_edges(hir);
    let mut analyser = BlockAnalyser {
        environment,
        expressions: &hir.expressions,
        incoming_edges: &incoming_edges,
        seeds: FxHashMap::default(),
        cache: None,
        current_block: 0,
        facts: NumericProofs::default(),
    };

    // Blocks are walked once in module order. A block with several predecessors, a loop
    // header or no usable incoming fact starts with no value-specific state, so value facts
    // still reset at every such entry. Each seed is consumed by its own block's walk.
    for (block_index, block) in hir.blocks.iter().enumerate() {
        analyser.current_block = block_index;
        analyser.cache = analyser.seeds.remove(&block.id);
        for statement in &block.statements {
            analyser.visit_statement(statement);
        }
        analyser.visit_terminator(&block.terminator);
    }

    analyser.facts
}

/// Incoming-edge facts for one block's seed eligibility.
///
/// WHY: seeding needs both facts together. A second terminator edge can deliver a different
///      value, and a callable starts its entry block without executing any edge transfer.
#[derive(Clone, Copy)]
struct IncomingEdgeFacts {
    /// Terminator edges targeting the block, counted through every terminator kind.
    count: u32,
    /// A callable starts this block directly rather than through a terminator edge.
    callable_entry: bool,
}

/// Counts incoming CFG edges for every dense post-compaction block id.
///
/// WHAT: one pass over every terminator target, including both Invoke edges, match arms,
///       conditional branches, jumps, breaks and continues, then flags every function entry.
/// WHY: seeding requires the successor to have no other way in, so the count must use the same
///      complete target set the rest of the compiler walks rather than selected edge kinds.
///      Dense post-compaction ids make the block index the block id.
fn count_incoming_edges(hir: &HirModule) -> Vec<IncomingEdgeFacts> {
    let mut incoming_edges = vec![
        IncomingEdgeFacts {
            count: 0,
            callable_entry: false,
        };
        hir.blocks.len()
    ];
    for block in &hir.blocks {
        for_each_terminator_target(&block.terminator, |target| {
            // Dense post-compaction ids make the index the block id. An out-of-range target can
            // only come from a malformed fixture, so it is ignored rather than panicking.
            if let Some(edges) = incoming_edges.get_mut(target.0 as usize) {
                edges.count = edges.count.saturating_add(1);
            }
        });
    }
    for function in &hir.functions {
        if let Some(edges) = incoming_edges.get_mut(function.entry.0 as usize) {
            edges.callable_entry = true;
        }
    }
    incoming_edges
}

/// One module analysis pass's conservative state.
struct BlockAnalyser<'a> {
    environment: &'a TypeEnvironment,
    expressions: &'a HirExpressionStore,
    /// Incoming-edge facts indexed by dense post-compaction block id.
    incoming_edges: &'a [IncomingEdgeFacts],
    /// Conservative enclosing entry intervals recorded by proven Invokes for their sole success
    /// blocks. Each seed is consumed by its block's walk; a seed recorded after that walk is
    /// discarded instead of retained.
    seeds: FxHashMap<BlockId, (LocalId, Interval)>,
    /// The single last-written-local interval cache. `None` means no value-specific fact.
    cache: Option<(LocalId, Interval)>,
    /// Dense index of the block currently being walked, used to reject late seeds.
    current_block: usize,
    facts: NumericProofs,
}

impl BlockAnalyser<'_> {
    /// Visits one statement, updating the proof table and the interval cache.
    fn visit_statement(&mut self, statement: &HirStatement) {
        match &statement.kind {
            HirStatementKind::Write { target, value } => {
                // Derive the source interval before any invalidation.
                let interval = self.expression_interval(*value);
                let target_local = match target {
                    HirWriteTarget::DefineLocal(local) => Some(*local),
                    HirWriteTarget::AssignPlace(place)
                        if self.expressions.projections(place.projections).is_empty() =>
                    {
                        Some(place.root)
                    }
                    HirWriteTarget::AssignPlace(_) => None,
                };
                if let Some(target_local) = target_local {
                    // Supported direct-local assignments retain only the destination's current
                    // interval; the one-entry cache makes every earlier fact unreachable.
                    self.cache = interval.map(|interval| (target_local, interval));
                } else {
                    // Projection writes may reach any value behind the base place.
                    self.cache = None;
                }
            }
            HirStatementKind::NumericOp {
                op,
                operands,
                result,
                ..
            } => {
                // Operand facts are derived before any invalidation.
                if let Some(result_interval) = self.prove_operation(op, operands) {
                    // Fast delivery is chosen by both scalar backends before the proof table, so
                    // a fast statement never publishes an operation fact. Its conservative
                    // enclosing result interval still feeds the one-entry cache: a proven interval
                    // fits the result domain, so the fast result cannot wrap and equals the
                    // mathematical result.
                    if !op.semantics.is_fast() {
                        self.facts.safe_operations.insert(statement.id);
                    }
                    // Statement operations write scalar results directly.
                    self.cache = Some((result.local(), result_interval));
                    return;
                }
                // Unproven operations conservatively drop the cache.
                self.cache = None;
            }
            // Loop-step failures always fail and have no numeric success interval to retain.
            HirStatementKind::LoopStepFailure { .. } => self.cache = None,
            // Calls, side-effect expressions, map operations, drops, float helpers and every
            // other unsupported statement conservatively drop value-specific facts.
            _ => self.cache = None,
        }
    }

    fn visit_terminator(&mut self, terminator: &HirTerminator) {
        let HirTerminator::Invoke {
            id,
            operation,
            success,
            success_results,
            ..
        } = terminator
        else {
            return;
        };
        // Read operand facts from this block before the edge transfer. A proof discharges
        // only the runtime predicate, never the failure edge or successor-local definitions.
        let result_interval = match operation {
            HirFallibleOperation::NumericOp { op, operands } => {
                let interval = self.prove_operation(op, operands);
                if interval.is_some() && !op.semantics.is_fast() {
                    self.facts.safe_operations.insert(*id);
                }
                interval
            }
            HirFallibleOperation::CastOp { policy, source } => {
                let interval = self.expression_interval(*source);
                match interval {
                    Some(interval) if self.cast_narrowing_is_provable(policy, interval) => {
                        self.facts.safe_narrowings.insert(*id);
                        Some(interval)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        // A sole success edge is the only way into its block, so the transfer's single result
        // local carries the proven interval when that block's walk starts. Another edge, a
        // callable entry, several success results or a block already walked before this Invoke
        // cannot establish that state; a later walk never revisits it.
        if let Some(interval) = result_interval
            && let [result] = success_results.as_slice()
            && let Some(edges) = self.incoming_edges.get(success.0 as usize)
            && edges.count == 1
            && !edges.callable_entry
            && success.0 as usize > self.current_block
        {
            self.seeds.insert(*success, (*result, interval));
        }
        self.cache = None;
    }

    /// Proves one checked numeric operation safe, returning a conservative enclosing result
    /// interval.
    ///
    /// `Power` never proves, and every non-integer domain (binary floats, exact
    /// `Dec`) stays checked. The returned interval must fit the operation domain, which is
    /// what makes the checked operation's failure impossible.
    fn prove_operation(
        &self,
        op: &HirNumericOp,
        operands: &HirNumericOperands,
    ) -> Option<Interval> {
        let domain = op.domain.integer_range()?;

        let interval = match op.operator {
            NumericOperator::Add | NumericOperator::Subtract | NumericOperator::Multiply => {
                let (left, right) = self.binary_intervals(operands)?;
                checked_result_range(op.operator, left, Some(right))?
            }
            NumericOperator::IntegerDivide => {
                let (left, right) = self.binary_intervals(operands)?;
                // A divisor interval that may contain zero can always fail.
                if interval_contains(right, 0) {
                    return None;
                }
                // Signed minimum divided by -1 overflows the domain (e.g. I64::MIN / -1).
                if domain.0 < 0 && interval_contains(left, domain.0) && interval_contains(right, -1)
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
                if interval_contains(right, 0) {
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
                // Only domains whose negation keeps the exact same type can prove here: unsigned
                // integers and `Byte` have no negation at all, and no signed or binary-float
                // domain widens. Binary floats and `Dec` still expose no bounded integer
                // interval, so their checks always stay.
                if negation_domain(op.domain) != Some(op.domain) {
                    return None;
                }
                let HirNumericOperands::Unary { operand } = operands else {
                    return None;
                };
                let interval = self.expression_interval(*operand)?;
                checked_result_range(op.operator, interval, None)?
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
            self.expression_interval(*left)?,
            self.expression_interval(*right)?,
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
        if !source.is_integer() {
            return false;
        }
        let Some((target_low, target_high)) = target.integer_range() else {
            return false;
        };
        source_interval.0 >= target_low && source_interval.1 <= target_high
    }

    /// Closed interval of one expression's runtime value, or `None` when it cannot bound one.
    ///
    /// Loads and copies use the current cache only for the exact direct local it names; every
    /// other value falls back to complete canonical integer type bounds.
    fn expression_interval(&self, expression_id: HirValueId) -> Option<Interval> {
        let expression = self.expressions.expression(expression_id);
        match &expression.kind {
            // Literals are singleton exact values.
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
                let target_range = target.integer_range()?;
                let source_interval = self.expression_interval(*source)?;
                let low = source_interval.0.max(target_range.0);
                let high = source_interval.1.min(target_range.1);
                (low <= high).then_some((low, high))
            }
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place)
                if self.expressions.projections(place.projections).is_empty() =>
            {
                match self.cache {
                    Some((cached_local, interval)) if cached_local == place.root => Some(interval),
                    _ => self.canonical_interval(expression.ty),
                }
            }
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
        NumericScalar::from_type_id(type_id, self.environment)?.integer_range()
    }
}

fn interval_contains((low, high): Interval, value: i128) -> bool {
    low <= value && value <= high
}
