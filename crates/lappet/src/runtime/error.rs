//! Classified runtime failures.
//!
//! Every failure the runtime reports is one [`RuntimeError`] variant chosen by the phase that
//! failed, never by matching message text. Wasmtime's own error is kept as the source for
//! display and logging, behind a standard error object so no Wasmtime type is public.

use std::error::Error;
use std::fmt;

/// The underlying failure, kept for display and never inspected for classification.
type ErrorSource = Box<dyn Error + Send + Sync + 'static>;

/// One runtime failure, classified by the phase that produced it.
#[derive(Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Wasmtime rejected the engine configuration on this host.
    EngineConfiguration { source: ErrorSource },

    /// The bytes are not a core Wasm module this engine accepts.
    InvalidModule { source: ErrorSource },
}

impl RuntimeError {
    pub(super) fn engine_configuration(error: wasmtime::Error) -> Self {
        Self::EngineConfiguration {
            source: error.into_boxed_dyn_error(),
        }
    }

    pub(super) fn invalid_module(error: wasmtime::Error) -> Self {
        Self::InvalidModule {
            source: error.into_boxed_dyn_error(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineConfiguration { .. } => {
                formatter.write_str("the native engine configuration was rejected")
            }
            Self::InvalidModule { .. } => {
                formatter.write_str("the bytes are not a supported core Wasm module")
            }
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EngineConfiguration { source } | Self::InvalidModule { source } => {
                Some(source.as_ref())
            }
        }
    }
}
