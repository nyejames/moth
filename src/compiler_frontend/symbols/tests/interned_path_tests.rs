use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn to_portable_string_normalizes_windows_separator() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_components(vec![
        string_table.intern("styles"),
        string_table.intern("docs"),
        string_table.intern("navbar"),
    ]);

    assert_eq!(path.to_portable_string(&string_table), "styles/docs/navbar");
}

#[test]
fn try_from_filesystem_path_round_trips_temp_directory_path() {
    let mut string_table = StringTable::new();
    let temp_dir = tempfile::tempdir().expect("should create temp directory");

    let canonical = temp_dir
        .path()
        .canonicalize()
        .expect("temp directory should canonicalize");

    let path = InternedPath::try_from_filesystem_path(&canonical, &mut string_table)
        .expect("test path should be UTF-8");

    assert_eq!(path.to_path_buf(&string_table), canonical);
}

#[test]
fn parent_join_append_and_join_str_preserve_component_order() {
    let mut string_table = StringTable::new();
    let root = InternedPath::from_components(vec![
        string_table.intern("src"),
        string_table.intern("compiler_frontend"),
    ]);
    let suffix = InternedPath::from_components(vec![
        string_table.intern("hir"),
        string_table.intern("hir_builder.rs"),
    ]);

    let joined = root.join(&suffix);
    assert_eq!(
        joined.to_portable_string(&string_table),
        "src/compiler_frontend/hir/hir_builder.rs"
    );

    let parent = joined.parent().expect("joined path should have a parent");
    assert_eq!(
        parent.to_portable_string(&string_table),
        "src/compiler_frontend/hir"
    );

    let appended = root.append(string_table.intern("tests"));
    assert_eq!(
        appended.to_portable_string(&string_table),
        "src/compiler_frontend/tests"
    );

    let joined_str = root.join_str("ast", &mut string_table);
    assert_eq!(
        joined_str.to_portable_string(&string_table),
        "src/compiler_frontend/ast"
    );
}

#[test]
fn parent_of_single_component_path_is_empty_path() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("moth", &mut string_table);

    let parent = path
        .parent()
        .expect("single-component path should have a root parent");
    assert!(parent.is_empty());
    assert_eq!(parent.to_portable_string(&string_table), "");
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
        let bad_component = OsString::from_vec(vec![0xFF, 0xFE]);
        let path = std::path::PathBuf::from("valid").join(bad_component);

        let error = InternedPath::try_from_filesystem_path(&path, &mut string_table)
            .expect_err("non-UTF-8 path component should be rejected");

        assert_eq!(error.path, path, "error should retain the original path");
    }

    #[test]
    fn try_from_filesystem_path_preserves_valid_utf8_path() {
        let mut string_table = StringTable::new();
        let path = std::path::PathBuf::from("src")
            .join("compiler_frontend")
            .join("main.moth");

        let interned = InternedPath::try_from_filesystem_path(&path, &mut string_table)
            .expect("valid UTF-8 path should convert");

        assert_eq!(interned.to_path_buf(&string_table), path);
        assert_eq!(
            interned.to_portable_string(&string_table),
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
