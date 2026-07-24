//! External import provider trait and associated request/response types.
//!
//! WHAT: defines the contract between the compiler and builder-registered providers that
//!       resolve non-Moth source files into typed external package surfaces.
//! WHY: keeps the provider API general so JS, WIT, Rust, and host-manifest providers all
//!      fit the same shape without leaking JS-specific concepts into the trait.

use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageId, ExternalPackageRegistry, ExternalTypeId,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;

/// Stable kind key for an external import provider.
///
/// WHAT: identifies which provider produced a result so caches, diagnostics, and backend
///       emission can reason about the source without inspecting file extensions.
/// WHY: provider kinds are builder extension points. Keeping this as an owned key avoids
/// hard-coding every future provider family into the compiler core.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalImportProviderKind(pub String);

impl ExternalImportProviderKind {
    /// Creates a provider kind key.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrows the provider kind as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ExternalImportProviderKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ExternalImportProviderKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Newtype for a file extension supported by an external import provider.
///
/// WHAT: wraps an owned extension string such as `"js"` or `"wit"`.
/// WHY: distinguishes raw extension strings from other strings at the type level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalFileExtension(pub String);

impl ExternalFileExtension {
    /// Borrows the extension as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ExternalFileExtension {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ExternalFileExtension {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Input facts passed to a provider when resolving a single external import.
///
/// WHAT: carries the import path, canonical filesystem location, and source location
///       so the provider can parse the file and emit diagnostics that point back to
///       the Moth source that requested the import.
/// WHY: context structs avoid long parameter lists and keep the trait stable.
#[derive(Debug, Clone)]
pub struct ExternalImportRequest {
    /// The import path as written in source (e.g. `@canvas/drawing.js`).
    pub import_path: String,
    /// Canonical absolute path to the external source file.
    pub canonical_source_path: PathBuf,
    /// Source location of the import statement in the requesting Moth file.
    pub source_location: SourceLocation,
}

/// Mutable context available during provider resolution.
///
/// WHAT: gives the provider access to the shared external package registry and the
///       build-owned cache so it can register new packages and reuse previous results.
/// WHY: provider results must feed into the same `ExternalPackageRegistry` that AST
///      and HIR consume, and caching must be build-scoped.
pub struct ExternalImportProviderContext<'a> {
    pub package_registry: &'a mut ExternalPackageRegistry,
    pub cache: &'a mut super::cache::ExternalImportProviderCache,
    pub string_table: &'a mut StringTable,
}

impl std::fmt::Debug for ExternalImportProviderContext<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalImportProviderContext")
            .field("package_registry", &"...")
            .field("cache", &self.cache)
            .field("string_table", &"...")
            .finish()
    }
}

/// General trait for resolving non-Moth source files into typed external imports.
///
/// WHAT: every builder-registered provider (JS, WIT, etc.) implements this trait.
/// WHY: keeps the compiler frontend agnostic to the exact syntax of external files.
///
/// Implementations must be `Send + Sync` because provider registries are stored in
/// `BuilderSurface` and may be cloned and shared across compilation jobs.
pub trait ExternalImportProvider: Send + Sync + std::fmt::Debug {
    /// Returns the general kind of this provider.
    fn kind(&self) -> ExternalImportProviderKind;

    /// Returns the file extensions this provider can handle.
    ///
    /// Examples: `["js"]` for a JavaScript provider, `["wit"]` for a WIT provider.
    fn supported_extensions(&self) -> &[ExternalFileExtension];

    /// Attempts to resolve an external import request.
    ///
    /// WHAT: parses the canonical source file, registers discovered symbols in the
    ///       package registry, and returns a structured result.
    /// WHY: the compiler frontend sees only typed external IDs, not raw file syntax.
    ///
    /// Returns `Ok(None)` when the provider declines this request (e.g. the file
    /// extension matches but the content is unrecognizable). Returns
    /// `Err(CompilerMessages)` when the provider emits user-facing diagnostics.
    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages>;
}

/// Identity for a runtime asset that the backend must emit.
///
/// WHAT: describes an external source file that should be copied or processed into
///       the build output.
/// WHY: separates asset tracking from package symbol metadata so backends can emit
///      JS, WIT, or other asset kinds using the same general model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeAssetIdentity {
    /// Canonical path to the asset source file.
    pub canonical_source_path: PathBuf,
    /// General asset category used by backends to decide emission strategy.
    /// Examples: `"js"`, `"wit"`, `"rust"`.
    pub asset_kind: String,
}

/// A runtime module import required by an external resolved import.
///
/// WHAT: records that generated backend glue must import specific symbols from a
///       registered core runtime module.
/// WHY: general enough for JS core modules, Wasm host imports, or other runtime
///      scaffolding without hard-coding JS import syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequiredRuntimeImport {
    /// Runtime module name, e.g. `"@moth/runtime"`.
    pub module_name: String,
    /// Symbols imported from that module, e.g. `["mothOk", "mothErr"]`.
    pub imported_names: Vec<String>,
}

/// Metadata for a builder-runtime package that needs runtime asset/glue emission.
///
/// WHAT: carries the minimal data needed to synthesize a `ModuleExternalImport` for
///       builder-owned packages that are registered directly in the registry.
/// WHY: builder-runtime packages and provider-resolved packages share the same backend
///      emission path, but builder-runtime packages never go through a provider.
#[derive(Debug, Clone)]
pub struct BuilderRuntimePackageMetadata {
    pub package_id: ExternalPackageId,
    pub runtime_asset: Option<RuntimeAssetIdentity>,
    pub required_runtime_imports: Vec<RequiredRuntimeImport>,
}

/// Result of resolving a single external import through a provider.
///
/// WHAT: carries the stable external IDs, asset identity, and metadata discovered
///       by parsing one external source file.
/// WHY: the compiler frontend uses only these IDs and metadata; it never sees raw
///      file syntax or paths after resolution.
#[derive(Debug, Clone)]
pub struct ResolvedExternalImport {
    /// The package that contains all symbols discovered in this file.
    pub package_id: ExternalPackageId,
    /// Opaque external types exported by this file.
    pub exported_types: Vec<ExternalTypeId>,
    /// Free functions exported by this file.
    pub exported_free_functions: Vec<ExternalFunctionId>,
    /// Optional runtime asset that the backend must emit.
    pub runtime_asset: Option<RuntimeAssetIdentity>,
    /// Diagnostics or warnings emitted while parsing the external file.
    pub diagnostics: Vec<CompilerDiagnostic>,
    /// Runtime module imports that generated glue must include.
    pub required_runtime_imports: Vec<RequiredRuntimeImport>,
}
