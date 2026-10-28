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
use super::{BuildProfile, BuilderDefinition, Error};
use crate::build_system::backend_build_context::BackendBuildContext;
use crate::build_system::build::{
    self, BackendBuilder, BackendInvocationFailure, Project, ProjectCompilation,
};
use crate::build_system::output::{BuilderIdentity, CleanupPolicy};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_reports;
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::projects::settings::{Config, ProjectConfigError};
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
/// WHAT: names the project root and build profile. The profile defaults to Dev, and
/// [`BuildRequest::profile`] selects another one.
/// WHY: the request carries the selected profile through compilation and into the builder's
/// `build` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    project_root: PathBuf,
    profile: BuildProfile,
}

impl BuildRequest {
    /// Creates a request for the project rooted at `project_root`.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            profile: BuildProfile::Dev,
        }
    }

    /// Selects the build profile for this request.
    ///
    /// WHAT: replaces the default Dev profile with `profile`.
    /// WHY: the request carries one profile choice through compilation and the build step.
    pub fn profile(mut self, profile: BuildProfile) -> Self {
        self.profile = profile;
        self
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
    };
    let release_flags = [Flag::Release];
    let flags: &[Flag] = match request.profile {
        BuildProfile::Dev => &[],
        BuildProfile::Release => &release_flags,
    };
    let build_result =
        build::build_project(&adapter, project_root, flags, &BuildConfigInputSet::new())?;

    let build_result = match build_result {
        Ok(CompilationOutcome::Success { value, .. }) => value,
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return Err(Error::from_diagnostic_reports(&reports));
        }
        Err(errors) => {
            return Err(Error::from_infrastructure_errors(errors));
        }
    };

    let warnings = match build_result.finish_output(Ok(())) {
        Ok(CompilationOutcome::Success { warnings, .. }) => warnings
            .map(|reports| format_terse_diagnostic_reports(&reports))
            .unwrap_or_default(),
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return Err(Error::from_diagnostic_reports(&reports));
        }
        Err(errors) => return Err(Error::from_infrastructure_errors(errors)),
    };

    Ok(BuiltProject { warnings })
}

/// Presents a `ProjectBuilder` to the compiler's shared build orchestration.
///
/// The adapter borrows the public builder for one synchronous call and carries the exact
/// builder-originated failure through the build-client result instead of the compiler lanes.
struct ProjectBuilderAdapter<'builder> {
    builder: &'builder dyn ProjectBuilder,
    definition: BuilderDefinition,
}

impl BackendBuilder for ProjectBuilderAdapter<'_> {
    type Error = Error;

    fn builder_identity(&self) -> BuilderIdentity {
        self.definition.builder_identity()
    }

    fn build_backend(
        &self,
        mut project_compilation: ProjectCompilation,
        _config: &Config,
        build_profile: BuildProfile,
        _flags: &[Flag],
        _backend_context: &mut BackendBuildContext<'_>,
    ) -> Result<Project, BackendInvocationFailure<Self::Error>> {
        let resource_inputs = project_compilation.take_resource_inputs();
        let compiled_project = CompiledProject::new(project_compilation);
        let mut context = BuildContext::new(build_profile);

        match self.builder.build(&compiled_project, &mut context) {
            // The prototype discards artifacts; it does not publish them.
            Ok(_artifacts) => Ok(Project {
                output_files: Vec::new(),
                entry_page_rel: None,
                cleanup_policy: CleanupPolicy::generic(std::iter::empty::<&str>()),
                deferred_resources: Vec::new(),
                resource_inputs,
            }),
            Err(builder_error) => Err(BackendInvocationFailure::Builder(builder_error)),
        }
    }

    fn frontend_surface(&self) -> BuilderSurface {
        self.definition.surface().clone()
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        // SDK builders register no template style directives.
        Vec::new()
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<(), ProjectConfigError> {
        // Generic config-schema validation already ran while loading config.
        Ok(())
    }
}
