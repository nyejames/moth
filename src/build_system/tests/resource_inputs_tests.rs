//! Invariants for the build-owned, byte-free resource input registry.

use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::builder_surface::package_metadata::PackageSetMembership;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::fs;
use std::path::Path;

fn origin(path: &str) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("resource-input-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_portable_spelling(path.to_owned())
            .expect("test resource path should be valid"),
    )
}

fn attach_origin(
    registry: &mut ResourceInputRegistry,
    origin: StableResourceOriginId,
    source: ResourceSourceId,
) -> Result<(), InfrastructureFailure> {
    let publication = registry
        .preflight_resource_source_associations(&[ResourceSourceAssociation { origin, source }])?;
    registry.reserve_resource_source_associations(&publication);
    registry.commit_resource_source_associations(publication);
    Ok(())
}

#[test]
fn repeated_physical_sources_share_one_unhashed_record() {
    let mut registry = ResourceInputRegistry::new();
    let first = registry.register_source(Path::new("/project/assets/logo.svg").to_path_buf());
    let second = registry.register_source(Path::new("/project/assets/logo.svg").to_path_buf());

    assert_eq!(first, second);
    assert_eq!(registry.records().len(), 1);
    assert_eq!(registry.records()[0].source_id, first);
    assert_eq!(
        registry.records()[0].canonical_source_path,
        Path::new("/project/assets/logo.svg")
    );
    assert_eq!(
        registry.records()[0].content(),
        ResourceContentState::Unhashed,
        "registration must not read or hash the physical source"
    );
}

#[test]
fn live_source_hash_and_read_reuse_one_cached_filesystem_read() {
    let directory = tempfile::tempdir().expect("should create resource input directory");
    let live_path = directory.path().join("live.bin");
    let unused_path = directory.path().join("unused.bin");
    fs::write(&live_path, [7_u8, 8, 9]).expect("should write live resource");

    let mut registry = ResourceInputRegistry::new();
    let live_source = registry.register_source(live_path.clone());
    let unused_source = registry.register_source(unused_path);
    attach_origin(&mut registry, origin("live.bin"), live_source)
        .expect("live resource origin should attach");

    let content_hash = registry
        .hash_source(live_source)
        .expect("live source should hash");
    assert!(matches!(
        registry.records()[live_source.index()].content(),
        ResourceContentState::Hashed {
            content_hash: cached_hash
        } if cached_hash == content_hash
    ));

    fs::remove_file(&live_path).expect("live source should be removable after the first read");
    assert_eq!(
        registry
            .read_source(live_source)
            .expect("read should use the cached hash bytes"),
        [7_u8, 8, 9]
    );
    assert_eq!(
        registry
            .hash_source(live_source)
            .expect("repeated hashing should use the cached bytes"),
        content_hash
    );
    assert_eq!(
        registry
            .read_source(live_source)
            .expect("repeated reading should use the cached bytes"),
        [7_u8, 8, 9]
    );
    assert!(matches!(
        registry.records()[live_source.index()].content(),
        ResourceContentState::Read {
            content_hash: cached_hash
        } if cached_hash == content_hash
    ));
    assert_eq!(
        registry.records()[unused_source.index()].content(),
        ResourceContentState::Unhashed,
        "unreachable registered sources must stay unhashed and unread"
    );
}

/// A resource read failure names its physical source as portable text.
///
/// WHY: generic IO prose keeps portable display text, while the structured host path stays native
/// for IO and renderer use.
#[test]
fn resource_read_failure_reports_portable_host_path() {
    let directory = tempfile::tempdir().expect("should create resource input directory");
    let missing_path = directory.path().join("missing").join("asset.bin");

    let mut registry = ResourceInputRegistry::new();
    let source = registry.register_source(missing_path.clone());
    let failure = registry
        .hash_source(source)
        .expect_err("a missing resource source should fail its first read");

    let portable_path = portable_path_text(&missing_path);
    assert!(
        failure
            .message
            .contains(&format!("Failed to read resource source '{portable_path}'")),
        "resource read failure must name the portable host path: {}",
        failure.message
    );
    assert_eq!(
        failure.host_path.as_deref(),
        Some(missing_path.as_path()),
        "the structured host path must stay native"
    );
    assert!(
        !failure.message.contains("incomplete"),
        "an ordinary resource read failure must not claim a damaged package set: {}",
        failure.message
    );
}

#[test]
fn duplicate_registration_preserves_shipped_membership_in_either_order() {
    let directory = tempfile::tempdir().expect("should create resource input directory");
    let missing_path = directory.path().join("missing-runtime-asset.js");

    for registrations in [
        [
            PackageSetMembership::CompilerShipped,
            PackageSetMembership::Ordinary,
        ],
        [
            PackageSetMembership::Ordinary,
            PackageSetMembership::CompilerShipped,
        ],
    ] {
        let mut registry = ResourceInputRegistry::new();
        let mut source_ids = Vec::new();
        for membership in registrations {
            source_ids
                .push(registry.register_source_with_membership(missing_path.clone(), membership));
        }
        // A later ordinary registration must not erase positive shipped provenance.
        source_ids.push(registry.register_source(missing_path.clone()));

        assert!(
            source_ids.windows(2).all(|pair| pair[0] == pair[1]),
            "one physical source must keep one shared source ID"
        );
        assert_eq!(
            registry.records().len(),
            1,
            "duplicate registrations must not add a second read record"
        );

        let failure = registry
            .hash_source(source_ids[0])
            .expect_err("the shared missing source should fail its only read");
        assert!(
            failure.message.contains("incomplete"),
            "shipped membership must survive duplicate registration in either order: {}",
            failure.message
        );
    }
}

#[cfg(unix)]
#[test]
fn unresolved_pending_aliases_do_not_form_a_redirect_chain() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("should create resource input directory");
    let root = fs::canonicalize(directory.path()).expect("fixture root should canonicalize");
    let canonical_target = root.join("c.js");
    let middle = root.join("b.js");
    let alias = root.join("a.js");
    symlink(&canonical_target, &middle).expect("middle symlink should be created");
    symlink(&middle, &alias).expect("alias symlink should be created");

    let mut registry = ResourceInputRegistry::new();
    let target_source = registry.register_source(canonical_target.clone());
    let middle_source = registry.register_deferred_shipped_source(middle.clone());
    let alias_source = registry.register_deferred_shipped_source(alias.clone());
    assert_ne!(target_source, middle_source);
    assert_ne!(middle_source, alias_source);

    let failure = registry
        .hash_source(alias_source)
        .expect_err("the dangling alias should fail its first read");
    assert!(
        failure.message.contains("incomplete"),
        "the shipped dangling alias reports its own damaged set: {}",
        failure.message
    );
    assert_eq!(
        failure.host_path.as_deref(),
        Some(alias.as_path()),
        "an unresolvable provisional key must leave the alias on its own host path"
    );
    registry
        .validate()
        .expect("a provisional pending key must not become a second redirect hop");
}

#[test]
fn missing_targets_keep_only_deduplicated_watch_interests() {
    let mut registry = ResourceInputRegistry::new();
    registry.record_missing_target_watch(Path::new("/project/assets/missing.svg").to_path_buf());
    registry.record_missing_target_watch(Path::new("/project/assets/missing.svg").to_path_buf());

    assert!(registry.records().is_empty());
    assert_eq!(registry.missing_watch_interests().len(), 1);
    assert_eq!(
        registry.missing_watch_interests()[0].canonical_path(),
        Path::new("/project/assets/missing.svg")
    );
}

#[test]
fn origin_attachment_reuses_registered_source_id() {
    let mut registry = ResourceInputRegistry::new();
    let source_id = registry.register_source(Path::new("/project/assets/logo.svg").to_path_buf());
    let resource_origin = origin("assets/logo.svg");

    attach_origin(&mut registry, resource_origin.clone(), source_id)
        .expect("an existing source may receive an explicit origin attachment");
    attach_origin(&mut registry, resource_origin.clone(), source_id)
        .expect("repeating the same origin/source attachment is idempotent");
    registry
        .validate()
        .expect("the attached origin must agree with the source table");
    assert_eq!(
        registry.source_for_origin(&resource_origin),
        Some(source_id)
    );
}

#[test]
fn distinct_origins_share_one_unhashed_source_record() {
    let mut registry = ResourceInputRegistry::new();
    let source_id = registry.register_source(Path::new("/project/assets/shared.svg").to_path_buf());
    let first_origin = origin("assets/shared.svg");
    let second_origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("resource-input-tests-second"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_portable_spelling("assets/shared.svg".to_owned())
            .expect("the second stable origin path should be valid"),
    );

    attach_origin(&mut registry, first_origin.clone(), source_id)
        .expect("the first origin should attach to the source");
    attach_origin(&mut registry, second_origin.clone(), source_id)
        .expect("a distinct origin may share the physical source");
    registry
        .validate()
        .expect("multiple origin attachments must preserve registry invariants");

    assert_eq!(registry.records().len(), 1);
    assert_eq!(registry.source_for_origin(&first_origin), Some(source_id));
    assert_eq!(registry.source_for_origin(&second_origin), Some(source_id));
    assert_eq!(
        registry.records()[0].content(),
        ResourceContentState::Unhashed,
        "origin attachment must not read or hash the shared source"
    );
}

#[test]
fn association_batch_preflight_failure_leaves_registry_unchanged() {
    let mut registry = ResourceInputRegistry::new();
    let first = registry.register_source(Path::new("/project/assets/first.svg").to_path_buf());
    let second = registry.register_source(Path::new("/project/assets/second.svg").to_path_buf());
    let resource_origin = origin("assets/logo.svg");
    let associations = vec![
        ResourceSourceAssociation {
            origin: resource_origin.clone(),
            source: first,
        },
        ResourceSourceAssociation {
            origin: resource_origin,
            source: second,
        },
    ];
    let before = registry.clone();

    let error = registry
        .preflight_resource_source_associations(&associations)
        .expect_err("one origin cannot target two sources in one publication batch");

    assert!(
        error.message.contains("attached to source ID"),
        "disagreement must be diagnosed at the publication boundary: {}",
        error.message
    );
    assert_eq!(
        registry, before,
        "a failed association preflight must not mutate any registry lane"
    );
}

#[test]
fn equal_origins_cannot_attach_to_different_sources() {
    let mut registry = ResourceInputRegistry::new();
    let first = registry.register_source(Path::new("/project/assets/first.svg").to_path_buf());
    let second = registry.register_source(Path::new("/project/assets/second.svg").to_path_buf());
    let resource_origin = origin("assets/logo.svg");

    attach_origin(&mut registry, resource_origin.clone(), first)
        .expect("the first source attachment should succeed");
    let error = attach_origin(&mut registry, resource_origin, second)
        .expect_err("one stable origin must not point at two physical sources");
    assert!(
        error.message.contains("attached to source ID"),
        "disagreement must be a compiler error with source ownership context: {}",
        error.message
    );
    registry
        .validate()
        .expect("a rejected disagreement must not corrupt the registry");
}

#[test]
fn origin_attachment_rejects_unknown_source_id() {
    let mut registry = ResourceInputRegistry::new();
    let error = attach_origin(
        &mut registry,
        origin("assets/logo.svg"),
        ResourceSourceId::from_index(0),
    )
    .expect_err("an origin cannot attach to a source from another registry");

    assert!(
        error.message.contains("unknown source ID"),
        "unknown source IDs must be compiler errors: {}",
        error.message
    );
}
