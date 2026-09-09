//! Focused invariant tests for the `ModuleDiagnostics` semantic-boundary owner.
//!
//! These tests protect the one central, lossless normalization at the retained-module semantic
//! boundary: user-facing diagnostics become `Diagnosed`, an outer-lane infrastructure failure
//! round-trips into a typed `CompilerError`, and a malformed or mixed sequence is a compiler
//! invariant failure. They do not duplicate end-to-end language behavior, which is owned by
//! integration cases.

use super::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, NameNamespace,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::path::PathBuf;

/// Build one user-facing rule diagnostic without source provenance.
fn user_rule_diagnostic(name: StringId) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        None,
        DiagnosticPayload::UnknownName {
            name,
            namespace: NameNamespace::Value,
        },
    )
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
fn user_diagnostics_become_diagnosed_and_round_trip() {
    // WHAT: a boundary message set carrying only user-facing diagnostics becomes `Diagnosed`, and
    //       `into_messages` reconstructs the exact diagnostics, string table and render contexts.
    // WHY: the semantic boundary must surface user diagnostics through the renderer and must not
    //      downgrade them into an infrastructure `CompilerError`.

    let mut string_table = StringTable::new();
    let name_id = string_table.intern("unknown_name");
    let diagnostic = user_rule_diagnostic(name_id);

    let messages = CompilerMessages::from_diagnostics(vec![diagnostic.clone()], string_table)
        .with_type_context_for_all_diagnostics(TypeEnvironment::new());

    let diagnosed = ModuleDiagnostics::from_messages(messages)
        .expect("user-only diagnostics should classify as Diagnosed");

    // The diagnosed payload retains the module-local string table so the renderer can resolve the
    // diagnostic's interned name and path after the local compilation call has finished.
    assert_eq!(
        diagnosed.string_table().resolve(name_id),
        "unknown_name",
        "Diagnosed payload should retain the module-local string table"
    );

    assert_eq!(
        diagnosed.diagnostics().len(),
        1,
        "Diagnosed payload should carry the one user diagnostic"
    );
    assert_eq!(
        diagnosed.diagnostics()[0].kind,
        diagnostic.kind,
        "Diagnosed payload should preserve the diagnostic kind"
    );
    assert_eq!(
        diagnosed.render_type_contexts().len(),
        1,
        "Diagnosed payload should preserve the render type context"
    );

    let recovered = diagnosed.into_messages();
    assert_eq!(
        recovered.diagnostics.len(),
        1,
        "into_messages should reconstruct the diagnostic stream losslessly"
    );
    assert!(
        recovered.diagnostics[0].payload == diagnostic.payload,
        "into_messages should preserve the user payload"
    );
    assert_eq!(
        recovered.render_type_contexts.len(),
        1,
        "into_messages should carry the render type context"
    );
}

#[test]
fn diagnosed_render_context_survives_round_trip_and_remap() {
    // WHAT: a `ModuleDiagnostics` built from a message set with a render type context keeps that
    //       context through `into_messages`, and an identity remap keeps the context range
    //       aligned with the diagnostic stream.
    // WHY: type diagnostics store `TypeId`s whose render table must stay bound to the diagnostic
    //      range after the semantic boundary transfers ownership and after string-table merging.

    let mut string_table = StringTable::new();
    let name_id = string_table.intern("unknown_name");
    let diagnostic = user_rule_diagnostic(name_id);

    let messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table)
        .with_type_context_for_all_diagnostics(TypeEnvironment::new());

    let diagnosed = ModuleDiagnostics::from_messages(messages)
        .expect("user-only diagnostics should classify as Diagnosed");
    let contexts_before = diagnosed.render_type_contexts();
    assert_eq!(contexts_before.len(), 1);
    assert_eq!(contexts_before[0].diagnostic_range, 0..1);

    let mut recovered = diagnosed.into_messages();
    assert_eq!(
        recovered.render_type_contexts[0].diagnostic_range,
        0..1,
        "into_messages should keep the render context range aligned"
    );

    // Build an identity remap covering every ID in the recovered table by merging that table into
    // a fresh target. The directory aggregation does the same shape of merge before remapping a
    // diagnosed module's diagnostics.
    let mut remap_target = StringTable::new();
    let remap = remap_target.merge_from(&recovered.string_table);
    assert!(remap.is_identity(), "fixture remap must be identity");
    recovered.remap_string_ids(&remap);
    assert_eq!(
        recovered.render_type_contexts[0].diagnostic_range,
        0..1,
        "identity remap should keep the render context aligned"
    );
}

#[test]
fn single_infrastructure_error_round_trips_into_compiler_error() {
    // WHAT: a boundary message set carrying exactly one outer-lane infrastructure failure
    //       recovers the originating `CompilerError` losslessly: message, span, ErrorType and
    //       metadata.
    // WHY: an infrastructure failure must become `Err(CompilerError)` at the semantic boundary,
    //      not a diagnosed result, so it cannot be mistaken for a user-facing source failure.

    let string_table = StringTable::new();
    let error = infrastructure_compiler_error();
    let expected_msg = error.msg.clone();
    let expected_span = error.source_span;
    let expected_host_path = error.host_path.clone();
    let expected_error_type = error.error_type.clone();
    let expected_metadata = error.metadata.clone();

    let messages = CompilerMessages::from_error(error, string_table);

    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("an outer infrastructure failure should classify as Err(CompilerError)");

    assert_eq!(
        recovered_error.msg, expected_msg,
        "message should round-trip"
    );
    assert_eq!(
        recovered_error.source_span, expected_span,
        "source span should round-trip exactly"
    );
    assert_eq!(
        recovered_error.host_path, expected_host_path,
        "host path should round-trip exactly"
    );
    assert_eq!(
        recovered_error.error_type, expected_error_type,
        "ErrorType should round-trip"
    );
    assert_eq!(
        recovered_error.metadata, expected_metadata,
        "metadata should round-trip"
    );
}

#[test]
fn mixed_user_and_infrastructure_sequence_is_invariant_failure() {
    // WHAT: a boundary message set mixing a user diagnostic with an outer infrastructure failure
    //       is treated as a compiler invariant failure rather than a diagnosed module.
    // WHY: the current stage contracts never blend user diagnostics with infrastructure
    //      failures. The boundary must not silently drop user diagnostics or pick an arbitrary
    //      infrastructure failure.

    let mut string_table = StringTable::new();
    let user_diagnostic = user_rule_diagnostic(string_table.intern("unknown_name"));
    let infra_error = infrastructure_compiler_error();

    let mut messages = CompilerMessages::from_error(infra_error, string_table);
    messages.extend_diagnostics(vec![user_diagnostic]);

    let invariant_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("a mixed sequence should be a compiler invariant failure");

    assert_eq!(
        invariant_error.error_type,
        ErrorType::Compiler,
        "the malformed-sequence invariant should be an internal CompilerError"
    );
    assert!(
        invariant_error
            .msg
            .contains("malformed diagnostic sequence"),
        "the invariant message should describe the malformed sequence, got: {}",
        invariant_error.msg
    );
}

#[test]
fn empty_failure_is_invariant_failure() {
    // WHAT: a boundary message set returned as a failure with no diagnostics is a compiler
    //       invariant failure, not an empty diagnosed module.
    // WHY: a failing stage must carry at least one diagnostic; an empty failure is malformed.

    let string_table = StringTable::new();
    let messages = CompilerMessages::empty(string_table);

    let invariant_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("an empty failure should be a compiler invariant failure");

    assert_eq!(
        invariant_error.error_type,
        ErrorType::Compiler,
        "the empty-failure invariant should be an internal CompilerError"
    );
    assert!(
        invariant_error
            .msg
            .contains("module semantic stage returned a failure with no diagnostics"),
        "the invariant message should describe the empty failure, got: {}",
        invariant_error.msg
    );
}

#[test]
fn infrastructure_error_preserves_host_path_through_reaggregation() {
    // Infrastructure paths are filesystem-owned rather than StringTable-backed. Rewrapping the
    // typed error through the module boundary and a second message boundary must preserve that
    // host path verbatim.
    let error = infrastructure_compiler_error();
    let expected_path = error.host_path.clone();

    let messages = CompilerMessages::from_error(error, StringTable::new());
    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("an outer infrastructure failure should classify as Err(CompilerError)");
    assert_eq!(recovered_error.host_path, expected_path);

    let packaged = CompilerMessages::from_error(recovered_error, StringTable::new());
    assert_eq!(
        packaged
            .infrastructure_error()
            .expect("the packaged outer failure must be preserved")
            .host_path,
        expected_path
    );
}

#[test]
fn warning_plus_infrastructure_error_recovers_original_and_preserves_host_path() {
    // A non-error companion warning may precede exactly one infrastructure failure. The typed
    // error survives the semantic boundary while the warning is discarded from the Err lane.
    let error = infrastructure_compiler_error();
    let expected_path = error.host_path.clone();
    let mut warning_table = StringTable::new();
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        None,
        DiagnosticPayload::UnknownName {
            name: warning_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );
    let messages = CompilerMessages::from_error_with_warnings(error, vec![warning], &warning_table);

    assert_eq!(messages.diagnostics.len(), 1);
    assert!(messages.infrastructure_error().is_some());
    assert_eq!(messages.warning_count(), 1);

    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("warning + one infrastructure failure should recover the CompilerError");
    assert_eq!(
        recovered_error.msg,
        "doc fragment #2 has invalid source span"
    );
    assert_eq!(recovered_error.host_path, expected_path);

    let packaged = CompilerMessages::from_error(recovered_error, StringTable::new());
    assert_eq!(
        packaged
            .infrastructure_error()
            .expect("the packaged outer failure must be preserved")
            .host_path,
        expected_path
    );
}

#[test]
fn user_error_plus_warning_becomes_diagnosed_module_preserving_order() {
    // Warnings may accompany a user-facing error and must remain in production order.
    let mut string_table = StringTable::new();
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        None,
        DiagnosticPayload::UnknownName {
            name: string_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );
    let error = user_rule_diagnostic(string_table.intern("unknown_name"));

    let messages =
        CompilerMessages::from_diagnostics(vec![warning.clone(), error.clone()], string_table);
    let diagnosed = ModuleDiagnostics::from_messages(messages)
        .expect("warning + user error should classify as Diagnosed");
    assert_eq!(diagnosed.diagnostics().len(), 2);
    assert_eq!(
        diagnosed.diagnostics()[0].severity,
        DiagnosticSeverity::Warning
    );
    assert_eq!(diagnosed.diagnostics()[0].payload, warning.payload);
    assert_eq!(
        diagnosed.diagnostics()[1].severity,
        DiagnosticSeverity::Error
    );
    assert_eq!(diagnosed.diagnostics()[1].payload, error.payload);
}

#[test]
fn warning_only_failure_is_invariant_failure() {
    // A failure carrying only a warning is malformed: no user-facing error can be surfaced.
    let mut string_table = StringTable::new();
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        None,
        DiagnosticPayload::UnknownName {
            name: string_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );

    let messages = CompilerMessages::from_diagnostics(vec![warning], string_table);
    let invariant_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("a warning-only failure should be a compiler invariant failure");
    assert_eq!(invariant_error.error_type, ErrorType::Compiler);
    assert!(
        invariant_error
            .msg
            .contains("no user-facing error diagnostic"),
        "the invariant message should describe the missing user error, got: {}",
        invariant_error.msg
    );
}
