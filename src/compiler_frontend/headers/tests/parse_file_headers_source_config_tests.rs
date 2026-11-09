use super::*;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[test]
fn source_config_contracts_are_normalized_in_authored_order() {
    let prepared = prepare_source_contract_syntax(
        "$config\nanalytics #Bool = false\n\
         $config\noptional_label #String? = none\n\
         $config\nrequired_count #I32\n",
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
fn source_config_numeric_defaults_materialise_at_their_declared_scalar() {
    let prepared = prepare_source_contract_syntax("$config\ncount #I64 = 4_000_000_000\n")
        .expect("an I64 default inside its range should prepare");

    assert_eq!(
        prepared.source_build_config_contracts[0].default,
        Some(
            crate::compiler_frontend::build_config::PrimitiveBuildValue::Fixed(
                FixedScalarValue::signed(FixedScalar::I64, 4_000_000_000)
                    .expect("4_000_000_000 is inside the I64 range"),
            )
        )
    );

    let error = match prepare_source_contract_syntax("$config\ncount #I32 = 4_000_000_000\n") {
        Ok(_) => panic!("an I32 default outside its range should fail during header preparation"),
        Err(error) => error,
    };
    let diagnostics = compact_bag_for_test(error, None);
    let diagnostic = diagnostics
        .first()
        .expect("expected a number literal diagnostic");
    let facts = diagnostic
        .family_view::<token_lexical::InvalidNumberLiteralView>()
        .expect("expected a typed number literal diagnostic");
    assert_eq!(
        facts.code,
        token_lexical::NumberLiteralCode::OutsideFixedScalarRange
    );
    assert_eq!(facts.scalar, Some(FixedScalar::I32));
}

#[test]
fn source_config_contracts_reject_invalid_name_type_and_default_during_preparation() {
    let cases = [
        ("$config\nBadName #Bool = false\n", "name"),
        ("$config\nvalue #Decimal = 1\n", "type"),
        ("$config\nvalue #I32 = other + 1\n", "default"),
        ("$config\nvalue #I32 = 1 + 2\n", "default"),
    ];

    for (source, expected_kind) in cases {
        let error = match prepare_source_contract_syntax(source) {
            Ok(_) => panic!("invalid source contract should fail before binding"),
            Err(error) => error,
        };
        let diagnostics = compact_bag_for_test(error, None);
        let diagnostic = diagnostics
            .first()
            .expect("expected source contract diagnostic");
        let expected_reason = match expected_kind {
            "name" => "invalid_config.config_contract_name_invalid",
            "type" => "invalid_config.config_qualifier_unsupported_type",
            "default" => "invalid_config.config_input_type_mismatch",
            _ => "",
        };
        assert_eq!(
            diagnostic.reason_key(),
            Some(expected_reason),
            "expected {expected_kind} source contract diagnostic"
        );
    }
}

#[test]
fn source_config_contracts_stay_out_of_header_topology_and_provider_symbols() {
    let ordinary = prepare_source_contract_syntax("@core/math\nanalytics #= false\n")
        .expect("ordinary source should prepare without providers");
    let prepared = prepare_source_contract_syntax("@core/math\n$config\nanalytics #Bool = false\n")
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
                    if declaration.config_contract.is_some()
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
            .contains(&header.declaration_path),
        "contract shell must not enter provider-bindable source symbols"
    );
}

#[test]
fn source_config_initializer_paths_stay_out_of_structural_file_references() {
    let mut string_table = StringTable::new();
    let file_path = PathBuf::from("src/@entry.moth");
    let (output, _span_builder) = prepare_single_file(
        "$config\nasset #String = @assets/missing.mtf\n",
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
                    if declaration.config_contract.is_some()
            )
        })
        .expect("expected source config constant header");
    assert!(
        header.local_ordering_hints.is_empty(),
        "config defaults must not add content-source ordering hints"
    );
}

#[test]
fn source_config_contracts_reject_comma_termination() {
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(
            "$config\nanalytics #Bool,\ntrailing #= 1\n",
            "src/@entry.moth",
            "src/@entry.moth",
        ),
        "a comma cannot terminate a marked top-level contract",
    );
    assert!(
        !errors.diagnostics.is_empty(),
        "expected a source contract placement diagnostic"
    );
}

#[test]
fn malformed_decorator_wrapper_constant_initializer_reports_eof_delimiter_error() {
    let result = parse_single_file_headers_with_entry(
        "broken #= [$before [:<li>]:\n<ul>[$slot]</ul>\n",
        "src/@entry.moth",
        "src/@entry.moth",
    );

    assert!(
        matches!(&result, HeaderTestOutcome::Diagnostics(_)),
        "unterminated decorator wrapper templates should fail instead of hanging"
    );
    let errors = expect_header_error(result, "expected parse errors");

    assert!(
        errors
            .diagnostics
            .views()
            .any(|diagnostic| { diagnostic.descriptor().code == "MOTH-SYNTAX-0022" })
    );
}
