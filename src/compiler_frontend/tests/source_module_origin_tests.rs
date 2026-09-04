//! Focused hidden-invariant tests for the source-module-origin side table.
//!
//! WHAT: exercises the invariants of `SourceModuleOriginTable` that integration output
//!      cannot inspect: active and imported source files map to distinct graph stable origins,
//!      ordinary files map to their nearest owning module, source-package files outside the
//!      graph map to `None`, single-file mappings use one synthetic origin, and the table is
//!      remap-free by construction.
//! WHY: these are construction invariants owned by `compiler_frontend::source_module_origin`,
//!      so they own a focused test beside the module rather than an end-to-end case.

use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use std::fs;
use std::path::PathBuf;

/// Create a tempdir-contained file so its canonical path exists and canonicalization succeeds.
fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, "").expect("temp file should be writable");
    fs::canonicalize(&path).expect("temp file should canonicalize after creation")
}

/// Build a `SourceDatabase` from canonical paths in single-file mode (no project path resolver).
fn build_source_database(canonical_paths: &[PathBuf]) -> (SourceDatabase, StringTable) {
    let mut string_table = StringTable::new();
    let entry = &canonical_paths[0];
    let table = SourceDatabase::build(
        canonical_paths.iter().cloned(),
        entry,
        None,
        &mut string_table,
    )
    .expect("source file table should build from canonical paths");
    (table, string_table)
}

/// Build a synthetic normal-module origin for a project name.
fn synthetic_origin(project_name: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(project_name),
        std::path::Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic origin should construct")
}

/// Build a nested normal-module origin for a project name and a logical sub-module path.
fn nested_origin(project_name: &str, logical_sub_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(project_name),
        std::path::Path::new(logical_sub_path),
        ModuleRootRole::Normal,
    )
    .expect("nested origin should construct")
}

/// Build a graph-origin lookup from canonical paths to origins.
fn graph_lookup(
    entries: &[(PathBuf, StableModuleOriginIdentity)],
) -> FxHashMap<PathBuf, StableModuleOriginIdentity> {
    entries.iter().cloned().collect()
}

#[test]
fn single_file_maps_every_source_file_to_one_synthetic_origin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let entry = touch(dir.path(), "entry.moth");
    let helper = touch(dir.path(), "helper.moth");

    let (table, _string_table) = build_source_database(&[entry.clone(), helper.clone()]);
    let origin = synthetic_origin("single");

    let origins = SourceModuleOriginTable::from_synthetic_origin(&table, &origin);

    assert_eq!(
        origins.len(),
        2,
        "table should have one entry per source file"
    );
    for identity in table.iter() {
        assert_eq!(
            origins
                .origin_for(identity.id)
                .expect("in-range SourceId must not error"),
            Some(&origin),
            "every source file in single-file mode must map to the one synthetic origin"
        );
    }
}

#[test]
fn directory_active_and_imported_files_map_to_distinct_graph_origins() {
    let dir = tempfile::tempdir().expect("temp dir");
    let active_root = touch(dir.path(), "@main.moth");
    let nested_dir = dir.path().join("sub");
    fs::create_dir_all(&nested_dir).expect("nested dir should be created");
    let imported_root = touch(&nested_dir, "@other.moth");

    let (table, _string_table) =
        build_source_database(&[active_root.clone(), imported_root.clone()]);

    let active_origin = synthetic_origin("project");
    let imported_origin = nested_origin("project", "sub");

    let lookup = graph_lookup(&[
        (active_root.clone(), active_origin.clone()),
        (imported_root.clone(), imported_origin.clone()),
    ]);

    let origins = SourceModuleOriginTable::from_graph_ownership(&table, &lookup);

    let active_id = table
        .get_by_canonical_path(&active_root)
        .expect("active root should be in the source file table")
        .id;
    let imported_id = table
        .get_by_canonical_path(&imported_root)
        .expect("imported root should be in the source file table")
        .id;

    assert_eq!(
        origins
            .origin_for(active_id)
            .expect("in-range SourceId must not error"),
        Some(&active_origin),
        "the active root must map to its own graph origin"
    );
    assert_eq!(
        origins
            .origin_for(imported_id)
            .expect("in-range SourceId must not error"),
        Some(&imported_origin),
        "the imported root must map to its own distinct graph origin"
    );
    assert_ne!(
        origins
            .origin_for(active_id)
            .expect("in-range SourceId must not error"),
        origins
            .origin_for(imported_id)
            .expect("in-range SourceId must not error"),
        "active and imported origins must be distinct"
    );
}

#[test]
fn source_package_files_outside_the_graph_map_to_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    let active_root = touch(dir.path(), "@main.moth");
    // A source-package file inside the tempdir so its canonical path exists, but it is not
    // present in the graph lookup, simulating a registered source-package file outside the
    // project module graph.
    let package_dir = dir.path().join("builder");
    fs::create_dir_all(&package_dir).expect("package dir should be created");
    let source_package_root = touch(&package_dir, "@mod.moth");

    let (table, _string_table) =
        build_source_database(&[active_root.clone(), source_package_root.clone()]);

    let active_origin = synthetic_origin("project");
    let lookup = graph_lookup(&[(active_root.clone(), active_origin.clone())]);

    let origins = SourceModuleOriginTable::from_graph_ownership(&table, &lookup);

    let active_id = table
        .get_by_canonical_path(&active_root)
        .expect("active root should be in the source file table")
        .id;
    let package_id = table
        .get_by_canonical_path(&source_package_root)
        .expect("source package root should be in the source file table")
        .id;

    assert_eq!(
        origins
            .origin_for(active_id)
            .expect("in-range SourceId must not error"),
        Some(&active_origin),
        "the active root must map to its graph origin"
    );
    assert_eq!(
        origins
            .origin_for(package_id)
            .expect("in-range SourceId must not error"),
        None,
        "a source-package file outside the project module graph must map to None"
    );
}

#[test]
fn ordinary_donor_files_map_to_their_nearest_owning_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let active_root = touch(dir.path(), "@main.moth");
    // An ordinary donor file inside a nested module so it carries a distinct nested module
    // origin rather than the active root's origin.
    let nested_dir = dir.path().join("nested");
    fs::create_dir_all(&nested_dir).expect("nested dir should be created");
    let donor_file = touch(&nested_dir, "helper.moth");

    let (table, _string_table) = build_source_database(&[active_root.clone(), donor_file.clone()]);

    let active_origin = synthetic_origin("project");
    let donor_origin = nested_origin("project", "nested");

    let lookup = graph_lookup(&[
        (active_root.clone(), active_origin.clone()),
        (donor_file.clone(), donor_origin.clone()),
    ]);

    let origins = SourceModuleOriginTable::from_graph_ownership(&table, &lookup);

    let donor_id = table
        .get_by_canonical_path(&donor_file)
        .expect("donor file should be in the source file table")
        .id;

    assert_eq!(
        origins
            .origin_for(donor_id)
            .expect("in-range SourceId must not error"),
        Some(&donor_origin),
        "an ordinary donor file must map to its nearest owning module origin"
    );
}

#[test]
fn out_of_range_file_id_is_an_internal_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let entry = touch(dir.path(), "entry.moth");

    let (table, _string_table) = build_source_database(&[entry]);
    let origin = synthetic_origin("project");

    let origins = SourceModuleOriginTable::from_synthetic_origin(&table, &origin);

    let result = origins.origin_for(SourceId::from_index(999));
    assert!(
        result.is_err(),
        "an out-of-range SourceId must return an internal CompilerError, not None"
    );
    let error = result.unwrap_err();
    let message = &error.msg;
    assert!(
        message.contains("out-of-range"),
        "the out-of-range error must state the violation clearly, got: {message}"
    );
}

#[test]
fn empty_source_file_table_produces_empty_origin_table() {
    let table = SourceDatabase::empty();
    let origin = synthetic_origin("project");

    let origins = SourceModuleOriginTable::from_synthetic_origin(&table, &origin);

    assert_eq!(
        origins.len(),
        0,
        "an empty source file table must produce an empty origin table"
    );
}

#[test]
fn checkout_root_movement_does_not_alter_stable_module_origin() {
    // The stable module origin is derived from the graph's logical module path, not from the
    // canonical filesystem path. Moving the checkout root changes the canonical path but must
    // not change the owning origin value.
    let origin = synthetic_origin("project");

    let dir_a = tempfile::tempdir().expect("temp dir A");
    let dir_b = tempfile::tempdir().expect("temp dir B");

    let file_a = touch(dir_a.path(), "@main.moth");
    let file_b = touch(dir_b.path(), "@main.moth");

    let (table_a, _string_a) = build_source_database(std::slice::from_ref(&file_a));
    let (table_b, _string_b) = build_source_database(std::slice::from_ref(&file_b));

    let lookup_a = graph_lookup(&[(file_a, origin.clone())]);
    let lookup_b = graph_lookup(&[(file_b, origin.clone())]);

    let origins_a = SourceModuleOriginTable::from_graph_ownership(&table_a, &lookup_a);
    let origins_b = SourceModuleOriginTable::from_graph_ownership(&table_b, &lookup_b);

    let id_a = table_a
        .get_by_canonical_path(
            &std::fs::canonicalize(dir_a.path().join("@main.moth"))
                .expect("canonical path should exist"),
        )
        .expect("file should be in table A")
        .id;
    let id_b = table_b
        .get_by_canonical_path(
            &std::fs::canonicalize(dir_b.path().join("@main.moth"))
                .expect("canonical path should exist"),
        )
        .expect("file should be in table B")
        .id;

    assert_eq!(
        origins_a
            .origin_for(id_a)
            .expect("in-range SourceId must not error"),
        origins_b
            .origin_for(id_b)
            .expect("in-range SourceId must not error"),
        "the stable module origin must be identical after a checkout-root move"
    );
    assert_eq!(
        origins_a
            .origin_for(id_a)
            .expect("in-range SourceId must not error"),
        Some(&origin),
        "the origin must be the graph-owned value, not a path-derived fallback"
    );
}
