//! Proves the compiler convenience path and standalone crate share MON types.

use moth::mon as facade;
use moth_mon as standalone;

#[test]
fn facade_and_standalone_share_schemas_and_values() {
    let facade_schema: standalone::PreparedSchema =
        facade::Schema::record(vec![facade::Field::required(
            "name",
            facade::SchemaType::String,
        )])
        .prepare()
        .expect("facade schema prepares as the standalone type");
    let standalone_value: standalone::Value =
        standalone::decode_document("name = \"Moth\"", &facade_schema)
            .expect("standalone reader accepts the facade schema");
    assert_eq!(
        standalone_value,
        standalone::Value::Record(vec![(
            "name".to_owned(),
            standalone::Value::String("Moth".to_owned()),
        )]),
    );

    let standalone_schema = standalone::Schema::record(vec![standalone::Field::required(
        "enabled",
        standalone::SchemaType::Bool,
    )])
    .prepare()
    .expect("standalone schema prepares");
    let facade_value: standalone::Value =
        facade::decode_document("enabled = true", &standalone_schema)
            .expect("facade reader accepts the standalone schema");
    assert_eq!(
        facade_value,
        facade::Value::Record(vec![("enabled".to_owned(), facade::Value::Bool(true),)]),
    );
}
