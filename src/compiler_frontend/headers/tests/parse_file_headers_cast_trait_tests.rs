use super::*;

// ------------------------------
//  Core cast trait name collision tests
// ------------------------------

#[test]
fn header_parsing_rejects_core_cast_trait_source_declaration() {
    let result = parse_single_file_headers_with_entry(
        "CASTABLE_TO_STRING must:\n    to_string |This| -> String\n;\n",
        "src/@page.moth",
        "src/@page.moth",
    );
    let errors = expect_header_error(
        result,
        "source declaration of a core cast trait name must be rejected",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.payload,
        DiagnosticPayload::ReservedNameCollision {
            reserved_by: ReservedNameOwner::CoreTrait,
            ..
        }
    )));
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
fn header_parsing_rejects_selection_alias_to_core_cast_trait_name() {
    let sources = vec![
        (
            "USER_TRAIT must:\n    to_string |This| -> String\n;\n".to_owned(),
            "src/helper.moth".to_owned(),
        ),
        (
            "@helper USER_TRAIT as CASTABLE_TO_STRING\n".to_owned(),
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
