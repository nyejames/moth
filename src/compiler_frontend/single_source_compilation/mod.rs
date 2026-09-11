//! Compiler-owned services for source that folds to values instead of becoming a module.
//!
//! WHAT: the two named short compiler paths — project config compilation and direct Moth template
//!       compilation — each owning its whole stage sequence and stopping at folded values.
//! WHY:  both callers legitimately need less than canonical module compilation. Neither exists so
//!       that build or project code may reach raw stage functions: the compiler sequences
//!       tokenization, declaration-shell preparation, interface binding, local declaration ordering
//!       and AST semantics. Config also returns its live source span builder so the build owner
//!       can finish config validation before freezing the source.
//!
//! # What this module owns
//! - [`config`]: the `config.moth` stage sequence, its dialect surface rules and its folded
//!   declaration output and source-span ownership handoff
//! - [`moth_template`]: the direct `.mtf` stage sequence and the folded `content` constant
//!
//! # What this module does NOT own
//! - HIR, borrow facts, link facts and public interfaces, which no service here constructs
//! - Config key schema and the application of folded values to project settings, which stay under
//!   `build_system/project_config`
//! - Moth template source collection and output packaging, which stay under
//!   `projects/html_project/moth_template`

// Each enabled service is reached through the re-exports below, so no submodule is a crate-wide
// path and no caller can enter part-way through its stage sequence.
mod config;
// WHY: The direct Moth template service is accepted design but has no production consumer today.
// Real `.mtf` files use the integrated Stage 0 `module_preparation`/`canonical` dependency path,
// so this service compiles only for its own tests until a consumer lands.
#[cfg(test)]
mod moth_template;

pub(crate) use config::{
    CompiledConfigSource, ConfigCompilationOutcome, ConfigCompilationRequest,
    FoldedConfigDeclaration, compile_config_source,
};
// WHY: The direct Moth template service is accepted design but has no production consumer today.
// Real `.mtf` files use the integrated Stage 0 `module_preparation`/`canonical` dependency path,
// so this re-export compiles only for its own tests until a consumer lands.
#[cfg(test)]
pub(crate) use moth_template::{
    MothTemplateCompilationRequest, MothTemplateFileValueBundle, compile_moth_template_source,
};
