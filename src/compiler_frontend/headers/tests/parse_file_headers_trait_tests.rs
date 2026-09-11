use super::*;

#[test]
fn trait_declaration_headers_parse_requirement_shells() {
    let (headers, string_table) = parse_single_file_headers_with_table(
        "DISPLAYABLE must:\n\
             display |This| -> String\n\
             reset |~This|\n\
             copy_value |This, other This| -> This\n\
         ;\n",
    );

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    let HeaderKind::Trait { declaration } = &trait_header.kind else {
        panic!("expected trait header kind");
    };

    assert_eq!(string_table.resolve(declaration.name), "DISPLAYABLE");
    assert_eq!(declaration.requirements.len(), 3);
    assert_eq!(
        declaration.requirements[0].signature.parameters[0].value_mode,
        ValueMode::ImmutableOwned
    );
    assert_eq!(
        declaration.requirements[1].signature.parameters[0].value_mode,
        ValueMode::MutableOwned
    );

    let copy_requirement = &declaration.requirements[2];
    assert!(matches!(
        copy_requirement.signature.parameters[1].type_annotation,
        ParsedTypeRef::This { .. }
    ));
    assert!(matches!(
        copy_requirement.signature.returns[0].value,
        FunctionReturnSyntax {
            type_annotation: ParsedTypeRef::This { .. },
            ..
        }
    ));
}

#[test]
fn empty_marker_trait_declaration_is_a_valid_header() {
    let headers = parse_single_file_headers("MARKER must:\n;\n");

    let trait_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .expect("expected trait header");

    let HeaderKind::Trait { declaration } = &trait_header.kind else {
        panic!("expected trait header kind");
    };

    assert!(
        declaration.requirements.is_empty(),
        "marker traits should parse with no requirement shells"
    );
}

#[test]
fn trait_conformance_headers_parse_single_and_continued_trait_lists() {
    let (headers, string_table) =
        parse_single_file_headers_with_table("Card must DISPLAYABLE,\n    SERIALIZABLE\n");

    let conformance_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitConformance { .. }))
        .expect("expected trait conformance header");

    let HeaderKind::TraitConformance { conformance } = &conformance_header.kind else {
        panic!("expected trait conformance header kind");
    };

    let trait_names = conformance
        .traits
        .iter()
        .map(|trait_ref| string_table.resolve(trait_ref.name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(string_table.resolve(conformance.target.name), "Card");
    assert_eq!(trait_names, vec!["DISPLAYABLE", "SERIALIZABLE"]);
}

#[test]
fn builtin_type_conformance_headers_parse_as_trait_conformances() {
    let (headers, string_table) = parse_single_file_headers_with_table("Int must DISPLAYABLE\n");

    let conformance_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitConformance { .. }))
        .expect("expected builtin trait conformance header");

    let HeaderKind::TraitConformance { conformance } = &conformance_header.kind else {
        panic!("expected trait conformance header kind");
    };

    assert_eq!(string_table.resolve(conformance.target.name), "Int");
    assert_eq!(
        string_table.resolve(conformance.traits[0].name),
        "DISPLAYABLE"
    );
}

#[test]
fn trait_requirement_rejects_lowercase_this_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |this|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "lowercase this should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::ThisNotAllowed
        }
    )));
}

#[test]
fn trait_requirement_rejects_missing_this_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |value Int|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "trait requirements should start with This");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitReceiverMustBeThis
        }
    )));
}

#[test]
fn trait_requirement_rejects_mutable_this_after_receiver() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, ~This|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "mutable This is receiver-only");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidSignatureMember {
            reason: InvalidSignatureMemberReason::TraitMutableThisOnlyFirstParameter
        }
    )));
}

#[test]
fn trait_requirement_rejects_composed_this_type_forms() {
    let result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This, values {This}|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "composed This forms are deferred");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            reason: InvalidTypeAnnotationReason::TraitThisMustBeDirect,
            ..
        }
    )));
}

#[test]
fn trait_requirement_rejects_method_bodies_and_reversed_mutability() {
    let method_body_result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This|:\n        return \"bad\"\n    ;\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let method_body_errors = expect_header_error(
        method_body_result,
        "trait requirements cannot have method bodies",
    );

    assert!(
        method_body_errors
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind
                == DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTokenInDeclaration))
    );

    let reversed_mutability_result = parse_single_file_headers_with_entry(
        "BAD must:\n    wrong |This ~|\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reversed_mutability_errors = expect_header_error(
        reversed_mutability_result,
        "trait receiver mutability must be written as ~This",
    );

    assert!(
        reversed_mutability_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::ExpectedToken { .. }
            ))
    );
}

#[test]
fn trait_conformance_rejects_missing_trait_name() {
    let result =
        parse_single_file_headers_with_entry("Card must\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "conformance declarations require a trait name");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitConformanceMissingTrait,
            ..
        }
    )));
}

#[test]
fn trait_declaration_and_reference_names_must_be_all_caps() {
    let declaration_result = parse_single_file_headers_with_entry(
        "Displayable must:\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let declaration_errors = expect_header_error(
        declaration_result,
        "trait declarations should require all-caps names",
    );

    assert!(
        declaration_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );

    let conformance_result = parse_single_file_headers_with_entry(
        "Card must Displayable\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let conformance_errors = expect_header_error(
        conformance_result,
        "trait references should require all-caps names",
    );

    assert!(
        conformance_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );
}

#[test]
fn trait_incompatibility_headers_parse_single_and_continued_trait_lists() {
    let (headers, string_table) = parse_single_file_headers_with_table("A must not B,\n    C\n");

    let incompatibility_header = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::TraitIncompatibility { .. }))
        .expect("expected trait incompatibility header");

    let HeaderKind::TraitIncompatibility { incompatibility } = &incompatibility_header.kind else {
        panic!("expected trait incompatibility header kind");
    };

    let trait_names = incompatibility
        .incompatible_traits
        .iter()
        .map(|trait_ref| string_table.resolve(trait_ref.name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(string_table.resolve(incompatibility.subject.name), "A");
    assert_eq!(trait_names, vec!["B", "C"]);
}

#[test]
fn trait_incompatibility_reuses_subject_trait_name_without_duplicate_error() {
    let headers = parse_single_file_headers(
        "A must:\n\
         ;
\
         A must not B\n",
    );

    let trait_headers = headers
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::Trait { .. }))
        .count();
    let incompatibility_headers = headers
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::TraitIncompatibility { .. }))
        .count();

    assert_eq!(trait_headers, 1, "subject trait should parse once");
    assert_eq!(
        incompatibility_headers, 1,
        "incompatibility declaration should parse without duplicate-name error"
    );
}

#[test]
fn trait_incompatibility_rejects_missing_trait_name() {
    let result =
        parse_single_file_headers_with_entry("A must not\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "incompatibility declarations require at least one trait name",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitIncompatibilityMissingTrait,
            ..
        }
    )));
}

#[test]
fn trait_incompatibility_rejects_trailing_comma() {
    let result =
        parse_single_file_headers_with_entry("A must not B,\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(result, "trailing incompatibility commas should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedTrailingComma
    )));
}

#[test]
fn trait_incompatibility_rejects_semicolon_terminator() {
    let result =
        parse_single_file_headers_with_entry("A must not B;\n", "src/@page.moth", "src/@page.moth");
    let errors = expect_header_error(
        result,
        "incompatibility declarations should be newline terminated",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::TraitIncompatibilitySemicolon,
            ..
        }
    )));
}

#[test]
fn trait_incompatibility_subject_and_reference_names_must_be_all_caps() {
    let declaration_result = parse_single_file_headers_with_entry(
        "Displayable must not Other\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let declaration_errors = expect_header_error(
        declaration_result,
        "incompatibility subjects should require all-caps names",
    );

    assert!(
        declaration_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );

    let reference_result = parse_single_file_headers_with_entry(
        "DISPLAYABLE must not Other\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let reference_errors = expect_header_error(
        reference_result,
        "incompatibility trait references should require all-caps names",
    );

    assert!(
        reference_errors
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::InvalidTraitName,
                    ..
                }
            ))
    );
}

#[test]
fn trait_this_outside_trait_declaration_is_targeted() {
    let result = parse_single_file_headers_with_entry(
        "value This = 1\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(result, "This outside trait declarations should be rejected");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidThisUsage {
            reason: InvalidThisUsageReason::OutsideTraitDeclaration
        }
    )));
}
