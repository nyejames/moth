//! Hidden invariants for retained provider-target classification and decoding.
//!
//! WHAT: checks prefix bounds, extension lookup and extension matching.
//! WHY: malformed `DependencyTargetKind` values never appear in authored source, so these
//!      cases belong in unit tests rather than integration fixtures.

use super::{
    DependencyTargetKind, checked_provider_target, classify_dependency_target,
    decode_dependency_target,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

fn interned_components(
    string_table: &mut StringTable,
    components: &[&str],
    path_fork: &mut PathInternerFork,
) -> PathId {
    let component_ids: Vec<_> = components
        .iter()
        .map(|component| string_table.intern(component))
        .collect();
    path_fork
        .try_intern_components(&component_ids)
        .expect("test path fits")
}

#[test]
fn decode_rejects_zero_prefix_count() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(&mut string_table, &["drawing.js"], &mut path_fork);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 0,
        extension: string_table.intern("js"),
    };

    let error = decode_dependency_target(path, &target, &path_fork, &string_table)
        .expect_err("a zero prefix count is malformed retained state");
    assert!(
        error.msg.contains("zero prefix component count"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_prefix_count_beyond_the_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(&mut string_table, &["drawing.js"], &mut path_fork);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 2,
        extension: string_table.intern("js"),
    };

    let error = decode_dependency_target(path, &target, &path_fork, &string_table)
        .expect_err("a prefix count outside the path is malformed retained state");
    assert!(
        error.msg.contains("outside the path"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_retained_extension_mismatch() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(&mut string_table, &["drawing.js"], &mut path_fork);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 1,
        extension: string_table.intern("css"),
    };

    let error = decode_dependency_target(path, &target, &path_fork, &string_table)
        .expect_err("a mismatched retained extension is malformed retained state");
    assert!(
        error.msg.contains("does not match the prefix component"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_invalid_retained_extension_id() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(&mut string_table, &["drawing.js"], &mut path_fork);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 1,
        extension: StringId::from_index(string_table.len() as u32 + 16),
    };

    let error = decode_dependency_target(path, &target, &path_fork, &string_table)
        .expect_err("an invalid extension id is malformed retained state");
    assert!(
        error.msg.contains("invalid extension string id"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_keeps_remaining_provider_specific_components() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(
        &mut string_table,
        &["widgets", "draw.js", "extra"],
        &mut path_fork,
    );
    let target = classify_dependency_target(path, &path_fork, &mut string_table);

    let decoded = decode_dependency_target(path, &target, &path_fork, &string_table)
        .expect("a valid provider prefix should decode")
        .expect("an explicit-extension path should decode as a provider target");
    assert_eq!(
        path_fork.render_portable(
            decoded.prefix_path_id(),
            &string_table,
            &mut Vec::new(),
        ),
        "widgets/draw.js"
    );
    assert_eq!(decoded.remaining_components().len(), 1);
    assert_eq!(
        string_table.resolve(decoded.remaining_components()[0]),
        "extra"
    );

    assert_eq!(decoded.extension_spelling(), "js");
}

#[test]
fn checked_provider_target_retains_owned_prefix_and_suffix_facts() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(
        &mut string_table,
        &["widgets", "draw.js", "extra"],
        &mut path_fork,
    );
    let target = classify_dependency_target(path, &path_fork, &mut string_table);
    let checked = checked_provider_target(path, &target, &path_fork, &string_table)
        .expect("classified provider target should validate")
        .expect("explicit extension should produce a checked provider fact");

    checked
        .validate(path, &target, &path_fork, &string_table)
        .expect("owned provider fact should remain structurally valid");
    assert_eq!(
        path_fork.render_portable(checked.prefix_path_id(), &string_table, &mut Vec::new()),
        "widgets/draw.js"
    );
    assert_eq!(checked.raw_prefix(), "widgets/draw.js");
    assert_eq!(
        string_table.resolve(checked.remaining_components()[0]),
        "extra"
    );
    assert_eq!(string_table.resolve(checked.extension_id()), "js");
}

#[test]
fn checked_provider_target_rejects_stale_path_ids_without_panicking() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(
        &mut string_table,
        &["widgets", "draw.js", "extra"],
        &mut path_fork,
    );
    let target = classify_dependency_target(path, &path_fork, &mut string_table);
    let checked = checked_provider_target(path, &target, &path_fork, &string_table)
        .expect("classified provider target should validate")
        .expect("explicit extension should produce a checked provider fact");
    let stale_path =
        PathId::try_from_index(path_fork.len() + 8).expect("test stale path should fit");

    let error = checked
        .validate(stale_path, &target, &path_fork, &string_table)
        .expect_err("a path from another fork must fail as compiler corruption");
    assert!(
        error.msg.contains("outside its owning path fork"),
        "unexpected stale-path error: {error:?}"
    );
}

#[test]
fn checked_provider_target_rejects_stale_prefix_ids_without_resolving_them() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = interned_components(
        &mut string_table,
        &["widgets", "draw.js", "extra"],
        &mut path_fork,
    );
    let target = classify_dependency_target(path, &path_fork, &mut string_table);
    let checked = checked_provider_target(path, &target, &path_fork, &string_table)
        .expect("classified provider target should validate")
        .expect("explicit extension should produce a checked provider fact");
    let stale_prefix =
        PathId::try_from_index(path_fork.len() + 8).expect("test stale prefix should fit");
    let mut corrupted = checked.clone();
    corrupted.prefix = stale_prefix;

    let error = corrupted
        .validate(path, &target, &path_fork, &string_table)
        .expect_err("a prefix from another fork must fail as compiler corruption");
    assert!(
        error.msg.contains("prefix is outside its owning path fork"),
        "unexpected stale-prefix error: {error:?}"
    );
}

#[test]
fn checked_provider_target_rejects_invalid_path_component_string_ids() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let invalid_component = StringId::from_index(string_table.len() as u32 + 16);
    let path = path_fork
        .try_intern_components(&[invalid_component])
        .expect("test path should fit");
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 1,
        extension: string_table.intern("js"),
    };

    let error = checked_provider_target(path, &target, &path_fork, &string_table)
        .expect_err("invalid retained component identity must fail closed");
    assert!(
        error.msg.contains("invalid component string id"),
        "unexpected invalid-component error: {error:?}"
    );
}
