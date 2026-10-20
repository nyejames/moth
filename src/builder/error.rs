//! Structured errors returned by the builder SDK.
//!
//! WHAT: an `Error` pairs a stable `ErrorKind` with a summary and, for compiler failures, the
//!       terse rendered compiler lines.
//! WHY: SDK callers branch on the kind and print the lines without matching compiler-private
//!      diagnostic types.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    InfrastructureClass, infrastructure_class,
};
use std::fmt;

/// The category of a builder SDK failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The builder definition is invalid. Nothing was registered.
    InvalidDefinition,
    /// The project failed to compile. The error's messages hold the rendered diagnostics, and
    /// the builder's `build` step did not run.
    Compilation,
    /// The builder's `build` step failed, or the project has no package facade.
    Build,
    /// The build could not read or write a file, or could not use a project path.
    Infrastructure,
    /// The compiler hit an internal error. This is a toolchain defect, not a source error.
    CompilerBug,
}

/// A builder SDK failure: its category, a summary, and any rendered compiler lines.
///
/// `Display` writes the summary, then each rendered line on its own line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    messages: Vec<String>,
}

impl Error {
    pub(crate) fn invalid_definition(message: impl Into<String>) -> Self {
        Self::without_messages(ErrorKind::InvalidDefinition, message)
    }

    /// Creates a failure from a builder's own `build` step.
    pub fn build(message: impl Into<String>) -> Self {
        Self::without_messages(ErrorKind::Build, message)
    }

    pub(crate) fn infrastructure(message: impl Into<String>) -> Self {
        Self::without_messages(ErrorKind::Infrastructure, message)
    }

    /// Classifies a failed compiler result and renders its messages.
    ///
    /// Infrastructure failures take precedence over diagnostics, so a compiler bug is reported
    /// as `CompilerBug` even when diagnostics accompany it.
    pub(crate) fn from_compiler_messages(compiler_messages: &CompilerMessages) -> Self {
        let messages = format_terse_compiler_messages(compiler_messages);
        let (kind, summary) = match compiler_messages.infrastructure_errors() {
            None => (ErrorKind::Compilation, "The project failed to compile."),
            Some(infrastructure_errors) => {
                if infrastructure_errors.iter().any(|error| {
                    infrastructure_class(&error.error_type) == InfrastructureClass::CompilerBug
                }) {
                    (
                        ErrorKind::CompilerBug,
                        "The build hit an internal compiler error.",
                    )
                } else {
                    (
                        ErrorKind::Infrastructure,
                        "The build hit an infrastructure error.",
                    )
                }
            }
        };
        Self {
            kind,
            message: summary.to_owned(),
            messages,
        }
    }

    fn without_messages(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            messages: Vec::new(),
        }
    }

    /// The stable category of this failure.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The lines are in terse display order: errors, then warnings, then notes, with order preserved
    /// within each severity group. Empty when no compiler messages produced this failure.
    pub fn messages(&self) -> &[String] {
        &self.messages
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        for line in &self.messages {
            write!(formatter, "\n{line}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}
