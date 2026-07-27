//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{
    FileKind, OutputFile, Project, ProjectBuilder, build_project, resolve_project_output_root,
};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, resolve_source_file_path, terse,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticPayload, InvalidConfigReason,
};
use crate::compiler_frontend::utilities::basic::normalize_path;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::Config;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn rendered_error_messages(messages: &CompilerMessages) -> Vec<String> {
    let context = DiagnosticRenderContext::new(&messages.string_table);
    messages
        .error_diagnostics()
        .map(|diagnostic| terse::format_terse_diagnostic_with_context(diagnostic, context))
        .collect()
}

fn assert_has_config_error(messages: &CompilerMessages) {
    assert!(
        messages
            .error_diagnostics()
            .any(|diagnostic| diagnostic.kind.category() == DiagnosticCategory::Config),
        "expected config-classified diagnostic"
    );
}

fn assert_invalid_project_setting(
    messages: &CompilerMessages,
    expected_key: &str,
    expected_value: &str,
) {
    let has_expected_diagnostic = messages.error_diagnostics().any(|diagnostic| {
        let DiagnosticPayload::InvalidConfig {
            key: Some(key),
            reason: InvalidConfigReason::InvalidProjectSettingValue { value, .. },
        } = &diagnostic.payload
        else {
            return false;
        };

        messages.string_table.resolve(*key) == expected_key
            && messages.string_table.resolve(*value) == expected_value
    });

    assert!(
        has_expected_diagnostic,
        "expected invalid project setting diagnostic for {expected_key}={expected_value}"
    );
}

#[test]
fn build_project_returns_result_without_writing_files() {
    let root = temp_dir("build_only");
    fs::create_dir_all(&root).expect("should create temp root");
    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        entry_file
            .to_str()
            .expect("temp file path should be valid UTF-8 for this test"),
        &[],
    )
    .expect("build should succeed");

    assert!(!result.project.output_files.is_empty());
    assert!(
        !root.join("index.html").exists(),
        "build_project should not write files to disk"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_project_preserves_builder_warnings_in_build_result() {
    let root = temp_dir("warnings");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join("main.moth"), "value = 1\n").expect("should write source file");

    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let result = build_project(
            &ProjectBuilder::new(Box::new(WarningBuilder)),
            "main.moth",
            &[],
        )
        .expect("build should succeed");

        assert!(
            result.warnings.len() == 1,
            "build result should include backend warnings"
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_project_calls_validate_project_config() {
    let root = temp_dir("validation_tracking");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join("main.moth"), "value = 1\n").expect("should write source file");
    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let validated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let built = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let builder = ProjectBuilder::new(Box::new(ValidationTrackingBuilder {
            validated: validated.clone(),
            built: built.clone(),
        }));

        build_project(&builder, "main.moth", &[]).expect("build should succeed");

        assert!(
            validated.load(std::sync::atomic::Ordering::SeqCst),
            "build_project should call validate_project_config"
        );
        assert!(
            built.load(std::sync::atomic::Ordering::SeqCst),
            "build_project should call build_backend"
        );
    }
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn project_compilation_selects_only_modules_with_root_activity_as_entries() {
    let root = temp_dir("project_compilation_entries");
    let src = root.join("src");
    fs::create_dir_all(src.join("api")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "value = 1\n").expect("should write active root");
    fs::write(src.join("api/#api.moth"), "value #= 1\n").expect("should write API-only root");

    let module_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let entry_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let builder = ProjectBuilder::new(Box::new(EntryTrackingBuilder {
        module_count: module_count.clone(),
        entry_count: entry_count.clone(),
    }));

    build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    )
    .expect("directory frontend and test backend should succeed");

    assert_eq!(module_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(entry_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn diagnosed_module_prevents_project_compilation_from_reaching_backend() {
    let root = temp_dir("diagnosed_project_compilation");
    let src = root.join("src");
    fs::create_dir_all(src.join("broken")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "value = 1\n").expect("should write valid root");
    fs::write(src.join("broken/#broken.moth"), "value = missing_name\n")
        .expect("should write diagnosed root");

    let validated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let built = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let builder = ProjectBuilder::new(Box::new(ValidationTrackingBuilder {
        validated: validated.clone(),
        built: built.clone(),
    }));

    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    assert!(
        result.is_err(),
        "diagnosed module should fail project compilation"
    );
    assert!(validated.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        !built.load(std::sync::atomic::Ordering::SeqCst),
        "backend must not receive a partial project compilation"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn write_project_outputs_writes_all_supported_artifacts_and_skips_not_built() {
    let root = temp_dir("writer_success");
    fs::create_dir_all(&root).expect("should create temp root");

    let project = Project {
        output_files: vec![
            OutputFile::new(PathBuf::from("assets"), FileKind::Directory),
            OutputFile::new(
                PathBuf::from("scripts/app.js"),
                FileKind::Js(String::from("console.log('hi');")),
            ),
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html></html>")),
            ),
            OutputFile::new(
                PathBuf::from("assets/logo.png"),
                FileKind::Bytes(vec![9, 8, 7, 6]),
            ),
            OutputFile::new(PathBuf::from("bin/app.wasm"), FileKind::Wasm(vec![0, 1, 2])),
            OutputFile::new(PathBuf::new(), FileKind::NotBuilt),
        ],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
    };

    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("writer should succeed");

    assert!(root.join("assets").is_dir());
    assert_eq!(
        fs::read_to_string(root.join("scripts/app.js")).expect("should read JS file"),
        "console.log('hi');"
    );
    assert_eq!(
        fs::read_to_string(root.join("index.html")).expect("should read HTML file"),
        "<html></html>"
    );
    assert_eq!(
        fs::read(root.join("assets/logo.png")).expect("should read binary file"),
        vec![9, 8, 7, 6]
    );
    assert_eq!(
        fs::read(root.join("bin/app.wasm")).expect("should read wasm file"),
        vec![0, 1, 2]
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn write_project_outputs_rejects_invalid_paths() {
    let root = temp_dir("writer_invalid");
    fs::create_dir_all(&root).expect("should create temp root");

    let invalid_projects = vec![
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("/tmp/absolute.js"),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
        },
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("../escape.js"),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
        },
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::new(),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
        },
    ];

    for project in invalid_projects {
        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        assert!(result.is_err(), "invalid output path should be rejected");
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn skip_unchanged_mode_preserves_existing_output_mtime() {
    let root = temp_dir("skip_unchanged_mtime");
    fs::create_dir_all(&root).expect("should create temp root");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>same</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let options = skip_unchanged_options(root.clone(), None);

    write_project_outputs(&project, &options).expect("first write should succeed");
    let first_modified = fs::metadata(root.join("index.html"))
        .expect("output file should exist")
        .modified()
        .expect("metadata should include modified time");

    thread::sleep(Duration::from_millis(30));
    write_project_outputs(&project, &options).expect("second write should succeed");
    let second_modified = fs::metadata(root.join("index.html"))
        .expect("output file should exist")
        .modified()
        .expect("metadata should include modified time");

    assert_eq!(first_modified, second_modified);
    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn skip_unchanged_mode_still_cleans_stale_manifest_tracked_outputs() {
    let root = temp_dir("skip_unchanged_cleanup");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let initial_project = html_project(
        vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("about/index.html"),
                FileKind::Html(String::from("<html>about</html>")),
            ),
        ],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &initial_project,
        &skip_unchanged_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("initial write should succeed");

    let index_modified = fs::metadata(output_root.join("index.html"))
        .expect("index should exist")
        .modified()
        .expect("metadata should include modified time");

    thread::sleep(Duration::from_millis(30));
    let follow_up_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &follow_up_project,
        &skip_unchanged_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("follow-up write should succeed");

    let updated_index_modified = fs::metadata(output_root.join("index.html"))
        .expect("index should still exist")
        .modified()
        .expect("metadata should include modified time");
    assert_eq!(index_modified, updated_index_modified);
    assert!(
        !output_root.join("about/index.html").exists(),
        "stale manifest-tracked output should still be removed in skip-unchanged mode"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_project_preserves_string_table_for_frontend_signature_diagnostics() {
    let root = temp_dir("frontend_signature_diagnostics");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(
        root.join("main.moth"),
        "use_missing |value Missing|:\n    return value\n;\n",
    )
    .expect("should write source file");

    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);
        let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
        let Err(messages) = build_project(&builder, "main.moth", &[]) else {
            panic!("build should fail with a frontend signature diagnostic");
        };
        let errors = messages.error_diagnostics().collect::<Vec<_>>();

        assert!(
            errors
                .iter()
                .any(|diagnostic| diagnostic.kind.descriptor().title == "Unknown type name"),
            "expected the named-type diagnostic to be preserved"
        );
        assert_eq!(
            resolve_source_file_path(&errors[0].primary_location.scope, &messages.string_table),
            normalize_path(
                &fs::canonicalize(root.join("main.moth")).expect("main file should canonicalize")
            )
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn config_validation_failure_returns_config_error_before_compilation() {
    let root = temp_dir("failing_validation");
    fs::create_dir_all(&root).expect("should create temp root");
    // Invalid frontend syntax to prove it fails BEFORE frontend compilation
    fs::write(root.join("main.moth"), "invalid syntax;;;;;").expect("should write source file");
    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let builder = ProjectBuilder::new(Box::new(FailingValidationBuilder));
        let result = build_project(&builder, "main.moth", &[]);

        let Err(messages) = result else {
            panic!("build_project should fail when config validation fails");
        };
        assert_has_config_error(&messages);
        assert!(
            rendered_error_messages(&messages)
                .iter()
                .any(|message| message.contains("fake_config_error")),
            "expected fake config validation message"
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn resolve_project_output_root_defaults_to_dev_and_release_for_directory_builds() {
    let root = temp_dir("output_defaults");
    let config = Config::new(root.clone());

    assert_eq!(resolve_project_output_root(&config, &[]), root.join("dev"));
    assert_eq!(
        resolve_project_output_root(&config, &[Flag::Release]),
        root.join("release")
    );
}

#[test]
fn resolve_project_output_root_respects_configured_dev_and_release_folders() {
    let root = temp_dir("output_overrides");
    let mut config = Config::new(root.clone());
    config.dev_folder = PathBuf::from("preview");
    config.release_folder = PathBuf::from("public");

    assert_eq!(
        resolve_project_output_root(&config, &[]),
        root.join("preview")
    );
    assert_eq!(
        resolve_project_output_root(&config, &[Flag::Release]),
        root.join("public")
    );
}

#[test]
fn resolve_project_output_root_uses_project_root_when_folder_is_explicitly_empty() {
    let root = temp_dir("output_root_fallback");
    let mut config = Config::new(root.clone());
    config.dev_folder = PathBuf::new();
    config.release_folder = PathBuf::new();

    assert_eq!(resolve_project_output_root(&config, &[]), root);
    assert_eq!(
        resolve_project_output_root(&config, &[Flag::Release]),
        config.entry_dir
    );
}

#[test]
fn build_directory_project_requires_artifact_root_in_configured_entry_root() {
    let root = temp_dir("missing_homepage");
    let src = root.join("src");
    fs::create_dir_all(src.join("about")).expect("should create about folder");

    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(src.join("about").join("#page.moth"), "#[:<h1>About</h1>]\n")
        .expect("should write about");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    assert!(result.is_err(), "missing root homepage should fail");
    let messages = result.err().expect("expected missing homepage error");
    assert_has_config_error(&messages);
    assert!(
        messages.first_infrastructure_error_for_tests().is_none(),
        "missing homepage should stay as a typed config diagnostic"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn build_project_routes_invalid_page_url_style_through_typed_config_diagnostic() {
    let root = temp_dir("invalid_page_url_style");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\npage_url_style #= \"slashy\"\n",
    )
    .expect("should write config");
    fs::write(src.join("#page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    let Err(messages) = result else {
        panic!("invalid page URL style should fail build");
    };
    assert_has_config_error(&messages);
    assert_invalid_project_setting(&messages, "page_url_style", "slashy");

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

use crate::compiler_tests::test_support::temp_dir;
