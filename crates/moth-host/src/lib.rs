//! Host contract vocabulary shared by the compiler and native runtimes.
//!
//! WHAT: the closed V1 semantic vocabulary a builder and runtime agree on. It covers the exact
//! scalar types except `F16`, `String`, one optional layer and the builtin `Error!` channel. It
//! also covers the canonical binding identity, the compiler-produced import and export
//! requirements, the builder's explicit export selection and a pure-Rust preflight that matches
//! required imports against declared Rust implementations.
//! WHY: builders describe host bindings without naming compiler-internal types, and a native
//! runtime reads the same semantic values without depending on the compiler. The compiler owns the
//! mapping onto its external-package registry and produces the requirements. This crate holds the
//! data both sides must agree on and no Wasmtime or compiler-internal type.
//!
//! Everything here is semantic. Physical ABI facts, such as Wasm carriers, module and item
//! spellings, result frames, reserved helper names and the host manifest, arrive with the
//! compiler's host manifest and are owned by the native host ABI.

mod descriptor;
mod identity;
mod preflight;
mod requirements;
mod selection;

pub use descriptor::{HostFunction, HostSignature, HostType};
pub use identity::HostBindingIdentity;
pub use preflight::{HostImplementation, PreflightError, PreflightErrors, preflight};
pub use requirements::{
    HostExportRequirement, HostImportRequirement, HostRequirements, RequirementsError,
};
pub use selection::{ExportSelectionError, HostExport, HostExportSelection, HostExports};

#[cfg(test)]
#[path = "tests/descriptor_tests.rs"]
mod descriptor_tests;

#[cfg(test)]
#[path = "tests/requirements_tests.rs"]
mod requirements_tests;

#[cfg(test)]
#[path = "tests/export_tests.rs"]
mod export_tests;

#[cfg(test)]
#[path = "tests/preflight_tests.rs"]
mod preflight_tests;
