//! Public custom-builder SDK.
//!
//! WHAT: lets a separate Rust crate describe the packages and source kinds its projects can use.
//! WHY: builders register through the compiler's existing package registries and receive
//! structured errors instead of reaching into compiler-internal builder plumbing.

mod definition;
mod error;

pub use definition::{BuilderDefinition, BuilderDefinitionDraft, HostPackage, SourcePackage};
pub use error::{Error, ErrorKind};

#[cfg(test)]
mod tests;
