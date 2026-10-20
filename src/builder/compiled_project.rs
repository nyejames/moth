//! Per-build values exchanged between the public `ProjectBuilder` trait and the compiler.
//!
//! WHAT: `CompiledProject` wraps one successful frontend compilation for a single `build` call and
//! hands out an owner-bound `ProjectPackage` for its facade. `BuildContext` carries per-build state
//! into `build`, and `BuildArtifacts` carries the build's outputs back out.
//! WHY: builders receive only these typed handles, so compiler-private compilation, HIR and
//! string-table types never appear in the public SDK surface.

use super::{BuildProfile, Error};
use crate::build_system::build::ProjectCompilation;
use std::fmt;
use std::marker::PhantomData;

/// One successfully compiled project, borrowed for a single `ProjectBuilder::build` call.
pub struct CompiledProject {
    pub(super) compilation: ProjectCompilation,
}

impl CompiledProject {
    pub(crate) fn new(compilation: ProjectCompilation) -> Self {
        Self { compilation }
    }

    /// Returns the handle for the project's package facade.
    ///
    /// Fails with `ErrorKind::Build` when the project has no `+<name>.moth` facade beside
    /// `config.moth` at the project root.
    pub fn require_package(&self) -> Result<ProjectPackage<'_>, Error> {
        if self.compilation.package_assembly().is_none() {
            return Err(Error::build(
                "This project has no package facade. Add a `+<name>.moth` file beside \
                 `config.moth` at the project root.",
            ));
        }

        Ok(ProjectPackage {
            _compiled_project: PhantomData,
        })
    }
}

impl fmt::Debug for CompiledProject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledProject")
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
}

/// The outputs one `ProjectBuilder::build` call produces.
///
/// WHY: a build returns its outputs through this type. The driver does not write them yet, so
/// they are discarded.
#[derive(Debug, Default)]
pub struct BuildArtifacts {
    _private: (),
}

impl BuildArtifacts {
    /// Creates an empty artifact set.
    pub fn new() -> Self {
        Self::default()
    }
}
