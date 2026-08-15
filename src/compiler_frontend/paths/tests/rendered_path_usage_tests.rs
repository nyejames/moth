//! Unit tests for semantic rendered-path capture helpers.

use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePathBase, CompileTimePathKind,
};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::rendered_path_usage::resolve_compile_time_path_for_rendered_output;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use std::fs;
use std::path::PathBuf;

struct TestHarness {
    project_root: PathBuf,
    resolver: ProjectPathResolver,
    string_table: StringTable,
    _temp_dir: tempfile::TempDir,
}

impl TestHarness {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let project_root = temp_dir.path().to_path_buf();
        let entry_root = project_root.join("src");

        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::create_dir_all(entry_root.join("assets/images")).expect("should create assets dir");
        fs::create_dir_all(entry_root.join("images")).expect("should create image dir");
        fs::create_dir_all(entry_root.join("docs")).expect("should create docs dir");
        fs::write(entry_root.join("assets/images/logo.png"), b"asset").expect("write asset");
        fs::write(entry_root.join("images/entry.png"), b"entry").expect("write entry asset");
        fs::write(entry_root.join("docs/local.png"), b"local").expect("write local asset");
        fs::write(entry_root.join("@page.moth"), b"").expect("write page");

        let resolver = ProjectPathResolver::new(
            project_root.clone(),
            entry_root,
            crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(
            ),
            &crate::builder_surface::SourceFileKindRegistry::default(),
        )
        .expect("resolver should build");

        Self {
            project_root,
            resolver,
            string_table: StringTable::new(),
            _temp_dir: temp_dir,
        }
    }

    fn path(&mut self, components: &[&str]) -> InternedPath {
        let mut path = InternedPath::new();
        for component in components {
            path.push_str(component, &mut self.string_table);
        }
        path
    }

    fn source_scope(&mut self) -> InternedPath {
        self.path(&["src", "@page.moth"])
    }

    fn declaring_file(&self) -> PathBuf {
        self.project_root.join("src/@page.moth")
    }

    fn render_location(&mut self) -> SourceLocation {
        SourceLocation::new(
            self.source_scope(),
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 12,
            },
        )
    }
}

#[test]
fn entry_root_render_capture_records_semantics_and_origin_aware_text() {
    let mut harness = TestHarness::new();
    let source_scope = harness.source_scope();
    let path = harness.path(&["assets", "images", "logo.png"]);
    let declaring_file = harness.declaring_file();
    let render_location = harness.render_location();

    let (_, recorded) = resolve_compile_time_path_for_rendered_output(
        &path,
        &harness.resolver,
        &declaring_file,
        &source_scope,
        &render_location,
        &PathStringFormatConfig {
            origin: String::from("/moth"),
            ..PathStringFormatConfig::default()
        },
        &mut harness.string_table,
    )
    .expect("capture should succeed");

    assert_eq!(recorded.rendered_text, "/moth/assets/images/logo.png");
    assert_eq!(recorded.usages.len(), 1);
    let usage = &recorded.usages[0];
    assert_eq!(usage.base, CompileTimePathBase::EntryRoot);
    assert_eq!(usage.kind, CompileTimePathKind::File);
    assert_eq!(
        usage.source_path.to_portable_string(&harness.string_table),
        "assets/images/logo.png"
    );
    assert_eq!(
        usage.public_path.to_portable_string(&harness.string_table),
        "assets/images/logo.png"
    );
    assert_eq!(usage.source_file_scope, source_scope);
    assert!(usage.filesystem_path.ends_with("assets/images/logo.png"));
}

#[test]
fn entry_root_render_capture_records_entry_root_semantics() {
    let mut harness = TestHarness::new();
    let source_scope = harness.source_scope();
    let path = harness.path(&["images", "entry.png"]);
    let declaring_file = harness.declaring_file();
    let render_location = harness.render_location();

    let (_, recorded) = resolve_compile_time_path_for_rendered_output(
        &path,
        &harness.resolver,
        &declaring_file,
        &source_scope,
        &render_location,
        &PathStringFormatConfig::default(),
        &mut harness.string_table,
    )
    .expect("capture should succeed");

    assert_eq!(recorded.rendered_text, "/images/entry.png");
    assert_eq!(recorded.usages[0].base, CompileTimePathBase::EntryRoot);
    assert_eq!(
        recorded.usages[0]
            .public_path
            .to_portable_string(&harness.string_table),
        "images/entry.png"
    );
}

#[test]
fn relative_render_capture_preserves_relative_text() {
    let mut harness = TestHarness::new();
    let source_scope = harness.source_scope();
    let path = harness.path(&[".", "docs", "local.png"]);
    let declaring_file = harness.declaring_file();
    let render_location = harness.render_location();

    let (_, recorded) = resolve_compile_time_path_for_rendered_output(
        &path,
        &harness.resolver,
        &declaring_file,
        &source_scope,
        &render_location,
        &PathStringFormatConfig {
            origin: String::from("/moth"),
            ..PathStringFormatConfig::default()
        },
        &mut harness.string_table,
    )
    .expect("capture should succeed");

    assert_eq!(recorded.rendered_text, "./docs/local.png");
    assert_eq!(recorded.usages[0].base, CompileTimePathBase::RelativeToFile);
    assert_eq!(
        recorded.usages[0]
            .public_path
            .to_portable_string(&harness.string_table),
        "./docs/local.png"
    );
}

#[test]
fn custom_origin_changes_rendered_text_but_not_semantic_public_path() {
    let mut harness = TestHarness::new();
    let source_scope = harness.source_scope();
    let path = harness.path(&["assets", "images", "logo.png"]);
    let declaring_file = harness.declaring_file();
    let render_location = harness.render_location();

    let (_, recorded) = resolve_compile_time_path_for_rendered_output(
        &path,
        &harness.resolver,
        &declaring_file,
        &source_scope,
        &render_location,
        &PathStringFormatConfig {
            origin: String::from("/custom"),
            ..PathStringFormatConfig::default()
        },
        &mut harness.string_table,
    )
    .expect("capture should succeed");

    assert_eq!(recorded.rendered_text, "/custom/assets/images/logo.png");
    assert_eq!(
        recorded.usages[0]
            .public_path
            .to_portable_string(&harness.string_table),
        "assets/images/logo.png"
    );
}
