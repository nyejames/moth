//! Snapshot-backed label text for each durable label kind.
//!
//! WHAT: one table pairs each `LabelDraftMessage` kind with its exact text, then captures
//! the referenced types into a display snapshot before decoding the compact labels.
//! WHY: the snapshot is the sole source of type spelling after the producer's environment drops.

use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    GenericSubstitutionDiagnostic, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::context::compact_label_message_text;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// A test-only family that declares every label role needed to carry all fifteen label kinds in one draft.
/// The universal roles need no declaration. It is never registered, so no render path can reach it.
static ALL_LABEL_ROLES_FAMILY: FamilyRow = FamilyRow {
    code: DiagnosticCode::new(9002),
    category: DiagnosticCategory::Rule,
    reserved: false,
    descriptor: crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
        "MOTH-TEST-9002",
        "Test family",
        crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error,
    ),
    layout: &[],
    extra: ExtraDataKind::None,
    labels: &[
        LabelMessageCode::PREVIOUS_DECLARATION,
        LabelMessageCode::CONFLICTING_ACCESS,
        LabelMessageCode::VALUE_MOVED_HERE,
        LabelMessageCode::CONFIG_INPUT_ORIGIN,
        LabelMessageCode::RENDERED_TEXT,
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
    let rendered_text = string_table.intern("Consider a smaller range.");
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
            LabelDraftMessage::RenderedText(rendered_text),
            "Consider a smaller range.".to_owned(),
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
    let CompactOutcome::Stored(id) = store.compact(draft) else {
        panic!("a small label draft stores");
    };
    let labels: Vec<_> = store.record(id).durable_labels().collect();
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
