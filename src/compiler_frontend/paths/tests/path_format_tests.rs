//! Unit tests for compile-time path string formatting.

use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathKind, CompileTimePaths,
};
use crate::compiler_frontend::paths::path_format::{
    OutputPathStyle, PathStringFormatConfig, format_compile_time_path, format_compile_time_paths,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

fn make_path(
    components: &[&str],
    base: CompileTimePathBase,
    kind: CompileTimePathKind,
    string_table: &mut StringTable,
) -> CompileTimePath {
    let mut interned = InternedPath::new();
    for c in components {
        interned.push_str(c, string_table);
    }

    CompileTimePath {
        source_path: interned.clone(),
        filesystem_path: PathBuf::from("unused"),
        public_path: interned,
        base,
        kind,
    }
}

#[test]
fn entry_root_file_with_default_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &["assets", "images", "logo.png"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let config = PathStringFormatConfig::default();

    assert_eq!(
        format_compile_time_path(&path, &config, &st),
        "/assets/images/logo.png"
    );
}

#[test]
fn entry_root_file_with_custom_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &["assets", "images", "logo.png"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let config = PathStringFormatConfig {
        origin: String::from("/moth"),
        output_style: OutputPathStyle::Portable,
    };

    assert_eq!(
        format_compile_time_path(&path, &config, &st),
        "/moth/assets/images/logo.png"
    );
}

#[test]
fn directory_gets_trailing_slash() {
    let mut st = StringTable::new();
    let path = make_path(
        &["docs"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::Directory,
        &mut st,
    );
    let config = PathStringFormatConfig {
        origin: String::from("/moth"),
        output_style: OutputPathStyle::Portable,
    };

    assert_eq!(format_compile_time_path(&path, &config, &st), "/moth/docs/");
}

#[test]
fn relative_file_stays_relative_no_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &[".", "images", "logo.png"],
        CompileTimePathBase::RelativeToFile,
        CompileTimePathKind::File,
        &mut st,
    );
    let config = PathStringFormatConfig {
        origin: String::from("/moth"),
        output_style: OutputPathStyle::Portable,
    };

    assert_eq!(
        format_compile_time_path(&path, &config, &st),
        "./images/logo.png"
    );
}

#[test]
fn relative_directory_stays_relative_with_trailing_slash() {
    let mut st = StringTable::new();
    let path = make_path(
        &[".", "docs"],
        CompileTimePathBase::RelativeToFile,
        CompileTimePathKind::Directory,
        &mut st,
    );
    let config = PathStringFormatConfig::default();

    assert_eq!(format_compile_time_path(&path, &config, &st), "./docs/");
}

#[test]
fn entry_root_file_with_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &["pages", "about.html"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let config = PathStringFormatConfig {
        origin: String::from("/mysite"),
        output_style: OutputPathStyle::Portable,
    };

    assert_eq!(
        format_compile_time_path(&path, &config, &st),
        "/mysite/pages/about.html"
    );
}

#[test]
fn entry_root_empty_directory_with_default_origin_formats_as_public_root() {
    let mut st = StringTable::new();
    let path = make_path(
        &[],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::Directory,
        &mut st,
    );
    let config = PathStringFormatConfig::default();

    assert_eq!(format_compile_time_path(&path, &config, &st), "/");
}

#[test]
fn entry_root_empty_directory_with_custom_origin_formats_as_origin_root() {
    let mut st = StringTable::new();
    let path = make_path(
        &[],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::Directory,
        &mut st,
    );
    let config = PathStringFormatConfig {
        origin: String::from("/moth"),
        output_style: OutputPathStyle::Portable,
    };

    assert_eq!(format_compile_time_path(&path, &config, &st), "/moth/");
}

// -----------------------------------------------------------------------
// Multi-path formatting (`format_compile_time_paths`)
// -----------------------------------------------------------------------

#[test]
fn format_multiple_paths_joins_with_comma() {
    let mut st = StringTable::new();
    let path_a = make_path(
        &["assets", "logo.png"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let path_b = make_path(
        &["assets", "style.css"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let paths = CompileTimePaths {
        paths: vec![path_a, path_b],
    };
    let config = PathStringFormatConfig::default();

    assert_eq!(
        format_compile_time_paths(&paths, &config, &st),
        "/assets/logo.png, /assets/style.css"
    );
}

#[test]
fn format_single_path_in_multi_wrapper_has_no_comma() {
    let mut st = StringTable::new();
    let path = make_path(
        &[".", "readme.txt"],
        CompileTimePathBase::RelativeToFile,
        CompileTimePathKind::File,
        &mut st,
    );
    let paths = CompileTimePaths { paths: vec![path] };
    let config = PathStringFormatConfig::default();

    assert_eq!(
        format_compile_time_paths(&paths, &config, &st),
        "./readme.txt"
    );
}

#[test]
fn format_multiple_paths_with_mixed_bases_and_origin() {
    let mut st = StringTable::new();
    let root_path = make_path(
        &["assets", "logo.png"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::File,
        &mut st,
    );
    let relative_path = make_path(
        &[".", "local.txt"],
        CompileTimePathBase::RelativeToFile,
        CompileTimePathKind::File,
        &mut st,
    );
    let dir_path = make_path(
        &["docs"],
        CompileTimePathBase::EntryRoot,
        CompileTimePathKind::Directory,
        &mut st,
    );
    let paths = CompileTimePaths {
        paths: vec![root_path, relative_path, dir_path],
    };
    let config = PathStringFormatConfig {
        origin: String::from("/mysite"),
        output_style: OutputPathStyle::Portable,
    };

    // Root-based gets origin, relative stays relative, entry-root gets origin + trailing slash
    assert_eq!(
        format_compile_time_paths(&paths, &config, &st),
        "/mysite/assets/logo.png, ./local.txt, /mysite/docs/"
    );
}
