//! Focused tests for the per-module generated-function transaction.
//!
//! WHAT: request registration, deduplication against already published work, boundary isolation
//!       and the diagnostic facts one request record owns.
//! WHY: reaching a module's generated fixed point is compiler semantics. Publication behaviour
//!      belongs to the build-owned boundary store and is tested there.

use super::*;
use crate::compiler_frontend::module_compilation::generated::GeneratedFunctionId;
use crate::compiler_frontend::module_compilation::generated::artefacts::CompletedGeneratedFunction;
use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::{
    facts, generated_identity, summary, test_sidecar,
};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

use rustc_hash::FxHashMap;

/// One already published boundary, as the transaction sees it.
struct PublishedBoundary {
    records: Vec<CompletedGeneratedFunction>,
    by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl PublishedBoundary {
    fn empty() -> Self {
        Self {
            records: Vec::new(),
            by_identity: FxHashMap::default(),
        }
    }

    fn with(identity: GeneratedFunctionIdentity, summary: PublicCallSummary) -> Self {
        let mut boundary = Self::empty();
        boundary
            .by_identity
            .insert(identity.clone(), GeneratedFunctionId::new(0));
        boundary.records.push(CompletedGeneratedFunction {
            identity: identity.clone(),
            summary: summary.clone(),
            sidecar: test_sidecar(identity, summary),
        });
        boundary
    }

    fn view(&self) -> KnownGeneratedFunctions<'_> {
        KnownGeneratedFunctions::new(&self.records, &self.by_identity)
    }
}

#[test]
fn registration_sorts_and_deduplicates_stable_identities_before_assigning_dense_ids() {
    let alpha = generated_identity("alpha");
    let beta = generated_identity("beta");
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());

    let ids = transaction.register_requests([
        facts("beta"),
        facts("alpha"),
        GeneratedRequestFacts {
            identity: beta,
            display_name: "beta".to_owned(),
            diagnostic_location: SourceLocation::new(
                InternedPath::from_single_str("src/@page.moth", &mut StringTable::new()),
                CharPosition::default(),
                CharPosition::default(),
            ),
        },
    ]);

    assert_eq!(ids, vec![GeneratedRequestId(0), GeneratedRequestId(1)]);
    assert_eq!(transaction.identity(ids[0]).unwrap(), &alpha);
    assert_eq!(transaction.records.len(), 2);
}

#[test]
fn repeated_request_registration_keeps_one_identity_state_record() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let parent_id = transaction.register_requests([facts("parent"), facts("parent")])[0];

    let child_ids = transaction.register_requests([facts("child"), facts("child"), facts("child")]);
    transaction.register_requests([facts("child")]);

    assert_eq!(child_ids.len(), 1);
    assert_eq!(parent_id, GeneratedRequestId(0));
    assert_eq!(child_ids[0], GeneratedRequestId(1));
    assert_eq!(transaction.records.len(), 2);
}

#[test]
fn completed_boundary_summary_suppresses_rematerialisation() {
    let identity = generated_identity("known");
    let expected = summary();
    let known = PublishedBoundary::with(identity.clone(), expected.clone());
    let mut transaction = GeneratedFunctionTransaction::new(known.view());

    let ids = transaction.register_requests([facts("known")]);

    assert!(ids.is_empty());
    assert_eq!(transaction.summary(&identity), Some(&expected));
    assert!(transaction.finish().is_ok());
}

#[test]
fn a_transaction_allocates_only_new_records() {
    let known_identity = generated_identity("known");
    let known = PublishedBoundary::with(known_identity, summary());
    let mut transaction = GeneratedFunctionTransaction::new(known.view());

    let known_ids = transaction.register_requests([facts("known")]);
    assert!(known_ids.is_empty());
    assert_eq!(
        transaction.records.len(),
        0,
        "known summaries seed the transaction"
    );

    let new_ids = transaction.register_requests([facts("new")]);
    assert_eq!(new_ids.len(), 1);
    assert_eq!(
        transaction.records.len(),
        1,
        "a session owns only its new delta"
    );
}

#[test]
fn request_records_own_diagnostic_facts() {
    let known = PublishedBoundary::empty();
    let mut transaction = GeneratedFunctionTransaction::new(known.view());
    let first_location = SourceLocation::new(
        crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
            "src/a.moth",
            &mut StringTable::new(),
        ),
        CharPosition {
            line_number: 3,
            char_column: 5,
        },
        CharPosition {
            line_number: 3,
            char_column: 9,
        },
    );
    let ids = transaction.register_requests([GeneratedRequestFacts {
        identity: generated_identity("make"),
        display_name: "make".to_owned(),
        diagnostic_location: first_location.clone(),
    }]);

    let (display_name, diagnostic_location) = transaction.request_facts(ids[0]).unwrap();
    assert_eq!(display_name, "make");
    assert_eq!(diagnostic_location, first_location);
}

#[test]
fn a_transaction_does_not_suppress_requests_known_only_in_another_boundary() {
    let identity = generated_identity("shared");
    let other_boundary = PublishedBoundary::with(identity.clone(), summary());
    let local_boundary = PublishedBoundary::empty();
    let mut local_transaction = GeneratedFunctionTransaction::new(local_boundary.view());

    let ids = local_transaction.register_requests([facts("shared")]);

    assert_eq!(
        ids.len(),
        1,
        "an identity completed in another boundary must still materialise locally"
    );
    assert!(local_transaction.summary(&identity).is_none());
    assert!(other_boundary.view().summary(&identity).is_some());
}

#[test]
fn transaction_summary_lookup_stays_inside_its_own_boundary() {
    let identity = generated_identity("shared");
    let first_summary = summary();
    let mut second_summary = summary();
    second_summary.parameters.push(
        crate::compiler_frontend::public_call_summary::PublicCallParameterSummary {
            access: crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Shared,
            mutation: crate::compiler_frontend::public_call_summary::PublicCallMutationEffect::NoWrite,
            transfer_eligibility: crate::compiler_frontend::public_call_summary::PublicCallTransferEligibility::Ineligible,
            transfer_effect: crate::compiler_frontend::public_call_summary::PublicCallTransferEffect::NeverConsumes,
            reactive_effect: crate::compiler_frontend::public_call_summary::PublicCallReactiveEffect::None,
        },
    );
    let first_boundary = PublishedBoundary::with(identity.clone(), first_summary.clone());
    let second_boundary = PublishedBoundary::with(identity.clone(), second_summary.clone());

    let first_transaction = GeneratedFunctionTransaction::new(first_boundary.view());
    let second_transaction = GeneratedFunctionTransaction::new(second_boundary.view());

    assert_eq!(first_transaction.summary(&identity), Some(&first_summary));
    assert_eq!(second_transaction.summary(&identity), Some(&second_summary));
    assert_ne!(
        first_transaction.summary(&identity),
        second_transaction.summary(&identity),
        "equal identities in unrelated boundaries must not share summaries"
    );
}
