use super::*;

#[test]
fn missing_default_value_after_assign_points_at_member_boundary() {
    // An authored `=` for an ordinary parameter or struct field must be followed by a
    // default expression. Each case below reaches a distinct member/EOF boundary before
    // any expression token begins, so the shared member-default owner reports
    // `MissingDefaultValue` (MOTH-RULE-0028) pointing at that boundary, not a generic
    // unexpected-token or end-of-file failure.
    let cases: &[(&str, Option<(&str, usize)>)] = &[
        // function parameter ending at the closing pipe
        ("label |prefix String =| -> String:\n;\n", Some(("|", 1))),
        // struct field ending at a comma
        ("Options = |\n    width Int =,\n|\n", Some((",", 0))),
        // struct field ending at the closing pipe
        ("Options = |\n    width Int =|\n", Some(("|", 1))),
        // newline immediately after the authored `=`
        ("label |prefix String =\n| -> String:\n;\n", Some(("\n", 0))),
        // block end (`;`) immediately after the authored `=`
        ("label |prefix String =;\n", Some((";", 0))),
        // end of file immediately after the authored `=`
        ("label |prefix String =", None),
    ];

    for (source, boundary) in cases {
        let result =
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
        let errors =
            expect_header_error(result, "an authored `=` with no value should be rejected");
        let diagnostic = errors
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.payload,
                    DiagnosticPayload::InvalidSignatureMember {
                        reason: InvalidSignatureMemberReason::MissingDefaultValue
                    }
                )
            })
            .expect("expected a MissingDefaultValue signature-member diagnostic");
        assert_eq!(
            diagnostic.kind.code(),
            "MOTH-RULE-0028",
            "MissingDefaultValue must keep the stable signature-member code"
        );
        let expected_span = boundary.map_or_else(
            || {
                // The final empty match is the exact zero-width span at EOF.
                source_span_for(source, "", source.len())
            },
            |(needle, occurrence)| source_span_for(source, needle, occurrence),
        );
        assert_eq!(
            diagnostic.primary_span,
            Some(expected_span),
            "MissingDefaultValue should point at the authored boundary for: {source}",
        );
    }
}

#[test]
fn special_member_default_reasons_win_over_missing_default_value() {
    // Reactive parameters, trait requirements and choice payload fields keep their own
    // more specific default-value reasons even when no expression follows the authored
    // `=`, so they never fall through to `MissingDefaultValue`.
    let reactive = parse_single_file_headers_with_entry(
        "label |event $String =| -> String:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reactive_errors =
        expect_header_error(reactive, "reactive parameter defaults must be rejected");
    assert!(
        reactive_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidSignatureMember {
                    reason: InvalidSignatureMemberReason::ReactiveParameterDefaultValue
                }
            ))
    );

    let trait_requirement = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, value Int =|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let trait_errors = expect_header_error(
        trait_requirement,
        "trait requirement defaults must be rejected",
    );
    assert!(trait_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitRequirementDefaultValue
        }
    )));

    let choice = parse_single_file_headers_with_entry(
        "Response ::\n    Err |\n        message String =|,\n    Success,\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let choice_errors =
        expect_header_error(choice, "choice payload field defaults must be rejected");
    assert!(choice_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::ChoicePayloadDefaultValue
        }
    )));
}

#[test]
fn authored_default_expression_survives_newline_and_multiline_continuation() {
    // A default that begins with a real expression token before any boundary stays valid.
    // The early missing-default check only fires before the first expression token, so a
    // value followed by a newline member boundary and an operator-continued multiline
    // default both parse successfully.
    let (single_line_then_newline, _string_table) =
        parse_single_file_headers_with_table("label |prefix String = \"a\"\n| -> String:\n;\n");
    let signature = first_function_signature(&single_line_then_newline);
    assert!(
        signature.parameters.iter().any(|parameter| parameter
            .default_tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))),
        "a default that begins before a newline should be captured"
    );

    let (multiline, _string_table) = parse_single_file_headers_with_table(
        "label |prefix String = \"a\" +\n    \"b\"| -> String:\n;\n",
    );
    let multiline_signature = first_function_signature(&multiline);
    assert!(
        multiline_signature
            .parameters
            .iter()
            .any(|parameter| parameter
                .default_tokens
                .iter()
                .filter(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
                .count()
                == 2),
        "an operator-continued multiline default should fold both string literals"
    );
}
