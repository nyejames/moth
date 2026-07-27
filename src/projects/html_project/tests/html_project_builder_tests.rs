//! Tests for HTML project builder orchestration.

use super::*;
use crate::backends::js::test_symbol_helpers::expected_dev_function_name;
use crate::build_system::build::ModuleExternalImport;
use crate::build_system::build::ResolvedConstFragment;
use crate::build_system::build::{
    FileKind, Module, ModuleRootActivity, Project, ProjectCompilation,
};
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePathBase, CompileTimePathKind,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::temp_dir;
use crate::projects::html_project::tests::test_support::{
    RenderedPathUsageInput, add_reachable_external_import, collect_output_paths,
    create_test_module, expect_bytes_output, expect_html_output, expect_js_output,
    rendered_path_usage,
};
use crate::projects::settings::Config;
use std::fs;
use std::path::PathBuf;

fn build_with_test_modules(
    builder: &HtmlProjectBuilder,
    entry_points: Vec<PathBuf>,
    config: &Config,
    flags: &[Flag],
) -> Result<Project, CompilerMessages> {
    let mut string_table = StringTable::new();
    let modules: Vec<Module> = entry_points
        .into_iter()
        .map(|entry_point| create_test_module(entry_point, &mut string_table))
        .collect();
    let project_compilation = ProjectCompilation::from_successful_modules(modules)
        .expect("test modules should assemble entries");
    builder.build_backend(project_compilation, config, flags, &mut string_table)
}

fn project_compilation(modules: Vec<Module>) -> ProjectCompilation {
    ProjectCompilation::from_successful_modules(modules)
        .expect("test modules should assemble entries")
}

fn first_invalid_config_reason(messages: &CompilerMessages) -> &InvalidConfigReason {
    assert!(
        messages.first_infrastructure_error_for_tests().is_none(),
        "project policy failures should stay as typed config diagnostics"
    );

    let diagnostic = messages
        .first_error()
        .expect("expected an error-severity diagnostic");
    let DiagnosticPayload::InvalidConfig { reason, .. } = &diagnostic.payload else {
        panic!("expected an invalid config diagnostic");
    };

    reason
}

#[test]
fn frontend_surface_registers_content_source_kinds() {
    let builder = HtmlProjectBuilder::new();
    let frontend_surface = builder.frontend_surface();

    assert_eq!(
        frontend_surface.source_file_kinds.kind_for_extension("mtf"),
        Some(crate::builder_surface::SourceFileKind::MothTemplate)
    );
    assert_eq!(
        frontend_surface.source_file_kinds.kind_for_extension("md"),
        Some(crate::builder_surface::SourceFileKind::PlainMarkdown)
    );

    assert_eq!(
        frontend_surface
            .source_file_kinds
            .kind_for_extension("moth"),
        None
    );
}

#[test]
fn frontend_surface_registers_core_packages_with_core_binding_metadata() {
    let frontend_surface = HtmlProjectBuilder::new().frontend_surface();

    for package_path in [
        "@core/collections",
        "@core/io",
        "@core/math",
        "@core/random",
        "@core/text",
        "@core/time",
    ] {
        let package = frontend_surface
            .binding_packages
            .get_package(package_path)
            .unwrap_or_else(|| panic!("HTML frontend surface should register {package_path}"));

        assert_eq!(
            package.metadata,
            crate::builder_surface::PackageMetadata::binding(
                crate::builder_surface::PackageOrigin::Core,
            )
        );
    }

    let html_package = frontend_surface
        .source_packages
        .get_root("html")
        .expect("HTML frontend surface should register @html");
    assert_eq!(
        html_package.metadata,
        crate::builder_surface::PackageMetadata::source(
            crate::builder_surface::PackageOrigin::Builder,
        )
    );

    let canvas_package = frontend_surface
        .binding_packages
        .get_package("@web/canvas")
        .expect("HTML frontend surface should register @web/canvas");
    assert_eq!(
        canvas_package.metadata,
        crate::builder_surface::PackageMetadata::binding(
            crate::builder_surface::PackageOrigin::Builder,
        )
    );

    assert!(
        frontend_surface
            .binding_packages
            .get_package("@core/prelude")
            .is_none(),
        "prelude is visibility policy, not a package"
    );
}

#[test]
fn build_backend_emits_single_html_output_file() {
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("#page.moth");
    let config = Config::new(entry_path.clone());

    let project = build_with_test_modules(&builder, vec![entry_path], &config, &[])
        .expect("build_backend should succeed");

    assert_eq!(project.output_files.len(), 1);
    assert_eq!(
        project.output_files[0].relative_output_path(),
        PathBuf::from("index.html")
    );
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));
    assert!(matches!(
        project.output_files[0].file_kind(),
        FileKind::Html(_)
    ));
}

#[test]
fn hash_prefixed_route_name_strips_hash_from_output() {
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("#404.moth");
    let config = Config::new(entry_path.clone());

    let project = build_with_test_modules(&builder, vec![entry_path], &config, &[])
        .expect("build_backend should succeed");

    assert_eq!(
        project.output_files[0].relative_output_path(),
        PathBuf::from("404.html")
    );
}

#[test]
fn build_backend_emits_html_for_multiple_modules() {
    let builder = HtmlProjectBuilder::new();
    let config = Config::new(PathBuf::from("docs.moth"));

    let project = build_with_test_modules(
        &builder,
        vec![PathBuf::from("#page.moth"), PathBuf::from("#404.moth")],
        &config,
        &[],
    )
    .expect("build_backend should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert_eq!(project.output_files.len(), 2);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("404.html")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));
}

#[test]
fn duplicate_output_paths_are_rejected() {
    let builder = HtmlProjectBuilder::new();
    let config = Config::new(PathBuf::from("docs.moth"));

    let result = build_with_test_modules(
        &builder,
        vec![PathBuf::from("#page.moth"), PathBuf::from("index.moth")],
        &config,
        &[],
    );

    let err = match result {
        Err(messages) => messages,
        Ok(_) => panic!("duplicate output paths should fail"),
    };
    let reason = first_invalid_config_reason(&err);
    let InvalidConfigReason::DuplicateHtmlOutputPath { output_path, .. } = reason else {
        panic!("expected duplicate HTML output-path config reason");
    };
    assert_eq!(err.string_table.resolve(*output_path), "index.html");
}

#[test]
fn emits_const_fragment_and_calls_start() {
    // WHAT: verify the builder embeds a compile-time const fragment and emits a start() call.
    // WHY: root activity metadata supplies the slot count; the test module has no runtime slots,
    //      so only the const fragment and start() invocation are asserted here.
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("#page.moth");
    let mut string_table = StringTable::new();
    let mut module = create_test_module(entry_path.clone(), &mut string_table);
    module.metadata.const_top_level_fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        rendered_text: String::from("<meta charset=\"utf-8\">"),
    }];

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &Config::new(entry_path),
            &[],
            &mut string_table,
        )
        .expect("build_backend should succeed");

    let html = expect_html_output(&project.output_files, "index.html");
    let start_name = expected_dev_function_name("start_entry", 0);

    assert!(html.contains("<meta charset=\"utf-8\">"));
    assert!(
        html.contains(&format!("{start_name}()")),
        "start() must be called in the emitted HTML"
    );
}

#[test]
fn directory_build_maps_routes_relative_to_entry_root() {
    let root = temp_dir("directory_routes");
    fs::create_dir_all(root.join("src/about")).expect("should create about dir");
    fs::create_dir_all(root.join("src/docs/basics")).expect("should create docs dir");
    fs::create_dir_all(root.join("src/blog")).expect("should create blog dir");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let project = build_with_test_modules(
        &builder,
        vec![
            entry_root.join("#home.moth"),
            entry_root.join("about").join("#anything.moth"),
            entry_root.join("docs").join("basics").join("#page.moth"),
            entry_root.join("blog").join("#404.moth"),
        ],
        &config,
        &[],
    )
    .expect("directory build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("about/index.html")));
    assert!(output_paths.contains(&PathBuf::from("docs/basics/index.html")));
    assert!(output_paths.contains(&PathBuf::from("blog/index.html")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn js_runtime_asset_emitted_verbatim() {
    let root = temp_dir("js_runtime_asset");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.js"), "export function foo() {}").expect("should write js");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("src/lib.js"),
                asset_kind: "js".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("build with JS asset should succeed");

    let js_paths: Vec<_> = collect_output_paths(&project.output_files)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("_moth/js/"))
        .collect();
    assert_eq!(
        js_paths.len(),
        1,
        "should emit exactly one JS runtime asset"
    );

    let js_path = js_paths[0].to_str().unwrap();
    let js_content = expect_js_output(&project.output_files, js_path);
    assert_eq!(js_content, "export function foo() {}");

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn js_runtime_asset_deduped_across_modules() {
    let root = temp_dir("js_runtime_dedupe");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.js"), "export function foo() {}").expect("should write js");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module_a = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module_a,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("src/lib.js"),
                asset_kind: "js".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );

    let mut module_b =
        create_test_module(canonical_root.join("docs/#page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module_b,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("src/lib.js"),
                asset_kind: "js".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module_a, module_b]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    let js_count = collect_output_paths(&project.output_files)
        .iter()
        .filter(|p| p.to_string_lossy().contains("_moth/js/"))
        .count();
    assert_eq!(
        js_count, 1,
        "same canonical JS source referenced by multiple modules should emit one output file"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn js_runtime_assets_with_same_stem_get_distinct_output_paths() {
    let root = temp_dir("js_runtime_same_stem");
    fs::create_dir_all(root.join("a")).expect("should create a dir");
    fs::create_dir_all(root.join("b")).expect("should create b dir");
    fs::write(root.join("a/lib.js"), "export function a() {}").expect("should write a");
    fs::write(root.join("b/lib.js"), "export function b() {}").expect("should write b");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("a/lib.js"),
                asset_kind: "js".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(2),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("b/lib.js"),
                asset_kind: "js".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    let js_paths: Vec<_> = collect_output_paths(&project.output_files)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains("_moth/js/"))
        .collect();
    assert_eq!(
        js_paths.len(),
        2,
        "two JS assets with same stem but different paths should get distinct output paths"
    );
    assert_ne!(js_paths[0], js_paths[1]);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn non_js_runtime_asset_is_ignored() {
    let root = temp_dir("non_js_runtime_asset");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.css"), "body {}").expect("should write css");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: canonical_root.join("src/lib.css"),
                asset_kind: "css".to_owned(),
            }),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    let has_js_assets = collect_output_paths(&project.output_files)
        .iter()
        .any(|p| p.to_string_lossy().contains("_moth/js/"));
    assert!(
        !has_js_assets,
        "non-JS runtime assets should not be emitted as JS"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn directory_build_supports_custom_entry_root_names() {
    let root = temp_dir("custom_entry_root");
    fs::create_dir_all(root.join("pages/docs")).expect("should create pages dir");
    let entry_root = fs::canonicalize(root.join("pages")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("pages");

    let project = build_with_test_modules(
        &builder,
        vec![
            entry_root.join("#page.moth"),
            entry_root.join("docs").join("#page.moth"),
        ],
        &config,
        &[],
    )
    .expect("directory build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("docs/index.html")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn directory_build_requires_homepage_at_entry_root() {
    let root = temp_dir("missing_homepage");
    fs::create_dir_all(root.join("src/about")).expect("should create about dir");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let result = build_with_test_modules(
        &builder,
        vec![entry_root.join("about").join("#page.moth")],
        &config,
        &[],
    );

    let err = match result {
        Err(messages) => messages,
        Ok(_) => panic!("missing homepage should fail"),
    };
    let reason = first_invalid_config_reason(&err);
    let InvalidConfigReason::MissingHtmlHomepage {
        entry_root: reported_entry_root,
    } = reason
    else {
        panic!("expected missing HTML homepage config reason");
    };
    assert_eq!(
        err.string_table.resolve(*reported_entry_root),
        entry_root.display().to_string()
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn directory_build_skips_api_only_sibling_from_all_artifact_planning() {
    let root = temp_dir("api_only_sibling");
    fs::create_dir_all(root.join("src/api")).expect("should create module directories");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let mut string_table = StringTable::new();

    let homepage = create_test_module(entry_root.join("#home.moth"), &mut string_table);
    let mut api_only = create_test_module(entry_root.join("api/#api.moth"), &mut string_table);
    api_only.metadata.root_activity = ModuleRootActivity::default();
    api_only.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(RuntimeAssetIdentity {
            canonical_source_path: entry_root.join("missing-runtime.js"),
            asset_kind: "js".to_owned(),
        }),
        required_runtime_imports: vec![],
    }];
    api_only
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["missing-asset.png"],
                public_path_components: &["assets", "missing-asset.png"],
                filesystem_path: entry_root.join("missing-asset.png"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["api", "#api.moth"],
                line_number: 1,
            },
        ));

    let project = builder
        .build_backend(
            project_compilation(vec![homepage, api_only]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("API-only modules should not enter artifact planning");

    let output_paths = collect_output_paths(&project.output_files);
    assert_eq!(output_paths, vec![PathBuf::from("index.html")]);
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn single_file_api_only_build_can_emit_no_artifacts() {
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("api.moth");
    let mut string_table = StringTable::new();
    let mut api_only = create_test_module(entry_path.clone(), &mut string_table);
    api_only.metadata.root_activity = ModuleRootActivity::default();

    let project = builder
        .build_backend(
            project_compilation(vec![api_only]),
            &Config::new(entry_path),
            &[],
            &mut string_table,
        )
        .expect("single-file API-only build should not require an entry page");

    assert!(project.output_files.is_empty());
    assert_eq!(project.entry_page_rel, None);
}

#[test]
fn wasm_flag_emits_html_js_and_wasm_artifacts() {
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("#page.moth");

    let project = build_with_test_modules(
        &builder,
        vec![entry_path.clone()],
        &Config::new(entry_path),
        &[Flag::HtmlWasm],
    )
    .expect("wasm mode build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("page.js")));
    assert!(output_paths.contains(&PathBuf::from("page.wasm")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));
    assert!(
        project
            .output_files
            .iter()
            .any(|file| matches!(file.file_kind(), FileKind::Wasm(_))),
        "expected one wasm artifact in wasm mode"
    );
}

#[test]
fn wasm_mode_uses_per_page_folder_layout() {
    let builder = HtmlProjectBuilder::new();
    let config = Config::new(PathBuf::from("docs.moth"));

    let project = build_with_test_modules(
        &builder,
        vec![PathBuf::from("#page.moth"), PathBuf::from("#404.moth")],
        &config,
        &[Flag::HtmlWasm],
    )
    .expect("wasm mode build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("page.js")));
    assert!(output_paths.contains(&PathBuf::from("page.wasm")));
    assert!(output_paths.contains(&PathBuf::from("404/index.html")));
    assert!(output_paths.contains(&PathBuf::from("404/page.js")));
    assert!(output_paths.contains(&PathBuf::from("404/page.wasm")));
}

#[test]
fn wasm_directory_build_preserves_nested_routes() {
    let root = temp_dir("wasm_directory_routes");
    fs::create_dir_all(root.join("src/docs")).expect("should create docs dir");
    fs::create_dir_all(root.join("src/blog")).expect("should create blog dir");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let project = build_with_test_modules(
        &builder,
        vec![
            entry_root.join("#page.moth"),
            entry_root.join("docs").join("#page.moth"),
            entry_root.join("blog").join("#404.moth"),
        ],
        &config,
        &[Flag::HtmlWasm],
    )
    .expect("wasm directory build should succeed without duplicate output paths");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("page.js")));
    assert!(output_paths.contains(&PathBuf::from("page.wasm")));
    assert!(output_paths.contains(&PathBuf::from("docs/index.html")));
    assert!(output_paths.contains(&PathBuf::from("docs/page.js")));
    assert!(output_paths.contains(&PathBuf::from("docs/page.wasm")));
    assert!(output_paths.contains(&PathBuf::from("blog/index.html")));
    assert!(output_paths.contains(&PathBuf::from("blog/page.js")));
    assert!(output_paths.contains(&PathBuf::from("blog/page.wasm")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn builder_rejects_invalid_origin_config() {
    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(PathBuf::from("."));
    config
        .settings
        .insert(String::from("origin"), String::from("not-a-slash"));

    let result = build_with_test_modules(&builder, vec![PathBuf::from("#page.moth")], &config, &[]);
    let messages = match result {
        Err(messages) => messages,
        Ok(_) => panic!("invalid origin should fail"),
    };
    let diagnostic = messages
        .first_error()
        .expect("invalid origin should produce a diagnostic");
    let DiagnosticPayload::InvalidConfig {
        reason: InvalidConfigReason::InvalidProjectSettingValue { expected, .. },
        ..
    } = &diagnostic.payload
    else {
        panic!("invalid origin should remain a typed config diagnostic");
    };
    assert!(
        messages
            .string_table
            .resolve(*expected)
            .contains("starts with '/'")
    );
}

#[test]
fn build_backend_emits_tracked_assets_and_dedupes_same_source_output() {
    let root = temp_dir("builder_tracked_asset_dedupe");
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::write(root.join("assets/logo.png"), [1_u8, 2, 3]).expect("should write asset");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut homepage = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    homepage
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "logo.png"],
                public_path_components: &["assets", "logo.png"],
                filesystem_path: canonical_root.join("assets/logo.png"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["#page.moth"],
                line_number: 1,
            },
        ));

    let mut docs_page =
        create_test_module(canonical_root.join("docs/#page.moth"), &mut string_table);
    docs_page
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "logo.png"],
                public_path_components: &["assets", "logo.png"],
                filesystem_path: canonical_root.join("assets/logo.png"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["docs", "#page.moth"],
                line_number: 1,
            },
        ));

    let project = builder
        .build_backend(
            project_compilation(vec![homepage, docs_page]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("tracked-asset build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("assets/logo.png")));
    assert_eq!(
        expect_bytes_output(&project.output_files, "assets/logo.png"),
        [1_u8, 2, 3]
    );
    assert_eq!(
        project
            .output_files
            .iter()
            .filter(|file| matches!(file.file_kind(), FileKind::Bytes(_)))
            .count(),
        1,
        "same source/same emitted path should dedupe"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_backend_allows_same_source_file_to_emit_multiple_relative_outputs() {
    let root = temp_dir("builder_tracked_asset_relative_copies");
    fs::create_dir_all(root.join("blog/post")).expect("should create blog dir");
    fs::create_dir_all(root.join("shared")).expect("should create shared dir");
    fs::write(root.join("shared/logo.png"), [4_u8, 5, 6]).expect("should write asset");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut homepage = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    homepage
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &[".", "logo.png"],
                public_path_components: &[".", "logo.png"],
                filesystem_path: canonical_root.join("shared/logo.png"),
                base: CompileTimePathBase::RelativeToFile,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["#page.moth"],
                line_number: 1,
            },
        ));

    let mut blog_page = create_test_module(
        canonical_root.join("blog/post/#page.moth"),
        &mut string_table,
    );
    blog_page
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["..", "shared", "logo.png"],
                public_path_components: &["..", "shared", "logo.png"],
                filesystem_path: canonical_root.join("shared/logo.png"),
                base: CompileTimePathBase::RelativeToFile,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["blog", "post", "#page.moth"],
                line_number: 1,
            },
        ));

    let project = builder
        .build_backend(
            project_compilation(vec![homepage, blog_page]),
            &config,
            &[],
            &mut string_table,
        )
        .expect("tracked-asset build should succeed");

    assert_eq!(
        expect_bytes_output(&project.output_files, "logo.png"),
        [4_u8, 5, 6]
    );
    assert_eq!(
        expect_bytes_output(&project.output_files, "blog/shared/logo.png"),
        [4_u8, 5, 6]
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_backend_rejects_conflicting_tracked_asset_output_paths() {
    let root = temp_dir("builder_tracked_asset_conflict");
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::create_dir_all(root.join("docs")).expect("should create docs dir");
    fs::write(root.join("assets/logo-a.png"), [1_u8]).expect("should write first asset");
    fs::write(root.join("assets/logo-b.png"), [2_u8]).expect("should write second asset");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut homepage = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    homepage
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "logo.png"],
                public_path_components: &["assets", "logo.png"],
                filesystem_path: canonical_root.join("assets/logo-a.png"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["#page.moth"],
                line_number: 1,
            },
        ));

    let mut docs_page =
        create_test_module(canonical_root.join("docs/#page.moth"), &mut string_table);
    docs_page
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "logo.png"],
                public_path_components: &["assets", "logo.png"],
                filesystem_path: canonical_root.join("assets/logo-b.png"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["docs", "#page.moth"],
                line_number: 1,
            },
        ));

    let error = match builder.build_backend(
        project_compilation(vec![homepage, docs_page]),
        &config,
        &[],
        &mut string_table,
    ) {
        Err(messages) => messages,
        Ok(_) => panic!("conflicting tracked assets should fail"),
    };

    let reason = first_invalid_config_reason(&error);
    let InvalidConfigReason::TrackedAssetOutputConflict { output_path, .. } = reason else {
        panic!("expected tracked-asset output conflict config reason");
    };
    assert_eq!(error.string_table.resolve(*output_path), "assets/logo.png");

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_backend_rejects_tracked_asset_output_that_matches_generated_html() {
    let root = temp_dir("builder_tracked_asset_generated_output_conflict");
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::write(root.join("assets/copied.html"), b"asset").expect("should write asset");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut homepage = create_test_module(canonical_root.join("#page.moth"), &mut string_table);
    homepage
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "copied.html"],
                public_path_components: &["index.html"],
                filesystem_path: canonical_root.join("assets/copied.html"),
                base: CompileTimePathBase::EntryRoot,
                kind: CompileTimePathKind::File,
                source_file_scope_components: &["#page.moth"],
                line_number: 1,
            },
        ));

    let error = match builder.build_backend(
        project_compilation(vec![homepage]),
        &config,
        &[],
        &mut string_table,
    ) {
        Err(messages) => messages,
        Ok(_) => panic!("tracked asset should not overwrite generated HTML output"),
    };

    let reason = first_invalid_config_reason(&error);
    let InvalidConfigReason::TrackedAssetBuilderOutputConflict { output_path, .. } = reason else {
        panic!("expected tracked asset versus generated output config reason");
    };
    assert_eq!(error.string_table.resolve(*output_path), "index.html");

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
