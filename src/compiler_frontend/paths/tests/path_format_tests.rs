//! Unit tests for compile-time path string formatting.

use crate::compiler_frontend::paths::compile_time_paths::{CompileTimePath, CompileTimePathBase};
use crate::compiler_frontend::paths::path_format::{
    OutputPathStyle, PathStringFormatConfig, format_compile_time_path,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

fn make_path(
    components: &[&str],
    base: CompileTimePathBase,
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
    }
}

fn origin(origin: &str) -> PathStringFormatConfig {
    PathStringFormatConfig {
        origin: String::from(origin),
        output_style: OutputPathStyle::Portable,
    }
}

#[test]
fn entry_root_file_with_default_origin_becomes_site_absolute() {
    let mut st = StringTable::new();
    let path = make_path(
        &["assets", "images", "logo.png"],
        CompileTimePathBase::EntryRoot,
        &mut st,
    );

    assert_eq!(
        format_compile_time_path(&path, &PathStringFormatConfig::default(), &st),
        "/assets/images/logo.png"
    );
}

#[test]
fn entry_root_file_is_prefixed_by_custom_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &["assets", "images", "logo.png"],
        CompileTimePathBase::EntryRoot,
        &mut st,
    );

    assert_eq!(
        format_compile_time_path(&path, &origin("/moth"), &st),
        "/moth/assets/images/logo.png"
    );
}

#[test]
fn relative_file_stays_relative_and_ignores_origin() {
    let mut st = StringTable::new();
    let path = make_path(
        &[".", "images", "logo.png"],
        CompileTimePathBase::RelativeToFile,
        &mut st,
    );

    assert_eq!(
        format_compile_time_path(&path, &origin("/moth"), &st),
        "./images/logo.png"
    );
}
