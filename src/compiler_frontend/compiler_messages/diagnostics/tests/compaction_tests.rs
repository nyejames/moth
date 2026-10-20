//! Compaction behaviour: capture, the label role check, foreign sites and view round trips.
//!
//! WHAT: these tests build drafts through the family constructors, compact them into a store and read the stored
//! record back through the same views the renderer uses.
//! WHY: compaction is the only point where a draft becomes durable data, so its ordering and writes need direct
//! checks, not only codec checks.

use super::{TEST_LABEL_FAMILY, exact_span, identity};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayCapacityError, TypeDisplayId,
};
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    FactReader, TypeFact, UnitFact, type_display_word,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::unreachable_match_arm;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unused_import;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    duplicate_declaration, unknown_value_name,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    ExpectedTokenView, compiler_capacity_exceeded, expected_token_from_projections,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TypeMismatchView, type_mismatch,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::{
    ForeignSiteDraft, ForeignSlot, SiteRef,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DraftType;
use crate::compiler_frontend::compiler_messages::diagnostics::record::SeverityOverride;
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticToken, GenericSubstitutionDiagnostic, HirCapacityResource,
    TokenTag, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// The display ID that a type at `index` in the display table captures to.
fn display(index: usize) -> TypeDisplayId {
    TypeDisplayId::from_index(index).expect("small test indices fit the display table")
}

/// The fact or label word that a display ID at `index` writes.
fn display_word(index: usize) -> u32 {
    type_display_word(display(index)).expect("small test indices fit one fact word")
}

/// Captures every test type to its own index, which is enough to check the rewrite.
fn identity_rewrite(id: TypeId) -> Result<TypeDisplayId, TypeDisplayCapacityError> {
    TypeDisplayId::from_index(id.0 as usize)
}

#[test]
fn capture_rewrites_fact_type_words_and_clears_marks() {
    let mut draft = type_mismatch(
        TypeId(3),
        TypeId(5),
        TypeMismatchContext::Assignment,
        Some(exact_span(0, 1)),
    );
    draft
        .capture_types(&mut identity_rewrite)
        .expect("small test types always capture");

    assert_eq!(draft.core.pending_facts, 0);
    let reader = FactReader::new(&draft.core.facts);
    assert_eq!(reader.read::<TypeFact>().index(), 3);
    assert_eq!(reader.read::<TypeFact>().index(), 5);
    assert_eq!(
        reader.read::<UnitFact<TypeMismatchContext>>(),
        TypeMismatchContext::Assignment
    );
    reader.finish();
}

#[test]
fn capture_rewrites_label_and_substitution_type_words() {
    let span = exact_span(0, 4);
    let mut draft = DiagnosticDraft::with_facts(&TEST_LABEL_FAMILY, Some(span), |_cursor| Ok(()))
        .with_label(
            Some(span),
            Some(DiagnosticLabelMessage::TypedFailureProducer {
                error_type_id: TypeId(3),
            }),
        )
        .with_label(
            Some(span),
            Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions {
                substitutions: vec![GenericSubstitutionDiagnostic {
                    parameter_name: StringId::from_index(2),
                    concrete_type_id: TypeId(7),
                }],
            }),
        );
    draft
        .capture_types(&mut identity_rewrite)
        .expect("small test types always capture");

    let extra = draft
        .extra
        .as_deref()
        .expect("labels create the extra part");
    assert_eq!(extra.labels[0].pending_types, 0);
    assert_eq!(extra.labels[0].data[0], display_word(3));
    assert_eq!(
        extra.substitutions[0].concrete,
        DraftType::Captured(display_word(7))
    );
}

#[test]
fn failed_capture_leaves_the_draft_unchanged() {
    let mut draft = type_mismatch(
        TypeId(3),
        TypeId(9),
        TypeMismatchContext::Assignment,
        Some(exact_span(0, 1)),
    );
    let before = draft.core;
    let result = draft.capture_types(&mut |id: TypeId| {
        if id.0 == 9 {
            Err(TypeDisplayCapacityError)
        } else {
            TypeDisplayId::from_index(id.0 as usize)
        }
    });

    assert_eq!(result, Err(TypeDisplayCapacityError));
    assert_eq!(draft.core.facts, before.facts);
    assert_eq!(draft.core.pending_facts, before.pending_facts);
    assert_eq!(draft.core.pending_facts, 0b11);
}

#[test]
#[should_panic(expected = "capture_types")]
fn compacting_a_pending_draft_panics() {
    let mut store = DiagnosticStore::new();
    let draft = type_mismatch(
        TypeId(3),
        TypeId(5),
        TypeMismatchContext::Assignment,
        Some(exact_span(0, 1)),
    );
    let _outcome = store.compact(draft);
}

#[test]
#[should_panic(expected = "not declared")]
fn compacting_an_undeclared_label_message_panics() {
    let mut store = DiagnosticStore::new();
    let draft = unknown_value_name(StringId::from_index(1), None).with_label(
        Some(exact_span(0, 1)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    );
    let _outcome = store.compact(draft);
}

#[test]
fn captured_draft_compacts_and_views_read_display_ids() {
    let mut store = DiagnosticStore::new();
    let mut draft = type_mismatch(
        TypeId(3),
        TypeId(5),
        TypeMismatchContext::Assignment,
        Some(exact_span(0, 1)),
    );
    draft
        .capture_types(&mut identity_rewrite)
        .expect("small test types always capture");

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a small captured draft stores");
    };
    let view = TypeMismatchView::from_record(store.record(id))
        .expect("the stored record is a type mismatch");
    assert_eq!(view.expected.index(), 3);
    assert_eq!(view.found.index(), 5);
    assert_eq!(view.context, TypeMismatchContext::Assignment);
    assert!(matches!(store.record(id).primary(), SiteRef::Local(_)));
}

#[test]
fn plain_draft_grows_only_the_record_arena() {
    let mut store = DiagnosticStore::new();
    let outcome = store.compact(unreachable_match_arm(Some(exact_span(0, 1))));

    assert!(matches!(outcome, CompactOutcome::Stored(_)));
    assert_eq!(store.records.len(), 1);
    assert!(store.extras.is_empty());
    assert!(store.labels.is_empty());
    assert!(store.label_data.is_empty());
    assert!(store.string_ids.is_empty());
    assert!(store.substitutions.is_empty());
    assert!(store.foreign_sites.is_empty());
    assert!(store.identity_contexts.is_empty());
}

#[test]
fn foreign_primary_keeps_its_span_and_adds_a_site_row() {
    let mut store = DiagnosticStore::new();
    let draft =
        compiler_capacity_exceeded(HirCapacityResource::ExpressionRows, Some(exact_span(2, 3)))
            .with_foreign(ForeignSiteDraft {
                slot: ForeignSlot::Primary,
                context: identity(),
            });

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a foreign primary stores");
    };
    assert!(matches!(
        store.record(id).primary(),
        SiteRef::Foreign { span, context }
            if span == exact_span(2, 3) && std::ptr::eq(context, &store.identity_contexts[0])
    ));
    assert_eq!(store.identity_contexts.len(), 1);
}

#[test]
fn foreign_label_and_primary_share_one_identity_context() {
    let mut store = DiagnosticStore::new();
    let handle = identity();
    let draft = duplicate_declaration(StringId::from_index(1), Some(exact_span(10, 2)))
        .with_label(
            Some(exact_span(0, 2)),
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        )
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Label(0),
            context: handle.clone(),
        })
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle,
        });

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a foreign label stores");
    };
    let record = store.record(id);
    assert!(matches!(record.label_site(0), SiteRef::Foreign { .. }));
    assert!(matches!(record.primary(), SiteRef::Foreign { .. }));
    assert_eq!(record.labels()[0].span(), Some(exact_span(0, 2)));
    assert_eq!(store.identity_contexts.len(), 1);
}

#[test]
fn distinct_identities_get_distinct_context_rows() {
    let mut store = DiagnosticStore::new();
    let draft = duplicate_declaration(StringId::from_index(1), Some(exact_span(10, 2)))
        .with_label(
            Some(exact_span(0, 2)),
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        )
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Label(0),
            context: identity(),
        })
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: identity(),
        });

    assert!(matches!(store.compact(draft), CompactOutcome::Stored(_)));
    assert_eq!(store.identity_contexts.len(), 2);
}

#[test]
fn expected_token_view_reads_back_what_was_compacted() {
    let mut store = DiagnosticStore::new();
    let draft = expected_token_from_projections(
        DiagnosticToken::from_static_tag(TokenTag::COMMA),
        None,
        None,
    );

    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a token draft stores");
    };
    let view = ExpectedTokenView::from_record(store.record(id))
        .expect("the stored record is an expected token");
    assert_eq!(
        view.expected.raw_words(),
        DiagnosticToken::from_static_tag(TokenTag::COMMA).raw_words()
    );
    assert!(view.found.is_none());
}

#[test]
fn severity_overrides_reach_the_stored_record() {
    let mut store = DiagnosticStore::new();
    for (severity, resolved) in [
        (SeverityOverride::Default, DiagnosticSeverity::Warning),
        (SeverityOverride::Error, DiagnosticSeverity::Error),
        (SeverityOverride::Warning, DiagnosticSeverity::Warning),
        (SeverityOverride::Note, DiagnosticSeverity::Note),
    ] {
        let mut draft = unreachable_match_arm(Some(exact_span(0, 1)));
        draft.core.severity = severity;
        let CompactOutcome::Stored(id) = store.compact(draft) else {
            panic!("a plain draft must fit in an empty store");
        };
        assert_eq!(store.record(id).severity(), severity);
        assert_eq!(
            store
                .record(id)
                .severity()
                .resolve(DiagnosticSeverity::Warning),
            resolved
        );
    }
}

#[test]
fn reserved_import_row_compacts_like_any_other_row() {
    let mut store = DiagnosticStore::new();
    assert!(matches!(
        store.compact(unused_import(Some(exact_span(0, 1)))),
        CompactOutcome::Stored(_)
    ));
}
