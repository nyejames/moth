//! Function-body terminality validation.
//!
//! WHAT: checks that a function body excludes reachable completions its contract forbids.
//! WHY: AST owns these user-facing rule diagnostics so HIR lowering only needs to handle
//! infrastructure invariants. Keeping the check at the AST stage lets body finalisation emit a
//! source-level diagnostic tied to the function declaration.
//!
//! The control-flow facts come from [`crate::compiler_frontend::ast::statements::control_flow_exits`].
//! This module owns only the policy (which function shapes require an explicit return or full
//! divergence, and which unannotated bodies must be rejected for silencing divergence) and the
//! diagnostics. It recognises any exit the shared analysis proves, including a normalised-true
//! loop with no reachable break.
//!
//! Never body proof boundary: divergence comes only from a folded-false `assert`, a folded-true
//! `loop` with no reachable `break`, and a standalone call whose published contract is `-> !`,
//! composed through scopes, `if`, match and statement sequencing. Divergence never propagates
//! through an ordinary contract, so no ordinary body can make a caller diverge.
//!
//! Recursive proof safety: an explicit signature is a contract for callers, not a proof for its
//! own body. [`never_proof_cycle`] rejects a self-edge or a mutually recursive group
//! whose divergence proof depends on its own unvalidated contracts. A recursive call after an
//! independent divergence is unreachable and therefore not a dependency.
//!
//! Two graphs are checked. The declaring module's template graph resolves concrete generic
//! instance calls to their template declarations and covers every declared `-> !` body, including
//! uncalled templates. A requester also unions its executable module's exact-target nodes with the
//! exact-target nodes of every generated generic sidecar it materialised, so a cycle that closes
//! through a trait-bound call inside a generated instance is rejected in the module whose source
//! proves it. Dependencies on imported providers stay trusted because their interfaces were
//! validated and published before the requester. Both graphs stay compiler-local and never enter
//! HIR or public interfaces.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::statements::control_flow_exits::{
    ControlFlowExitSummary, analyze_control_flow_exits,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::InvalidReturnShapeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::datatypes::FunctionReturnContract;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use rustc_hash::FxHashMap;
use std::hash::Hash;

/// Policy that decides which body completions a function contract forbids.
///
/// WHAT: distinguishes functions that may implicitly fall through, functions that must return on
/// every path, functions that must diverge on every path, unannotated functions that must not
/// silently diverge, and the special entry `start()` shape.
/// WHY: the policy is separate from the checker so callers can decide the shape based on the
/// function signature and origin without duplicating the control-flow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionTerminalityPolicy {
    /// The function has no required success-channel return values; falling off the end is allowed.
    AllowImplicitUnit,

    /// The function has one or more required success-channel return values; every path must
    /// reach a terminator before the body ends.
    RequireExplicitReturn,

    /// A declared `-> !` body: every reachable active path must diverge instead of completing,
    /// returning or resuming outside the body.
    RequireDivergence,

    /// An omitted return contract: the body must not diverge on every reachable active path,
    /// because that behaviour needs an explicit `-> !` contract or an ordinary return type.
    RejectUnannotatedDivergence,

    /// Entry `start()` is implicitly completed by returning the accumulated fragment vector.
    ///
    /// WHAT: preserves the existing entry-start contract where the compiler synthesizes the
    /// return value from top-level runtime fragments.
    /// WHY: users must not be required to author an explicit return of the fragment vector.
    EntryStartImplicitReturn,
}

/// Chooses the terminality policy for a function from its resolved signature and origin.
///
/// WHAT: maps the function signature to the appropriate policy.
/// WHY: this keeps policy selection in one place and makes the call sites read as a named step.
///      An ordinary `Returns` shape with no success slot and no error channel can only come from
///      omitted `->` user syntax: an authored ordinary contract must name at least one return
///      type, including the explicit no-success `-> Error!` form.
pub(crate) fn terminality_policy_for_signature(
    signature: &FunctionSignature,
    is_entry_start: bool,
) -> FunctionTerminalityPolicy {
    if is_entry_start {
        return FunctionTerminalityPolicy::EntryStartImplicitReturn;
    }

    match &signature.return_contract {
        FunctionReturnContract::Never => FunctionTerminalityPolicy::RequireDivergence,
        FunctionReturnContract::Returns(returns) => {
            if !returns.success().is_empty() {
                FunctionTerminalityPolicy::RequireExplicitReturn
            } else if returns.is_fallible() {
                FunctionTerminalityPolicy::AllowImplicitUnit
            } else {
                FunctionTerminalityPolicy::RejectUnannotatedDivergence
            }
        }
    }
}

/// Validates that a function body satisfies the requested terminality policy.
///
/// WHAT: asks the shared exit analysis whether any reachable path completes the way the policy
/// forbids and returns the matching diagnostic.
/// WHY: body parsing already owns user-facing diagnostics, so this is the natural place to
/// reject missing returns and invalid Never bodies before HIR lowering is invoked. The caller
/// runs this after static specialisation, so only active reachable paths participate in the
/// Never proof while ordinary checking already covered inactive branches.
pub(crate) fn validate_function_body_terminality(
    body: &[AstNode],
    policy: FunctionTerminalityPolicy,
    span: Option<SourceSpan>,
    function_name: Option<StringId>,
    expressions: &ExpressionStore,
    const_values: &ConstValueStore,
) -> Option<DiagnosticDraft> {
    if matches!(
        policy,
        FunctionTerminalityPolicy::AllowImplicitUnit
            | FunctionTerminalityPolicy::EntryStartImplicitReturn
    ) {
        return None;
    }
    let exits = analyze_control_flow_exits(body, expressions, const_values);
    validate_function_body_terminality_with_exits(exits, policy, span, function_name)
}

/// Validates a body against its policy using an already-computed exit summary.
///
/// WHAT: the policy half of terminality validation, without repeating the control-flow walk.
/// WHY: the Never proof graph already analyses every declared Never body; reusing that summary
///      keeps one analysis per body instead of two.
pub(crate) fn validate_function_body_terminality_with_exits(
    exits: ControlFlowExitSummary,
    policy: FunctionTerminalityPolicy,
    span: Option<SourceSpan>,
    function_name: Option<StringId>,
) -> Option<DiagnosticDraft> {
    match policy {
        FunctionTerminalityPolicy::AllowImplicitUnit
        | FunctionTerminalityPolicy::EntryStartImplicitReturn => None,

        FunctionTerminalityPolicy::RequireExplicitReturn => {
            if exits.may_complete_normally() {
                Some(calls_data::invalid_return_shape(
                    InvalidReturnShapeReason::FunctionMayFallThrough,
                    span,
                ))
            } else {
                None
            }
        }

        FunctionTerminalityPolicy::RequireDivergence => {
            let reason = if exits.returns_to_caller {
                InvalidReturnShapeReason::NeverBodyReturnsToCaller
            } else if exits.diverges_on_every_path() {
                return None;
            } else {
                InvalidReturnShapeReason::NeverBodyMayCompleteNormally
            };
            Some(calls_data::invalid_return_shape_named(
                reason,
                function_name,
                span,
            ))
        }

        FunctionTerminalityPolicy::RejectUnannotatedDivergence => {
            if exits.diverges_on_every_path() {
                Some(calls_data::invalid_return_shape_named(
                    InvalidReturnShapeReason::UnannotatedFunctionAlwaysDiverges,
                    function_name,
                    span,
                ))
            } else {
                None
            }
        }
    }
}

/// Divergence-proof dependencies of one declared `-> !` function.
///
/// WHAT: the function's own key and the standalone Never calls its reachable statement bodies
///       depend on.
/// WHY: recursive proof safety needs only an edge list per declaration. Keeping the node free of
///      validation state means the graph never leaks into HIR, public interfaces or backend facts.
///      `K` is the caller's node identity: the declaring module's graph uses [`PathId`], while a
///      requester's generated-instance union uses a domain-qualified key because the same path
///      spelling can denote different identities in different materialisation domains.
#[derive(Debug)]
pub(crate) struct NeverProofNode<K = PathId> {
    pub(crate) key: K,
    pub(crate) function_name: Option<StringId>,
    pub(crate) span: Option<SourceSpan>,
    /// Never call targets reached by a body that can still fall through, exactly as called.
    /// Repeats are harmless to the cycle search.
    pub(crate) dependencies: Vec<K>,
}

/// Rejects a `-> !` proof that depends on itself and names the reported node's index.
///
/// WHAT: the diagnostic for the first declared Never function, in node order, that lies on a
///       dependency cycle (a self-edge or a strongly connected group), plus its index in `nodes`.
///       `resolve` maps each recorded dependency onto the caller's node keys.
/// WHY: two or more declarations that only diverge because of each other's unvalidated contract
///      agree on an empty proof. A cycle is therefore a deferred proof, not an accepted program:
///      one member must diverge independently before the rest may rely on it.
///
/// Two callers key the graph differently. The declaring module resolves concrete generic
/// instance calls to their template declarations, so it proves every declaration, including
/// uncalled templates; dependencies on imported providers are trusted because their interfaces
/// were validated and published before this module. A requester unions its executable module's
/// exact-target nodes with every materialised generic sidecar's exact-target nodes, so a cycle
/// through a generic body's trait-bound call is rejected where the evidence lives. Requester
/// nodes come first, so the index tells that caller whether a source declaration owns the
/// report. An instance-only cycle needs instance-to-instance edges, which are nested requests,
/// so depth-first materialisation rejects it as recursive generic instantiation (MOTH-RULE-0057)
/// before this check runs.
///
/// One iterative Tarjan pass over a flattened edge list keeps the check linear in declarations
/// plus dependency edges; a per-declaration reachability search would rescan shared suffixes.
pub(crate) fn never_proof_cycle<K: Copy + Eq + Hash>(
    nodes: &[NeverProofNode<K>],
    resolve: impl Fn(K) -> K,
) -> Option<(usize, DiagnosticDraft)> {
    let first = first_cyclic_proof(nodes, resolve)?;
    let node = &nodes[first];
    let diagnostic = calls_data::invalid_return_shape_named(
        InvalidReturnShapeReason::NeverProofDependsOnCycle,
        node.function_name,
        node.span,
    );
    Some((first, diagnostic))
}

/// Lowest declaration index lying on a dependency cycle, if any.
fn first_cyclic_proof<K: Copy + Eq + Hash>(
    nodes: &[NeverProofNode<K>],
    resolve: impl Fn(K) -> K,
) -> Option<usize> {
    const UNVISITED: u32 = u32::MAX;

    let node_count = nodes.len();
    if node_count == 0 {
        return None;
    }

    let mut index_by_key: FxHashMap<K, usize> =
        FxHashMap::with_capacity_and_hasher(node_count, Default::default());
    for (index, node) in nodes.iter().enumerate() {
        index_by_key.insert(node.key, index);
    }

    // Edges of node `i` are `edge_targets[edge_offsets[i]..edge_offsets[i + 1]]`, resolved once.
    let mut edge_offsets = Vec::with_capacity(node_count + 1);
    let mut edge_targets = Vec::new();
    edge_offsets.push(0);
    for node in nodes {
        edge_targets.extend(
            node.dependencies
                .iter()
                .filter_map(|target| index_by_key.get(&resolve(*target)).copied()),
        );
        edge_offsets.push(edge_targets.len());
    }

    let mut discovery = vec![UNVISITED; node_count];
    let mut low_link = vec![0u32; node_count];
    let mut on_stack = vec![false; node_count];
    let mut component_stack = Vec::new();
    // (node, next edge cursor) frames replace recursion so deep chains cannot overflow.
    let mut frames: Vec<(usize, usize)> = Vec::new();
    let mut next_discovery = 0u32;
    let mut first_cyclic: Option<usize> = None;

    for root in 0..node_count {
        if discovery[root] != UNVISITED {
            continue;
        }
        discovery[root] = next_discovery;
        low_link[root] = next_discovery;
        next_discovery += 1;
        component_stack.push(root);
        on_stack[root] = true;
        frames.push((root, edge_offsets[root]));

        while let Some(&(node, cursor)) = frames.last() {
            if cursor < edge_offsets[node + 1] {
                let target = edge_targets[cursor];
                if let Some(frame) = frames.last_mut() {
                    frame.1 = cursor + 1;
                }
                if discovery[target] == UNVISITED {
                    discovery[target] = next_discovery;
                    low_link[target] = next_discovery;
                    next_discovery += 1;
                    component_stack.push(target);
                    on_stack[target] = true;
                    frames.push((target, edge_offsets[target]));
                } else if on_stack[target] {
                    low_link[node] = low_link[node].min(discovery[target]);
                }
                continue;
            }

            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low_link[parent] = low_link[parent].min(low_link[node]);
            }
            if low_link[node] != discovery[node] {
                continue;
            }

            // `node` roots a strongly connected component: pop it and test it for a cycle.
            let mut component_size = 0usize;
            let mut component_first = node;
            while let Some(member) = component_stack.pop() {
                on_stack[member] = false;
                component_size += 1;
                component_first = component_first.min(member);
                if member == node {
                    break;
                }
            }
            let self_edge =
                edge_targets[edge_offsets[node]..edge_offsets[node + 1]].contains(&node);
            if component_size > 1 || self_edge {
                first_cyclic =
                    Some(first_cyclic.map_or(component_first, |first| first.min(component_first)));
            }
        }
    }

    first_cyclic
}

#[cfg(test)]
#[path = "tests/terminality_tests.rs"]
mod terminality_tests;
