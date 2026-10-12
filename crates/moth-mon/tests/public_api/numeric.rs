//! Public API coverage for explicit-width numeric behaviour.

use super::*;

#[test]
fn negative_ints_preserve_signed_i32_boundaries() {
    use moth_mon::{Field, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required("whole", SchemaType::I32)])
        .prepare()
        .expect("the I32 schema prepares");

    assert_eq!(
        decode_document("whole = -2147483648", &schema).expect("the smallest I32 is accepted"),
        Value::Record(vec![("whole".into(), Value::I32(i32::MIN))]),
    );
    assert_eq!(
        decode_document("whole = -5", &schema).expect("negative I32 values keep their sign"),
        Value::Record(vec![("whole".into(), Value::I32(-5))]),
    );
    assert_eq!(
        decode_document("whole = -2147483649", &schema)
            .expect_err("values below the I32 range are rejected")
            .code,
        MonErrorCode::NumericRange,
    );
}

#[test]
fn decimal_exponents_preserve_scale_and_exact_text() {
    use moth_mon::{Field, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required(
        "amount",
        SchemaType::Decimal { scale: 2 },
    )])
    .prepare()
    .expect("the Decimal schema prepares");
    assert_eq!(
        decode_document("amount = 1_2e-2", &schema).expect("in-scale exponent decodes"),
        Value::Record(vec![("amount".into(), Value::Decimal("12e-2".into()))]),
    );

    let reader_error = decode_document("amount = 1e-30", &schema)
        .expect_err("an exponent beyond the Decimal scale must fail");
    assert_eq!(reader_error.code, MonErrorCode::NumericScale);
}

#[test]
fn extreme_float_exponent_reports_non_finite_float() {
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required("v", SchemaType::F64)])
        .with_limits(Limits {
            max_numeric_digits: 10_000,
            ..Limits::default()
        })
        .prepare()
        .expect("the exponent schema fits its limits");

    let value = decode_document("v = 1e+21", &schema).expect("large finite exponent decodes");
    assert_eq!(value, Value::Record(vec![("v".into(), Value::F64(1e21))]));
    let error = decode_document("v = 1e10000", &schema)
        .expect_err("a non-finite exponent must fail as float range");
    assert_eq!(error.code, MonErrorCode::NonFiniteFloat);
}

#[test]
fn explicit_width_and_byte_values_round_trip_through_public_paths() {
    use moth_mon::{
        Field, Schema, SchemaType, Value, decode_document, decode_document_bytes, encode_document,
    };

    let schema = Schema::record(vec![
        Field::required("width_i8", SchemaType::I8),
        Field::required("width_i16", SchemaType::I16),
        Field::required("width_i32", SchemaType::I32),
        Field::required("width_i64", SchemaType::I64),
        Field::required("width_u8", SchemaType::U8),
        Field::required("width_u16", SchemaType::U16),
        Field::required("width_u32", SchemaType::U32),
        Field::required("width_u64", SchemaType::U64),
        Field::required("half", SchemaType::F16),
        Field::required("single", SchemaType::F32),
        Field::required("double", SchemaType::F64),
        Field::required("octet", SchemaType::Byte),
    ])
    .prepare()
    .expect("explicit-width schema prepares");

    let document = Value::Record(vec![
        ("width_i8".into(), Value::I8(i8::MIN)),
        ("width_i16".into(), Value::I16(i16::MIN)),
        ("width_i32".into(), Value::I32(i32::MIN)),
        ("width_i64".into(), Value::I64(i64::MIN)),
        ("width_u8".into(), Value::U8(u8::MAX)),
        ("width_u16".into(), Value::U16(u16::MAX)),
        ("width_u32".into(), Value::U32(u32::MAX)),
        ("width_u64".into(), Value::U64(u64::MAX)),
        ("half".into(), Value::F16(0.099_975_585_937_5)),
        ("single".into(), Value::F32(f64::from(0.1f32))),
        ("double".into(), Value::F64(-0.0)),
        ("octet".into(), Value::Byte(u8::MAX)),
    ]);
    let encoded = encode_document(&document, &schema).expect("explicit widths encode");
    assert!(encoded.contains("width_i8 = -128"), "{encoded}");
    assert!(
        encoded.contains("width_u64 = 18446744073709551615"),
        "{encoded}"
    );
    assert!(encoded.contains("half = 0.1"), "{encoded}");
    assert!(encoded.contains("single = 0.1"), "{encoded}");
    assert!(encoded.contains("double = -0.0"), "{encoded}");
    assert!(encoded.contains("octet = 255"), "{encoded}");

    let from_text = decode_document(&encoded, &schema).expect("explicit widths decode");
    let from_bytes =
        decode_document_bytes(encoded.as_bytes(), &schema).expect("bytes decode agrees");
    assert_eq!(from_text, from_bytes);
    drop(encoded);
    assert_eq!(from_text, document);
    let Value::Record(fields) = &from_text else {
        panic!("explicit-width document decodes to a record");
    };
    let double = fields
        .iter()
        .find(|(name, _)| name == "double")
        .map(|(_, value)| value)
        .expect("double field");
    assert!(
        matches!(double, Value::F64(number) if number.to_bits() == (-0.0f64).to_bits()),
        "F64 negative zero keeps its sign bit",
    );
}

#[test]
fn error_code_u32_round_trips_through_public_paths() {
    let schema = Schema::record(vec![
        Field::required("message", SchemaType::String),
        Field::with_default("code", SchemaType::U32, Value::U32(0)),
    ])
    .prepare()
    .expect("runtime Error field schema prepares");

    for code in [0, i32::MAX as u32 + 1, u32::MAX] {
        let source = format!("message = \"wide code\", code = {code}");
        let decoded = decode_document(&source, &schema).expect("U32 code decodes");
        drop(source);
        let expected = Value::Record(vec![
            ("message".into(), Value::String("wide code".into())),
            ("code".into(), Value::U32(code)),
        ]);
        assert_eq!(decoded, expected, "code={code}");

        let encoded = encode_document(&decoded, &schema).expect("U32 code encodes");
        let round_trip =
            decode_document_bytes(encoded.as_bytes(), &schema).expect("U32 bytes decode");
        drop(encoded);
        drop(decoded);
        assert_eq!(round_trip, expected, "code={code}");
    }

    assert_eq!(
        decode_document("message = \"default\"", &schema).expect("omitted code completes"),
        Value::Record(vec![
            ("message".into(), Value::String("default".into())),
            ("code".into(), Value::U32(0)),
        ]),
    );
    for code in ["-1", "4294967296"] {
        let source = format!("message = \"invalid\", code = {code}");
        let error = decode_document(&source, &schema).expect_err("code must fit U32");
        assert_eq!(error.code, MonErrorCode::NumericRange, "{code}");
        assert_eq!(error.path, vec![PathSegment::Field("code".into())]);
    }
}

#[test]
fn f16_edge_values_round_trip_bits_through_public_paths() {
    use moth_mon::{Field, Schema, SchemaType, Value, decode_document, encode_document};

    // (label, exact `f64` carrier `Value::F16` stores); each label names the canonical binary16
    // encoding the carrier must keep exact through the MON round trip.
    let edges = [
        (
            "positive zero (0x0000)",
            f64::from_bits(0x0000_0000_0000_0000),
        ),
        (
            "negative zero (0x8000)",
            f64::from_bits(0x8000_0000_0000_0000),
        ),
        (
            "smallest positive subnormal (0x0001)",
            f64::from_bits(0x3E70_0000_0000_0000),
        ),
        (
            "smallest negative subnormal (0x8001)",
            f64::from_bits(0xBE70_0000_0000_0000),
        ),
        (
            "largest positive subnormal (0x03FF)",
            f64::from_bits(0x3F0F_F800_0000_0000),
        ),
        (
            "largest negative subnormal (0x83FF)",
            f64::from_bits(0xBF0F_F800_0000_0000),
        ),
        (
            "smallest positive normal (0x0400)",
            f64::from_bits(0x3F10_0000_0000_0000),
        ),
        (
            "smallest negative normal (0x8400)",
            f64::from_bits(0xBF10_0000_0000_0000),
        ),
    ];

    let schema = Schema::record(vec![Field::required("half", SchemaType::F16)])
        .prepare()
        .expect("F16 schema prepares");

    for (name, carrier) in edges {
        let document = Value::Record(vec![("half".into(), Value::F16(carrier))]);
        let encoded = encode_document(&document, &schema)
            .unwrap_or_else(|error| panic!("{name} encodes: {error:?}"));
        let decoded = decode_document(&encoded, &schema)
            .unwrap_or_else(|error| panic!("{name} decodes: {error:?}"));
        drop(encoded);

        let Value::Record(fields) = &decoded else {
            panic!("{name} decodes to a record");
        };
        let Some((_, Value::F16(number))) = fields.iter().find(|(field, _)| field == "half") else {
            panic!("{name} decodes an F16 half field");
        };

        // Equality cannot assert signed zero, so the carrier's own bits are the assertion.
        assert_eq!(
            number.to_bits(),
            carrier.to_bits(),
            "{name}: the decoded F16 keeps the encoded bits",
        );
    }

    // The default path applies the signed zero without any literal, which only a bit
    // assertion can check.
    let zero_default_schema = Schema::record(vec![Field::with_default(
        "half",
        SchemaType::F16,
        Value::F16(-0.0),
    )])
    .prepare()
    .expect("the negative zero F16 default prepares");
    let expanded = decode_document("", &zero_default_schema).expect("the F16 default expands");
    let Value::Record(fields) = &expanded else {
        panic!("the default expansion is a record");
    };
    let Some((_, Value::F16(number))) = fields.iter().find(|(field, _)| field == "half") else {
        panic!("the default expansion has an F16 half field");
    };
    assert_eq!(
        number.to_bits(),
        (-0.0f64).to_bits(),
        "the F16 default keeps its sign bit",
    );
}

#[test]
fn signed_bounds_bind_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, Schema, SchemaType, Value, decode_document, encode_document,
    };

    type Carrier = fn(i64) -> Value;

    // Each width pairs its schema type with its bounds, its carrier and the spellings one step
    // outside either bound.
    let widths: [(&str, SchemaType, i64, i64, Carrier, &str, &str); 2] = [
        (
            "I32",
            SchemaType::I32,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
            |value: i64| Value::I32(i32::try_from(value).expect("I32 bounds fit")),
            "2147483648",
            "-2147483649",
        ),
        (
            "I64",
            SchemaType::I64,
            i64::MIN,
            i64::MAX,
            Value::I64,
            "9223372036854775808",
            "-9223372036854775809",
        ),
    ];

    for (name, schema_type, min, max, carrier, above_max, below_min) in widths {
        let schema = Schema::record(vec![Field::required("whole", schema_type)])
            .prepare()
            .expect("the signed schema prepares");
        for bound in [min, max] {
            let source = format!("whole = {bound}");
            let expected = Value::Record(vec![("whole".into(), carrier(bound))]);
            assert_eq!(
                decode_document(&source, &schema),
                Ok(expected.clone()),
                "{name}, {bound}"
            );
            let encoded = encode_document(&expected, &schema).expect("bound encodes");
            assert_eq!(
                decode_document(&encoded, &schema),
                Ok(expected),
                "{name}, {bound}"
            );
        }
        // One step outside either bound fails with the range code.
        for outside in [above_max, below_min] {
            let error = decode_document(&format!("whole = {outside}"), &schema)
                .expect_err("outside the bounds rejects");
            assert_eq!(error.code, MonErrorCode::NumericRange, "{name}, {outside}");
        }
    }

    // A carrier of another width is a family mismatch, not a range error.
    let narrow = Schema::record(vec![Field::required("whole", SchemaType::I32)])
        .prepare()
        .expect("the I32 schema prepares");
    let wide = Value::Record(vec![("whole".into(), Value::I64(i64::from(i32::MAX) + 1))]);
    assert_eq!(
        encode_document(&wide, &narrow)
            .expect_err("an I64 carrier rejects in an I32 field")
            .code,
        MonErrorCode::TypeMismatch,
    );
}

#[test]
fn float_widths_bind_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, Schema, SchemaType, Value, decode_document, encode_document,
    };

    type Carrier = fn(f64) -> Value;

    // F32 rounds `0.1` to single precision; F64 keeps the double value.
    let widths: [(&str, SchemaType, f64, Carrier); 2] = [
        ("F32", SchemaType::F32, f64::from(0.1f32), Value::F32),
        ("F64", SchemaType::F64, 0.1, Value::F64),
    ];
    for (name, schema_type, rounded, carrier) in widths {
        let schema = Schema::record(vec![Field::required("value", schema_type)])
            .prepare()
            .expect("the float schema prepares");
        let expected = Value::Record(vec![("value".into(), carrier(rounded))]);
        assert_eq!(
            decode_document("value = 0.1", &schema),
            Ok(expected.clone()),
            "{name}"
        );
        let encoded = encode_document(&expected, &schema).expect("float value encodes");
        assert_eq!(decode_document(&encoded, &schema), Ok(expected), "{name}");
    }

    // A finite source beyond the F32 range fails there, while F64 keeps it as a finite value.
    let single = Schema::record(vec![Field::required("value", SchemaType::F32)])
        .prepare()
        .expect("the F32 schema prepares");
    assert_eq!(
        decode_document("value = 1.0e40", &single)
            .expect_err("beyond F32 fails")
            .code,
        MonErrorCode::NonFiniteFloat,
    );
    let double = Schema::record(vec![Field::required("value", SchemaType::F64)])
        .prepare()
        .expect("the F64 schema prepares");
    assert_eq!(
        decode_document("value = 1.0e40", &double),
        Ok(Value::Record(vec![("value".into(), Value::F64(1.0e40))])),
    );
}

#[test]
fn map_key_families_preserve_order_and_reject_duplicates_and_mismatches() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    let cases = [
        (
            SchemaType::String,
            [Value::String("b".into()), Value::String("a".into())],
            r#"{"b" = 20, "a" = 10}"#,
            r#"{"b" = 20, "\u{62}" = 10}"#,
            "b",
        ),
        (
            SchemaType::Bool,
            [Value::Bool(true), Value::Bool(false)],
            "{true = 20, false = 10}",
            "{true = 20, true = 10}",
            "true",
        ),
        (
            SchemaType::Char,
            [Value::Char('b'), Value::Char('a')],
            "{'b' = 20, 'a' = 10}",
            r#"{'b' = 20, '\u{62}' = 10}"#,
            "b",
        ),
        (
            SchemaType::I8,
            [Value::I8(2), Value::I8(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::I16,
            [Value::I16(2), Value::I16(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::I32,
            [Value::I32(2), Value::I32(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::I64,
            [Value::I64(i64::MAX), Value::I64(i64::MIN)],
            "{9223372036854775807 = 20, -9223372036854775808 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::U8,
            [Value::U8(2), Value::U8(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::U16,
            [Value::U16(2), Value::U16(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::U32,
            [Value::U32(2), Value::U32(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::U64,
            [Value::U64(u64::MAX), Value::U64(1)],
            "{18446744073709551615 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::Byte,
            [Value::Byte(255), Value::Byte(0)],
            "{255 = 20, 0 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
    ];

    for (key_type, keys, literal, duplicate_literal, duplicate_name) in &cases {
        let map_type = SchemaType::Map {
            key: Box::new(key_type.clone()),
            value: Box::new(SchemaType::I32),
        };
        let map = Value::Map(vec![
            (keys[0].clone(), Value::I32(20)),
            (keys[1].clone(), Value::I32(10)),
        ]);
        let expected = Value::Record(vec![("values".into(), map.clone())]);
        let schema = Schema::record(vec![Field::required("values", map_type.clone())])
            .prepare()
            .expect("supported map key family prepares");
        assert_eq!(
            decode_document(&format!("values = {literal}"), &schema).unwrap(),
            expected,
            "{key_type:?}",
        );
        let encoded = encode_document(&expected, &schema).unwrap();
        assert_eq!(
            decode_document(&encoded, &schema).unwrap(),
            expected,
            "{key_type:?}",
        );

        let default_schema =
            Schema::record(vec![Field::with_default("values", map_type.clone(), map)])
                .prepare()
                .expect("ordered map default prepares");
        assert_eq!(decode_document("()", &default_schema).unwrap(), expected);
        let encoded_default = encode_document(&Value::Record(Vec::new()), &default_schema).unwrap();
        assert_eq!(
            decode_document(&encoded_default, &schema).unwrap(),
            expected
        );

        let duplicate = decode_document(&format!("values = {duplicate_literal}"), &schema)
            .expect_err("decoded-equivalent keys are duplicates");
        assert_eq!(
            duplicate.code,
            MonErrorCode::DuplicateMapKey,
            "{key_type:?}"
        );
        assert_eq!(
            duplicate.path,
            vec![
                PathSegment::Field("values".into()),
                PathSegment::MapKey((*duplicate_name).into()),
            ],
        );

        let duplicate_map = Value::Map(vec![
            (keys[0].clone(), Value::I32(20)),
            (keys[0].clone(), Value::I32(10)),
        ]);
        let duplicate_value = Value::Record(vec![("values".into(), duplicate_map.clone())]);
        assert_eq!(
            encode_document(&duplicate_value, &schema).unwrap_err().code,
            MonErrorCode::DuplicateMapKey,
            "{key_type:?}",
        );
        assert_eq!(
            Schema::record(vec![Field::with_default(
                "values",
                map_type.clone(),
                duplicate_map,
            )])
            .prepare()
            .unwrap_err()
            .code,
            MonErrorCode::InvalidDefault,
            "{key_type:?}",
        );

        // The second key is otherwise valid data, but it belongs to another exact family.
        // In particular, equal integer carriers must fail typing rather than alias a key.
        for (other_type, other_keys, ..) in &cases {
            if other_type == key_type {
                continue;
            }
            let mixed_map = Value::Map(vec![
                (keys[1].clone(), Value::I32(20)),
                (other_keys[1].clone(), Value::I32(10)),
            ]);
            let mixed = Value::Record(vec![("values".into(), mixed_map.clone())]);
            let error = encode_document(&mixed, &schema)
                .expect_err("a map rejects a key from another family");
            assert_eq!(
                error.code,
                MonErrorCode::TypeMismatch,
                "{key_type:?}/{other_type:?}",
            );
            assert_eq!(
                error.path,
                vec![PathSegment::Field("values".into()), PathSegment::Index(1)],
            );
            let default_error = Schema::record(vec![Field::with_default(
                "values",
                map_type.clone(),
                mixed_map,
            )])
            .prepare()
            .expect_err("a map default rejects a key from another family");
            assert_eq!(default_error.code, MonErrorCode::InvalidDefault);
            assert_eq!(default_error.path, error.path);
        }
    }

    let byte_schema = Schema::record(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Byte),
            value: Box::new(SchemaType::I32),
        },
    )])
    .prepare()
    .unwrap();
    assert_eq!(
        decode_document("values = {256 = 1}", &byte_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange,
    );
    let float_key_error = Schema::value(SchemaType::Map {
        key: Box::new(SchemaType::F32),
        value: Box::new(SchemaType::I32),
    })
    .prepare()
    .unwrap_err();
    assert_eq!(float_key_error.code, MonErrorCode::InvalidMapKey);
}

#[test]
fn decimal_scale_capacity_reaches_256_through_public_paths() {
    use moth_mon::{Field, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let wide = Schema::record(vec![Field::required(
        "value",
        SchemaType::Decimal { scale: 256 },
    )])
    .prepare()
    .expect("the widened scale capacity prepares");
    assert_eq!(
        decode_document("value = 1e-256", &wide),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Decimal("1e-256".into())
        )])),
    );
    assert_eq!(
        decode_document("value = 1e-257", &wide)
            .expect_err("above scale 256 fails")
            .code,
        MonErrorCode::NumericScale,
    );
    let too_wide = Schema::value(SchemaType::Decimal { scale: 257 })
        .prepare()
        .expect_err("a scale above the capacity fails");
    assert_eq!(too_wide.code, MonErrorCode::NumericScale);

    let nineteen = Schema::record(vec![Field::required(
        "value",
        SchemaType::Decimal { scale: 19 },
    )])
    .prepare()
    .expect("a scale inside the former ceiling prepares");
    assert_eq!(
        decode_document("value = 1e-19", &nineteen),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Decimal("1e-19".into())
        )])),
    );
    assert_eq!(
        decode_document("value = 1e-20", &nineteen)
            .expect_err("above scale 19 fails")
            .code,
        MonErrorCode::NumericScale,
    );
}

#[test]
fn exact_scale_edges_preserve_text_and_reject_above_scale() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Span, Value, decode_document,
        encode_value,
    };

    let huge_exponent = "9".repeat(26);
    let huge_zero = format!("-0.000e-{huge_exponent}");
    let huge_nonzero = format!("1e-{huge_exponent}");
    for (scale, text, fits) in [
        (0, "1e+2", true),
        (0, "1.0e0", true),
        (0, "1.5", false),
        (1, "0.10", true),
        (1, "100e-3", true),
        (1, "-10.5e-1", false),
        (256, "10e-257", true),
        (256, "11e-257", false),
        (0, huge_zero.as_str(), true),
        (0, huge_nonzero.as_str(), false),
    ] {
        let document_schema =
            Schema::record(vec![Field::required("v", SchemaType::Decimal { scale })])
                .prepare()
                .expect("a supported scale prepares");
        let scalar_schema = Schema::value(SchemaType::Decimal { scale })
            .prepare()
            .expect("a supported scalar scale prepares");
        let decoded = decode_document(&format!("v = {text}"), &document_schema);
        let encoded = encode_value(&Value::Decimal(text.into()), &scalar_schema);
        if fits {
            assert_eq!(
                decoded,
                Ok(Value::Record(vec![(
                    "v".into(),
                    Value::Decimal(text.into())
                )])),
                "scale {scale}, {text}",
            );
            assert_eq!(encoded, Ok(text.into()), "scale {scale}, {text}");
        } else {
            let read_error = decoded.expect_err("inexact decimal input rejects");
            assert_eq!(read_error.code, MonErrorCode::NumericScale);
            assert_eq!(read_error.path, vec![PathSegment::Field("v".into())]);
            assert_eq!(
                read_error.span,
                Some(Span {
                    start: 4,
                    end: 4 + text.len() as u32,
                }),
            );
            let write_error = encoded.expect_err("inexact programmatic input rejects");
            assert_eq!(write_error.code, MonErrorCode::NumericScale);
            assert_eq!(write_error.span, None);
        }
    }

    // Defaults use the same exact-fit rule, including the zero/exponent special case.
    let defaults = Schema::record(vec![Field::with_default(
        "v",
        SchemaType::Decimal { scale: 0 },
        Value::Decimal(huge_zero.clone()),
    )])
    .prepare()
    .expect("exact scale-zero default prepares");
    assert_eq!(
        decode_document("", &defaults),
        Ok(Value::Record(vec![("v".into(), Value::Decimal(huge_zero))])),
    );
    let invalid_default = Schema::record(vec![Field::with_default(
        "v",
        SchemaType::Decimal { scale: 0 },
        Value::Decimal("1e-1".into()),
    )])
    .prepare()
    .expect_err("inexact default rejects during preparation");
    assert_eq!(invalid_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(invalid_default.path, vec![PathSegment::Field("v".into())]);
    assert_eq!(invalid_default.span, None);
}

#[test]
fn widened_scale_large_exact_values_round_trip_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    // Both the 63-digit integer and the final decimal digit exceed a binary carrier's precision.
    let magnitude = "123456789".repeat(7);
    let decimal = format!("-{magnitude}.{}1", "0".repeat(255));
    let schema = Schema::record(vec![
        Field::required("count", SchemaType::Integer),
        Field::required("total", SchemaType::Decimal { scale: 256 }),
    ])
    .prepare()
    .expect("large exact schema prepares");
    let document = Value::Record(vec![
        ("count".into(), Value::Integer(format!("-{magnitude}"))),
        ("total".into(), Value::Decimal(decimal.clone())),
    ]);
    assert_eq!(
        decode_document(&format!("count = -{magnitude}, total = {decimal}"), &schema),
        Ok(document.clone()),
    );
    let encoded = encode_document(&document, &schema).expect("large exact data encodes");
    assert_eq!(decode_document(&encoded, &schema), Ok(document));

    let oversized = encode_document(
        &Value::Record(vec![
            ("count".into(), Value::Integer("7".into())),
            ("total".into(), Value::Decimal(format!("{decimal}1"))),
        ]),
        &schema,
    )
    .expect_err("nonzero digit beyond scale 256 rejects");
    assert_eq!(oversized.code, MonErrorCode::NumericScale);
    assert_eq!(oversized.path, vec![PathSegment::Field("total".into())]);
    assert_eq!(oversized.span, None);
}

#[test]
fn fixed_and_byte_programmatic_defaults_validate_through_public_paths() {
    use moth_mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![
        Field::with_default("code", SchemaType::I8, Value::I8(i8::MIN)),
        Field::with_default("octet", SchemaType::Byte, Value::Byte(255)),
    ])
    .prepare()
    .expect("matching fixed defaults prepare");
    assert_eq!(
        decode_document("", &schema).expect("fixed defaults apply"),
        Value::Record(vec![
            ("code".into(), Value::I8(i8::MIN)),
            ("octet".into(), Value::Byte(255)),
        ]),
    );

    let mismatched = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I8,
        Value::I16(1),
    )])
    .prepare()
    .expect_err("a fixed default must match its own width");
    assert_eq!(mismatched.code, MonErrorCode::InvalidDefault);

    let duplicate_byte_keys = Schema::record(vec![Field::with_default(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Byte),
            value: Box::new(SchemaType::I32),
        },
        Value::Map(vec![
            (Value::Byte(1), Value::I32(2)),
            (Value::Byte(1), Value::I32(3)),
        ]),
    )])
    .prepare()
    .expect_err("duplicate Byte keys in a default fail");
    assert_eq!(duplicate_byte_keys.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        duplicate_byte_keys.path,
        vec![
            PathSegment::Field("values".into()),
            PathSegment::MapKey("1".into()),
        ],
    );

    let above_int32 = i64::from(i32::MAX) + 1;
    let in_range = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I64,
        Value::I64(above_int32),
    )])
    .prepare()
    .expect("an I64 default above the I32 range prepares");
    assert_eq!(
        decode_document("", &in_range).expect("the I64 default applies"),
        Value::Record(vec![("value".into(), Value::I64(above_int32))]),
    );

    let above_float32 = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::F32,
        Value::F32(1.0e300),
    )])
    .prepare()
    .expect_err("an F32 default outside single precision fails");
    assert_eq!(above_float32.code, MonErrorCode::InvalidDefault);
}

#[test]
fn explicit_width_numerics_preserve_budgets_and_categories() {
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let digit_limited = Schema::record(vec![Field::required("value", SchemaType::U64)])
        .with_limits(Limits {
            max_numeric_digits: 2,
            ..Limits::default()
        })
        .prepare()
        .expect("digit budget schema prepares");
    assert_eq!(
        decode_document("value = 18446744073709551615", &digit_limited)
            .expect_err("long fixed literals hit the digit budget")
            .code,
        MonErrorCode::NumericBudget,
    );

    let byte_limited = Schema::record(vec![Field::required("v", SchemaType::F16)])
        .with_limits(Limits {
            max_decoded_bytes: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("decoded budget schema prepares");
    assert_eq!(
        decode_document("v = 0.1", &byte_limited)
            .expect_err("fixed-float scratch hits the decoded budget")
            .code,
        MonErrorCode::DecodedBudget,
    );

    let key_limited = Schema::record(vec![Field::required(
        "v",
        SchemaType::Map {
            key: Box::new(SchemaType::U64),
            value: Box::new(SchemaType::Byte),
        },
    )])
    .with_limits(Limits {
        max_decoded_bytes: 2,
        ..Limits::default()
    })
    .prepare()
    .expect("key budget schema prepares");
    assert_eq!(
        decode_document("v = {18446744073709551615 = 1}", &key_limited)
            .expect_err("fixed-key scratch hits the decoded budget")
            .code,
        MonErrorCode::DecodedBudget,
    );

    let fixed = Schema::record(vec![Field::required("value", SchemaType::I8)])
        .prepare()
        .expect("fixed schema prepares");
    assert_eq!(
        decode_document("value = 128", &fixed)
            .expect_err("above I8 fails")
            .code,
        MonErrorCode::NumericRange,
    );
    assert_eq!(
        decode_document("value = 3.0", &fixed)
            .expect_err("decimal spelling fails")
            .code,
        MonErrorCode::NumericType,
    );
    assert_eq!(
        decode_document("value = 1e3", &fixed)
            .expect_err("exponent spelling fails")
            .code,
        MonErrorCode::NumericType,
    );

    let octet = Schema::record(vec![Field::required("value", SchemaType::Byte)])
        .prepare()
        .expect("Byte schema prepares");
    assert_eq!(
        decode_document("value = 256", &octet)
            .expect_err("above Byte fails")
            .code,
        MonErrorCode::NumericRange,
    );
    assert_eq!(
        decode_document("value = 2.5", &octet)
            .expect_err("decimal Byte fails")
            .code,
        MonErrorCode::NumericType,
    );
}

/// Outside-crate end-to-end example using only `moth_mon` public paths.
///
/// A host crate builds an explicit schema with fixed-width, `Byte` and
/// widened-scale members, then round-trips owned values through the public
/// encode/decode adapters.
fn telemetry_round_trip() -> Value {
    use moth_mon::{
        Field, Schema, SchemaType, Value, decode_document, decode_document_bytes, encode_document,
    };

    let schema = Schema::record(vec![
        Field::required("seq", SchemaType::U64),
        Field::required("sample", SchemaType::F32),
        Field::required("half", SchemaType::F16),
        Field::required(
            "flags",
            SchemaType::Map {
                key: Box::new(SchemaType::Byte),
                value: Box::new(SchemaType::Bool),
            },
        ),
        Field::required("total", SchemaType::Decimal { scale: 256 }),
    ])
    .prepare()
    .expect("example schema prepares");

    let document = Value::Record(vec![
        ("seq".into(), Value::U64(u64::MAX)),
        ("sample".into(), Value::F32(f64::from(0.1f32))),
        ("half".into(), Value::F16(0.099_975_585_937_5)),
        (
            "flags".into(),
            Value::Map(vec![
                (Value::Byte(1), Value::Bool(true)),
                (Value::Byte(2), Value::Bool(false)),
            ]),
        ),
        ("total".into(), Value::Decimal("1e-256".into())),
    ]);
    let encoded = encode_document(&document, &schema).expect("example document encodes");
    assert!(encoded.contains("seq = 18446744073709551615"), "{encoded}");
    let from_text = decode_document(&encoded, &schema).expect("example document decodes");
    let from_bytes =
        decode_document_bytes(encoded.as_bytes(), &schema).expect("bytes decode agrees");
    assert_eq!(from_text, from_bytes);
    drop(encoded);
    assert_eq!(from_text, document);
    from_text
}

#[test]
fn outside_crate_telemetry_round_trip_example() {
    let value = telemetry_round_trip();
    let Value::Record(fields) = value else {
        panic!("example produces a record");
    };
    assert!(
        fields
            .iter()
            .any(|(name, value)| { name == "seq" && *value == Value::U64(u64::MAX) })
    );
}

#[test]
fn unsigned_widths_bind_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Span, Value, decode_document,
        decode_document_bytes, encode_document,
    };

    type Carrier = fn(u64) -> Value;
    type Spellings = &'static [&'static str];

    // Each width pairs its schema type with its maximum, its carrier, the spellings it accepts
    // and the spellings it rejects for range.
    let widths: [(&str, SchemaType, u64, Carrier, Spellings, Spellings); 2] = [
        (
            "U32",
            SchemaType::U32,
            u64::from(u32::MAX),
            |value: u64| Value::U32(u32::try_from(value).expect("U32 spellings fit")),
            &["0", "4_294_967_295", "2147483648", "4294967295"],
            &["4294967296", "9223372036854775808", "18446744073709551615"],
        ),
        (
            "U64",
            SchemaType::U64,
            u64::MAX,
            Value::U64,
            &[
                "0",
                "4_294_967_295",
                "2147483648",
                "18446744073709551615",
                "9007199254740993",
                "9223372036854775807",
                "9223372036854775808",
            ],
            &["18446744073709551616"],
        ),
    ];

    for (name, schema_type, maximum, carrier, accepted, rejected) in widths {
        let schema = Schema::record(vec![Field::required("count", schema_type)])
            .prepare()
            .expect("the unsigned width schema prepares");

        // Zero, separators, signed-max-plus-one and the width maximum decode exactly.
        for spelling in accepted {
            let source = format!("count = {spelling}");
            let decoded = decode_document(&source, &schema).expect("in-range unsigned decodes");
            drop(source);
            let expected = Value::Record(vec![(
                "count".into(),
                carrier(
                    spelling
                        .replace('_', "")
                        .parse::<u64>()
                        .expect("accepted spelling fits u64"),
                ),
            )]);
            assert_eq!(decoded, expected, "{name}, {spelling}");
            let encoded = encode_document(&decoded, &schema).expect("unsigned encodes");
            let round_trip =
                decode_document_bytes(encoded.as_bytes(), &schema).expect("unsigned bytes decode");
            drop(encoded);
            drop(decoded);
            assert_eq!(round_trip, expected, "{name}, {spelling}");
        }

        // Text overflow past the width maximum rejects with range, path and span.
        for spelling in rejected {
            let source = format!("count = {spelling}");
            let error = decode_document(&source, &schema).expect_err("unsigned overflow rejects");
            assert_eq!(error.code, MonErrorCode::NumericRange, "{name}, {spelling}");
            assert_eq!(
                error.path,
                vec![PathSegment::Field("count".into())],
                "{name}, {spelling}"
            );
            assert_eq!(
                error.span,
                Some(Span {
                    start: 8,
                    end: 8 + spelling.len() as u32,
                }),
                "{name}, {spelling}"
            );
        }

        // Negative spellings, including `-0`, and non-whole spellings reject.
        for spelling in ["-1", "-0", "1.0", "1e2"] {
            let source = format!("count = {spelling}");
            let error =
                decode_document(&source, &schema).expect_err("non-unsigned spelling rejects");
            let expected_code = if spelling.starts_with('-') {
                MonErrorCode::NumericRange
            } else {
                MonErrorCode::NumericType
            };
            assert_eq!(error.code, expected_code, "{name}, {spelling}");
            assert_eq!(
                error.path,
                vec![PathSegment::Field("count".into())],
                "{name}, {spelling}"
            );
            assert_eq!(
                error.span,
                Some(Span {
                    start: 8,
                    end: 8 + spelling.len() as u32,
                }),
                "{name}, {spelling}"
            );
        }

        // Programmatic values at the width maximum validate and round trip.
        let supplied = Value::Record(vec![("count".into(), carrier(maximum))]);
        assert_eq!(
            encode_document(&supplied, &schema).and_then(|text| decode_document(&text, &schema)),
            Ok(supplied.clone()),
            "{name}",
        );
    }

    // A U64 value in a U32 field is a family mismatch, not a range error.
    let narrow = Schema::record(vec![Field::required("count", SchemaType::U32)])
        .prepare()
        .expect("the U32 schema prepares");
    let wide = Value::Record(vec![("count".into(), Value::U64(u64::from(u32::MAX) + 1))]);
    let error = encode_document(&wide, &narrow).expect_err("a U64 value rejects in a U32 field");
    assert_eq!(error.code, MonErrorCode::TypeMismatch);
    assert_eq!(error.path, vec![PathSegment::Field("count".into())]);
}

#[test]
fn uint_keeps_strict_category_identity_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    let schema = Schema::record(vec![
        Field::required("signed", SchemaType::I32),
        Field::required("fixed", SchemaType::U64),
        Field::required("narrow", SchemaType::U32),
    ])
    .prepare()
    .expect("mixed integer schema prepares");
    let document = Value::Record(vec![
        ("signed".into(), Value::I32(2_147_483_647)),
        ("fixed".into(), Value::U64(u64::MAX)),
        ("narrow".into(), Value::U32(u32::MAX)),
    ]);
    let encoded = encode_document(&document, &schema).expect("distinct integer values encode");
    assert_eq!(decode_document(&encoded, &schema), Ok(document));

    // Equal carriers never alias another integer family, in either direction.
    let families: [(SchemaType, Value); 3] = [
        (SchemaType::I32, Value::I32(7)),
        (SchemaType::U32, Value::U32(7)),
        (SchemaType::U64, Value::U64(7)),
    ];
    for (index, (own_type, own_value)) in families.iter().enumerate() {
        let own_schema = Schema::record(vec![Field::required("value", own_type.clone())])
            .prepare()
            .expect("integer schema prepares");
        let own_document = Value::Record(vec![("value".into(), own_value.clone())]);
        assert_eq!(
            encode_document(&own_document, &own_schema)
                .and_then(|text| decode_document(&text, &own_schema)),
            Ok(own_document),
            "{own_type:?}",
        );
        for (other_index, (other_type, other_value)) in families.iter().enumerate() {
            if other_index == index {
                continue;
            }
            let crossed = Value::Record(vec![("value".into(), other_value.clone())]);
            let error =
                encode_document(&crossed, &own_schema).expect_err("integer families never alias");
            assert_eq!(
                error.code,
                MonErrorCode::TypeMismatch,
                "{own_type:?}/{other_type:?}"
            );
            assert_eq!(
                error.path,
                vec![PathSegment::Field("value".into())],
                "{own_type:?}/{other_type:?}"
            );
            let default_error = Schema::record(vec![Field::with_default(
                "value",
                own_type.clone(),
                other_value.clone(),
            )])
            .prepare()
            .expect_err("integer defaults never alias");
            assert_eq!(default_error.code, MonErrorCode::InvalidDefault);
        }
    }

    // Explicit U32 and U64 fields keep their own maxima.
    let explicit = Schema::record(vec![
        Field::required("narrow", SchemaType::U32),
        Field::required("wide", SchemaType::U64),
    ])
    .prepare()
    .expect("explicit widths prepare");
    let source = "narrow = 4294967295, wide = 18446744073709551615";
    assert_eq!(
        decode_document(source, &explicit),
        Ok(Value::Record(vec![
            ("narrow".into(), Value::U32(u32::MAX)),
            ("wide".into(), Value::U64(u64::MAX)),
        ])),
    );
}

#[test]
fn uint_defaults_optionals_nesting_and_map_keys_round_trip_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    let schema = Schema::record(vec![
        Field::with_default("total", SchemaType::U32, Value::U32(2_147_483_648)),
        Field::required("maybe", SchemaType::Optional(Box::new(SchemaType::U32))),
        Field::required(
            "pair",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::U32)],
            },
        ),
        Field::required(
            "counts",
            SchemaType::Map {
                key: Box::new(SchemaType::U32),
                value: Box::new(SchemaType::U32),
            },
        ),
        Field::required(
            "batch",
            SchemaType::Collection {
                element: Box::new(SchemaType::U32),
            },
        ),
    ])
    .prepare()
    .expect("the U32 default/optional/nested/map schema prepares");
    let expected = Value::Record(vec![
        ("total".into(), Value::U64(2_147_483_648)),
        ("maybe".into(), Value::None),
        (
            "pair".into(),
            Value::Record(vec![("inner".into(), Value::U64(0))]),
        ),
        (
            "counts".into(),
            Value::Map(vec![
                (Value::U64(u64::MAX), Value::U64(1)),
                (Value::U64(0), Value::U64(2)),
            ]),
        ),
        ("batch".into(), Value::Collection(vec![Value::U64(3)])),
    ]);
    // The U32 schema rejects the `u64::MAX` map key, so this decode fails before the U64 schema
    // below. The `2147483648` default fits U32, so preparation already accepted it.
    let standard_error = decode_document(
        "maybe = none, pair = (inner = 0), counts = {18446744073709551615 = 1, 0 = 2}, batch = {3}",
        &schema,
    )
    .expect_err("U64-scale keys reject under the U32 schema");
    assert_eq!(standard_error.code, MonErrorCode::NumericRange);

    let wide = Schema::record(vec![
        Field::with_default("total", SchemaType::U64, Value::U64(2_147_483_648)),
        Field::required("maybe", SchemaType::Optional(Box::new(SchemaType::U64))),
        Field::required(
            "pair",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::U64)],
            },
        ),
        Field::required(
            "counts",
            SchemaType::Map {
                key: Box::new(SchemaType::U64),
                value: Box::new(SchemaType::U64),
            },
        ),
        Field::required(
            "batch",
            SchemaType::Collection {
                element: Box::new(SchemaType::U64),
            },
        ),
    ])
    .prepare()
    .expect("the U64 default/optional/nested/map schema prepares");
    let decoded = decode_document(
        "maybe = none, pair = (inner = 0), counts = {18446744073709551615 = 1, 0 = 2}, batch = {3}",
        &wide,
    )
    .expect("U64-scale document decodes");
    assert_eq!(decoded, expected);
    let encoded = encode_document(&decoded, &wide).expect("U64 document encodes");
    assert_eq!(decode_document(&encoded, &wide), Ok(expected.clone()));
    // The same prepared schema decodes again unchanged.
    assert_eq!(
        decode_document(
            "total = 1, maybe = 2, pair = (inner = 3), counts = {=}, batch = {}",
            &wide
        ),
        Ok(Value::Record(vec![
            ("total".into(), Value::U64(1)),
            ("maybe".into(), Value::U64(2)),
            (
                "pair".into(),
                Value::Record(vec![("inner".into(), Value::U64(3))]),
            ),
            ("counts".into(), Value::Map(vec![])),
            ("batch".into(), Value::Collection(vec![])),
        ])),
    );
    // An omitted field completes from its prepared U64 default.
    assert_eq!(
        decode_document(
            "maybe = 2, pair = (inner = 3), counts = {=}, batch = {}",
            &wide
        ),
        Ok(Value::Record(vec![
            ("total".into(), Value::U64(2_147_483_648)),
            ("maybe".into(), Value::U64(2)),
            (
                "pair".into(),
                Value::Record(vec![("inner".into(), Value::U64(3))]),
            ),
            ("counts".into(), Value::Map(vec![])),
            ("batch".into(), Value::Collection(vec![])),
        ])),
    );

    // Duplicate U64 keys report the decoded key path.
    let duplicate = decode_document(
        "maybe = none, pair = (inner = 0), counts = {7 = 1, 0_7 = 2}, batch = {3}",
        &wide,
    )
    .expect_err("decoded-equivalent U64 keys are duplicates");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        duplicate.path,
        vec![
            PathSegment::Field("counts".into()),
            PathSegment::MapKey("7".into()),
        ],
    );

    // A U64 default in a U32 field is a family mismatch, so it fails during preparation.
    let invalid_default = Schema::record(vec![Field::with_default(
        "total",
        SchemaType::U32,
        Value::U64(u64::MAX),
    )])
    .prepare()
    .expect_err("a U64 default in a U32 field rejects");
    assert_eq!(invalid_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        invalid_default.path,
        vec![PathSegment::Field("total".into())]
    );
    assert_eq!(invalid_default.span, None);

    // A U64 default prepares and completes on omission.
    let widened = Schema::record(vec![Field::with_default(
        "total",
        SchemaType::U64,
        Value::U64(u64::MAX),
    )])
    .prepare()
    .expect("the U64 default prepares");
    assert_eq!(
        decode_document("", &widened),
        Ok(Value::Record(vec![("total".into(), Value::U64(u64::MAX))])),
    );
}

#[test]
fn uint_scalar_value_encodes_through_public_paths() {
    use moth_mon::{MonErrorCode, Schema, SchemaType, Value, encode_value};

    let scalar = Schema::value(SchemaType::U64)
        .prepare()
        .expect("scalar U64 schema prepares");
    assert_eq!(
        encode_value(&Value::U64(u64::MAX), &scalar),
        Ok(u64::MAX.to_string()),
    );
    let narrow = Schema::value(SchemaType::U32)
        .prepare()
        .expect("scalar U32 schema prepares");
    assert_eq!(
        encode_value(&Value::U64(u64::MAX), &narrow)
            .expect_err("scalar U32 rejects a U64 value")
            .code,
        MonErrorCode::TypeMismatch,
    );
}

#[test]
fn uint_rejects_source_arithmetic_and_keeps_bounded_failures_through_public_paths() {
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required("count", SchemaType::U32)])
        .prepare()
        .expect("U32 schema prepares");

    // Source arithmetic is literal data here, not an expression to evaluate. The first input
    // fails while scanning the root entry, so it carries no field path; the parenthesised
    // input parses as a record value first, so its comma failure lands on the field path.
    for (source, code, path, start, end) in [
        (
            "count = 1 + 2",
            MonErrorCode::MissingComma,
            Vec::new(),
            10,
            11,
        ),
        (
            "count = (0 - 1) + 2",
            MonErrorCode::MissingComma,
            vec![PathSegment::Field("count".into())],
            11,
            12,
        ),
    ] {
        let error = decode_document(source, &schema).expect_err("arithmetic text is invalid MON");
        assert_eq!(error.code, code, "{source}");
        assert_eq!(error.path, path, "{source}");
        assert_eq!(error.span, Some(moth_mon::Span { start, end }), "{source}");
    }
    // Bounded failures keep their budget lane, and the prepared schema reports the same
    // bounded failure deterministically on every call.
    let tight = Schema::record(vec![Field::required(
        "values",
        SchemaType::Collection {
            element: Box::new(SchemaType::U32),
        },
    )])
    .with_limits(Limits {
        max_nodes: 6,
        ..Limits::default()
    })
    .prepare()
    .expect("tight U32 schema prepares");
    let input = "values = {1, 2, 3, 4, 5, 6, 7, 8}";
    let budget_error = decode_document(input, &tight).expect_err("node budget binds reuse");
    assert_eq!(budget_error.code, MonErrorCode::NodeBudget);
    assert_eq!(
        decode_document(input, &tight)
            .expect_err("schema reuse is deterministic")
            .code,
        MonErrorCode::NodeBudget,
    );
    // A long U32 spelling charges its own unsigned-token bytes before range checking.
    let byte_tight = Schema::record(vec![Field::required("count", SchemaType::U32)])
        .with_limits(Limits {
            max_decoded_bytes: 8,
            ..Limits::default()
        })
        .prepare()
        .expect("byte-tight U32 schema prepares");
    assert_eq!(
        decode_document("count = 18446744073709551615", &byte_tight)
            .expect_err("decoded-byte budget binds U32 input")
            .code,
        MonErrorCode::DecodedBudget,
    );
}
