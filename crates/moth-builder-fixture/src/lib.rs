//! Acceptance fixture for the public builder SDK.
//!
//! WHAT: defines a game builder over one source package, one manual host package and one project
//! configuration section, then publishes an artefact from the compiled project.
//! WHY: the SDK must be usable from a crate outside the compiler. This crate depends only on
//! `moth` and names only `moth::builder` and `moth::host`, so a compiler-internal path or a native
//! engine cannot enter the proof unnoticed.

use moth::builder::{
    BuildArtifacts, BuildContext, BuilderDefinition, CompiledProject, Error, ErrorKind,
    HostPackage, ProjectBuilder, SourcePackage,
};
use moth::host::{HostFunction, HostType};
use std::cell::RefCell;
use std::path::PathBuf;

/// The builder identity the fixture project uses as its configuration section name.
const BUILDER_IDENTITY: &str = "game";

/// The Moth source package the fixture builder installs.
const SOURCE_PACKAGE: &str = "@builder/game/core";

/// The native host package the fixture builder installs.
const HOST_PACKAGE: &str = "@builder/game/platform";

/// The host function the fixture project imports.
const HOST_FUNCTION: &str = "has_prefix";

/// The artefact the fixture publishes while native Wasm compilation is unavailable.
pub const STATUS_ARTEFACT: &str = "game.status";

/// What one fixture build observed from the compiled project.
#[derive(Debug, Clone, Default)]
pub struct BuildObservation {
    /// The title the project authored in the builder's configuration section.
    pub title: Option<String>,
    /// The debug overlay the project authored in the builder's configuration section.
    pub debug_overlay: Option<bool>,
    /// The capability failure `compile_wasm` reported, absent when the request succeeded.
    pub wasm_error: Option<Error>,
}

/// A builder that publishes a game artefact for the fixture project.
pub struct GameBuilder {
    package_root: PathBuf,
    observation: RefCell<BuildObservation>,
}

impl GameBuilder {
    /// Creates a builder whose `@game` source package is installed at `package_root`.
    pub fn new(package_root: impl Into<PathBuf>) -> Self {
        Self {
            package_root: package_root.into(),
            observation: RefCell::new(BuildObservation::default()),
        }
    }

    /// Returns what the last `build` call observed.
    pub fn observation(&self) -> BuildObservation {
        self.observation.borrow().clone()
    }
}

impl ProjectBuilder for GameBuilder {
    fn definition(&self) -> Result<BuilderDefinition, Error> {
        BuilderDefinition::new(BUILDER_IDENTITY)
            .source_package(SourcePackage::new(SOURCE_PACKAGE).from_dir(&self.package_root))
            .host_package(
                HostPackage::new(HOST_PACKAGE).function(
                    HostFunction::new(HOST_FUNCTION)
                        .parameter(HostType::String)
                        .parameter(HostType::String)
                        .returns(HostType::Bool),
                ),
            )
            .config(BUILDER_IDENTITY, |config| {
                config.string("title").required();
                config.bool("debug_overlay").default(false);
            })
            .finish()
    }

    fn build(
        &self,
        project: &CompiledProject,
        context: &mut BuildContext,
    ) -> Result<BuildArtifacts, Error> {
        let settings = project.settings();
        let title = settings.string("title").map(str::to_owned);
        let debug_overlay = settings.bool("debug_overlay");
        let package = project.require_package()?;

        let mut output = BuildArtifacts::new();
        let wasm_error = match context.compile_wasm(package) {
            Ok(module) => {
                output.add("game.wasm", module)?;
                None
            }
            // The capability gap is not a build failure. Record that no native module was produced
            // instead of writing a placeholder module.
            Err(error) if error.kind() == ErrorKind::UnsupportedCapability => Some(error),
            Err(error) => return Err(error),
        };

        if let Some(error) = &wasm_error {
            let title = title.as_deref().unwrap_or("<unset>");
            let status = format!("native_wasm = unavailable\ntitle = {title}\ncause = {error}\n");
            output.add(STATUS_ARTEFACT, status)?;
        }

        *self.observation.borrow_mut() = BuildObservation {
            title,
            debug_overlay,
            wasm_error,
        };
        Ok(output)
    }
}
