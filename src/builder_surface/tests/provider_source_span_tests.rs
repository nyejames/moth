//! Unit tests for provider-owned source span minting.
//!
//! WHAT: proves `ProviderSourceHandle::span` enforces the registered source length and reports a
//!       provider defect instead of panicking or emitting an authored-source diagnostic.
//! WHY: providers are trusted code whose byte offsets still reach the source span tables, so the
//!      handle must bound every range it accepts.

use crate::builder_surface::external_import_providers::provider_sources::{
    ProviderSourceHandle, ProviderSourceSpanError,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};

#[test]
fn provider_span_resolves_the_requested_bytes() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(0);
    let (span, end_span) = {
        let mut handle = ProviderSourceHandle::new(source, 32, &mut builder);
        (
            handle
                .span(4, 9)
                .expect("an in-range span should be minted"),
            handle
                .span(32, 32)
                .expect("the empty range at the end of the source should be minted"),
        )
    };

    let range = span.resolve_with(builder.resolver_for(source));
    assert_eq!((range.start(), range.end()), (4, 9));
    let end_range = end_span.resolve_with(builder.resolver_for(source));
    assert_eq!((end_range.start(), end_range.end()), (32, 32));
}

#[test]
fn reversed_provider_span_is_rejected() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(0);
    let mut handle = ProviderSourceHandle::new(source, 32, &mut builder);

    let error = handle
        .span(9, 4)
        .expect_err("a reversed range cannot become a span");

    assert_eq!(
        error,
        ProviderSourceSpanError::InvalidRange {
            byte_start: 9,
            byte_end: 4,
            source_length: 32,
        }
    );
}

#[test]
fn provider_span_past_the_registered_length_is_rejected() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(0);
    {
        let mut handle = ProviderSourceHandle::new(source, 100_000, &mut builder);
        let error = handle
            .span(50_000, 100_001)
            .expect_err("a range past the registered source length cannot become a span");

        assert_eq!(
            error,
            ProviderSourceSpanError::InvalidRange {
                byte_start: 50_000,
                byte_end: 100_001,
                source_length: 100_000,
            }
        );
    }

    assert_eq!(
        builder.len(),
        0,
        "a rejected range must not append to the extended span table"
    );
}

#[test]
fn unrepresentable_provider_span_is_rejected() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(0);
    let unrepresentable = u32::MAX as usize + 1;
    let mut handle = ProviderSourceHandle::new(source, unrepresentable * 2, &mut builder);

    let error = handle
        .span(unrepresentable, unrepresentable + 1)
        .expect_err("an offset a source span cannot store must be rejected");

    assert_eq!(
        error,
        ProviderSourceSpanError::InvalidRange {
            byte_start: unrepresentable,
            byte_end: unrepresentable + 1,
            source_length: unrepresentable * 2,
        }
    );
}
