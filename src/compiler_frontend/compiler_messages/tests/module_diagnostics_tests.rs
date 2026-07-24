//! Focused invariant tests for the `ModuleDiagnostics` semantic-boundary owner.
//!
//! These tests protect the one central, lossless normalization at the retained-module semantic
//! boundary: user-facing diagnostics become `Diagnosed`, an infrastructure payload round-trips
//! into a typed `CompilerError`, and a malformed or mixed sequence is a compiler invariant
//! failure. They do not duplicate end-to-end language behavior, which is owned by integration
//! cases.

use super::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, NameNamespace,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Build one user-facing rule diagnostic at a known location in the caller's string table.
fn user_rule_diagnostic(name_path: InternedPath, name: StringId) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        SourceLocation::new(
            name_path,
            CharPosition {
                line_number: 1,
                char_column: 2,
            },
            CharPosition {
                line_number: 1,
                char_column: 4,
            },
        ),
        DiagnosticPayload::UnknownName {
            name,
            namespace: NameNamespace::Value,
        },
    )
}

/// Build one infrastructure `CompilerError` with metadata, mirroring how deeper semantic stages
/// route internal failures through `CompilerMessages::from_error`.
fn infrastructure_compiler_error(string_table: &mut StringTable) -> CompilerError {
    let path = InternedPath::from_single_str("module/#page.moth", string_table);
    let location = SourceLocation::new(
        path,
        CharPosition {
            line_number: 9,
            char_column: 3,
        },
        CharPosition {
            line_number: 9,
            char_column: 6,
        },
    );
    let mut error = CompilerError::new(
        "doc fragment #2 has invalid location",
        location,
        ErrorType::Compiler,
    );
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
    let name_path = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let name_id = string_table.intern("unknown_name");
    let diagnostic = user_rule_diagnostic(name_path, name_id);

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
    let name_path = InternedPath::from_single_str("src/page.moth", &mut string_table);
    let name_id = string_table.intern("unknown_name");
    let diagnostic = user_rule_diagnostic(name_path, name_id);

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
fn single_infrastructure_payload_round_trips_into_compiler_error() {
    // WHAT: a boundary message set carrying exactly one infrastructure diagnostic recovers the
    //       originating `CompilerError` losslessly: message, location, ErrorType and metadata.
    // WHY: an infrastructure failure must become `Err(CompilerError)` at the semantic boundary,
    //      not a diagnosed result, so it cannot be mistaken for a user-facing source failure.

    let mut string_table = StringTable::new();
    let error = infrastructure_compiler_error(&mut string_table);
    let expected_msg = error.msg.clone();
    let expected_location = error.location.clone();
    let expected_error_type = error.error_type.clone();
    let expected_metadata = error.metadata.clone();

    let messages = CompilerMessages::from_error(error, string_table);

    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("an infrastructure payload should classify as Err(CompilerError)");

    assert_eq!(
        recovered_error.msg, expected_msg,
        "message should round-trip"
    );
    assert_eq!(
        recovered_error.location, expected_location,
        "location should round-trip exactly"
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
    // WHAT: a boundary message set mixing a user diagnostic with an infrastructure diagnostic is
    //       treated as a compiler invariant failure rather than a diagnosed module.
    // WHY: the current stage contracts never blend user diagnostics with infrastructure
    //      failures. The boundary must not silently drop user diagnostics or pick an arbitrary
    //      infrastructure failure.

    let mut string_table = StringTable::new();
    let name_path = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let user_diagnostic = user_rule_diagnostic(name_path, string_table.intern("unknown_name"));
    let infra_error = infrastructure_compiler_error(&mut string_table);
    let infra_diagnostic =
        crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic(&infra_error);

    let messages =
        CompilerMessages::from_diagnostics(vec![user_diagnostic, infra_diagnostic], string_table);

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
fn infrastructure_error_preserves_module_local_path_through_aggregation() {
    // WHAT: an infrastructure error whose location path was interned only in a module-local fork
    //       survives the full boundary round trip and directory aggregation, so the final
    //       aggregated table resolves the path exactly.
    // WHY: Phase 6c must not drop a module-local `StringTable` when an infrastructure diagnostic
    //      is recovered into a typed `CompilerError`. The error carries its render-identity
    //      context so `from_error` can merge and remap the location exactly once. The previous
    //      fresh-fork reconstruction left the location pointing at a table that no longer existed.

    // 1. Non-empty base build table so module-local path IDs land after the inherited prefix.
    let mut base = StringTable::new();
    base.intern("src/main.moth");
    base.intern("base string");
    let fork_source = base.fork_source();

    // 2. Module-local fork; intern the infrastructure path only in the local delta.
    let (mut local_table, base_len) = fork_source.fork_for_module().into_parts();
    let path = InternedPath::from_single_str("module/#page.moth", &mut local_table);
    let location = SourceLocation::new(
        path,
        CharPosition {
            line_number: 9,
            char_column: 3,
        },
        CharPosition {
            line_number: 9,
            char_column: 6,
        },
    );
    let error = CompilerError::new(
        "doc fragment has invalid location",
        location,
        ErrorType::Compiler,
    );

    // 3. Deeper stage routes the error through `CompilerMessages` with the module-local table.
    let messages = CompilerMessages::from_error(error, local_table);

    // 4. Semantic boundary recovers the typed `CompilerError` with its render context attached.
    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("an infrastructure payload should classify as Err(CompilerError)");

    // 5. Directory orchestration packages the typed error through `from_error` with a fresh
    //    module-local fork as the merge target, exactly as the directory compile path does.
    let (merge_target, _) = fork_source.fork_for_module().into_parts();
    let mut packaged = CompilerMessages::from_error(recovered_error, merge_target);

    // 6. Aggregation merges the packaged module table into the aggregated table and remaps.
    let (mut aggregated_table, _) = fork_source.fork_for_module().into_parts();
    let remap = aggregated_table.merge_delta_from(&packaged.string_table, base_len);
    if !remap.is_identity() {
        packaged.remap_string_ids(&remap);
    }
    packaged.string_table = aggregated_table;

    // 7. The final location path resolves exactly in the aggregated table.
    let resolved_path = packaged.diagnostics[0]
        .primary_location
        .scope
        .to_path_buf(&packaged.string_table);
    assert_eq!(
        resolved_path,
        std::path::PathBuf::from("module/#page.moth"),
        "the infrastructure path must resolve exactly in the aggregated table"
    );
}

#[test]
fn warning_plus_infrastructure_error_recovers_original_and_preserves_path() {
    // WHAT: the legitimate `from_error_with_warnings` shape — a non-error companion warning
    //       preceding exactly one infrastructure failure — recovers the originating
    //       `CompilerError` losslessly, discards the warning, and preserves the module-local
    //       path through the `from_error` rewrap and directory aggregation.
    // WHY: AST/HIR stages route internal failures through `from_error_with_warnings`, so a
    //      legitimate boundary sequence can carry a warning before one infrastructure
    //      diagnostic. The semantic boundary must not treat that as malformed; it must recover
    //      the original error exactly and drop the warning because the typed `Err` lane aborts
    //      the owning compilation and does not surface warnings.

    // 1. Non-empty base build table so module-local path IDs land after the inherited prefix.
    let mut base = StringTable::new();
    base.intern("src/main.moth");
    base.intern("base string");
    let fork_source = base.fork_source();

    // 2. Module-local fork; intern the infrastructure path only in the local delta.
    let (mut local_table, base_len) = fork_source.fork_for_module().into_parts();
    let path = InternedPath::from_single_str("module/#page.moth", &mut local_table);
    let location = SourceLocation::new(
        path,
        CharPosition {
            line_number: 9,
            char_column: 3,
        },
        CharPosition {
            line_number: 9,
            char_column: 6,
        },
    );
    let error = CompilerError::new(
        "doc fragment has invalid location",
        location,
        ErrorType::Compiler,
    );

    // 3. A non-error companion warning produced against the same module-local table, mirroring
    //    the AST/HIR `from_error_with_warnings` production shape.
    let warning_path = InternedPath::from_single_str("src/page.moth", &mut local_table);
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        SourceLocation::new(
            warning_path,
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 8,
            },
        ),
        DiagnosticPayload::UnknownName {
            name: local_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );

    // 4. Deeper stage routes the error with the warning through the production boundary helper.
    let messages = CompilerMessages::from_error_with_warnings(error, vec![warning], &local_table);
    assert_eq!(
        messages.diagnostics.len(),
        2,
        "the production shape carries the warning before the infrastructure failure"
    );
    assert_eq!(
        messages.warning_count(),
        1,
        "the production shape carries exactly one warning"
    );

    // 5. Semantic boundary recovers the typed `CompilerError` exactly, discarding the warning.
    let recovered_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("warning + one infrastructure failure should recover the CompilerError");
    assert_eq!(
        recovered_error.msg, "doc fragment has invalid location",
        "the recovered error should keep the original message"
    );
    assert_eq!(
        recovered_error.error_type,
        ErrorType::Compiler,
        "the recovered error should keep the original ErrorType"
    );

    // 6. Directory orchestration packages the typed error through `from_error` with a fresh
    //    module-local fork as the merge target, exactly as the directory compile path does.
    let (merge_target, _) = fork_source.fork_for_module().into_parts();
    let mut packaged = CompilerMessages::from_error(recovered_error, merge_target);

    // 7. Aggregation merges the packaged module table into the aggregated table and remaps.
    let (mut aggregated_table, _) = fork_source.fork_for_module().into_parts();
    let remap = aggregated_table.merge_delta_from(&packaged.string_table, base_len);
    if !remap.is_identity() {
        packaged.remap_string_ids(&remap);
    }
    packaged.string_table = aggregated_table;

    // 8. The final location path resolves exactly in the aggregated table.
    let resolved_path = packaged.diagnostics[0]
        .primary_location
        .scope
        .to_path_buf(&packaged.string_table);
    assert_eq!(
        resolved_path,
        std::path::PathBuf::from("module/#page.moth"),
        "the infrastructure path must resolve exactly after rewrapping the recovered error"
    );
}

#[test]
fn user_error_plus_warning_becomes_diagnosed_module_preserving_order() {
    // WHAT: a boundary message set carrying a non-error warning and a user-facing error (no
    //       infrastructure payload) becomes `Diagnosed` with production order preserved.
    // WHY: warnings and notes may legitimately accompany a user-facing error and must stay in
    //      order for rendering. The boundary must not downgrade such a sequence into an
    //      invariant failure or reorder the diagnostics.

    let mut string_table = StringTable::new();
    let warning_path = InternedPath::from_single_str("src/page.moth", &mut string_table);
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        SourceLocation::new(
            warning_path,
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 8,
            },
        ),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );
    let name_path = InternedPath::from_single_str("src/main.moth", &mut string_table);
    let error = user_rule_diagnostic(name_path, string_table.intern("unknown_name"));

    // Production order: warning first, then the user-facing error.
    let messages =
        CompilerMessages::from_diagnostics(vec![warning.clone(), error.clone()], string_table);

    let diagnosed = ModuleDiagnostics::from_messages(messages)
        .expect("warning + user error should classify as Diagnosed");
    assert_eq!(
        diagnosed.diagnostics().len(),
        2,
        "Diagnosed payload should carry both the warning and the user error"
    );
    assert_eq!(
        diagnosed.diagnostics()[0].severity,
        DiagnosticSeverity::Warning,
        "production order should keep the warning first"
    );
    assert_eq!(
        diagnosed.diagnostics()[0].payload,
        warning.payload,
        "the leading warning should be preserved"
    );
    assert_eq!(
        diagnosed.diagnostics()[1].severity,
        DiagnosticSeverity::Error,
        "the user-facing error should follow the warning"
    );
    assert_eq!(
        diagnosed.diagnostics()[1].payload,
        error.payload,
        "the user-facing error payload should be preserved"
    );
}

#[test]
fn warning_only_failure_is_invariant_failure() {
    // WHAT: a boundary message set carrying only a non-error warning (no user error and no
    //       infrastructure payload) is a compiler invariant failure, not a diagnosed module.
    // WHY: a failing stage must surface at least one user-facing `Error` diagnostic. A
    //      warning-only failure carries nothing the renderer is allowed to treat as a module
    //      failure, so the boundary reports it as a malformed sequence.

    let mut string_table = StringTable::new();
    let warning_path = InternedPath::from_single_str("src/page.moth", &mut string_table);
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        SourceLocation::new(
            warning_path,
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 8,
            },
        ),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("unused"),
            namespace: NameNamespace::Value,
        },
    );

    let messages = CompilerMessages::from_diagnostics(vec![warning], string_table);

    let invariant_error = ModuleDiagnostics::from_messages(messages)
        .expect_err("a warning-only failure should be a compiler invariant failure");
    assert_eq!(
        invariant_error.error_type,
        ErrorType::Compiler,
        "the warning-only invariant should be an internal CompilerError"
    );
    assert!(
        invariant_error
            .msg
            .contains("no user-facing error diagnostic"),
        "the invariant message should describe the missing user error, got: {}",
        invariant_error.msg
    );
}
