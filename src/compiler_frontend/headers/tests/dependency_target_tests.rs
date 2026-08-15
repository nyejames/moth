//! Hidden invariants for retained provider-target classification and decoding.
//!
//! WHAT: checks prefix bounds, extension lookup, extension matching and remapped targets.
//! WHY: malformed `DependencyTargetKind` values never appear in authored source, so these
//!      cases belong in unit tests rather than integration fixtures.

use super::{DependencyTargetKind, classify_dependency_target, decode_dependency_target};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

fn interned_components(string_table: &mut StringTable, components: &[&str]) -> InternedPath {
    InternedPath::from_components(
        components
            .iter()
            .map(|component| string_table.intern(component))
            .collect(),
    )
}

#[test]
fn decode_rejects_zero_prefix_count() {
    let mut string_table = StringTable::new();
    let path = interned_components(&mut string_table, &["drawing.js"]);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 0,
        extension: string_table.intern("js"),
    };

    let error = decode_dependency_target(&path, &target, &string_table)
        .expect_err("a zero prefix count is malformed retained state");
    assert!(
        error.msg.contains("zero prefix component count"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_prefix_count_beyond_the_path() {
    let mut string_table = StringTable::new();
    let path = interned_components(&mut string_table, &["drawing.js"]);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 2,
        extension: string_table.intern("js"),
    };

    let error = decode_dependency_target(&path, &target, &string_table)
        .expect_err("a prefix count outside the path is malformed retained state");
    assert!(
        error.msg.contains("outside the path"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_retained_extension_mismatch() {
    let mut string_table = StringTable::new();
    let path = interned_components(&mut string_table, &["drawing.js"]);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 1,
        extension: string_table.intern("css"),
    };

    let error = decode_dependency_target(&path, &target, &string_table)
        .expect_err("a mismatched retained extension is malformed retained state");
    assert!(
        error.msg.contains("does not match the prefix component"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_rejects_invalid_retained_extension_id() {
    let mut string_table = StringTable::new();
    let path = interned_components(&mut string_table, &["drawing.js"]);
    let target = DependencyTargetKind::ExternalProvider {
        prefix_component_count: 1,
        extension: StringId::from_index(string_table.len() as u32 + 16),
    };

    let error = decode_dependency_target(&path, &target, &string_table)
        .expect_err("an invalid extension id is malformed retained state");
    assert!(
        error.msg.contains("invalid extension string id"),
        "unexpected invariant error: {error:?}"
    );
}

#[test]
fn decode_keeps_remaining_provider_specific_components() {
    let mut string_table = StringTable::new();
    let path = interned_components(&mut string_table, &["widgets", "draw.js", "extra"]);
    let target = classify_dependency_target(&path, &mut string_table);

    let decoded = decode_dependency_target(&path, &target, &string_table)
        .expect("a valid provider prefix should decode")
        .expect("an explicit-extension path should decode as a provider target");
    assert_eq!(
        decoded.prefix_path().to_portable_string(&string_table),
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
fn remapped_target_preserves_the_decoded_extension() {
    let mut local = StringTable::new();
    let path = interned_components(&mut local, &["drawing.js"]);
    let mut target = classify_dependency_target(&path, &mut local);

    let mut global = StringTable::new();
    global.intern("unrelated-prefix");
    let remap = global.merge_from(&local);
    let mut remapped_path = path.clone();
    remapped_path.remap_string_ids(&remap);
    target.remap_string_ids(&remap);

    let decoded = decode_dependency_target(&remapped_path, &target, &global)
        .expect("a remapped valid target should decode")
        .expect("the remapped target should remain a provider");
    assert_eq!(decoded.extension_spelling(), "js");
    assert_eq!(
        decoded.prefix_path().to_portable_string(&global),
        "drawing.js"
    );
}
