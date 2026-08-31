//! Invariants for the build-owned, byte-free resource input registry.

use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
) -> Result<(), CompilerError> {
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

    let mut string_table = StringTable::new();
    let content_hash = registry
        .hash_source(live_source, &mut string_table)
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
            .read_source(live_source, &mut string_table)
            .expect("read should use the cached hash bytes"),
        [7_u8, 8, 9]
    );
    assert_eq!(
        registry
            .hash_source(live_source, &mut string_table)
            .expect("repeated hashing should use the cached bytes"),
        content_hash
    );
    assert_eq!(
        registry
            .read_source(live_source, &mut string_table)
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
        error.msg.contains("attached to source ID"),
        "disagreement must be diagnosed at the publication boundary: {}",
        error.msg
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
        error.msg.contains("attached to source ID"),
        "disagreement must be a compiler error with source ownership context: {}",
        error.msg
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
        error.msg.contains("unknown source ID"),
        "unknown source IDs must be compiler errors: {}",
        error.msg
    );
}
