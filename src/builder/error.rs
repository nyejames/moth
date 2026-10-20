//! Structured errors returned by the builder SDK.

use std::fmt;

/// The category of an SDK failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The builder definition is invalid. Nothing was registered.
    InvalidDefinition,
}

/// An SDK failure with its category and a readable explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub(crate) fn invalid_definition(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::InvalidDefinition,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
