//! Per-build values exchanged between the public `ProjectBuilder` trait and the compiler.
//!
//! WHAT: `CompiledProject` wraps one successful frontend compilation for a single `build` call and
//! hands out an owner-bound `ProjectPackage` for its facade plus typed reads of the builder's
//! configuration section. `BuildContext` carries per-build state into `build` and answers the
//! native Wasm request, and `BuildArtifacts` carries the build's outputs back out.
//! WHY: builders receive only these typed handles, so compiler-private compilation, HIR and
//! string-table types never appear in the public SDK surface.

use super::{BuildProfile, Error};
use crate::build_system::build::{FileKind, OutputFile, ProjectCompilation};
use crate::build_system::output::manifest::BUILD_MANIFEST_FILENAME;
use crate::build_system::output::{
    component_identity_prefixes, parse_relative_path, path_starts_with_component_identity,
};
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use crate::projects::settings::BuilderSettings;
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

/// The builder-facing sentence for a build with no project package facade.
///
/// `require_package` and host requirement production both name the same missing file, so one
/// sentence keeps the two paths from drifting.
pub(super) const NO_PACKAGE_FACADE_MESSAGE: &str =
    "This project has no package facade. Add an `@package.moth` file at the project root.";

/// One successfully compiled project, borrowed for a single `ProjectBuilder::build` call.
pub struct CompiledProject {
    pub(super) compilation: ProjectCompilation,
    /// The builder's declared section name, absent when the definition declares no section.
    config_section: Option<String>,
    /// The values the project's config validation stored for its builder sections.
    builder_settings: BuilderSettings,
}

impl CompiledProject {
    pub(crate) fn new(
        compilation: ProjectCompilation,
        config_section: Option<String>,
        builder_settings: BuilderSettings,
    ) -> Self {
        Self {
            compilation,
            config_section,
            builder_settings,
        }
    }

    /// Returns the handle for the project's package facade.
    ///
    /// Fails with `ErrorKind::Build` when the project has no `@package.moth` facade at the project
    /// root.
    pub fn require_package(&self) -> Result<ProjectPackage<'_>, Error> {
        if self.compilation.package_assembly().is_none() {
            return Err(Error::build(NO_PACKAGE_FACADE_MESSAGE));
        }

        Ok(ProjectPackage {
            _compiled_project: PhantomData,
        })
    }

    /// Returns typed reads of the builder's declared configuration section.
    ///
    /// A read returns the project's authored value, or the default the builder declared for a
    /// field the project omitted. A definition without a section, an undeclared field, a field
    /// declared with a different type, or an omitted field with no default reads `None`. The
    /// automatic `dev_output` and `release_output` roots stay with output resolution and are not
    /// readable here.
    pub fn settings(&self) -> Settings<'_> {
        Settings {
            section: self.config_section.as_deref(),
            values: &self.builder_settings,
        }
    }
}

impl fmt::Debug for CompiledProject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledProject")
            .finish_non_exhaustive()
    }
}

/// Typed reads of one builder's configuration section for the current `build` call.
///
/// WHAT: resolves a field the builder declared through `BuilderDefinitionDraft::config` to the
/// value the project authored or the default the builder declared.
/// WHY: a builder reads its own section only. The automatic output roots have their own owner, so
/// a view over the declared fields needs no general config surface.
pub struct Settings<'project> {
    section: Option<&'project str>,
    values: &'project BuilderSettings,
}

impl Settings<'_> {
    /// Returns the value of the string field `name`.
    ///
    /// The value is the project's authored text or the default the builder declared. `None` when
    /// the builder declared no string field named `name`, including a field declared with another
    /// type, or the project omitted the field and it has no default.
    pub fn string(&self, name: &str) -> Option<&str> {
        self.values.string(self.section?, name)
    }

    /// Returns the value of the bool field `name`.
    ///
    /// The value is the project's authored flag or the default the builder declared. `None` when
    /// the builder declared no bool field named `name`, including a field declared with another
    /// type, or the project omitted the field and it has no default.
    pub fn bool(&self, name: &str) -> Option<bool> {
        self.values.bool(self.section?, name)
    }
}

impl fmt::Debug for Settings<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Settings")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

/// Owner-bound handle to the project's package facade.
///
/// WHY: the lifetime ties the handle to the `CompiledProject` that owns the facade's assembly.
/// The handle has no public methods yet; native lowering will carry that assembly through it.
#[derive(Debug)]
pub struct ProjectPackage<'project> {
    _compiled_project: PhantomData<&'project CompiledProject>,
}

/// Per-build state passed to `ProjectBuilder::build`.
///
/// WHAT: carries the selected build profile into the builder's `build` step.
/// WHY: builders can apply their own profile policy without re-deriving the build system's choice.
#[derive(Debug)]
pub struct BuildContext {
    profile: BuildProfile,
}

impl BuildContext {
    pub(crate) fn new(profile: BuildProfile) -> Self {
        Self { profile }
    }

    /// Returns the profile selected for this build.
    pub fn profile(&self) -> BuildProfile {
        self.profile
    }

    /// Requests the native WebAssembly module for the project's `package`.
    ///
    /// WHAT: the SDK's native lowering entry point. A successful module converts into the bytes
    /// [`BuildArtifacts::add`] publishes under the builder's output root.
    /// WHY: a builder asks for validated Wasm output instead of assembling module bytes itself.
    ///
    /// Native Wasm compilation of a project package is not available in this compiler yet. It
    /// needs the accepted native host ABI lowering, so this method currently returns
    /// [`ErrorKind::UnsupportedCapability`](super::ErrorKind::UnsupportedCapability) and never a
    /// placeholder module. A builder can match that gap and publish a different artefact, which
    /// keeps the rest of its build honest.
    pub fn compile_wasm(&mut self, package: ProjectPackage<'_>) -> Result<WasmModule, Error> {
        // The accepted native host ABI will lower this handle. Until it does, the request fails
        // before it reads the handle, so no empty module can stand in for a real one.
        let _ = package;
        Err(Error::unsupported_capability(
            "Native Wasm compilation of a project package is not available in this compiler yet.",
        ))
    }
}

/// The bytes of one compiled native WebAssembly module.
///
/// WHAT: the success value of [`BuildContext::compile_wasm`]. It converts into the bytes
/// [`BuildArtifacts::add`] records, so a builder publishes it under a relative output path.
/// WHY: a module exists only when the compiler produced one from the accepted native host ABI.
/// There is no public constructor and no empty or placeholder module, so a builder cannot invent
/// one and a dropped request leaves no output behind.
///
/// `Debug` reports the byte length only, so it never prints the module bytes.
pub struct WasmModule {
    bytes: Vec<u8>,
}

impl From<WasmModule> for Vec<u8> {
    fn from(module: WasmModule) -> Self {
        module.bytes
    }
}

impl fmt::Debug for WasmModule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WasmModule")
            .field("byte_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// The outputs one `ProjectBuilder::build` call produces.
///
/// WHAT: records artefacts under validated relative output paths. `write_outputs` publishes them
/// through the compiler's central writer and manifest.
/// WHY: a build returns its outputs through this type; no file is written until the caller
/// publishes, so a dropped build leaves the output root untouched.
#[derive(Default)]
pub struct BuildArtifacts {
    artifacts: Vec<BuildArtifact>,
}

/// One recorded artefact with the deterministic identity its path registered.
struct BuildArtifact {
    identity: Vec<String>,
    file: OutputFile,
}

impl BuildArtifacts {
    /// Creates an empty artefact set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `bytes` under the relative output path `path`.
    ///
    /// The path is relative to the builder's selected output root. It must be portable, may not
    /// be empty or absolute, may not contain `.` or `..` components, and may not collide with an
    /// earlier artefact by case-folded path identity. A path that would place a file above or
    /// below another artefact (for example `asset` and `asset/child.bin`) is rejected in either
    /// insertion order. The compiler's reserved build-manifest path is rejected too. A rejected
    /// path returns [`ErrorKind::Build`](super::ErrorKind::Build) and names the conflicting
    /// paths.
    ///
    /// This records bytes only. It does not write a file or decide the output root; the central
    /// output writer validates destinations, writes the batch and applies manifest cleanup.
    pub fn add(&mut self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) -> Result<(), Error> {
        let path = path.as_ref();
        let raw = path.to_str().ok_or_else(|| {
            Error::build(format!(
                "The build artefact path {path:?} is not valid UTF-8."
            ))
        })?;
        let portable = parse_relative_path(raw).map_err(|reason| {
            Error::build(format!(
                "The build artefact path '{}' cannot be used: {}.",
                portable_path_text(path),
                describe_rejected_artefact_path(reason)
            ))
        })?;
        let relative_path = portable.to_path_buf();
        if path_starts_with_component_identity(&relative_path, Path::new(BUILD_MANIFEST_FILENAME)) {
            return Err(Error::build(format!(
                "The build artefact path '{}' is reserved for the compiler's build manifest.",
                portable_path_text(&relative_path)
            )));
        }

        let identity = portable.identity_components();
        if let Some(existing) = self.artifacts.iter().find(|artifact| {
            artifact.identity == identity
                || component_identity_prefixes(&identity)
                    .any(|prefix| prefix == artifact.identity.as_slice())
                || component_identity_prefixes(&artifact.identity)
                    .any(|prefix| prefix == identity.as_slice())
        }) {
            let existing_path = existing.file.relative_output_path();
            let message = if existing.identity == identity {
                format!(
                    "The build artefact path '{}' collides with the already registered artefact '{}'.",
                    portable_path_text(&relative_path),
                    portable_path_text(existing_path)
                )
            } else {
                format!(
                    "The build artefact path '{}' conflicts with the already registered artefact '{}': one path is a file and the other is its descendant.",
                    portable_path_text(&relative_path),
                    portable_path_text(existing_path)
                )
            };
            return Err(Error::build(message));
        }

        self.artifacts.push(BuildArtifact {
            identity,
            file: OutputFile::new(relative_path, FileKind::Bytes(bytes.into())),
        });
        Ok(())
    }

    /// Hands the recorded artefacts to the build system's output records.
    pub(super) fn into_output_files(self) -> Vec<OutputFile> {
        self.artifacts
            .into_iter()
            .map(|artifact| artifact.file)
            .collect()
    }
}

impl fmt::Debug for BuildArtifacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(
                self.artifacts
                    .iter()
                    .map(|artifact| artifact.file.relative_output_path()),
            )
            .finish()
    }
}

/// One builder-facing sentence explaining why a portable relative path was rejected.
fn describe_rejected_artefact_path(reason: InvalidOutputFolderReason) -> &'static str {
    match reason {
        InvalidOutputFolderReason::Empty => "the path is empty",
        InvalidOutputFolderReason::NonUtf8 => "the path is not valid UTF-8",
        InvalidOutputFolderReason::AbsolutePath => "an absolute path is not allowed",
        InvalidOutputFolderReason::RootOrPrefix => "a drive or root prefix is not allowed",
        InvalidOutputFolderReason::ParentDirectorySegment => "'..' path components are not allowed",
        InvalidOutputFolderReason::CurrentDirectory => "'.' path components are not allowed",
        InvalidOutputFolderReason::InvalidPathComponent => {
            "the path contains a component that is not portable"
        }
        InvalidOutputFolderReason::VisibleDirectoryOutputPath
        | InvalidOutputFolderReason::ResolvesOutsideProjectRoot => "the path must stay relative",
    }
}
