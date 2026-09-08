//! Tests for build-table forks and delta remapping.
//!
//! WHAT: exercises the compact identity-prefix remap used when module-local string tables are
//! merged back into the build table.
//! WHY: parallel module compilation can merge overlapping local suffixes, so remapping must stay
//! correct even when inherited IDs remain identity.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

#[test]
fn shared_fork_resolves_base_strings_without_local_entries() {
    let mut build_table = StringTable::new();
    let inherited_id = build_table.intern("base");

    let fork_source = build_table.fork_source();
    let fork = fork_source.fork_for_module();

    assert_eq!(fork.base_len(), build_table.len());

    let (module_table, _) = fork.into_parts();

    assert_eq!(module_table.len(), build_table.len());
    assert_eq!(module_table.resolve(inherited_id), "base");
}

#[test]
fn fork_interning_existing_base_string_returns_inherited_id() {
    let mut build_table = StringTable::new();
    let inherited_id = build_table.intern("shared-base");

    let fork_source = build_table.fork_source();
    let fork = fork_source.fork_for_module();
    let (mut module_table, base_len) = fork.into_parts();

    let fork_id = module_table.intern("shared-base");

    assert_eq!(fork_id, inherited_id);
    assert_eq!(module_table.len(), base_len);
}

#[test]
fn fork_without_new_strings_produces_identity_delta_remap() {
    let mut build_table = StringTable::new();
    let inherited_id = build_table.intern("base");

    let fork = build_table.fork_for_module();
    assert_eq!(fork.base_len(), build_table.len());

    let (module_table, base_len) = fork.into_parts();
    let remap = build_table.merge_delta_from(&module_table, base_len);

    assert!(remap.is_identity());
    assert!(!remap.has_non_identity_after(base_len));
    assert_eq!(remap.get(inherited_id), inherited_id);
}

#[test]
fn delta_merge_maps_inherited_ids_identity_and_appends_new_strings() {
    let mut build_table = StringTable::new();
    let inherited_id = build_table.intern("base");

    let fork = build_table.fork_for_module();
    let (mut module_table, base_len) = fork.into_parts();
    let module_only_id = module_table.intern("module-only");

    let remap = build_table.merge_delta_from(&module_table, base_len);
    let final_module_only_id = build_table.intern("module-only");

    assert!(remap.is_identity());
    assert_eq!(remap.get(inherited_id), inherited_id);
    assert_eq!(remap.get(module_only_id), final_module_only_id);
    assert_eq!(build_table.resolve(final_module_only_id), "module-only");
}

#[test]
fn overlapping_module_forks_remap_diverging_local_suffixes() {
    let mut build_table = StringTable::new();
    build_table.intern("base");

    let fork_source = build_table.fork_source();

    let first_fork = fork_source.fork_for_module();
    let (mut first_table, first_base_len) = first_fork.into_parts();
    let first_shared_id = first_table.intern("shared");
    let _first_only_id = first_table.intern("first-only");

    let second_fork = fork_source.fork_for_module();
    let (mut second_table, second_base_len) = second_fork.into_parts();
    let second_shared_id = second_table.intern("shared");
    let second_only_id = second_table.intern("second-only");

    let first_remap = build_table.merge_delta_from(&first_table, first_base_len);
    let shared_global_id = first_remap.get(first_shared_id);
    assert!(first_remap.is_identity());

    let second_remap = build_table.merge_delta_from(&second_table, second_base_len);
    let second_only_global_id = second_remap.get(second_only_id);

    assert!(!second_remap.is_identity());
    assert!(second_remap.has_non_identity_after(second_base_len));
    assert_eq!(second_remap.get(second_shared_id), shared_global_id);
    assert_ne!(second_only_global_id, second_only_id);
    assert_eq!(build_table.resolve(second_only_global_id), "second-only");

    let final_strings = build_table
        .iter()
        .map(|(_, string)| string.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        final_strings,
        vec![
            "base".to_owned(),
            "shared".to_owned(),
            "first-only".to_owned(),
            "second-only".to_owned(),
        ]
    );
}

#[test]
fn freezing_root_table_preserves_ids_and_moves_string_storage() {
    let mut table = StringTable::new();
    let first = table.intern("first");
    let second = table.intern("second");
    let first_pointer = table.resolve(first).as_ptr();
    let second_pointer = table.resolve(second).as_ptr();

    let frozen = table.freeze();

    assert_eq!(frozen.len(), 2);
    assert_eq!(frozen.resolve(first), "first");
    assert_eq!(frozen.resolve(second), "second");
    assert_eq!(frozen.resolve(first).as_ptr(), first_pointer);
    assert_eq!(frozen.resolve(second).as_ptr(), second_pointer);
    assert_eq!(frozen.try_resolve(first), Some("first"));
    assert_eq!(frozen.try_resolve(StringId::from_index(2)), None);
    assert_eq!(
        frozen.iter().collect::<Vec<_>>(),
        vec![(first, "first"), (second, "second")]
    );
}

#[test]
fn freezing_after_non_identity_merge_preserves_remapped_ids() {
    let mut merged = StringTable::new();
    let shared_id = merged.intern("shared");

    let mut module = StringTable::new();
    let module_only_id = module.intern("module-only");
    let module_shared_id = module.intern("shared");
    let remap = merged.merge_from(&module);

    let merged_module_only_id = remap.get(module_only_id);
    let merged_shared_id = remap.get(module_shared_id);
    let frozen = merged.freeze();

    assert_ne!(merged_module_only_id, module_only_id);
    assert_eq!(merged_shared_id, shared_id);
    assert_eq!(frozen.resolve(merged_module_only_id), "module-only");
    assert_eq!(frozen.resolve(merged_shared_id), "shared");
}

#[test]
#[should_panic(expected = "StringTable::freeze requires a merged root table")]
fn freezing_module_fork_requires_merge_into_root_first() {
    let mut root = StringTable::new();
    root.intern("inherited");
    let (fork, _) = root.fork_for_module().into_parts();

    let _ = fork.freeze();
}
