//! Real project builds for imported constant-overflow provenance.
//!
//! Each case builds one consumer operation in a fresh project so first-error handling cannot
//! hide the scalar or const-record provenance contract for either input origin.

use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildConfigValueOrigin, BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::source::line_index::LinePosition;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
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
        "@facade {imported_name}\n\noverflow || -> I32, Error!:\n    return {expression}\n;\n"
    );
    let consumer_path = root.join("src/@page.moth");
    fs::write(&consumer_path, consumer_source).expect("should write consumer source");
    let facade_path = facade_directory.join("@mod.moth");
    fs::write(
        &facade_path,
        "configured_maximum #Config of I32 = 2147483647\n\nexport:\n    maximum #I32 = configured_maximum\n    counts #= (maximum = configured_maximum)\n;\n",
    )
    .expect("should write facade source");

    let builder = ProjectBuilder::new(HtmlProjectBuilder::new());
    let reports = match build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &inputs,
    ) {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("known consumer overflow should fail the build with source diagnostics")
        }
        Err(errors) => {
            panic!("known consumer overflow produced infrastructure errors: {errors:?}")
        }
    };

    let errors = reports
        .iter_severity(DiagnosticSeverity::Error)
        .collect::<Vec<_>>();
    assert_eq!(
        errors.len(),
        1,
        "the error should be one source diagnostic, not an infrastructure failure"
    );
    let (report, view) = errors[0];
    assert_eq!(view.descriptor().code, "MOTH-RULE-0053");
    assert_eq!(
        view.reason_key(),
        Some("compile_time_evaluation_error.integer_overflow")
    );

    let render_context = DiagnosticRenderContext::for_report(report);
    let primary_position = render_context
        .site_position(view.primary_site())
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

    let (origin_label, origin) = view
        .durable_labels()
        .find_map(|label| {
            let DurableLabelMessage::ConfigInputOrigin { input_name, origin } = label.message
            else {
                return None;
            };
            (render_context.string_table.resolve(input_name) == "configured_maximum")
                .then_some((label, origin))
        })
        .expect("the overflow should carry configured_maximum provenance");
    assert_eq!(
        origin, expected_origin,
        "the label should distinguish the declared default from the explicit command input"
    );

    let origin_position = render_context
        .site_position(origin_label.site)
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
            PrimitiveBuildValue::Fixed(
                FixedScalarValue::signed(FixedScalar::I32, 2_147_483_647)
                    .expect("2_147_483_647 is in I32 range"),
            ),
            BuildConfigValueLocation::Command(BuildCommandLocation::new(0)),
        ))
        .expect("a single builder input should not collide");
    inputs
}
