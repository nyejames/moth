//! Requester-side Never proof union over generated instances.
//!
//! WHAT: joins the requester module's exact-target Never nodes with every sidecar it materialises
//!       and finds proof cycles that close through generated instances.
//! WHY:  the declaring module checks a generic template once, where a trait-bound call has no
//!       target. Only the requester sees which concrete implementation each instance dispatches
//!       to, so it owns the cycle check that spans its own declarations and generated bodies.

use crate::compiler_frontend::ast::statements::terminality::{NeverProofNode, never_proof_cycle};
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::module_compilation::generated::requests::CanonicalGeneratedRequest;
use crate::compiler_frontend::module_compilation::generated::transaction::{
    GeneratedFunctionTransaction, GeneratedRequestId,
};
use crate::compiler_frontend::symbols::path_interner::PathId;

/// Domain-qualified key for the requester's generated Never proof union.
///
/// WHAT: authored source declarations keep their path; generated instances use the request ID
///       the module's generated-function transaction registered for their identity.
/// WHY: `__generic_instance_<TypeId>` spellings are only unique inside one materialisation
///      domain. Each sidecar rebuilds its own `TypeEnvironment`, so the same path spelling can
///      denote different identities in the base module and in a sidecar. The transaction's
///      request ID stays equal for one instance across domains and distinct between instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum NeverProofKey {
    Declared(PathId),
    Generated(GeneratedRequestId),
}

/// Domain-aware Never proof accumulator for one requester module compilation.
///
/// WHAT: keeps the requester's exact-target nodes first, then every materialised sidecar's exact
///       nodes, translating each domain's path spellings through that domain's installed requests.
/// WHY: a sidecar's root is emitted at the template's `__generated_instance` path, while requester
///      Never calls target `__generic_instance_<arguments>` spellings produced by the requesting
///      domain's own `TypeEnvironment`. Translating with the domain that authored the spelling,
///      and keying generated identities by request ID, joins the two halves of a real cycle
///      without merging equal spellings that denote different instances in different domains.
///      A reportable cycle needs a requester node, so a module without declared Never functions
///      or generated requests keeps the accumulator empty and translates nothing.
pub(in crate::compiler_frontend::module_compilation) struct GeneratedNeverProofs {
    /// Requester nodes first; sidecar nodes follow in materialisation order.
    nodes: Vec<NeverProofNode<NeverProofKey>>,
    requester_count: usize,
}

impl GeneratedNeverProofs {
    /// Starts the union with the requester module's own exact-target nodes.
    ///
    /// `requester_requests` are the requests installed into the executable module AST; their
    /// paths are the only spellings that the requester's Never call targets can use. Call this
    /// after the transaction registered them.
    pub(in crate::compiler_frontend::module_compilation) fn new(
        requester_nodes: Vec<NeverProofNode>,
        requester_requests: &[CanonicalGeneratedRequest],
        transaction: &GeneratedFunctionTransaction<'_>,
    ) -> Self {
        let mut proofs = Self {
            nodes: Vec::new(),
            requester_count: 0,
        };
        if !requester_requests.is_empty() {
            proofs.push_domain(requester_nodes, requester_requests, transaction, None);
            proofs.requester_count = proofs.nodes.len();
        }
        proofs
    }

    /// Appends one sidecar's exact-target nodes using that sidecar domain's spellings.
    ///
    /// Call this after the transaction registered the sidecar's nested requests and before
    /// recursing into their materialisation. The sidecar root always receives its own request
    /// ID, so equal `__generated_instance` root paths across different instances cannot merge.
    pub(super) fn push_sidecar(
        &mut self,
        root: (PathId, GeneratedRequestId),
        nodes: Vec<NeverProofNode>,
        nested_requests: &[CanonicalGeneratedRequest],
        transaction: &GeneratedFunctionTransaction<'_>,
    ) {
        if self.requester_count != 0 {
            self.push_domain(nodes, nested_requests, transaction, Some(root));
        }
    }

    /// Returns the first cyclic proof, or `None` when no materialised instance contributed
    /// Never nodes, leaving the declaring module's template check as the sole authority.
    pub(in crate::compiler_frontend::module_compilation) fn cycle(
        &self,
    ) -> Option<(usize, DiagnosticDraft)> {
        if self.nodes.len() == self.requester_count {
            return None;
        }
        never_proof_cycle(&self.nodes, |key| key)
    }

    /// Number of leading nodes that belong to the requester module itself.
    pub(in crate::compiler_frontend::module_compilation) fn requester_count(&self) -> usize {
        self.requester_count
    }

    /// Translates one domain's exact-target nodes into the domain-qualified key space.
    ///
    /// `requests` are the domain's installed requests: a dependency on one of their spellings
    /// becomes that identity's request ID. An already published identity has no request here and
    /// no node in this compilation, so its spelling stays a dangling path. `root` carries the
    /// sidecar's own root path and request ID. Authored source nodes keep [`PathId`] keys: every
    /// domain interns authored functions in the shared requester path fork. Never bodies hold few
    /// calls, so a scan of the domain's requests avoids building a lookup table per domain.
    fn push_domain(
        &mut self,
        nodes: Vec<NeverProofNode>,
        requests: &[CanonicalGeneratedRequest],
        transaction: &GeneratedFunctionTransaction<'_>,
        root: Option<(PathId, GeneratedRequestId)>,
    ) {
        let translate = |path: PathId| {
            if let Some((root_path, root_id)) = root
                && path == root_path
            {
                return NeverProofKey::Generated(root_id);
            }
            requests
                .iter()
                .find(|request| request.instance_path == path)
                .and_then(|request| transaction.request_id(&request.identity))
                .map_or(NeverProofKey::Declared(path), NeverProofKey::Generated)
        };
        self.nodes
            .extend(nodes.into_iter().map(|node| NeverProofNode {
                key: translate(node.key),
                function_name: node.function_name,
                span: node.span,
                dependencies: node.dependencies.into_iter().map(translate).collect(),
            }));
    }
}

#[cfg(test)]
#[path = "tests/never_proofs_tests.rs"]
mod tests;
