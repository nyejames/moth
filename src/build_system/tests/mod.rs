//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, FileKind, ModuleRootActivity, OutputFile, Project,
};
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, OutputPlan, SingleFileOutputPlan, WriteMode,
    WriteOptions, write_project_outputs as write_project_outputs_with_table,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, InvalidConfigReason,
    NameNamespace, RuleDiagnosticKind,
};
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::{Config, ProjectConfigError};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

struct CurrentDirGuard {
    _lock: MutexGuard<'static, ()>,
    previous: PathBuf,
}

impl CurrentDirGuard {
    fn set_to(path: &PathBuf) -> Self {
        // Intentionally recover from a poisoned mutex. This lock only serializes cwd-mutating
        // tests — it does not protect shared semantic state. Recovering here prevents one
        // panicking test from cascading PoisonError into every subsequent cwd-mutating test.
        let lock = current_dir_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::current_dir().expect("current dir should resolve");
        std::env::set_current_dir(path).expect("should change current directory for test");
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.previous);
    }
}

fn current_dir_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn html_cleanup_policy() -> CleanupPolicy {
    CleanupPolicy::html()
}

fn generic_cleanup_policy() -> CleanupPolicy {
    CleanupPolicy::generic([".html", ".js", ".wasm"])
}

#[test]
fn module_root_activity_html_policy_requires_any_root_activity() {
    assert!(!ModuleRootActivity::default().has_html_artifact_activity());
    assert!(
        ModuleRootActivity {
            has_non_trivial_root_body: true,
            ..ModuleRootActivity::default()
        }
        .has_html_artifact_activity()
    );
    assert!(
        ModuleRootActivity {
            const_fragment_count: 1,
            ..ModuleRootActivity::default()
        }
        .has_html_artifact_activity()
    );
    assert!(
        ModuleRootActivity {
            runtime_fragment_count: 1,
            ..ModuleRootActivity::default()
        }
        .has_html_artifact_activity()
    );
}

fn write_project_outputs(
    project: &Project,
    options: &WriteOptions,
) -> Result<(), CompilerMessages> {
    write_project_outputs_with_table(project, options, &StringTable::default())
}

fn always_write_options(output_root: PathBuf, project_entry_dir: Option<PathBuf>) -> WriteOptions {
    WriteOptions {
        output_plan: test_output_plan(output_root, project_entry_dir, BuildProfile::Dev),
        write_mode: WriteMode::AlwaysWrite,
    }
}

fn always_write_options_for_profile(
    output_root: PathBuf,
    project_entry_dir: Option<PathBuf>,
    profile: BuildProfile,
) -> WriteOptions {
    WriteOptions {
        output_plan: test_output_plan(output_root, project_entry_dir, profile),
        write_mode: WriteMode::AlwaysWrite,
    }
}

fn skip_unchanged_options(
    output_root: PathBuf,
    project_entry_dir: Option<PathBuf>,
) -> WriteOptions {
    WriteOptions {
        output_plan: test_output_plan(output_root, project_entry_dir, BuildProfile::Dev),
        write_mode: WriteMode::SkipUnchanged,
    }
}

fn test_output_plan(
    output_root: PathBuf,
    project_root: Option<PathBuf>,
    profile: BuildProfile,
) -> OutputPlan {
    OutputPlan::SingleFile(SingleFileOutputPlan {
        output_root,
        project_root,
        owner: OutputOwner {
            builder: BuilderKind::Html,
            profile,
        },
        setting_location: SourceLocation::default(),
    })
}

fn html_project(output_files: Vec<OutputFile>, entry_page_rel: Option<PathBuf>) -> Project {
    Project {
        output_files,
        entry_page_rel,
        cleanup_policy: html_cleanup_policy(),
        warnings: vec![],
    }
}

fn unused_variable_warning(name: StringId, location: SourceLocation) -> CompilerDiagnostic {
    CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnusedVariable),
        DiagnosticSeverity::Warning,
        location,
        DiagnosticPayload::UnusedName { name },
    )
}

fn unknown_name_error(
    name: StringId,
    namespace: NameNamespace,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        location,
        DiagnosticPayload::UnknownName { name, namespace },
    )
}

struct WarningBuilder;

impl BackendBuilder for WarningBuilder {
    fn build_backend(
        &self,
        _project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        Ok(Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("generated.js"),
                FileKind::Js(String::from("console.log('ok');")),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic([".js"]),
            warnings: vec![unused_variable_warning(
                StringTable::new().get_or_intern("x".to_string()),
                SourceLocation::default(),
            )],
        })
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

struct ValidationTrackingBuilder {
    validated: std::sync::Arc<std::sync::atomic::AtomicBool>,
    built: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct EntryTrackingBuilder {
    module_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    entry_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl BackendBuilder for EntryTrackingBuilder {
    fn build_backend(
        &self,
        project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        self.module_count.store(
            project_compilation.modules().len(),
            std::sync::atomic::Ordering::SeqCst,
        );
        self.entry_count.store(
            project_compilation.entries().len(),
            std::sync::atomic::Ordering::SeqCst,
        );

        Ok(Project {
            output_files: vec![],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic(Vec::<&str>::new()),
            warnings: vec![],
        })
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

impl BackendBuilder for ValidationTrackingBuilder {
    fn build_backend(
        &self,
        _project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        self.built.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Project {
            output_files: vec![],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic(Vec::<&str>::new()),
            warnings: vec![],
        })
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        self.validated
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        Vec::new()
    }
}

struct FailingValidationBuilder;

impl BackendBuilder for FailingValidationBuilder {
    fn build_backend(
        &self,
        _project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        panic!("should not call build_backend if validation fails");
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Err(CompilerDiagnostic::invalid_config_reason(
            Some(string_table.intern("fake_config_error")),
            InvalidConfigReason::UnsupportedScalarValue,
            SourceLocation::default(),
        )
        .into())
    }

    fn frontend_surface(&self) -> BuilderSurface {
        BuilderSurface::with_mandatory_core()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        Vec::new()
    }
}

struct NoDirectiveBuilder;

impl BackendBuilder for NoDirectiveBuilder {
    fn build_backend(
        &self,
        _project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        Ok(Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::new()),
            )],
            entry_page_rel: Some(PathBuf::from("index.html")),
            cleanup_policy: CleanupPolicy::generic([".html"]),
            warnings: vec![],
        })
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

struct MultiModuleDiagnosticBuilder;

impl BackendBuilder for MultiModuleDiagnosticBuilder {
    fn build_backend(
        &self,
        project_compilation: super::ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        let homepage = project_compilation
            .modules()
            .iter()
            .find(|module| module.metadata.entry_point.ends_with("src/@page.moth"))
            .expect("directory build should discover homepage module");
        let docs_page = project_compilation
            .modules()
            .iter()
            .find(|module| module.metadata.entry_point.ends_with("src/docs/@page.moth"))
            .expect("directory build should discover docs module");

        let warning = unused_variable_warning(
            string_table.get_or_intern("x".to_string()),
            SourceLocation::from_path(&docs_page.metadata.entry_point, string_table),
        );
        let error = unknown_name_error(
            string_table.get_or_intern("homepage diagnostic".to_string()),
            NameNamespace::Value,
            SourceLocation::from_path(&homepage.metadata.entry_point, string_table),
        );

        Err(CompilerMessages::from_diagnostics(
            vec![error, warning],
            string_table.clone(),
        ))
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

mod build_cleanup_tests;
mod build_directive_tests;
mod build_import_tests;
mod build_infrastructure_tests;
mod build_orchestration_tests;
mod build_profile_tests;
mod module_lane_tests;
