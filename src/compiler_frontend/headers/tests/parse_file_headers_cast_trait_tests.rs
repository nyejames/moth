use super::*;

// ------------------------------
//  Core cast trait name collision tests
// ------------------------------

#[test]
fn header_parsing_rejects_core_cast_trait_source_declaration() {
    for (trait_name, method_name, return_types) in [
        ("CASTABLE_TO_I8", "to_i8", "I8"),
        ("TRY_CASTABLE_TO_F64", "try_to_f64", "F64, Error!"),
    ] {
        let source = format!("{trait_name} must:\n    {method_name} |This| -> {return_types}\n;\n");
        let result =
            parse_single_file_headers_with_entry(&source, "src/@page.moth", "src/@page.moth");
        let error_message =
            format!("source declaration of {trait_name} must be rejected as a core cast trait");
        let errors = expect_header_error(result, &error_message);

        assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.payload,
            DiagnosticPayload::ReservedNameCollision {
                reserved_by: ReservedNameOwner::CoreTrait,
                ..
            }
        )));
    }
}

#[test]
fn header_parsing_allows_displayable_source_declaration() {
    let headers = parse_single_file_headers("DISPLAYABLE must:\n    display |This| -> String\n;\n");

    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Trait { .. })),
        "DISPLAYABLE declarations must remain valid outside the core cast trait hardening slice"
    );
}

#[test]
fn header_parsing_allows_retired_profile_cast_names_as_user_traits() {
    for (trait_name, method_name, target_name) in [
        ("CASTABLE_TO_INT", "to_int", "Int"),
        ("TRY_CASTABLE_TO_INT", "try_to_int", "Int"),
        ("CASTABLE_TO_FLOAT", "to_float", "Float"),
        ("TRY_CASTABLE_TO_FLOAT", "try_to_float", "Float"),
    ] {
        let source = format!("{trait_name} must:\n    {method_name} |This| -> {target_name}\n;\n");
        let headers = parse_single_file_headers(&source);
        assert_eq!(
            headers
                .headers
                .iter()
                .filter(|header| matches!(header.kind, HeaderKind::Trait { .. }))
                .count(),
            1,
            "{trait_name} must follow ordinary user-trait name resolution"
        );
    }
}

#[test]
fn header_parsing_rejects_selection_alias_to_core_cast_trait_name() {
    let sources = vec![
        (
            "USER_TRAIT must:\n    to_string |This| -> String\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
        (
            "@helper USER_TRAIT as TRY_CASTABLE_TO_F64\n".to_owned(),
            "src/@page.moth".to_owned(),
        ),
    ];

    let (result, _warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");
    assert!(
        result.is_err(),
        "a dependency selection alias to a core cast trait name must be rejected"
    );

    let errors = result.err().expect("expected parse errors");
    assert!(errors.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
}

#[test]
fn header_parsing_rejects_module_public_surface_re_export_with_core_cast_trait_name() {
    let sources = vec![
        (
            "USER_TRAIT must:\n    to_string |This| -> String\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
        (
            "export:\n    @helper USER_TRAIT as CASTABLE_TO_STRING\n;\n".to_owned(),
            "src/@mod.moth".to_owned(),
        ),
        ("@helper\n".to_owned(), "src/@page.moth".to_owned()),
    ];

    let (result, _warnings, _string_table) =
        parse_multi_file_headers_with_result(&sources, "src/@page.moth");
    assert!(
        result.is_err(),
        "module public-surface re-export under a core cast trait name must be rejected"
    );

    let errors = result.err().expect("expected parse errors");
    assert!(errors.diagnostics().iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
}
