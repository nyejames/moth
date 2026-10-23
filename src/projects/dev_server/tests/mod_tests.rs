//! Tests for dev-server orchestration and entry-path validation.

use super::{DevServerOptions, resolve_dev_runtime_paths, validate_dev_entry_path};
use crate::build_system::build::{BackendBuilder, Project, ProjectBuilder};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::InvalidDependencyClauseView;
#[cfg(unix)]
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidConfigView, decode_invalid_config,
};
#[cfg(unix)]
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, InvalidConfigReason, InvalidOutputFolderReason,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticSeverity, InvalidDependencyClauseReason, PreparedFailure,
};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveHandlerSpec, StyleDirectiveSpec, TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
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
    ) -> Result<Project, PreparedFailure> {
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
    ) -> Result<Project, PreparedFailure> {
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
    assert!(
        defaults.inputs.is_empty(),
        "default dev options start with no explicit build inputs"
    );
}

#[test]
fn entry_path_validation_accepts_moth_files() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let file = root.join("main.moth");
    fs::write(&file, "x = 1").expect("should write test file");

    let validated = validate_dev_entry_path(
        file.to_str()
            .expect("temp path should be valid utf-8 for this test"),
    )
    .expect("valid moth path should pass validation");

    assert!(validated.ends_with("main.moth"));
}

#[test]
fn entry_path_validation_preserves_missing_file_cause() {
    let directory = tempfile::tempdir().expect("should create temp dir");
    let missing_path = directory.path().join("missing.moth");
    let error = validate_dev_entry_path(
        missing_path
            .to_str()
            .expect("temp path should be valid UTF-8"),
    )
    .expect_err("missing dev entry must fail at path validation");

    assert_eq!(error.error_type, ErrorType::File);
    assert_eq!(error.host_path.as_deref(), Some(missing_path.as_path()));
    assert_eq!(error.msg, "Path does not exist");
}

#[test]
fn entry_path_validation_accepts_directories() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let validated = validate_dev_entry_path(
        root.to_str()
            .expect("temp path should be valid utf-8 for this test"),
    )
    .expect("directories should be accepted");

    assert_eq!(
        validated,
        root.canonicalize().expect("temp dir should canonicalize")
    );
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
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(
        root.join(CONFIG_FILE_NAME),
        "project #= (\n    name = \"docs\",\n)\nhtml #= (\n    dev_output = \"preview\",\n)\n",
    )
    .expect("should write config");

    let builder = ProjectBuilder::new(HtmlProjectBuilder::new());
    let resolved = resolve_dev_runtime_paths(&builder, &root, &[], &BuildConfigInputSet::new())
        .expect("directory output dir should resolve");

    assert_eq!(resolved.output_dir, root.join("preview"));
}

#[cfg(unix)]
#[test]
fn resolve_dev_runtime_paths_rejects_symlinked_output_roots() {
    use std::os::unix::fs::symlink;

    for (_case_name, target_name, expected_reason) in [
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
        let _tmp_root = tempfile::tempdir().expect("should create temp dir");
        let root = _tmp_root.path().to_path_buf();
        let source_root = root.join("src");
        let _temp1 = tempfile::tempdir().expect("should create temp dir");
        let outside = _temp1.path().to_path_buf();
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
            "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
        )
        .expect("should write config");

        let builder = ProjectBuilder::new(NoopBuilder);
        let failure = resolve_dev_runtime_paths(&builder, &root, &[], &BuildConfigInputSet::new())
            .expect_err("dev startup must reject symlinked output roots");
        assert!(matches!(
            &failure,
            crate::build_system::build::BootstrapFailure::Prepared {
                failure: PreparedFailure::Diagnosed(_),
                ..
            }
        ));
        let boundary = failure.into_boundary_failure();
        assert_eq!(boundary.error_count(), 1);
        let errors = boundary
            .reports
            .iter_severity(DiagnosticSeverity::Error)
            .collect::<Vec<_>>();
        assert_eq!(errors.len(), 1);
        let diagnostic = errors[0].1;
        assert_eq!(diagnostic.category(), DiagnosticCategory::Config);
        let facts = diagnostic
            .family_view::<InvalidConfigView>()
            .expect("output validation should retain typed invalid-config facts");
        let (_, reason) = decode_invalid_config(&facts, diagnostic.string_ids())
            .expect("invalid-config facts should decode to the output-folder reason");
        assert!(matches!(
            reason,
            InvalidConfigReason::InvalidOutputFolder {
                reason,
                ..
            } if reason == expected_reason
        ));

        fs::remove_dir_all(&outside).expect("should remove target root");
    }
}

#[test]
fn resolve_dev_runtime_paths_rejects_empty_dev_folder() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(
        root.join(CONFIG_FILE_NAME),
        "html #= (\n    dev_output = \"\",\n)\n",
    )
    .expect("should write config");

    let builder = ProjectBuilder::new(HtmlProjectBuilder::new());
    let result = resolve_dev_runtime_paths(&builder, &root, &[], &BuildConfigInputSet::new());

    assert!(
        result.is_err(),
        "empty dev folder should be rejected at config validation"
    );
}

#[test]
fn resolve_dev_runtime_paths_return_config_load_failures() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join(CONFIG_FILE_NAME), "@core/math sin\n").expect("should write bad config");

    let builder = ProjectBuilder::new(NoopBuilder);
    let failure = resolve_dev_runtime_paths(&builder, &root, &[], &BuildConfigInputSet::new())
        .expect_err("bad config should fail directory bootstrap");
    let crate::build_system::build::BootstrapFailure::Boundary(failure) = failure else {
        panic!("header parse failures should retain their prepared boundary carrier");
    };
    let errors = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 1);
    let facts = errors[0]
        .1
        .family_view::<InvalidDependencyClauseView>()
        .expect("header parse failure should retain typed dependency-clause facts");
    assert!(
        facts.reason == InvalidDependencyClauseReason::DependencyClauseNotAllowed,
        "unexpected dependency clause reason: {:?}",
        facts.reason
    );
}

#[test]
fn resolve_dev_runtime_paths_return_style_directive_merge_failures() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let builder = ProjectBuilder::new(ConflictingDirectiveBuilder);
    let failure = resolve_dev_runtime_paths(&builder, &root, &[], &BuildConfigInputSet::new())
        .expect_err("conflicting directives should fail bootstrap");
    let boundary = failure.into_boundary_failure();
    assert_eq!(boundary.error_count(), 1);
    let error = boundary
        .infrastructure_errors()
        .and_then(|errors| errors.iter().next())
        .expect("directive conflict should remain an infrastructure failure");
    assert!(error.msg.contains("cannot override") || error.msg.contains("already exists"));
}
