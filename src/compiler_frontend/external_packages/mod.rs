//! Builtin external function metadata and registry.
//!
//! WHAT: defines the external-call surface the frontend and borrow checker understand.
//! WHY: external calls need one canonical metadata source for signature lowering and call semantics.
//!
//! External symbols are registered by package scope: `(package_path, symbol_path)` uniquely
//! identifies a function, type, or constant. The same leaf name may exist under different
//! namespace paths in the same package. The prelude `io` namespace alias is the only exception where
//! bare-name lookup is valid. All other external symbol resolution must go through file-local
//! `visible_external_symbols`.
//!
//! ## Module layout
//!
//! - `ids`: stable identifiers (`ExternalFunctionId`, `ExternalTypeId`, `ExternalPackageId`, etc.)
//! - `abi`: backend-agnostic ABI types (`ExternalAbiType`, `ExternalParameter`, etc.)
//! - `definitions`: function/type/constant definitions and lowering metadata
//! - `registry`: `ExternalPackageRegistry` with registration and lookup APIs
//! - `symbol_path`: structured multi-component external symbol path (`ExternalSymbolPath`)
//! - `packages/`: test-only package definition files

mod abi;
mod definitions;
mod ids;
mod registry;
mod symbol_path;

mod packages;

#[cfg(test)]
mod tests;

pub use abi::*;
pub use definitions::*;
pub use ids::*;
pub use registry::*;
pub use symbol_path::*;

/// Builds the mandatory external package registry used by normal frontend compilation.
///
/// WHAT: calls each package-specific registration helper in order to produce a fully
/// populated `ExternalPackageRegistry`.
/// WHY: the registry constructor should read like orchestration, not like a data dump.
/// Keeping package definitions in `src/builder_surface/core_packages/` prevents the constructor from
/// growing into an unmaintainable wall of struct literals and separates package
/// identity from registry mechanics.
pub fn build_builtin_registry() -> ExternalPackageRegistry {
    let mut registry = ExternalPackageRegistry::default();
    crate::builder_surface::core_packages::register_core_io_package(&mut registry);
    crate::builder_surface::core_packages::register_core_collections_package(&mut registry);
    crate::builder_surface::core_packages::register_core_prelude(&mut registry);
    registry
}
