//! Tests for dev-server orchestration and entry-path validation.

use super::{DevServerOptions, resolve_dev_runtime_paths, validate_dev_entry_path};
use crate::build_system::build::{BackendBuilder, Project, ProjectBuilder};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidDependencyClauseReason,
};
#[cfg(unix)]
use crate::compiler_frontend::compiler_messages::{InvalidConfigReason, InvalidOutputFolderReason};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveHandlerSpec, StyleDirectiveSpec, TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use crate::compiler_tests::test_support::unused_temp_path;
use crate::projects::settings::{CONFIG_FILE_NAME, Config, ProjectConfigError};
use std::fs;

struct NoopBuilder;

impl BackendBuilder for NoopBuilder {
    fn build_backend(
        &self,
        _project_compilation: crate::build_system::build::ProjectCompilation,
        _config: &Config,
        _build_profile: crate::build_system::BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, crate::compiler_frontend::compiler_errors::CompilerMessages> {
        panic!("build_backend should not run in dev-server output-dir tests");
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Ok(())
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        Vec::new()
    }
}

struct ConflictingDirectiveBuilder;

impl BackendBuilder for ConflictingDirectiveBuilder {
    fn build_backend(
        &self,
        _project_compilation: crate::build_system::build::ProjectCompilation,
        _config: &Config,
        _build_profile: crate::build_system::BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, crate::compiler_frontend::compiler_errors::CompilerMessages> {
        panic!("build_backend should not run in dev-server output-dir tests");
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Ok(())
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        vec![StyleDirectiveSpec::handler(
            "md",
            TemplateBodyMode::Normal,
            TemplateHeadCompatibility::fully_compatible_meaningful(),
            StyleDirectiveHandlerSpec::no_op(),
        )]
    }
}

#[test]
fn defaults_match_dev_server_contract() {
    let defaults = DevServerOptions::default();
    assert_eq!(defaults.host, "127.0.0.1");
    assert_eq!(defaults.port, 6342);
    assert_eq!(defaults.poll_interval_ms, 300);
}

#[test]
fn entry_path_validation_accepts_moth_files() {
    let root = unused_temp_path("entry_file");
    fs::create_dir_all(&root).expect("should create temp root");
    let file = root.join("main.moth");
    fs::write(&file, "x = 1").expect("should write test file");

    let validated = validate_dev_entry_path(
        file.to_str()
            .expect("temp path should be valid utf-8 for this test"),
    )
    .expect("valid moth path should pass validation");

    assert!(validated.ends_with("main.moth"));
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}

#[test]
fn entry_path_validation_accepts_directories() {
    let root = unused_temp_path("entry_dir");
    fs::create_dir_all(&root).expect("should create temp root");
    let validated = validate_dev_entry_path(
        root.to_str()
            .expect("temp path should be valid utf-8 for this test"),
    )
    .expect("directories should be accepted");

    assert_eq!(
        validated,
        root.canonicalize().expect("temp dir should canonicalize")
    );
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}

#[test]
fn empty_entry_path_uses_current_directory() {
    let expected = std::env::current_dir()
        .expect("current directory should resolve")
        .canonicalize()
        .expect("current directory should canonicalize");
    let validated = validate_dev_entry_path("").expect("empty path should use current directory");
    assert_eq!(validated, expected);
}

#[test]
fn resolve_dev_runtime_paths_use_configured_dev_folder_for_directory_projects() {
    let root = unused_temp_path("configured_dev_folder");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join(CONFIG_FILE_NAME), "dev_folder #= \"preview\"\n")
        .expect("should write config");

    let builder = ProjectBuilder::new(Box::new(NoopBuilder));
    let resolved = resolve_dev_runtime_paths(&builder, &root, &[])
        .expect("directory output dir should resolve");

    assert_eq!(resolved.output_dir, root.join("preview"));
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}

#[cfg(unix)]
#[test]
fn resolve_dev_runtime_paths_rejects_symlinked_output_roots() {
    use std::os::unix::fs::symlink;

    for (case_name, target_name, expected_reason) in [
        (
            "sibling",
            "outside",
            InvalidOutputFolderReason::ResolvesOutsideProjectRoot,
        ),
        (
            "entry",
            "src",
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot,
        ),
    ] {
        let root = unused_temp_path(&format!("dev_runtime_output_symlink_{case_name}"));
        let source_root = root.join("src");
        let outside = unused_temp_path(&format!("dev_runtime_output_target_{case_name}"));
        fs::create_dir_all(&source_root).expect("should create source root");
        fs::create_dir_all(&outside).expect("should create outside root");
        let output_root = root.join("dev");
        if target_name == "src" {
            symlink(&source_root, &output_root).expect("should create entry-root symlink");
        } else {
            symlink(&outside, &output_root).expect("should create sibling symlink");
        }
        fs::write(
            root.join(CONFIG_FILE_NAME),
            "entry_root #= \"src\"\ndev_folder #= \"dev\"\noutput_folder #= \"release\"\n",
        )
        .expect("should write config");

        let builder = ProjectBuilder::new(Box::new(NoopBuilder));
        let messages = resolve_dev_runtime_paths(&builder, &root, &[])
            .expect_err("dev startup must reject symlinked output roots");
        assert!(messages.error_diagnostics().any(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::InvalidOutputFolder {
                        reason,
                        ..
                    },
                    ..
                } if *reason == expected_reason
            )
        }));

        fs::remove_dir_all(&root).expect("should remove project root");
        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

#[test]
fn resolve_dev_runtime_paths_rejects_empty_dev_folder() {
    let root = unused_temp_path("empty_dev_folder");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join(CONFIG_FILE_NAME), "dev_folder #= \"\"\n").expect("should write config");

    let builder = ProjectBuilder::new(Box::new(NoopBuilder));
    let result = resolve_dev_runtime_paths(&builder, &root, &[]);

    assert!(
        result.is_err(),
        "empty dev folder should be rejected at config validation"
    );
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}

#[test]
fn resolve_dev_runtime_paths_return_config_load_failures() {
    let root = unused_temp_path("bad_config");
    fs::create_dir_all(&root).expect("should create temp root");
    fs::write(root.join(CONFIG_FILE_NAME), "@core/math sin\n").expect("should write bad config");

    let builder = ProjectBuilder::new(Box::new(NoopBuilder));
    let messages = resolve_dev_runtime_paths(&builder, &root, &[])
        .expect_err("bad config should fail directory bootstrap");

    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    assert!(
        matches!(
            &diagnostics[0].payload,
            DiagnosticPayload::InvalidDependencyClause {
                reason: InvalidDependencyClauseReason::DependencyClauseNotAllowed,
                ..
            }
        ),
        "unexpected diagnostic payload: {:?}",
        diagnostics[0].payload
    );
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}

#[test]
fn resolve_dev_runtime_paths_return_style_directive_merge_failures() {
    let root = unused_temp_path("style_directive_conflict");
    fs::create_dir_all(&root).expect("should create temp root");

    let builder = ProjectBuilder::new(Box::new(ConflictingDirectiveBuilder));
    let messages = resolve_dev_runtime_paths(&builder, &root, &[])
        .expect_err("conflicting directives should fail bootstrap");

    assert_eq!(messages.error_count(), 1);
    let (_error_type, message, _location) = messages
        .first_infrastructure_error_for_tests()
        .expect("directive conflict should be wrapped for rendering");
    assert!(message.contains("cannot override") || message.contains("already exists"));
    fs::remove_dir_all(&root).expect("should clean up temp dir");
}
