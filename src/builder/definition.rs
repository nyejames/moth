//! Builder definitions and the package descriptions they register.
//!
//! WHAT: a fluent draft collects source packages, host packages, source kinds and an optional
//! configuration section, then `finish` registers them into one `BuilderSurface` and validates it.
//! WHY: registration has one validated path. The draft records descriptions only, so a failed
//! `finish` leaves no partially registered definition behind.

use super::Error;
use super::config_section::ConfigSection;
use super::host_conversion;
use crate::build_system::output::BuilderIdentity;
use crate::builder_surface::definition::BuilderSurfaceValidationError;
use crate::builder_surface::{BuilderSurface, OptionalSourceKind, PackageOrigin};
use crate::compiler_frontend::external_packages::ExternalRegistrationError;
use moth_host::HostFunction;
use std::fmt;
use std::path::PathBuf;

/// An immutable, validated builder definition.
#[derive(Clone)]
pub struct BuilderDefinition {
    identity: BuilderIdentity,
    surface: BuilderSurface,
}

impl BuilderDefinition {
    /// Starts a definition for the builder with the stable declared `identity`.
    ///
    /// The identity owns the builder's outputs. It must have Moth identifier shape and must not
    /// be a reserved first-party identity such as `html`. `finish` checks both.
    // The accepted SDK spelling starts a definition here and `finish` returns the immutable
    // value, so an unvalidated definition is never observable.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(identity: impl Into<String>) -> BuilderDefinitionDraft {
        BuilderDefinitionDraft {
            identity: identity.into(),
            source_packages: Vec::new(),
            host_packages: Vec::new(),
            source_kinds: Vec::new(),
            config_sections: Vec::new(),
        }
    }

    /// Returns the stable identity this builder declares, as `finish` validated it.
    ///
    /// The identity owns the builder's output manifests, so a later build only cleans artefacts
    /// its own builder identity wrote and a foreign manifest is never adopted. It is not the Rust
    /// type name. It does not name the configuration section, which `.config` names separately.
    pub fn identity(&self) -> &str {
        self.identity.as_str()
    }

    pub(crate) fn surface(&self) -> &BuilderSurface {
        &self.surface
    }

    /// The configuration section this definition registered, if any.
    ///
    /// The registered schema owns the section name, and `finish` accepts at most one section, so
    /// the surface is the single source of that fact.
    pub(crate) fn config_section_name(&self) -> Option<&str> {
        self.surface
            .config_schemas
            .project_sections()
            .first()
            .map(|section| section.name.as_ref())
    }

    /// The stable identity the compiler uses to name this builder's outputs.
    pub(crate) fn builder_identity(&self) -> BuilderIdentity {
        self.identity.clone()
    }
}

/// Prints the identity and registered package paths. The full surface is a compiler registry,
/// too large to print usefully in assertion failures or logs.
impl fmt::Debug for BuilderDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source_packages = self
            .surface
            .source_packages
            .iter()
            .map(|root| format!("@{}", root.package_prefix))
            .collect::<Vec<_>>();
        let mut host_packages = self
            .surface
            .binding_packages
            .package_paths()
            .collect::<Vec<_>>();
        // The registry iterates a hash map, so sort for a stable summary.
        host_packages.sort_unstable();
        formatter
            .debug_struct("BuilderDefinition")
            .field("identity", &self.identity())
            .field("source_packages", &source_packages)
            .field("host_packages", &host_packages)
            .finish()
    }
}

/// A builder definition under construction. `finish` validates and registers it.
#[derive(Debug, Clone)]
pub struct BuilderDefinitionDraft {
    identity: String,
    source_packages: Vec<SourcePackage>,
    host_packages: Vec<HostPackage>,
    source_kinds: Vec<OptionalSourceKind>,
    config_sections: Vec<ConfigSection>,
}

impl BuilderDefinitionDraft {
    /// Makes a Moth source package available to the project's source.
    pub fn source_package(mut self, package: SourcePackage) -> Self {
        self.source_packages.push(package);
        self
    }

    /// Makes a native Rust host package available to the project's source.
    pub fn host_package(mut self, package: HostPackage) -> Self {
        self.host_packages.push(package);
        self
    }

    /// Accepts plain `.md` Markdown content files.
    pub fn markdown(mut self) -> Self {
        self.source_kinds.push(OptionalSourceKind::PlainMarkdown);
        self
    }

    /// Describes the project configuration section this builder reads.
    ///
    /// `name` is the record name a project authors in `@config.moth`, so it must be a Moth
    /// identifier that is not `project`, not a first-party builder identity and not a reserved
    /// Moth name. The closure declares string and bool fields with [`ConfigSection::string`] and
    /// [`ConfigSection::bool`]. Field names follow the same rules and must stay unique. A
    /// definition has at most one section, and `finish` also adds the automatic `dev_output`
    /// and `release_output` fields.
    ///
    /// A section whose fields are all optional may be omitted by the project, which then reads
    /// the declared defaults. `finish` rejects the definition when a section declares a required
    /// field and a default on the same field.
    pub fn config(
        mut self,
        name: impl Into<String>,
        describe: impl FnOnce(&mut ConfigSection),
    ) -> Self {
        let mut section = ConfigSection::new(name);
        describe(&mut section);
        self.config_sections.push(section);
        self
    }

    /// Validates the complete registration and returns the immutable definition.
    pub fn finish(self) -> Result<BuilderDefinition, Error> {
        let identity = BuilderIdentity::custom(&self.identity).map_err(|reason| {
            Error::invalid_definition(format!("Builder identity '{}' {reason}.", self.identity))
        })?;
        let mut surface = BuilderSurface::with_mandatory_core();

        // The config section is a definition-level contract, so it is validated and registered
        // before any package description reaches its registry.
        if self.config_sections.len() > 1 {
            return Err(Error::invalid_definition(format!(
                "A builder definition declares at most one config section, but this definition \
                 declares '{}' and '{}'.",
                self.config_sections[0].name(),
                self.config_sections[1].name()
            )));
        }
        if let Some(section) = self.config_sections.first() {
            surface
                .config_schemas
                .register_project_section(section.build_schema()?);
        }

        for package in &self.source_packages {
            let prefix = package_path_without_introducer(&package.path)?;
            let root = package.root.as_ref().ok_or_else(|| {
                Error::invalid_definition(format!(
                    "Source package '{}' needs an installed directory from `from_dir`.",
                    package.path
                ))
            })?;
            if !root.is_absolute() {
                return Err(Error::invalid_definition(format!(
                    "Source package '{}' directory '{}' must be an absolute path.",
                    package.path,
                    root.display()
                )));
            }
            if !root.is_dir() {
                return Err(Error::invalid_definition(format!(
                    "Source package '{}' directory '{}' does not exist.",
                    package.path,
                    root.display()
                )));
            }
            surface
                .source_packages
                .register_filesystem_root(prefix, root.clone(), PackageOrigin::Builder)
                .map_err(|error| Error::invalid_definition(error.msg))?;
        }

        // These registrations only validate this draft's descriptors and use IDs issued by
        // this private registry. Rejections belong to the definition, not compiler execution.
        let registration_error_to_builder_error = |error: ExternalRegistrationError| match error {
            ExternalRegistrationError::InvalidDefinition(message) => {
                Error::invalid_definition(message)
            }
            ExternalRegistrationError::Infrastructure(failure) => {
                Error::invalid_definition(String::from(failure.message))
            }
        };

        for package in &self.host_packages {
            // The host backing travels with the package, so requirement production can name
            // exactly the bindings a native Rust host implements.
            let package_id = surface
                .binding_packages
                .register_host_package(&package.path, PackageOrigin::Builder)
                .map_err(registration_error_to_builder_error)?;
            for function in &package.functions {
                let spec = host_conversion::external_function_spec(function).map_err(|reason| {
                    Error::invalid_definition(format!(
                        "Host function '{}' in host package '{}' {reason}.",
                        function.name(),
                        package.path
                    ))
                })?;
                surface
                    .binding_packages
                    .register_external_function(package_id, spec)
                    .map_err(registration_error_to_builder_error)?;
            }
        }

        for kind in self.source_kinds {
            surface.source_file_kinds.register(kind);
        }

        match surface.validate() {
            Ok(()) => {}
            Err(BuilderSurfaceValidationError::InvalidDefinition(message)) => {
                return Err(Error::invalid_definition(message));
            }
            Err(BuilderSurfaceValidationError::Infrastructure(failure)) => {
                return Err(Error::from_infrastructure_errors(failure.into()));
            }
        }
        Ok(BuilderDefinition { identity, surface })
    }
}

/// Source package paths are written with their `@` introducer, such as `@ui`, while the source
/// registry stores the bare prefix.
fn package_path_without_introducer(path: &str) -> Result<&str, Error> {
    path.strip_prefix('@').ok_or_else(|| {
        Error::invalid_definition(format!("Source package '{path}' must start with '@'."))
    })
}

/// A reusable Moth source package installed with the builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackage {
    path: String,
    root: Option<PathBuf>,
}

impl SourcePackage {
    /// Assigns the public package root, such as `@builder/game/ui`.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            root: None,
        }
    }

    /// Supplies the directory holding the package's facade and source files.
    pub fn from_dir(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }
}

/// A package of native Rust host functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPackage {
    path: String,
    functions: Vec<HostFunction>,
}

impl HostPackage {
    /// Assigns the public package root, such as `@builder/game/platform`.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            functions: Vec::new(),
        }
    }

    /// Adds one host function descriptor.
    pub fn function(mut self, function: HostFunction) -> Self {
        self.functions.push(function);
        self
    }
}
