//! Compiler-owned services for source that folds to values instead of becoming a module.
//!
//! WHAT: the two named short compiler paths — project config compilation and direct Moth template
//!       compilation — each owning its whole stage sequence and stopping at folded values.
//! WHY:  both callers legitimately need less than canonical module compilation. Neither exists so
//!       that build or project code may reach raw stage functions: the compiler sequences
//!       tokenization, declaration-shell preparation, interface binding, local declaration ordering
//!       and AST semantics, and hands back only the folded result its caller consumes.
//!
//! # What this module owns
//! - [`config`]: the `config.moth` stage sequence, its dialect surface rules and its folded
//!   declaration output
//! - [`moth_template`]: the direct `.mtf` stage sequence and the folded `content` constant
//!
//! # What this module does NOT own
//! - HIR, borrow facts, link facts and public interfaces, which no service here constructs
//! - Config key schema and the application of folded values to project settings, which stay under
//!   `build_system/project_config`
//! - Moth template source collection and output packaging, which stay under
//!   `projects/html_project/moth_template`

// Both services are reached through the re-exports below, so neither submodule is a crate-wide
// path and neither can be entered part-way through its stage sequence.
mod config;
mod moth_template;

pub(crate) use config::{
    CompiledConfigSource, ConfigCompilationRequest, FoldedConfigDeclaration, compile_config_source,
};
pub(crate) use moth_template::{
    MothTemplateCompilationRequest, MothTemplateFileValueBundle, compile_moth_template_source,
};
