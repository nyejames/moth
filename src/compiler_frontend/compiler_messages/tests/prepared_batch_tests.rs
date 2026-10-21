//! Focused behavioural tests for premerge and prepared-batch classification.

use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticSeverity, PremergeDiagnosticBatch, PremergeFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::path::PathBuf;
use std::sync::Arc;

/// Build one production unknown-value diagnostic without source provenance.
fn user_value_name_diagnostic(name: StringId) -> CompilerDiagnostic {
    CompilerDiagnostic::unknown_value_name(name, None)
}

/// Build a production unknown-value warning without source provenance.
fn warning_value_name_diagnostic(name: StringId) -> CompilerDiagnostic {
    let mut diagnostic = CompilerDiagnostic::unknown_value_name(name, None);
    diagnostic.severity = DiagnosticSeverity::Warning;
    diagnostic
}

/// Build one infrastructure `CompilerError` with metadata, mirroring how deeper semantic stages
/// route internal failures through `CompilerMessages::from_error`.
fn infrastructure_compiler_error() -> CompilerError {
    let mut error = CompilerError::new(
        "doc fragment #2 has invalid source span",
        None,
        ErrorType::Compiler,
    );
    error.host_path = Some(PathBuf::from("module/@page.moth"));
    error.new_metadata_entry(
        CompilerErrorMetadataKey::CompilationStage,
        String::from("Module Metadata Validation"),
    );
    error
}

#[test]
fn diagnosed_premerge_batch_retains_local_table_and_type_context() {
    let mut string_table = StringTable::new();
    let name_id = string_table.intern("unknown_name");
    let diagnostic = user_value_name_diagnostic(name_id);
    let messages = CompilerMessages::from_diagnostics(vec![diagnostic.clone()], string_table)
        .with_type_context_for_all_diagnostics(TypeEnvironment::new());

    let PremergeFailure::Diagnosed(batch) = PremergeFailure::from(messages) else {
        panic!("user-only diagnostics should remain in the diagnosed premerge lane");
    };
    let (diagnostics, strings, type_contexts, path_contexts) = batch.into_parts();

    assert_eq!(strings.resolve(name_id), "unknown_name");
    assert_eq!(diagnostics.diagnostics().len(), 1);
    assert_eq!(diagnostics.diagnostics()[0].kind, diagnostic.kind);
    assert_eq!(type_contexts.len(), 1);
    assert_eq!(type_contexts[0].diagnostic_range, 0..1);
    assert!(path_contexts.is_empty());
}

#[test]
fn infrastructure_aggregate_preserves_every_typed_cause() {
    let string_table = StringTable::new();
    let error = infrastructure_compiler_error();
    let expected_msg = error.msg.clone();
    let expected_span = error.source_span;
    let expected_host_path = error.host_path.clone();
    let expected_error_type = error.error_type.clone();
    let expected_metadata = error.metadata.clone();

    let mut messages = CompilerMessages::from_error(error, string_table);
    let mut secondary = CompilerError::file_error(
        &PathBuf::from("module/secondary.moth"),
        "secondary infrastructure cause",
    );
    secondary.new_metadata_entry(
        CompilerErrorMetadataKey::CompilationStage,
        "read".to_owned(),
    );
    messages.set_infrastructure_error(secondary.clone());

    let PremergeFailure::Infrastructure(recovered_errors) = PremergeFailure::from(messages) else {
        panic!("infrastructure failures should remain on the aggregate lane");
    };
    assert_eq!(
        recovered_errors
            .iter()
            .map(|error| error.msg.as_str())
            .collect::<Vec<_>>(),
        [expected_msg.as_str(), "secondary infrastructure cause"]
    );
    let recovered_secondary = &recovered_errors.additional[0];
    assert_eq!(recovered_secondary.error_type, secondary.error_type);
    assert_eq!(recovered_secondary.host_path, secondary.host_path);
    assert_eq!(recovered_secondary.metadata, secondary.metadata);
    let recovered_error = &recovered_errors.primary;
    assert_eq!(recovered_error.msg, expected_msg);
    assert_eq!(recovered_error.source_span, expected_span);
    assert_eq!(recovered_error.host_path, expected_host_path);
    assert_eq!(recovered_error.error_type, expected_error_type);
    assert_eq!(recovered_error.metadata, expected_metadata);
}

#[test]
fn mixed_user_and_infrastructure_sequence_preserves_causes_and_d12_message() {
    let mut string_table = StringTable::new();
    let user_diagnostic = user_value_name_diagnostic(string_table.intern("unknown_name"));
    let infra_error = infrastructure_compiler_error();

    let mut messages = CompilerMessages::from_error(infra_error, string_table);
    messages.set_infrastructure_error(CompilerError::compiler_error(
        "secondary infrastructure cause",
    ));
    messages.extend_diagnostics(vec![user_diagnostic]);

    let PremergeFailure::Infrastructure(errors) = PremergeFailure::from(messages) else {
        panic!("a mixed sequence must stay on the infrastructure lane");
    };
    assert_eq!(errors.iter().count(), 3);
    assert_eq!(
        errors.primary.msg,
        "doc fragment #2 has invalid source span"
    );
    assert_eq!(
        errors.iter().nth(1).expect("secondary cause").msg,
        "secondary infrastructure cause"
    );
    let invariant_error = errors.iter().last().expect("malformed-sequence invariant");
    assert_eq!(invariant_error.error_type, ErrorType::Compiler);
    assert_eq!(
        invariant_error.msg,
        "module semantic stage returned a malformed diagnostic sequence mixing a user-facing error with an infrastructure failure among 1 total"
    );
}

#[test]
fn empty_semantic_failure_keeps_its_d12_message() {
    let messages = CompilerMessages::empty(StringTable::new());
    let PremergeFailure::Infrastructure(errors) = PremergeFailure::from(messages) else {
        panic!("an empty semantic failure must be an invariant error");
    };
    assert_eq!(
        errors.primary.msg,
        "module semantic stage returned a failure with no diagnostics"
    );
    assert_eq!(errors.primary.error_type, ErrorType::Compiler);
}

#[test]
fn source_context_before_canonical_merge_keeps_its_d12_message() {
    let mut string_table = StringTable::new();
    let diagnostic = user_value_name_diagnostic(string_table.intern("unknown_name"));
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    messages.set_source_database(Arc::new(SourceDatabase::empty()));

    let PremergeFailure::Infrastructure(errors) = PremergeFailure::from(messages) else {
        panic!("diagnosed source rows must not enter the premerge batch");
    };
    assert_eq!(
        errors.primary.msg,
        "diagnosed module reached canonical merging with 1 attached source contexts beside 1 diagnostics; source attachment must happen after merging"
    );
}

#[test]
fn infrastructure_error_host_path_survives_premerge_reaggregation() {
    let error = infrastructure_compiler_error();
    let expected_path = error.host_path.clone();

    let mut messages = CompilerMessages::from_error(error, StringTable::new());
    messages.set_infrastructure_error(CompilerError::compiler_error(
        "secondary infrastructure cause",
    ));
    let PremergeFailure::Infrastructure(recovered_errors) = PremergeFailure::from(messages) else {
        panic!("infrastructure causes must survive premerge classification");
    };
    assert_eq!(recovered_errors.primary.host_path, expected_path);

    let packaged =
        PremergeFailure::Infrastructure(recovered_errors).into_messages(&StringTable::new());
    assert_eq!(
        packaged
            .infrastructure_errors()
            .expect("aggregate retained")
            .iter()
            .count(),
        2
    );
    assert_eq!(
        packaged
            .primary_infrastructure_error()
            .expect("the packaged outer failure must be preserved")
            .host_path,
        expected_path
    );
}

#[test]
fn warning_plus_infrastructure_error_discards_only_the_warning() {
    let error = infrastructure_compiler_error();
    let expected_path = error.host_path.clone();
    let mut warning_table = StringTable::new();
    let warning = warning_value_name_diagnostic(warning_table.intern("unused"));
    let mut messages =
        CompilerMessages::from_error_with_warnings(error, vec![warning], &warning_table);
    messages.set_infrastructure_error(CompilerError::compiler_error(
        "secondary infrastructure cause",
    ));

    assert_eq!(messages.diagnostics.len(), 1);
    assert!(messages.infrastructure_errors().is_some());
    assert_eq!(messages.warning_count(), 1);

    let PremergeFailure::Infrastructure(recovered_error) = PremergeFailure::from(messages) else {
        panic!("warning plus infrastructure failure should recover the typed causes");
    };
    assert_eq!(
        recovered_error.primary.msg,
        "doc fragment #2 has invalid source span"
    );
    assert_eq!(recovered_error.primary.host_path, expected_path);
    assert_eq!(recovered_error.iter().count(), 2);
}

#[test]
fn user_error_and_warning_keep_diagnostic_order() {
    let mut string_table = StringTable::new();
    let warning = warning_value_name_diagnostic(string_table.intern("unused"));
    let error = user_value_name_diagnostic(string_table.intern("unknown_name"));
    let messages =
        CompilerMessages::from_diagnostics(vec![warning.clone(), error.clone()], string_table);

    let PremergeFailure::Diagnosed(batch) = PremergeFailure::from(messages) else {
        panic!("warning plus user error should classify as diagnosed");
    };
    let (diagnostics, ..) = batch.into_parts();
    assert_eq!(diagnostics.diagnostics().len(), 2);
    assert_eq!(
        diagnostics.diagnostics()[0].severity,
        DiagnosticSeverity::Warning
    );
    assert_eq!(diagnostics.diagnostics()[0].payload, warning.payload);
    assert_eq!(
        diagnostics.diagnostics()[1].severity,
        DiagnosticSeverity::Error
    );
    assert_eq!(diagnostics.diagnostics()[1].payload, error.payload);
}

#[test]
fn warning_only_failure_keeps_its_d12_message() {
    let mut string_table = StringTable::new();
    let warning = warning_value_name_diagnostic(string_table.intern("unused"));
    let messages = CompilerMessages::from_diagnostics(vec![warning], string_table);

    let PremergeFailure::Infrastructure(errors) = PremergeFailure::from(messages) else {
        panic!("a warning-only failure must be an invariant error");
    };
    assert_eq!(errors.primary.error_type, ErrorType::Compiler);
    assert_eq!(
        errors.primary.msg,
        "module semantic stage returned a failure with no user-facing error diagnostic"
    );
}

#[test]
fn prepared_premerge_classifier_keeps_empty_and_warning_only_d12_messages() {
    let empty = PreparedDiagnosticBatch::from_premerge(PremergeDiagnosticBatch::from_diagnostics(
        Vec::new(),
        StringTable::new(),
    ));
    let empty_error = match empty.classify_premerge() {
        Ok(_) => panic!("an empty premerge batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        empty_error.msg,
        "module semantic stage returned a premerge batch with no diagnostics"
    );

    let mut strings = StringTable::new();
    let warning = warning_value_name_diagnostic(strings.intern("unused"));
    let warning_only = PreparedDiagnosticBatch::from_premerge(
        PremergeDiagnosticBatch::from_diagnostics(vec![warning], strings),
    );
    let warning_error = match warning_only.classify_premerge() {
        Ok(_) => panic!("a warning-only premerge batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        warning_error.msg,
        "module semantic stage returned a premerge batch with no user-facing error diagnostic"
    );
}

#[test]
fn mixed_premerge_failure_preserves_diagnostics_and_all_infrastructure_causes() {
    let mut string_table = StringTable::new();
    let diagnostic = user_value_name_diagnostic(string_table.intern("unknown_name"));
    let batch = PremergeDiagnosticBatch::from_parts(
        vec![diagnostic.clone()],
        string_table,
        Vec::new(),
        Vec::new(),
    );
    let mut errors: Box<InfrastructureErrors> = infrastructure_compiler_error().into();
    errors.push(CompilerError::compiler_error(
        "finalization secondary cause",
    ));

    let messages = PremergeFailure::Mixed {
        batch,
        error: errors,
    }
    .into_messages(&StringTable::new());
    assert_eq!(messages.diagnostics.len(), 1);
    assert_eq!(messages.diagnostics[0].payload, diagnostic.payload);
    assert_eq!(
        messages
            .infrastructure_errors()
            .expect("mixed causes retained")
            .iter()
            .map(|error| error.msg.as_str())
            .collect::<Vec<_>>(),
        [
            "doc fragment #2 has invalid source span",
            "finalization secondary cause"
        ]
    );
}
