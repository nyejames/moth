//! Focused tests for manifest ownership parsing and recovery.

use super::super::manifest::{
    BUILD_MANIFEST_FILENAME, ManifestReadResult, ManifestRecoveryReason, read_build_manifest,
};
use super::super::policy::{BuilderIdentity, CleanupPolicy};

use std::fs;

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
