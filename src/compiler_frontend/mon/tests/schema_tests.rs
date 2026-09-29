use crate::compiler_frontend::mon::{
    Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, Variant,
};

#[test]
fn malformed_schema_and_defaults_fail_during_preparation() {
    let invalid_default = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::String("not an Int".into()),
    )])
    .prepare()
    .expect_err("schema defaults are checked before use");
    assert_eq!(invalid_default.code, MonErrorCode::InvalidDefault);

    let duplicate_map_key_default = Schema::record(vec![Field::with_default(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
        Value::Map(vec![
            (Value::Int(1), Value::Int(2)),
            (Value::Int(1), Value::Int(3)),
        ]),
    )])
    .prepare()
    .expect_err("prepared map defaults reject duplicate keys before copying");
    assert_eq!(duplicate_map_key_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        duplicate_map_key_default.path,
        vec![
            PathSegment::Field("values".into()),
            PathSegment::MapKey("1".into()),
        ],
    );

    let invalid_key = Schema::record(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            }),
            value: Box::new(SchemaType::Int),
        },
    )])
    .prepare()
    .expect_err("only supported scalar families may be map keys");
    assert_eq!(invalid_key.code, MonErrorCode::InvalidMapKey);

    let duplicate_variant = Schema::record(vec![Field::required(
        "value",
        SchemaType::Choice {
            name: "Choice".into(),
            variants: vec![Variant::unit("Same"), Variant::unit("Same")],
        },
    )])
    .prepare()
    .expect_err("choice variant names must be unique");
    assert_eq!(duplicate_variant.code, MonErrorCode::InvalidSchema);
}

#[test]
fn preparation_captures_the_numeric_profile_and_widened_capacity() {
    use crate::compiler_frontend::datatypes::numeric_profile::{
        FloatPrecision, IntWidth, NumericProfile,
    };

    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let prepared = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .with_profile(profile)
        .prepare()
        .expect("profile schema prepares");
    assert_eq!(prepared.profile(), profile);
    assert_eq!(
        Schema::record(Vec::new())
            .prepare()
            .expect("default schema prepares")
            .profile(),
        NumericProfile::STANDARD
    );
    assert_eq!(
        Schema::record(Vec::new()).with_profile(profile).profile(),
        profile
    );

    // Exact-decimal scale capacity reaches the shared 256 owner and stops there.
    assert!(
        Schema::value(SchemaType::Decimal { scale: 256 })
            .prepare()
            .is_ok()
    );
    let too_wide = Schema::value(SchemaType::Decimal { scale: 257 })
        .prepare()
        .expect_err("a scale above the capacity fails");
    assert_eq!(too_wide.code, MonErrorCode::NumericScale);

    // Profile-selected `Int` and `Float` defaults are checked when the schema is prepared.
    let above_int32 = i64::from(i32::MAX) + 1;
    let out_of_range = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(above_int32),
    )])
    .prepare()
    .expect_err("an Int default outside the captured width fails");
    assert_eq!(out_of_range.code, MonErrorCode::InvalidDefault);
    assert!(
        Schema::record(vec![Field::with_default(
            "value",
            SchemaType::Int,
            Value::Int(above_int32),
        )])
        .with_profile(profile)
        .prepare()
        .is_ok()
    );

    let non_finite = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Float,
        Value::Float(f64::INFINITY),
    )])
    .prepare()
    .expect_err("a non-finite Float default fails");
    assert_eq!(non_finite.code, MonErrorCode::InvalidDefault);
    let above_float32 = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Float,
        Value::Float(1.0e300),
    )])
    .with_profile(profile)
    .prepare()
    .expect_err("a Float default outside the profile precision fails");
    assert_eq!(above_float32.code, MonErrorCode::InvalidDefault);

    // Explicit widths stay independent of the captured profile and check their own family.
    let mismatched_family = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I8,
        Value::I16(1),
    )])
    .prepare()
    .expect_err("a fixed default must match its own width");
    assert_eq!(mismatched_family.code, MonErrorCode::InvalidDefault);
    let fixed_default = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I8,
        Value::I8(i8::MIN),
    )])
    .with_profile(profile)
    .prepare()
    .expect("a matching fixed default prepares under any profile");
    assert_eq!(fixed_default.profile(), profile);

    // Fixed integers and Byte are key families; fixed binary floats are not.
    assert!(
        Schema::record(vec![Field::required(
            "values",
            SchemaType::Map {
                key: Box::new(SchemaType::U64),
                value: Box::new(SchemaType::Byte),
            },
        )])
        .prepare()
        .is_ok()
    );
    let float_key = Schema::record(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::F32),
            value: Box::new(SchemaType::Int),
        },
    )])
    .prepare()
    .expect_err("fixed binary floats are not map-key families");
    assert_eq!(float_key.code, MonErrorCode::InvalidMapKey);

    // Prepared default maps keep fixed-width key identity in their duplicate check.
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
}
