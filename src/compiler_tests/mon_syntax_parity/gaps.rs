//! Named, executable differences between source syntax and MON literal data.
//!
//! WHAT: records intentional contract boundaries and accepted source capabilities not yet delivered.
//! WHY: exact current diagnostics make each source gap fail automatically when it is implemented.

use std::collections::BTreeSet;

use crate::compiler_frontend::compiler_messages::{
    InvalidMapLiteralReason, InvalidStringEscapeReason,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{
    Field, Limits, MonErrorCode, PathSegment, Schema, SchemaType, Span, Value, Variant,
};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_declarations,
    in_literal, mon_span, source_invalid_map_literal, source_invalid_string_escape,
    source_unexpected_token,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DifferenceKind {
    IntentionalDifference,
    SourceGap,
}

pub(super) struct ParityDifference {
    pub(super) name: &'static str,
    kind: DifferenceKind,
    pub(super) capability: &'static str,
}

const PARITY_DIFFERENCES: &[ParityDifference] = &[
    ParityDifference {
        name: "mon-document-envelope",
        kind: DifferenceKind::IntentionalDifference,
        capability: "MON frames one root record, including implicit and empty roots; source receivers also accept expression fragments.",
    },
    ParityDifference {
        name: "schema-versus-source-identity",
        kind: DifferenceKind::IntentionalDifference,
        capability: "MON schemas supply shape and nominal identity; source infers anonymous records and requires explicit nominal constructors.",
    },
    ParityDifference {
        name: "literal-data-versus-source-syntax",
        kind: DifferenceKind::IntentionalDifference,
        capability: "MON accepts literal data, not source expressions, bindings, imports, mutable access or declaration syntax.",
    },
    ParityDifference {
        name: "quoted-text-contract",
        kind: DifferenceKind::IntentionalDifference,
        capability: "MON preserves raw quoted text and has its own bounded escape contract.",
    },
    ParityDifference {
        name: "source-empty-map",
        kind: DifferenceKind::SourceGap,
        capability: "Moth source does not implement the explicit `{=}` empty-map spelling.",
    },
    ParityDifference {
        name: "source-contextual-choice",
        kind: DifferenceKind::SourceGap,
        capability: "Moth source does not implement contextual `::Variant` construction.",
    },
    ParityDifference {
        name: "source-unicode-escape",
        kind: DifferenceKind::SourceGap,
        capability: "Moth source does not implement MON's `\\u{...}` Unicode escape.",
    },
    ParityDifference {
        name: "text-versus-value-limits",
        kind: DifferenceKind::IntentionalDifference,
        capability: "MON bounds exact input text and materialisation; those codec budgets do not limit source values.",
    },
];

/// Look up a named difference; an unknown name is a broken fixture.
pub(super) fn parity_difference(name: &str) -> &'static ParityDifference {
    PARITY_DIFFERENCES
        .iter()
        .find(|difference| difference.name == name)
        .unwrap_or_else(|| panic!("parity case names unknown difference '{name}'"))
}

struct GapCase {
    difference: &'static ParityDifference,
    fixture: ParityFixture,
}

impl GapCase {
    /// Replace the default literal envelope with a fully framed source receiver.
    fn with_source_override(mut self, source: impl Into<String>) -> Self {
        self.fixture = self.fixture.with_source_override(source);
        self
    }

    /// Expect differing source/MON outcomes owned by this case's parity difference.
    fn with_outcome_difference(mut self) -> Self {
        self.fixture = self.fixture.with_outcome_difference(self.difference.name);
        self
    }
}

#[test]
fn named_parity_differences_have_executable_exact_outcomes() {
    let _guard = lock_counter_test();
    let cases = gap_cases();
    let exercised = cases
        .iter()
        .map(|case| case.difference.name)
        .collect::<BTreeSet<_>>();

    for difference in PARITY_DIFFERENCES {
        assert!(
            exercised.contains(difference.name),
            "named {:?} parity capability '{}' ({}) has no executable case",
            difference.kind,
            difference.name,
            difference.capability
        );
    }

    assert_eq!(
        cases.len(),
        PARITY_DIFFERENCES.len(),
        "each named parity difference has one deliberately owned case"
    );
    for case in cases {
        assert_fixture(case.fixture);
    }
}

fn gap_cases() -> Vec<GapCase> {
    let expression_literal = "2 + 3";
    let expression = gap_fixture(
        "mon-document-envelope",
        "expression-fragment",
        expression_literal,
        "",
        Schema::record(vec![Field::required("result", SchemaType::Int)]),
        SourceExpectation::AcceptObserved(Value::Int(5)),
        MonExpectation::Reject {
            code: MonErrorCode::RootNotRecord,
            span: mon_span(expression_literal, "2", 1),
            path: Vec::new(),
        },
    )
    .with_outcome_difference();

    // The schema can receive a Point-shaped record, while source keeps the unqualified value
    // anonymous. This is not implicit source nominal construction.
    let schema_supplied_identity = gap_fixture(
        "schema-versus-source-identity",
        "anonymous-record-under-struct-schema",
        "point = (x = 1)",
        "Point = | x Int |\n",
        Schema::record(vec![Field::required(
            "point",
            SchemaType::Struct {
                name: "Point".into(),
                fields: vec![Field::required("x", SchemaType::Int)],
            },
        )]),
        SourceExpectation::AcceptObserved(Value::Record(vec![(
            "point".into(),
            Value::Record(vec![("x".into(), Value::Int(1))]),
        )])),
        MonExpectation::Accept(Value::Record(vec![(
            "point".into(),
            Value::Record(vec![("x".into(), Value::Int(1))]),
        )])),
    );

    let reference = "result = base";
    let source_syntax = gap_fixture(
        "literal-data-versus-source-syntax",
        "constant-reference",
        reference,
        "base #Int = 4\n",
        Schema::record(vec![Field::required("result", SchemaType::Int)]),
        SourceExpectation::AcceptObserved(Value::Record(vec![("result".into(), Value::Int(4))])),
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(reference, "base", 1),
            path: vec![PathSegment::Field("result".into())],
        },
    )
    .with_outcome_difference();

    let raw_newline = "text = \"before\r\nafter\"";
    let quoted_text = gap_fixture(
        "quoted-text-contract",
        "raw-crlf-is-normalised-by-source-and-preserved-by-mon",
        raw_newline,
        "",
        Schema::record(vec![Field::required("text", SchemaType::String)]),
        SourceExpectation::AcceptObserved(Value::Record(vec![(
            "text".into(),
            Value::String("before\nafter".into()),
        )])),
        MonExpectation::Accept(Value::Record(vec![(
            "text".into(),
            Value::String("before\r\nafter".into()),
        )])),
    );

    // These are validly typed source receivers for deferred capabilities. When either capability
    // lands, move its case from this gap to common acceptance. An empty source map takes its type
    // from an explicit declaration, not a constructor argument slot, so the map is bound first.
    let empty_map = "scores = {=}";
    let source_empty_map_source = concat!(
        "MapHolder = | scores {String = Int} |\n",
        "scores {String = Int} = {=}\n",
        "data = MapHolder(scores = scores)\n",
    );
    let source_empty_map = gap_fixture(
        "source-empty-map",
        "explicit-empty-map-spelling",
        empty_map,
        "",
        Schema::record(vec![Field::required(
            "scores",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        )]),
        source_invalid_map_literal(
            InvalidMapLiteralReason::MissingKeyExpression,
            in_declarations(source_empty_map_source, "=", 5),
        ),
        MonExpectation::Accept(Value::Record(vec![(
            "scores".into(),
            Value::Map(Vec::new()),
        )])),
    )
    .with_source_override(source_empty_map_source)
    .with_outcome_difference();

    let contextual_choice = "state = ::Ready";
    let source_contextual_choice_source = concat!(
        "Theme ::\n",
        "    Ready,\n",
        ";\n",
        "Holder = | state Theme |\n",
        "data = Holder(state = ::Ready)\n",
    );
    let source_contextual_choice = gap_fixture(
        "source-contextual-choice",
        "unqualified-choice-variant",
        contextual_choice,
        "",
        Schema::record(vec![Field::required(
            "state",
            SchemaType::Choice {
                name: "Theme".into(),
                variants: vec![Variant::unit("Ready")],
            },
        )]),
        source_unexpected_token(in_declarations(source_contextual_choice_source, "::", 2)),
        MonExpectation::Accept(Value::Record(vec![(
            "state".into(),
            Value::Choice {
                qualifier: None,
                variant: "Ready".into(),
                fields: Vec::new(),
            },
        )])),
    )
    .with_source_override(source_contextual_choice_source)
    .with_outcome_difference();

    let unicode_escape = "text = \"\\u{1}\"";
    let source_unicode_escape = gap_fixture(
        "source-unicode-escape",
        "mon-writer-control-character-escape",
        unicode_escape,
        "",
        Schema::record(vec![Field::required("text", SchemaType::String)]),
        source_invalid_string_escape(
            InvalidStringEscapeReason::UnsupportedEscape { escaped: 'u' },
            in_literal(unicode_escape, "\\u", 1),
        ),
        MonExpectation::Accept(Value::Record(vec![(
            "text".into(),
            Value::String("\u{1}".into()),
        )])),
    )
    .with_outcome_difference();

    let limited_text = "text = \"Moth\"";
    let limits = Limits {
        max_input_bytes: 8,
        ..Limits::default()
    };
    let text_limits = gap_fixture(
        "text-versus-value-limits",
        "codec-input-budget-versus-constant-value",
        limited_text,
        "",
        Schema::record(vec![Field::required("text", SchemaType::String)]).with_limits(limits),
        SourceExpectation::AcceptObserved(Value::Record(vec![(
            "text".into(),
            Value::String("Moth".into()),
        )])),
        MonExpectation::Reject {
            code: MonErrorCode::InputBudget,
            span: Span {
                start: 0,
                end: limited_text.len() as u32,
            },
            path: Vec::new(),
        },
    )
    .with_outcome_difference();

    vec![
        expression,
        schema_supplied_identity,
        source_syntax,
        quoted_text,
        source_empty_map,
        source_contextual_choice,
        source_unicode_escape,
        text_limits,
    ]
}

fn gap_fixture(
    difference_name: &'static str,
    case_name: &str,
    literal: &str,
    declarations: &str,
    schema: Schema,
    expected_source: SourceExpectation,
    expected_mon: MonExpectation,
) -> GapCase {
    let difference = parity_difference(difference_name);

    let fixture_name = format!(
        "{} ({})::{case_name}",
        difference.name, difference.capability
    );

    GapCase {
        difference,
        fixture: fixture(
            fixture_name,
            literal,
            declarations,
            schema,
            expected_source,
            expected_mon,
        ),
    }
}
