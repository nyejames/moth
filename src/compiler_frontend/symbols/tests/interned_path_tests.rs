use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathInternError, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn to_portable_string_normalizes_windows_separator() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_components(&[
            string_table.intern("styles"),
            string_table.intern("docs"),
            string_table.intern("navbar"),
        ])
        .expect("test path fits");
    let mut scratch = Vec::new();
    assert_eq!(
        path_fork.render_portable(path, &string_table, &mut scratch),
        "styles/docs/navbar"
    );
}

#[test]
fn try_from_filesystem_path_round_trips_temp_directory_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let temp_dir = tempfile::tempdir().expect("should create temporary directory");
    let canonical = temp_dir
        .path()
        .canonicalize()
        .expect("temp directory should canonicalize");
    let path = path_fork
        .try_intern_filesystem_path(&canonical, &mut string_table)
        .expect("test path should be UTF-8");
    let mut scratch = Vec::new();
    assert_eq!(
        path_fork.render_native(path, &string_table, &mut scratch),
        canonical
    );
}

#[test]
fn parent_join_append_and_join_str_preserve_component_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let root = path_fork
        .try_intern_components(&[
            string_table.intern("src"),
            string_table.intern("compiler_frontend"),
        ])
        .expect("test path fits");
    let suffix = path_fork
        .try_intern_components(&[
            string_table.intern("hir"),
            string_table.intern("hir_builder.rs"),
        ])
        .expect("test path fits");
    let mut scratch = Vec::new();
    let joined = path_fork
        .try_join(root, suffix, &mut scratch)
        .expect("joined path should fit");
    assert_eq!(
        path_fork.render_portable(joined, &string_table, &mut scratch),
        "src/compiler_frontend/hir/hir_builder.rs"
    );
    let parent = path_fork.parent(joined).expect("joined path should have a parent");
    assert_eq!(
        path_fork.render_portable(parent, &string_table, &mut scratch),
        "src/compiler_frontend/hir"
    );
    let appended = path_fork
        .try_intern_child(root, string_table.intern("tests"))
        .expect("appended path should fit");
    assert_eq!(
        path_fork.render_portable(appended, &string_table, &mut scratch),
        "src/compiler_frontend/tests"
    );
    let joined_str = path_fork
        .try_intern_child(root, string_table.intern("ast"))
        .expect("string child should fit");
    assert_eq!(
        path_fork.render_portable(joined_str, &string_table, &mut scratch),
        "src/compiler_frontend/ast"
    );
}

#[test]
fn parent_of_single_component_path_is_empty_path() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("moth", &mut string_table)
        .expect("test path fits");
    let parent = path_fork
        .parent(path)
        .expect("single-component path should have a root parent");
    assert_eq!(parent, PathId::ROOT);
    let mut scratch = Vec::new();
    assert_eq!(
        path_fork.render_portable(parent, &string_table, &mut scratch),
        ""
    );
}

#[cfg(unix)]
mod non_utf8_filesystem_conversion {
    use super::*;
    use crate::compiler_frontend::symbols::interned_path::NonUtf8PathComponent;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn try_from_filesystem_path_rejects_non_utf8_component() {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let bad_component = OsString::from_vec(vec![0xFF, 0xFE]);
        let path = std::path::PathBuf::from("valid").join(bad_component);

        let error = path_fork.try_intern_filesystem_path(&path, &mut string_table)
            .expect_err("non-UTF-8 path component should be rejected");

        assert!(matches!(
            error,
            PathInternError::NonUtf8(NonUtf8PathComponent { path: actual }) if actual == path
        ));
    }

    #[test]
    fn try_from_filesystem_path_preserves_valid_utf8_path() {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let path = std::path::PathBuf::from("src")
            .join("compiler_frontend")
            .join("main.moth");

        let interned = path_fork.try_intern_filesystem_path(&path, &mut string_table)
            .expect("valid UTF-8 path should convert");

        let mut scratch = Vec::new();
        assert_eq!(path_fork.render_native(interned, &string_table, &mut scratch), path);
        assert_eq!(
            path_fork.render_portable(interned, &string_table, &mut scratch),
            "src/compiler_frontend/main.moth"
        );
    }

    #[test]
    fn non_utf8_path_component_retains_original_path() {
        let bad_component = OsString::from_vec(vec![0xC3, 0x28]);
        let path = std::path::PathBuf::from("root").join(bad_component);

        let error = NonUtf8PathComponent { path: path.clone() };

        assert_eq!(error.path, path);
    }
}
