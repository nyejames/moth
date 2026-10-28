//! Compiler diagnostic drafts, compact reports, and render-boundary helpers.
//!
//! WHAT: owns typed family drafts, stage-local accumulation, prepared failures, compact reports,
//! and the renderers that turn their facts into user-visible output.
//! WHY: compiler stages exchange structured facts while each final boundary resolves sources and
//! renders text.

pub(crate) mod compiler_bug;
pub(crate) mod compiler_dev_logging;
pub(crate) mod compiler_errors;
pub(crate) mod compiler_stage;
pub(crate) mod diagnostic_bag;
pub(crate) mod diagnostic_descriptor;
pub(crate) mod diagnostic_identity;
pub(crate) mod diagnostic_severity;
pub(crate) mod diagnostic_token;
pub(crate) mod diagnostic_types;
pub(crate) mod diagnostics;
pub(crate) mod display_messages;
pub(crate) mod prepared_batch;
pub(crate) mod render;
pub(crate) mod trait_keyword_diagnostics;

pub(crate) use crate::compiler_frontend::tokenizer::tokens::TokenDescriptorPayload;
#[cfg(test)]
pub(crate) use crate::compiler_frontend::tokenizer::tokens::TokenTag;
pub(crate) use compiler_stage::CompilerStage;
pub(crate) use diagnostic_bag::{DiagnosticBag, PreparedFailure};
pub(crate) use diagnostic_descriptor::DiagnosticDescriptor;
pub(crate) use diagnostic_identity::is_well_formed_reason_key;
pub(crate) use diagnostic_severity::DiagnosticSeverity;
pub(crate) use diagnostic_token::DiagnosticToken;
pub(crate) use diagnostics::draft::DiagnosticDraft;
pub(crate) use diagnostics::labels::GenericSubstitutionDiagnostic;
pub(crate) use diagnostics::reasons::*;
pub(crate) use diagnostics::schema::DiagnosticCategory;

#[cfg(test)]
#[path = "tests/diagnostic_token_projection_tests.rs"]
mod diagnostic_token_projection_tests;

#[cfg(test)]
#[path = "tests/prepared_batch_tests.rs"]
mod prepared_batch_tests;

#[cfg(test)]
#[path = "tests/type_rendering_tests.rs"]
mod type_rendering_tests;

#[cfg(test)]
#[path = "tests/boundary_failure_tests.rs"]
mod boundary_failure_tests;
#[cfg(test)]
#[path = "tests/compiler_bug_tests.rs"]
mod compiler_bug_tests;
