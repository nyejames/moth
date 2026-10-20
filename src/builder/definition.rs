//! Builder definitions and the package descriptions they register.
//!
//! WHAT: a fluent draft collects source packages, host packages and source kinds, then `finish`
//! registers them into one `BuilderSurface` and validates it.
//! WHY: registration has one validated path. The draft records descriptions only, so a failed
//! `finish` leaves no partially registered definition behind.

use super::Error;
use crate::build_system::output::BuilderIdentity;
use crate::builder_surface::{BuilderSurface, PackageOrigin, SourceFileKind};
use crate::host::HostFunction;
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
        }
    }

    pub fn identity(&self) -> &str {
        self.identity.as_str()
    }

    pub(crate) fn surface(&self) -> &BuilderSurface {
        &self.surface
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
    source_kinds: Vec<SourceFileKind>,
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

    /// Accepts `.mtf` template content files.
    pub fn moth_templates(mut self) -> Self {
        self.source_kinds.push(SourceFileKind::MothTemplate);
        self
    }

    /// Accepts plain `.md` Markdown content files.
    pub fn markdown(mut self) -> Self {
        self.source_kinds.push(SourceFileKind::PlainMarkdown);
        self
    }

    /// Validates the complete registration and returns the immutable definition.
    pub fn finish(self) -> Result<BuilderDefinition, Error> {
        let identity = BuilderIdentity::custom(&self.identity).map_err(|reason| {
            Error::invalid_definition(format!("Builder identity '{}' {reason}.", self.identity))
        })?;
        let mut surface = BuilderSurface::with_mandatory_core();

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
            if package.available_in_templates {
                if !self.source_kinds.contains(&SourceFileKind::MothTemplate) {
                    return Err(Error::invalid_definition(format!(
                        "Source package '{}' is available in templates, so the definition must \
                         call `moth_templates`.",
                        package.path
                    )));
                }
                surface.register_implicit_template_scope_source_package(prefix);
            }
        }

        for package in &self.host_packages {
            let package_id = surface
                .binding_packages
                .register_package(&package.path, PackageOrigin::Builder)
                .map_err(|error| Error::invalid_definition(error.msg))?;
            for function in &package.functions {
                let spec = function.to_spec().map_err(|reason| {
                    Error::invalid_definition(format!(
                        "Host function '{}' in host package '{}' {reason}.",
                        function.name(),
                        package.path
                    ))
                })?;
                surface
                    .binding_packages
                    .register_external_function(package_id, spec)
                    .map_err(|error| Error::invalid_definition(error.msg))?;
            }
        }

        for kind in self.source_kinds {
            surface.source_file_kinds.register(kind.extension(), kind);
        }

        surface
            .validate()
            .map_err(|error| Error::invalid_definition(error.msg))?;
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
    available_in_templates: bool,
}

impl SourcePackage {
    /// Assigns the public package root, such as `@ui`.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            root: None,
            available_in_templates: false,
        }
    }

    /// Supplies the directory holding the package's facade and source files.
    pub fn from_dir(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Makes the package's exported constants eligible for implicit `.mtf` template scope.
    pub fn available_in_templates(mut self) -> Self {
        self.available_in_templates = true;
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
    /// Assigns the public package root, such as `@platform`.
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
