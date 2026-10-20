//! Source-backed package registry.
//!
//! WHAT: tracks builder-provided and project-local source-backed package roots.
//! WHY: source-backed package dependencies resolve to actual `.moth` files, not binding-backed
//!      packages, so the path resolver needs to know where each package prefix lives.

//! Roots are stored in a `BTreeMap` so that iteration surfaces one canonical
//! package-prefix order at every Stage 0 and header boundary, with no need for
//! downstream consumers to re-sort.

use crate::builder_surface::package_metadata::{
    PackageMetadata, PackageOrigin, validate_package_path,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Registry of source-backed package roots indexed by their package prefix.
///
/// WHAT: maps `@`-stripped package prefixes like `"html"` to filesystem roots.
/// WHY: the path resolver checks these prefixes before falling back to entry-root resolution.
#[derive(Clone, Debug, Default)]
pub struct SourcePackageRegistry {
    roots: BTreeMap<String, SourcePackageRoot>,
}

impl SourcePackageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a filesystem source-backed package root.
    ///
    /// Each prefix has one root. A second registration, including an ASCII case-only variant,
    /// is rejected rather than replacing the first. The prefix follows the shared package-path
    /// rule and is one component, because source resolution matches only the first component of
    /// a dependency path.
    pub fn register_filesystem_root(
        &mut self,
        package_prefix: impl Into<String>,
        root: PathBuf,
        origin: PackageOrigin,
    ) -> Result<(), CompilerError> {
        let prefix = package_prefix.into();
        if let Err(reason) = validate_package_path(&prefix, origin) {
            return Err(CompilerError::compiler_error(format!(
                "Source package path '@{prefix}' {reason}."
            )));
        }
        if prefix.contains('/') {
            return Err(CompilerError::compiler_error(format!(
                "Source package path '@{prefix}' must have exactly one component."
            )));
        }
        if self
            .roots
            .keys()
            .any(|existing| existing.eq_ignore_ascii_case(&prefix))
        {
            return Err(CompilerError::compiler_error(format!(
                "Source package '@{prefix}' is already registered."
            )));
        }
        self.roots.insert(
            prefix.clone(),
            SourcePackageRoot {
                package_prefix: prefix,
                root: ProvidedSourceRoot::Filesystem(root),
                metadata: PackageMetadata::source(origin),
            },
        );
        Ok(())
    }

    /// Returns the root for a given package prefix, if any.
    pub fn get_root(&self, prefix: &str) -> Option<&SourcePackageRoot> {
        self.roots.get(prefix)
    }

    /// Returns true if the registry contains a root with the given prefix.
    pub fn has_prefix(&self, prefix: &str) -> bool {
        self.roots.contains_key(prefix)
    }

    /// Iterate over all registered roots.
    pub fn iter(&self) -> impl Iterator<Item = &SourcePackageRoot> {
        self.roots.values()
    }
}

/// One source-backed package root.
#[derive(Clone, Debug)]
pub struct SourcePackageRoot {
    pub package_prefix: String,
    pub root: ProvidedSourceRoot,
    pub metadata: PackageMetadata,
}

/// Where a source-backed package's files live.
#[derive(Clone, Debug)]
pub enum ProvidedSourceRoot {
    /// Files on the local filesystem.
    Filesystem(PathBuf),
}
