//! Focused tests for the requester-side Never proof union.
//!
//! WHAT: joining the requester's exact-target evidence with each materialised sidecar's evidence
//!       without merging path spellings that denote different generated instances.
//! WHY: `__generic_instance_<TypeId>` spellings are only unique inside one materialisation
//!      domain, so the union keys generated nodes by transaction request ID instead. A real
//!      cycle must still be joined across domains, and the reported index must name the
//!      requester declaration whenever one lies on the cycle.

use super::*;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::{
    PublishedBoundary, generated_identity,
};
use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedRequestFacts;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

fn path(index: usize) -> PathId {
    PathId::try_from_index(index).expect("fixture path fits")
}

fn proof_node(index: usize, dependencies: &[usize]) -> NeverProofNode {
    NeverProofNode {
        key: path(index),
        // The fixture names each node by its path index so failures identify the reported node.
        function_name: Some(StringId::from_index(index as u32)),
        span: None,
        dependencies: dependencies.iter().copied().map(path).collect(),
    }
}

fn request(name: &str, instance_path: usize) -> CanonicalGeneratedRequest {
    CanonicalGeneratedRequest {
        identity: generated_identity(name),
        instance_path: path(instance_path),
        function_name: Some(StringId::from_index(0)),
        call_span: None,
    }
}

/// Registers one domain's installed requests, as materialisation does before pushing it.
fn register(
    transaction: &mut GeneratedFunctionTransaction<'_>,
    requests: &[CanonicalGeneratedRequest],
) {
    transaction.register_requests(requests.iter().map(|request| GeneratedRequestFacts {
        identity: request.identity.clone(),
        display_name: String::new(),
        call_span: None,
    }));
}

fn id(transaction: &GeneratedFunctionTransaction<'_>, name: &str) -> GeneratedRequestId {
    transaction
        .request_id(&generated_identity(name))
        .expect("fixture identity is registered")
}

fn reported_index(proofs: &GeneratedNeverProofs) -> Option<usize> {
    proofs.cycle().map(|(index, _)| index)
}

#[test]
fn equal_spellings_in_different_domains_do_not_merge_without_a_real_back_edge() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    // The requester spells `a` at path 50 and its Never declaration 1 depends on that spelling.
    let requester_requests = [request("a", 50), request("b", 60)];
    register(&mut transaction, &requester_requests);
    let mut proofs = GeneratedNeverProofs::new(
        vec![proof_node(1, &[50])],
        &requester_requests,
        &transaction,
    );

    // Sidecar `b` spells a different identity `c` at the same path 50 in its fresh domain and
    // calls the requester's declaration. Treating 50 as one global node would invent a cycle.
    let b_requests = [request("c", 50)];
    register(&mut transaction, &b_requests);
    proofs.push_sidecar(
        (path(400), id(&transaction, "b")),
        vec![proof_node(400, &[1, 50])],
        &b_requests,
        &transaction,
    );
    // `c` also dispatches to the requester declaration; merging spelling 50 would close 1 -> c -> 1.
    proofs.push_sidecar(
        (path(402), id(&transaction, "c")),
        vec![proof_node(402, &[1])],
        &[],
        &transaction,
    );
    assert_eq!(
        reported_index(&proofs),
        None,
        "equal instance spellings from different domains must stay separate"
    );

    // The sidecar for `a` closes the real back edge: 1 -> a -> 1.
    proofs.push_sidecar(
        (path(401), id(&transaction, "a")),
        vec![proof_node(401, &[1])],
        &[],
        &transaction,
    );
    assert_eq!(
        reported_index(&proofs),
        Some(0),
        "the requester declaration that closes the real cycle is reported"
    );
}

#[test]
fn a_later_domain_spelling_joins_an_already_materialised_instance() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    // The requester spells `a` at 50 and `b` at 51; only `b` is on the requester's Never path.
    let requester_requests = [request("a", 50), request("b", 51)];
    register(&mut transaction, &requester_requests);
    let mut proofs = GeneratedNeverProofs::new(
        vec![proof_node(1, &[51])],
        &requester_requests,
        &transaction,
    );

    // Sidecar `a` materialises first and spells `b` at 70 in its own domain.
    let a_requests = [request("b", 70)];
    register(&mut transaction, &a_requests);
    proofs.push_sidecar(
        (path(400), id(&transaction, "a")),
        vec![proof_node(400, &[70])],
        &a_requests,
        &transaction,
    );
    // Sidecar `b` dispatches back to the requester declaration. The requester's spelling 51 and
    // the earlier sidecar's spelling 70 both denote `b`, so the cycle 1 -> b -> 1 is visible.
    proofs.push_sidecar(
        (path(401), id(&transaction, "b")),
        vec![proof_node(401, &[1])],
        &[],
        &transaction,
    );
    assert_eq!(reported_index(&proofs), Some(0));
}

#[test]
fn sidecars_sharing_one_generated_root_path_stay_separate() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let requester_requests = [request("a", 50), request("b", 60)];
    register(&mut transaction, &requester_requests);
    // The requester depends on the raw root path 400, which no domain installed as a spelling.
    let mut proofs = GeneratedNeverProofs::new(
        vec![proof_node(1, &[400])],
        &requester_requests,
        &transaction,
    );

    // Both sidecars report their own generated root at the same template path 400. Root paths
    // are not identities, so the roots must not collapse into one node and form 1 -> root -> 1.
    for name in ["a", "b"] {
        proofs.push_sidecar(
            (path(400), id(&transaction, name)),
            vec![proof_node(400, &[1])],
            &[],
            &transaction,
        );
    }
    assert_eq!(
        reported_index(&proofs),
        None,
        "equal generated root paths belong to different instances"
    );
}

#[test]
fn lowest_requester_index_on_the_cycle_is_reported() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let requester_requests = [request("a", 50)];
    register(&mut transaction, &requester_requests);
    // Nodes 11 and 12 form a cycle through generated instance `a`; node 10 is not cyclic.
    let requester = vec![
        proof_node(10, &[]),
        proof_node(11, &[50]),
        proof_node(12, &[11]),
    ];
    let mut proofs = GeneratedNeverProofs::new(requester, &requester_requests, &transaction);
    proofs.push_sidecar(
        (path(400), id(&transaction, "a")),
        vec![proof_node(400, &[12])],
        &[],
        &transaction,
    );
    assert_eq!(
        reported_index(&proofs),
        Some(1),
        "the reported index must name the lowest requester declaration on the cycle"
    );
}

#[test]
fn a_requester_without_never_declarations_ignores_sidecar_cycles() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let requester_requests = [request("a", 50)];
    register(&mut transaction, &requester_requests);
    let mut proofs = GeneratedNeverProofs::new(Vec::new(), &requester_requests, &transaction);

    // A sidecar-only self-cycle has no requester declaration to report; recursive
    // instantiation owns that case during materialisation.
    proofs.push_sidecar(
        (path(400), id(&transaction, "a")),
        vec![proof_node(400, &[400])],
        &[],
        &transaction,
    );
    assert_eq!(reported_index(&proofs), None);
}
