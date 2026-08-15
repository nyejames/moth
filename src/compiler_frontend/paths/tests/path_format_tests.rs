//! Unit tests for compile-time path string formatting.

use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathKind,
};
use crate::compiler_frontend::paths::path_format::{
    OutputPathStyle, PathStringFormatConfig, format_compile_time_path,
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
