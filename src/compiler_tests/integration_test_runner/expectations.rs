//! Expectation file parsing for integration test cases.
//!
//! WHAT: reads and validates `expect.toml`, building typed expectation contracts per backend.
//! WHY: isolating TOML parsing here keeps fixture loading free of deserialization details and
//!      makes expectation format changes easy to find and update.

use super::path_validation::{CurrentDirectoryRule, validate_relative_path};
use super::types::{
    DiagnosticAssertion, DiagnosticMatchMode, ExactWarningExpectation, RenderedOutputExpectation,
    SecondaryLabelAssertion, SuccessContract,
};
use super::{
    ArtifactAssertion, ArtifactKind, BackendId, ExpectationMode, GoldenMode,
    ParsedBackendExpectation, ParsedExpectationFile, WarningExpectation,
};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_messages::is_well_formed_reason_key;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectationToml {
    mode: Option<ExpectationMode>,
    entry: Option<String>,
    #[serde(default)]
    flags: Vec<String>,
    builder: Option<String>,
    warnings: Option<String>,
    warning_codes: Option<Vec<String>>,
    #[serde(default)]
    message_contains: Vec<String>,
    #[serde(default)]
    artifact_assertions: Vec<ArtifactAssertionToml>,
    #[serde(default)]
    backends: BTreeMap<String, BackendExpectationToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackendExpectationToml {
    mode: ExpectationMode,
    #[serde(default)]
    flags: Vec<String>,
    success_contract: Option<String>,
    warnings: Option<String>,
    warning_codes: Option<Vec<String>>,
    #[serde(default)]
    message_contains: Vec<String>,
    #[serde(default)]
    diagnostic_codes: Vec<String>,
    #[serde(default)]
    diagnostic_assertions: Vec<DiagnosticAssertionToml>,
    diagnostic_match: Option<String>,
    diagnostic_match_reason: Option<String>,
    #[serde(default)]
    artifact_assertions: Vec<ArtifactAssertionToml>,
    golden_mode: Option<String>,
    #[serde(default)]
    rendered_output_exact: Option<String>,
    #[serde(default)]
    rendered_output_contains: Vec<String>,
    #[serde(default)]
    rendered_output_not_contains: Vec<String>,
    #[serde(default)]
    rendered_output_contains_in_order: Option<Vec<String>>,
    #[serde(default)]
    rendered_output_contains_exactly_once: Option<Vec<String>>,
    #[serde(default)]
    artifacts_must_not_exist: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticAssertionToml {
    code: String,
    occurrence: Option<usize>,
    reason: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    count: Option<usize>,
    #[serde(default)]
    secondary_labels: Vec<SecondaryLabelAssertionToml>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecondaryLabelAssertionToml {
    occurrence: Option<usize>,
    path: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactAssertionToml {
    path: String,
    kind: String,
    #[serde(default)]
    must_contain: Vec<String>,
    #[serde(default)]
    must_not_contain: Vec<String>,
    #[serde(default)]
    must_contain_in_order: Vec<String>,
    #[serde(default)]
    must_contain_exactly_once: Vec<String>,
    #[serde(default)]
    normalized_contains: Vec<String>,
    #[serde(default)]
    normalized_not_contains: Vec<String>,
    #[serde(default)]
    validate_wasm: bool,
    #[serde(default)]
    must_export: Vec<String>,
    #[serde(default)]
    must_import: Vec<String>,
}

pub(crate) fn parse_expectation_file(path: &Path) -> Result<ParsedExpectationFile, String> {
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read expectation file '{}': {error}",
            path.display()
        )
    })?;

    let parsed: ExpectationToml = toml::from_str(&source).map_err(|error| {
        format!(
            "Failed to parse expectation file '{}' as TOML: {error}",
            path.display()
        )
    })?;

    if let Some(builder) = &parsed.builder
        && builder != "html"
    {
        return Err(format!(
            "Expectation file '{}' only supports builder = \"html\" right now",
            path.display()
        ));
    }

    if parsed.backends.is_empty() {
        return Err(format!(
            "Expectation file '{}' must declare at least one '[backends.<id>]' section. Legacy top-level mode/flags/error fields are no longer supported.",
            path.display()
        ));
    }

    parse_matrix_expectation_file(path, parsed)
}

fn parse_matrix_expectation_file(
    path: &Path,
    parsed: ExpectationToml,
) -> Result<ParsedExpectationFile, String> {
    // In matrix mode, all mode/outcome keys must be declared inside explicit
    // backend sections so each backend can evolve independently.
    if parsed.mode.is_some()
        || !parsed.flags.is_empty()
        || parsed.warnings.is_some()
        || parsed.warning_codes.is_some()
        || !parsed.message_contains.is_empty()
        || !parsed.artifact_assertions.is_empty()
    {
        return Err(format!(
            "Expectation file '{}' uses backend matrix mode and must keep mode/warnings/flags/error/artifact keys inside '[backends.<id>]'.",
            path.display()
        ));
    }

    let mut backend_expectations = Vec::new();
    for (backend_key, backend_expectation) in parsed.backends {
        let backend_id = BackendId::parse(&backend_key).map_err(|error| {
            format!(
                "Expectation file '{}' has invalid backend key '{}': {error}",
                path.display(),
                backend_key
            )
        })?;
        let context = format!("[backends.{}]", backend_id.as_str());
        let warnings = parse_warning_expectation(
            backend_expectation.warnings.as_deref(),
            backend_expectation.warning_codes,
            path,
            &context,
        )?;
        let flags = parse_case_flags(&backend_expectation.flags, path, &context)?;
        let artifact_assertions =
            parse_artifact_assertions(path, &context, &backend_expectation.artifact_assertions)?;
        validate_code_identities(
            path,
            &context,
            "diagnostic_codes",
            &backend_expectation.diagnostic_codes,
        )?;
        let diagnostic_assertions = parse_diagnostic_assertions(
            path,
            &context,
            &backend_expectation.diagnostic_codes,
            &backend_expectation.diagnostic_assertions,
        )?;
        let success_contract = parse_success_contract(
            path,
            &context,
            backend_expectation.success_contract.as_deref(),
        )?;
        let diagnostic_match = parse_diagnostic_match_mode(
            path,
            &context,
            backend_expectation.diagnostic_match.as_deref(),
        )?;
        if backend_expectation.mode == ExpectationMode::Failure {
            validate_exact_diagnostic_match_reason(
                path,
                &context,
                diagnostic_match.unwrap_or(DiagnosticMatchMode::Exact),
                backend_expectation.diagnostic_match_reason.as_deref(),
            )?;
        }

        if backend_expectation.mode == ExpectationMode::Failure && success_contract.is_some() {
            return Err(format!(
                "Expectation file '{}' {} uses mode = \"failure\" and must not set 'success_contract'.",
                path.display(),
                context
            ));
        }

        let golden_mode =
            parse_golden_mode(path, &context, backend_expectation.golden_mode.as_deref())?;
        let rendered_output = parse_rendered_output_expectation(
            path,
            &context,
            backend_expectation.rendered_output_exact,
            backend_expectation.rendered_output_contains,
            backend_expectation.rendered_output_not_contains,
            backend_expectation.rendered_output_contains_in_order,
            backend_expectation.rendered_output_contains_exactly_once,
        )?;

        let has_authored_expected_warning = matches!(&warnings, WarningExpectation::Exact(_));
        if success_contract.is_some()
            && (!artifact_assertions.is_empty()
                || backend_expectation.golden_mode.is_some()
                || rendered_output.is_present()
                || !backend_expectation.artifacts_must_not_exist.is_empty()
                || !diagnostic_assertions.is_empty()
                || has_authored_expected_warning)
        {
            return Err(format!(
                "Expectation file '{}' {} declares success_contract = \"acceptance_only\" and must not combine it with artifact assertions, golden_mode, rendered-output assertions, artifact-absence assertions, or an authored expected-warning contract.",
                path.display(),
                context
            ));
        }

        // rendered_output_* is only valid for success mode; validate here so the
        // error message can reference the backend context.
        if backend_expectation.mode == ExpectationMode::Failure && rendered_output.is_present() {
            return Err(format!(
                "Expectation file '{}' {} uses mode = \"failure\" and must not set \
                 'rendered_output_exact', 'rendered_output_contains', \
                 'rendered_output_not_contains', 'rendered_output_contains_in_order', or \
                 'rendered_output_contains_exactly_once'.",
                path.display(),
                context
            ));
        }

        // artifacts_must_not_exist is a success-only negative contract.
        // Reject it in failure mode so absence expectations never couple to
        // diagnostic assertions.
        if backend_expectation.mode == ExpectationMode::Failure
            && !backend_expectation.artifacts_must_not_exist.is_empty()
        {
            return Err(format!(
                "Expectation file '{}' {} uses mode = \"failure\" and must not set \
                 'artifacts_must_not_exist'.",
                path.display(),
                context
            ));
        }

        let artifacts_must_not_exist = parse_artifacts_must_not_exist(
            path,
            &context,
            &backend_expectation.artifacts_must_not_exist,
        )?;

        backend_expectations.push(ParsedBackendExpectation {
            backend_id,
            flags,
            mode: backend_expectation.mode,
            warnings,
            success_contract,
            message_contains: backend_expectation.message_contains,
            diagnostic_codes: backend_expectation.diagnostic_codes,
            diagnostic_assertions,
            diagnostic_match,
            diagnostic_match_reason: backend_expectation.diagnostic_match_reason,
            artifact_assertions,
            golden_mode,
            rendered_output,
            artifacts_must_not_exist,
        });
    }

    Ok(ParsedExpectationFile {
        entry: parsed.entry,
        backend_expectations,
    })
}

fn parse_diagnostic_assertions(
    path: &Path,
    context: &str,
    authored_codes: &[String],
    assertions: &[DiagnosticAssertionToml],
) -> Result<Vec<DiagnosticAssertion>, String> {
    let mut authored_code_counts = BTreeMap::new();
    for code in authored_codes {
        *authored_code_counts.entry(code.as_str()).or_insert(0) += 1;
    }

    let mut selectors = BTreeSet::new();
    let mut parsed_assertions = Vec::with_capacity(assertions.len());

    for (index, assertion) in assertions.iter().enumerate() {
        let assertion_label = diagnostic_assertion_label(context, index);
        if assertion.code.trim().is_empty() {
            return Err(format!(
                "Expectation file '{}' {} requires a non-empty 'code'.",
                path.display(),
                assertion_label
            ));
        }

        let Some(&authored_count) = authored_code_counts.get(assertion.code.as_str()) else {
            return Err(format!(
                "Expectation file '{}' {} names diagnostic code '{}' which is absent from 'diagnostic_codes'.",
                path.display(),
                assertion_label,
                assertion.code
            ));
        };

        let occurrence = match assertion.occurrence {
            Some(0) => {
                return Err(format!(
                    "Expectation file '{}' {} must use a one-based 'occurrence'.",
                    path.display(),
                    assertion_label
                ));
            }
            Some(occurrence) => occurrence,
            None if authored_count == 1 => 1,
            None => {
                return Err(format!(
                    "Expectation file '{}' {} must author 'occurrence' because diagnostic code '{}' appears {} times in 'diagnostic_codes'.",
                    path.display(),
                    assertion_label,
                    assertion.code,
                    authored_count
                ));
            }
        };

        if occurrence > authored_count {
            return Err(format!(
                "Expectation file '{}' {} selects occurrence {} of diagnostic code '{}', but 'diagnostic_codes' contains it {} time(s).",
                path.display(),
                assertion_label,
                occurrence,
                assertion.code,
                authored_count
            ));
        }

        if !selectors.insert((assertion.code.clone(), occurrence)) {
            return Err(format!(
                "Expectation file '{}' {} duplicates diagnostic code '{}' occurrence {}.",
                path.display(),
                assertion_label,
                assertion.code,
                occurrence
            ));
        }

        validate_diagnostic_assertion_fields(path, &assertion_label, assertion)?;
        let secondary_labels =
            parse_secondary_label_assertions(path, &assertion_label, &assertion.secondary_labels)?;

        let has_structured_fact = assertion.reason.is_some()
            || assertion.path.is_some()
            || assertion.line.is_some()
            || assertion.column.is_some()
            || assertion.count.is_some()
            || !secondary_labels.is_empty();
        if !has_structured_fact {
            return Err(format!(
                "Expectation file '{}' {} must assert at least one structured diagnostic fact besides its selector.",
                path.display(),
                assertion_label
            ));
        }

        if let Some(count) = assertion.count
            && count != authored_count
        {
            return Err(format!(
                "Expectation file '{}' {} sets 'count = {}' for diagnostic code '{}', but 'diagnostic_codes' contains it {} time(s).",
                path.display(),
                assertion_label,
                count,
                assertion.code,
                authored_count
            ));
        }

        parsed_assertions.push(DiagnosticAssertion {
            code: assertion.code.clone(),
            occurrence,
            reason: assertion.reason.clone(),
            path: assertion.path.as_deref().map(portable_path_text),
            line: assertion.line,
            column: assertion.column,
            count: assertion.count,
            secondary_labels,
        });
    }

    Ok(parsed_assertions)
}

fn diagnostic_assertion_label(context: &str, index: usize) -> String {
    if context.is_empty() {
        format!("diagnostic_assertions[{index}]")
    } else {
        format!("{context}.diagnostic_assertions[{index}]")
    }
}

fn validate_diagnostic_assertion_fields(
    path: &Path,
    assertion_label: &str,
    assertion: &DiagnosticAssertionToml,
) -> Result<(), String> {
    if let Some(reason) = &assertion.reason {
        if reason.trim().is_empty() {
            return Err(format!(
                "Expectation file '{}' {} requires a non-empty 'reason'.",
                path.display(),
                assertion_label
            ));
        }

        if !is_well_formed_reason_key(reason) {
            return Err(format!(
                "Expectation file '{}' {} has invalid 'reason' '{}'; expected a qualified lowercase snake-case key.",
                path.display(),
                assertion_label,
                reason
            ));
        }
    }

    validate_diagnostic_path(path, assertion_label, assertion.path.as_deref())?;

    validate_positive_diagnostic_number(path, assertion_label, "line", assertion.line)?;
    validate_positive_diagnostic_number(path, assertion_label, "column", assertion.column)?;

    Ok(())
}

fn parse_secondary_label_assertions(
    path: &Path,
    diagnostic_assertion_label: &str,
    assertions: &[SecondaryLabelAssertionToml],
) -> Result<Vec<SecondaryLabelAssertion>, String> {
    let mut parsed_assertions = Vec::with_capacity(assertions.len());

    for (index, assertion) in assertions.iter().enumerate() {
        let assertion_label = format!("{diagnostic_assertion_label}.secondary_labels[{index}]");
        let occurrence = match assertion.occurrence {
            None => {
                return Err(format!(
                    "Expectation file '{}' {} requires a one-based 'occurrence'.",
                    path.display(),
                    assertion_label
                ));
            }
            Some(0) => {
                return Err(format!(
                    "Expectation file '{}' {} must use a one-based 'occurrence'.",
                    path.display(),
                    assertion_label
                ));
            }
            Some(occurrence) => occurrence,
        };

        validate_diagnostic_path(path, &assertion_label, assertion.path.as_deref())?;

        validate_positive_diagnostic_number(path, &assertion_label, "line", assertion.line)?;
        validate_positive_diagnostic_number(path, &assertion_label, "column", assertion.column)?;

        if assertion.path.is_none() && assertion.line.is_none() && assertion.column.is_none() {
            return Err(format!(
                "Expectation file '{}' {} must assert at least one secondary-label location fact ('path', 'line', or 'column').",
                path.display(),
                assertion_label
            ));
        }

        parsed_assertions.push(SecondaryLabelAssertion {
            occurrence,
            path: assertion.path.as_deref().map(portable_path_text),
            line: assertion.line,
            column: assertion.column,
        });
    }

    Ok(parsed_assertions)
}

fn validate_diagnostic_path(
    expectation_path: &Path,
    assertion_label: &str,
    expected_path: Option<&str>,
) -> Result<(), String> {
    let Some(expected_path) = expected_path else {
        return Ok(());
    };

    if expected_path.trim().is_empty() {
        return Err(format!(
            "Expectation file '{}' {} requires a non-empty 'path'.",
            expectation_path.display(),
            assertion_label
        ));
    }

    let field_name = format!("{assertion_label} 'path'");
    validate_relative_path(expected_path, &field_name, CurrentDirectoryRule::Forbid).map_err(
        |error| {
            format!(
                "Expectation file '{}': {error}.",
                expectation_path.display()
            )
        },
    )
}

fn validate_positive_diagnostic_number(
    path: &Path,
    assertion_label: &str,
    field_name: &str,
    value: Option<usize>,
) -> Result<(), String> {
    if value == Some(0) {
        return Err(format!(
            "Expectation file '{}' {} requires a positive '{}' value.",
            path.display(),
            assertion_label,
            field_name
        ));
    }

    Ok(())
}

fn parse_artifact_assertions(
    path: &Path,
    context: &str,
    assertions: &[ArtifactAssertionToml],
) -> Result<Vec<ArtifactAssertion>, String> {
    let mut parsed_assertions = Vec::with_capacity(assertions.len());

    for (index, assertion) in assertions.iter().enumerate() {
        let assertion_label = artifact_assertion_label(context, index);

        if assertion.path.trim().is_empty() {
            return Err(format!(
                "Expectation file '{}' {} requires a non-empty 'path'.",
                path.display(),
                assertion_label
            ));
        }

        let kind = parse_artifact_kind(path, &assertion.kind, &assertion_label)?;
        validate_artifact_assertion_fields(path, &assertion_label, assertion)?;
        validate_artifact_assertion_shape(path, &assertion_label, kind, assertion)?;

        parsed_assertions.push(ArtifactAssertion {
            path: portable_path_text(&assertion.path),
            kind,
            must_contain: assertion.must_contain.clone(),
            must_not_contain: assertion.must_not_contain.clone(),
            must_contain_in_order: assertion.must_contain_in_order.clone(),
            must_contain_exactly_once: assertion.must_contain_exactly_once.clone(),
            normalized_contains: assertion.normalized_contains.clone(),
            normalized_not_contains: assertion.normalized_not_contains.clone(),
            validate_wasm: assertion.validate_wasm,
            must_export: assertion.must_export.clone(),
            must_import: assertion.must_import.clone(),
        });
    }

    Ok(parsed_assertions)
}

fn artifact_assertion_label(context: &str, index: usize) -> String {
    if context.is_empty() {
        format!("artifact_assertions[{index}]")
    } else {
        format!("{context}.artifact_assertions[{index}]")
    }
}

fn validate_artifact_assertion_fields(
    path: &Path,
    assertion_label: &str,
    assertion: &ArtifactAssertionToml,
) -> Result<(), String> {
    for (field_name, values) in [
        ("must_contain", &assertion.must_contain),
        ("must_not_contain", &assertion.must_not_contain),
        ("must_contain_in_order", &assertion.must_contain_in_order),
        (
            "must_contain_exactly_once",
            &assertion.must_contain_exactly_once,
        ),
        ("normalized_contains", &assertion.normalized_contains),
        (
            "normalized_not_contains",
            &assertion.normalized_not_contains,
        ),
        ("must_export", &assertion.must_export),
        ("must_import", &assertion.must_import),
    ] {
        validate_artifact_strings(path, assertion_label, field_name, values)?;
    }

    Ok(())
}

fn validate_artifact_assertion_shape(
    path: &Path,
    assertion_label: &str,
    kind: ArtifactKind,
    assertion: &ArtifactAssertionToml,
) -> Result<(), String> {
    match kind {
        ArtifactKind::Html | ArtifactKind::Js => {
            if assertion.validate_wasm
                || !assertion.must_export.is_empty()
                || !assertion.must_import.is_empty()
            {
                return Err(format!(
                    "Expectation file '{}' {} uses wasm-only fields on a text artifact assertion.",
                    path.display(),
                    assertion_label
                ));
            }
            if assertion.must_contain.is_empty()
                && assertion.must_not_contain.is_empty()
                && assertion.must_contain_in_order.is_empty()
                && assertion.must_contain_exactly_once.is_empty()
                && assertion.normalized_contains.is_empty()
                && assertion.normalized_not_contains.is_empty()
            {
                return Err(format!(
                    "Expectation file '{}' {} must define at least one of 'must_contain', \
                     'must_not_contain', 'must_contain_in_order', 'must_contain_exactly_once', \
                     'normalized_contains', or 'normalized_not_contains' for text artifacts.",
                    path.display(),
                    assertion_label
                ));
            }
        }
        ArtifactKind::Wasm => {
            if !assertion.must_contain.is_empty()
                || !assertion.must_not_contain.is_empty()
                || !assertion.must_contain_in_order.is_empty()
                || !assertion.must_contain_exactly_once.is_empty()
                || !assertion.normalized_contains.is_empty()
                || !assertion.normalized_not_contains.is_empty()
            {
                return Err(format!(
                    "Expectation file '{}' {} uses text-only fields on a wasm artifact assertion.",
                    path.display(),
                    assertion_label
                ));
            }
            if !assertion.validate_wasm
                && assertion.must_export.is_empty()
                && assertion.must_import.is_empty()
            {
                return Err(format!(
                    "Expectation file '{}' {} must enable 'validate_wasm' or require imports/exports for wasm assertions.",
                    path.display(),
                    assertion_label
                ));
            }
        }
        ArtifactKind::Binary => {
            if !assertion.must_contain.is_empty()
                || !assertion.must_not_contain.is_empty()
                || !assertion.must_contain_in_order.is_empty()
                || !assertion.must_contain_exactly_once.is_empty()
                || !assertion.normalized_contains.is_empty()
                || !assertion.normalized_not_contains.is_empty()
                || assertion.validate_wasm
                || !assertion.must_export.is_empty()
                || !assertion.must_import.is_empty()
            {
                return Err(format!(
                    "Expectation file '{}' {} uses text-only or wasm-only fields on a binary artifact assertion.",
                    path.display(),
                    assertion_label
                ));
            }
        }
    }

    Ok(())
}

fn validate_artifact_strings(
    path: &Path,
    assertion_label: &str,
    field_name: &str,
    values: &[String],
) -> Result<(), String> {
    for value in values {
        if value.is_empty() {
            return Err(format!(
                "Expectation file '{}' {} contains an empty '{}' value.",
                path.display(),
                assertion_label,
                field_name
            ));
        }
    }

    Ok(())
}

fn parse_golden_mode(
    path: &Path,
    context: &str,
    raw: Option<&str>,
) -> Result<Option<GoldenMode>, String> {
    match raw {
        None => Ok(None),
        Some("strict") => Ok(Some(GoldenMode::Strict)),
        Some("normalized") => Ok(Some(GoldenMode::Normalized)),
        Some(other) => Err(format!(
            "Expectation file '{}' {} has unsupported golden_mode '{other}'. \
             Supported values: \"strict\", \"normalized\".",
            path.display(),
            context
        )),
    }
}

fn parse_success_contract(
    path: &Path,
    context: &str,
    raw: Option<&str>,
) -> Result<Option<SuccessContract>, String> {
    match raw {
        None => Ok(None),
        Some("acceptance_only") => Ok(Some(SuccessContract::AcceptanceOnly)),
        Some(other) => Err(format!(
            "Expectation file '{}' {} has unsupported success_contract '{other}'. Supported values: \"acceptance_only\".",
            path.display(),
            context
        )),
    }
}

fn parse_diagnostic_match_mode(
    path: &Path,
    context: &str,
    raw: Option<&str>,
) -> Result<Option<DiagnosticMatchMode>, String> {
    match raw {
        None => Ok(None),
        Some("exact") => Ok(Some(DiagnosticMatchMode::Exact)),
        Some("contains") => Ok(Some(DiagnosticMatchMode::Contains)),
        Some(other) => Err(format!(
            "Expectation file '{}' {} has unsupported diagnostic_match '{}'. Supported values: \"exact\", \"contains\".",
            path.display(),
            context,
            other
        )),
    }
}

fn validate_exact_diagnostic_match_reason(
    path: &Path,
    context: &str,
    mode: DiagnosticMatchMode,
    reason: Option<&str>,
) -> Result<(), String> {
    if mode == DiagnosticMatchMode::Exact && reason.is_some() {
        return Err(format!(
            "Expectation file '{}' {} uses diagnostic_match = \"exact\" and must not set 'diagnostic_match_reason'.",
            path.display(),
            context
        ));
    }

    Ok(())
}

fn parse_rendered_output_expectation(
    path: &Path,
    context: &str,
    exact: Option<String>,
    contains: Vec<String>,
    not_contains: Vec<String>,
    contains_in_order: Option<Vec<String>>,
    contains_exactly_once: Option<Vec<String>>,
) -> Result<RenderedOutputExpectation, String> {
    if exact.is_some()
        && (!contains.is_empty()
            || !not_contains.is_empty()
            || contains_in_order.is_some()
            || contains_exactly_once.is_some())
    {
        return Err(format!(
            "Expectation file '{}' {} sets 'rendered_output_exact' and must not combine it with any other rendered-output assertion field.",
            path.display(),
            context
        ));
    }

    validate_rendered_output_strings(path, context, "rendered_output_contains", &contains)?;
    validate_rendered_output_strings(path, context, "rendered_output_not_contains", &not_contains)?;

    let contains_in_order_was_authored = contains_in_order.is_some();
    let contains_in_order = contains_in_order.unwrap_or_default();
    if contains_in_order_was_authored && contains_in_order.len() < 2 {
        return Err(format!(
            "Expectation file '{}' {} requires 'rendered_output_contains_in_order' to contain at least two entries.",
            path.display(),
            context
        ));
    }
    validate_rendered_output_strings(
        path,
        context,
        "rendered_output_contains_in_order",
        &contains_in_order,
    )?;

    let contains_exactly_once_was_authored = contains_exactly_once.is_some();
    let contains_exactly_once = contains_exactly_once.unwrap_or_default();
    if contains_exactly_once_was_authored && contains_exactly_once.is_empty() {
        return Err(format!(
            "Expectation file '{}' {} requires 'rendered_output_contains_exactly_once' to contain at least one entry.",
            path.display(),
            context
        ));
    }
    validate_rendered_output_strings(
        path,
        context,
        "rendered_output_contains_exactly_once",
        &contains_exactly_once,
    )?;

    let mut authored_exactly_once = BTreeSet::new();
    for fragment in &contains_exactly_once {
        if !authored_exactly_once.insert(fragment) {
            return Err(format!(
                "Expectation file '{}' {} contains duplicate 'rendered_output_contains_exactly_once' value '{}'.",
                path.display(),
                context,
                fragment
            ));
        }
    }

    Ok(RenderedOutputExpectation {
        exact,
        contains,
        not_contains,
        contains_in_order,
        contains_exactly_once,
    })
}

fn validate_rendered_output_strings(
    path: &Path,
    context: &str,
    field_name: &str,
    values: &[String],
) -> Result<(), String> {
    for value in values {
        if value.is_empty() {
            return Err(format!(
                "Expectation file '{}' {} contains an empty '{field_name}' value.",
                path.display(),
                context
            ));
        }
    }
    Ok(())
}

fn parse_artifact_kind(
    path: &Path,
    raw_kind: &str,
    assertion_label: &str,
) -> Result<ArtifactKind, String> {
    match raw_kind {
        "html" => Ok(ArtifactKind::Html),
        "js" => Ok(ArtifactKind::Js),
        "wasm" => Ok(ArtifactKind::Wasm),
        "binary" => Ok(ArtifactKind::Binary),
        other => Err(format!(
            "Expectation file '{}' {} has unsupported artifact kind '{}'.",
            path.display(),
            assertion_label,
            other
        )),
    }
}

pub(crate) fn parse_warning_expectation(
    warnings_mode: Option<&str>,
    warning_codes: Option<Vec<String>>,
    path: &Path,
    context: &str,
) -> Result<WarningExpectation, String> {
    let context_prefix = if context.is_empty() {
        String::new()
    } else {
        format!("{context} ")
    };

    let Some(mode) = warnings_mode else {
        return Err(format!(
            "Expectation file '{}' {}is missing required key 'warnings'.",
            path.display(),
            context_prefix
        ));
    };

    match mode {
        "ignore" => {
            if warning_codes.is_some() {
                return Err(format!(
                    "Expectation file '{}' {}sets 'warning_codes' but warnings != \"exact\".",
                    path.display(),
                    context_prefix
                ));
            }
            Ok(WarningExpectation::Ignore)
        }
        "forbid" => {
            if warning_codes.is_some() {
                return Err(format!(
                    "Expectation file '{}' {}sets 'warning_codes' but warnings != \"exact\".",
                    path.display(),
                    context_prefix
                ));
            }
            Ok(WarningExpectation::Forbid)
        }
        "exact" => {
            let Some(expected_codes) = warning_codes else {
                return Err(format!(
                    "Expectation file '{}' {}uses warnings = \"exact\" but must author 'warning_codes'.",
                    path.display(),
                    context_prefix
                ));
            };

            if expected_codes.is_empty() {
                return Err(format!(
                    "Expectation file '{}' {}uses warnings = \"exact\" with an empty 'warning_codes' list; an exact warning contract must contain at least one warning identity.",
                    path.display(),
                    context_prefix
                ));
            }

            validate_code_identities(path, context, "warning_codes", &expected_codes)?;

            Ok(WarningExpectation::Exact(ExactWarningExpectation {
                expected_codes,
            }))
        }
        other => Err(format!(
            "Expectation file '{}' {}has unsupported warnings mode '{other}'.",
            path.display(),
            context_prefix
        )),
    }
}

pub(crate) fn parse_case_flags(
    flag_names: &[String],
    path: &Path,
    context: &str,
) -> Result<Vec<Flag>, String> {
    let mut flags = Vec::with_capacity(flag_names.len());
    for flag_name in flag_names {
        let parsed = match flag_name.as_str() {
            "release" => Flag::Release,
            "html_wasm" => Flag::HtmlWasm,
            other => {
                if context.is_empty() {
                    return Err(format!(
                        "Expectation file '{}' has unsupported flag '{}'.",
                        path.display(),
                        other
                    ));
                }
                return Err(format!(
                    "Expectation file '{}' {} has unsupported flag '{}'.",
                    path.display(),
                    context,
                    other
                ));
            }
        };
        flags.push(parsed);
    }
    Ok(flags)
}

/// Parses, validates and normalises `artifacts_must_not_exist` entries.
///
/// WHAT: rejects empty entries and normalises each path to forward slashes in one pass.
/// WHY: keeps absence-contract paths comparable to built-artifact paths and catches
///      degenerate expectations early at parse time.
fn parse_artifacts_must_not_exist(
    path: &Path,
    context: &str,
    raw_paths: &[String],
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::with_capacity(raw_paths.len());
    for raw_path in raw_paths {
        if raw_path.trim().is_empty() {
            return Err(format!(
                "Expectation file '{}' {} contains an empty 'artifacts_must_not_exist' entry.",
                path.display(),
                context
            ));
        }
        normalized.push(portable_path_text(raw_path));
    }

    Ok(normalized)
}

/// Validates that every diagnostic or warning code identity is non-blank.
///
/// WHAT: rejects blank entries in the selected code field while preserving exact multisets and
///       duplicates.
/// WHY: a blank identity cannot match a structured compiler diagnostic, so it cannot form a
///      truthful error or warning contract.
fn validate_code_identities(
    path: &Path,
    context: &str,
    field_name: &str,
    codes: &[String],
) -> Result<(), String> {
    let context_prefix = if context.is_empty() {
        String::new()
    } else {
        format!("{context} ")
    };

    for code in codes {
        if code.trim().is_empty() {
            return Err(format!(
                "Expectation file '{}' {}contains an empty '{field_name}' entry.",
                path.display(),
                context_prefix
            ));
        }
    }

    Ok(())
}
