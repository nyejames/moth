//! General external import provider registry and types.
//!
//! WHAT: defines the trait, request/response types, provider registry, and build-owned cache
//!       that allow builders to register external file parsers (JS, WIT, Rust, host manifests).
//! WHY: the compiler needs a general, non-JS-specific hook for resolving imports that target
//!      non-Moth source files into typed external package surfaces.
//!
//! ## Module layout
//!
//! - `provider`: the `ExternalImportProvider` trait and its input/output types.
//! - `registry`: `ExternalImportProviderRegistry` with registration and lookup helpers.
//! - `cache`: `ExternalImportProviderCache` keyed by canonical path + provider kind.

pub mod cache;
pub mod provider;
pub mod registry;
pub mod resolution_table;
