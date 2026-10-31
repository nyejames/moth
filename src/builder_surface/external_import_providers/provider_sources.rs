//! Source-backed registration for provider-owned files.
//!
//! WHAT: gives an external import provider the boundary's provider-owned source identity, its
//!       live extended-span builder and a place for the source text it already read.
//! WHY: provider diagnostics that describe authored provider syntax need exact spans in the
//!      provider-owned file, not a synthetic range borrowed from the importing Moth source.
//!
//! Stage 0 owns the concrete registrar because it owns source identity, span tables and text
//! retention. Providers only see this narrow service through `ExternalImportProviderContext`.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanCapacityError,
};
use std::path::Path;

/// Registers one provider-owned source and mints spans inside it.
///
/// Implementations retain the supplied text for the boundary source database and keep the
/// returned span builder until `finish` returns it to the source owner.
pub trait ProviderSourceRegistrar {
    /// Register the provider's canonical source path and already-read text.
    ///
    /// The caller moves the text it parsed, so registration never copies the JavaScript body.
    /// The returned handle retains that text's byte length, so every span it mints is bounded by
    /// the bytes the provider actually parsed.
    fn register_provider_source<'a>(
        &'a mut self,
        canonical_path: &Path,
        logical_source_path: &str,
        source: String,
    ) -> Result<ProviderSourceHandle<'a>, ProviderSourceRegistrationError>;

    /// Return any live builder to the source owner after the provider invocation.
    fn finish(&mut self);
}

/// Why a provider-owned source could not be registered.
#[derive(Debug)]
pub enum ProviderSourceRegistrationError {
    /// The source bytes cannot be addressed by a `u32` source snapshot.
    SourceTooLarge,
    /// Stage 0 could not provide the source identity or text slot.
    Infrastructure(InfrastructureFailure),
}

impl From<InfrastructureFailure> for ProviderSourceRegistrationError {
    fn from(failure: InfrastructureFailure) -> Self {
        Self::Infrastructure(failure)
    }
}

/// Why a provider-owned span could not be minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSourceSpanError {
    /// The requested range is reversed, reaches past the registered source length or uses an
    /// offset a source span cannot store.
    ///
    /// A provider mints these offsets from the text it parsed, so this names a provider defect
    /// rather than an authored Moth error.
    InvalidRange {
        byte_start: usize,
        byte_end: usize,
        source_length: usize,
    },
    /// The exact range could not be packed into the source's span table.
    Capacity(SpanCapacityError),
}

/// One registered provider-owned source with its live span builder.
pub struct ProviderSourceHandle<'a> {
    source: SourceId,
    /// Byte length of the text this provider read and parsed.
    source_length: usize,
    span_builder: &'a mut ExtendedSpanBuilder,
}

impl<'a> ProviderSourceHandle<'a> {
    /// Build a handle for one registered source, its text length and its live builder.
    pub(crate) fn new(
        source: SourceId,
        source_length: usize,
        span_builder: &'a mut ExtendedSpanBuilder,
    ) -> Self {
        Self {
            source,
            source_length,
            span_builder,
        }
    }

    /// The boundary source identity that owns this provider file.
    pub fn source(&self) -> SourceId {
        self.source
    }

    /// Mint the exact half-open span `[byte_start, byte_end)` inside the source.
    ///
    /// The handle retains the registered source length, so every range is checked against the
    /// text the provider parsed before any offset reaches a span table. A malformed range and an
    /// offset a source span cannot represent are reported as typed provider failures rather than
    /// panicking or becoming an authored-source error.
    pub fn span(
        &mut self,
        byte_start: usize,
        byte_end: usize,
    ) -> Result<SourceSpan, ProviderSourceSpanError> {
        let invalid_range = || ProviderSourceSpanError::InvalidRange {
            byte_start,
            byte_end,
            source_length: self.source_length,
        };
        if byte_start > byte_end || byte_end > self.source_length {
            return Err(invalid_range());
        }
        let start = u32::try_from(byte_start).map_err(|_| invalid_range())?;
        let length = u32::try_from(byte_end - byte_start).map_err(|_| invalid_range())?;
        let local = LocalSpan::exact(start, length, self.span_builder)
            .map_err(ProviderSourceSpanError::Capacity)?;
        Ok(SourceSpan::new(self.source, local))
    }
}
