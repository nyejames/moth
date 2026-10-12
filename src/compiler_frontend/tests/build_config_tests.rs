use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigContractConflictReason, BuildConfigContractFact,
    BuildConfigFingerprint, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigInputValue,
    BuildConfigResolutionError, BuildConfigResolutionIndex, BuildConfigValueLocation,
    BuildConfigValueOrigin, BuildInputName, BuildInputNameError, BuildInputType,
    BuildInputValueError, BuilderConfigGlobalSet, ContractValueError, PrimitiveBuildInputType,
    PrimitiveBuildValue, build_config_fingerprint, resolve_build_config_values,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::project_globals::{
    ProjectGlobalsFieldInput, ProjectGlobalsInterface,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
fn source_span_at(_path: &str, _string_table: &mut StringTable) -> SourceSpan {
    SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start())
}

fn source_span(string_table: &mut StringTable) -> SourceSpan {
    source_span_at("src/@page.moth", string_table)
}

fn command_location(argument_index: usize) -> BuildConfigValueLocation {
    BuildConfigValueLocation::Command(BuildCommandLocation::new(argument_index))
}

fn entry(
    name: &str,
    value: impl Into<BuildConfigInputValue>,
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
        "café",
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
        i32_value(-4).primitive_type(),
        PrimitiveBuildInputType::Fixed(FixedScalar::I32)
    );
    assert_eq!(
        float_value(0.75).primitive_type(),
        PrimitiveBuildInputType::Fixed(FixedScalar::F64)
    );
    assert_eq!(
        PrimitiveBuildValue::Bool(true).primitive_type(),
        PrimitiveBuildInputType::Bool
    );
    assert_eq!(
        PrimitiveBuildValue::Char(':').primitive_type(),
        PrimitiveBuildInputType::Char
    );
}

#[test]
fn build_input_types_accept_only_the_exact_primitive_or_matching_optional() {
    let primitives = [
        PrimitiveBuildInputType::String,
        PrimitiveBuildInputType::Fixed(FixedScalar::I32),
        PrimitiveBuildInputType::Fixed(FixedScalar::F64),
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
    let declared = BuildConfigValueLocation::Source(source_span(&mut string_table));

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
        .insert(entry("mid_way", i32_value(7), command_location(1)))
        .expect("third insert should succeed");

    assert_eq!(inputs.len(), 3);
    assert!(!inputs.is_empty());

    let names: Vec<&str> = inputs.iter().map(|input| input.name().as_str()).collect();
    assert_eq!(names, ["alpha", "mid_way", "zulu"]);

    let alpha_name = BuildInputName::new("alpha").expect("test input name should validate");
    let alpha = inputs.get(&alpha_name).expect("alpha should be present");
    assert_eq!(
        alpha.value(),
        &BuildConfigInputValue::from(PrimitiveBuildValue::String("a".to_owned()))
    );
    assert_eq!(alpha.location(), &declared);
}

#[test]
fn build_config_input_set_rejects_duplicate_names_deterministically() {
    let mut string_table = StringTable::new();
    let declared = BuildConfigValueLocation::Source(source_span(&mut string_table));

    let mut inputs = BuildConfigInputSet::new();
    inputs
        .insert(entry("retries", i32_value(4), declared.clone()))
        .expect("first insert should succeed");

    let duplicate = entry("retries", i32_value(9), command_location(3));
    let error = inputs
        .insert(duplicate)
        .expect_err("duplicate input name must be rejected");

    assert_eq!(
        error.rejected().value(),
        &BuildConfigInputValue::from(i32_value(9))
    );
    assert_eq!(error.rejected().location(), &command_location(3));
    assert_eq!(error.existing_location(), &declared);

    // The earlier typed value and location remain owned by the set.
    assert_eq!(inputs.len(), 1);
    let retries_name = BuildInputName::new("retries").expect("test input name should validate");
    let retained = inputs
        .get(&retries_name)
        .expect("retries should be present");
    assert_eq!(retained.value(), &BuildConfigInputValue::from(i32_value(4)));
    assert_eq!(retained.location(), &declared);
}

// -------------------------
//  Command-text primitive inference
// -------------------------

fn classify(value: &str) -> BuildConfigInputValue {
    BuildConfigInputValue::from_command_text(value)
        .expect("value should classify without a command-input diagnostic")
}

/// Typed command text (bool, string, char) is decided from the authored text alone.
fn infer(value: &str) -> PrimitiveBuildValue {
    match classify(value) {
        BuildConfigInputValue::Typed(value) => value,
        BuildConfigInputValue::Deferred(_) => panic!("typed command text should not defer"),
    }
}

fn infer_error(value: &str) -> BuildInputValueError {
    BuildConfigInputValue::from_command_text(value)
        .expect_err("value should produce a command-input diagnostic")
}

/// Numeric command text materialises once the receiving contract is known.
fn infer_at(value: &str, contract: PrimitiveBuildInputType) -> PrimitiveBuildValue {
    classify(value)
        .materialise_for(BuildInputType::Primitive(contract))
        .expect("value should materialise at the receiving contract")
}

fn infer_error_at(value: &str, contract: PrimitiveBuildInputType) -> ContractValueError {
    classify(value)
        .materialise_for(BuildInputType::Primitive(contract))
        .expect_err("value should be rejected by the receiving contract")
}

fn signed_value(scalar: FixedScalar, value: i64) -> PrimitiveBuildValue {
    let fixed = FixedScalarValue::signed(scalar, value);
    PrimitiveBuildValue::Fixed(fixed.expect("test integer should fit its scalar"))
}

fn i32_value(value: i64) -> PrimitiveBuildValue {
    signed_value(FixedScalar::I32, value)
}

fn f32_value(value: f32) -> PrimitiveBuildValue {
    let fixed = FixedScalarValue::binary_float(FixedScalar::F32, f64::from(value));
    PrimitiveBuildValue::Fixed(fixed.expect("test float should be finite at binary32"))
}

fn float_value(value: f64) -> PrimitiveBuildValue {
    let fixed = FixedScalarValue::binary_float(FixedScalar::F64, value);
    PrimitiveBuildValue::Fixed(fixed.expect("test float should be finite at binary64"))
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
fn command_text_materialises_complete_signed_whole_numbers_at_i32() {
    let i32_contract = PrimitiveBuildInputType::Fixed(FixedScalar::I32);
    assert_eq!(infer_at("0", i32_contract), i32_value(0));
    assert_eq!(infer_at("4", i32_contract), i32_value(4));
    assert_eq!(infer_at("-7", i32_contract), i32_value(-7));
    assert_eq!(infer_at("-0", i32_contract), i32_value(0));
    assert_eq!(
        infer_at("2147483647", i32_contract),
        i32_value(i64::from(i32::MAX))
    );
    assert_eq!(
        infer_at("-2147483648", i32_contract),
        i32_value(i64::from(i32::MIN))
    );
    assert_eq!(infer_at("1_000", i32_contract), i32_value(1_000));
    assert_eq!(infer_at("-1_0", i32_contract), i32_value(-10));
}

#[test]
fn command_text_materialises_decimal_and_exponent_literals_at_f64() {
    let f64_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F64);
    assert_eq!(infer_at("0.75", f64_contract), float_value(0.75));
    assert_eq!(infer_at("-1.5", f64_contract), float_value(-1.5));
    assert_eq!(infer_at("1_000.5", f64_contract), float_value(1_000.5));
    assert_eq!(infer_at("1e6", f64_contract), float_value(1e6));
    assert_eq!(infer_at("2e31", f64_contract), float_value(2e31));
    assert_eq!(infer_at("-1.5e-2", f64_contract), float_value(-1.5e-2));
    // An exponent literal is a float even when its value is integral.
    assert_eq!(infer_at("2e3", f64_contract), float_value(2e3));
}

#[test]
fn command_text_defers_numeric_range_failures_to_the_receiving_scalar() {
    // Integer-shaped text is deferred, so the I32 that receives it diagnoses the range.
    let i32_contract = PrimitiveBuildInputType::Fixed(FixedScalar::I32);
    for value in ["2147483648", "-2147483649", "99999999999999999999"] {
        assert_eq!(
            infer_error_at(value, i32_contract),
            ContractValueError::LiteralOutOfRange {
                text: String::from(value),
                reason: NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
            }
        );
    }

    // Non-finite decimal text diagnoses at the F64 that receives it.
    let f64_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F64);
    for value in ["1e400", "-1e400"] {
        assert_eq!(
            infer_error_at(value, f64_contract),
            ContractValueError::LiteralOutOfRange {
                text: String::from(value),
                reason: NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F64),
            }
        );
    }
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
    assert_eq!(
        infer_at("5", PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        i32_value(5)
    );
    assert_eq!(infer("-"), PrimitiveBuildValue::String(String::from("-")));
}

/// Numeric command text has no width of its own: the receiving scalar decides whether it fits
/// and how it rounds, so one authored text can differ between two contracts.
#[test]
fn command_text_materialises_each_numeric_value_at_its_receiving_scalar() {
    let i32_contract = PrimitiveBuildInputType::Fixed(FixedScalar::I32);
    let i64_contract = PrimitiveBuildInputType::Fixed(FixedScalar::I64);
    let f32_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F32);
    let f64_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F64);

    // 3_000_000_000 is outside I32 but exact in I64.
    assert_eq!(
        infer_at("3000000000", i64_contract),
        signed_value(FixedScalar::I64, 3_000_000_000)
    );
    assert_eq!(
        infer_error_at("3000000000", i32_contract),
        ContractValueError::LiteralOutOfRange {
            text: String::from("3000000000"),
            reason: NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
        }
    );

    // 0.1 rounds once to the nearest binary32 value, which is not the nearest binary64 value.
    assert_eq!(infer_at("0.1", f32_contract), f32_value(0.1));
    assert_eq!(infer_at("0.1", f64_contract), float_value(0.1));
    assert_ne!(infer_at("0.1", f32_contract), infer_at("0.1", f64_contract));

    // 1e39 overflows binary32 but is finite in binary64.
    assert_eq!(
        infer_error_at("1e39", f32_contract),
        ContractValueError::LiteralOutOfRange {
            text: String::from("1e39"),
            reason: NumberLiteralErrorReason::NonFiniteFixedFloat(FixedScalar::F32),
        }
    );
    assert_eq!(infer_at("1e39", f64_contract), float_value(1e39));

    // A whole spelling materialises directly at a float contract; a fractional spelling never
    // narrows to an integer contract.
    assert_eq!(infer_at("1", f32_contract), f32_value(1.0));
    assert_eq!(
        infer_error_at("1.0", i32_contract),
        ContractValueError::TypeMismatch {
            provided: PrimitiveBuildInputType::Fixed(FixedScalar::F64),
        }
    );
}

#[test]
fn typed_numeric_inputs_require_exact_contracts_without_reparsing() {
    for (value, scalar, other_scalar) in [
        (
            signed_value(FixedScalar::I64, 1),
            FixedScalar::I64,
            FixedScalar::I32,
        ),
        (float_value(1.0), FixedScalar::F64, FixedScalar::F32),
    ] {
        let input = BuildConfigInputValue::Typed(value.clone());
        let exact_contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(scalar));
        let other_contract =
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(other_scalar));
        assert_eq!(input.materialise_for(exact_contract), Ok(value),);
        assert_eq!(
            input.materialise_for(other_contract),
            Err(ContractValueError::TypeMismatch {
                provided: PrimitiveBuildInputType::Fixed(scalar),
            }),
        );
    }
}

/// Spellings just above a binary midpoint round once at the receiving precision. Rounding first
/// through binary64 would land exactly on the midpoint and tie to the even neighbour below.
#[test]
fn command_text_rounds_binary_midpoint_spellings_once_at_the_receiving_precision() {
    let f16_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F16);
    let f32_contract = PrimitiveBuildInputType::Fixed(FixedScalar::F32);

    // 1 + 2^-11 is the F16 midpoint between 1 and 1 + 2^-10.
    let above_f16_midpoint = "1.00048828125000000001";
    let f16_above = FixedScalarValue::binary_float(FixedScalar::F16, 1.0 + f64::powi(2.0, -10))
        .expect("1 + 2^-10 is finite at binary16");
    assert_eq!(
        infer_at(above_f16_midpoint, f16_contract),
        PrimitiveBuildValue::Fixed(f16_above)
    );

    // 1 + 2^-24 is the F32 midpoint between 1 and 1 + 2^-23.
    let above_f32_midpoint = "1.00000005960464477539062501";
    assert_eq!(
        infer_at(above_f32_midpoint, f32_contract),
        f32_value(1.0 + f32::EPSILON)
    );
    assert_eq!(
        infer_at("1.000000059604644775390625", f32_contract),
        f32_value(1.0)
    );
}

/// A numeric fingerprint names the exact scalar its value was materialised at. The same value at
/// another scalar therefore never shares an identity with it.
#[test]
fn build_config_fingerprints_name_the_exact_scalar_of_numeric_values() {
    let i32_seven = i32_value(7);
    let i64_seven = signed_value(FixedScalar::I64, 7);
    let f32_three_quarters = f32_value(0.75);
    let f64_three_quarters = float_value(0.75);
    let i32_contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32));
    let i64_contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I64));
    let f32_contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::F32));
    let f64_contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::F64));

    assert_ne!(
        build_config_fingerprint("field", i32_contract, Some(&i32_seven)),
        build_config_fingerprint("field", i64_contract, Some(&i64_seven)),
        "an I32 field must not share an identity with an I64 field of the same value"
    );
    assert_ne!(
        build_config_fingerprint("field", f32_contract, Some(&f32_three_quarters)),
        build_config_fingerprint("field", f64_contract, Some(&f64_three_quarters)),
        "an F32 field must not share an identity with an F64 field of the same value"
    );
    assert_eq!(
        build_config_fingerprint("field", i32_contract, Some(&i32_seven)),
        build_config_fingerprint("field", i32_contract, Some(&i32_value(7))),
        "equal exact values must fingerprint identically"
    );
}

/// Explicit numeric input materialises at the scalar its contract names. A literal that fits a
/// wider contract diagnoses against a narrower one and resolves exactly at the wider one.
#[test]
fn resolution_materialises_explicit_literals_at_the_contract_scalar() {
    let mut string_table = StringTable::new();
    let mut inputs = BuildConfigInputSet::new();
    inputs
        .insert(entry(
            "count",
            BuildConfigInputValue::from_command_text("3000000000")
                .expect("test command text should classify"),
            command_location(0),
        ))
        .expect("test input should not duplicate");

    let i32_facts = vec![contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        None,
    )];
    let error = resolve_build_config_values(
        &i32_facts,
        &[],
        &[],
        &inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("an explicit literal outside its I32 contract must not resolve");
    assert_eq!(error.name().as_str(), "count");
    let BuildConfigResolutionError::NumericLiteralOutOfRange { text, reason, .. } = &error else {
        panic!("expected a numeric literal range error, got {error:?}");
    };
    assert_eq!(text, "3000000000");
    assert_eq!(
        *reason,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32)
    );

    let i64_facts = vec![contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I64)),
        true,
        None,
    )];
    let resolved = resolve_build_config_values(
        &i64_facts,
        &[],
        &[],
        &inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect("the I64 contract must accept the same literal");
    let name = BuildInputName::new("count").expect("test input name should validate");
    assert_eq!(
        resolved.get(&name).expect("count should resolve").value(),
        Some(&signed_value(FixedScalar::I64, 3_000_000_000))
    );
}

/// A default is typed by its contract, so a default of another scalar is rejected at
/// resolution while the same scalar resolves.
#[test]
fn resolution_rejects_a_default_of_another_scalar() {
    let mut string_table = StringTable::new();
    let i32_facts = vec![contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        false,
        Some(signed_value(FixedScalar::I64, 3_000_000_000)),
    )];

    let error = resolve_build_config_values(
        &i32_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("a default of another scalar must not resolve");
    assert_eq!(error.name().as_str(), "count");
    let BuildConfigResolutionError::DefaultTypeMismatch { provided, .. } = &error else {
        panic!("expected a default type mismatch, got {error:?}");
    };
    assert_eq!(*provided, PrimitiveBuildInputType::Fixed(FixedScalar::I64));

    let i64_facts = vec![contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I64)),
        false,
        Some(signed_value(FixedScalar::I64, 3_000_000_000)),
    )];
    let resolved = resolve_build_config_values(
        &i64_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect("the I64 contract must accept its own default");
    let name = BuildInputName::new("count").expect("test input name should validate");
    assert_eq!(
        resolved.get(&name).expect("count should resolve").value(),
        Some(&signed_value(FixedScalar::I64, 3_000_000_000))
    );
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
        Some(source_span(string_table)),
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
    value: i64,
    fingerprint: BuildConfigFingerprint,
) -> ProjectGlobalsInterface {
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        crate::compiler_frontend::project_globals::PROJECT_GLOBALS_DEPENDENCY_NAME,
        name,
    );
    let field = ProjectGlobalsFieldInput::new(
        name,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32)),
        PublicFoldedValue::FixedScalar(
            FixedScalarValue::signed(FixedScalar::I32, value).expect("test i64 fits I32"),
        ),
        Some(SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::source_start(),
        )),
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
    let value = i32_value(7);
    let contract = BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32));
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
        build_config_fingerprint("same_value", contract, Some(&value),)
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
            .fingerprint,
        defaulted_project
            .member("same_value")
            .expect("defaulted project field should exist")
            .fingerprint
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        Some(i32_value(41)),
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
        .insert(entry("fixed_value", i32_value(99), command_location(0)))
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
        ("global_value", i32_value(7)),
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
        Some(&i32_value(41))
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
        Some(&i32_value(7))
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
                BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
                first: PrimitiveBuildInputType::Fixed(FixedScalar::I32),
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
                BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
                true,
                None,
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
                BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
                true,
                None,
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
                BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
                false,
                Some(i32_value(1)),
            ),
            contract_fact(
                &mut string_table,
                "setting",
                BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
                false,
                Some(i32_value(2)),
            ),
        ],
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different normalized source defaults must conflict");
    let BuildConfigResolutionError::SourceContractConflict {
        reason: BuildConfigContractConflictReason::Default { first, conflicting },
        ..
    } = default_error
    else {
        panic!("expected a default conflict, found {default_error:?}");
    };
    assert_eq!(first, Some(i32_value(1)));
    assert_eq!(conflicting, Some(i32_value(2)));
}

#[test]
fn boundary_resolver_checks_project_contracts_against_source_contracts() {
    let mut string_table = StringTable::new();
    let direct_error = resolve_build_config_values(
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
                conflicting: PrimitiveBuildInputType::Fixed(FixedScalar::I32),
            },
            ..
        }
    ));

    let fixed_error = resolve_build_config_values(
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            true,
            None,
        )],
        &[contract_fact(
            &mut string_table,
            "setting",
            BuildInputType::Optional(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            false,
            Some(i32_value(4)),
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            false,
            Some(i32_value(1)),
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
                conflicting: PrimitiveBuildInputType::Fixed(FixedScalar::I32),
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
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        false,
        Some(i32_value(7)),
    )];
    let first_check_only_facts = vec![contract_fact(
        &mut string_table,
        "transient_value",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        false,
        Some(i32_value(1)),
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
    let changed_globals = builder_globals(&[("canonical_value", i32_value(99))]);
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
        Some(&i32_value(1))
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
fn check_only_conflict_keeps_canonical_and_transient_spans() {
    let mut string_table = StringTable::new();
    let canonical_span = source_span(&mut string_table);
    let transient_span = source_span(&mut string_table);

    let name = BuildInputName::new("shared_setting").expect("test input name should validate");
    let canonical_fact = BuildConfigContractFact::new(
        name.clone(),
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        None,
        Some(canonical_span),
    );
    let transient_fact = BuildConfigContractFact::new(
        name,
        BuildInputType::Primitive(PrimitiveBuildInputType::String),
        true,
        None,
        Some(transient_span),
    );
    let canonical_values = resolve_build_config_values(
        std::slice::from_ref(&canonical_fact),
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &builder_globals(&[("shared_setting", i32_value(7))]),
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

    assert_eq!(first.span(), Some(canonical_span));
    assert_eq!(conflicting.span(), Some(transient_span));
}
#[test]
fn boundary_resolver_reports_typed_value_mismatches_with_locations() {
    let mut string_table = StringTable::new();
    let source_facts = [contract_fact(
        &mut string_table,
        "count",
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
    .expect_err("a String must not satisfy an I32 contract");
    assert_eq!(error.name().as_str(), "count");
    assert_eq!(error.value_location(), Some(&command_location(4)));
    assert!(matches!(
        &error,
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
        &builder_globals(&[("enabled", i32_value(1))]),
    )
    .expect_err("a builder I32 must not satisfy a Bool contract");
    assert!(matches!(
        &builder_error,
        BuildConfigResolutionError::ValueTypeMismatch {
            provided: PrimitiveBuildInputType::Fixed(FixedScalar::I32),
            ..
        }
    ));
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
    assert_eq!(error.contract_span(), None);
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
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            false,
            Some(i32_value(8)),
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            false,
            Some(i32_value(8)),
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
