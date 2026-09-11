use super::*;

// ---------------------------------------------------------------------------
//  Provider-independent preparation vs provider-dependent prelude collisions
// ---------------------------------------------------------------------------
#[test]
fn prelude_symbol_declaration_prepared_without_registry_then_collides_at_binding() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    // Preparation takes no registry input, so a declaration that reuses a prelude symbol name
    // still parses into a retained declaration shell during provider-independent preparation.
    let (output, mut span_builder) = prepare_single_file(
        "prelude_fn |x Int| -> Int:\n    return x\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "provider-independent preparation should retain the prelude-named declaration shell"
    );
    let expected_span = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("expected retained prelude-named function shell")
        .name_span
        .expect("retained declaration should keep its authored name span");

    let registry = registry_with_prelude_function_symbol("prelude_fn");
    let result = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &registry,
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    );
    let binding_error = match result {
        Ok(_) => panic!("binding should reject a declaration that collides with a prelude symbol"),
        Err(bag) => bag,
    };
    let diagnostic = binding_error
        .diagnostics()
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.kind,
                DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName)
            )
        })
        .expect("binding should preserve the reserved builtin-name diagnostic");
    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0027");
    assert_eq!(diagnostic.primary_span, Some(expected_span));
}

#[test]
fn prelude_type_generic_parameter_prepared_without_registry_then_collides_at_binding() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    // A generic parameter reusing a prelude type name parses during provider-independent
    // preparation; the collision is provider-dependent and is validated during binding.
    let (output, mut span_builder) = prepare_single_file(
        "Box type PreludeType = |\n    value PreludeType,\n|\n",
        &file_path,
        &file_path,
        &mut string_table,
    );
    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Struct { .. })),
        "provider-independent preparation should retain the generic declaration shell"
    );

    let registry = registry_with_prelude_type_symbol("PreludeType");
    let result = prepare_and_bind_headers_result(
        vec![output],
        std::slice::from_mut(&mut span_builder),
        &registry,
        &ExternalImportResolutionTable::default(),
        None,
        &mut string_table,
    );
    let binding_error = match result {
        Ok(_) => panic!("binding should reject a generic parameter naming a prelude type"),
        Err(bag) => bag,
    };
    assert!(
        binding_error.diagnostics().iter().any(|diagnostic| {
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidDeclaration {
                    reason: InvalidDeclarationReason::GenericParameterNameCollision { .. },
                    ..
                }
            )
        }),
        "prelude-type generic parameter collision should be diagnosed during binding"
    );
}

#[test]
fn direct_selection_does_not_reserve_provider_basename_for_generic_parameter() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "@core/Math add\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert!(
        output
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "direct selections must not reserve the provider basename as a local namespace"
    );
}

#[test]
fn direct_selection_alias_still_reserves_effective_local_name_for_generic_parameter() {
    for source in [
        "@core/math add as Math\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        "identity type Math |value Math| -> Math:\n    return value\n;\n@core/math add as Math\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}

#[test]
fn namespace_provider_basename_reserves_generic_parameter_name_in_either_source_order() {
    for source in [
        "@core/Math\nidentity type Math |value Math| -> Math:\n    return value\n;\n",
        "identity type Math |value Math| -> Math:\n    return value\n;\n@core/Math\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}

#[test]
fn explicit_extension_namespace_stem_reserves_generic_parameter_name_in_either_source_order() {
    for source in [
        "@Drawing.js as Drawing\nidentity type Drawing |value Drawing| -> Drawing:\n    return value\n;\n",
        "identity type Drawing |value Drawing| -> Drawing:\n    return value\n;\n@Drawing.js as Drawing\n",
    ] {
        assert_generic_dependency_name_collision(source);
    }
}
