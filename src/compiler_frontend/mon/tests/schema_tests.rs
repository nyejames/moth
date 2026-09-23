use crate::compiler_frontend::mon::{Field, MonErrorCode, Schema, SchemaType, Value, Variant};

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
