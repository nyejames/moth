use super::*;

#[test]
fn source_config_contracts_are_normalized_in_authored_order() {
    let prepared = prepare_source_contract_syntax(
        "analytics #Config of Bool = false\n\
         optional_label #Config of String? = none\n\
         required_count #Config of Int\n",
    )
    .expect("source config contracts should prepare without providers");

    assert_eq!(prepared.source_build_config_contracts.len(), 3);
    let contracts = &prepared.source_build_config_contracts;
    assert_eq!(contracts[0].name.as_str(), "analytics");
    assert_eq!(
        contracts[0].value_type,
        crate::compiler_frontend::build_config::BuildInputType::Primitive(
            crate::compiler_frontend::build_config::PrimitiveBuildInputType::Bool
        )
    );
    assert_eq!(
        contracts[0].default,
        Some(crate::compiler_frontend::build_config::PrimitiveBuildValue::Bool(false))
    );
    assert!(!contracts[0].required);

    assert_eq!(contracts[1].name.as_str(), "optional_label");
    assert_eq!(
        contracts[1].value_type,
        crate::compiler_frontend::build_config::BuildInputType::Optional(
            crate::compiler_frontend::build_config::PrimitiveBuildInputType::String
        )
    );
    assert_eq!(contracts[1].default, None);
    assert!(!contracts[1].required);

    assert_eq!(contracts[2].name.as_str(), "required_count");
    assert!(contracts[2].required);
    assert_eq!(contracts[2].default, None);
    assert!(
        prepared
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Constant { .. })),
        "the declaration shell must remain retained for the later config barrier"
    );
}

#[test]
fn source_config_contracts_reject_invalid_name_type_and_default_during_preparation() {
    let cases = [
        ("BadName #Config of Bool = false\n", "name"),
        ("value #Config of Decimal = 1\n", "type"),
        ("value #Config of Int = other + 1\n", "default"),
    ];

    for (source, expected_kind) in cases {
        let error = match prepare_source_contract_syntax(source) {
            Ok(_) => panic!("invalid source contract should fail before binding"),
            Err(error) => error,
        };
        let diagnostic = error
            .into_diagnostics()
            .into_iter()
            .next()
            .expect("expected source contract diagnostic");
        let matches_expected = match expected_kind {
            "name" => matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::ConfigContractNameInvalid,
                    ..
                }
            ),
            "type" => matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::ConfigQualifierUnsupportedType,
                    ..
                }
            ),
            "default" => matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::ConfigInputTypeMismatch { .. },
                    ..
                }
            ),
            _ => false,
        };
        assert!(
            matches_expected,
            "expected {expected_kind} source contract diagnostic, got {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn source_config_contracts_reject_nested_and_runtime_qualifiers_before_ast() {
    let cases = [
        "settings #= | flag #Config of Bool = false |\n",
        "load |input Bool| -> Bool:\n\
             runtime #Config of Bool = true\n\
             return input\n\
         ;\n",
    ];

    for source in cases {
        let error = match prepare_source_contract_syntax(source) {
            Ok(_) => panic!("nested or runtime #Config must fail during header preparation"),
            Err(error) => error,
        };
        let diagnostic = error
            .into_diagnostics()
            .into_iter()
            .next()
            .expect("expected source placement diagnostic");
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::ConfigQualifierInvalidPlacement,
                ..
            }
        ));
    }
}

#[test]
fn source_config_contracts_stay_out_of_header_topology_and_provider_symbols() {
    let ordinary = prepare_source_contract_syntax("@core/math\nanalytics #= false\n")
        .expect("ordinary source should prepare without providers");
    let prepared =
        prepare_source_contract_syntax("@core/math\nanalytics #Config of Bool = false\n")
            .expect("source config contract should prepare without providers");
    assert_eq!(
        ordinary.module_symbols.module_file_paths, prepared.module_symbols.module_file_paths,
        "contract collection must not change the prepared module file set"
    );
    assert_eq!(
        ordinary
            .module_symbols
            .file_dependency_clauses_by_source
            .len(),
        prepared
            .module_symbols
            .file_dependency_clauses_by_source
            .len(),
        "contract collection must not change provider clause count"
    );
    assert!(
        ordinary
            .module_symbols
            .file_dependency_clauses_by_source
            .keys()
            .all(|source| {
                prepared
                    .module_symbols
                    .file_dependency_clauses_by_source
                    .contains_key(source)
            }),
        "contract collection must not change provider clause source identities"
    );
    let header = prepared
        .headers
        .iter()
        .find(|header| {
            matches!(
                &header.kind,
                HeaderKind::Constant { declaration }
                    if declaration.config_qualifier.is_some()
            )
        })
        .expect("expected source config constant header");

    assert!(
        header.local_ordering_hints.is_empty(),
        "contract type must not create ordinary local ordering edges"
    );
    assert!(
        !prepared
            .module_symbols
            .dependency_bindable_source_symbol_paths
            .contains(&header.tokens.src_path),
        "contract shell must not enter provider-bindable source symbols"
    );
}

#[test]
fn source_config_initializer_paths_stay_out_of_structural_file_references() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@page.moth");
    let (output, _span_builder) = prepare_single_file(
        "asset #Config of String = @assets/missing.mtf\n",
        &file_path,
        &file_path,
        &mut string_table,
    );

    assert!(
        output.structural_file_references.references().is_empty(),
        "config defaults must not enter Stage 0 file-value references before validation"
    );
    let header = output
        .headers
        .iter()
        .find(|header| {
            matches!(
                &header.kind,
                HeaderKind::Constant { declaration }
                    if declaration.config_qualifier.is_some()
            )
        })
        .expect("expected source config constant header");
    assert!(
        header.local_ordering_hints.is_empty(),
        "config defaults must not add content-source ordering hints"
    );
}

#[test]
fn imported_root_discarded_body_config_marker_is_rejected() {
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(
            "value = @assets/missing.mtf #Config of String\n",
            "src/@page.moth",
            "src/@root.moth",
        ),
        "discarded imported-root body config markers must be rejected during preparation",
    );

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::ConfigQualifierInvalidPlacement,
            ..
        }
    )));
}

#[test]
fn source_config_contracts_reject_generic_parameters_and_comma_termination() {
    let cases = [
        "analytics type T #Config of Bool = false\n",
        "analytics #Config of Bool,\ntrailing #= 1\n",
    ];

    for source in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@page.moth", "src/@page.moth"),
            "malformed source config declaration should fail during header preparation",
        );
        assert!(
            !errors.diagnostics.is_empty(),
            "expected a source config declaration diagnostic for {source:?}"
        );
    }
}

#[test]
fn malformed_children_wrapper_constant_initializer_reports_eof_delimiter_error() {
    let result = parse_single_file_headers_with_entry(
        "broken #= [$children([:<li>[$slot]</li>):\n<ul>[$slot]</ul>\n]\n",
        "src/@page.moth",
        "src/@page.moth",
    );

    assert!(
        result.is_err(),
        "unterminated '$children(..)' wrapper templates should fail instead of hanging"
    );
    let errors = result.err().expect("expected parse errors");

    assert!(errors.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedEndOfFile { .. }
    )));
}
