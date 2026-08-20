//! Focused tests for the build-owned boundary generated store.
//!
//! WHAT: transactional publication, duplicate prevention and boundary isolation of completed
//!       generated records.
//! WHY: the boundary owns availability, deduplication, storage and publication. Reaching a
//!       module's generated fixed point is compiler work and is tested with its transaction owner.

use super::*;

use crate::compiler_frontend::module_compilation::generated::test_fixtures::{
    generated_identity, summary, test_sidecar,
};
use crate::compiler_frontend::module_compilation::{
    CompletedGeneratedFunction, GeneratedFunctionDelta,
};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;

fn store_with(
    identity: GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> BoundaryGeneratedFunctionStore {
    let mut store = BoundaryGeneratedFunctionStore::default();
    store.push_completed_for_test(CompletedGeneratedFunction {
        identity: identity.clone(),
        summary: summary.clone(),
        sidecar: test_sidecar(identity, summary),
    });
    store
}

#[test]
fn every_completed_record_has_one_identity_summary_and_sidecar() {
    let identity = generated_identity("record");
    let expected = summary();
    let store = store_with(identity.clone(), expected.clone());

    assert_eq!(store.records.len(), 1);
    assert_eq!(store.by_identity.len(), 1);
    assert_eq!(store.sidecars().count(), 1);
    assert_eq!(store.known_generated().summary(&identity), Some(&expected));
    assert_eq!(store.sidecar_at(0).unwrap().identity, identity);
}

#[test]
fn equal_generated_identities_publish_across_independent_boundaries() {
    let identity = generated_identity("shared");
    let mut first_store = BoundaryGeneratedFunctionStore::default();
    first_store
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            },
        ]))
        .unwrap();
    let mut second_store = BoundaryGeneratedFunctionStore::default();
    second_store
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            },
        ]))
        .unwrap();

    assert_eq!(first_store.sidecars().count(), 1);
    assert_eq!(second_store.sidecars().count(), 1);
    assert_eq!(first_store.sidecar_at(0).unwrap().identity, identity);
    assert_eq!(second_store.sidecar_at(0).unwrap().identity, identity);
}

#[test]
fn boundary_rejects_publishing_the_same_generated_identity_twice() {
    let identity = generated_identity("duplicate");
    let mut boundary = BoundaryGeneratedFunctionStore::default();
    boundary
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            },
        ]))
        .unwrap();

    let error = boundary
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity, summary()),
            },
        ]))
        .unwrap_err();

    assert!(error.msg.contains("published more than once"));
}

#[test]
fn late_generated_duplicate_leaves_existing_owners_unchanged() {
    let existing = generated_identity("existing");
    let late_duplicate = generated_identity("late");
    let mut store = BoundaryGeneratedFunctionStore::default();
    store
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: existing.clone(),
                summary: summary(),
                sidecar: test_sidecar(existing.clone(), summary()),
            },
        ]))
        .unwrap();
    let records_before = store.records.len();
    let sidecars_before = store.sidecars().count();

    let error = store
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: late_duplicate.clone(),
                summary: summary(),
                sidecar: test_sidecar(late_duplicate.clone(), summary()),
            },
            CompletedGeneratedFunction {
                identity: late_duplicate.clone(),
                summary: summary(),
                sidecar: test_sidecar(late_duplicate.clone(), summary()),
            },
        ]))
        .unwrap_err();

    assert!(
        error
            .msg
            .contains("duplicated inside one publication delta")
    );
    assert_eq!(
        store.records.len(),
        records_before,
        "a failing delta must not append any row"
    );
    assert_eq!(store.sidecars().count(), sidecars_before);
    assert!(store.known_generated().summary(&existing).is_some());
    assert!(store.by_identity.len() == 1);
}

#[test]
fn sidecar_record_identity_disagreement_leaves_store_unchanged() {
    let record_identity = generated_identity("record");
    let other_identity = generated_identity("other");
    let mut store = BoundaryGeneratedFunctionStore::default();

    let error = store
        .publish(GeneratedFunctionDelta::from_records(vec![
            CompletedGeneratedFunction {
                identity: record_identity,
                summary: summary(),
                sidecar: test_sidecar(other_identity, summary()),
            },
        ]))
        .unwrap_err();

    assert!(error.msg.contains("disagrees with its record identity"));
    assert!(store.records.is_empty());
    assert!(store.by_identity.is_empty());
    assert_eq!(store.sidecars().count(), 0);
}
