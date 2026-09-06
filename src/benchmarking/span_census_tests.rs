use super::span_census::{
    LengthHistogramBucket, PrototypeEncodedSpan, decode_candidate_span, encode_candidate_span,
    inline_start_limit, tokenize_corpus, tokenize_source,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;

/// Hand-computed 22/10 split: 2^10 - 2 = 1022 is the last inline length, 1023 is not.
#[test]
fn length_bits_10_inline_length_boundary_is_1022() {
    let mut extended = Vec::new();
    let inline = encode_candidate_span(10, 0, 1022, &mut extended)
        .expect("length 1022 should encode for LENGTH_BITS=10");
    let overflow = encode_candidate_span(10, 0, 1023, &mut extended)
        .expect("length 1023 should encode as extended for LENGTH_BITS=10");

    assert!(matches!(inline, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(overflow, PrototypeEncodedSpan::Extended(_)));
    assert_eq!(extended, vec![(0, 1023)]);
}

/// Hand-computed 22/10 split: 2^22 - 1 = 4_194_303 is the last inline start, 4_194_304 is not.
#[test]
fn length_bits_10_inline_start_boundary_is_4_194_303() {
    let mut extended = Vec::new();
    let inline = encode_candidate_span(10, 4_194_303, 0, &mut extended)
        .expect("start 4_194_303 should encode for LENGTH_BITS=10");
    let overflow = encode_candidate_span(10, 4_194_304, 0, &mut extended)
        .expect("start 4_194_304 should encode as extended for LENGTH_BITS=10");

    assert!(matches!(inline, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(overflow, PrototypeEncodedSpan::Extended(_)));
    assert_eq!(extended, vec![(4_194_304, 0)]);
}

/// Hand-computed 24/8 split: 2^8 - 2 = 254 is inline, 255 is not.
#[test]
fn length_bits_8_inline_length_boundary_is_254() {
    let mut extended = Vec::new();
    let inline = encode_candidate_span(8, 0, 254, &mut extended)
        .expect("length 254 should encode for LENGTH_BITS=8");
    let overflow = encode_candidate_span(8, 0, 255, &mut extended)
        .expect("length 255 should encode as extended for LENGTH_BITS=8");

    assert!(matches!(inline, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(overflow, PrototypeEncodedSpan::Extended(_)));
}

/// Hand-computed 24/8 split: 2^24 - 1 = 16_777_215 is the last inline start.
#[test]
fn length_bits_8_inline_start_boundary_is_16_777_215() {
    let mut extended = Vec::new();
    let inline = encode_candidate_span(8, 16_777_215, 0, &mut extended)
        .expect("start 16_777_215 should encode for LENGTH_BITS=8");
    let overflow = encode_candidate_span(8, 16_777_216, 0, &mut extended)
        .expect("start 16_777_216 should encode as extended for LENGTH_BITS=8");

    assert!(matches!(inline, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(overflow, PrototypeEncodedSpan::Extended(_)));
}

/// Hand-computed 20/12 split: 2^12 - 2 = 4094 is inline, 4095 is not.
#[test]
fn length_bits_12_inline_length_boundary_is_4094() {
    let mut extended = Vec::new();
    let inline = encode_candidate_span(12, 0, 4094, &mut extended)
        .expect("length 4094 should encode for LENGTH_BITS=12");
    let overflow = encode_candidate_span(12, 0, 4095, &mut extended)
        .expect("length 4095 should encode as extended for LENGTH_BITS=12");

    assert!(matches!(inline, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(overflow, PrototypeEncodedSpan::Extended(_)));
}

#[test]
fn length_histogram_puts_each_boundary_length_in_the_closed_bucket() {
    assert_eq!(
        LengthHistogramBucket::for_length(0),
        LengthHistogramBucket::AtMost254
    );
    assert_eq!(
        LengthHistogramBucket::for_length(254),
        LengthHistogramBucket::AtMost254
    );
    assert_eq!(
        LengthHistogramBucket::for_length(255),
        LengthHistogramBucket::AtMost510
    );
    assert_eq!(
        LengthHistogramBucket::for_length(510),
        LengthHistogramBucket::AtMost510
    );
    assert_eq!(
        LengthHistogramBucket::for_length(511),
        LengthHistogramBucket::AtMost1022
    );
    assert_eq!(
        LengthHistogramBucket::for_length(1022),
        LengthHistogramBucket::AtMost1022
    );
    assert_eq!(
        LengthHistogramBucket::for_length(1023),
        LengthHistogramBucket::AtMost2046
    );
    assert_eq!(
        LengthHistogramBucket::for_length(2046),
        LengthHistogramBucket::AtMost2046
    );
    assert_eq!(
        LengthHistogramBucket::for_length(2047),
        LengthHistogramBucket::AtMost4094
    );
    assert_eq!(
        LengthHistogramBucket::for_length(4094),
        LengthHistogramBucket::AtMost4094
    );
    assert_eq!(
        LengthHistogramBucket::for_length(4095),
        LengthHistogramBucket::From4095
    );
}

#[test]
fn prototype_codec_round_trips_inline_and_extended_spans() {
    let mut extended = Vec::new();

    let zero = encode_candidate_span(10, 0, 0, &mut extended)
        .expect("zero-length span at start 0 should encode");
    let typical = encode_candidate_span(10, 1200, 40, &mut extended)
        .expect("typical inline span should encode");
    let long =
        encode_candidate_span(10, 8, 1023, &mut extended).expect("length overflow should encode");
    let far = encode_candidate_span(10, 4_194_304, 12, &mut extended)
        .expect("start overflow should encode");

    assert!(matches!(zero, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(typical, PrototypeEncodedSpan::Inline(_)));
    assert!(matches!(long, PrototypeEncodedSpan::Extended(_)));
    assert!(matches!(far, PrototypeEncodedSpan::Extended(_)));

    assert_eq!(
        decode_candidate_span(10, zero, &extended).expect("zero-length inline should decode"),
        (0, 0)
    );
    assert_eq!(
        decode_candidate_span(10, typical, &extended).expect("typical inline should decode"),
        (1200, 40)
    );
    assert_eq!(
        decode_candidate_span(10, long, &extended).expect("length overflow should decode"),
        (8, 1023)
    );
    assert_eq!(
        decode_candidate_span(10, far, &extended).expect("start overflow should decode"),
        (4_194_304, 12)
    );
}

/// The one inline word the niche cannot afford to lose: the largest start packed with the largest
/// length is `0xFFFFFFFE`, which stores as `0xFFFFFFFF`. One bit further and the encoder would
/// have to reject a span the split promises to hold inline.
#[test]
fn largest_inline_start_and_length_still_pack_into_the_niche() {
    let mut extended = Vec::new();

    let extreme = encode_candidate_span(10, 4_194_303, 1022, &mut extended)
        .expect("the largest inline start and length should encode together");

    assert!(matches!(extreme, PrototypeEncodedSpan::Inline(_)));
    assert_eq!(
        decode_candidate_span(10, extreme, &extended)
            .expect("the extreme inline span should decode"),
        (4_194_303, 1022)
    );
    assert!(
        extended.is_empty(),
        "the extreme inline span must not spill into the extended table"
    );
}

/// The usable extended capacity is `2^(32 - LENGTH_BITS) - 1` entries, not `2^(32 - LENGTH_BITS)`:
/// the last index is reserved. The safety-margin figures in the architecture document are computed
/// from this number.
#[test]
fn extended_table_accepts_its_last_usable_index_and_rejects_the_next() {
    let capacity = inline_start_limit(12) - 1;
    let last_index = capacity - 1;
    let mut extended = vec![(0u32, 0u32); last_index as usize];

    let last = encode_candidate_span(12, 0, 4095, &mut extended)
        .expect("the last usable extended index should encode");

    assert!(matches!(last, PrototypeEncodedSpan::Extended(index) if index == last_index));
    assert_eq!(extended.len() as u32, capacity);

    encode_candidate_span(12, 0, 4095, &mut extended)
        .expect_err("one entry past the usable capacity must be rejected");
}

/// The census must tokenize with the same directives `moth_template::compile` registers. Measuring
/// with built-ins alone silently rejected every `$html` template - the documentation files that
/// carry the longest spans in the corpus - and a census that loses its longest spans selects the
/// wrong split. This drives `tokenize_corpus`, not the registry helper, so that the wiring is
/// covered as well as the construction.
#[test]
fn the_census_tokenizes_an_html_template_that_built_ins_reject() {
    let source = "[codeblock, $code(\"moth\"):\n    image #= [$html:\n        <img src=\"logo.svg\">\n    ]\n]\n";
    let workspace = tempfile::tempdir().expect("should create a temporary workspace");
    let template = workspace.path().join("probe.mtf");
    std::fs::write(&template, source).expect("should write the probe template");

    let tokenized = tokenize_corpus(workspace.path(), &[template])
        .expect("tokenizing a one-file corpus should not fail the census");

    assert!(
        tokenized.failures.is_empty(),
        "the census must tokenize an $html template, not report it as a failure: {:?}",
        tokenized.failures
    );
    let span_counts: Vec<usize> = tokenized
        .collected
        .iter()
        .map(|source| source.spans.len())
        .collect();

    assert_eq!(
        span_counts,
        vec![13],
        "the template must tokenize to its authored tokens, not to nothing"
    );

    tokenize_source(
        "probe.mtf",
        source,
        &mut Vec::new(),
        &StyleDirectiveRegistry::built_ins(),
    )
    .expect_err("built-ins alone must reject $html, which is why the census merges the project's");
}
