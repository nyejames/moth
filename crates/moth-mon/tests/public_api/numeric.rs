//! Public API coverage for profile-selected and explicit-width numeric behavior.

use super::*;

#[test]
fn negative_ints_preserve_signed_i32_boundaries() {
    use moth_mon::{Field, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required("whole", SchemaType::Int)])
        .prepare()
        .expect("the Int schema prepares");

    assert_eq!(
        decode_document("whole = -2147483648", &schema).expect("the smallest Int is accepted"),
        Value::Record(vec![("whole".into(), Value::Int(i64::from(i32::MIN)))]),
    );
    assert_eq!(
        decode_document("whole = -5", &schema).expect("negative Int values keep their sign"),
        Value::Record(vec![("whole".into(), Value::Int(-5))]),
    );
    assert_eq!(
        decode_document("whole = -2147483649", &schema)
            .expect_err("values below the Int range are rejected")
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

    let schema = Schema::record(vec![Field::required("v", SchemaType::Float)])
        .with_limits(Limits {
            max_numeric_digits: 10_000,
            ..Limits::default()
        })
        .prepare()
        .expect("the exponent schema fits its limits");

    let value = decode_document("v = 1e+21", &schema).expect("large finite exponent decodes");
    assert_eq!(value, Value::Record(vec![("v".into(), Value::Float(1e21))]));
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
fn error_code_u32_round_trips_independently_of_native_int_profile() {
    use moth_mon::{FloatPrecision, IntWidth, NumericProfile};

    for int_width in [IntWidth::Bits32, IntWidth::Bits64] {
        let schema = Schema::record(vec![
            Field::required("message", SchemaType::String),
            Field::with_default("code", SchemaType::U32, Value::U32(0)),
        ])
        .with_profile(NumericProfile {
            int_width,
            float_precision: FloatPrecision::Bits64,
        })
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
            assert_eq!(decoded, expected, "{int_width:?}, code={code}");

            let encoded = encode_document(&decoded, &schema).expect("U32 code encodes");
            let round_trip =
                decode_document_bytes(encoded.as_bytes(), &schema).expect("U32 bytes decode");
            drop(encoded);
            drop(decoded);
            assert_eq!(round_trip, expected, "{int_width:?}, code={code}");
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
            assert_eq!(
                error.code,
                MonErrorCode::NumericRange,
                "{int_width:?}, {code}"
            );
            assert_eq!(error.path, vec![PathSegment::Field("code".into())]);
        }
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
fn profile_selected_int_and_float_bind_through_public_paths() {
    use moth_mon::{
        Field, FloatPrecision, IntWidth, MonErrorCode, NumericProfile, Schema, SchemaType, Value,
        decode_document, encode_document,
    };

    for int_width in [IntWidth::Bits32, IntWidth::Bits64] {
        for float_precision in [FloatPrecision::Bits32, FloatPrecision::Bits64] {
            let profile = NumericProfile {
                int_width,
                float_precision,
            };
            let schema = Schema::record(vec![
                Field::required("selected_int", SchemaType::Int),
                Field::required("selected_float", SchemaType::Float),
                Field::required("explicit_i64", SchemaType::I64),
                Field::required("explicit_f64", SchemaType::F64),
            ])
            .with_profile(profile)
            .prepare()
            .expect("every numeric profile prepares");
            assert_eq!(schema.profile(), profile);

            // The captured profile alone bounds the selected Int and rounds the selected
            // Float; the explicit I64/F64 controls keep their own widths under it.
            let (min, max) = match int_width {
                IntWidth::Bits32 => (i64::from(i32::MIN), i64::from(i32::MAX)),
                IntWidth::Bits64 => (i64::MIN, i64::MAX),
            };
            let (above_max, below_min) = match int_width {
                IntWidth::Bits32 => ("2147483648", "-2147483649"),
                IntWidth::Bits64 => ("9223372036854775808", "-9223372036854775809"),
            };
            let rounded = match float_precision {
                FloatPrecision::Bits32 => f64::from(0.1f32),
                FloatPrecision::Bits64 => 0.1,
            };
            let document = |int_text: &str, float_text: &str| {
                format!(
                    "selected_int = {int_text}, selected_float = {float_text}, \
                     explicit_i64 = {}, explicit_f64 = 0.1",
                    i64::MAX,
                )
            };
            let record = |int_value: i64, float_value: f64| {
                Value::Record(vec![
                    ("selected_int".into(), Value::Int(int_value)),
                    ("selected_float".into(), Value::Float(float_value)),
                    ("explicit_i64".into(), Value::I64(i64::MAX)),
                    ("explicit_f64".into(), Value::F64(0.1)),
                ])
            };

            // Both text receive and encode->decode bind the exact width bounds and the
            // exact rounding through the same prepared schema.
            for (name, bound) in [("min", min), ("max", max)] {
                let expected = record(bound, rounded);
                let received = decode_document(&document(&bound.to_string(), "0.1"), &schema)
                    .unwrap_or_else(|error| panic!("{profile:?} {name} receive: {error:?}"));
                assert_eq!(
                    received, expected,
                    "{profile:?}: the {name} bound receives profile-typed values",
                );
                let Value::Record(fields) = &received else {
                    panic!("{profile:?} {name} receives a record");
                };
                let Some((_, Value::Float(number))) =
                    fields.iter().find(|(field, _)| field == "selected_float")
                else {
                    panic!("{profile:?} {name} receives a Float field");
                };
                assert_eq!(
                    number.to_bits(),
                    rounded.to_bits(),
                    "{profile:?}: 0.1 rounds at the captured float precision",
                );
                let encoded = encode_document(&expected, &schema)
                    .unwrap_or_else(|error| panic!("{profile:?} {name} encode: {error:?}"));
                let round_trip = decode_document(&encoded, &schema)
                    .unwrap_or_else(|error| panic!("{profile:?} {name} redecode: {error:?}"));
                assert_eq!(
                    round_trip, expected,
                    "{profile:?}: the {name} bound round-trips"
                );
            }

            // One step outside either bound fails with the range code.
            for (name, outside) in [("above the max", above_max), ("below the min", below_min)] {
                assert_eq!(
                    decode_document(&document(outside, "0.1"), &schema)
                        .expect_err("outside the captured Int width")
                        .code,
                    MonErrorCode::NumericRange,
                    "{profile:?}: {name} is rejected with the range code",
                );
            }

            // A finite source outside the Float32 finite range fails there but stays an
            // ordinary finite Float64 value.
            let huge = document(&min.to_string(), "1.0e40");
            match float_precision {
                FloatPrecision::Bits32 => assert_eq!(
                    decode_document(&huge, &schema)
                        .expect_err("above Float32 fails")
                        .code,
                    MonErrorCode::NonFiniteFloat,
                    "{profile:?}: 1e40 is outside the Float32 finite range",
                ),
                FloatPrecision::Bits64 => assert_eq!(
                    decode_document(&huge, &schema).expect("Float64 keeps finite 1e40"),
                    record(min, 1.0e40),
                    "{profile:?}: 1e40 is an ordinary finite Float64",
                ),
            }

            // Programmatic encode follows the same captured width: an out-of-range Int32
            // payload fails, while the Int64 payload above is the max bound.
            if int_width == IntWidth::Bits32 {
                assert_eq!(
                    encode_document(&record(i64::MAX, rounded), &schema)
                        .expect_err("above Int32 fails to encode")
                        .code,
                    MonErrorCode::NumericRange,
                    "{profile:?}: programmatic Int32 overflow fails to encode",
                );
            }
        }
    }

    // The default profile is its own public contract: STANDARD stays Int32/Float64.
    let standard = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .prepare()
        .expect("default schema prepares");
    assert_eq!(standard.profile(), NumericProfile::STANDARD);
    assert_eq!(
        decode_document("value = 2147483648", &standard)
            .expect_err("above Int32 fails")
            .code,
        MonErrorCode::NumericRange,
    );
    let wide = Value::Record(vec![("value".into(), Value::Int(i64::MAX))]);
    assert_eq!(
        encode_document(&wide, &standard)
            .expect_err("above Int32 fails to encode")
            .code,
        MonErrorCode::NumericRange,
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
            SchemaType::Int,
            [Value::Int(2), Value::Int(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
        ),
        (
            SchemaType::Uint,
            [Value::Uint(2), Value::Uint(1)],
            "{2 = 20, 1 = 10}",
            "{10 = 20, 1_0 = 10}",
            "10",
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
            value: Box::new(SchemaType::Int),
        };
        let map = Value::Map(vec![
            (keys[0].clone(), Value::Int(20)),
            (keys[1].clone(), Value::Int(10)),
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
            (keys[0].clone(), Value::Int(20)),
            (keys[0].clone(), Value::Int(10)),
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
                (keys[1].clone(), Value::Int(20)),
                (other_keys[1].clone(), Value::Int(10)),
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
            value: Box::new(SchemaType::Int),
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
        value: Box::new(SchemaType::Int),
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
    use moth_mon::{
        Field, FloatPrecision, IntWidth, MonErrorCode, NumericProfile, PathSegment, Schema,
        SchemaType, Value, decode_document,
    };

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
            value: Box::new(SchemaType::Int),
        },
        Value::Map(vec![
            (Value::Byte(1), Value::Int(2)),
            (Value::Byte(1), Value::Int(3)),
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
    let out_of_range = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(above_int32),
    )])
    .prepare()
    .expect_err("an Int default outside the captured width fails");
    assert_eq!(out_of_range.code, MonErrorCode::InvalidDefault);
    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };
    let in_range = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(above_int32),
    )])
    .with_profile(int64_profile)
    .prepare()
    .expect("an Int default inside the Int64 width prepares");
    assert_eq!(
        decode_document("", &in_range).expect("the Int64 default applies"),
        Value::Record(vec![("value".into(), Value::Int(above_int32))]),
    );

    let float32_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let above_float32 = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Float,
        Value::Float(1.0e300),
    )])
    .with_profile(float32_profile)
    .prepare()
    .expect_err("a Float default outside the profile precision fails");
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
/// A host crate selects its numeric profile once, builds an explicit schema
/// with fixed-width, `Byte` and widened-scale members, then round-trips owned
/// values through the public encode/decode adapters.
fn profiled_telemetry_round_trip() -> Value {
    use moth_mon::{
        Field, FloatPrecision, IntWidth, NumericProfile, Schema, SchemaType, Value,
        decode_document, decode_document_bytes, encode_document,
    };

    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let schema = Schema::record(vec![
        Field::required("seq", SchemaType::U64),
        Field::required("sample", SchemaType::Float),
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
    .with_profile(profile)
    .prepare()
    .expect("example schema prepares");
    assert_eq!(schema.profile(), profile);

    let document = Value::Record(vec![
        ("seq".into(), Value::U64(u64::MAX)),
        ("sample".into(), Value::Float(f64::from(0.1f32))),
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
fn outside_crate_profiled_round_trip_example() {
    let value = profiled_telemetry_round_trip();
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
fn uint_captures_the_profile_int_width_through_public_paths() {
    use moth_mon::{
        Field, FloatPrecision, IntWidth, MonErrorCode, NumericProfile, PathSegment, Schema,
        SchemaType, Span, Value, decode_document, decode_document_bytes, encode_document,
    };

    let profiles = [
        (
            "int32_float64",
            NumericProfile {
                int_width: IntWidth::Bits32,
                float_precision: FloatPrecision::Bits64,
            },
        ),
        (
            "int32_float32",
            NumericProfile {
                int_width: IntWidth::Bits32,
                float_precision: FloatPrecision::Bits32,
            },
        ),
        (
            "int64_float64",
            NumericProfile {
                int_width: IntWidth::Bits64,
                float_precision: FloatPrecision::Bits64,
            },
        ),
        (
            "int64_float32",
            NumericProfile {
                int_width: IntWidth::Bits64,
                float_precision: FloatPrecision::Bits32,
            },
        ),
    ];

    for (name, profile) in profiles {
        let schema = Schema::record(vec![Field::required("count", SchemaType::Uint)])
            .with_profile(profile)
            .prepare()
            .expect("Uint schema prepares");
        assert_eq!(schema.profile(), profile, "{name}");
        let maximum = profile.int_width.unsigned_max_value();

        // Zero, separators, signed-max-plus-one and the profile maximum decode exactly.
        let mut accepted: Vec<(String, u64)> = vec![
            ("0".into(), 0),
            ("4_294_967_295".into(), u32::MAX as u64),
            ("2147483648".into(), 2_147_483_648),
            (maximum.to_string(), maximum),
        ];
        if profile.int_width == IntWidth::Bits64 {
            accepted.push(("9007199254740993".into(), 9_007_199_254_740_993));
            accepted.push(("9223372036854775807".into(), i64::MAX as u64));
            accepted.push(("9223372036854775808".into(), 9_223_372_036_854_775_808));
        }
        for (spelling, expected) in &accepted {
            let source = format!("count = {spelling}");
            let decoded = decode_document(&source, &schema).expect("in-range Uint decodes");
            drop(source);
            let expected_value = Value::Record(vec![("count".into(), Value::Uint(*expected))]);
            assert_eq!(decoded, expected_value, "{name}, {spelling}");
            let encoded = encode_document(&decoded, &schema).expect("Uint encodes");
            let round_trip =
                decode_document_bytes(encoded.as_bytes(), &schema).expect("Uint bytes decode");
            drop(encoded);
            drop(decoded);
            assert_eq!(round_trip, expected_value, "{name}, {spelling}");
        }

        // The other width's maximum and text overflow reject with range, path and span.
        let mut range_rejected: Vec<String> = vec![
            maximum
                .checked_add(1)
                .map_or("18446744073709551616".to_string(), |next| next.to_string()),
        ];
        if profile.int_width == IntWidth::Bits32 {
            range_rejected.push(u64::MAX.to_string());
            range_rejected.push("9223372036854775808".into());
        }
        for spelling in &range_rejected {
            let source = format!("count = {spelling}");
            let error = decode_document(&source, &schema).expect_err("Uint overflow rejects");
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
            let error = decode_document(&source, &schema).expect_err("non-Uint spelling rejects");
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

        // Programmatic values validate against the captured profile too.
        let supplied = Value::Record(vec![("count".into(), Value::Uint(maximum))]);
        assert_eq!(
            encode_document(&supplied, &schema).and_then(|text| decode_document(&text, &schema)),
            Ok(supplied.clone()),
            "{name}",
        );
        let overflow = maximum
            .checked_add(1)
            .map(|next| Value::Record(vec![("count".into(), Value::Uint(next))]));
        if profile.int_width == IntWidth::Bits32 {
            let overflow = overflow.expect("Uint32 maximum has a successor");
            let error =
                encode_document(&overflow, &schema).expect_err("Uint32 rejects u64 above max");
            assert_eq!(error.code, MonErrorCode::NumericRange, "{name}");
            assert_eq!(
                error.path,
                vec![PathSegment::Field("count".into())],
                "{name}"
            );
            assert_eq!(error.span, None, "{name}");
        } else {
            assert!(overflow.is_none(), "{name}");
        }
    }
}

#[test]
fn uint_keeps_strict_category_identity_through_public_paths() {
    use moth_mon::{
        Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    let schema = Schema::record(vec![
        Field::required("profiled", SchemaType::Uint),
        Field::required("signed", SchemaType::Int),
        Field::required("fixed", SchemaType::U64),
        Field::required("narrow", SchemaType::U32),
    ])
    .prepare()
    .expect("mixed integer schema prepares");
    let document = Value::Record(vec![
        ("profiled".into(), Value::Uint(2_147_483_648)),
        ("signed".into(), Value::Int(2_147_483_647)),
        ("fixed".into(), Value::U64(u64::MAX)),
        ("narrow".into(), Value::U32(u32::MAX)),
    ]);
    let encoded = encode_document(&document, &schema).expect("distinct integer values encode");
    assert_eq!(decode_document(&encoded, &schema), Ok(document));

    // Equal carriers never alias another integer family, in either direction.
    let families: [(SchemaType, Value); 4] = [
        (SchemaType::Uint, Value::Uint(7)),
        (SchemaType::Int, Value::Int(7)),
        (SchemaType::U64, Value::U64(7)),
        (SchemaType::U32, Value::U32(7)),
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

    // Explicit U32/U64 stay distinct and profile-independent.
    for int_width in [moth_mon::IntWidth::Bits32, moth_mon::IntWidth::Bits64] {
        let profile = moth_mon::NumericProfile {
            int_width,
            float_precision: moth_mon::FloatPrecision::Bits64,
        };
        let explicit = Schema::record(vec![
            Field::required("narrow", SchemaType::U32),
            Field::required("wide", SchemaType::U64),
        ])
        .with_profile(profile)
        .prepare()
        .expect("explicit widths prepare under any profile");
        let source = "narrow = 4294967295, wide = 18446744073709551615";
        assert_eq!(
            decode_document(source, &explicit),
            Ok(Value::Record(vec![
                ("narrow".into(), Value::U32(u32::MAX)),
                ("wide".into(), Value::U64(u64::MAX)),
            ])),
            "{int_width:?}",
        );
    }
}

#[test]
fn uint_defaults_optionals_nesting_and_map_keys_round_trip_through_public_paths() {
    use moth_mon::{
        Field, IntWidth, MonErrorCode, NumericProfile, PathSegment, Schema, SchemaType, Value,
        decode_document, encode_document,
    };

    let schema = Schema::record(vec![
        Field::with_default("total", SchemaType::Uint, Value::Uint(2_147_483_648)),
        Field::required("maybe", SchemaType::Optional(Box::new(SchemaType::Uint))),
        Field::required(
            "pair",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::Uint)],
            },
        ),
        Field::required(
            "counts",
            SchemaType::Map {
                key: Box::new(SchemaType::Uint),
                value: Box::new(SchemaType::Uint),
            },
        ),
        Field::required(
            "batch",
            SchemaType::Collection {
                element: Box::new(SchemaType::Uint),
            },
        ),
    ])
    .prepare()
    .expect("Uint default/optional/nested/map schema prepares");
    let expected = Value::Record(vec![
        ("total".into(), Value::Uint(2_147_483_648)),
        ("maybe".into(), Value::None),
        (
            "pair".into(),
            Value::Record(vec![("inner".into(), Value::Uint(0))]),
        ),
        (
            "counts".into(),
            Value::Map(vec![
                (Value::Uint(u64::MAX), Value::Uint(1)),
                (Value::Uint(0), Value::Uint(2)),
            ]),
        ),
        ("batch".into(), Value::Collection(vec![Value::Uint(3)])),
    ]);
    // The delivered default is Uint32: the `u64::MAX` map key is out of range here, so this
    // first decode must fail before the profiled schema below. The `2147483648` default is
    // signed-max-plus-one and fits Uint32, so preparation already accepted it.
    let standard_error = decode_document(
        "maybe = none, pair = (inner = 0), counts = {18446744073709551615 = 1, 0 = 2}, batch = {3}",
        &schema,
    )
    .expect_err("Uint64-scale keys reject under the Uint32 default");
    assert_eq!(standard_error.code, MonErrorCode::NumericRange);

    let profiled = Schema::record(vec![
        Field::with_default("total", SchemaType::Uint, Value::Uint(2_147_483_648)),
        Field::required("maybe", SchemaType::Optional(Box::new(SchemaType::Uint))),
        Field::required(
            "pair",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::Uint)],
            },
        ),
        Field::required(
            "counts",
            SchemaType::Map {
                key: Box::new(SchemaType::Uint),
                value: Box::new(SchemaType::Uint),
            },
        ),
        Field::required(
            "batch",
            SchemaType::Collection {
                element: Box::new(SchemaType::Uint),
            },
        ),
    ])
    .with_profile(NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: moth_mon::FloatPrecision::Bits64,
    })
    .prepare()
    .expect("Uint64-scale schema prepares");
    let decoded = decode_document(
        "maybe = none, pair = (inner = 0), counts = {18446744073709551615 = 1, 0 = 2}, batch = {3}",
        &profiled,
    )
    .expect("Uint64-scale document decodes");
    assert_eq!(decoded, expected);
    let encoded = encode_document(&decoded, &profiled).expect("Uint document encodes");
    assert_eq!(decode_document(&encoded, &profiled), Ok(expected.clone()));
    // The same prepared schema decodes again unchanged.
    assert_eq!(
        decode_document(
            "total = 1, maybe = 2, pair = (inner = 3), counts = {=}, batch = {}",
            &profiled
        ),
        Ok(Value::Record(vec![
            ("total".into(), Value::Uint(1)),
            ("maybe".into(), Value::Uint(2)),
            (
                "pair".into(),
                Value::Record(vec![("inner".into(), Value::Uint(3))]),
            ),
            ("counts".into(), Value::Map(vec![])),
            ("batch".into(), Value::Collection(vec![])),
        ])),
    );
    // An omitted field completes from its prepared Uint default.
    assert_eq!(
        decode_document(
            "maybe = 2, pair = (inner = 3), counts = {=}, batch = {}",
            &profiled
        ),
        Ok(Value::Record(vec![
            ("total".into(), Value::Uint(2_147_483_648)),
            ("maybe".into(), Value::Uint(2)),
            (
                "pair".into(),
                Value::Record(vec![("inner".into(), Value::Uint(3))]),
            ),
            ("counts".into(), Value::Map(vec![])),
            ("batch".into(), Value::Collection(vec![])),
        ])),
    );

    // Duplicate Uint keys report the decoded key path.
    let duplicate = decode_document(
        "maybe = none, pair = (inner = 0), counts = {7 = 1, 0_7 = 2}, batch = {3}",
        &profiled,
    )
    .expect_err("decoded-equivalent Uint keys are duplicates");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        duplicate.path,
        vec![
            PathSegment::Field("counts".into()),
            PathSegment::MapKey("7".into()),
        ],
    );

    // An out-of-range Uint default fails during preparation, not at first use.
    let invalid_default = Schema::record(vec![Field::with_default(
        "total",
        SchemaType::Uint,
        Value::Uint(u64::MAX),
    )])
    .prepare()
    .expect_err("Uint default above Uint32 rejects");
    assert_eq!(invalid_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        invalid_default.path,
        vec![PathSegment::Field("total".into())]
    );
    assert_eq!(invalid_default.span, None);

    // A Uint64 default prepares under a 64-bit profile and completes on omission.
    let widened = Schema::record(vec![Field::with_default(
        "total",
        SchemaType::Uint,
        Value::Uint(u64::MAX),
    )])
    .with_profile(NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: moth_mon::FloatPrecision::Bits64,
    })
    .prepare()
    .expect("Uint64-scale default prepares");
    assert_eq!(
        decode_document("", &widened),
        Ok(Value::Record(vec![("total".into(), Value::Uint(u64::MAX))])),
    );
}

#[test]
fn uint_scalar_value_encodes_through_public_paths() {
    use moth_mon::{
        IntWidth, MonErrorCode, NumericProfile, Schema, SchemaType, Value, encode_value,
    };

    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: moth_mon::FloatPrecision::Bits64,
    };
    let scalar = Schema::value(SchemaType::Uint)
        .with_profile(profile)
        .prepare()
        .expect("scalar Uint schema prepares");
    assert_eq!(
        encode_value(&Value::Uint(u64::MAX), &scalar),
        Ok(u64::MAX.to_string()),
    );
    let narrow = Schema::value(SchemaType::Uint)
        .prepare()
        .expect("scalar Uint32 schema prepares");
    assert_eq!(
        encode_value(&Value::Uint(u64::MAX), &narrow)
            .expect_err("scalar Uint32 rejects u64 above max")
            .code,
        MonErrorCode::NumericRange,
    );
}

#[test]
fn uint_rejects_source_arithmetic_and_keeps_bounded_failures_through_public_paths() {
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required("count", SchemaType::Uint)])
        .prepare()
        .expect("Uint schema prepares");

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
            element: Box::new(SchemaType::Uint),
        },
    )])
    .with_limits(Limits {
        max_nodes: 6,
        ..Limits::default()
    })
    .prepare()
    .expect("tight Uint schema prepares");
    let input = "values = {1, 2, 3, 4, 5, 6, 7, 8}";
    let budget_error = decode_document(input, &tight).expect_err("node budget binds reuse");
    assert_eq!(budget_error.code, MonErrorCode::NodeBudget);
    assert_eq!(
        decode_document(input, &tight)
            .expect_err("schema reuse is deterministic")
            .code,
        MonErrorCode::NodeBudget,
    );
    // A long Uint spelling charges its own unsigned-token bytes before range checking.
    let byte_tight = Schema::record(vec![Field::required("count", SchemaType::Uint)])
        .with_limits(Limits {
            max_decoded_bytes: 8,
            ..Limits::default()
        })
        .prepare()
        .expect("byte-tight Uint schema prepares");
    assert_eq!(
        decode_document("count = 18446744073709551615", &byte_tight)
            .expect_err("decoded-byte budget binds Uint input")
            .code,
        MonErrorCode::DecodedBudget,
    );
}
