use super::builder::PathNode;
use super::{PathId, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::mem::{align_of, size_of};

#[test]
fn path_ids_and_nodes_have_dense_layout() {
    assert_eq!(size_of::<PathId>(), 4);
    assert_eq!(align_of::<PathId>(), 4);
    assert_eq!(size_of::<Option<PathId>>(), 4);
    assert_eq!(size_of::<PathNode>(), 8);
}

#[test]
fn root_has_no_parent_and_zero_depth() {
    let table = PathInternerBuilder::new().freeze();

    assert_eq!(table.parent(PathId::ROOT), None);
    assert_eq!(table.component(PathId::ROOT), None);
    assert_eq!(table.depth(PathId::ROOT), 0);
}

#[test]
fn append_reuses_children_and_adds_distinct_components_once() {
    let mut string_table = StringTable::new();
    let first_component = string_table.intern("first");
    let second_component = string_table.intern("second");
    let mut builder = PathInternerBuilder::new();

    let first = builder.intern_child(PathId::ROOT, first_component);
    let reused = builder.intern_child(PathId::ROOT, first_component);
    let second = builder.intern_child(PathId::ROOT, second_component);
    let table = builder.freeze();

    assert_eq!(first, reused);
    assert_ne!(first, second);
    assert_eq!(table.depth(first), 1);
    assert_eq!(table.depth(second), 1);
}

#[test]
fn shared_prefixes_have_one_node_per_unique_path() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let abc = builder.intern_portable_path("a/b/c", &mut string_table);
    let abd = builder.intern_portable_path("a/b/d", &mut string_table);
    let table = builder.freeze();

    let ab = table.parent(abc).expect("c should have a parent");
    assert_eq!(ab, table.parent(abd).expect("d should have a parent"));
    let a = table.parent(ab).expect("b should have a parent");
    assert_eq!(table.parent(a), Some(PathId::ROOT));
    assert_eq!(table.depth(abc), 3);
    assert_eq!(table.depth(abd), 3);
    assert_ne!(abc, abd);
}

#[test]
fn identical_complete_paths_are_equal_but_prefixes_are_not() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let first_path = builder.intern_portable_path("same/path", &mut string_table);
    let second_path = builder.intern_portable_path("same/path", &mut string_table);
    let prefix = builder.intern_portable_path("same", &mut string_table);

    assert_eq!(first_path, second_path);
    assert_ne!(first_path, prefix);
}

#[test]
fn parent_walking_reaches_root_in_order() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let a = builder.intern_portable_path("a", &mut string_table);
    let ab = builder.intern_portable_path("a/b", &mut string_table);
    let abc = builder.intern_portable_path("a/b/c", &mut string_table);
    let table = builder.freeze();

    assert_eq!(table.parent(abc), Some(ab));
    assert_eq!(table.parent(ab), Some(a));
    assert_eq!(table.parent(a), Some(PathId::ROOT));
    assert_eq!(table.parent(PathId::ROOT), None);
}

#[test]
fn rendering_is_portable_and_empty_paths_are_empty() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let path = builder.intern_portable_path("styles/docs/navbar", &mut string_table);
    let root = builder.intern_portable_path("", &mut string_table);
    let slash = builder.intern_portable_path("/", &mut string_table);
    let trailing_separator = builder.intern_portable_path("styles/docs/navbar/", &mut string_table);
    let table = builder.freeze();
    let mut scratch = Vec::new();
    let rendered = table.render_portable(path, &string_table, &mut scratch);

    assert_eq!(rendered, "styles/docs/navbar");
    assert_eq!(root, PathId::ROOT);
    assert_ne!(slash, PathId::ROOT);
    assert_ne!(trailing_separator, path);
    assert_eq!(table.render_portable(root, &string_table, &mut scratch), "");
    assert_eq!(
        table.render_portable(slash, &string_table, &mut scratch),
        "/"
    );
    assert_eq!(
        table.render_portable(trailing_separator, &string_table, &mut scratch),
        "styles/docs/navbar/"
    );
    assert!(rendered.contains('/'));
    assert!(!rendered.contains('\\'));
}

#[test]
fn component_resolution_reuses_scratch_without_stale_entries() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let long_path = builder.intern_portable_path("a/b/c", &mut string_table);
    let short_path = builder.intern_portable_path("x", &mut string_table);
    let table = builder.freeze();
    let a = string_table.intern("a");
    let b = string_table.intern("b");
    let c = string_table.intern("c");
    let x = string_table.intern("x");
    let mut scratch = Vec::new();

    assert_eq!(
        table.resolve_components(long_path, &mut scratch),
        &[a, b, c]
    );
    assert_eq!(table.resolve_components(short_path, &mut scratch), &[x]);
    assert_eq!(scratch, vec![x]);
}
