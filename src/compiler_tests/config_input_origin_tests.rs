//! Real project builds for imported constant-overflow provenance.
//!
//! Each case builds one consumer operation in a fresh project so first-error handling cannot
//! hide the scalar or const-record provenance contract for either input origin.

use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildConfigValueOrigin, BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticLabelMessage, DiagnosticPayload, DiagnosticSeverity,
};
use crate::compiler_frontend::source::line_index::LinePosition;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use moth_lexical::numeric::profile::NumericProfile;
use std::fs;

#[test]
fn config_input_origin_labels_trace_facade_declaration_for_consumer_overflows() {
    assert_config_input_overflow_build(
        "maximum",
        "maximum + 1",
        19,
        BuildConfigInputSet::new(),
        BuildConfigValueOrigin::DeclarationDefault,
    );
    assert_config_input_overflow_build(
        "maximum",
        "maximum + 1",
        19,
        explicit_configured_maximum_input(),
        BuildConfigValueOrigin::ExplicitInput,
    );
    assert_config_input_overflow_build(
        "counts",
        "counts.maximum + 1",
        26,
        BuildConfigInputSet::new(),
        BuildConfigValueOrigin::DeclarationDefault,
    );
    assert_config_input_overflow_build(
        "counts",
        "counts.maximum + 1",
        26,
        explicit_configured_maximum_input(),
        BuildConfigValueOrigin::ExplicitInput,
    );
}

fn assert_config_input_overflow_build(
    imported_name: &str,
    expression: &str,
    operator_column: u32,
    inputs: BuildConfigInputSet,
    expected_origin: BuildConfigValueOrigin,
) {
    let temporary_directory = tempfile::tempdir().expect("should create temp dir");
    let root = temporary_directory.path();
    let facade_directory = root.join("src/facade");
    fs::create_dir_all(&facade_directory).expect("should create facade module folder");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"k1_diagnostic_regression\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");

    let consumer_source = format!(
        "@facade {imported_name}\n\noverflow || -> Int, Error!:\n    return {expression}\n;\n"
    );
    let consumer_path = root.join("src/@page.moth");
    fs::write(&consumer_path, consumer_source).expect("should write consumer source");
    let facade_path = facade_directory.join("@mod.moth");
    fs::write(
        &facade_path,
        "configured_maximum #Config of Int = 2147483647\n\nexport:\n    maximum #Int = configured_maximum\n    counts #= (maximum = configured_maximum)\n;\n",
    )
    .expect("should write facade source");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let Err(messages) = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &inputs,
    ) else {
        panic!("known consumer overflow should fail the build with source diagnostics");
    };

    assert_eq!(
        messages.error_count(),
        1,
        "each independent project should produce exactly one total error"
    );
    let errors = messages
        .diagnostics()
        .enumerate()
        .filter(|(_, diagnostic)| diagnostic.severity == DiagnosticSeverity::Error)
        .collect::<Vec<_>>();
    assert_eq!(
        errors.len(),
        1,
        "the error should be one source diagnostic, not an infrastructure failure"
    );
    let (diagnostic_index, diagnostic) = errors[0];
    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0053");
    let DiagnosticPayload::CompileTimeEvaluationError {
        reason,
        numeric_profile,
        ..
    } = &diagnostic.payload
    else {
        panic!("MOTH-RULE-0053 should carry its typed compile-time evaluation payload");
    };
    assert_eq!(*reason, CompileTimeEvaluationErrorReason::IntegerOverflow);
    assert_eq!(
        *numeric_profile,
        Some(NumericProfile::STANDARD),
        "the HtmlProjectBuilder default profile should be the reported boundary"
    );

    let render_context = messages.diagnostic_render_context(diagnostic_index);
    let primary_position = render_context
        .primary_position(diagnostic)
        .expect("authored overflow should resolve through its retained identity");
    let expected_consumer_path =
        fs::canonicalize(&consumer_path).expect("consumer source should canonicalize");
    assert_eq!(
        primary_position.host_path,
        Some(expected_consumer_path.as_path()),
        "the overflow primary must remain owned by the consumer module"
    );
    assert_eq!(
        primary_position.start,
        LinePosition {
            line: 3,
            column: operator_column,
        },
        "the primary operator position should stay exact"
    );
    assert_eq!(
        primary_position.end,
        LinePosition {
            line: 3,
            column: operator_column + 1,
        },
        "the primary span should cover only the authored operator"
    );
    assert_eq!(
        primary_position
            .line
            .get(operator_column as usize..operator_column as usize + 1),
        Some("+"),
        "the retained source snapshot should identify the authored overflow operator"
    );

    let (origin_label, origin) = diagnostic
        .labels
        .iter()
        .find_map(|label| {
            let Some(DiagnosticLabelMessage::ConfigInputOrigin { input_name, origin }) =
                label.message.as_ref()
            else {
                return None;
            };
            (render_context.string_table.resolve(*input_name) == "configured_maximum")
                .then_some((label, *origin))
        })
        .expect("the overflow should carry configured_maximum provenance");
    assert_eq!(
        origin, expected_origin,
        "the label should distinguish the declared default from the explicit command input"
    );

    let origin_position = render_context
        .label_position(origin_label)
        .expect("the matching provenance label should resolve through its retained identity");
    let expected_facade_path =
        fs::canonicalize(&facade_path).expect("facade source should canonicalize");
    assert_eq!(
        origin_position.host_path,
        Some(expected_facade_path.as_path()),
        "the matching label span must stay owned by the facade declaration"
    );
    assert_eq!(
        origin_position.start.line, 0,
        "the provenance anchor should resolve to the configured_maximum declaration"
    );
}

fn explicit_configured_maximum_input() -> BuildConfigInputSet {
    let mut inputs = BuildConfigInputSet::new();
    inputs
        .insert(BuildConfigInputEntry::new(
            BuildInputName::new("configured_maximum")
                .expect("declared input name should validate as lower_snake_case"),
            PrimitiveBuildValue::Int(2_147_483_647),
            BuildConfigValueLocation::Command(BuildCommandLocation::new(0)),
        ))
        .expect("a single builder input should not collide");
    inputs
}
