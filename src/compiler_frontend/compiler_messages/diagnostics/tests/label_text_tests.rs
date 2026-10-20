//! Label text parity: compact label decoding and legacy rendering print the same text for every label kind.
//!
//! WHAT: one table pairs each `DiagnosticLabelMessage` kind with its exact text. The labels are decoded by
//! `compact_label_message_text`, then rendered on `InvalidDeclaration` with no payload; the family adapter declines
//! that diagnostic, so its labels still take the legacy text path.
//! WHY: the compact label decoder must read every field the legacy renderer reads, including labels on a declined
//! diagnostic.

use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::diagnostic_label::GenericSubstitutionDiagnostic;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::context::compact_label_message_text;
use crate::compiler_frontend::compiler_messages::render::terminal::format_label_messages_with_context;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticLabel, DiagnosticLabelMessage,
};
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
};

/// Each label kind with the exact text it renders. Type names resolve through `type_environment`, which is the
/// spelling a render with a live environment and a path table prints.
fn label_cases(
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> Vec<(DiagnosticLabelMessage, String)> {
    let input_name = string_table.intern("max_length");
    let rendered_text = string_table.intern("Consider a smaller range.");
    let parameter = string_table.intern("T");
    let second_parameter = string_table.intern("U");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string_type = type_environment.builtins().string;

    vec![
        (
            DiagnosticLabelMessage::PreviousDeclaration,
            "previous declaration here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::ConflictingAccess,
            "earlier conflicting access here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::ValueMovedHere,
            "value moved here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::ConfigInputOrigin {
                input_name,
                origin: BuildConfigValueOrigin::BuilderGlobal,
            },
            "configuration input `max_length` resolved from a builder-provided global".to_owned(),
        ),
        (
            DiagnosticLabelMessage::RenderedText(rendered_text),
            "Consider a smaller range.".to_owned(),
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationBodySite,
            "generic body operation failed here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationDeclarationSite,
            "generic function declared here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::GenericInstantiationSubstitutions {
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
            DiagnosticLabelMessage::GenericInferencePreviousEvidence,
            "previous generic inference evidence here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::ImmutableBindingDeclaration,
            "immutable binding declared here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::TypedFailureProducer {
                error_type_id: i32_type,
            },
            "`I32!` failure produced here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::ImplicitFailureProducer,
            "implicit built-in failure may escape here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::BuiltinFailureCall,
            "implicit built-in failure propagates through this call".to_owned(),
        ),
        (
            DiagnosticLabelMessage::BuiltinFailureOrigin,
            "original failing numeric or range operation here".to_owned(),
        ),
        (
            DiagnosticLabelMessage::BuiltinWriteBackOrigin {
                target_type_id: string_type,
            },
            "write-back into `String` here after the compound update".to_owned(),
        ),
    ]
}

#[test]
fn compact_and_legacy_label_text_agree_for_every_label_kind() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let cases = label_cases(&mut string_table, &type_environment);
    let path_fork = PathInternerFork::empty();
    let path_table = path_fork.snapshot_table();
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

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

    // Legacy: a test-only mismatched kind/payload keeps this generic label check on the legacy path.
    let carrier_labels: Vec<DiagnosticLabel> = cases
        .iter()
        .map(|(message, _)| DiagnosticLabel::secondary(None, Some(message.clone())))
        .collect();
    let diagnostic = CompilerDiagnostic::new(
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::InvalidDeclaration,
        ),
        None,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::None,
    )
    .with_labels(carrier_labels);
    let expected_lines: Vec<String> = cases
        .iter()
        .map(|(_, text)| format!("info: - {text}"))
        .collect();
    assert_eq!(
        format_label_messages_with_context(&diagnostic, context),
        expected_lines
    );
}
