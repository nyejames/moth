//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    DeferredResourceOutput, FileKind, OutputFile, Project, ProjectBuilder, build_project,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceContentState;
#[cfg(unix)]
use crate::build_system::output::ValidatedOutputPlan;
use crate::build_system::output::manifest::BUILD_MANIFEST_FILENAME;
use crate::build_system::output::{
    BuilderKind, OutputDestinationOutcome, OutputOwner, OutputPlan, OutputWriteOutcome,
};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, resolve_source_file_path, terse,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticPayload, DiagnosticSeverity, InvalidConfigReason,
};
use crate::compiler_frontend::utilities::basic::normalize_path;
use crate::compiler_tests::test_diagnostics::{
    assert_no_infrastructure_errors, assert_output_rejection,
};
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};

fn rendered_error_messages(messages: &CompilerMessages) -> Vec<String> {
    messages
        .diagnostics()
        .enumerate()
        .filter(|(_, diagnostic)| diagnostic.severity == DiagnosticSeverity::Error)
        .map(|(diagnostic_index, diagnostic)| {
            terse::format_terse_diagnostic_with_context(
                diagnostic,
                messages.diagnostic_render_context(diagnostic_index),
            )
        })
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
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("main.moth");
    fs::write(&entry_file, "value = 1\n").expect("should write source file");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        entry_file
            .to_str()
            .expect("temp file path should be valid UTF-8 for this test"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("build should succeed");

    assert!(!result.project.output_files.is_empty());
    assert_path_missing(&root.join("index.html"));
}

#[test]
fn build_project_preserves_builder_warnings_in_build_result() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "value = 1\n").expect("should write source file");

    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let result = build_project(
            &ProjectBuilder::new(Box::new(WarningBuilder)),
            "main.moth",
            &[],
            &BuildConfigInputSet::new(),
        )
        .expect("build should succeed");

        assert!(
            result.warnings.len() == 1,
            "build result should include backend warnings"
        );
    }
}

#[test]
fn build_project_calls_validate_project_config() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "value = 1\n").expect("should write source file");
    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let validated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let built = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let builder = ProjectBuilder::new(Box::new(ValidationTrackingBuilder {
            validated: validated.clone(),
            built: built.clone(),
        }));

        build_project(&builder, "main.moth", &[], &BuildConfigInputSet::new())
            .expect("build should succeed");

        assert!(
            validated.load(std::sync::atomic::Ordering::SeqCst),
            "build_project should call validate_project_config"
        );
        assert!(
            built.load(std::sync::atomic::Ordering::SeqCst),
            "build_project should call build_backend"
        );
    }
}

#[test]
fn project_compilation_selects_only_modules_with_root_activity_as_entries() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("api")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write active root");
    fs::write(src.join("api/@api.moth"), "value #= 1\n").expect("should write API-only root");

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
        &BuildConfigInputSet::new(),
    )
    .expect("directory frontend and test backend should succeed");

    assert_eq!(module_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(entry_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    let _ = temp;
}

#[test]
fn diagnosed_module_prevents_project_compilation_from_reaching_backend() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("broken")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write valid root");
    fs::write(src.join("broken/@broken.moth"), "value = missing_name\n")
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
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("diagnosed module should fail project compilation");
    };
    assert_no_infrastructure_errors(&messages);
    assert!(validated.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        !built.load(std::sync::atomic::Ordering::SeqCst),
        "backend must not receive a partial project compilation"
    );

    let _ = temp;
}

#[test]
fn write_project_outputs_writes_all_supported_artifacts_and_skips_not_built() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let summary = write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("writer should succeed");

    // The reported destinations are the writer's own account of what it emitted, in preparation
    // order. The `NotBuilt` entry is deliberately absent rather than reported as an empty write.
    let emitted = |path: &str, outcome: OutputWriteOutcome| OutputDestinationOutcome {
        relative_path: PathBuf::from(path),
        outcome,
    };
    assert_eq!(
        summary.destinations(),
        &[
            emitted("assets", OutputWriteOutcome::DirectoryCreated),
            emitted("scripts/app.js", OutputWriteOutcome::Written),
            emitted("index.html", OutputWriteOutcome::Written),
            emitted("assets/logo.png", OutputWriteOutcome::Written),
            emitted("bin/app.wasm", OutputWriteOutcome::Written),
        ]
    );
    assert_eq!(summary.emitted_count(), 5);

    assert_directory(&root.join("assets"));
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
}

#[test]
fn write_project_outputs_rejects_invalid_paths() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let invalid_projects = vec![
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("/var/absolute.js"),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("../escape.js"),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::new(),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("line\nbreak.js"),
                FileKind::Js(String::from("x")),
            )],
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        },
    ];

    for project in invalid_projects {
        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("invalid output path should be rejected");
        };
        assert_output_rejection(&messages, "invalid-relative-output-path");
    }
}

#[test]
fn reserved_manifest_destination_is_rejected_before_emission() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let collision_root = _temp.path().to_path_buf();

    let collision_project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from(".moth_manifest"),
                FileKind::Js(String::from("not a manifest")),
            ),
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            ),
        ],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: html_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };
    let result = write_project_outputs(
        &collision_project,
        &always_write_options(collision_root.clone(), None),
    );
    let Err(messages) = result else {
        panic!("reserved manifest destination should be rejected");
    };
    assert_output_rejection(&messages, "reserved-manifest-destination");
    assert_path_missing(&collision_root.join("index.html"));
    assert_path_missing(&collision_root.join(".moth_manifest"));

    for reserved_descendant in [
        PathBuf::from(".moth_manifest/child.js"),
        PathBuf::from(r".MOTH_MANIFEST\child.js"),
    ]
    .into_iter()
    {
        let _descendant_temp = tempfile::tempdir().expect("should create descendant temp dir");
        let descendant_root = _descendant_temp.path().to_path_buf();
        fs::create_dir_all(&descendant_root).expect("should create descendant root");
        let descendant_project = Project {
            output_files: vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html>home</html>")),
                ),
                OutputFile::new(reserved_descendant, FileKind::Js(String::from("child"))),
            ],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: html_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };
        let result = write_project_outputs(
            &descendant_project,
            &always_write_options(descendant_root.clone(), None),
        );
        let Err(messages) = result else {
            panic!("reserved manifest descendant should be rejected");
        };
        assert_output_rejection(&messages, "reserved-manifest-destination");
        assert_path_missing(&descendant_root.join("index.html"));
        assert_path_missing(&descendant_root.join(".moth_manifest"));

        let _ = _descendant_temp;
    }

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let directory_root = _temp.path().to_path_buf();
    fs::create_dir_all(directory_root.join(".moth_manifest"))
        .expect("should create manifest directory");
    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let result = write_project_outputs(
        &project,
        &always_write_options(directory_root.clone(), None),
    );
    let Err(messages) = result else {
        panic!("reserved manifest directory should be rejected");
    };
    assert_output_rejection(&messages, "manifest-not-regular-file");
    assert_path_missing(&directory_root.join("index.html"));
    assert_directory(&directory_root.join(".moth_manifest"));
}

#[cfg(unix)]
#[test]
fn manifest_symlink_destinations_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    for (_case_name, target_kind) in ["inside", "outside", "dangling"]
        .into_iter()
        .map(|case_name| (case_name, case_name))
    {
        let _temp1 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp1.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create symlink test root");
        let _temp2 = tempfile::tempdir().expect("should create temp dir");
        let outside = _temp2.path().to_path_buf();
        if target_kind == "outside" {
            fs::create_dir_all(&outside).expect("should create outside symlink target root");
        }
        let target = match target_kind {
            "inside" => root.join("manifest_target"),
            "outside" => outside.join("manifest_target"),
            "dangling" => root.join("missing_manifest_target"),
            _ => unreachable!("test case names are fixed"),
        };
        if target_kind != "dangling" {
            fs::write(&target, "unchanged").expect("should create symlink target");
        }
        symlink(&target, root.join(".moth_manifest")).expect("should create manifest symlink");

        let project = html_project(
            vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            )],
            Some(PathBuf::from("index.html")),
        );
        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("manifest symlink destination should be rejected");
        };
        assert_output_rejection(&messages, "manifest-not-regular-file");
        assert_path_missing(&root.join("index.html"));
        assert!(
            fs::symlink_metadata(root.join(".moth_manifest"))
                .expect("manifest symlink should remain")
                .file_type()
                .is_symlink()
        );
        if target_kind != "dangling" {
            assert_eq!(
                fs::read(&target).expect("symlink target should remain"),
                b"unchanged"
            );
        }

        fs::remove_dir_all(&root).expect("should remove symlink test root");
        if target_kind == "outside" {
            fs::remove_dir_all(&outside).expect("should remove outside target root");
        }
    }
}

#[cfg(unix)]
#[test]
fn output_alias_to_manifest_destination_is_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    for (case_name, target_path, target_contents) in [
        (
            "exact_case_variant",
            PathBuf::from(".MOTH_MANIFEST"),
            String::from("existing case-variant manifest"),
        ),
        (
            "descendant_case_variant",
            PathBuf::from(".MOTH_MANIFEST/child.js"),
            String::from("existing case-variant child"),
        ),
        (
            "descendant_literal_backslash",
            PathBuf::from(r".MOTH_MANIFEST\child.js"),
            String::from("existing literal-backslash child"),
        ),
    ] {
        let _temp3 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp3.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create output root");
        let target = root.join(&target_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("should create case-variant target parent");
        }
        fs::write(&target, target_contents.as_bytes()).expect("should create case-variant target");
        symlink(&target, root.join("manifest_alias.js"))
            .expect("should create output alias to case-variant manifest path");

        let project = Project {
            output_files: vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html>home</html>")),
                ),
                OutputFile::new(
                    PathBuf::from("manifest_alias.js"),
                    FileKind::Js(String::from("not a manifest")),
                ),
            ],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: html_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };
        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("case-variant manifest aliases must be rejected before emission: {case_name}");
        };
        let expected_reason = match case_name {
            "exact_case_variant" | "descendant_case_variant" => {
                "reserved-manifest-destination-canonical"
            }
            "descendant_literal_backslash" => "non-lossless-canonical-path",
            _ => unreachable!("unexpected case: {case_name}"),
        };
        assert_output_rejection(&messages, expected_reason);
        assert_path_missing(&root.join("index.html"));
        assert_eq!(
            fs::read(&target).expect("case-variant target should remain unchanged"),
            target_contents.as_bytes()
        );
        assert!(
            fs::symlink_metadata(root.join("manifest_alias.js"))
                .expect("output alias should remain")
                .file_type()
                .is_symlink()
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[cfg(unix)]
#[test]
fn non_portable_canonical_aliases_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    for (case_name, target_path, unrelated_paths) in [
        (
            "literal_backslash",
            PathBuf::from(r"safe\literal.js"),
            vec![(
                PathBuf::from("safe/literal.js"),
                String::from("unrelated slash"),
            )],
        ),
        (
            "line_break",
            PathBuf::from("safe\nliteral.js"),
            vec![
                (
                    PathBuf::from("safe"),
                    String::from("unrelated first record"),
                ),
                (
                    PathBuf::from("literal.js"),
                    String::from("unrelated second record"),
                ),
            ],
        ),
    ] {
        let _temp4 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp4.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create output root");
        let target = root.join(&target_path);
        fs::write(&target, "target unchanged").expect("should create non-portable target");
        for (unrelated_path, contents) in &unrelated_paths {
            let absolute_path = root.join(unrelated_path);
            if let Some(parent) = absolute_path.parent() {
                fs::create_dir_all(parent).expect("should create unrelated parent");
            }
            fs::write(absolute_path, contents).expect("should create unrelated path");
        }
        symlink(&target, root.join("alias.js")).expect("should create non-portable output alias");

        let project = Project {
            output_files: vec![
                OutputFile::new(
                    PathBuf::from("index.html"),
                    FileKind::Html(String::from("<html>home</html>")),
                ),
                OutputFile::new(
                    PathBuf::from("alias.js"),
                    FileKind::Js(String::from("not portable")),
                ),
            ],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: html_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };
        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("non-portable canonical aliases must be rejected before emission: {case_name}");
        };
        assert_output_rejection(&messages, "non-lossless-canonical-path");
        assert_path_missing(&root.join("index.html"));
        assert_eq!(
            fs::read(&target).expect("non-portable target should remain unchanged"),
            b"target unchanged"
        );
        for (unrelated_path, contents) in &unrelated_paths {
            assert_eq!(
                fs::read(root.join(unrelated_path)).expect("unrelated path should remain"),
                contents.as_bytes()
            );
        }

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

#[cfg(unix)]
#[test]
fn invalid_utf8_authored_output_path_is_rejected_before_emission() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let invalid_path = PathBuf::from(OsString::from_vec(b"safe-\xFF-file.js".to_vec()));
    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            ),
            OutputFile::new(invalid_path, FileKind::Js(String::from("invalid"))),
        ],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: html_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("invalid UTF-8 output paths must be rejected before emission");
    };
    assert_output_rejection(&messages, "invalid-relative-output-path");
    assert_path_missing(&root.join("index.html"));
    assert_path_missing(&root.join("safe-�-file.js"));
    assert_path_missing(&root.join(BUILD_MANIFEST_FILENAME));
}

#[cfg(unix)]
#[test]
fn canonical_case_collisions_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let lower_target = root.join("pages");
    let upper_target = root.join("PAGES");
    fs::write(&lower_target, "lower unchanged").expect("should create lower target");
    fs::write(&upper_target, "upper unchanged").expect("should create upper target");
    let lower_contents_before = fs::read(&lower_target).expect("should read lower target");
    let upper_contents_before = fs::read(&upper_target).expect("should read upper target");
    symlink(&lower_target, root.join("lower_alias.js")).expect("should create lower alias");
    symlink(&upper_target, root.join("upper_alias.js")).expect("should create upper alias");

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("lower_alias.js"),
                FileKind::Js(String::from("lower output")),
            ),
            OutputFile::new(
                PathBuf::from("upper_alias.js"),
                FileKind::Js(String::from("upper output")),
            ),
        ],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: html_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("canonical case-only aliases must be rejected before emission");
    };
    assert_output_rejection(&messages, "canonical-destination-collision");
    assert_path_missing(&root.join("index.html"));
    assert_eq!(
        fs::read(&lower_target).expect("lower target should remain unchanged"),
        lower_contents_before
    );
    assert_eq!(
        fs::read(&upper_target).expect("upper target should remain unchanged"),
        upper_contents_before
    );
}

#[cfg(any(unix, windows))]
#[test]
fn hard_linked_outputs_are_rejected_before_emission() {
    use std::fs::hard_link;

    for case_name in [
        "output_to_manifest",
        "manifest_to_outside",
        "output_to_outside",
        "directory_to_outside",
    ] {
        let _temp5 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp5.path().to_path_buf();
        let _temp6 = tempfile::tempdir().expect("should create temp dir");
        let outside = _temp6.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create output root");
        fs::create_dir_all(&outside).expect("should create outside root");
        let manifest_path = root.join(".moth_manifest");
        let linked_output = root.join("linked.js");
        let outside_target = outside.join("outside.txt");

        match case_name {
            "output_to_manifest" => {
                fs::write(&manifest_path, "unchanged").expect("should create manifest");
                hard_link(&manifest_path, &linked_output)
                    .expect("should hard-link output to manifest");
            }
            "manifest_to_outside" => {
                fs::write(&outside_target, "unchanged").expect("should create outside target");
                hard_link(&outside_target, &manifest_path)
                    .expect("should hard-link manifest outside the output root");
            }
            "output_to_outside" | "directory_to_outside" => {
                fs::write(&outside_target, "unchanged").expect("should create outside target");
                hard_link(&outside_target, &linked_output)
                    .expect("should hard-link output outside the output root");
            }
            _ => unreachable!("hard-link cases are fixed"),
        }

        let mut output_files = vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>home</html>")),
        )];
        if case_name != "manifest_to_outside" {
            let linked_kind = if case_name == "directory_to_outside" {
                FileKind::Directory
            } else {
                FileKind::Js(String::from("not a manifest"))
            };
            output_files.push(OutputFile::new(PathBuf::from("linked.js"), linked_kind));
        }
        let project = Project {
            output_files,
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: html_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };

        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("hard-linked destinations must be rejected before emission: {case_name}");
        };
        let expected_reason = match case_name {
            "manifest_to_outside" => "manifest-hard-linked",
            "directory_to_outside" => "directory-destination-exists-as-non-directory",
            "output_to_manifest" | "output_to_outside" => "hard-linked-destination",
            _ => unreachable!("unexpected case: {case_name}"),
        };
        assert_output_rejection(&messages, expected_reason);
        assert_path_missing(&root.join("index.html"));
        match case_name {
            "output_to_manifest" => {
                assert_eq!(
                    fs::read(&manifest_path).expect("manifest should remain unchanged"),
                    b"unchanged"
                );
                assert_eq!(
                    fs::read(&linked_output).expect("hard-linked output should remain unchanged"),
                    b"unchanged"
                );
            }
            "manifest_to_outside" => {
                assert_eq!(
                    fs::read(&manifest_path).expect("manifest should remain unchanged"),
                    b"unchanged"
                );
                assert_eq!(
                    fs::read(&outside_target).expect("outside target should remain unchanged"),
                    b"unchanged"
                );
            }
            "output_to_outside" | "directory_to_outside" => {
                assert_eq!(
                    fs::read(&linked_output).expect("hard-linked output should remain unchanged"),
                    b"unchanged"
                );
                assert_eq!(
                    fs::read(&outside_target).expect("outside target should remain unchanged"),
                    b"unchanged"
                );
                assert_path_missing(&manifest_path);
            }
            _ => unreachable!("hard-link cases are fixed"),
        }

        fs::remove_dir_all(&root).expect("should remove output root");
        fs::remove_dir_all(&outside).expect("should remove outside root");
    }
}

#[test]
fn file_output_to_existing_directory_is_rejected_before_emission() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("occupied")).expect("should create existing directory");

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("occupied"),
                FileKind::Js(String::from("not a directory")),
            ),
        ],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: html_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("file outputs must reject existing directories before emission");
    };
    assert_output_rejection(&messages, "file-destination-exists-as-non-file");
    assert_path_missing(&root.join("index.html"));
    assert_directory(&root.join("occupied"));
    assert_path_missing(&root.join(BUILD_MANIFEST_FILENAME));
}

/// Skip-unchanged mode must leave an identical destination untouched.
///
/// The evidence is the writer's own outcome, not a modification time: filesystems with
/// second-granularity timestamps report the same value after a rewrite, so an mtime comparison
/// passes whether or not the file was skipped.
#[test]
fn skip_unchanged_mode_reports_an_identical_destination_as_skipped() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>same</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let options = skip_unchanged_options(root.clone(), None);

    let first = write_project_outputs(&project, &options).expect("first write should succeed");
    assert_eq!(
        first.outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::Written),
        "the first write has nothing to compare against and must emit the file"
    );

    let second = write_project_outputs(&project, &options).expect("second write should succeed");
    assert_eq!(
        second.outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::SkippedUnchanged),
        "identical content must be skipped rather than rewritten"
    );
    assert_eq!(
        second.emitted_count(),
        0,
        "an unchanged project emits no artifact; the manifest is cleanup's own concern"
    );
}

#[test]
fn skip_unchanged_mode_still_cleans_stale_manifest_tracked_outputs() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

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

    let follow_up_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let follow_up = write_project_outputs(
        &follow_up_project,
        &skip_unchanged_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("follow-up write should succeed");

    assert_eq!(
        follow_up.outcome_for(Path::new("index.html")),
        Some(OutputWriteOutcome::SkippedUnchanged),
        "the retained page keeps its existing content"
    );
    assert_path_missing(&output_root.join("about/index.html"));
}

#[test]
fn build_project_preserves_string_table_for_frontend_signature_diagnostics() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(
        root.join("main.moth"),
        "use_missing |value Missing|:\n    return value\n;\n",
    )
    .expect("should write source file");

    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);
        let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
        let Err(messages) = build_project(&builder, "main.moth", &[], &BuildConfigInputSet::new())
        else {
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
}

#[test]
fn config_validation_failure_returns_config_error_before_compilation() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    // Invalid frontend syntax to prove it fails BEFORE frontend compilation
    fs::write(root.join("main.moth"), "invalid syntax;;").expect("should write source file");
    {
        let _cwd_guard = CurrentDirGuard::set_to(&root);

        let builder = ProjectBuilder::new(Box::new(FailingValidationBuilder));
        let result = build_project(&builder, "main.moth", &[], &BuildConfigInputSet::new());

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
}

#[test]
fn validated_output_settings_select_default_profile_roots() {
    let root = unused_temp_path("output_defaults");
    let config = Config::new(root.clone());
    let mut string_table = StringTable::new();
    let settings = crate::build_system::project_config::validate_directory_output_settings(
        &config,
        &mut string_table,
    )
    .expect("default output folders should validate");

    assert_eq!(settings.dev.resolved_path, root.join("dev"));
    assert_eq!(settings.release.resolved_path, root.join("release"));

    // The validated settings are selected by the build profile without re-resolving paths.
    assert_eq!(
        settings
            .select(
                root.clone(),
                root.clone(),
                OutputOwner {
                    builder: BuilderKind::Html,
                    profile: BuildProfile::Dev,
                }
            )
            .output_root,
        root.join("dev")
    );
}

#[test]
fn validated_output_settings_preserve_configured_profile_roots() {
    let root = unused_temp_path("output_overrides");
    let mut config = Config::new(root.clone());
    config.html_section.dev_output = Some("preview".to_owned());
    config.html_section.release_output = Some("public".to_owned());
    let mut string_table = StringTable::new();
    let settings = crate::build_system::project_config::validate_directory_output_settings(
        &config,
        &mut string_table,
    )
    .expect("configured output folders should validate");

    assert_eq!(settings.dev.resolved_path, root.join("preview"));
    assert_eq!(settings.release.resolved_path, root.join("public"));
}

#[test]
fn directory_frontend_skips_separator_normalized_output_roots() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let normalized_dev_root = root.join("generated/site");
    fs::create_dir_all(&normalized_dev_root).expect("should create normalized output root");
    fs::write(
        root.join("config.moth"),
        r#"project #= |
    name = "docs",
    entry_root = "src",
|
html #= |
    dev_output = "generated\\site",
    release_output = "generated\\release",
|
"#,
    )
    .expect("should write config");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write entry module");
    fs::write(
        normalized_dev_root.join("@stale.moth"),
        "value = missing_stale_value\n",
    )
    .expect("should write source-looking stale output");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip the normalized output root instead of compiling stale output: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );
}

#[cfg(unix)]
#[test]
fn directory_frontend_skips_symlink_ancestor_output_aliases() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let physical_output_root = root.join("generated/site");
    fs::create_dir_all(&physical_output_root).expect("should create physical output root");
    symlink(root.join("generated"), root.join("preview"))
        .expect("should create output-root symlink alias");
    fs::write(
        root.join("config.moth"),
        r#"project #= |
    name = "docs",
    entry_root = "src",
|
html #= |
    dev_output = "generated\\site",
    release_output = "generated\\release",
|
"#,
    )
    .expect("should write config");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write entry module");
    fs::write(
        physical_output_root.join("@stale.moth"),
        "value = missing_stale_value\n",
    )
    .expect("should write source-looking stale output");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip the output root reached through a symlink ancestor: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );
}

#[cfg(unix)]
#[test]
fn directory_frontend_skips_symlink_aliases_to_output_descendants() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let physical_output_root = root.join("generated/site");
    let physical_output_descendant = physical_output_root.join("nested");
    fs::create_dir_all(&physical_output_descendant)
        .expect("should create physical output descendant");
    symlink(&physical_output_descendant, root.join("preview"))
        .expect("should create descendant output symlink alias");
    fs::write(
        root.join("config.moth"),
        r#"project #= |
    name = "docs",
    entry_root = "src",
|
html #= |
    dev_output = "generated\\site",
    release_output = "generated\\release",
|
"#,
    )
    .expect("should write config");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write entry module");
    fs::write(
        physical_output_descendant.join("@stale.moth"),
        "value = missing_stale_value\n",
    )
    .expect("should write source-looking stale output");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip symlink aliases that target output descendants: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );
}

#[cfg(unix)]
#[test]
fn validated_output_settings_reject_canonical_root_aliases() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let entry_root = root.join("src");
    let shared_root = root.join("shared-output");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::create_dir_all(&shared_root).expect("should create shared output root");
    symlink(&shared_root, root.join("dev-alias")).expect("should create dev output alias");
    symlink(&shared_root, root.join("release-alias")).expect("should create release output alias");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    config.html_section.dev_output = Some("dev-alias".to_owned());
    config.html_section.release_output = Some("release-alias".to_owned());
    let mut string_table = StringTable::new();
    let errors = crate::build_system::project_config::validate_directory_output_settings(
        &config,
        &mut string_table,
    )
    .expect_err("canonical output-root aliases must be rejected");

    let diagnostic = errors
        .iter()
        .find(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::OutputFoldersNotDistinct { .. },
                    ..
                }
            )
        })
        .expect("canonical alias should produce an output-folder collision diagnostic");
    let DiagnosticPayload::InvalidConfig {
        reason:
            InvalidConfigReason::OutputFoldersNotDistinct {
                dev_folder,
                release_folder,
            },
        ..
    } = &diagnostic.payload
    else {
        unreachable!("the diagnostic was matched above");
    };
    assert_eq!(string_table.resolve(*dev_folder), "dev-alias");
    assert_eq!(string_table.resolve(*release_folder), "release-alias");
    assert_eq!(
        resolve_source_file_path(&diagnostic.primary_location.scope, &string_table),
        root.join("config.moth")
    );
    let rendered = terse::format_terse_diagnostic_with_context(
        diagnostic,
        DiagnosticRenderContext::new(&string_table),
    );
    assert!(
        rendered.contains("resolve to the same output root and must be distinct"),
        "canonical-alias rejection should explain the physical output-root conflict: {rendered}"
    );
}

#[test]
fn build_directory_project_requires_artifact_root_in_configured_entry_root() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("about")).expect("should create about folder");

    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("about").join("@page.moth"), "#[:<h1>About</h1>]\n")
        .expect("should write about");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("missing root homepage should fail");
    };
    assert_has_config_error(&messages);
    assert!(
        messages.first_infrastructure_error_for_tests().is_none(),
        "missing homepage should stay as a typed config diagnostic"
    );
}

#[test]
fn build_project_routes_invalid_page_url_style_through_typed_config_diagnostic() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    page_url_style = \"slashy\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("invalid page URL style should fail build");
    };
    assert_has_config_error(&messages);
    assert_invalid_project_setting(&messages, "page_url_style", "slashy");
}

// -------------------------
//  Output setting validation and preflight tests
// -------------------------

#[test]
fn duplicate_output_destination_causes_zero_files_written() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>A</html>")),
            ),
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>B</html>")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("duplicate output path should be rejected");
    };
    assert_output_rejection(&messages, "duplicate-destination");

    assert_path_missing(&root.join("index.html"));
}

#[test]
fn windows_ambiguous_output_aliases_fail_before_emission() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("page.js"),
                FileKind::Js(String::from("console.log('first');")),
            ),
            OutputFile::new(
                PathBuf::from("page.js."),
                FileKind::Js(String::from("console.log('second');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("Windows-normalized output aliases must fail during preflight");
    };
    assert_output_rejection(&messages, "invalid-relative-output-path");
    assert_path_missing(&root.join("page.js"));
    assert_path_missing(&root.join("page.js."));
}

#[test]
fn file_ancestor_conflict_causes_zero_files_written() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("assets/app.js"),
                FileKind::Js(String::from("console.log('app');")),
            ),
            OutputFile::new(
                PathBuf::from("assets/app.js/chunk.js"),
                FileKind::Js(String::from("console.log('chunk');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("a file cannot contain a child output");
    };
    assert_output_rejection(&messages, "file-ancestor-conflict");
    assert_path_missing(&root.join("assets"));
}

#[test]
fn file_ancestor_conflict_uses_component_boundaries_before_emission() {
    let output_paths = ["assets", "assets-keep.js", "assets/chunk.js"];
    let input_orders = [[0, 1, 2], [1, 2, 0], [2, 0, 1]];

    for order in input_orders {
        let _temp7 = tempfile::tempdir().expect("should create temp dir");
        let root = _temp7.path().to_path_buf();
        fs::create_dir_all(&root).expect("should create temp root");
        let output_files = order
            .into_iter()
            .map(|index| {
                OutputFile::new(
                    PathBuf::from(output_paths[index]),
                    FileKind::Js(format!("console.log('{index}');")),
                )
            })
            .collect();
        let project = Project {
            output_files,
            entry_page_rel: None,
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };

        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        let Err(messages) = result else {
            panic!("a file ancestor must be rejected regardless of lexical sibling ordering");
        };
        assert_output_rejection(&messages, "file-ancestor-conflict");
        assert!(
            fs::read_dir(&root)
                .expect("output root should remain readable")
                .next()
                .is_none(),
            "component-aware preflight must reject the complete batch before emission"
        );

        fs::remove_dir_all(&root).expect("should remove temp dir");
    }
}

#[test]
fn deferred_resource_file_ancestor_conflict_stays_unhashed() {
    let _temp = tempfile::tempdir().expect("should create deferred resource fixture");
    let root = _temp.path().to_path_buf();
    let output_root = root.join("out");
    let source_path = root.join("resource.bin");
    fs::write(&source_path, [1_u8, 2, 3]).expect("should write deferred resource");

    let mut resource_inputs = ResourceInputRegistry::new();
    let source_id = resource_inputs.register_source(
        fs::canonicalize(&source_path).expect("deferred resource should canonicalize"),
    );
    let mut project = Project {
        output_files: vec![OutputFile::new(
            PathBuf::from("assets/app.js"),
            FileKind::Js(String::from("console.log('app');")),
        )],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: vec![DeferredResourceOutput {
            relative_output_path: PathBuf::from("assets/app.js/logo.bin"),
            source_id,
        }],
        resource_inputs,
    };

    let mut string_table = StringTable::new();
    let result = write_project_outputs_with_table(
        &mut project,
        &always_write_options(output_root.clone(), None),
        &mut string_table,
    );
    let Err(messages) = result else {
        panic!("a deferred resource below a file output must be rejected");
    };

    assert_output_rejection(&messages, "file-ancestor-conflict");
    assert_eq!(
        project.resource_inputs.records()[0].content(),
        ResourceContentState::Unhashed,
        "file-ancestor conflicts must occur before deferred resource IO"
    );
    assert_path_missing(&output_root);
}

#[cfg(unix)]
#[test]
fn deferred_resource_symlink_destination_conflict_stays_unhashed() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create deferred symlink fixture");
    let root = _temp.path().to_path_buf();
    let output_root = root.join("out");
    let real_directory = output_root.join("real");
    fs::create_dir_all(&real_directory).expect("should create real output directory");
    symlink(&real_directory, output_root.join("alias")).expect("should create output symlink");

    let source_path = root.join("resource.bin");
    fs::write(&source_path, [4_u8, 5, 6]).expect("should write deferred resource");
    let mut resource_inputs = ResourceInputRegistry::new();
    let source_id = resource_inputs.register_source(
        fs::canonicalize(&source_path).expect("deferred resource should canonicalize"),
    );
    let mut project = Project {
        output_files: vec![OutputFile::new(
            PathBuf::from("real/logo.bin"),
            FileKind::Js(String::from("console.log('builder');")),
        )],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: vec![DeferredResourceOutput {
            relative_output_path: PathBuf::from("alias/logo.bin"),
            source_id,
        }],
        resource_inputs,
    };

    let mut string_table = StringTable::new();
    let result = write_project_outputs_with_table(
        &mut project,
        &always_write_options(output_root.clone(), None),
        &mut string_table,
    );
    let Err(messages) = result else {
        panic!("canonical aliases must reject a deferred resource before emission");
    };

    assert_output_rejection(&messages, "canonical-destination-collision");
    assert_eq!(
        project.resource_inputs.records()[0].content(),
        ResourceContentState::Unhashed,
        "canonical destination conflicts must occur before deferred resource IO"
    );
    assert_path_missing(&real_directory.join("logo.bin"));
}

#[test]
fn explicit_directory_output_may_contain_child_files() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(PathBuf::from("assets"), FileKind::Directory),
            OutputFile::new(
                PathBuf::from("assets/logo.png"),
                FileKind::Bytes(vec![1, 2, 3]),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("an explicit directory output should contain child files");
    assert_directory(&root.join("assets"));
    assert_eq!(
        fs::read(root.join("assets/logo.png")).expect("child output should exist"),
        vec![1, 2, 3]
    );
}

#[test]
fn file_and_directory_same_destination_is_rejected_before_writing() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("assets"),
                FileKind::Js(String::from("console.log('file');")),
            ),
            OutputFile::new(PathBuf::from("assets"), FileKind::Directory),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("a file and directory cannot claim one destination");
    };
    assert_output_rejection(&messages, "duplicate-destination");
    assert_path_missing(&root.join("assets"));
}

#[test]
fn case_only_output_collision_causes_zero_files_written() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("Pages/index.html"),
                FileKind::Html(String::from("<p>one</p>")),
            ),
            OutputFile::new(
                PathBuf::from("pages/index.html"),
                FileKind::Html(String::from("<p>two</p>")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("case-only output collisions must be rejected");
    };
    assert_output_rejection(&messages, "duplicate-destination");
    assert_path_missing(&root.join("Pages"));
    assert_path_missing(&root.join("pages"));
}

#[cfg(unix)]
#[test]
fn symlinked_output_ancestor_escape_causes_zero_files_written() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let outside = _temp.path().to_path_buf();

    symlink(&outside, root.join("link")).expect("should create output symlink");

    let project = Project {
        output_files: vec![OutputFile::new(
            PathBuf::from("link/escape.js"),
            FileKind::Js(String::from("console.log('escape');")),
        )],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("symlink escapes must be rejected before writes");
    };
    assert_output_rejection(&messages, "escapes-output-root");
    assert_path_missing(&outside.join("escape.js"));
}

#[cfg(unix)]
#[test]
fn symlink_alias_destinations_are_rejected_before_writing() {
    use std::os::unix::fs::symlink;

    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let real = root.join("real");
    fs::create_dir_all(&real).expect("should create real output directory");
    symlink(&real, root.join("left")).expect("should create first alias");
    symlink(&real, root.join("right")).expect("should create second alias");

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("left/app.js"),
                FileKind::Js(String::from("console.log('left');")),
            ),
            OutputFile::new(
                PathBuf::from("right/app.js"),
                FileKind::Js(String::from("console.log('right');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("distinct relative paths that alias one canonical file must be rejected");
    };
    assert_output_rejection(&messages, "canonical-destination-collision");
    assert_path_missing(&real.join("app.js"));
}

#[test]
fn nested_explicit_directory_outputs_may_contain_child_files() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("assets/scripts/pages/app.js"),
                FileKind::Js(String::from("console.log('app');")),
            ),
            OutputFile::new(PathBuf::from("assets/scripts/pages"), FileKind::Directory),
            OutputFile::new(PathBuf::from("assets/scripts"), FileKind::Directory),
            OutputFile::new(PathBuf::from("assets"), FileKind::Directory),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("nested explicit directories should contain child files");
    assert_eq!(
        fs::read(root.join("assets/scripts/pages/app.js")).expect("child output should exist"),
        b"console.log('app');"
    );
}

#[cfg(unix)]
#[test]
fn symlink_alias_file_ancestor_conflict_is_rejected_before_writing() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let real_file = root.join("real");
    fs::write(&real_file, "existing").expect("should create existing file ancestor");
    symlink(&real_file, root.join("alias")).expect("should create file alias");

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("real"),
                FileKind::Js(String::from("console.log('file');")),
            ),
            OutputFile::new(
                PathBuf::from("alias/chunk.js"),
                FileKind::Js(String::from("console.log('chunk');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("a symlinked path below a canonical file ancestor must fail preflight");
    };
    assert_output_rejection(&messages, "dangling-symlink-in-destination");
    assert_eq!(
        fs::read(&real_file).expect("existing file should remain"),
        b"existing"
    );
}

#[cfg(unix)]
#[test]
fn dangling_symlink_aliases_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    symlink(root.join("real"), root.join("alias")).expect("should create dangling alias");
    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("real/app.js"),
                FileKind::Js(String::from("console.log('real');")),
            ),
            OutputFile::new(
                PathBuf::from("alias/app.js"),
                FileKind::Js(String::from("console.log('alias');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };
    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("dangling symlink should be rejected");
    };
    assert_output_rejection(&messages, "dangling-symlink-in-destination");
    assert_path_missing(&root.join("real/app.js"));

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    symlink(root.join("real"), root.join("alias")).expect("should create dangling alias");
    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("real"),
                FileKind::Js(String::from("console.log('file');")),
            ),
            OutputFile::new(
                PathBuf::from("alias/chunk.js"),
                FileKind::Js(String::from("console.log('chunk');")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };
    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("dangling symlink directory should be rejected");
    };
    assert_output_rejection(&messages, "dangling-symlink-in-destination");
    assert_path_missing(&root.join("real"));
}

#[cfg(unix)]
#[test]
fn directory_output_root_symlink_escape_causes_zero_files_written() {
    use std::os::unix::fs::symlink;

    for (_case_name, target_name) in [("sibling", "outside"), ("entry", "src")] {
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let _temp8 = tempfile::tempdir().expect("should create temp dir");
        let outside = _temp8.path().to_path_buf();
        let entry_root = root.join("src");
        let output_root = root.join("dev");
        fs::create_dir_all(&entry_root).expect("should create entry root");
        fs::create_dir_all(&outside).expect("should create outside root");
        if target_name == "src" {
            symlink(&entry_root, &output_root).expect("should create entry-root symlink");
        } else {
            symlink(&outside, &output_root).expect("should create sibling symlink");
        }

        let owner = OutputOwner {
            builder: BuilderKind::Html,
            profile: BuildProfile::Dev,
        };
        let project = Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>symlink</html>")),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: generic_cleanup_policy(),
            warnings: vec![],
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        };
        let options = WriteOptions {
            output_plan: OutputPlan::Directory(ValidatedOutputPlan {
                output_root: output_root.clone(),
                project_root: root.clone(),
                entry_root: entry_root.clone(),
                owner,
                setting_location: SourceLocation::default(),
            }),
            write_mode: WriteMode::AlwaysWrite,
        };

        let result = write_project_outputs(&project, &options);
        let Err(messages) = result else {
            panic!(
                "directory output roots must reject symlink targets outside their validated boundary"
            );
        };
        assert_output_rejection(&messages, "output-root-not-inside-project");
        assert_path_missing(&outside.join("index.html"));
        assert_path_missing(&entry_root.join("index.html"));
        assert!(
            fs::symlink_metadata(&output_root)
                .expect("output symlink should remain")
                .file_type()
                .is_symlink()
        );

        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

#[test]
fn invalid_later_output_path_causes_zero_files_written() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let project = Project {
        output_files: vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>Home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("../escape.js"),
                FileKind::Js(String::from("x")),
            ),
        ],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
        deferred_resources: Vec::new(),
        resource_inputs: ResourceInputRegistry::new(),
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    let Err(messages) = result else {
        panic!("invalid later path should be rejected");
    };
    assert_output_rejection(&messages, "invalid-relative-output-path");

    assert_path_missing(&root.join("index.html"));
}

#[test]
fn empty_directory_output_setting_is_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("empty dev_folder should fail build");
    };
    assert_has_config_error(&messages);
}

#[test]
fn absolute_output_setting_is_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"/absolute/path\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("absolute dev_folder should fail build");
    };
    assert_has_config_error(&messages);
}

#[test]
fn output_folder_inside_entry_root_is_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"src\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("output folder inside entry_root should fail build");
    };
    assert_has_config_error(&messages);
}

#[test]
fn identical_dev_and_release_folders_are_rejected() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"output\",\n    release_output = \"output\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    );

    let Err(messages) = result else {
        panic!("identical dev and release folders should fail build");
    };
    assert_has_config_error(&messages);
}

#[test]
fn valid_distinct_output_folders_resolve_unchanged() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"dev\",\n    release_output = \"release\",\n|\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let build_result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("valid distinct folders should build");

    assert_eq!(
        build_result
            .directory_output_plan
            .as_ref()
            .expect("directory build should carry an output plan")
            .output_root,
        root.join("dev")
    );
}

#[test]
fn first_dev_and_release_builds_create_independent_owned_manifests() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= |\n    dev_output = \"dev\",\n    release_output = \"release\",\n|\n",
    )
    .expect("should write config");
    fs::write(source_root.join("@page.moth"), "#[:<h1>Home</h1>]\n")
        .expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let dev_build = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("first dev build should compile");
    let dev_plan = dev_build
        .directory_output_plan
        .clone()
        .expect("directory build should carry its output plan");
    write_project_outputs(
        &dev_build.project,
        &WriteOptions {
            output_plan: OutputPlan::Directory(dev_plan),
            write_mode: WriteMode::AlwaysWrite,
        },
    )
    .expect("first dev build should write its manifest");

    let dev_manifest = fs::read_to_string(root.join("dev/.moth_manifest"))
        .expect("dev manifest should exist after the first build");
    assert!(dev_manifest.contains("# profile: dev"));
    assert_path_missing(&root.join("release/.moth_manifest"));

    let release_build = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[Flag::Release],
        &BuildConfigInputSet::new(),
    )
    .expect("first release build should compile");
    let release_plan = release_build
        .directory_output_plan
        .clone()
        .expect("release build should carry its output plan");
    write_project_outputs(
        &release_build.project,
        &WriteOptions {
            output_plan: OutputPlan::Directory(release_plan),
            write_mode: WriteMode::AlwaysWrite,
        },
    )
    .expect("first release build should write its manifest");

    let release_manifest = fs::read_to_string(root.join("release/.moth_manifest"))
        .expect("release manifest should exist after the first build");
    assert!(release_manifest.contains("# profile: release"));
}

use crate::compiler_tests::test_fs::{assert_directory, assert_path_missing};
use crate::compiler_tests::test_support::unused_temp_path;
