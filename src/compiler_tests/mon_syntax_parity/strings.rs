//! Common quoted string and character parity fixtures.
//!
//! WHAT: covers source/MON shared escapes, Unicode literal scalars, string-contained syntax, and
//!       character scalar boundaries.
//! WHY: quoted content must not leak delimiters or comments into the surrounding parser.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, SourceReason, assert_fixture, fixture,
    in_literal, mon_span,
};

fn string_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    let escaped_literal = r#"escaped = "quote: \" slash: \\ newline: \n carriage: \r tab: \t", unicode = "Moth 🪁 café", delimiters = "// -- ) , =", letter = '🧪'"#;
    fixtures.push(fixture(
        "common_quoted_escapes_unicode_and_delimiter_text",
        escaped_literal,
        "",
        Schema::record(vec![
            Field::required("escaped", SchemaType::String),
            Field::required("unicode", SchemaType::String),
            Field::required("delimiters", SchemaType::String),
            Field::required("letter", SchemaType::Char),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            (
                "escaped".into(),
                Value::String("quote: \" slash: \\ newline: \n carriage: \r tab: \t".into()),
            ),
            ("unicode".into(), Value::String("Moth 🪁 café".into())),
            ("delimiters".into(), Value::String("// -- ) , =".into())),
            ("letter".into(), Value::Char('🧪')),
        ])),
    ));

    let boundary_literal = r#"empty = "", quote = "\"", slash = "\\", pair = "\\\"""#;
    fixtures.push(fixture(
        "quoted_string_escape_boundaries",
        boundary_literal,
        "",
        Schema::record(vec![
            Field::required("empty", SchemaType::String),
            Field::required("quote", SchemaType::String),
            Field::required("slash", SchemaType::String),
            Field::required("pair", SchemaType::String),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("empty".into(), Value::String(String::new())),
            ("quote".into(), Value::String("\"".into())),
            ("slash".into(), Value::String("\\".into())),
            ("pair".into(), Value::String("\\\"".into())),
        ])),
    ));

    let multi_scalar_literal = "letter = 'ab'";
    fixtures.push(fixture(
        "character_rejects_multiple_unicode_scalars",
        multi_scalar_literal,
        "",
        Schema::record(vec![Field::required("letter", SchemaType::Char)]),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0009",
            reason: SourceReason::InvalidCharLiteral,
            site: in_literal(multi_scalar_literal, "'a", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::InvalidCharacter,
            span: mon_span(multi_scalar_literal, "b", 1),
            path: vec![PathSegment::Field("letter".into())],
        },
    ));

    fixtures
}

#[test]
fn quoted_strings_and_characters_match_mon_data() {
    let _guard = lock_counter_test();
    for fixture in string_fixtures() {
        assert_fixture(fixture);
    }
}
