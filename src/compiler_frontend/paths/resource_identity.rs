//! Stable cross-build identity for one authored or generated resource.
//!
//! WHAT: owns the portable owner-relative resource path spelling, the module and provider
//! resource owners, and the stable resource origin those two facts compose into.
//! WHY: resource origin is semantic identity. It has to survive checkout roots, traversal
//! order, the ordinary source file that happens to contain a declaration, consumer aliases and
//! every later build decision, so it carries no absolute path, output path, route, URL or
//! content hash.
//!
//! This module owns identity only. Filesystem resolution belongs to path resolution, byte
//! sources and output placement belong to the build system, and dense module-local handles
//! belong to `module_resources`.

// Resource identity is built before its consumers. The AST resource classifier interns these
// origins, and the build-owned byte-source registry keys on them, so this allowance is removed by
// the slice that wires resolution into AST expression parsing.
#![allow(dead_code)]

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::{
    StableModuleOriginIdentity, StablePackageIdentity, portable_relative_logical_path_from,
};
use std::path::Path;

/// Owned, portable, owner-root-relative spelling of one resource path.
///
/// WHAT: the forward-slash logical path of a resource relative to the root of the owner that
/// declares it, including the final filename and its explicit extension.
/// WHY: identity must be platform-independent and self-contained. Storing a `PathBuf` would make
/// the same logical resource compare differently across checkout roots and path separators.
///
/// Construction is total. Empty paths and paths whose final component carries no explicit
/// extension are internal invariant violations rather than diagnostics: the AST resource
/// classifier reports those authoring mistakes with source context before identity is built.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PortableResourcePath {
    spelling: String,
}

impl PortableResourcePath {
    /// Build the portable spelling from an owner-root-relative logical path.
    ///
    /// Only normal relative UTF-8 components are accepted, so `CurDir`, `ParentDir`, `RootDir`
    /// and `Prefix` components cannot collapse two different inputs onto one identity.
    pub(crate) fn from_relative_logical_path(relative: &Path) -> Result<Self, CompilerError> {
        let spelling = portable_relative_logical_path_from(relative)?;

        if spelling.is_empty() {
            return Err(CompilerError::compiler_error(
                "a resource path identity cannot be built from an empty owner-relative path; a \
                 resource always names one file inside its owner",
            ));
        }

        if !has_explicit_extension(&spelling) {
            return Err(CompilerError::compiler_error(format!(
                "resource path {spelling:?} has no explicit extension on its final component; \
                 extension validation is an AST diagnostic that runs before identity construction"
            )));
        }

        Ok(Self { spelling })
    }

    /// The portable forward-slash spelling.
    pub(crate) fn as_str(&self) -> &str {
        &self.spelling
    }
    /// Rebuild a resource path from its already-portable forward-slash spelling.
    ///
    /// Persistent generic materialisation has no filesystem path to pass back through the
    /// resolver. Reusing the canonical relative-path validation keeps the rebuilt identity subject
    /// to the same component and extension invariants as Stage 0.
    pub(crate) fn from_portable_spelling(spelling: String) -> Result<Self, CompilerError> {
        Self::from_relative_logical_path(Path::new(&spelling))
    }
}

/// True when the final component of a portable spelling carries one explicit extension.
///
/// A leading dot is a dotfile rather than an extension, and a trailing dot names no extension.
fn has_explicit_extension(spelling: &str) -> bool {
    let final_component = match spelling.rsplit_once('/') {
        Some((_, last)) => last,
        None => spelling,
    };

    match final_component.rfind('.') {
        Some(0) | None => false,
        Some(dot) => dot + 1 < final_component.len(),
    }
}

/// Owned, hashable, cross-build identity for one provider that owns generated resources.
///
/// WHAT: composes the builder-registered provider kind with the stable package identity the
/// provider produced the resource for.
/// WHY: provider output that transforms or generates bytes cannot borrow the module-owned origin
/// of its input, because two providers may legitimately derive different resources from the same
/// source file. Provider output that reuses an unchanged module-owned source deliberately keeps
/// the module owner instead.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StableProviderResourceOwnerId {
    provider_kind: String,
    package: StablePackageIdentity,
}

impl StableProviderResourceOwnerId {
    pub(crate) fn new(provider_kind: &str, package: StablePackageIdentity) -> Self {
        Self {
            provider_kind: provider_kind.to_owned(),
            package,
        }
    }

    pub(crate) fn provider_kind(&self) -> &str {
        &self.provider_kind
    }

    pub(crate) fn package(&self) -> &StablePackageIdentity {
        &self.package
    }
}

/// The owner whose filesystem or generation authority produced one resource.
///
/// Module-owned resources derive their package identity through the stable module origin, so
/// package identity is never duplicated beside the module owner.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum StableResourceOwnerId {
    Module(StableModuleOriginIdentity),
    Provider(StableProviderResourceOwnerId),
}

/// Owned, hashable, cross-build semantic identity for one resource.
///
/// WHAT: one resource owner plus the owner-relative portable resource path.
/// WHY: this is the only resource fact that crosses a module or package boundary. Uses, byte
/// sources, output paths and rendered URLs are separate facts keyed by this identity.
///
/// Identity excludes absolute paths, content hashes, output paths, aliases, export bindings,
/// source locations, routes and builder prefixes. Moving a declaration between ordinary files in
/// one module leaves it unchanged. Moving or renaming the resource within its owner changes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StableResourceOriginId {
    owner: StableResourceOwnerId,
    logical_path: PortableResourcePath,
}

impl StableResourceOriginId {
    pub(crate) fn new(owner: StableResourceOwnerId, logical_path: PortableResourcePath) -> Self {
        Self {
            owner,
            logical_path,
        }
    }

    /// Identity for one resource owned by a module's private filesystem ownership.
    pub(crate) fn module_owned(
        module_origin: StableModuleOriginIdentity,
        logical_path: PortableResourcePath,
    ) -> Self {
        Self::new(StableResourceOwnerId::Module(module_origin), logical_path)
    }

    pub(crate) fn owner(&self) -> &StableResourceOwnerId {
        &self.owner
    }

    pub(crate) fn logical_path(&self) -> &PortableResourcePath {
        &self.logical_path
    }
}

#[cfg(test)]
#[path = "tests/resource_identity_tests.rs"]
mod tests;
