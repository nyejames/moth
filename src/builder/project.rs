//! The public `ProjectBuilder` trait and the `build_project` driver that runs it.
//!
//! WHAT: `build_project` reads the builder's definition once, runs the compiler's shared build
//! orchestration on the request's project root, and then runs the builder's `build` step on the
//! compiled project. A private adapter presents the builder to that orchestration as a
//! `BackendBuilder`.
//! WHY: SDK builds reach the same frontend and config path as the CLI and dev server, so they
//! cannot drift from the built-in flow. The driver borrows the builder for one synchronous build,
//! so the public trait needs no `Send` or `Sync` bound.

use super::compiled_project::{BuildArtifacts, BuildContext, CompiledProject};
use super::{BuilderDefinition, Error};
use crate::build_system::BuildProfile;
use crate::build_system::build::{self, BackendBuilder, Project, ProjectCompilation};
use crate::build_system::output::{BuilderIdentity, CleanupPolicy};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, ProjectConfigError};
use std::cell::Cell;
use std::path::PathBuf;

/// The builder a project author implements to turn a compiled project into outputs.
///
/// `definition` runs once per build, before any source is compiled. `build` runs only after the
/// project compiles without diagnostics.
pub trait ProjectBuilder {
    /// Describes the builder's identity, source packages, host packages and config surface.
    fn definition(&self) -> Result<BuilderDefinition, Error>;

    /// Produces the build's outputs from one compiled project.
    fn build(
        &self,
        project: &CompiledProject,
        context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error>;
}

/// The inputs to one `build_project` call.
///
/// WHAT: names the project root to compile. Every build runs in the Dev profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    project_root: PathBuf,
}

impl BuildRequest {
    /// Creates a request for the project rooted at `project_root`.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }
}

/// The result of a successful `build_project` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltProject {
    warnings: Vec<String>,
}

impl BuiltProject {
    /// The compiler warnings for this build, one rendered terse line each.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Compiles the project at `request` and runs `builder` on the compiled project.
///
/// WHAT: `definition` runs first and its error returns unchanged. Source diagnostics return
/// `ErrorKind::Compilation` before `build` runs. A failure from `build` returns unchanged.
/// Infrastructure failures map to `CompilerBug` or `Infrastructure`.
/// WHY: SDK callers get one entry point and one error type, and the compile step is the same
/// orchestration the CLI uses, so the two paths cannot drift.
pub fn build_project(
    builder: &dyn ProjectBuilder,
    request: BuildRequest,
) -> Result<BuiltProject, Error> {
    let definition = builder.definition()?;

    let Some(project_root) = request.project_root.to_str() else {
        return Err(Error::infrastructure(format!(
            "The project root path '{}' is not valid UTF-8.",
            request.project_root.display()
        )));
    };

    let adapter = ProjectBuilderAdapter {
        builder,
        definition,
        builder_error: Cell::new(None),
    };
    let build_result =
        build::build_project(&adapter, project_root, &[], &BuildConfigInputSet::new());

    // WHY: `CompilerMessages` cannot carry the SDK error, so `build_backend` parks it in the
    // adapter and returns a placeholder failure. The parked error is checked first, so the
    // placeholder never reaches the caller.
    if let Some(builder_error) = adapter.builder_error.take() {
        return Err(builder_error);
    }

    let mut build_result = build_result
        .map_err(|compiler_messages| Error::from_compiler_messages(&compiler_messages))?;

    let warnings = match build_result.take_warning_messages() {
        None => Vec::new(),
        Some(warning_messages) if warning_messages.infrastructure_errors().is_some() => {
            return Err(Error::from_compiler_messages(&warning_messages));
        }
        Some(warning_messages) => format_terse_compiler_messages(&warning_messages),
    };

    Ok(BuiltProject { warnings })
}

/// Presents a `ProjectBuilder` to the compiler's shared build orchestration.
///
/// WHAT: answers the orchestration's identity, surface and config questions from the builder's
/// definition, and runs the builder's `build` step in the backend slot.
/// WHY: the orchestration only understands `BackendBuilder`, so the SDK reuses it instead of
/// duplicating the frontend and config flow.
struct ProjectBuilderAdapter<'builder> {
    builder: &'builder dyn ProjectBuilder,
    definition: BuilderDefinition,
    /// The builder's error, parked here because the orchestration carries only compiler messages.
    builder_error: Cell<Option<Error>>,
}

impl BackendBuilder for ProjectBuilderAdapter<'_> {
    fn builder_identity(&self) -> BuilderIdentity {
        self.definition.builder_identity()
    }

    fn build_backend(
        &self,
        mut project_compilation: ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        let resource_inputs = project_compilation.take_resource_inputs();
        let compiled_project = CompiledProject::new(project_compilation);
        let mut context = BuildContext::new();

        match self.builder.build(&compiled_project, &mut context) {
            // WHY: the driver does not write outputs yet, so the artifacts are dropped here.
            Ok(_artifacts) => Ok(Project {
                output_files: Vec::new(),
                entry_page_rel: None,
                cleanup_policy: CleanupPolicy::generic(std::iter::empty::<&str>()),
                warnings: Vec::new(),
                deferred_resources: Vec::new(),
                resource_inputs,
            }),
            Err(builder_error) => {
                self.builder_error.set(Some(builder_error));
                Err(CompilerMessages::from_error(
                    CompilerError::compiler_error("The project builder reported a failure."),
                    StringTable::new(),
                ))
            }
        }
    }

    fn frontend_surface(&self) -> BuilderSurface {
        self.definition.surface().clone()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        // WHY: SDK builders register no template style directives.
        Vec::new()
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        // WHY: SDK builders add no checks beyond the generic config schema validation that already
        // ran while loading config.
        Ok(())
    }
}
