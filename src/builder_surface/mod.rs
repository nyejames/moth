//! Builder surface: package metadata, config schemas, source kinds, and provider registries.
//!
//! WHAT: defines builder surface identity for core packages, builder packages,
//! source-backed packages, external import providers, config schemas and source file kinds.
//! WHY: separates builder surface definition from frontend parsing and backend
//! lowering so each stage has one clear responsibility.

pub mod config_schema;
pub mod core_packages;
pub mod definition;
pub mod external_import_providers;
pub mod package_metadata;
pub mod source_file_kind_registry;
pub mod source_package_registry;

pub use definition::BuilderSurface;
pub use package_metadata::{PackageMetadata, PackageOrigin};
pub use source_file_kind_registry::{SourceFileKind, SourceFileKindRegistry};
pub use source_package_registry::{ProvidedSourceRoot, SourcePackageRegistry};

#[cfg(test)]
mod tests;
