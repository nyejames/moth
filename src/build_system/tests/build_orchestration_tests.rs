//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::BuildProfile;
use crate::build_system::build::{FileKind, OutputFile, Project, ProjectBuilder, build_project};
#[cfg(unix)]
use crate::build_system::output::ValidatedOutputPlan;
use crate::build_system::output::manifest::BUILD_MANIFEST_FILENAME;
use crate::build_system::output::{BuilderKind, OutputOwner, OutputPlan};
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
    let root = unused_temp_path("build_only");
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
    let root = unused_temp_path("warnings");
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
    let root = unused_temp_path("validation_tracking");
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
    let root = unused_temp_path("project_compilation_entries");
    let src = root.join("src");
    fs::create_dir_all(src.join("api")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
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
    )
    .expect("directory frontend and test backend should succeed");

    assert_eq!(module_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(entry_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn diagnosed_module_prevents_project_compilation_from_reaching_backend() {
    let root = unused_temp_path("diagnosed_project_compilation");
    let src = root.join("src");
    fs::create_dir_all(src.join("broken")).expect("should create module directories");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
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
    let root = unused_temp_path("writer_success");
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
    let root = unused_temp_path("writer_invalid");
    fs::create_dir_all(&root).expect("should create temp root");

    let invalid_projects = vec![
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("/var/absolute.js"),
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
        Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("line\nbreak.js"),
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
fn reserved_manifest_destination_is_rejected_before_emission() {
    let collision_root = unused_temp_path("manifest_destination_collision");
    fs::create_dir_all(&collision_root).expect("should create collision root");
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
    };
    assert!(
        write_project_outputs(
            &collision_project,
            &always_write_options(collision_root.clone(), None)
        )
        .is_err()
    );
    assert!(!collision_root.join("index.html").exists());
    assert!(!collision_root.join(".moth_manifest").exists());
    fs::remove_dir_all(&collision_root).expect("should remove collision root");

    for (case_index, reserved_descendant) in [
        PathBuf::from(".moth_manifest/child.js"),
        PathBuf::from(r".MOTH_MANIFEST\child.js"),
    ]
    .into_iter()
    .enumerate()
    {
        let descendant_root =
            unused_temp_path(&format!("manifest_destination_descendant_{case_index}"));
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
        };
        assert!(
            write_project_outputs(
                &descendant_project,
                &always_write_options(descendant_root.clone(), None)
            )
            .is_err()
        );
        assert!(!descendant_root.join("index.html").exists());
        assert!(!descendant_root.join(".moth_manifest").exists());
        fs::remove_dir_all(&descendant_root).expect("should remove descendant root");
    }

    let directory_root = unused_temp_path("manifest_destination_directory");
    fs::create_dir_all(directory_root.join(".moth_manifest"))
        .expect("should create manifest directory");
    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    assert!(
        write_project_outputs(
            &project,
            &always_write_options(directory_root.clone(), None)
        )
        .is_err()
    );
    assert!(!directory_root.join("index.html").exists());
    assert!(directory_root.join(".moth_manifest").is_dir());
    fs::remove_dir_all(&directory_root).expect("should remove manifest directory root");
}

#[cfg(unix)]
#[test]
fn manifest_symlink_destinations_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    for (case_name, target_kind) in ["inside", "outside", "dangling"]
        .into_iter()
        .map(|case_name| (case_name, case_name))
    {
        let root = unused_temp_path(&format!("manifest_symlink_{case_name}"));
        fs::create_dir_all(&root).expect("should create symlink test root");
        let outside = unused_temp_path(&format!("manifest_symlink_target_{case_name}"));
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
        assert!(
            write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err()
        );
        assert!(!root.join("index.html").exists());
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
        let root = unused_temp_path(&format!("output_alias_manifest_{case_name}"));
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
        };
        assert!(
            write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
            "case-variant manifest aliases must be rejected before emission: {case_name}"
        );
        assert!(!root.join("index.html").exists());
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
        let root = unused_temp_path(&format!("non_portable_canonical_alias_{case_name}"));
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
        };
        assert!(
            write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
            "non-portable canonical aliases must be rejected before emission: {case_name}"
        );
        assert!(!root.join("index.html").exists());
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

    let root = unused_temp_path("invalid_utf8_authored_output");
    fs::create_dir_all(&root).expect("should create output root");
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
    };

    assert!(
        write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
        "invalid UTF-8 output paths must be rejected before emission"
    );
    assert!(!root.join("index.html").exists());
    assert!(!root.join("safe-�-file.js").exists());
    assert!(!root.join(BUILD_MANIFEST_FILENAME).exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn canonical_case_collisions_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("canonical_case_collision");
    fs::create_dir_all(&root).expect("should create output root");
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
    };

    assert!(
        write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
        "canonical case-only aliases must be rejected before emission"
    );
    assert!(!root.join("index.html").exists());
    assert_eq!(
        fs::read(&lower_target).expect("lower target should remain unchanged"),
        lower_contents_before
    );
    assert_eq!(
        fs::read(&upper_target).expect("upper target should remain unchanged"),
        upper_contents_before
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
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
        let root = unused_temp_path(&format!("hard_link_output_{case_name}"));
        let outside = unused_temp_path(&format!("hard_link_target_{case_name}"));
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
        };

        assert!(
            write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
            "hard-linked destinations must be rejected before emission: {case_name}"
        );
        assert!(!root.join("index.html").exists());
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
                assert!(!manifest_path.exists());
            }
            _ => unreachable!("hard-link cases are fixed"),
        }

        fs::remove_dir_all(&root).expect("should remove output root");
        fs::remove_dir_all(&outside).expect("should remove outside root");
    }
}

#[test]
fn file_output_to_existing_directory_is_rejected_before_emission() {
    let root = unused_temp_path("file_output_existing_directory");
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
    };

    assert!(
        write_project_outputs(&project, &always_write_options(root.clone(), None)).is_err(),
        "file outputs must reject existing directories before emission"
    );
    assert!(!root.join("index.html").exists());
    assert!(root.join("occupied").is_dir());
    assert!(!root.join(BUILD_MANIFEST_FILENAME).exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn skip_unchanged_mode_preserves_existing_output_mtime() {
    let root = unused_temp_path("skip_unchanged_mtime");
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
    let root = unused_temp_path("skip_unchanged_cleanup");
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
    let root = unused_temp_path("frontend_signature_diagnostics");
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
    let root = unused_temp_path("failing_validation");
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
    config.dev_folder = PathBuf::from("preview");
    config.release_folder = PathBuf::from("public");
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
    let root = unused_temp_path("stage0_separator_normalized_output_skip");
    let normalized_dev_root = root.join("generated/site");
    fs::create_dir_all(&normalized_dev_root).expect("should create normalized output root");
    fs::write(
        root.join("config.moth"),
        r#"dev_folder #= "generated\\site"
output_folder #= "generated\\release"
"#,
    )
    .expect("should write config");
    fs::write(root.join("@page.moth"), "value = 1\n").expect("should write entry module");
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
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip the normalized output root instead of compiling stale output: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn directory_frontend_skips_symlink_ancestor_output_aliases() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("stage0_symlink_ancestor_output_alias_skip");
    let physical_output_root = root.join("generated/site");
    fs::create_dir_all(&physical_output_root).expect("should create physical output root");
    symlink(root.join("generated"), root.join("preview"))
        .expect("should create output-root symlink alias");
    fs::write(
        root.join("config.moth"),
        r#"dev_folder #= "generated\\site"
output_folder #= "generated\\release"
"#,
    )
    .expect("should write config");
    fs::write(root.join("@page.moth"), "value = 1\n").expect("should write entry module");
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
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip the output root reached through a symlink ancestor: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn directory_frontend_skips_symlink_aliases_to_output_descendants() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("stage0_symlink_output_descendant_skip");
    let physical_output_root = root.join("generated/site");
    let physical_output_descendant = physical_output_root.join("nested");
    fs::create_dir_all(&physical_output_descendant)
        .expect("should create physical output descendant");
    symlink(&physical_output_descendant, root.join("preview"))
        .expect("should create descendant output symlink alias");
    fs::write(
        root.join("config.moth"),
        r#"dev_folder #= "generated\\site"
output_folder #= "generated\\release"
"#,
    )
    .expect("should write config");
    fs::write(root.join("@page.moth"), "value = 1\n").expect("should write entry module");
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
    );

    assert!(
        result.is_ok(),
        "Stage 0 must skip symlink aliases that target output descendants: {:?}",
        result
            .err()
            .map(|messages| rendered_error_messages(&messages))
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn validated_output_settings_reject_canonical_root_aliases() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("output_canonical_aliases");
    let entry_root = root.join("src");
    let shared_root = root.join("shared-output");
    fs::create_dir_all(&entry_root).expect("should create entry root");
    fs::create_dir_all(&shared_root).expect("should create shared output root");
    symlink(&shared_root, root.join("dev-alias")).expect("should create dev output alias");
    symlink(&shared_root, root.join("release-alias")).expect("should create release output alias");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    config.dev_folder = PathBuf::from("dev-alias");
    config.release_folder = PathBuf::from("release-alias");
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

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn build_directory_project_requires_artifact_root_in_configured_entry_root() {
    let root = unused_temp_path("missing_homepage");
    let src = root.join("src");
    fs::create_dir_all(src.join("about")).expect("should create about folder");

    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(src.join("about").join("@page.moth"), "#[:<h1>About</h1>]\n")
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
    let root = unused_temp_path("invalid_page_url_style");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\noutput_folder #= \"release\"\npage_url_style #= \"slashy\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

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

// -------------------------
//  Output setting validation and preflight tests
// -------------------------

#[test]
fn duplicate_output_destination_causes_zero_files_written() {
    let root = unused_temp_path("duplicate_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(result.is_err(), "duplicate output path should be rejected");

    assert!(
        !root.join("index.html").exists(),
        "no files should be written when a duplicate destination is detected"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn windows_ambiguous_output_aliases_fail_before_emission() {
    let root = unused_temp_path("windows_ambiguous_output_alias");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "Windows-normalized output aliases must fail during preflight"
    );
    assert!(!root.join("page.js").exists());
    assert!(!root.join("page.js.").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn file_ancestor_conflict_causes_zero_files_written() {
    let root = unused_temp_path("file_ancestor_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(result.is_err(), "a file cannot contain a child output");
    assert!(
        !root.join("assets").exists(),
        "preflight must reject the batch before creating an ancestor"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn file_ancestor_conflict_uses_component_boundaries_before_emission() {
    let output_paths = ["assets", "assets-keep.js", "assets/chunk.js"];
    let input_orders = [[0, 1, 2], [1, 2, 0], [2, 0, 1]];

    for (case_index, order) in input_orders.into_iter().enumerate() {
        let root = unused_temp_path(&format!("file_ancestor_component_{case_index}"));
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
        };

        let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
        assert!(
            result.is_err(),
            "a file ancestor must be rejected regardless of lexical sibling ordering"
        );
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
fn explicit_directory_output_may_contain_child_files() {
    let root = unused_temp_path("directory_child_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("an explicit directory output should contain child files");
    assert!(root.join("assets").is_dir());
    assert_eq!(
        fs::read(root.join("assets/logo.png")).expect("child output should exist"),
        vec![1, 2, 3]
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn file_and_directory_same_destination_is_rejected_before_writing() {
    let root = unused_temp_path("file_directory_same_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "a file and directory cannot claim one destination"
    );
    assert!(!root.join("assets").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn case_only_output_collision_causes_zero_files_written() {
    let root = unused_temp_path("case_only_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "case-only output collisions must be rejected"
    );
    assert!(!root.join("Pages").exists());
    assert!(!root.join("pages").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(unix)]
#[test]
fn symlinked_output_ancestor_escape_causes_zero_files_written() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("symlink_output_escape");
    let outside = unused_temp_path("symlink_output_outside");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::create_dir_all(&outside).expect("should create outside root");
    symlink(&outside, root.join("link")).expect("should create output symlink");

    let project = Project {
        output_files: vec![OutputFile::new(
            PathBuf::from("link/escape.js"),
            FileKind::Js(String::from("console.log('escape');")),
        )],
        entry_page_rel: None,
        cleanup_policy: generic_cleanup_policy(),
        warnings: vec![],
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "symlink escapes must be rejected before writes"
    );
    assert!(!outside.join("escape.js").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
    fs::remove_dir_all(&outside).expect("should remove outside root");
}

#[cfg(unix)]
#[test]
fn symlink_alias_destinations_are_rejected_before_writing() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("symlink_alias_destinations");
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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "distinct relative paths that alias one canonical file must be rejected"
    );
    assert!(!real.join("app.js").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn nested_explicit_directory_outputs_may_contain_child_files() {
    let root = unused_temp_path("nested_directory_child_dest");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("nested explicit directories should contain child files");
    assert_eq!(
        fs::read(root.join("assets/scripts/pages/app.js")).expect("child output should exist"),
        b"console.log('app');"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn symlink_alias_file_ancestor_conflict_is_rejected_before_writing() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("symlink_alias_file_ancestor");
    fs::create_dir_all(&root).expect("should create output root");
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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(
        result.is_err(),
        "a symlinked path below a canonical file ancestor must fail preflight"
    );
    assert_eq!(
        fs::read(&real_file).expect("existing file should remain"),
        b"existing"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn dangling_symlink_aliases_are_rejected_before_emission() {
    use std::os::unix::fs::symlink;

    let root = unused_temp_path("dangling_symlink_alias_file");
    fs::create_dir_all(&root).expect("should create output root");
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
    };
    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(result.is_err());
    assert!(!root.join("real/app.js").exists());
    fs::remove_dir_all(&root).expect("should remove temp root");

    let root = unused_temp_path("dangling_symlink_alias_ancestor");
    fs::create_dir_all(&root).expect("should create output root");
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
    };
    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(result.is_err());
    assert!(!root.join("real").exists());
    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn directory_output_root_symlink_escape_causes_zero_files_written() {
    use std::os::unix::fs::symlink;

    for (case_name, target_name) in [("sibling", "outside"), ("entry", "src")] {
        let root = unused_temp_path(&format!("directory_output_root_symlink_{case_name}"));
        let outside = unused_temp_path(&format!("directory_output_root_target_{case_name}"));
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
        assert!(
            result.is_err(),
            "directory output roots must reject symlink targets outside their validated boundary"
        );
        assert!(!outside.join("index.html").exists());
        assert!(!entry_root.join("index.html").exists());
        assert!(
            fs::symlink_metadata(&output_root)
                .expect("output symlink should remain")
                .file_type()
                .is_symlink()
        );

        fs::remove_dir_all(&root).expect("should remove project root");
        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

#[test]
fn invalid_later_output_path_causes_zero_files_written() {
    let root = unused_temp_path("invalid_later_path");
    fs::create_dir_all(&root).expect("should create temp root");

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
    };

    let result = write_project_outputs(&project, &always_write_options(root.clone(), None));
    assert!(result.is_err(), "invalid later path should be rejected");

    assert!(
        !root.join("index.html").exists(),
        "preflight must reject the batch before any file is written"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn empty_directory_output_setting_is_rejected() {
    let root = unused_temp_path("empty_output_setting");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    let Err(messages) = result else {
        panic!("empty dev_folder should fail build");
    };
    assert_has_config_error(&messages);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn absolute_output_setting_is_rejected() {
    let root = unused_temp_path("absolute_output_setting");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"/absolute/path\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    let Err(messages) = result else {
        panic!("absolute dev_folder should fail build");
    };
    assert_has_config_error(&messages);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn output_folder_inside_entry_root_is_rejected() {
    let root = unused_temp_path("output_inside_entry_root");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"src\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    let Err(messages) = result else {
        panic!("output folder inside entry_root should fail build");
    };
    assert_has_config_error(&messages);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn identical_dev_and_release_folders_are_rejected() {
    let root = unused_temp_path("identical_dev_release");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"output\"\noutput_folder #= \"output\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
    );

    let Err(messages) = result else {
        panic!("identical dev and release folders should fail build");
    };
    assert_has_config_error(&messages);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn valid_distinct_output_folders_resolve_unchanged() {
    let root = unused_temp_path("valid_output_folders");
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source folder");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"dev\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "#[:<h1>Home</h1>]\n").expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let build_result = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
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

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn first_dev_and_release_builds_create_independent_owned_manifests() {
    let root = unused_temp_path("first_profile_manifests");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\ndev_folder #= \"dev\"\noutput_folder #= \"release\"\n",
    )
    .expect("should write config");
    fs::write(source_root.join("@page.moth"), "#[:<h1>Home</h1>]\n")
        .expect("should write home page");

    let builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let dev_build = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[],
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
    assert!(!root.join("release/.moth_manifest").exists());

    let release_build = build_project(
        &builder,
        root.to_str().expect("root path should be valid UTF-8"),
        &[Flag::Release],
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

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

use crate::compiler_tests::test_support::unused_temp_path;
