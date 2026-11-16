//! Focused tests for manifest ownership parsing and recovery.

use super::super::manifest::{
    BUILD_MANIFEST_FILENAME, ManifestReadResult, ManifestRecoveryReason, prepare_output_cleanup,
    read_build_manifest, write_build_manifest,
};
use super::super::policy::{BuilderIdentity, CleanupPolicy, OutputOwner};
use crate::build_system::BuildProfile;
use crate::build_system::output::WriteMode;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidConfigView, decode_invalid_config,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticSeverity, InvalidConfigReason, PreparedFailure,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

use std::collections::HashSet;
use std::fs;
#[cfg(unix)]
use std::path::PathBuf;

#[test]
fn owner_conflict_names_the_output_root_as_portable_text() {
    // WHAT: the conflict's `output_root` fact is portable text for the plan's native root.
    // WHY: `out\put` is one literal-backslash directory name on Unix and a nested path on Windows.
    //      Equality with the formatter pins producer provenance, while the literal suffix
    //      independently pins `/` separators without deriving every byte from that formatter.
    let temp = tempfile::tempdir().expect("should create manifest directory");
    let project_dir = temp.path().join("project");
    let output_root = project_dir.join("out\\put");
    fs::create_dir_all(&output_root).expect("should create output root");
    fs::write(
        output_root.join(BUILD_MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions: .html,.js,.wasm\nindex.html\n",
    )
    .expect("should write foreign-profile manifest");

    let mut string_table = StringTable::new();
    let prepared_failure = {
        let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
        prepare_output_cleanup(
            &output_root,
            Some(&project_dir),
            false,
            &OutputOwner {
                builder: BuilderIdentity::html(),
                profile: BuildProfile::Dev,
            },
            None,
            &CleanupPolicy::html(),
            &mut diagnostic_strings,
        )
        .expect_err("a foreign profile must not claim the output root")
    };
    let PreparedFailure::Diagnosed(batch) = prepared_failure else {
        panic!("owner conflict should be diagnosed, not operational");
    };

    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Frozen)
        .expect("owner conflict diagnostics should freeze");

    let (report, view) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("owner conflict diagnostic should exist");
    let facts = view
        .family_view::<InvalidConfigView>()
        .expect("owner conflict should retain typed invalid-config facts");
    let (_, reason) = decode_invalid_config(&facts, view.string_ids())
        .expect("owner conflict should decode to its typed reason");
    let InvalidConfigReason::OutputManifestOwnerConflict {
        output_root: fact, ..
    } = reason
    else {
        panic!("expected an output manifest owner conflict reason");
    };

    let context = DiagnosticRenderContext::for_report(report);
    let fact_text = context.string_table.resolve(fact);
    assert_eq!(fact_text, portable_path_text(&output_root));
    assert!(
        fact_text.ends_with("/project/out/put"),
        "the output root fact must use portable separators, got {fact_text}"
    );
}

#[test]
fn unreadable_v4_profile_preserves_a_foreign_builder_identity() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    let mut manifest = b"# moth-manifest v4\n# builder: html\n# profile: ".to_vec();
    manifest.push(0xFF);
    manifest.extend_from_slice(b"\n# managed_extensions: .html,.js,.wasm\nindex.html\n");
    fs::write(temp.path().join(BUILD_MANIFEST_FILENAME), manifest)
        .expect("should write manifest with unreadable profile metadata");

    let policy = CleanupPolicy::html();
    let custom_builder =
        BuilderIdentity::custom("game").expect("game should be a valid custom builder identity");
    let ManifestReadResult::ForeignOwner {
        reason, builder, ..
    } = read_build_manifest(temp.path(), &custom_builder, &policy)
    else {
        panic!("a custom builder must see the declared HTML owner as foreign");
    };
    assert_eq!(reason, ManifestRecoveryReason::Unreadable);
    assert_eq!(builder, "html");
    assert_eq!(
        read_build_manifest(temp.path(), &BuilderIdentity::html(), &policy),
        ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Unreadable,
        }
    );
}

#[test]
fn custom_builder_cannot_claim_a_v3_html_manifest() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    fs::write(
        temp.path().join(BUILD_MANIFEST_FILENAME),
        "# moth-manifest v3\n# builder: html\n# managed_extensions: .html,.js,.wasm\nindex.html\n",
    )
    .expect("should write v3 manifest");

    let custom_builder =
        BuilderIdentity::custom("game").expect("game should be a valid custom builder identity");
    let ManifestReadResult::ForeignOwner {
        reason, builder, ..
    } = read_build_manifest(temp.path(), &custom_builder, &CleanupPolicy::html())
    else {
        panic!("a custom builder must see the declared v3 HTML owner as foreign");
    };
    assert_eq!(reason, ManifestRecoveryReason::UnsupportedVersion);
    assert_eq!(builder, "html");
}

#[test]
fn empty_managed_extension_policy_round_trips_its_manifest_identity() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    fs::write(
        temp.path().join(BUILD_MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: game\n# profile: dev\n# managed_extensions: (none)\ngame.wasm\n",
    )
    .expect("should write manifest for a policy with no managed extensions");

    let custom_builder =
        BuilderIdentity::custom("game").expect("game should be a valid custom builder identity");
    let ManifestReadResult::Valid(manifest) = read_build_manifest(
        temp.path(),
        &custom_builder,
        &CleanupPolicy::generic(Vec::<&str>::new()),
    ) else {
        panic!("an explicit empty managed-extension policy must remain readable");
    };

    assert!(manifest.managed_extensions.is_empty());
    assert_eq!(manifest.paths, vec![std::path::PathBuf::from("game.wasm")]);
}

#[test]
fn html_policy_never_writes_the_empty_extensions_token() {
    assert_eq!(
        CleanupPolicy::html().manifest_extensions_csv(),
        ".html,.js,.wasm"
    );
}

#[test]
fn html_policy_cannot_adopt_an_empty_extensions_manifest() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    fs::write(
        temp.path().join(BUILD_MANIFEST_FILENAME),
        "# moth-manifest v4\n# builder: html\n# profile: dev\n# managed_extensions: (none)\nindex.html\n",
    )
    .expect("should write manifest with an empty managed-extension set");

    let ManifestReadResult::RecoverableWithOwner { reason, owner } = read_build_manifest(
        temp.path(),
        &BuilderIdentity::html(),
        &CleanupPolicy::html(),
    ) else {
        panic!("HTML must not adopt a manifest that declares no managed extensions");
    };
    let ManifestRecoveryReason::ManagedExtensionsMismatch {
        manifest_extensions,
        active_extensions,
    } = reason
    else {
        panic!("the HTML policy's extension set must mismatch the empty manifest set");
    };

    assert!(manifest_extensions.is_empty());
    assert!(!active_extensions.is_empty());
    assert_eq!(owner.builder.as_str(), "html");
}

#[test]
fn manifest_write_failure_names_the_destination_in_portable_text() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    // The parent directories are never created, so the write fails with a generic IO error rather
    // than a path rejection. Generic IO prose names the destination through portable display text.
    let destination = temp
        .path()
        .join("missing")
        .join("parent")
        .join(BUILD_MANIFEST_FILENAME);

    let failure = write_build_manifest(
        &destination,
        &HashSet::new(),
        &OutputOwner {
            builder: BuilderIdentity::html(),
            profile: BuildProfile::Dev,
        },
        &CleanupPolicy::html(),
        WriteMode::AlwaysWrite,
    )
    .expect_err("a missing manifest parent must fail the write");

    assert_eq!(
        failure.host_path.as_deref(),
        Some(destination.as_path()),
        "the native manifest destination stays authoritative"
    );
    let portable_destination = portable_path_text(&destination);
    assert!(
        failure
            .message
            .contains(&format!("'{portable_destination}'")),
        "failure text should name the portable destination, got: {}",
        failure.message
    );
}

#[cfg(unix)]
#[test]
fn non_portable_managed_path_message_names_the_rejected_native_path() {
    let temp = tempfile::tempdir().expect("should create manifest directory");
    let managed_path = PathBuf::from("nested\\page.js");
    let paths: HashSet<PathBuf> = [managed_path.clone()].into_iter().collect();

    let failure = write_build_manifest(
        &temp.path().join(BUILD_MANIFEST_FILENAME),
        &paths,
        &OutputOwner {
            builder: BuilderIdentity::html(),
            profile: BuildProfile::Dev,
        },
        &CleanupPolicy::html(),
        WriteMode::AlwaysWrite,
    )
    .expect_err("a literal-backslash manifest spelling is not portable");

    assert_eq!(
        failure.host_path.as_deref(),
        Some(managed_path.as_path()),
        "the rejected native path stays authoritative"
    );
    let native_spelling = format!("{managed_path:?}");
    assert!(
        failure.message.contains(&native_spelling),
        "failure text should name the rejected native path {native_spelling}, got: {}",
        failure.message
    );
    assert!(
        !failure.message.contains("nested/page.js"),
        "failure text must not present the portable alias as the rejected path, got: {}",
        failure.message
    );
}
