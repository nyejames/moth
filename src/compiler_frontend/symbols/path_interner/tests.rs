use super::builder::PathNode;
use super::{PathId, PathIdRemap, PathInternerBuilder, PathInternerForkSource};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::mem::{align_of, size_of};
use std::path::PathBuf;

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
fn builder_exposes_current_table_before_consuming_freeze() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let path = builder
        .try_intern_portable_path("shared/prefix", &mut string_table)
        .unwrap();

    assert_eq!(builder.paths().depth(path), 2);
    let parent = builder
        .paths()
        .parent(path)
        .expect("path should have a parent");
    assert_eq!(builder.paths().depth(parent), 1);

    let frozen = builder.freeze();
    assert_eq!(frozen.depth(path), 2);
}

#[test]
fn append_reuses_children_and_adds_distinct_components_once() {
    let mut string_table = StringTable::new();
    let first_component = string_table.intern("first");
    let second_component = string_table.intern("second");
    let mut builder = PathInternerBuilder::new();

    let first = builder
        .try_intern_child(PathId::ROOT, first_component)
        .unwrap();
    let reused = builder
        .try_intern_child(PathId::ROOT, first_component)
        .unwrap();
    let second = builder
        .try_intern_child(PathId::ROOT, second_component)
        .unwrap();
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
    let abc = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();
    let abd = builder
        .try_intern_portable_path("a/b/d", &mut string_table)
        .unwrap();
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
    let first_path = builder
        .try_intern_portable_path("same/path", &mut string_table)
        .unwrap();
    let second_path = builder
        .try_intern_portable_path("same/path", &mut string_table)
        .unwrap();
    let prefix = builder
        .try_intern_portable_path("same", &mut string_table)
        .unwrap();

    assert_eq!(first_path, second_path);
    assert_ne!(first_path, prefix);
}

#[test]
fn parent_walking_reaches_root_in_order() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let a = builder
        .try_intern_portable_path("a", &mut string_table)
        .unwrap();
    let ab = builder
        .try_intern_portable_path("a/b", &mut string_table)
        .unwrap();
    let abc = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();
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
    let path = builder
        .try_intern_portable_path("styles/docs/navbar", &mut string_table)
        .unwrap();
    let root = builder
        .try_intern_portable_path("", &mut string_table)
        .unwrap();
    let slash = builder
        .try_intern_portable_path("/", &mut string_table)
        .unwrap();
    let trailing_separator = builder
        .try_intern_portable_path("styles/docs/navbar/", &mut string_table)
        .unwrap();
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
    let long_path = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();
    let short_path = builder
        .try_intern_portable_path("x", &mut string_table)
        .unwrap();
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

#[test]
fn path_id_try_from_index_spans_the_full_compact_domain() {
    assert_eq!(PathId::try_from_index(0), Some(PathId::ROOT));

    let last_index = u32::MAX as usize - 1;
    assert_eq!(
        PathId::try_from_index(last_index).map(PathId::index),
        Some(last_index)
    );

    // One past the last usable index is authored table exhaustion, reported fallibly instead
    // of wrapping the identity or panicking.
    assert_eq!(PathId::try_from_index(u32::MAX as usize), None);
    assert_eq!(
        PathId::try_from_index(u32::MAX as usize + 1),
        None,
        "indexes beyond the u32 domain must be rejected without a lossy cast"
    );
}

#[test]
fn prefix_checks_walk_ancestors_without_allocating() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let a = builder
        .try_intern_portable_path("a", &mut string_table)
        .unwrap();
    let ab = builder
        .try_intern_portable_path("a/b", &mut string_table)
        .unwrap();
    let abc = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();
    let abd = builder
        .try_intern_portable_path("a/b/d", &mut string_table)
        .unwrap();
    let table = builder.freeze();

    assert!(table.starts_with(abc, PathId::ROOT));
    assert!(table.starts_with(abc, a));
    assert!(table.starts_with(abc, ab));
    assert!(table.starts_with(abc, abc));
    assert!(table.starts_with(PathId::ROOT, PathId::ROOT));

    assert!(!table.starts_with(a, abc));
    assert!(!table.starts_with(abc, abd));
    assert!(!table.starts_with(PathId::ROOT, a));
}

#[test]
fn suffix_checks_compare_tip_components_without_allocating() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let ab = builder
        .try_intern_portable_path("a/b", &mut string_table)
        .unwrap();
    let abc = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();
    let bc = builder
        .try_intern_portable_path("b/c", &mut string_table)
        .unwrap();
    let c = builder
        .try_intern_portable_path("c", &mut string_table)
        .unwrap();
    let xy = builder
        .try_intern_portable_path("x/y", &mut string_table)
        .unwrap();
    let table = builder.freeze();

    assert!(table.ends_with(abc, c));
    assert!(table.ends_with(abc, bc));
    assert!(table.ends_with(abc, abc));
    assert!(table.ends_with(abc, PathId::ROOT));

    // A shared interior prefix is not a suffix unless it sits at the tip.
    assert!(!table.ends_with(abc, ab));
    assert!(!table.ends_with(c, abc));
    assert!(!table.ends_with(abc, xy));
}

#[test]
fn domain_wrapper_keeps_compact_layout_and_inner_equality() {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct LogicalPath(PathId);

    assert_eq!(size_of::<LogicalPath>(), 4);
    assert_eq!(size_of::<Option<LogicalPath>>(), 4);

    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let first = builder
        .try_intern_portable_path("domain/path", &mut string_table)
        .unwrap();
    let second = builder
        .try_intern_portable_path("domain/path", &mut string_table)
        .unwrap();
    let other = builder
        .try_intern_portable_path("domain/other", &mut string_table)
        .unwrap();

    assert_eq!(LogicalPath(first), LogicalPath(second));
    assert_ne!(LogicalPath(first), LogicalPath(other));
}

#[test]
fn checked_lookup_misses_foreign_ids() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let base = builder
        .try_intern_portable_path("base/leaf", &mut string_table)
        .unwrap();
    let fork_source = builder.fork_source();
    let mut fork = fork_source.fork_for_module();
    // Worker strings must fork the same base so inherited component IDs stay comparable;
    // a fresh table would reuse base numerics for different text and falsely share nodes.
    let string_source = string_table.fork_source();
    let (mut fork_strings, _) = string_source.fork_for_module().into_parts();
    let fork_only = fork
        .try_intern_portable_path("fork/only", &mut fork_strings)
        .unwrap();

    // An index past the destination length is invalid on that table.
    let table = builder.paths();
    let out_of_range = PathId::try_from_index(table.len())
        .expect("one past the table length must still be a well-formed handle");

    assert!(!table.contains(out_of_range));
    assert_eq!(table.try_parent(out_of_range), None);
    assert_eq!(table.try_component(out_of_range), None);
    assert_eq!(table.try_depth(out_of_range), None);

    // A worker-local identity is invalid on the destination until it is remapped.
    assert!(!table.contains(fork_only));
    assert_eq!(table.try_depth(fork_only), None);

    // Sanity: the inherited base identity remains valid in both contexts.
    assert!(table.contains(base));
    assert_eq!(table.try_depth(base), Some(2));
    assert!(fork.contains(base));
    assert_eq!(fork.try_depth(base), Some(2));
}

#[test]
fn fork_shares_base_identities_and_continues_local_suffix() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let base = builder
        .try_intern_portable_path("base/leaf", &mut string_table)
        .unwrap();
    let base_len = builder.len();

    let fork_source: PathInternerForkSource = builder.fork_source();
    assert_eq!(fork_source.base_len(), base_len);

    let mut fork = fork_source.fork_for_module();
    assert_eq!(fork.base_len(), base_len);
    assert_eq!(fork.len(), base_len);

    // Inherited prefix identities are shared by numeric value.
    assert_eq!(fork.parent(base), builder.paths().parent(base));
    assert_eq!(fork.depth(base), builder.paths().depth(base));

    let local = fork
        .try_intern_portable_path("base/leaf/child", &mut string_table)
        .unwrap();

    assert!(local.index() >= base_len);
    assert_eq!(fork.depth(local), 3);
    assert!(!builder.paths().contains(local));

    // The fork serves the same allocation-free reads as the frozen table.
    let leaf_component = string_table.intern("leaf");
    let local_parent = fork.parent(local).expect("child must have a parent");
    assert_eq!(local_parent, base);
    assert_eq!(fork.component(local_parent), Some(leaf_component));
    assert!(fork.starts_with(local, base));
    assert!(fork.ends_with(local, local));
    assert!(!fork.starts_with(base, local));

    let mut scratch = Vec::new();
    let child_component = string_table.intern("child");
    assert_eq!(
        fork.resolve_components(local, &mut scratch),
        &[string_table.intern("base"), leaf_component, child_component]
    );
    assert_eq!(
        fork.render_portable(local, &string_table, &mut scratch),
        "base/leaf/child"
    );
    assert_eq!(
        fork.render_native(local, &string_table, &mut scratch),
        PathBuf::from("base/leaf/child")
    );

    // Joining through the fork reuses existing children without extra allocation.
    let suffix = fork
        .try_intern_portable_path("leaf/child", &mut string_table)
        .unwrap();
    let prefix = fork
        .try_intern_portable_path("base", &mut string_table)
        .unwrap();
    let mut join_scratch = Vec::new();
    let joined = fork
        .try_join(prefix, suffix, &mut join_scratch)
        .expect("fork join must succeed");
    assert_eq!(joined, local);

    // Frozen strings serve the same fork renderers without copying the table.
    let frozen_strings = string_table.clone().freeze();
    assert_eq!(
        fork.render_portable_frozen(local, &frozen_strings, &mut scratch),
        "base/leaf/child"
    );
    assert_eq!(
        fork.render_native_frozen(local, &frozen_strings, &mut scratch),
        PathBuf::from("base/leaf/child")
    );

    // Filesystem interning in the fork converts components without storing a `PathBuf`.
    let filesystem_path = fork
        .try_intern_filesystem_path(
            std::path::Path::new("base/leaf/child"),
            &mut string_table,
        )
        .expect("fork filesystem interning must succeed");
    assert_eq!(filesystem_path, local);
}

fn intern_fork_paths(
    fork: &mut super::PathInternerFork,
    strings: &mut StringTable,
    spellings: &[&str],
) -> Vec<PathId> {
    spellings
        .iter()
        .map(|spelling| {
            fork.try_intern_portable_path(spelling, strings)
                .expect("fork interning must succeed")
        })
        .collect()
}

#[test]
fn forked_merges_collapse_shared_paths_in_either_order() {
    // First merge order: the earliest caller wins the shared numeric slot.
    let mut strings = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let base = builder
        .try_intern_portable_path("base/leaf", &mut strings)
        .unwrap();
    let path_base_len = builder.len();
    let string_source = strings.fork_source();
    let path_source = builder.fork_source();

    let (mut first_strings, first_string_base) =
        string_source.fork_for_module().into_parts();
    let mut first_fork = path_source.fork_for_module();
    let first_paths = intern_fork_paths(
        &mut first_fork,
        &mut first_strings,
        &["shared/new", "only-first"],
    );

    let (mut second_strings, second_string_base) =
        string_source.fork_for_module().into_parts();
    let mut second_fork = path_source.fork_for_module();
    let second_paths = intern_fork_paths(
        &mut second_fork,
        &mut second_strings,
        &["shared/new", "only-second"],
    );

    let first_string_remap = strings.merge_delta_from(&first_strings, first_string_base);
    let first_remap: PathIdRemap = builder
        .merge_delta_from(&first_fork, &first_string_remap)
        .expect("first path delta must merge");
    let second_string_remap = strings.merge_delta_from(&second_strings, second_string_base);
    let second_remap: PathIdRemap = builder
        .merge_delta_from(&second_fork, &second_string_remap)
        .expect("second path delta must merge");

    assert_eq!(first_remap.get(base), base);
    assert_eq!(second_remap.get(base), base);
    assert_eq!(first_remap.identity_prefix_len(), path_base_len);
    assert_eq!(second_remap.identity_prefix_len(), path_base_len);
    assert_eq!(first_remap.mapped_len(), 3);
    assert_eq!(second_remap.mapped_len(), 3);
    assert!(first_remap.is_identity());
    assert!(!second_remap.is_identity());
    assert_eq!(
        first_remap.get(first_paths[0]),
        second_remap.get(second_paths[0]),
        "independently interned shared paths must collapse"
    );
    assert_ne!(
        first_remap.get(first_paths[1]),
        second_remap.get(second_paths[1]),
        "distinct worker paths must stay distinct"
    );

    // Second merge order on a fresh base: the same collapse holds with caller-order numbering.
    let mut strings = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let base = builder
        .try_intern_portable_path("base/leaf", &mut strings)
        .unwrap();
    let string_source = strings.fork_source();
    let path_source = builder.fork_source();

    let (mut first_strings, first_string_base) =
        string_source.fork_for_module().into_parts();
    let mut first_fork = path_source.fork_for_module();
    let first_paths = intern_fork_paths(
        &mut first_fork,
        &mut first_strings,
        &["shared/new", "only-first"],
    );

    let (mut second_strings, second_string_base) =
        string_source.fork_for_module().into_parts();
    let mut second_fork = path_source.fork_for_module();
    let second_paths = intern_fork_paths(
        &mut second_fork,
        &mut second_strings,
        &["shared/new", "only-second"],
    );

    let second_string_remap = strings.merge_delta_from(&second_strings, second_string_base);
    let second_remap: PathIdRemap = builder
        .merge_delta_from(&second_fork, &second_string_remap)
        .expect("second path delta must merge");
    let first_string_remap = strings.merge_delta_from(&first_strings, first_string_base);
    let first_remap: PathIdRemap = builder
        .merge_delta_from(&first_fork, &first_string_remap)
        .expect("first path delta must merge");

    assert_eq!(first_remap.get(base), base);
    assert_eq!(second_remap.get(base), base);
    assert_eq!(
        first_remap.get(first_paths[0]),
        second_remap.get(second_paths[0]),
    );
    assert_ne!(
        first_remap.get(first_paths[1]),
        second_remap.get(second_paths[1]),
    );
}

#[test]
fn string_ids_remap_during_path_merge() {
    let mut strings = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let base = builder
        .try_intern_portable_path("base", &mut strings)
        .unwrap();

    let string_source = strings.fork_source();
    let path_source = builder.fork_source();

    let (mut worker_strings, string_base_len) =
        string_source.fork_for_module().into_parts();
    let mut worker_fork = path_source.fork_for_module();

    // This component exists only in the worker string table.
    let worker_component = worker_strings.intern("worker-only-component");
    let worker_path = worker_fork
        .try_intern_child(base, worker_component)
        .expect("fork interning must succeed");

    let string_remap = strings.merge_delta_from(&worker_strings, string_base_len);
    assert_eq!(string_remap.get(worker_component), strings.intern("worker-only-component"));

    let path_remap = builder
        .merge_delta_from(&worker_fork, &string_remap)
        .expect("worker path delta must merge");
    let merged = path_remap.get(worker_path);

    let table = builder.paths();
    let merged_component = table
        .component(merged)
        .expect("merged path must carry a component");
    assert_eq!(strings.resolve(merged_component), "worker-only-component");

    let mut scratch = Vec::new();
    assert_eq!(
        table.render_portable(merged, &strings, &mut scratch),
        "base/worker-only-component"
    );
}

#[test]
fn consuming_freeze_keeps_lookup_without_interning() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let ab = builder
        .try_intern_portable_path("a/b", &mut string_table)
        .unwrap();
    let abc = builder
        .try_intern_portable_path("a/b/c", &mut string_table)
        .unwrap();

    // `freeze` consumes the builder, so the resulting table offers lookup-only behaviour.
    let table = builder.freeze();
    let mut scratch = Vec::new();

    assert_eq!(table.parent(abc), Some(ab));
    assert_eq!(table.depth(abc), 3);
    assert!(table.starts_with(abc, ab));
    assert!(table.ends_with(abc, abc));
    assert_eq!(
        table.render_portable(abc, &string_table, &mut scratch),
        "a/b/c"
    );
    assert_eq!(
        table.render_native(abc, &string_table, &mut scratch),
        PathBuf::from("a/b/c")
    );
}

#[test]
fn portable_and_native_rendering_share_components() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let path = builder
        .try_intern_portable_path("styles/docs/navbar", &mut string_table)
        .unwrap();
    let table = builder.freeze();
    let mut scratch = Vec::new();

    let portable = table.render_portable(path, &string_table, &mut scratch);
    let native = table.render_native(path, &string_table, &mut scratch);

    assert_eq!(portable, "styles/docs/navbar");
    assert_eq!(native, PathBuf::from("styles/docs/navbar"));

    let mut expected = PathBuf::new();
    expected.push("styles");
    expected.push("docs");
    expected.push("navbar");
    assert_eq!(native, expected);

    assert_eq!(table.render_portable(PathId::ROOT, &string_table, &mut scratch), "");
    assert_eq!(
        table.render_native(PathId::ROOT, &string_table, &mut scratch),
        PathBuf::new()
    );

    // Frozen strings serve the same renderers without copying the table.
    let frozen_strings = string_table.clone().freeze();
    assert_eq!(
        table.render_portable_frozen(path, &frozen_strings, &mut scratch),
        "styles/docs/navbar"
    );
    assert_eq!(
        table.render_native_frozen(path, &frozen_strings, &mut scratch),
        PathBuf::from("styles/docs/navbar")
    );
    assert_eq!(
        table.render_native_frozen(PathId::ROOT, &frozen_strings, &mut scratch),
        PathBuf::new()
    );
}

#[test]
fn join_and_append_match_child_by_child_identity() {
    let mut string_table = StringTable::new();
    let mut builder = PathInternerBuilder::new();
    let prefix = builder
        .try_intern_portable_path("a/b", &mut string_table)
        .unwrap();
    let suffix = builder
        .try_intern_portable_path("c/d", &mut string_table)
        .unwrap();
    let direct = builder
        .try_intern_portable_path("a/b/c/d", &mut string_table)
        .unwrap();

    let mut scratch = Vec::new();
    let joined = builder
        .try_join(prefix, suffix, &mut scratch)
        .expect("join must succeed");

    assert_eq!(joined, direct);

    let component = string_table.intern("e");
    let appended = builder
        .try_intern_child(joined, component)
        .expect("append must succeed");
    let direct_appended = builder
        .try_intern_portable_path("a/b/c/d/e", &mut string_table)
        .unwrap();

    assert_eq!(appended, direct_appended);

    // Joining the empty suffix leaves the prefix unchanged.
    let mut scratch = Vec::new();
    let rejoined = builder
        .try_join(prefix, PathId::ROOT, &mut scratch)
        .expect("empty join must succeed");

    assert_eq!(rejoined, prefix);
}

#[cfg(unix)]
mod non_utf8_filesystem_conversion {
    use super::*;
    use crate::compiler_frontend::symbols::path_interner::{
        NonUtf8PathComponent, PathInternError,
    };
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn filesystem_interning_rejects_non_utf8_component_without_lossy_conversion() {
        let mut string_table = StringTable::new();
        let mut path_fork = super::super::PathInternerFork::empty();
        let bad_component = OsString::from_vec(vec![0xFF, 0xFE]);
        let path = PathBuf::from("valid").join(bad_component);

        let error = path_fork
            .try_intern_filesystem_path(&path, &mut string_table)
            .expect_err("non-UTF-8 path component should be rejected");

        assert!(matches!(
            error,
            PathInternError::NonUtf8(NonUtf8PathComponent { path: actual }) if actual == path
        ));
    }
}
