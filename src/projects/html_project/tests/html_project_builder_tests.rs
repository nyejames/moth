//! Tests for HTML project builder orchestration.

use super::*;
use crate::backends::js::test_symbol_helpers::expected_dev_function_name;
use crate::build_system::BuildProfile;
use crate::build_system::build::{FileKind, Project, ProjectCompilation};
use crate::build_system::create_project_modules::resource_inputs::{
    ResourceContentState, ResourceInputRegistry,
};
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan, WriteMode,
    WriteOptions, write_project_outputs,
};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::module_compilation::ModuleExternalImport;
use crate::compiler_frontend::module_compilation::ResolvedConstFragment;
use crate::compiler_frontend::module_compilation::{Module, ModuleRootActivity};
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::projects::html_project::resource_output_plan::ResourceUseKind;
use crate::projects::html_project::tests::test_support::{
    add_reachable_external_import, collect_output_paths, create_test_module, expect_html_output,
    js_runtime_asset_import, non_js_runtime_asset_import,
};
use crate::projects::settings::Config;
use std::fs;
use std::path::Path;

fn attach_origin(
    registry: &mut ResourceInputRegistry,
    origin: StableResourceOriginId,
    source: ResourceSourceId,
) -> Result<(), CompilerError> {
    let publication = registry
        .preflight_resource_source_associations(&[ResourceSourceAssociation { origin, source }])?;
    registry.reserve_resource_source_associations(&publication);
    registry.commit_resource_source_associations(publication);
    Ok(())
}

fn project_resource_origin(resource_path: &str) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_portable_spelling(resource_path.to_owned())
            .expect("test resource path should be valid"),
    )
}

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
    let project_compilation =
        crate::build_system::test_support::project_compilation_from_test_modules(modules)
            .expect("test modules should assemble entries");
    builder.build_backend(
        project_compilation,
        config,
        BuildProfile::from_flags(flags),
        flags,
        &mut string_table,
    )
}

fn project_compilation(modules: Vec<Module>) -> ProjectCompilation {
    crate::build_system::test_support::project_compilation_from_test_modules(modules)
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
    let entry_path = PathBuf::from("@page.moth");
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
fn shared_resource_origin_defers_one_output_file_across_pages() {
    let directory = tempfile::tempdir().expect("should create resource directory");
    let source_path = directory.path().join("static/shared.bin");
    fs::create_dir_all(source_path.parent().expect("resource should have a parent"))
        .expect("should create resource directory");
    fs::write(&source_path, [1_u8, 2, 3]).expect("should write shared resource");
    let canonical_source_path =
        fs::canonicalize(&source_path).expect("shared resource should canonicalize");

    let resource_origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_portable_spelling("static/shared.bin".to_owned())
            .expect("shared resource path should be valid"),
    );
    let mut resource_inputs = ResourceInputRegistry::new();
    let source_id = resource_inputs.register_source(canonical_source_path);
    attach_origin(&mut resource_inputs, resource_origin.clone(), source_id)
        .expect("shared resource origin should attach");

    let mut string_table = StringTable::new();
    let mut first_page = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let mut second_page = create_test_module(PathBuf::from("docs.moth"), &mut string_table);
    for module in [&mut first_page, &mut second_page] {
        module
            .executable
            .resource_table
            .intern_origin(resource_origin.clone(), SourceLocation::default());
        module.metadata.const_top_level_fragments = vec![ResolvedConstFragment {
            runtime_insertion_index: 0,
            location: SourceLocation::default(),
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(
                resource_origin.clone(),
            )]),
        }];
    }

    let mut config = Config::new(PathBuf::from("docs.moth"));
    config.project_name = "test".to_owned();
    let project = HtmlProjectBuilder::new()
        .build_backend(
            crate::build_system::test_support::project_compilation_from_test_modules_with_resources(
                vec![first_page, second_page],
                resource_inputs,
            )
            .expect("test modules should assemble with resource inputs"),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("shared resource build should succeed");

    let shared_outputs = project
        .deferred_resources
        .iter()
        .filter(|resource| resource.relative_output_path == Path::new("static/shared.bin"))
        .count();
    assert_eq!(
        shared_outputs, 1,
        "shared origin/path should produce one deferred output"
    );
    assert_eq!(
        project.resource_inputs.records()[0].content(),
        ResourceContentState::Unhashed,
        "HTML planning must not read or hash shared resource bytes"
    );
}

#[test]
fn resource_destination_collision_preflights_before_read() {
    let directory = tempfile::tempdir().expect("should create resource directory");
    let source_path = directory.path().join("logo.svg");
    fs::write(&source_path, [1_u8, 2, 3]).expect("should write resource");
    let canonical_source_path =
        fs::canonicalize(&source_path).expect("resource should canonicalize");

    let origin = project_resource_origin("assets/logo.svg");
    let mut resource_inputs = ResourceInputRegistry::new();
    let source_id = resource_inputs.register_source(canonical_source_path);
    attach_origin(&mut resource_inputs, origin.clone(), source_id)
        .expect("resource origin should attach");

    let mut string_table = StringTable::new();
    let mut resource_output_plan = HtmlResourceOutputPlan::new("test");
    resource_output_plan
        .plan_origin(
            origin,
            SourceLocation::default(),
            ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .expect("resource should be planned");

    let output_path = PathBuf::from("assets/logo.svg");
    let output_paths = HashSet::from([output_path.clone()]);
    let output_files = Vec::new();
    let result = emit_planned_resource_outputs(
        resource_output_plan,
        &resource_inputs,
        &output_files,
        &output_paths,
        &mut string_table,
    );

    let messages = result.expect_err("an existing destination should fail emission");
    assert!(matches!(
        first_invalid_config_reason(&messages),
        InvalidConfigReason::ResourceOutputPathReserved { .. }
    ));
    assert!(output_files.is_empty(), "a failed preflight emits no files");
    assert_eq!(
        resource_inputs.records()[0].content(),
        ResourceContentState::Unhashed,
        "destination conflicts must not hash the source"
    );
}

#[test]
fn missing_module_source_preflights_before_reading_other_records() {
    let directory = tempfile::tempdir().expect("should create resource directory");
    let attached_path = directory.path().join("attached.bin");
    let unattached_path = directory.path().join("unattached.bin");
    fs::write(&attached_path, [1_u8, 2, 3]).expect("should write attached resource");
    fs::write(&unattached_path, [4_u8, 5, 6]).expect("should write unattached resource");

    let mut resource_inputs = ResourceInputRegistry::new();
    let attached_source = resource_inputs.register_source(
        fs::canonicalize(&attached_path).expect("attached resource should canonicalize"),
    );
    resource_inputs.register_source(
        fs::canonicalize(&unattached_path).expect("unattached resource should canonicalize"),
    );
    let attached_origin = project_resource_origin("assets/attached.bin");
    let unattached_origin = project_resource_origin("assets/unattached.bin");
    attach_origin(
        &mut resource_inputs,
        attached_origin.clone(),
        attached_source,
    )
    .expect("attached resource origin should attach");

    let mut string_table = StringTable::new();
    let mut resource_output_plan = HtmlResourceOutputPlan::new("test");
    for origin in [attached_origin, unattached_origin] {
        resource_output_plan
            .plan_origin(
                origin,
                SourceLocation::default(),
                ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
                &mut string_table,
                ResourceUseKind::Executable,
            )
            .expect("resource should be planned");
    }

    let output_files = Vec::new();
    let output_paths = HashSet::new();
    let result = emit_planned_resource_outputs(
        resource_output_plan,
        &resource_inputs,
        &output_files,
        &output_paths,
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "a missing module source should fail emission"
    );
    assert!(output_files.is_empty(), "a failed preflight emits no files");
    assert!(
        resource_inputs
            .records()
            .iter()
            .all(|record| record.content() == ResourceContentState::Unhashed),
        "a missing source must not allow an earlier record to be read"
    );
}

#[test]
fn successful_resource_emit_reads_and_writes_bytes() {
    let directory = tempfile::tempdir().expect("should create resource directory");
    let source_path = directory.path().join("logo.bin");
    fs::write(&source_path, [7_u8, 8, 9]).expect("should write resource");

    let origin = project_resource_origin("assets/logo.bin");
    let mut resource_inputs = ResourceInputRegistry::new();
    let source_id = resource_inputs
        .register_source(fs::canonicalize(&source_path).expect("resource should canonicalize"));
    attach_origin(&mut resource_inputs, origin.clone(), source_id)
        .expect("resource origin should attach");

    let mut string_table = StringTable::new();
    let mut resource_output_plan = HtmlResourceOutputPlan::new("test");
    resource_output_plan
        .plan_origin(
            origin,
            SourceLocation::default(),
            ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .expect("resource should be planned");

    let output_files = Vec::new();
    let output_paths = HashSet::new();
    let deferred_resources = emit_planned_resource_outputs(
        resource_output_plan,
        &resource_inputs,
        &output_files,
        &output_paths,
        &mut string_table,
    )
    .expect("resource should remain deferred after HTML planning");

    assert!(
        output_files.is_empty(),
        "HTML planning must not emit file bytes"
    );
    assert_eq!(
        resource_inputs.records()[0].content(),
        ResourceContentState::Unhashed,
        "HTML planning must not read or hash resource bytes"
    );
    assert!(output_paths.is_empty());
    assert_eq!(deferred_resources.len(), 1);

    let output_root = directory.path().join("output");
    let mut project = Project {
        output_files,
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: CleanupPolicy::html(),
        warnings: Vec::new(),
        deferred_resources,
        resource_inputs,
    };
    let options = WriteOptions {
        output_plan: OutputPlan::SingleFile(SingleFileOutputPlan {
            output_root: output_root.clone(),
            project_root: None,
            owner: OutputOwner {
                builder: BuilderKind::Html,
                profile: BuildProfile::Dev,
            },
            setting_location: SourceLocation::default(),
        }),
        write_mode: WriteMode::AlwaysWrite,
    };
    write_project_outputs(&mut project, &options, &mut string_table)
        .expect("central writer should materialise deferred resource bytes");

    assert_eq!(
        fs::read(output_root.join("assets/logo.bin")).expect("resource output should exist"),
        [7_u8, 8, 9]
    );
    assert!(matches!(
        project.resource_inputs.records()[0].content(),
        ResourceContentState::Read { .. }
    ));
}

#[test]
fn at_prefixed_route_name_strips_at_from_output() {
    let builder = HtmlProjectBuilder::new();
    let entry_path = PathBuf::from("@404.moth");
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
        vec![PathBuf::from("@page.moth"), PathBuf::from("@404.moth")],
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
        vec![PathBuf::from("@page.moth"), PathBuf::from("index.moth")],
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
    let entry_path = PathBuf::from("@page.moth");
    let mut string_table = StringTable::new();
    let mut module = create_test_module(entry_path.clone(), &mut string_table);
    module.metadata.const_top_level_fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        location: SourceLocation::default(),
        value: OwnedFoldedString::Text(String::from("<meta charset=\"utf-8\">")),
    }];

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &Config::new(entry_path),
            BuildProfile::Dev,
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
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
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
            entry_root.join("@home.moth"),
            entry_root.join("about").join("@anything.moth"),
            entry_root.join("docs").join("basics").join("@page.moth"),
            entry_root.join("blog").join("@404.moth"),
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
}

#[test]
fn js_runtime_asset_is_deferred_and_written_verbatim() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.js"), "export function foo() {}").expect("should write js");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("@page.moth"), &mut string_table);
    let runtime_asset =
        js_runtime_asset_import(Path::new("src/lib.js"), canonical_root.join("src/lib.js"));
    let asset_output_path = PathBuf::from(runtime_asset.origin.logical_path().as_str());
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(runtime_asset),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("build with JS asset should succeed");

    assert!(
        collect_output_paths(&project.output_files)
            .iter()
            .all(|path| !portable_path_text(path).contains("_moth/js/")),
        "planned JS runtime assets must not emit eager output files"
    );
    assert_eq!(project.deferred_resources.len(), 1);
    assert_eq!(
        project.deferred_resources[0].relative_output_path,
        asset_output_path
    );
    assert!(
        project
            .resource_inputs
            .records()
            .iter()
            .all(|record| record.content() == ResourceContentState::Unhashed),
        "HTML planning must not read or hash JS asset bytes"
    );

    let output_root = root.join("output");
    let mut project = project;
    let options = WriteOptions {
        output_plan: OutputPlan::SingleFile(SingleFileOutputPlan {
            output_root: output_root.clone(),
            project_root: None,
            owner: OutputOwner {
                builder: BuilderKind::Html,
                profile: BuildProfile::Dev,
            },
            setting_location: SourceLocation::default(),
        }),
        write_mode: WriteMode::AlwaysWrite,
    };
    write_project_outputs(&mut project, &options, &mut string_table)
        .expect("central writer should materialise the deferred JS asset");

    assert_eq!(
        fs::read_to_string(output_root.join(&asset_output_path))
            .expect("JS asset output should exist"),
        "export function foo() {}"
    );
    assert!(matches!(
        project.resource_inputs.records()[0].content(),
        ResourceContentState::Read { .. }
    ));
}

#[test]
fn js_runtime_asset_deduped_across_modules() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.js"), "export function foo() {}").expect("should write js");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module_a = create_test_module(canonical_root.join("@page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module_a,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("src/lib.js"),
                canonical_root.join("src/lib.js"),
            )),
            required_runtime_imports: vec![],
        },
    );

    let mut module_b =
        create_test_module(canonical_root.join("docs/@page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module_b,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("src/lib.js"),
                canonical_root.join("src/lib.js"),
            )),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module_a, module_b]),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    assert_eq!(
        project.deferred_resources.len(),
        1,
        "same canonical JS source referenced by multiple modules should plan one deferred output"
    );
}

#[test]
fn js_runtime_assets_with_same_stem_get_distinct_output_paths() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("a")).expect("should create a dir");
    fs::create_dir_all(root.join("b")).expect("should create b dir");
    fs::write(root.join("a/lib.js"), "export function a() {}").expect("should write a");
    fs::write(root.join("b/lib.js"), "export function b() {}").expect("should write b");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("@page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("a/lib.js"),
                canonical_root.join("a/lib.js"),
            )),
            required_runtime_imports: vec![],
        },
    );
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(2),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("b/lib.js"),
                canonical_root.join("b/lib.js"),
            )),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    let asset_paths: Vec<_> = project
        .deferred_resources
        .iter()
        .map(|resource| portable_path_text(&resource.relative_output_path))
        .collect();
    assert_eq!(
        asset_paths.len(),
        2,
        "two JS assets with same stem but different paths should get distinct output paths"
    );
    assert_ne!(asset_paths[0], asset_paths[1]);
    for asset_path in &asset_paths {
        assert!(
            asset_path.contains("_moth/js/"),
            "every JS asset should plan under _moth/js, got: {asset_path}"
        );
    }
}

#[test]
fn non_js_runtime_asset_is_ignored() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::write(root.join("src/lib.css"), "body {}").expect("should write css");
    let canonical_root = fs::canonicalize(&root).expect("root should resolve");

    let builder = HtmlProjectBuilder::new();
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();

    let mut module = create_test_module(canonical_root.join("@page.moth"), &mut string_table);
    add_reachable_external_import(
        &mut module,
        ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: Some(non_js_runtime_asset_import(
                "css",
                canonical_root.join("src/lib.css"),
            )),
            required_runtime_imports: vec![],
        },
    );

    let project = builder
        .build_backend(
            project_compilation(vec![module]),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("build should succeed");

    let has_js_assets = collect_output_paths(&project.output_files)
        .iter()
        .any(|p| portable_path_text(p).contains("_moth/js/"));
    assert!(
        !has_js_assets && project.deferred_resources.is_empty(),
        "non-JS runtime assets should be neither eagerly emitted nor deferred"
    );
}

#[test]
fn directory_build_supports_custom_entry_root_names() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("pages/docs")).expect("should create pages dir");
    let entry_root = fs::canonicalize(root.join("pages")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("pages");

    let project = build_with_test_modules(
        &builder,
        vec![
            entry_root.join("@page.moth"),
            entry_root.join("docs").join("@page.moth"),
        ],
        &config,
        &[],
    )
    .expect("directory build should succeed");

    let output_paths = collect_output_paths(&project.output_files);
    assert!(output_paths.contains(&PathBuf::from("index.html")));
    assert!(output_paths.contains(&PathBuf::from("docs/index.html")));
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));
}

#[test]
fn directory_build_requires_homepage_at_entry_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/about")).expect("should create about dir");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let result = build_with_test_modules(
        &builder,
        vec![entry_root.join("about").join("@page.moth")],
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
}

#[test]
fn directory_build_skips_api_only_sibling_from_all_artifact_planning() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/api")).expect("should create module directories");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let mut string_table = StringTable::new();

    let homepage = create_test_module(entry_root.join("@home.moth"), &mut string_table);
    let mut api_only = create_test_module(entry_root.join("api/@api.moth"), &mut string_table);
    api_only.metadata.root_activity = ModuleRootActivity::default();
    api_only.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(js_runtime_asset_import(
            Path::new("missing-runtime.js"),
            entry_root.join("missing-runtime.js"),
        )),
        required_runtime_imports: vec![],
    }];

    let project = builder
        .build_backend(
            project_compilation(vec![homepage, api_only]),
            &config,
            BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("API-only modules should not enter artifact planning");

    let output_paths = collect_output_paths(&project.output_files);
    assert_eq!(output_paths, vec![PathBuf::from("index.html")]);
    assert_eq!(project.entry_page_rel, Some(PathBuf::from("index.html")));
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
            BuildProfile::Dev,
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
    let entry_path = PathBuf::from("@page.moth");

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
        vec![PathBuf::from("@page.moth"), PathBuf::from("@404.moth")],
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
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/docs")).expect("should create docs dir");
    fs::create_dir_all(root.join("src/blog")).expect("should create blog dir");
    let entry_root = fs::canonicalize(root.join("src")).expect("entry root should resolve");

    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");

    let project = build_with_test_modules(
        &builder,
        vec![
            entry_root.join("@page.moth"),
            entry_root.join("docs").join("@page.moth"),
            entry_root.join("blog").join("@404.moth"),
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
}

#[test]
fn builder_rejects_invalid_origin_config() {
    let builder = HtmlProjectBuilder::new();
    let mut config = Config::new(PathBuf::from("."));
    config
        .settings
        .insert(String::from("origin"), String::from("not-a-slash"));

    let result = build_with_test_modules(&builder, vec![PathBuf::from("@page.moth")], &config, &[]);
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
