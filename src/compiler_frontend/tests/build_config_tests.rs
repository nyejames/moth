use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigContractConflictReason, BuildConfigContractFact,
    BuildConfigFingerprint, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigResolutionError,
    BuildConfigResolutionIndex, BuildConfigValueLocation, BuildConfigValueOrigin, BuildInputName,
    BuildInputNameError, BuildInputType, BuildInputValueError, BuilderConfigGlobalSet,
    PrimitiveBuildInputType, PrimitiveBuildValue, build_config_fingerprint,
    resolve_build_config_values,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::folded_value::{FiniteFloat, PublicFoldedValue};
use crate::compiler_frontend::project_globals::{
    ProjectGlobalsFieldInput, ProjectGlobalsInterface,
};
use crate::compiler_frontend::public_interface::PublicDiagnosticLocation;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
fn source_location_at(path: &str, string_table: &mut StringTable) -> SourceLocation {
    let scope = InternedPath::from_single_str(path, string_table);
    SourceLocation::new(
        scope,
        CharPosition {
            line_number: 1,
            char_column: 0,
        },
        CharPosition {
            line_number: 1,
            char_column: 12,
        },
    )
}

fn source_location(string_table: &mut StringTable) -> SourceLocation {
    source_location_at("src/@page.moth", string_table)
}

fn command_location(argument_index: usize) -> BuildConfigValueLocation {
    BuildConfigValueLocation::Command(BuildCommandLocation::new(argument_index))
}

fn entry(
    name: &str,
    value: PrimitiveBuildValue,
    location: BuildConfigValueLocation,
) -> BuildConfigInputEntry {
    BuildConfigInputEntry::new(
        BuildInputName::new(name).expect("test input name should validate"),
        value,
        location,
    )
}

#[test]
fn build_input_names_follow_the_lower_snake_case_policy() {
    for valid in [
        "analytics",
        "api_url",
        "build_number",
        "value2",
        "_hidden",
        "a",
    ] {
        let name = BuildInputName::new(valid).expect("lower_snake_case name should validate");
        assert_eq!(name.as_str(), valid);
    }

    for invalid in [
        "",
        "Analytics",
        "API_URL",
        "2fast",
        "bad-name",
        "value name",
        "café",
        "_",
    ] {
        assert_eq!(
            BuildInputName::new(invalid),
            Err(BuildInputNameError::NotLowerSnakeCase),
            "{invalid:?} must be rejected as a build input name"
        );
    }
}

#[test]
fn primitive_values_report_their_exact_types() {
    assert_eq!(
        PrimitiveBuildValue::String("alpha".to_owned()).primitive_type(),
        PrimitiveBuildInputType::String
    );
    assert_eq!(
        PrimitiveBuildValue::Int(-4).primitive_type(),
        PrimitiveBuildInputType::Int
    );
    assert_eq!(
        PrimitiveBuildValue::float(0.75)
            .expect("finite float should construct")
            .primitive_type(),
        PrimitiveBuildInputType::Float
    );
    assert_eq!(
        PrimitiveBuildValue::Bool(true).primitive_type(),
        PrimitiveBuildInputType::Bool
    );
    assert_eq!(
        PrimitiveBuildValue::Char(':').primitive_type(),
        PrimitiveBuildInputType::Char
    );

    for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            PrimitiveBuildValue::float(non_finite).is_err(),
            "non-finite float must be rejected"
        );
    }
}

#[test]
fn build_input_types_accept_only_the_exact_primitive_or_matching_optional() {
    let primitives = [
        PrimitiveBuildInputType::String,
        PrimitiveBuildInputType::Int,
        PrimitiveBuildInputType::Float,
        PrimitiveBuildInputType::Bool,
        PrimitiveBuildInputType::Char,
    ];

    for required in primitives {
        let required_contract = BuildInputType::Primitive(required);
        assert!(!required_contract.is_optional());
        assert_eq!(required_contract.primitive(), required);

        let optional_contract = BuildInputType::Optional(required);
        assert!(optional_contract.is_optional());
        assert_eq!(optional_contract.primitive(), required);

        for value_type in primitives {
            assert_eq!(
                required_contract.accepts_primitive(value_type),
                value_type == required
            );
            assert_eq!(
                optional_contract.accepts_primitive(value_type),
                value_type == required,
                "only a concrete value of the matching primitive satisfies the optional"
            );
        }
    }
}

#[test]
fn build_config_input_set_iterates_in_deterministic_name_order() {
    let mut string_table = StringTable::new();
    let declared = BuildConfigValueLocation::Source(source_location(&mut string_table));

    let mut inputs = BuildConfigInputSet::new();
    assert!(inputs.is_empty());

    inputs
        .insert(entry(
            "zulu",
            PrimitiveBuildValue::Bool(true),
            command_location(2),
        ))
        .expect("first insert should succeed");
    inputs
        .insert(entry(
            "alpha",
            PrimitiveBuildValue::String("a".to_owned()),
            declared.clone(),
        ))
        .expect("second insert should succeed");
    inputs
        .insert(entry(
            "mid_way",
            PrimitiveBuildValue::Int(7),
            command_location(1),
        ))
        .expect("third insert should succeed");

    assert_eq!(inputs.len(), 3);
    assert!(!inputs.is_empty());

    let names: Vec<&str> = inputs.iter().map(|input| input.name().as_str()).collect();
    assert_eq!(names, ["alpha", "mid_way", "zulu"]);

    let alpha_name = BuildInputName::new("alpha").expect("test input name should validate");
    let alpha = inputs.get(&alpha_name).expect("alpha should be present");
    assert_eq!(alpha.value(), &PrimitiveBuildValue::String("a".to_owned()));
    assert_eq!(alpha.location(), &declared);
}

#[test]
fn build_config_input_set_rejects_duplicate_names_deterministically() {
    let mut string_table = StringTable::new();
    let declared = BuildConfigValueLocation::Source(source_location(&mut string_table));

    let mut inputs = BuildConfigInputSet::new();
    inputs
        .insert(entry(
            "retries",
            PrimitiveBuildValue::Int(4),
            declared.clone(),
        ))
        .expect("first insert should succeed");

    let duplicate = entry("retries", PrimitiveBuildValue::Int(9), command_location(3));
    let error = inputs
        .insert(duplicate)
        .expect_err("duplicate input name must be rejected");

    assert_eq!(error.rejected().value(), &PrimitiveBuildValue::Int(9));
    assert_eq!(error.rejected().location(), &command_location(3));
    assert_eq!(error.existing_location(), &declared);

    // The earlier typed value and location remain owned by the set.
    assert_eq!(inputs.len(), 1);
    let retries_name = BuildInputName::new("retries").expect("test input name should validate");
    let retained = inputs
        .get(&retries_name)
        .expect("retries should be present");
    assert_eq!(retained.value(), &PrimitiveBuildValue::Int(4));
    assert_eq!(retained.location(), &declared);
}

// -------------------------
//  Command-text primitive inference
// -------------------------

fn infer(value: &str) -> PrimitiveBuildValue {
    PrimitiveBuildValue::from_command_text(value)
        .expect("value should infer without a command-input diagnostic")
}

fn infer_error(value: &str) -> BuildInputValueError {
    PrimitiveBuildValue::from_command_text(value)
        .expect_err("value should produce a command-input diagnostic")
}

fn float_value(value: f64) -> PrimitiveBuildValue {
    PrimitiveBuildValue::Float(FiniteFloat::new(value).expect("test float should be finite"))
}

/// The inference path is a pure function of the authored text: it takes no project config,
/// no source contract and no build-system state, so these cases prove the primitive type is
/// decided immediately without waiting for any contract to be discovered.
#[test]
fn command_text_infers_bool_first_and_exactly() {
    assert_eq!(infer("true"), PrimitiveBuildValue::Bool(true));
    assert_eq!(infer("false"), PrimitiveBuildValue::Bool(false));
    // Case variants are ordinary String fallback text, never Bool.
    assert_eq!(
        infer("True"),
        PrimitiveBuildValue::String(String::from("True"))
    );
    assert_eq!(
        infer("TRUE"),
        PrimitiveBuildValue::String(String::from("TRUE"))
    );
}

#[test]
fn command_text_infers_complete_signed_whole_numbers_as_int() {
    assert_eq!(infer("0"), PrimitiveBuildValue::Int(0));
    assert_eq!(infer("4"), PrimitiveBuildValue::Int(4));
    assert_eq!(infer("-7"), PrimitiveBuildValue::Int(-7));
    assert_eq!(infer("-0"), PrimitiveBuildValue::Int(0));
    assert_eq!(infer("2147483647"), PrimitiveBuildValue::Int(i32::MAX));
    assert_eq!(infer("-2147483648"), PrimitiveBuildValue::Int(i32::MIN));
    assert_eq!(infer("1_000"), PrimitiveBuildValue::Int(1_000));
    assert_eq!(infer("-1_0"), PrimitiveBuildValue::Int(-10));
}

#[test]
fn command_text_infers_decimal_and_exponent_literals_as_float() {
    assert_eq!(infer("0.75"), float_value(0.75));
    assert_eq!(infer("-1.5"), float_value(-1.5));
    assert_eq!(infer("1_000.5"), float_value(1_000.5));
    assert_eq!(infer("1e6"), float_value(1e6));
    assert_eq!(infer("2e31"), float_value(2e31));
    assert_eq!(infer("-1.5e-2"), float_value(-1.5e-2));
    // An exponent literal is Float even when its value is integral.
    assert_eq!(infer("2e3"), float_value(2e3));
}

#[test]
fn command_text_rejects_int_overflow_and_non_finite_floats_as_diagnostics() {
    // Integer-shaped out-of-range values diagnose rather than fall through to Float or String.
    assert_eq!(
        infer_error("2147483648"),
        BuildInputValueError::IntOutOfRange {
            text: String::from("2147483648"),
        }
    );
    assert_eq!(
        infer_error("-2147483649"),
        BuildInputValueError::IntOutOfRange {
            text: String::from("-2147483649"),
        }
    );
    assert_eq!(
        infer_error("99999999999999999999"),
        BuildInputValueError::IntOutOfRange {
            text: String::from("99999999999999999999"),
        }
    );
    // Exponent-shaped non-finite values reject.
    assert_eq!(
        infer_error("1e400"),
        BuildInputValueError::NonFiniteFloat {
            text: String::from("1e400"),
        }
    );
    assert_eq!(
        infer_error("-1e400"),
        BuildInputValueError::NonFiniteFloat {
            text: String::from("-1e400"),
        }
    );
}

#[test]
fn command_text_falls_back_to_string_for_every_other_value() {
    // `+1`, NaN and Infinity are ordinary String text.
    assert_eq!(infer("+1"), PrimitiveBuildValue::String(String::from("+1")));
    assert_eq!(
        infer("NaN"),
        PrimitiveBuildValue::String(String::from("NaN"))
    );
    assert_eq!(
        infer("Infinity"),
        PrimitiveBuildValue::String(String::from("Infinity"))
    );
    // Malformed numeric-looking text falls back instead of diagnosing.
    assert_eq!(
        infer("1.2.3"),
        PrimitiveBuildValue::String(String::from("1.2.3"))
    );
    assert_eq!(infer("1_"), PrimitiveBuildValue::String(String::from("1_")));
    assert_eq!(
        infer("1E6"),
        PrimitiveBuildValue::String(String::from("1E6"))
    );
    assert_eq!(infer("1e"), PrimitiveBuildValue::String(String::from("1e")));
    assert_eq!(infer("1."), PrimitiveBuildValue::String(String::from("1.")));
    // Backtick and raw text stay String; Char never infers from one unquoted character.
    assert_eq!(
        infer("`raw`"),
        PrimitiveBuildValue::String(String::from("`raw`"))
    );
    assert_eq!(infer("x"), PrimitiveBuildValue::String(String::from("x")));
    assert_eq!(infer("5"), PrimitiveBuildValue::Int(5));
    assert_eq!(infer("-"), PrimitiveBuildValue::String(String::from("-")));
}

#[test]
fn command_text_preserves_the_exact_authored_remainder() {
    assert_eq!(
        infer("alpha"),
        PrimitiveBuildValue::String(String::from("alpha"))
    );
    assert_eq!(
        infer("https://example.com/?a=1&b=2"),
        PrimitiveBuildValue::String(String::from("https://example.com/?a=1&b=2"))
    );
    assert_eq!(
        infer("0.1.0-beta"),
        PrimitiveBuildValue::String(String::from("0.1.0-beta"))
    );
    // The empty value after `name=` is String, not absence.
    assert_eq!(infer(""), PrimitiveBuildValue::String(String::from("")));
    // Bare `none` has no CLI meaning and is String text.
    assert_eq!(
        infer("none"),
        PrimitiveBuildValue::String(String::from("none"))
    );
}

#[test]
fn command_text_parses_quoted_literals_through_the_ordinary_grammar() {
    assert_eq!(infer("':'"), PrimitiveBuildValue::Char(':'));
    assert_eq!(infer("' '"), PrimitiveBuildValue::Char(' '));
    assert_eq!(infer("'é'"), PrimitiveBuildValue::Char('é'));
    // Explicit quotes force String even when the content looks like another primitive.
    assert_eq!(
        infer("\"true\""),
        PrimitiveBuildValue::String(String::from("true"))
    );
    assert_eq!(
        infer("\"42\""),
        PrimitiveBuildValue::String(String::from("42"))
    );
    assert_eq!(
        infer("\"0.75\""),
        PrimitiveBuildValue::String(String::from("0.75"))
    );
    assert_eq!(
        infer("\":\""),
        PrimitiveBuildValue::String(String::from(":"))
    );
    // Ordinary Moth String escapes apply exactly as in source.
    assert_eq!(
        infer("\"a\\nb\""),
        PrimitiveBuildValue::String(String::from("a\nb"))
    );
    assert_eq!(
        infer("\"a\\\"b\""),
        PrimitiveBuildValue::String(String::from("a\"b"))
    );
    // A physical newline is content inside the ordinary String literal, not terminal trivia.
    assert_eq!(
        infer("\"a\nb\""),
        PrimitiveBuildValue::String(String::from("a\nb"))
    );
    // Empty quoted String stays String.
    assert_eq!(infer("\"\""), PrimitiveBuildValue::String(String::from("")));
}

#[test]
fn command_text_rejects_any_suffix_after_an_explicit_quote() {
    let malformed_chars = [
        "'a",
        "'",
        "''",
        "'ab'",
        "'a'b",
        "':'--ignored",
        "':'\nignored",
        "':' ",
        "':' \"other\"",
    ];
    for value in malformed_chars {
        match infer_error(value) {
            BuildInputValueError::MalformedCharLiteral { text, reason } => {
                assert_eq!(text, value);
                assert!(!reason.is_empty(), "char rejection should carry a reason");
            }
            other => panic!("'{value}' should reject as a malformed Char literal, got {other:?}"),
        }
    }

    let malformed_strings = [
        "\"abc",
        "\"a\\q\"",
        "\"a\"x",
        "\"a\" 'b'",
        "\"alpha\"--ignored",
        "\"alpha\"\nignored",
        "\"alpha\" ",
    ];
    for value in malformed_strings {
        match infer_error(value) {
            BuildInputValueError::MalformedStringLiteral { text, reason } => {
                assert_eq!(text, value);
                assert!(!reason.is_empty(), "string rejection should carry a reason");
            }
            other => {
                panic!("'{value}' should reject as a malformed String literal, got {other:?}")
            }
        }
    }
}

fn contract_fact(
    string_table: &mut StringTable,
    name: &str,
    value_type: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
) -> BuildConfigContractFact {
    BuildConfigContractFact::new(
        BuildInputName::new(name).expect("test input name should validate"),
        value_type,
        required,
        default,
        source_location(string_table),
    )
}

fn builder_globals(entries: &[(&str, PrimitiveBuildValue)]) -> BuilderConfigGlobalSet {
    let mut globals = BuilderConfigGlobalSet::new();
    for (name, value) in entries {
        globals
            .insert(
                BuildInputName::new(name).expect("test input name should validate"),
                value.clone(),
            )
            .expect("test builder global should be platform-neutral");
    }
    globals
}

#[test]
fn builder_globals_reject_backend_and_platform_identity_names() {
    for forbidden_name in [
        "target_os",
        "target_arch",
        "backend",
        "is_wasm",
        "is_javascript",
        "is_browser",
    ] {
        let mut globals = BuilderConfigGlobalSet::new();
        let name = BuildInputName::new(forbidden_name).expect("forbidden name should be valid");
        let error = globals
            .insert(name, PrimitiveBuildValue::Bool(true))
            .expect_err("platform identity globals must be rejected");
        assert_eq!(error.name().as_str(), forbidden_name);
        assert!(globals.is_empty());
    }
}

#[test]
fn builder_surface_registers_platform_neutral_globals_only() {
    let mut surface = BuilderSurface::with_mandatory_core();
    let name = BuildInputName::new("release_channel").expect("test input name should validate");
    let value = PrimitiveBuildValue::String("stable".to_owned());

    assert_eq!(
        surface
            .register_config_global(name.clone(), value.clone())
            .expect("platform-neutral global should register"),
        None
    );
    assert_eq!(surface.config_globals().get(&name), Some(&value));

    let forbidden = BuildInputName::new("backend").expect("forbidden name should be valid");
    let error = surface
        .register_config_global(forbidden, PrimitiveBuildValue::String("html".to_owned()))
        .expect_err("backend identity must not enter the builder surface");
    assert_eq!(error.name().as_str(), "backend");
}

fn project_global_with_fingerprint(
    name: &str,
    value: i32,
    fingerprint: BuildConfigFingerprint,
) -> ProjectGlobalsInterface {
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        crate::compiler_frontend::project_globals::PROJECT_GLOBALS_DEPENDENCY_NAME,
        name,
    );
    let field = ProjectGlobalsFieldInput::new(
        name,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        PublicFoldedValue::Int(value),
        PublicDiagnosticLocation {
            scope_components: vec!["config.moth".to_owned()],
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 1,
        },
        fingerprint,
        SyntheticInterfaceProvenance::single(member),
    );

    ProjectGlobalsInterface::new(
        StablePackageIdentity::project_local("config-state"),
        vec![field],
    )
    .expect("test project-global field should construct")
}

#[test]
fn build_config_fingerprints_ignore_origin_for_config_and_project_globals() {
    let mut string_table = StringTable::new();
    let value = PrimitiveBuildValue::Int(7);
    let contract = BuildInputType::Primitive(PrimitiveBuildInputType::Int);
    let facts = vec![contract_fact(
        &mut string_table,
        "same_value",
        contract,
        false,
        Some(value.clone()),
    )];

    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(entry("same_value", value.clone(), command_location(0)))
        .expect("test explicit input should be unique");

    let explicit = resolve_build_config_values(
        &facts,
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect("explicit input should resolve");
    let defaulted = resolve_build_config_values(
        &facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("declaration default should resolve");
    let name = BuildInputName::new("same_value").expect("test input name should validate");
    let explicit_value = explicit.get(&name).expect("explicit value should resolve");
    let defaulted_value = defaulted
        .get(&name)
        .expect("defaulted value should resolve");

    assert_eq!(
        explicit_value.origin(),
        BuildConfigValueOrigin::ExplicitInput
    );
    assert_eq!(
        defaulted_value.origin(),
        BuildConfigValueOrigin::DeclarationDefault
    );
    assert_eq!(explicit_value.value(), Some(&value));
    assert_eq!(defaulted_value.value(), Some(&value));
    assert_eq!(explicit_value.fingerprint(), defaulted_value.fingerprint());
    assert_eq!(
        explicit_value.fingerprint(),
        build_config_fingerprint("same_value", contract, Some(&value))
    );

    // An @project consumer sees the same semantic member fingerprint even when the provider
    // origin changes. Origin remains available on the resolved config value for provenance, but
    // it cannot create a semantic config or project-global invalidation.
    let explicit_project =
        project_global_with_fingerprint("same_value", 7, explicit_value.fingerprint());
    let defaulted_project =
        project_global_with_fingerprint("same_value", 7, defaulted_value.fingerprint());
    assert_eq!(
        explicit_project
            .member("same_value")
            .expect("explicit project field should exist")
            .fingerprint(),
        defaulted_project
            .member("same_value")
            .expect("defaulted project field should exist")
            .fingerprint()
    );
}

#[test]
fn boundary_resolver_applies_fixed_direct_explicit_global_default_precedence() {
    let mut string_table = StringTable::new();
    let source_facts = vec![
        // The source contract is required, but the fixed project value is authoritative.
        contract_fact(
            &mut string_table,
            "fixed_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
        ),
        // The direct project contract resolves its own input before the source default.
        contract_fact(
            &mut string_table,
            "direct_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            false,
            Some(PrimitiveBuildValue::Bool(false)),
        ),
        contract_fact(
            &mut string_table,
            "explicit_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::String),
            false,
            Some(PrimitiveBuildValue::String("source".to_owned())),
        ),
        contract_fact(
            &mut string_table,
            "global_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
        ),
        contract_fact(
            &mut string_table,
            "default_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Char),
            false,
            Some(PrimitiveBuildValue::Char('d')),
        ),
        contract_fact(
            &mut string_table,
            "optional_value",
            BuildInputType::Optional(PrimitiveBuildInputType::String),
            false,
            None,
        ),
    ];
    let fixed_project_facts = vec![contract_fact(
        &mut string_table,
        "fixed_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        true,
        Some(PrimitiveBuildValue::Int(41)),
    )];
    let direct_contract_type = BuildInputType::Primitive(PrimitiveBuildInputType::Bool);
    let direct_value = PrimitiveBuildValue::Bool(true);
    let direct_project_facts = vec![
        contract_fact(
            &mut string_table,
            "direct_value",
            direct_contract_type,
            false,
            Some(PrimitiveBuildValue::Bool(false)),
        )
        .with_resolved_provider(
            Some(direct_value.clone()),
            BuildConfigValueOrigin::ExplicitInput,
            build_config_fingerprint("direct_value", direct_contract_type, Some(&direct_value)),
            Some(command_location(1)),
        ),
    ];

    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(entry(
            "fixed_value",
            PrimitiveBuildValue::Int(99),
            command_location(0),
        ))
        .expect("fixed input should be retained");
    explicit_inputs
        .insert(entry(
            "direct_value",
            PrimitiveBuildValue::Bool(true),
            command_location(1),
        ))
        .expect("direct input should be retained");
    explicit_inputs
        .insert(entry(
            "explicit_value",
            PrimitiveBuildValue::String("command".to_owned()),
            command_location(2),
        ))
        .expect("source input should be retained");

    let globals = builder_globals(&[
        ("direct_value", PrimitiveBuildValue::Bool(false)),
        ("global_value", PrimitiveBuildValue::Int(7)),
    ]);
    let resolved = resolve_build_config_values(
        &source_facts,
        &fixed_project_facts,
        &direct_project_facts,
        &explicit_inputs,
        &globals,
    )
    .expect("all test contracts should resolve");

    assert_eq!(
        resolved
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "default_value",
            "direct_value",
            "explicit_value",
            "fixed_value",
            "global_value",
            "optional_value",
        ]
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("fixed_value").expect("name should validate"))
            .expect("fixed value should resolve")
            .value(),
        Some(&PrimitiveBuildValue::Int(41))
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("fixed_value").expect("name should validate"))
            .expect("fixed value should resolve")
            .origin(),
        BuildConfigValueOrigin::FixedProjectField
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("direct_value").expect("name should validate"))
            .expect("direct value should resolve")
            .value(),
        Some(&PrimitiveBuildValue::Bool(true))
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("direct_value").expect("name should validate"))
            .expect("direct value should resolve")
            .origin(),
        BuildConfigValueOrigin::ExplicitInput
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("explicit_value").expect("name should validate"))
            .expect("explicit value should resolve")
            .value(),
        Some(&PrimitiveBuildValue::String("command".to_owned()))
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("global_value").expect("name should validate"))
            .expect("global value should resolve")
            .value(),
        Some(&PrimitiveBuildValue::Int(7))
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("default_value").expect("name should validate"))
            .expect("default value should resolve")
            .value(),
        Some(&PrimitiveBuildValue::Char('d'))
    );
    assert_eq!(
        resolved
            .get(&BuildInputName::new("optional_value").expect("name should validate"))
            .expect("optional value should resolve")
            .value(),
        None
    );
}

#[test]
fn boundary_resolver_reports_each_same_name_source_compatibility_conflict() {
    let mut string_table = StringTable::new();

    let primitive_type_error = resolve_build_config_values(
        &[
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Primitive(PrimitiveBuildInputType::Int),
                true,
                None,
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Primitive(PrimitiveBuildInputType::String),
                true,
                None,
            ),
        ],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different primitive source contracts must conflict");
    assert!(matches!(
        primitive_type_error,
        BuildConfigResolutionError::SourceContractConflict {
            reason: BuildConfigContractConflictReason::PrimitiveType {
                first: PrimitiveBuildInputType::Int,
                conflicting: PrimitiveBuildInputType::String,
            },
            ..
        }
    ));

    let optionality_error = resolve_build_config_values(
        &[
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Primitive(PrimitiveBuildInputType::Int),
                true,
                None,
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Int),
                false,
                None,
            ),
        ],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different source optionality must conflict");
    assert!(matches!(
        optionality_error,
        BuildConfigResolutionError::SourceContractConflict {
            reason: BuildConfigContractConflictReason::Optionality {
                first: false,
                conflicting: true,
            },
            ..
        }
    ));

    let required_error = resolve_build_config_values(
        &[
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Int),
                true,
                None,
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Int),
                false,
                None,
            ),
        ],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different source required states must conflict");
    assert!(matches!(
        required_error,
        BuildConfigResolutionError::SourceContractConflict {
            reason: BuildConfigContractConflictReason::Required {
                first: true,
                conflicting: false,
            },
            ..
        }
    ));

    let default_error = resolve_build_config_values(
        &[
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Int),
                false,
                Some(PrimitiveBuildValue::Int(1)),
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Int),
                false,
                Some(PrimitiveBuildValue::Int(2)),
            ),
        ],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different normalized source defaults must conflict");
    assert!(matches!(
        default_error,
        BuildConfigResolutionError::SourceContractConflict {
            reason: BuildConfigContractConflictReason::Default {
                first: Some(PrimitiveBuildValue::Int(1)),
                conflicting: Some(PrimitiveBuildValue::Int(2)),
            },
            ..
        }
    ));
}

#[test]
fn boundary_resolver_checks_project_contracts_against_source_contracts() {
    let mut string_table = StringTable::new();
    let direct_error = resolve_build_config_values(
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
        )],
        &[],
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            false,
            Some(PrimitiveBuildValue::Bool(false)),
        )],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("direct project and source contracts must agree");
    assert!(matches!(
        direct_error,
        BuildConfigResolutionError::ProjectSourceContractConflict {
            reason: BuildConfigContractConflictReason::PrimitiveType {
                first: PrimitiveBuildInputType::Bool,
                conflicting: PrimitiveBuildInputType::Int,
            },
            ..
        }
    ));

    let fixed_error = resolve_build_config_values(
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
        )],
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Optional(PrimitiveBuildInputType::Int),
            false,
            Some(PrimitiveBuildValue::Int(4)),
        )],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("fixed project and source contracts must agree on type");
    assert!(matches!(
        fixed_error,
        BuildConfigResolutionError::FixedProjectSourceTypeMismatch {
            reason: BuildConfigContractConflictReason::Optionality {
                first: true,
                conflicting: false,
            },
            ..
        }
    ));
}

#[test]
fn boundary_resolver_reports_project_source_conflicts_in_collected_source_order() {
    let mut string_table = StringTable::new();
    let source_facts = vec![
        // `zulu_value` is collected first even though `alpha_value` sorts first by name.
        contract_fact(
            &mut string_table,
            "zulu_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
        ),
        contract_fact(
            &mut string_table,
            "alpha_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            true,
            None,
        ),
    ];
    let direct_project_facts = vec![
        contract_fact(
            &mut string_table,
            "zulu_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::String),
            false,
            Some(PrimitiveBuildValue::String("zulu".to_owned())),
        ),
        contract_fact(
            &mut string_table,
            "alpha_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            false,
            Some(PrimitiveBuildValue::Int(1)),
        ),
    ];

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &direct_project_facts,
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("the first collected project/source conflict should be reported");

    assert_eq!(error.name().as_str(), "zulu_value");
    assert!(matches!(
        error,
        BuildConfigResolutionError::ProjectSourceContractConflict {
            reason: BuildConfigContractConflictReason::PrimitiveType {
                first: PrimitiveBuildInputType::String,
                conflicting: PrimitiveBuildInputType::Int,
            },
            ..
        }
    ));
}

#[test]
fn check_only_contracts_resolve_independently_from_borrowed_canonical_state() {
    let mut string_table = StringTable::new();
    let canonical_facts = vec![contract_fact(
        &mut string_table,
        "canonical_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        false,
        Some(PrimitiveBuildValue::Int(7)),
    )];
    let first_check_only_facts = vec![contract_fact(
        &mut string_table,
        "transient_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        false,
        Some(PrimitiveBuildValue::Int(1)),
    )];
    let second_check_only_facts = vec![contract_fact(
        &mut string_table,
        "transient_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::String),
        false,
        Some(PrimitiveBuildValue::String("one".to_owned())),
    )];

    let canonical = resolve_build_config_values(
        &canonical_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("canonical facts should resolve");
    let index = crate::compiler_frontend::build_config::BuildConfigResolutionIndex::from_validated(
        &canonical,
        &canonical_facts,
        &[],
        &[],
    );
    let changed_globals = builder_globals(&[("canonical_value", PrimitiveBuildValue::Int(99))]);
    let first_check_only = index
        .resolve_with_transient_source_facts(
            &first_check_only_facts,
            &BuildConfigInputSet::new(),
            &changed_globals,
        )
        .expect("the first check-only unit should resolve independently");
    let second_check_only = index
        .resolve_with_transient_source_facts(
            &second_check_only_facts,
            &BuildConfigInputSet::new(),
            &changed_globals,
        )
        .expect("the second check-only unit should resolve independently");

    let transient_name =
        BuildInputName::new("transient_value").expect("test input name should validate");
    let canonical_name =
        BuildInputName::new("canonical_value").expect("test input name should validate");
    assert_eq!(
        first_check_only.get(&canonical_name),
        canonical.get(&canonical_name),
        "check-only resolution must retain the canonical provider result"
    );
    assert_eq!(
        second_check_only.get(&canonical_name),
        canonical.get(&canonical_name),
        "each check-only unit must retain the same canonical provider result"
    );
    assert!(canonical.get(&transient_name).is_none());
    assert_eq!(
        first_check_only
            .get(&transient_name)
            .expect("first transient contract should be retained in its private map")
            .value(),
        Some(&PrimitiveBuildValue::Int(1))
    );
    assert_eq!(
        second_check_only
            .get(&transient_name)
            .expect("second transient contract should be retained in its private map")
            .value(),
        Some(&PrimitiveBuildValue::String("one".to_owned()))
    );
}

#[test]
fn check_only_conflict_keeps_canonical_and_transient_locations_in_their_own_tables() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("canonical-boundary-prefix");
    let canonical_location = source_location(&mut boundary_table);
    let fork_source = boundary_table.fork_source();
    let (mut transient_table, _) = fork_source.fork_for_module().into_parts();
    transient_table.intern("transient-local-prefix");
    let transient_location = source_location_at("src/+check-only.moth", &mut transient_table);
    assert_ne!(
        canonical_location.scope, transient_location.scope,
        "the regression fixture must use distinct inherited/local StringIds"
    );

    let name = BuildInputName::new("shared_setting").expect("test input name should validate");
    let canonical_fact = BuildConfigContractFact::new(
        name.clone(),
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        true,
        None,
        canonical_location.clone(),
    );
    let transient_fact = BuildConfigContractFact::new(
        name,
        BuildInputType::Primitive(PrimitiveBuildInputType::String),
        true,
        None,
        transient_location.clone(),
    );
    let canonical_values = resolve_build_config_values(
        std::slice::from_ref(&canonical_fact),
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &builder_globals(&[("shared_setting", PrimitiveBuildValue::Int(7))]),
    )
    .expect("canonical contract should resolve");
    let index = BuildConfigResolutionIndex::from_validated(
        &canonical_values,
        std::slice::from_ref(&canonical_fact),
        &[],
        &[],
    );

    let error = index
        .resolve_with_transient_source_facts(
            std::slice::from_ref(&transient_fact),
            &BuildConfigInputSet::new(),
            &BuilderConfigGlobalSet::new(),
        )
        .expect_err("a transient contract differing from canonical state should conflict");
    let BuildConfigResolutionError::SourceContractConflict {
        first, conflicting, ..
    } = error
    else {
        panic!("expected a canonical/transient source contract conflict");
    };

    assert_eq!(first.location(), &canonical_location);
    assert_eq!(conflicting.location(), &transient_location);
    assert_eq!(
        first.location().scope.to_portable_string(&transient_table),
        "src/@page.moth"
    );
    assert_eq!(
        conflicting
            .location()
            .scope
            .to_portable_string(&transient_table),
        "src/+check-only.moth"
    );
}
#[test]
fn boundary_resolver_reports_typed_value_mismatches_with_locations() {
    let mut string_table = StringTable::new();
    let source_facts = [contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        true,
        None,
    )];
    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(entry(
            "count",
            PrimitiveBuildValue::String("four".to_owned()),
            command_location(4),
        ))
        .expect("test input should insert");

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("a String must not satisfy an Int contract");
    assert_eq!(error.name().as_str(), "count");
    assert_eq!(error.provided_type(), Some(PrimitiveBuildInputType::String));
    assert_eq!(error.value_location(), Some(&command_location(4)));
    assert!(matches!(
        error,
        BuildConfigResolutionError::ValueTypeMismatch {
            provided: PrimitiveBuildInputType::String,
            ..
        }
    ));

    let builder_error = resolve_build_config_values(
        &[contract_fact(
            &mut string_table,
            "enabled",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            true,
            None,
        )],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &builder_globals(&[("enabled", PrimitiveBuildValue::Int(1))]),
    )
    .expect_err("a builder Int must not satisfy a Bool contract");
    assert_eq!(
        builder_error.provided_type(),
        Some(PrimitiveBuildInputType::Int)
    );
    assert_eq!(builder_error.value_location(), None);
}

#[test]
fn boundary_resolver_checks_unknown_inputs_after_all_contracts_are_known() {
    let mut string_table = StringTable::new();
    let source_facts = [contract_fact(
        &mut string_table,
        "known_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::String),
        false,
        Some(PrimitiveBuildValue::String("default".to_owned())),
    )];
    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(entry(
            "known_value",
            PrimitiveBuildValue::String("known".to_owned()),
            command_location(0),
        ))
        .expect("known input should insert");
    explicit_inputs
        .insert(entry(
            "unknown_value",
            PrimitiveBuildValue::String("unknown".to_owned()),
            command_location(1),
        ))
        .expect("unknown input should insert");

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("unknown explicit names must be rejected after contract collection");
    assert_eq!(error.name().as_str(), "unknown_value");
    assert_eq!(error.contract_location(), None);
    assert_eq!(error.value_location(), Some(&command_location(1)));
    assert!(matches!(
        error,
        BuildConfigResolutionError::UnknownExplicitInput { .. }
    ));
}

#[test]
fn boundary_resolver_distinguishes_optional_absence_from_required_missing() {
    let mut string_table = StringTable::new();
    let optional = [contract_fact(
        &mut string_table,
        "optional_value",
        BuildInputType::Optional(PrimitiveBuildInputType::String),
        false,
        None,
    )];
    let resolved = resolve_build_config_values(
        &optional,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("optional omission should resolve to absence");
    let optional_value = resolved
        .get(&BuildInputName::new("optional_value").expect("name should validate"))
        .expect("optional value should be present in the map");
    assert_eq!(optional_value.value(), None);
    assert_eq!(
        optional_value.origin(),
        BuildConfigValueOrigin::DeclarationDefault
    );
    assert_eq!(optional_value.value_location(), None);

    let required = [contract_fact(
        &mut string_table,
        "required_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        true,
        None,
    )];
    let error = resolve_build_config_values(
        &required,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("required omission must diagnose");
    assert!(matches!(
        error,
        BuildConfigResolutionError::MissingRequiredValue { .. }
    ));
    assert_eq!(error.name().as_str(), "required_value");
}

#[test]
fn boundary_resolver_keeps_name_order_and_fingerprints_deterministic() {
    let mut first_string_table = StringTable::new();
    let first_facts = vec![
        contract_fact(
            &mut first_string_table,
            "zulu_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            false,
            Some(PrimitiveBuildValue::Bool(true)),
        ),
        contract_fact(
            &mut first_string_table,
            "alpha_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            false,
            Some(PrimitiveBuildValue::Int(8)),
        ),
    ];
    let first = resolve_build_config_values(
        &first_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("first ordering should resolve");

    let mut second_string_table = StringTable::new();
    let second_facts = vec![
        contract_fact(
            &mut second_string_table,
            "alpha_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            false,
            Some(PrimitiveBuildValue::Int(8)),
        ),
        contract_fact(
            &mut second_string_table,
            "zulu_value",
            BuildInputType::Primitive(PrimitiveBuildInputType::Bool),
            false,
            Some(PrimitiveBuildValue::Bool(true)),
        ),
    ];
    let second = resolve_build_config_values(
        &second_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("second ordering should resolve");

    assert_eq!(
        first
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["alpha_value", "zulu_value"]
    );
    assert_eq!(first, second);
    let alpha_name = BuildInputName::new("alpha_value").expect("name should validate");
    let zulu_name = BuildInputName::new("zulu_value").expect("name should validate");
    assert_ne!(
        first
            .get(&alpha_name)
            .expect("alpha should resolve")
            .fingerprint(),
        first
            .get(&zulu_name)
            .expect("zulu should resolve")
            .fingerprint()
    );
    assert_ne!(
        first
            .get(&alpha_name)
            .expect("alpha should resolve")
            .fingerprint()
            .0,
        0
    );
}
