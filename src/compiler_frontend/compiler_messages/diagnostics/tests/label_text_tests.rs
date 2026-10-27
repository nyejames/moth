//! Snapshot-backed label text for each durable label kind.
//!
//! WHAT: one table pairs each `LabelDraftMessage` kind with its exact text, then captures
//! the referenced types into a display snapshot before decoding the compact labels.
//! WHY: the snapshot is the sole source of type spelling after the producer's environment drops.

use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, DuplicateDeclarationView, InvalidAssignmentTargetView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DurableLabelMessage, GenericSubstitutionDiagnostic, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    compact_bag_for_test, exact_span_in_source,
};
use crate::compiler_frontend::compiler_messages::render::context::compact_label_message_text;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticCategory, DiagnosticDescriptor, DiagnosticPlace, DiagnosticSeverity,
    InvalidAssignmentTargetReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, SourceDatabase, SourceId,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::path::Path;

/// A test-only family that declares every label role needed to carry all fifteen label kinds in one draft.
/// The universal roles need no declaration. It is never registered, so no render path can reach it.
static ALL_LABEL_ROLES_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9002),
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: DiagnosticDescriptor::new(
        "MOTH-TEST-9002",
        "Test family",
        DiagnosticSeverity::Error,
    ),
    layout: &[],
    extra: ExtraDataKind::None,
    labels: &[
        LabelMessageCode::PREVIOUS_DECLARATION,
        LabelMessageCode::CONFLICTING_ACCESS,
        LabelMessageCode::VALUE_MOVED_HERE,
        LabelMessageCode::CONFIG_INPUT_ORIGIN,
        LabelMessageCode::TRAIT_REQUIREMENT_SITE,
        LabelMessageCode::RECEIVER_METHOD_SITE,
        LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE,
        LabelMessageCode::IMMUTABLE_BINDING_DECLARATION,
        LabelMessageCode::TYPED_FAILURE_PRODUCER,
        LabelMessageCode::IMPLICIT_FAILURE_PRODUCER,
        LabelMessageCode::BUILTIN_FAILURE_CALL,
        LabelMessageCode::BUILTIN_FAILURE_ORIGIN,
        LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
    ],
    render: None,
    reason_key: |_| None,
};

/// Each label kind with the exact text it renders from a captured type-display snapshot.
fn label_cases(
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> Vec<(LabelDraftMessage, String)> {
    let input_name = string_table.intern("max_length");
    let parameter = string_table.intern("T");
    let second_parameter = string_table.intern("U");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string_type = type_environment.builtins().string;

    vec![
        (
            LabelDraftMessage::PreviousDeclaration,
            "previous declaration here".to_owned(),
        ),
        (
            LabelDraftMessage::ConflictingAccess,
            "earlier conflicting access here".to_owned(),
        ),
        (
            LabelDraftMessage::ValueMovedHere,
            "value moved here".to_owned(),
        ),
        (
            LabelDraftMessage::ConfigInputOrigin {
                input_name,
                origin: BuildConfigValueOrigin::BuilderGlobal,
            },
            "configuration input `max_length` resolved from a builder-provided global".to_owned(),
        ),
        (
            LabelDraftMessage::TraitRequirementSite,
            "trait requirement".to_owned(),
        ),
        (
            LabelDraftMessage::ReceiverMethodSite,
            "receiver method".to_owned(),
        ),
        (
            LabelDraftMessage::GenericInstantiationBodySite,
            "generic body operation failed here".to_owned(),
        ),
        (
            LabelDraftMessage::GenericInstantiationDeclarationSite,
            "generic function declared here".to_owned(),
        ),
        (
            LabelDraftMessage::GenericInstantiationSubstitutions {
                substitutions: vec![
                    GenericSubstitutionDiagnostic {
                        parameter_name: parameter,
                        concrete_type_id: i32_type,
                    },
                    GenericSubstitutionDiagnostic {
                        parameter_name: second_parameter,
                        concrete_type_id: string_type,
                    },
                ],
            },
            "generic substitution: T = I32, U = String".to_owned(),
        ),
        (
            LabelDraftMessage::GenericInferencePreviousEvidence,
            "previous generic inference evidence here".to_owned(),
        ),
        (
            LabelDraftMessage::ImmutableBindingDeclaration,
            "immutable binding declared here".to_owned(),
        ),
        (
            LabelDraftMessage::TypedFailureProducer {
                error_type_id: i32_type,
            },
            "`I32!` failure produced here".to_owned(),
        ),
        (
            LabelDraftMessage::ImplicitFailureProducer,
            "implicit built-in failure may escape here".to_owned(),
        ),
        (
            LabelDraftMessage::BuiltinFailureCall,
            "implicit built-in failure propagates through this call".to_owned(),
        ),
        (
            LabelDraftMessage::BuiltinFailureOrigin,
            "original failing numeric or range operation here".to_owned(),
        ),
        (
            LabelDraftMessage::BuiltinWriteBackOrigin {
                target_type_id: string_type,
            },
            "write-back into `String` here after the compound update".to_owned(),
        ),
    ]
}

#[test]
fn compact_label_text_uses_captured_snapshot_for_every_label_kind() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let cases = label_cases(&mut string_table, &type_environment);
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);

    // Compact: every kind, then one message-less label, in one stored record.
    let mut draft = DiagnosticDraft::with_facts(&ALL_LABEL_ROLES_FAMILY, None, |_| Ok(()));
    for (message, _) in &cases {
        draft = draft.with_label(None, Some(message.clone()));
    }
    draft = draft.with_label(None, None);

    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&type_environment));
    draft
        .capture_types(&mut |type_id| builder.display_top_level(type_id))
        .expect("small test types always capture");
    let types = builder.finish();

    let mut store = DiagnosticStore::new();
    let CompactOutcome::Stored(id) = store.compact(draft, &mut Vec::new()) else {
        panic!("a small label draft stores");
    };
    let labels: Vec<_> = store.record(id, &[]).durable_labels().collect();
    assert_eq!(labels.len(), cases.len() + 1);
    for (label, (_, expected)) in labels.iter().zip(&cases) {
        assert_eq!(
            compact_label_message_text(label.message, &types, context).as_deref(),
            Some(expected.as_str()),
        );
    }
    assert_eq!(
        compact_label_message_text(labels[cases.len()].message, &types, context),
        None,
    );
}

#[test]
fn duplicate_declaration_with_previous_location_keeps_secondary_label() {
    let mut string_table = StringTable::new();
    let declaration_name = string_table.intern("Button");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let previous_location = exact_span_in_source(source, 0, 6, &mut span_builder);
    let duplicate_location = exact_span_in_source(source, 10, 6, &mut span_builder);
    let draft =
        names_declarations::duplicate_declaration(declaration_name, Some(duplicate_location))
            .with_label(
                Some(previous_location),
                Some(LabelDraftMessage::PreviousDeclaration),
            );
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let view = compact.first().expect("duplicate declaration compacts");
    let facts = view
        .family_view::<DuplicateDeclarationView>()
        .expect("duplicate declaration facts");
    assert_eq!(string_table.resolve(facts.name), "Button");
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == duplicate_location));
    let labels = view.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1, "only the secondary label is kept");
    assert!(matches!(labels[0].site, SiteRef::Local(span) if span == previous_location));
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::PreviousDeclaration
    ));
}

#[test]
fn duplicate_declaration_without_previous_location_omits_secondary_label() {
    let mut string_table = StringTable::new();
    let declaration_name = string_table.intern("print");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let duplicate_location = exact_span_in_source(source, 0, 5, &mut span_builder);
    let compact = compact_bag_for_test(
        DiagnosticBag::from(names_declarations::duplicate_declaration(
            declaration_name,
            Some(duplicate_location),
        )),
        None,
    );
    let view = compact.first().expect("duplicate declaration compacts");
    let facts = view
        .family_view::<DuplicateDeclarationView>()
        .expect("duplicate declaration facts");
    assert_eq!(string_table.resolve(facts.name), "print");
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == duplicate_location));
    assert_eq!(
        view.durable_labels().count(),
        0,
        "prelude symbols have no authored previous span"
    );
}

#[test]
fn immutable_binding_diagnostic_carries_secondary_declaration_label() {
    let mut string_table = StringTable::new();
    let source_path = Path::new("/project/main.moth");
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .expect("test source identity should build");
    let source = source_database
        .get_by_canonical_path(source_path)
        .expect("registered source")
        .id;
    source_database
        .retain_text(source, "value = value\n".to_owned())
        .expect("test source text");
    let value_name = string_table.intern("value");
    let mut span_builder = ExtendedSpanBuilder::new();
    let declaration_location = exact_span_in_source(source, 0, 5, &mut span_builder);
    let assignment_location = exact_span_in_source(source, 8, 5, &mut span_builder);
    let draft = names_declarations::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(value_name),
        None,
        None,
        Some(assignment_location),
    )
    .with_label(
        Some(declaration_location),
        Some(LabelDraftMessage::ImmutableBindingDeclaration),
    );
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let view = compact.first().expect("assignment-target draft compacts");
    assert!(view.family_view::<InvalidAssignmentTargetView>().is_some());
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == assignment_location));
    let frozen_identity = FrozenIdentityContext::from_parts(string_table, source_database);
    let context = DiagnosticRenderContext::new(frozen_identity.strings())
        .with_frozen_identity(&frozen_identity);
    let labels = terminal::format_label_messages_view(view, context);
    assert_eq!(labels, vec!["info: 1:1 - immutable binding declared here"]);
    let label = view
        .durable_labels()
        .next()
        .expect("secondary declaration label");
    assert!(matches!(label.site, SiteRef::Local(span) if span == declaration_location));
    assert!(matches!(
        label.message,
        DurableLabelMessage::ImmutableBindingDeclaration
    ));
}

#[test]
fn borrow_labels_preserve_order_and_exact_source_spans() {
    let primary_source = SourceId::from_index(1);
    let related_source = SourceId::from_index(2);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span_in_source(primary_source, 40, 8, &mut builder);
    let related = exact_span_in_source(related_source, 11, 7, &mut builder);
    let mut strings = StringTable::new();
    let value_name = strings.intern("value");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::multiple_mutable_borrows(
            DiagnosticPlace::Local(value_name),
            None,
            Some(related),
            Some(primary),
        )),
        None,
    );
    let view = compact.first().expect("borrow diagnostic compacts");
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == primary));
    let labels = view.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(matches!(labels[0].site, SiteRef::Local(span) if span == related));
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::ConflictingAccess
    ));
}
