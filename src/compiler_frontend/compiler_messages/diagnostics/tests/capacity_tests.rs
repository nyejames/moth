//! Capacity behaviour: each arena ceiling rejects with its own resource, and a rejection latches the store full.
//!
//! WHAT: these tests shrink one ceiling at a time and compact a draft that needs that arena.
//! WHY: the capacity path is the only way a draft fails to store, so each ceiling needs a direct check that it names
//! the right resource and leaves the store in its documented state.

use super::{TEST_LABEL_FAMILY, exact_span, identity};
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    non_exhaustive_match, unreachable_match_arm,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::duplicate_declaration;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    CompilerCapacityExceededView, compiler_capacity_exceeded,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::{
    ForeignSiteDraft, ForeignSlot, SiteRef,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    GenericSubstitutionDiagnostic, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::diagnostics::store::{
    DiagnosticId, DiagnosticStore,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticCapacityResource, HirCapacityResource, NonExhaustiveMatchReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// A store with one ceiling changed from production. Each case gets its own store, because a rejection latches the
/// store full.
fn limited(change: impl FnOnce(&mut DiagnosticLimits)) -> DiagnosticStore {
    let mut limits = DiagnosticLimits::production();
    change(&mut limits);
    DiagnosticStore::with_limits(limits)
}

/// A capacity draft whose primary site is foreign, so compacting it needs both a foreign-site
/// row and a frozen identity context.
fn foreign_capacity_draft() -> DiagnosticDraft {
    compiler_capacity_exceeded(HirCapacityResource::ExpressionRows, Some(exact_span(2, 3)))
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: identity(),
        })
}

/// A draft on the test family with one label, so compacting it needs a label row and an extra part.
fn labelled_test_draft(message: LabelDraftMessage) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&TEST_LABEL_FAMILY, None, |_cursor| Ok(()))
        .with_label(Some(exact_span(0, 2)), Some(message))
}

#[test]
fn label_ceiling_rejects_with_labels() {
    let mut store = limited(|limits| limits.labels = 0);
    let draft = duplicate_declaration(StringId::from_index(1), Some(exact_span(10, 2))).with_label(
        Some(exact_span(0, 2)),
        Some(LabelDraftMessage::PreviousDeclaration),
    );

    assert_eq!(
        store.compact(draft),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Labels)
    );
}

#[test]
fn extra_ceiling_rejects_with_extras() {
    let mut store = limited(|limits| limits.extras = 0);
    let draft = duplicate_declaration(StringId::from_index(1), Some(exact_span(10, 2))).with_label(
        Some(exact_span(0, 2)),
        Some(LabelDraftMessage::PreviousDeclaration),
    );

    assert_eq!(
        store.compact(draft),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Extras)
    );
}

#[test]
fn string_id_ceiling_rejects_with_string_ids() {
    let mut store = limited(|limits| limits.string_ids = 0);
    let draft = non_exhaustive_match(
        NonExhaustiveMatchReason::MissingVariants,
        vec![StringId::from_index(1)],
        Some(exact_span(0, 2)),
    );

    assert_eq!(
        store.compact(draft),
        CompactOutcome::Rejected(DiagnosticCapacityResource::StringIds)
    );
}

#[test]
fn label_data_ceiling_rejects_with_label_data() {
    let mut store = limited(|limits| limits.label_data = 0);
    let draft = labelled_test_draft(LabelDraftMessage::ConfigInputOrigin {
        input_name: StringId::from_index(1),
        origin: BuildConfigValueOrigin::ExplicitInput,
    });

    assert_eq!(
        store.compact(draft),
        CompactOutcome::Rejected(DiagnosticCapacityResource::LabelData)
    );
}

#[test]
fn substitution_ceiling_rejects_with_substitutions() {
    let mut store = limited(|limits| limits.substitutions = 0);
    let mut draft = labelled_test_draft(LabelDraftMessage::GenericInstantiationSubstitutions {
        substitutions: vec![GenericSubstitutionDiagnostic {
            parameter_name: StringId::from_index(2),
            concrete_type_id: TypeId(7),
        }],
    });
    draft
        .capture_types(&mut |id: TypeId| TypeDisplayId::from_index(id.0 as usize))
        .expect("small test types always capture");

    assert_eq!(
        store.compact(draft),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Substitutions)
    );
}

#[test]
fn foreign_site_ceiling_rejects_with_foreign_sites() {
    let mut store = limited(|limits| limits.foreign_sites = 0);

    assert_eq!(
        store.compact(foreign_capacity_draft()),
        CompactOutcome::Rejected(DiagnosticCapacityResource::ForeignSites)
    );
}

#[test]
fn identity_context_ceiling_rejects_with_identity_contexts() {
    let mut store = limited(|limits| limits.identity_contexts = 0);

    assert_eq!(
        store.compact(foreign_capacity_draft()),
        CompactOutcome::Rejected(DiagnosticCapacityResource::IdentityContexts)
    );
}

#[test]
fn record_ceiling_stores_one_capacity_record_then_rejects_everything() {
    let mut store = limited(|limits| limits.records = 2);
    let span = exact_span(0, 1);

    assert!(matches!(
        store.compact(unreachable_match_arm(Some(span))),
        CompactOutcome::Stored(_)
    ));
    assert_eq!(
        store.compact(unreachable_match_arm(Some(span))),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Records)
    );
    assert_eq!(
        store.compact(unreachable_match_arm(Some(span))),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Records)
    );

    assert_eq!(store.records.len(), 2);
    let stand_in =
        CompilerCapacityExceededView::from_record(store.record(DiagnosticId::from_index(1)))
            .expect("the latched store ends with the capacity record");
    assert!(matches!(
        stand_in.resource,
        CapacityResource::Store(DiagnosticCapacityResource::Records)
    ));
    assert!(matches!(
        store.record(DiagnosticId::from_index(1)).primary(),
        SiteRef::Local(local) if local == span
    ));
}

#[test]
fn record_ceiling_stand_in_is_spanless_for_a_spanless_draft() {
    let mut store = limited(|limits| limits.records = 2);
    assert!(matches!(
        store.compact(unreachable_match_arm(None)),
        CompactOutcome::Stored(_)
    ));
    assert_eq!(
        store.compact(unreachable_match_arm(None)),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Records)
    );

    let record = store.record(DiagnosticId::from_index(1));
    CompilerCapacityExceededView::from_record(record)
        .expect("the latched store ends with the capacity record");
    assert!(matches!(record.primary(), SiteRef::Absent));
}

#[test]
fn record_ceiling_stand_in_drops_a_foreign_primary_span() {
    let mut store = limited(|limits| limits.records = 2);
    assert!(matches!(
        store.compact(unreachable_match_arm(Some(exact_span(0, 1)))),
        CompactOutcome::Stored(_)
    ));
    assert_eq!(
        store.compact(foreign_capacity_draft()),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Records)
    );

    let record = store.record(DiagnosticId::from_index(1));
    CompilerCapacityExceededView::from_record(record)
        .expect("the latched store ends with the capacity record");
    assert!(matches!(record.primary(), SiteRef::Absent));
}

#[test]
fn capacity_record_keeps_a_span_only_when_the_draft_had_one() {
    let mut store = DiagnosticStore::new();
    let CompactOutcome::Stored(spanless) = store.compact(compiler_capacity_exceeded(
        HirCapacityResource::ExpressionRows,
        None,
    )) else {
        panic!("a spanless capacity draft stores");
    };
    let CompactOutcome::Stored(spanned) = store.compact(compiler_capacity_exceeded(
        HirCapacityResource::ExpressionRows,
        Some(exact_span(0, 1)),
    )) else {
        panic!("a spanned capacity draft stores");
    };

    let spanless_record = store.record(spanless);
    CompilerCapacityExceededView::from_record(spanless_record)
        .expect("the stored record is a capacity record");
    assert!(matches!(spanless_record.primary(), SiteRef::Absent));
    let spanned_record = store.record(spanned);
    CompilerCapacityExceededView::from_record(spanned_record)
        .expect("the stored record is a capacity record");
    assert!(matches!(spanned_record.primary(), SiteRef::Local(_)));
}

#[cfg(target_pointer_width = "64")]
#[test]
fn oversized_count_becomes_a_fact_value_capacity_record_without_latching() {
    use crate::compiler_frontend::compiler_messages::InvalidMultiBindReason;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::invalid_multi_bind;

    let mut store = DiagnosticStore::new();
    let draft = invalid_multi_bind(
        InvalidMultiBindReason::ArityMismatch {
            expected: u32::MAX as usize + 1,
            found: 1,
        },
        None,
        Some(exact_span(0, 1)),
    );

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a fact fault stores a capacity record");
    };
    let view = CompilerCapacityExceededView::from_record(store.record(id))
        .expect("the stored record is a capacity record");
    assert!(matches!(
        view.resource,
        CapacityResource::Store(DiagnosticCapacityResource::FactValue)
    ));
    assert!(matches!(store.record(id).primary(), SiteRef::Local(_)));
    assert!(store.full.is_none());
}

#[cfg(target_pointer_width = "64")]
#[test]
fn fact_fault_drops_a_primary_marked_foreign_afterwards() {
    use crate::compiler_frontend::compiler_messages::InvalidMultiBindReason;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::invalid_multi_bind;

    let mut store = DiagnosticStore::new();
    let draft = invalid_multi_bind(
        InvalidMultiBindReason::ArityMismatch {
            expected: u32::MAX as usize + 1,
            found: 1,
        },
        None,
        Some(exact_span(0, 1)),
    )
    .with_foreign(ForeignSiteDraft {
        slot: ForeignSlot::Primary,
        context: identity(),
    });

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a fact fault stores a capacity record");
    };
    let record = store.record(id);
    CompilerCapacityExceededView::from_record(record)
        .expect("the stored record is a capacity record");
    assert!(matches!(record.primary(), SiteRef::Absent));
}
