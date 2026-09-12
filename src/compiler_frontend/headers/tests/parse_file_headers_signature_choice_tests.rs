use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

#[test]
fn function_signature_reports_missing_arrow_before_return_type() {
    let result = parse_single_file_headers_with_entry(
        "f|x Int| Int:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        result.is_err(),
        "missing arrow before return type must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingArrowOrColon { .. }
        }
    )));
}

#[test]
fn function_signature_reports_missing_colon_after_return_list() {
    let result =
        parse_single_file_headers_with_entry("f|| -> Int\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "missing ':' after return declarations must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingColonAfterReturns
        }
    )));
}

#[test]
fn function_signature_reports_missing_return_type_after_arrow_colon() {
    // An authored `->` immediately followed by `:` has no return type. The signature
    // parser owns this boundary and must report `MissingReturnType` at the colon rather
    // than the function name or parameter list.
    let result =
        parse_single_file_headers_with_entry("f|| -> :\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "an arrow immediately followed by ':' must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingReturnType
        }
    )));
}

#[test]
fn function_signature_reports_missing_return_type_after_arrow_newline() {
    // A newline immediately after `->` is a missing-return-type boundary, not a valid
    // empty return list. The signature parser reports `MissingReturnType` at the newline.
    let result =
        parse_single_file_headers_with_entry("f|| ->\n;\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "an arrow immediately followed by a newline must fail"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingReturnType
        }
    )));
}

#[test]
fn trait_requirement_reports_missing_return_type_after_arrow_colon() {
    // A trait requirement is bodyless, so an authored `->` followed by `:` is a missing
    // return type. The shared boundary predicate routes the trait-requirement parser to
    // `MissingTraitRequirementReturnType`, which never tells a bodyless requirement to add
    // the function-body `:` terminator. The diagnostic points at the first missing-type
    // boundary after the arrow, not at the requirement name or `This` receiver.
    let source = "DISPLAYABLE must:\n    display |This| -> :\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "a trait requirement arrow followed by ':' must fail"
    );
    let errors = result.err().expect("expected parse errors");

    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidFunctionSignature {
                    reason: InvalidFunctionSignatureReason::MissingTraitRequirementReturnType
                }
            )
        })
        .expect("expected MissingTraitRequirementReturnType");

    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, ":", 1)),
        "missing return type should point at the authored boundary after `->`",
    );
}

#[test]
fn trait_requirement_reports_missing_return_type_after_arrow_newline() {
    // A newline after `->` is also a missing-return-type boundary for a trait requirement.
    // The requirement-specific reason is used so the guidance never suggests the function
    // body `:` terminator.
    let source = "DISPLAYABLE must:\n    display |This| ->\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "a trait requirement arrow followed by a newline must fail"
    );
    let errors = result.err().expect("expected parse errors");

    let diagnostic = errors
        .diagnostics
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidFunctionSignature {
                    reason: InvalidFunctionSignatureReason::MissingTraitRequirementReturnType
                }
            )
        })
        .expect("expected MissingTraitRequirementReturnType");
    assert_eq!(
        diagnostic.primary_span,
        Some(source_span_for(source, "\n", 1)),
        "missing return type should point at the authored newline boundary",
    );
}

#[test]
fn duplicate_top_level_function_names_error_during_header_parsing() {
    let result = parse_single_file_headers_with_entry(
        "simple_function |number Int| -> Int:\n\
             return number + 1\n\
         ;\n\
         \n\
         simple_function |value Int| -> Int:\n\
             return value + 2\n\
         ;\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "duplicate top-level function names should fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        )
    }));
}

#[test]
fn duplicate_header_detection_ignores_qualified_match_arms() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_file = path_fork.try_intern_portable_path("src/@page.moth", &mut string_table).expect("test path fits");
    let status = string_table.intern("Status");
    let ready = string_table.intern("Ready");
    let write = string_table.intern("write");
    let span = LocalSpan::source_start();

    let mut token_stream = FileTokens::new(
        source_file,
        SourceId::COMPILATION_ROOT,
        vec![
            Token::new(TokenKind::Symbol(status), span),
            Token::new(TokenKind::DoubleColon, span),
            Token::new(TokenKind::Symbol(ready), span),
            Token::new(TokenKind::FatArrow, span),
            Token::new(TokenKind::Symbol(write), span),
            Token::new(TokenKind::Eof, span),
        ],
    );
    token_stream.index = 1;

    assert!(
        !super::super::super::top_level_classifier::starts_duplicate_top_level_header_declaration(
            &token_stream
        ),
        "qualified match arms in the start body are not choice declarations"
    );
}

#[test]
fn choice_headers_parse_unit_variants_in_declaration_order() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Status :: Ready,\nBusy,\nIdle,\n;\n");
    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    let HeaderKind::Choice { variants, .. } = &choice_header.kind else {
        panic!("expected choice metadata");
    };

    assert_eq!(variants.len(), 3, "expected three parsed variants");
    assert_eq!(string_table.resolve(variants[0].id), "Ready");
    assert_eq!(string_table.resolve(variants[1].id), "Busy");
    assert_eq!(string_table.resolve(variants[2].id), "Idle");
}

#[test]
fn choice_headers_reject_duplicate_variants() {
    let result = parse_single_file_headers_with_entry(
        "Status :: Ready, Ready;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(result.is_err(), "duplicate choice variants must fail");
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        )
    }));
}

#[test]
fn choice_headers_reject_invalid_payload_forms() {
    // Shorthand payload is invalid by design (not deferred).
    let payload_shorthand_result = parse_single_file_headers_with_entry(
        "Status :: Ready String;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        payload_shorthand_result.is_err(),
        "shorthand payload variants must be rejected"
    );
    let payload_errors = payload_shorthand_result
        .err()
        .expect("expected payload parse errors");
    assert!(payload_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidChoiceVariant {
            reason: InvalidChoiceVariantReason::PayloadShorthandNotSupported,
            ..
        }
    )));

    // Constructor-style declarations are invalid by design.
    let payload_paren_result = parse_single_file_headers_with_entry(
        "Status :: Ready(String);\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        payload_paren_result.is_err(),
        "constructor-style payload variants must be rejected"
    );
    let payload_paren_errors = payload_paren_result
        .err()
        .expect("expected constructor-style payload parse errors");
    assert!(
        payload_paren_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidChoiceVariant {
                    reason: InvalidChoiceVariantReason::ConstructorStyleNotSupported,
                    ..
                }
            ))
    );

    // Default values remain deferred.
    let defaults_result = parse_single_file_headers_with_entry(
        "Status :: Ready = true;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    assert!(
        defaults_result.is_err(),
        "choice variant defaults must fail"
    );
    let default_errors = defaults_result
        .err()
        .expect("expected default parse errors");
    assert!(default_errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::DeferredFeature {
            reason: DeferredFeatureReason::ChoiceVariantDefaultValue
        }
    )));
}

#[test]
fn choice_headers_accept_record_payload_variants() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Status :: Pending |\n    RetryCount Int,\n|;\n");

    let choice_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Choice { .. }))
        .expect("expected choice header");

    let HeaderKind::Choice { variants, .. } = &choice_header.kind else {
        panic!("expected choice metadata");
    };

    assert_eq!(variants.len(), 1, "expected one parsed variant");
    assert_eq!(
        string_table.resolve(variants[0].id),
        "Pending",
        "expected Pending variant"
    );
    match &variants[0].payload {
        ChoiceVariantPayloadSyntax::Record { fields } => {
            assert_eq!(fields.len(), 1, "expected one payload field");
            assert_ne!(
                fields[0].id,
                PathId::ROOT,
                "expected RetryCount field to have a non-root path"
            );
        }
        other => panic!("expected Record payload, got {other:?}"),
    }
}

#[test]
fn header_parsing_emits_naming_warnings_for_non_camel_type_like_symbols() {
    let (headers, warnings) = parse_single_file_headers_with_warnings(
        "SITE_TITLE #= \"Moth\"\nStatus_type :: bad_variant;\n",
    );

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Choice { .. })),
        "fixture should still parse a choice header"
    );
    assert_eq!(
        warnings.len(),
        2,
        "expected warnings for choice name and variant only; uppercase constant should be allowed"
    );
    assert!(
        warnings
            .iter()
            .all(|warning| matches!(
                warning.kind,
                crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
                    crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::IdentifierNamingConvention
                )
            )),
        "expected naming convention warnings for choice name and variant only"
    );
}

#[test]
fn header_parsing_rejects_keyword_shadow_constant_name() {
    let result =
        parse_single_file_headers_with_entry("FALSE #= 1\n", "src/@page.moth", "src/@page.moth");
    assert!(
        result.is_err(),
        "keyword-shadow top-level constants must fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::Keyword,
            ..
        }
    )));
}

#[test]
fn trait_declarations_using_must_parse_as_trait_headers() {
    let headers = parse_single_file_headers("DISPLAYABLE must:\n    display |This| -> String\n;\n");

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Trait { .. })),
        "trait declarations using 'must:' should produce trait headers"
    );
}

#[test]
fn generic_type_aliases_are_rejected_during_header_parsing() {
    let result = parse_single_file_headers_with_entry(
        "Response type T as ResultShape of T, Error\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "generic type aliases should fail during header parsing"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration)
            && matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::ParameterizedGenericTypeAlias,
                    ..
                }
            )
    }));
}
