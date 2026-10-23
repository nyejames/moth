//! Compaction behaviour: capture, the label role check, foreign sites and view round trips.
//!
//! WHAT: these tests build drafts through the family constructors, compact them into a store and read the stored
//! record back through the same views the renderer uses.
//! WHY: compaction is the only point where a draft becomes durable data, so its ordering and writes need direct
//! checks, not only codec checks.

use super::{TEST_LABEL_FAMILY, exact_span, identity};
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayCapacityError, TypeDisplayId, type_display_snapshot::TypeDisplaySnapshot,
    type_display_snapshot_builder::TypeDisplaySnapshotBuilder,
};
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    FactReader, NO_TYPE_WORD, OptTypeFact, TypeFact, UnitFact, type_display_from_word,
    type_display_word,
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
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DraftType, GenericSubstitutionDiagnostic, LabelDraftMessage, LabelMessageCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::{
    DiagnosticCode, SeverityOverride,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{FactDescriptor, FamilyRow};
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticDescriptor, DiagnosticSeverity, DiagnosticToken,
    HirCapacityResource, TokenTag, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

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

static TYPE_WORD_TEST_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9002),
    category: DiagnosticCategory::Type,
    reserved: false,
    descriptor: DiagnosticDescriptor::new(
        "MOTH-TEST-9002",
        "Type word test",
        DiagnosticSeverity::Error,
    ),
    layout: &[
        FactDescriptor::of::<TypeFact>(),
        FactDescriptor::of::<TypeFact>(),
    ],
    extra: ExtraDataKind::None,
    labels: &[LabelMessageCode::TYPED_FAILURE_PRODUCER],
    render: None,
    reason_key: |_| None,
};

static OPTIONAL_TYPE_TEST_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9003),
    category: DiagnosticCategory::Type,
    reserved: false,
    descriptor: DiagnosticDescriptor::new(
        "MOTH-TEST-9003",
        "Optional type word test",
        DiagnosticSeverity::Error,
    ),
    layout: &[FactDescriptor::of::<OptTypeFact>()],
    extra: ExtraDataKind::None,
    labels: &[],
    render: None,
    reason_key: |_| None,
};

/// Builds a draft with type words in facts, a label and a substitution.
fn draft_with_type_words(
    first: TypeId,
    second: TypeId,
    label_type: TypeId,
    substitution_type: TypeId,
) -> DiagnosticDraft {
    let span = exact_span(0, 1);
    DiagnosticDraft::with_facts(&TYPE_WORD_TEST_FAMILY, Some(span), |cursor| {
        cursor.push::<TypeFact>(&first)?;
        cursor.push::<TypeFact>(&second)
    })
    .with_label(
        Some(span),
        Some(LabelDraftMessage::TypedFailureProducer {
            error_type_id: label_type,
        }),
    )
    .with_label(
        Some(span),
        Some(LabelDraftMessage::GenericInstantiationSubstitutions {
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name: StringId::from_index(2),
                concrete_type_id: substitution_type,
            }],
        }),
    )
}

/// Returns all captured type words from the draft's facts, label data and substitution arena.
fn captured_type_words(draft: &DiagnosticDraft) -> Vec<u32> {
    let mut words = Vec::new();
    for (index, &word) in draft.core.facts.iter().enumerate() {
        if draft.core.type_facts & (1 << index) != 0 {
            words.push(word);
        }
    }
    if let Some(extra) = draft.extra.as_deref() {
        for label in &extra.labels {
            for (index, &word) in label.data.iter().enumerate() {
                if label.type_words & (1 << index) != 0 {
                    words.push(word);
                }
            }
        }
        for substitution in &extra.substitutions {
            let DraftType::Captured(word) = substitution.concrete else {
                panic!("test expected every substitution type to be captured");
            };
            words.push(word);
        }
    }
    words
}

/// Renders present type words, retaining `None` for an absent type.
fn render_type_words(
    snapshot: &TypeDisplaySnapshot,
    words: &[u32],
    strings: &StringTable,
    paths: &PathTable,
) -> Vec<Option<String>> {
    words
        .iter()
        .map(|&word| {
            type_display_from_word(word).map(|type_id| snapshot.display(type_id, strings, paths))
        })
        .collect()
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

    assert_eq!(draft.core.type_facts, 0b11);
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
            Some(LabelDraftMessage::TypedFailureProducer {
                error_type_id: TypeId(3),
            }),
        )
        .with_label(
            Some(span),
            Some(LabelDraftMessage::GenericInstantiationSubstitutions {
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
    assert_eq!(extra.labels[0].type_words, 0b01);
    assert_eq!(extra.labels[0].pending_types, 0);
    assert_eq!(extra.labels[0].data[0], display_word(3));
    assert_eq!(
        extra.substitutions[0].concrete,
        DraftType::Captured(display_word(7))
    );
}

#[test]
fn capture_rewrites_types_added_after_prior_capture_and_is_noop_when_complete() {
    let span = exact_span(0, 1);
    let mut draft = draft_with_type_words(TypeId(3), TypeId(4), TypeId(5), TypeId(6));
    draft
        .capture_types(&mut identity_rewrite)
        .expect("the initial test types capture");
    draft = draft
        .with_label(
            Some(span),
            Some(LabelDraftMessage::TypedFailureProducer {
                error_type_id: TypeId(8),
            }),
        )
        .with_label(
            Some(span),
            Some(LabelDraftMessage::GenericInstantiationSubstitutions {
                substitutions: vec![GenericSubstitutionDiagnostic {
                    parameter_name: StringId::from_index(3),
                    concrete_type_id: TypeId(9),
                }],
            }),
        );

    let mut rewritten = Vec::new();
    draft
        .capture_types(&mut |type_id| {
            rewritten.push(type_id.0);
            identity_rewrite(type_id)
        })
        .expect("newly added types capture after the earlier types");
    assert_eq!(rewritten, vec![8, 9]);
    assert_eq!(captured_type_words(&draft), vec![4, 5, 6, 9, 7, 10]);

    let mut rewrite_calls = 0;
    draft
        .capture_types(&mut |_type_id| {
            rewrite_calls += 1;
            Err(TypeDisplayCapacityError)
        })
        .expect("a draft without pending types is a no-op");
    assert_eq!(rewrite_calls, 0);
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
    assert_eq!(draft.core.type_facts, before.type_facts);
    assert_eq!(draft.core.facts, before.facts);
    assert_eq!(draft.core.pending_facts, before.pending_facts);
    assert_eq!(draft.core.pending_facts, 0b11);
}

#[test]
fn rebasing_captured_draft_keeps_every_type_spelling_after_snapshot_append() {
    let mut environment = TypeEnvironment::new();
    let bool_type = environment.builtins().bool;
    let string_type = environment.builtins().string;
    let range_type = environment.builtins().range;
    let optional_string_type = environment.intern_option(string_type);
    let collection_type = environment.intern_collection(bool_type, Some(3));

    let mut destination_builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    destination_builder
        .display_id(range_type)
        .expect("the destination snapshot fits");
    let mut merged = destination_builder.finish();

    let mut draft = draft_with_type_words(
        bool_type,
        optional_string_type,
        collection_type,
        string_type,
    );
    let mut source_builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    draft
        .capture_types(&mut |type_id| source_builder.display_id(type_id))
        .expect("the test types fit in a display snapshot");
    let source = source_builder.finish();
    let words_before = captured_type_words(&draft);
    let strings = StringTable::new();
    let paths = PathInternerBuilder::new().freeze();
    let spellings_before = render_type_words(&source, &words_before, &strings, &paths);

    let base = merged
        .append(source)
        .expect("the snapshots fit after append");
    assert_eq!(base, 1, "the destination already holds one type");
    draft
        .rebase_type_displays(base)
        .expect("captured display words fit after append");

    let words_after = captured_type_words(&draft);
    let spellings_after = render_type_words(&merged, &words_after, &strings, &paths);
    assert_eq!(spellings_after, spellings_before);
    assert_eq!(
        spellings_after.len(),
        4,
        "both facts, the label and substitution render"
    );
}

#[test]
fn rebasing_by_zero_is_identity_for_every_draft_type_word() {
    let mut draft = draft_with_type_words(TypeId(3), TypeId(4), TypeId(5), TypeId(6));
    draft
        .capture_types(&mut identity_rewrite)
        .expect("small test types always capture");
    let words_before = captured_type_words(&draft);

    draft
        .rebase_type_displays(0)
        .expect("rebasing by zero cannot overflow");

    assert_eq!(captured_type_words(&draft), words_before);
    assert_eq!(draft.core.type_facts, 0b11);
    assert_eq!(
        draft
            .extra
            .as_deref()
            .expect("type labels create the extra part")
            .labels[0]
            .type_words,
        0b01
    );
}

#[test]
fn rebasing_preserves_absent_type_words() {
    let absent: Option<TypeId> = None;
    let mut draft = DiagnosticDraft::with_facts(
        &OPTIONAL_TYPE_TEST_FAMILY,
        Some(exact_span(0, 1)),
        |cursor| cursor.push::<OptTypeFact>(&absent),
    );
    let mut rewrite_calls = 0;
    draft
        .capture_types(&mut |_type_id| {
            rewrite_calls += 1;
            Ok(display(0))
        })
        .expect("an absent type does not require a display id");

    assert_eq!(rewrite_calls, 0);
    assert_eq!(draft.core.facts[0], NO_TYPE_WORD);
    assert_eq!(draft.core.type_facts, 0b1);
    draft
        .rebase_type_displays(u32::MAX)
        .expect("absent type words are not offset");
    assert_eq!(draft.core.facts[0], NO_TYPE_WORD);
}

#[test]
fn rebase_overflow_leaves_facts_labels_and_substitutions_unchanged() {
    let mut draft = draft_with_type_words(TypeId(3), TypeId(4), TypeId(5), TypeId(6));
    let maximum_index =
        usize::try_from(u32::MAX - 1).expect("the maximum display index fits usize");
    draft
        .capture_types(&mut |type_id| {
            let index = if type_id.0 == 6 { maximum_index } else { 0 };
            TypeDisplayId::from_index(index)
        })
        .expect("the maximum one-based display word fits");

    let facts_before = draft.core.facts;
    let type_facts_before = draft.core.type_facts;
    let pending_facts_before = draft.core.pending_facts;
    let labels_before: Vec<_> = draft
        .extra
        .as_deref()
        .expect("labels create the extra part")
        .labels
        .iter()
        .map(|label| (label.type_words, label.pending_types, label.data))
        .collect();
    let substitutions_before: Vec<_> = draft
        .extra
        .as_deref()
        .expect("substitutions create the extra part")
        .substitutions
        .iter()
        .map(|entry| (entry.parameter, entry.concrete))
        .collect();

    assert_eq!(draft.rebase_type_displays(1), Err(TypeDisplayCapacityError));

    assert_eq!(draft.core.facts, facts_before);
    assert_eq!(draft.core.type_facts, type_facts_before);
    assert_eq!(draft.core.pending_facts, pending_facts_before);
    let extra = draft.extra.as_deref().expect("the extra remains present");
    assert_eq!(
        extra
            .labels
            .iter()
            .map(|label| (label.type_words, label.pending_types, label.data))
            .collect::<Vec<_>>(),
        labels_before
    );
    assert_eq!(
        extra
            .substitutions
            .iter()
            .map(|entry| (entry.parameter, entry.concrete))
            .collect::<Vec<_>>(),
        substitutions_before
    );
}

#[test]
#[should_panic(expected = "type-display rebasing needs `capture_types` to run first")]
fn rebasing_an_uncaptured_draft_panics() {
    let mut draft = draft_with_type_words(TypeId(3), TypeId(4), TypeId(5), TypeId(6));
    let _result = draft.rebase_type_displays(1);
}

#[test]
#[should_panic(expected = "type-display rebasing needs `capture_types` to run first")]
fn rebasing_a_draft_with_an_uncaptured_label_panics() {
    let span = exact_span(0, 1);
    let mut draft = DiagnosticDraft::with_facts(&TEST_LABEL_FAMILY, Some(span), |_cursor| Ok(()));
    draft
        .capture_types(&mut identity_rewrite)
        .expect("a draft with no types captures");
    let mut draft = draft.with_label(
        Some(span),
        Some(LabelDraftMessage::TypedFailureProducer {
            error_type_id: TypeId(5),
        }),
    );
    let _result = draft.rebase_type_displays(1);
}

#[test]
#[should_panic(expected = "type-display rebasing needs `capture_types` to run first")]
fn rebasing_a_draft_with_an_uncaptured_substitution_panics() {
    let span = exact_span(0, 1);
    let mut draft = DiagnosticDraft::with_facts(&TEST_LABEL_FAMILY, Some(span), |_cursor| Ok(()));
    draft
        .capture_types(&mut identity_rewrite)
        .expect("a draft with no types captures");
    let mut draft = draft.with_label(
        Some(span),
        Some(LabelDraftMessage::GenericInstantiationSubstitutions {
            substitutions: vec![GenericSubstitutionDiagnostic {
                parameter_name: StringId::from_index(2),
                concrete_type_id: TypeId(6),
            }],
        }),
    );
    let _result = draft.rebase_type_displays(1);
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
        Some(LabelDraftMessage::PreviousDeclaration),
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
            Some(LabelDraftMessage::PreviousDeclaration),
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
            Some(LabelDraftMessage::PreviousDeclaration),
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
