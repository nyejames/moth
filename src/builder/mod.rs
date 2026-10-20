//! Public custom-builder SDK.
//!
//! WHAT: lets a separate Rust crate describe the packages and source kinds its projects can use,
//!       and run a build of its project through `build_project`.
//! WHY: builders register through the compiler's existing package registries, build through the
//!      same orchestration as the CLI, and receive structured errors instead of reaching into
//!      compiler-internal builder plumbing.

mod compiled_project;
mod definition;
mod error;
mod project;

pub use compiled_project::{BuildArtifacts, BuildContext, CompiledProject, ProjectPackage};
pub use definition::{BuilderDefinition, BuilderDefinitionDraft, HostPackage, SourcePackage};
pub use error::{Error, ErrorKind};
pub use project::{BuildRequest, BuiltProject, ProjectBuilder, build_project};

#[cfg(test)]
mod tests;
