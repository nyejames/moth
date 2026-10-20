//! Cause-derived infrastructure presentation shared by terminal, terse and HTML output.
//!
//! Original errors stay untouched. This transitional policy does not resolve source spans:
//! a compact source ID alone is not evidence of a matching retained source owner.

use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, ErrorType,
};

pub(crate) const ISSUE_URL: &str = "https://github.com/nyejames/moth/issues";

/// The code and title of the infrastructure failure record. The descriptor table carries the same
/// pair, and a test keeps the two equal, so every renderer reads one spelling.
pub(crate) const INFRASTRUCTURE_FAILURE_CODE: &str = "MOTH-INFRA-0001";
pub(crate) const INFRASTRUCTURE_FAILURE_TITLE: &str = "Infrastructure failure";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfrastructureClass {
    CompilerBug,
    Operational,
}

impl InfrastructureClass {
    pub(crate) fn introduction(self) -> &'static str {
        match self {
            Self::CompilerBug => {
                "COMPILER BUG: Moth encountered an internal compiler or build-system error.\nThis is a Moth toolchain defect, not a source-code error. Please report it at:"
            }
            Self::Operational => {
                "OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation."
            }
        }
    }
}

pub(crate) fn infrastructure_class(error_type: &ErrorType) -> InfrastructureClass {
    match error_type {
        ErrorType::File | ErrorType::DevServer => InfrastructureClass::Operational,
        ErrorType::Compiler
        | ErrorType::HirTransformation
        | ErrorType::Backend(BackendErrorType::LirTransformation)
        | ErrorType::Backend(BackendErrorType::WasmGeneration) => InfrastructureClass::CompilerBug,
    }
}

pub(crate) fn infrastructure_heading(error_type: &ErrorType) -> &'static str {
    match error_type {
        ErrorType::Compiler => "Compiler Bug",
        ErrorType::File => "Filesystem Failure",
        ErrorType::DevServer => "Dev Server Failure",
        ErrorType::HirTransformation => "HIR Transformation Bug",
        ErrorType::Backend(BackendErrorType::LirTransformation) => "LIR Transformation Bug",
        ErrorType::Backend(BackendErrorType::WasmGeneration) => "Wasm Generation Bug",
    }
}

pub(crate) fn format_error_guidance_lines(error: &CompilerError) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(stage) = error
        .metadata
        .get(&CompilerErrorMetadataKey::CompilationStage)
    {
        lines.push(format!("Stage: {stage}"));
    }
    if let Some(suggestion) = error
        .metadata
        .get(&CompilerErrorMetadataKey::PrimarySuggestion)
    {
        lines.push(suggestion.to_owned());
    }
    if let Some(alternative) = error
        .metadata
        .get(&CompilerErrorMetadataKey::AlternativeSuggestion)
    {
        lines.push(format!("Alternative: {alternative}"));
    }
    if let Some(replacement) = error
        .metadata
        .get(&CompilerErrorMetadataKey::SuggestedReplacement)
    {
        lines.push(format!("Suggested replacement: {replacement}"));
    }
    match (
        error
            .metadata
            .get(&CompilerErrorMetadataKey::SuggestedInsertion),
        error
            .metadata
            .get(&CompilerErrorMetadataKey::SuggestedLocation),
    ) {
        (Some(insertion), Some(location)) => {
            lines.push(format!("Suggested insertion: '{insertion}' {location}"))
        }
        (Some(insertion), None) => lines.push(format!("Suggested insertion: '{insertion}'")),
        (None, Some(location)) => lines.push(format!("Suggested location: {location}")),
        (None, None) => {}
    }
    lines
}
