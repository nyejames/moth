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
use super::{BuildProfile, BuilderDefinition, Error, ErrorKind};
use crate::build_system::backend_build_context::BackendBuildContext;
use crate::build_system::build::{
    self, BackendBuilder, BackendInvocationFailure, BuildResult, OutputFailureClass, Project,
    ProjectCompilation, PublishedBuildOutput, WrittenBuildOutputs, publish_build_outputs,
};
use crate::build_system::output::{
    BuilderIdentity, CleanupPolicy, OutputPlan, ValidatedOutputPlan,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_reports;
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticDiscoveryStops};
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use crate::projects::settings::{Config, ProjectConfigError};
use std::fmt;
use std::path::{Path, PathBuf};

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
    ///
    /// `project_root` must name a project directory. The SDK accepts only project directories,
    /// because it publishes through the project's builder output root and reads its project
    /// configuration section. [`build_project`] rejects an existing non-directory root with
    /// [`ErrorKind::Build`] before it compiles any source.
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

/// The result of a successful `build_project` call, which owns the build until a terminal
/// operation consumes it.
///
/// Compiler warnings are finalised once by the terminal operation: [`Self::write_outputs`]
/// returns them with a successful publication, and [`Self::into_warnings`] returns them without
/// writing. A failed publication discards staged compiler warnings and returns the output
/// failure, matching the CLI lane. Dropping this value without calling a terminal operation
/// writes nothing.
pub struct BuiltProject {
    build_result: BuildResult,
    /// The directory output plan the SDK build always produces for a project directory.
    directory_output_plan: ValidatedOutputPlan,
    discovery_stops: DiagnosticDiscoveryStops,
}

impl BuiltProject {
    /// Publishes this build's artefacts through the compiler's central output writer and
    /// finalises the compiler warnings.
    ///
    /// WHAT: plans the output root from the builder section's validated output roots and this
    /// request's profile, writes the complete artefact batch once, then finalises the build's
    /// output result.
    /// WHY: SDK builders emit real files through the same ownership, destination-safety,
    /// manifest and stale-cleanup path as the CLI, so an embedding tool cannot drift from the
    /// compiler's output contract.
    ///
    /// On success the frozen compiler warnings are available from [`WrittenOutputs::warnings`].
    /// On failure the returned error carries the output failure and the staged compiler warnings
    /// are discarded, matching the CLI's canonical lane. Call [`Self::into_warnings`] instead
    /// when the host only needs to inspect the build. If warning finalisation itself fails after
    /// the writer emitted the batch, the error is a compilation diagnostic; the written files may
    /// already be on disk.
    ///
    /// Destination preflight and manifest-ownership preparation happen before the writer mutates
    /// the output root, so a rejection there leaves earlier artefacts and the manifest untouched.
    /// The writer then emits every artefact before stale cleanup and manifest persistence; stale
    /// cleanup therefore never runs early. A failure during emission can leave the artefacts
    /// already emitted in this batch on disk, and a failure persisting the manifest happens after
    /// stale removal and does not restore the previous manifest. A build with no artefacts still
    /// writes the manifest, so artefacts this builder published earlier are cleaned by the central
    /// policy.
    ///
    /// A write failure returns [`ErrorKind::Build`] for a user output diagnostic,
    /// [`ErrorKind::Infrastructure`] for a host failure or [`ErrorKind::CompilerBug`] for an
    /// internal invariant. This method consumes the build.
    pub fn write_outputs(self) -> Result<WrittenOutputs, Error> {
        let Self {
            build_result,
            directory_output_plan,
            discovery_stops,
        } = self;
        let publication = publish_build_outputs(build_result, |_| {
            Ok(OutputPlan::Directory(directory_output_plan))
        })
        .map_err(Error::from_infrastructure_errors)?;

        match publication {
            PublishedBuildOutput::Written { outputs, warnings } => {
                Ok(WrittenOutputs::from_build(outputs, warnings))
            }
            PublishedBuildOutput::Failed { class, reports } => {
                let kind = match class {
                    OutputFailureClass::Diagnosed => ErrorKind::Build,
                    OutputFailureClass::Infrastructure => ErrorKind::Infrastructure,
                    OutputFailureClass::CompilerBug => ErrorKind::CompilerBug,
                };
                Err(Error::from_output_reports(
                    kind,
                    "The build could not publish its outputs.",
                    &reports,
                ))
            }
            PublishedBuildOutput::FinalisationFailed { reports } => {
                Err(Error::from_diagnostic_reports(&reports, discovery_stops))
            }
        }
    }

    /// Finalises this build without writing any output and returns its compiler warnings.
    ///
    /// This is the terminal operation for a host that builds for inspection only. It freezes the
    /// same warning lane as a successful [`Self::write_outputs`] call, but it does not plan an
    /// output root, create a directory, write an artefact or touch a manifest.
    ///
    /// A warning-capacity or other diagnostic finalisation failure returns the rendered
    /// diagnostics as an `Error`; infrastructure failures map to
    /// [`ErrorKind::Infrastructure`].
    pub fn into_warnings(self) -> Result<Vec<String>, Error> {
        match self.build_result.finish_output(Ok(())) {
            Ok(CompilationOutcome::Success { warnings, .. }) => Ok(warnings
                .map(|reports| format_terse_diagnostic_reports(&reports))
                .unwrap_or_default()),
            Ok(CompilationOutcome::Diagnosed(reports)) => Err(Error::from_diagnostic_reports(
                &reports,
                self.discovery_stops,
            )),
            Err(errors) => Err(Error::from_infrastructure_errors(errors)),
        }
    }
}

impl fmt::Debug for BuiltProject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltProject")
            .finish_non_exhaustive()
    }
}

/// The files and compiler warnings one [`BuiltProject::write_outputs`] call produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenOutputs {
    output_root: PathBuf,
    written_paths: Vec<PathBuf>,
    warnings: Vec<String>,
    output_warnings: Vec<String>,
}

impl WrittenOutputs {
    fn from_build(outputs: WrittenBuildOutputs, warnings: Option<DiagnosticReportSet>) -> Self {
        Self {
            output_root: outputs.output_root,
            written_paths: outputs.written_paths,
            warnings: warnings
                .map(|reports| format_terse_diagnostic_reports(&reports))
                .unwrap_or_default(),
            output_warnings: outputs.output_warnings,
        }
    }

    /// The resolved output root the central writer used.
    ///
    /// For a directory project this is the builder section's development or release root,
    /// resolved under the project root.
    pub fn output_root(&self) -> &Path {
        &self.output_root
    }

    /// The output-root-relative paths this write created, in preparation order.
    pub fn written_paths(&self) -> &[PathBuf] {
        &self.written_paths
    }

    /// The compiler warnings finalised with this successful publication, one terse line each.
    ///
    /// Empty when the build produced no compiler warnings. A failed publication returns its
    /// error instead and discards these staged warnings.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Non-fatal output cleanup messages, one rendered line each.
    ///
    /// These are output-pipeline messages such as a recoverable manifest warning, not compiler
    /// warnings. Compiler warnings are on [`Self::warnings`].
    pub fn output_warnings(&self) -> &[String] {
        &self.output_warnings
    }
}

/// Compiles the project at `request` and runs `builder` on the compiled project.
///
/// WHAT: `definition` runs first and its error returns unchanged. An existing non-directory root
/// is a `Build` error before source compilation. Source diagnostics return
/// `ErrorKind::Compilation` before `build` runs. A failure from `build` returns unchanged.
/// Infrastructure failures map to `CompilerBug` or `Infrastructure`.
/// WHY: SDK callers get one entry point and one error type, and the compile step is the same
/// orchestration the CLI uses, so the two paths cannot drift.
///
/// This function writes no files. The returned [`BuiltProject`] publishes its artefacts through
/// [`BuiltProject::write_outputs`], finalises warnings without writing through
/// [`BuiltProject::into_warnings`], or leaves the output root untouched when dropped.
///
/// The request root must be a project directory. The frontend sends every existing non-directory
/// root down its synthetic single-file lane, but an SDK build publishes through the project's
/// builder output root and reads the project's `@config.moth`, so no such root can be a valid SDK
/// target. A missing root is not an existing non-directory root and keeps the shared
/// path-validation error.
pub fn build_project(
    builder: &dyn ProjectBuilder,
    request: BuildRequest,
) -> Result<BuiltProject, Error> {
    let definition = builder.definition()?;

    let Some(project_root) = request.project_root.to_str() else {
        return Err(Error::infrastructure(format!(
            "The project root path {:?} is not valid UTF-8.",
            request.project_root
        )));
    };

    // The frontend dispatch in `create_project_modules` sends every existing non-directory root
    // down its synthetic single-file lane. A project directory is the only SDK build root, so
    // reject any such root here rather than after compilation has produced a plan the SDK cannot
    // publish. A missing path is not an existing root and falls through to shared path validation.
    if request.project_root.exists() && !request.project_root.is_dir() {
        return Err(Error::build(format!(
            "The project root '{}' is not a project directory. Point `BuildRequest` at the \
             project directory instead.",
            portable_path_text(&request.project_root)
        )));
    }

    let adapter = ProjectBuilderAdapter {
        builder,
        definition,
    };
    let release_flags = [Flag::Release];
    let flags: &[Flag] = match request.profile {
        BuildProfile::Dev => &[],
        BuildProfile::Release => &release_flags,
    };
    let mut discovery_stops = DiagnosticDiscoveryStops::NONE;
    let build_result = build::build_project(
        &adapter,
        project_root,
        flags,
        &BuildConfigInputSet::new(),
        &mut discovery_stops,
    )?;

    let mut build_result = match build_result {
        Ok(CompilationOutcome::Success { value, .. }) => value,
        Ok(CompilationOutcome::Diagnosed(reports)) => {
            return Err(Error::from_diagnostic_reports(&reports, discovery_stops));
        }
        Err(errors) => {
            return Err(Error::from_infrastructure_errors(errors));
        }
    };

    // The request root is a directory, so the shared build produced a directory output plan.
    // Taking it here makes the SDK's publication path independent of the CLI's single-file plan.
    let Some(directory_output_plan) = build_result.directory_output_plan.take() else {
        return Err(Error::from_infrastructure_errors(
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                "The builder SDK compiled a project directory without a directory output plan.",
            )
            .into(),
        ));
    };

    Ok(BuiltProject {
        build_result,
        directory_output_plan,
        discovery_stops,
    })
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
        config: &Config,
        build_profile: BuildProfile,
        _flags: &[Flag],
        _backend_context: &mut BackendBuildContext<'_>,
    ) -> Result<Project, BackendInvocationFailure<Self::Error>> {
        let resource_inputs = project_compilation.take_resource_inputs();
        // A build carries at most one builder section, so this copies that section's validated
        // values rather than the project config.
        let compiled_project = CompiledProject::new(
            project_compilation,
            self.definition.config_section_name().map(str::to_owned),
            config.builder_settings.clone(),
        );
        let mut context = BuildContext::new(build_profile);

        match self.builder.build(&compiled_project, &mut context) {
            Ok(artifacts) => Ok(Project {
                output_files: artifacts.into_output_files(),
                entry_page_rel: None,
                // Recorded artefacts are `FileKind::Bytes`, which the central writer always
                // tracks, so the policy declares no extension-managed paths. The manifest still
                // records the builder identity, profile and emitted path list.
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

    fn validate_project_config(
        &self,
        _config: &Config,
        _strings: &mut DiagnosticStringBuilder<'_>,
    ) -> Result<(), ProjectConfigError> {
        // Generic config-schema validation already ran while loading config.
        Ok(())
    }
}
