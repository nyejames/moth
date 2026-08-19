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
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::projects::settings::{Config, ProjectConfigError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

#[cfg(test)]
type RestoreFn = Box<dyn Fn(&Path) -> Result<(), std::io::Error> + Send>;

struct CurrentDirGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<PathBuf>,
    /// Optional injection seam for testing restore failures. When set, this
    /// function is called instead of `std::env::set_current_dir`.
    #[cfg(test)]
    restore_override: Option<RestoreFn>,
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
            previous: Some(previous),
            #[cfg(test)]
            restore_override: None,
        }
    }

    /// Explicitly restore the previous directory, returning an error if restoration fails.
    ///
    /// WHAT: takes ownership of the restore responsibility so `Drop` will not retry it.
    /// WHY: without this, `Drop` would run after `finish` and attempt restoration again.
    ///   `Drop` cannot return errors, so the normal path must use `finish()` when the
    ///   caller cares about restore success.
    fn finish(mut self) -> Result<(), std::io::Error> {
        if let Some(previous) = self.previous.take() {
            restore_directory(&previous, &self.restore_override)?;
        }
        Ok(())
    }

    /// Set a custom restore function for testing restore-failure scenarios.
    #[cfg(test)]
    fn with_restore_override(mut self, f: RestoreFn) -> Self {
        self.restore_override = Some(f);
        self
    }
}

/// Restore the working directory, using the override if set (test seam).
#[cfg(test)]
fn restore_directory(path: &Path, override_fn: &Option<RestoreFn>) -> Result<(), std::io::Error> {
    if let Some(f) = override_fn {
        f(path)
    } else {
        std::env::set_current_dir(path)
    }
}

#[cfg(not(test))]
fn restore_directory(path: &Path, _override_fn: &Option<()>) -> Result<(), std::io::Error> {
    std::env::set_current_dir(path)
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        // If `finish()` already restored, `previous` is `None` and we do nothing.
        if let Some(previous) = self.previous.take() {
            let restore_result = restore_directory(&previous, &self.restore_override);
            if let Err(ref error) = restore_result
                && !std::thread::panicking()
            {
                panic!(
                    "CurrentDirGuard failed to restore directory to {:?}: {}",
                    previous, error
                );
            } else if let Err(ref error) = restore_result
                && std::thread::panicking()
            {
                eprintln!(
                    "CurrentDirGuard failed to restore directory to {:?}: {}",
                    previous, error
                );
            }
        }
    }
}

fn current_dir_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// One unique, normalized view of the artifacts a build produced.
///
/// WHAT: indexes every emitted `OutputFile` by its portable relative path and offers
///       cardinality-proving selectors.
/// WHY: `output_files.iter().find_map(..)` returns the first match, so a build that emitted
///      two glue modules, two HTML pages or a duplicate path would still satisfy an assertion
///      about "the" artifact. These selectors prove exactly-one before returning anything, and
///      `paths()` makes the whole emitted set assertable instead of merely non-empty.
struct BuiltOutputs<'a> {
    by_path: std::collections::BTreeMap<String, &'a OutputFile>,
}

impl<'a> BuiltOutputs<'a> {
    /// Index a project's outputs, failing on duplicate or non-UTF-8 paths.
    #[track_caller]
    fn index(project: &'a Project) -> Self {
        let mut by_path: std::collections::BTreeMap<String, &'a OutputFile> =
            std::collections::BTreeMap::new();

        for output in &project.output_files {
            if matches!(output.file_kind(), FileKind::NotBuilt) {
                continue;
            }

            let relative_path = output.relative_output_path();
            assert!(
                relative_path.to_str().is_some(),
                "artifact path {relative_path:?} is not valid UTF-8, so it cannot be compared \
                 with an expected path"
            );

            let normalized = portable_path_text(relative_path);
            assert!(
                by_path.insert(normalized.clone(), output).is_none(),
                "the build emitted more than one artifact at '{normalized}'"
            );
        }

        Self { by_path }
    }

    /// Every emitted artifact path, in portable sorted order.
    fn paths(&self) -> Vec<&str> {
        self.by_path.keys().map(String::as_str).collect()
    }

    /// The artifact at exactly this portable path.
    #[track_caller]
    fn at(&self, path: &str) -> &'a OutputFile {
        match self.by_path.get(path) {
            Some(output) => output,
            None => panic!(
                "expected an artifact at '{path}', but the build emitted {:?}",
                self.paths()
            ),
        }
    }

    /// The single artifact whose portable path satisfies `predicate`.
    ///
    /// Panics when zero or more than one artifact matches, so a test about "the glue module"
    /// cannot pass while a second glue module is also being emitted.
    #[track_caller]
    fn exactly_one(&self, description: &str, predicate: impl Fn(&str) -> bool) -> &'a OutputFile {
        let matches = self
            .by_path
            .iter()
            .filter(|(path, _)| predicate(path))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(_, output)] => output,
            [] => panic!(
                "expected exactly one {description}, but the build emitted {:?}",
                self.paths()
            ),
            several => panic!(
                "expected exactly one {description}, but {} matched: {:?}",
                several.len(),
                several.iter().map(|(path, _)| path).collect::<Vec<_>>()
            ),
        }
    }

    /// The portable path of the single artifact satisfying `predicate`.
    #[track_caller]
    fn exactly_one_path(&self, description: &str, predicate: impl Fn(&str) -> bool) -> &str {
        let matches = self
            .by_path
            .keys()
            .filter(|path| predicate(path))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [path] => path.as_str(),
            [] => panic!(
                "expected exactly one {description}, but the build emitted {:?}",
                self.paths()
            ),
            several => panic!(
                "expected exactly one {description}, but {} matched: {several:?}",
                several.len()
            ),
        }
    }

    /// Assert that no emitted artifact path satisfies `predicate`.
    #[track_caller]
    fn none_matching(&self, description: &str, predicate: impl Fn(&str) -> bool) {
        let matches = self
            .by_path
            .keys()
            .filter(|path| predicate(path))
            .collect::<Vec<_>>();
        assert!(
            matches.is_empty(),
            "expected no {description}, but found {matches:?}"
        );
    }
}

/// A stable name for an artifact kind, for failure messages.
fn file_kind_name(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::NotBuilt => "not-built",
        FileKind::Wasm(_) => "wasm",
        FileKind::Bytes(_) => "bytes",
        FileKind::Js(_) => "js",
        FileKind::Html(_) => "html",
        FileKind::Directory => "directory",
    }
}

/// The HTML text of an artifact, proving its kind.
#[track_caller]
fn html_text(output: &OutputFile) -> &str {
    match output.file_kind() {
        FileKind::Html(html) => html.as_str(),
        other => panic!(
            "expected an HTML artifact at {:?}, found a {} artifact",
            output.relative_output_path(),
            file_kind_name(other)
        ),
    }
}

/// The JavaScript text of an artifact, proving its kind.
#[track_caller]
fn js_text(output: &OutputFile) -> &str {
    match output.file_kind() {
        FileKind::Js(source) => source.as_str(),
        other => panic!(
            "expected a JS artifact at {:?}, found a {} artifact",
            output.relative_output_path(),
            file_kind_name(other)
        ),
    }
}

#[cfg(test)]
mod built_outputs_tests {
    use super::*;

    fn project_with(files: Vec<(&str, FileKind)>) -> Project {
        Project {
            output_files: files
                .into_iter()
                .map(|(path, kind)| OutputFile::new(PathBuf::from(path), kind))
                .collect(),
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn index_rejects_duplicate_artifact_paths() {
        let project = project_with(vec![
            ("index.html", FileKind::Html("first".to_owned())),
            ("index.html", FileKind::Html("second".to_owned())),
        ]);

        crate::compiler_tests::test_support::assert_panics_with(
            "more than one artifact at 'index.html'",
            || {
                BuiltOutputs::index(&project);
            },
        );
    }

    #[test]
    fn index_skips_not_built_entries() {
        let project = project_with(vec![
            ("index.html", FileKind::NotBuilt),
            ("page.js", FileKind::Js("// page".to_owned())),
        ]);

        assert_eq!(BuiltOutputs::index(&project).paths(), vec!["page.js"]);
    }

    #[test]
    fn exactly_one_rejects_multiple_matches() {
        let project = project_with(vec![
            ("_moth/js/glue/a.js", FileKind::Js("// a".to_owned())),
            ("_moth/js/glue/b.js", FileKind::Js("// b".to_owned())),
        ]);
        let outputs = BuiltOutputs::index(&project);

        crate::compiler_tests::test_support::assert_panics_with(
            "expected exactly one glue module, but 2 matched",
            || {
                outputs.exactly_one("glue module", |path| path.contains("_moth/js/glue/"));
            },
        );
    }

    #[test]
    fn exactly_one_rejects_no_match() {
        let project = project_with(vec![("index.html", FileKind::Html(String::new()))]);
        let outputs = BuiltOutputs::index(&project);

        crate::compiler_tests::test_support::assert_panics_with(
            "expected exactly one glue module",
            || {
                outputs.exactly_one("glue module", |path| path.contains("_moth/js/glue/"));
            },
        );
    }

    #[test]
    fn at_reports_the_emitted_set_when_a_path_is_missing() {
        let project = project_with(vec![("main.html", FileKind::Html(String::new()))]);
        let outputs = BuiltOutputs::index(&project);

        crate::compiler_tests::test_support::assert_panics_with(
            "expected an artifact at 'index.html'",
            || {
                outputs.at("index.html");
            },
        );
    }

    #[test]
    fn html_text_rejects_a_js_artifact() {
        let project = project_with(vec![("page.js", FileKind::Js("// page".to_owned()))]);
        let outputs = BuiltOutputs::index(&project);

        crate::compiler_tests::test_support::assert_panics_with(
            "expected an HTML artifact",
            || {
                html_text(outputs.at("page.js"));
            },
        );
    }
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
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        Ok(Project {
            output_files: vec![OutputFile::new(
                PathBuf::from("generated.js"),
                FileKind::Js(String::from("console.log('ok');")),
            )],
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic([".js"]),
            warnings: vec![unused_variable_warning(
                string_table.get_or_intern("x".to_string()),
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
            project_compilation.module_count(),
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
            .find(|module| module.metadata.entry_point.ends_with("src/@page.moth"))
            .expect("directory build should discover homepage module");
        let docs_page = project_compilation
            .modules()
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
mod build_dependency_tests;
mod build_directive_tests;
mod build_infrastructure_tests;
mod build_orchestration_tests;
mod build_profile_tests;
mod module_lane_tests;
