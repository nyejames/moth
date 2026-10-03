//! Qualified choice separator spacing parity.
//!
//! WHAT: checks that source and MON both reject trivia around a qualified `::` separator.
//! WHY: `Choice::Variant` is one lexical header in both parsers, while spaced `Name ::` remains
//! valid only for source choice declarations.

use crate::compiler_frontend::compiler_messages::{
    ChoiceVariantSeparatorGap, CommonSyntaxMistakeReason, DiagnosticPayload, NameNamespace,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{
    Field, MonErrorCode, PathSegment, Schema, SchemaType, Span, Variant, decode_document,
};

use super::support::{
    MonExpectation, SourceExpectation, SourceReason, assert_fixture, fixture, in_literal, mon_span,
};

fn theme_schema() -> Schema {
    Schema::record(vec![Field::required(
        "status",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![Variant::unit("Ready")],
        },
    )])
}

fn mon_separator_gap_span(literal: &str, gap: ChoiceVariantSeparatorGap) -> Span {
    match gap {
        ChoiceVariantSeparatorGap::Before | ChoiceVariantSeparatorGap::Both => {
            mon_span(literal, "Theme", 1)
        }
        ChoiceVariantSeparatorGap::After => {
            let separator_end = literal.find("::").expect("fixture has a separator") + 2;
            let first_gap_character = literal[separator_end..]
                .chars()
                .next()
                .expect("fixture has trivia after the separator");
            let gap_text = &literal[separator_end..separator_end + first_gap_character.len_utf8()];
            let occurrence = literal[..separator_end].match_indices(gap_text).count() + 1;
            mon_span(literal, gap_text, occurrence)
        }
    }
}

fn invalid_separator_fixture(
    name: &str,
    literal: &str,
    gap: ChoiceVariantSeparatorGap,
) -> super::support::ParityFixture {
    let mon_code = match gap {
        ChoiceVariantSeparatorGap::Before | ChoiceVariantSeparatorGap::Both => {
            MonErrorCode::UnexpectedToken
        }
        ChoiceVariantSeparatorGap::After => MonErrorCode::InvalidIdentifier,
    };

    fixture(
        name,
        literal,
        "Theme :: Ready;\n",
        theme_schema(),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0031",
            reason: SourceReason::CommonSyntaxMistake(
                CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap },
            ),
            site: in_literal(literal, "::", 1),
        },
        MonExpectation::Reject {
            code: mon_code,
            span: mon_separator_gap_span(literal, gap),
            path: vec![PathSegment::Field("status".into())],
        },
    )
}

#[test]
fn source_and_mon_reject_nonadjacent_choice_separators() {
    let _guard = lock_counter_test();
    for (name, literal, gap) in [
        (
            "space_before_choice_separator",
            "status = Theme ::Ready",
            ChoiceVariantSeparatorGap::Before,
        ),
        (
            "tab_before_choice_separator",
            "status = Theme\t::Ready",
            ChoiceVariantSeparatorGap::Before,
        ),
        (
            "space_after_choice_separator",
            "status = Theme:: Ready",
            ChoiceVariantSeparatorGap::After,
        ),
        (
            "tab_after_choice_separator",
            "status = Theme::\tReady",
            ChoiceVariantSeparatorGap::After,
        ),
        (
            "both_sides_of_choice_separator",
            "status = Theme :: Ready",
            ChoiceVariantSeparatorGap::Both,
        ),
        (
            "line_break_after_choice_separator",
            "status = Theme::\nReady",
            ChoiceVariantSeparatorGap::After,
        ),
        (
            "comment_after_choice_separator",
            "status = Theme::-- note\nReady",
            ChoiceVariantSeparatorGap::After,
        ),
    ] {
        assert_fixture(invalid_separator_fixture(name, literal, gap));
    }
}

#[test]
fn source_and_mon_reject_pre_separator_line_breaks_and_comments() {
    let _guard = lock_counter_test();
    let prepared_schema = theme_schema()
        .prepare()
        .expect("the paired theme schema is valid");

    for (name, literal) in [
        (
            "line_break_before_choice_separator",
            "status = Theme\n::Ready",
        ),
        (
            "comment_before_choice_separator",
            "status = Theme -- note\n::Ready",
        ),
    ] {
        let source = format!("Theme :: Ready;\n{literal}\n");
        let diagnostic =
            crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic(
                &source,
            );
        assert_eq!(
            diagnostic.identity().code,
            "MOTH-RULE-0037",
            "source case: {name}"
        );
        assert!(matches!(
            &diagnostic.payload,
            DiagnosticPayload::NamespaceMisuse {
                expected: NameNamespace::Value,
                found: NameNamespace::Type,
                ..
            }
        ));

        let error = decode_document(literal, &prepared_schema)
            .expect_err("MON must reject trivia before a qualified separator");
        assert_eq!(
            error.code,
            MonErrorCode::UnexpectedToken,
            "MON case: {name}"
        );
        assert_eq!(
            error.span,
            Some(mon_span(literal, "Theme", 1)),
            "MON span for {name}"
        );
        assert_eq!(
            error.path,
            vec![PathSegment::Field("status".into())],
            "MON path for {name}"
        );
    }
}
