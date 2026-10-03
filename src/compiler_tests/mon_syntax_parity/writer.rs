//! Typed values written by the public MON encoder and consumed at source receivers.
//!
//! WHAT: tests `encode_document` and `encode_value` through MON decoding and real source parsing.
//! WHY: writer output is a literal fragment, not a second text format or a source evaluator.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{
    Field, Schema, SchemaType, Value, Variant, decode_document, encode_document, encode_value,
};

use super::support::{MonExpectation, SourceExpectation, assert_fixture, fixture};

#[test]
fn public_writer_values_decode_and_compile_in_compatible_source_receivers() {
    let _guard = lock_counter_test();

    let expected = Value::Record(vec![
        (
            "text".into(),
            Value::String("line\nwith \"quotes\" and \\ backslash".into()),
        ),
        (
            "ratio".into(),
            Value::Float(f64::from_bits(0x8000_0000_0000_0000)),
        ),
        (
            "items".into(),
            Value::Collection(vec![Value::Int(1), Value::Int(20)]),
        ),
    ]);
    let schema = Schema::record(vec![
        Field::required("text", SchemaType::String),
        Field::required("ratio", SchemaType::Float),
        Field::required(
            "items",
            SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            },
        ),
    ]);
    let prepared = schema
        .clone()
        .prepare()
        .expect("the writer receiver schema is valid");
    let document = encode_document(&expected, &prepared).expect("typed record encodes");
    let decoded = decode_document(&document, &prepared).expect("writer document decodes");
    assert_eq!(decoded, expected);
    let Value::Record(fields) = &decoded else {
        panic!("the MON document root is a record");
    };
    let Value::Float(ratio) = &fields[1].1 else {
        panic!("the ratio field is a binary float");
    };
    assert_eq!(
        ratio.to_bits(),
        0x8000_0000_0000_0000,
        "the writer and MON decoder preserve the negative-zero sign bit"
    );

    assert_fixture(fixture(
        "writer_document_into_source_const_record",
        document,
        "",
        schema,
        SourceExpectation::Accept,
        MonExpectation::Accept(expected),
    ));

    let map_type = SchemaType::Map {
        key: Box::new(SchemaType::String),
        value: Box::new(SchemaType::Int),
    };
    let map_value = Value::Map(vec![
        (Value::String("first".into()), Value::Int(3)),
        (Value::String("second".into()), Value::Int(8)),
    ]);
    let map_encoder_schema = Schema::value(map_type.clone())
        .prepare()
        .expect("the map value schema is valid");
    let map_fragment =
        encode_value(&map_value, &map_encoder_schema).expect("typed map value encodes");
    let map_schema = Schema::record(vec![Field::required("scores", map_type)]);
    let map_prepared = map_schema
        .clone()
        .prepare()
        .expect("the map document schema is valid");
    let map_document = format!("scores = {map_fragment}");
    let expected_document = Value::Record(vec![("scores".into(), map_value.clone())]);
    assert_eq!(
        decode_document(&map_document, &map_prepared).expect("writer map document decodes"),
        expected_document
    );

    // Maps are source expressions rather than folded const values, so observe their ordinary
    // start-body binding while MON receives the equivalent root-record document.
    assert_fixture(
        fixture(
            "writer_value_into_source_runtime_map",
            map_document,
            "",
            map_schema,
            SourceExpectation::AcceptObserved(map_value),
            MonExpectation::Accept(expected_document),
        )
        .with_source_override(format!("data = {map_fragment}\n")),
    );
}

#[test]
fn qualified_writer_input_is_checked_before_qualifier_omission() {
    let _guard = lock_counter_test();

    let point_type = SchemaType::Struct {
        name: "Point".into(),
        fields: vec![Field::required("x", SchemaType::Int)],
    };
    let point_schema = Schema::record(vec![Field::required("point", point_type)]);
    let prepared_point_schema = point_schema
        .clone()
        .prepare()
        .expect("the nominal receiver schema is valid");

    let wrong_qualifier = "point = Other(x = 1)";
    let error = decode_document(wrong_qualifier, &prepared_point_schema)
        .expect_err("a qualified MON value must match its schema before writing");
    assert_eq!(error.code, crate::mon::MonErrorCode::QualifierMismatch);
    let qualifier_start = wrong_qualifier
        .find("Other")
        .expect("wrong qualifier is present") as u32;
    assert_eq!(
        error.span,
        Some(crate::mon::Span {
            start: qualifier_start,
            end: qualifier_start + "Other".len() as u32,
        })
    );
    assert_eq!(
        error.path,
        vec![crate::mon::PathSegment::Field("point".into())]
    );

    let qualified_document = "point = Point(x = 1)";
    let decoded_point = decode_document(qualified_document, &prepared_point_schema)
        .expect("the matching qualifier is valid data");
    let emitted_point =
        encode_document(&decoded_point, &prepared_point_schema).expect("validated point encodes");
    assert!(
        !emitted_point.contains("Point"),
        "the typed writer omits an already validated struct qualifier: {emitted_point:?}"
    );
    assert_eq!(
        decode_document(&emitted_point, &prepared_point_schema)
            .expect("unqualified output is still schema-directed data"),
        decoded_point
    );

    // Source infers this emitted nested record as anonymous. It does not implicitly construct
    // `Point`, even though the MON schema used Point to validate the original qualified input.
    assert_fixture(fixture(
        "writer_qualified_struct_emits_source_anonymous_record",
        emitted_point,
        "Point = | x Int |\n",
        point_schema,
        SourceExpectation::AcceptObserved(decoded_point.clone()),
        MonExpectation::Accept(decoded_point),
    ));

    let choice_type = SchemaType::Choice {
        name: "Theme".into(),
        variants: vec![Variant::unit("Ready")],
    };
    let choice_schema = Schema::record(vec![Field::required("state", choice_type.clone())]);
    let prepared_choice_schema = choice_schema
        .clone()
        .prepare()
        .expect("the choice receiver schema is valid");
    let qualified_choice = decode_document("state = Theme::Ready", &prepared_choice_schema)
        .expect("the explicit choice qualifier is valid");
    let Value::Record(fields) = &qualified_choice else {
        panic!("the MON document root is a record");
    };
    let encoded_choice = encode_value(
        &fields[0].1,
        &Schema::value(choice_type)
            .prepare()
            .expect("the nested choice schema is valid"),
    )
    .expect("validated choice encodes");
    assert_eq!(encoded_choice, "::Ready");
    let contextual_choice_document = format!("state = {encoded_choice}");
    let expected_contextual_choice = Value::Record(vec![(
        "state".into(),
        Value::Choice {
            qualifier: None,
            variant: "Ready".into(),
            fields: Vec::new(),
        },
    )]);
    assert_eq!(
        decode_document(&contextual_choice_document, &prepared_choice_schema)
            .expect("writer contextual output remains MON data"),
        expected_contextual_choice
    );

    // gaps.rs mirrors the MON-only choice and control-text fragments as source-gap cases.

    let string_schema = Schema::record(vec![Field::required("text", SchemaType::String)]);
    let prepared_string_schema = string_schema
        .clone()
        .prepare()
        .expect("the string receiver schema is valid");
    let control_document = "text = \"\\u{1}\"";
    let decoded_control = decode_document(control_document, &prepared_string_schema)
        .expect("MON decodes its bounded Unicode escape");
    let Value::Record(fields) = &decoded_control else {
        panic!("the MON document root is a record");
    };
    let encoded_control = encode_value(
        &fields[0].1,
        &Schema::value(SchemaType::String)
            .prepare()
            .expect("the nested string schema is valid"),
    )
    .expect("typed control string encodes");
    assert_eq!(encoded_control, "\"\\u{1}\"");
    let emitted_control_document = format!("text = {encoded_control}");
    assert_eq!(
        decode_document(&emitted_control_document, &prepared_string_schema)
            .expect("writer Unicode output remains MON data"),
        decoded_control
    );
}
