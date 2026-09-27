//! Literal match-pattern parsing regression tests.
//!
//! WHAT: validates destination-aware fixed scalar and profile Float literal patterns, existing
//!       `Int` boundaries, and relational-pattern literal typing.
//! WHY: pattern literals have an independent parser path whose receiver-aware and signed numeric
//!      behavior must stay aligned with the shared materialization contract.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, RelationalPatternOp,
    literal::{parse_literal_pattern, parse_non_choice_pattern},
    option::parse_option_pattern,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, NumberLiteralErrorReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};

type LiteralPatternTestResult<T> = Result<T, ExpressionParseError>;

#[test]
fn parse_literal_pattern_accepts_i32_boundary_values() {
    let max = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483647").unwrap();
    assert!(matches!(max.kind, ExpressionKind::Int(2147483647)));

    let min = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483648").unwrap();
    assert!(matches!(min.kind, ExpressionKind::Int(-2147483648)));
}

#[test]
fn parse_literal_pattern_rejects_i32_out_of_range() {
    let error = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483648").unwrap_err();
    assert_invalid_number_literal_reason(error, NumberLiteralErrorReason::OutsideIntRange);

    let error = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483649").unwrap_err();
    assert_invalid_number_literal_reason(error, NumberLiteralErrorReason::OutsideIntRange);
}

#[test]
fn parse_literal_pattern_negative_fallback_allows_i32_min() {
    let min = parse_negative_number_pattern("2147483648").unwrap();
    assert!(matches!(min.kind, ExpressionKind::Int(-2147483648)));
}

#[test]
fn parse_literal_pattern_negative_fallback_rejects_i32_underflow() {
    let error = parse_negative_number_pattern("2147483649").unwrap_err();
    assert_invalid_number_literal_reason(error, NumberLiteralErrorReason::OutsideIntRange);
}

#[test]
fn fixed_scalar_literal_patterns_materialize_at_the_subject_type() {
    let u8_pattern = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "200",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_fixed_scalar_pattern(&u8_pattern, FixedScalar::U8, None, Some(200), None);

    let i8_min = parse_negative_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::I8),
        "128",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_fixed_scalar_pattern(&i8_min, FixedScalar::I8, Some(-128), None, None);

    let f16 = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::F16),
        "0.5",
        NumericLiteralKind::DecimalPoint,
    )
    .unwrap();
    assert_fixed_scalar_pattern(&f16, FixedScalar::F16, None, None, Some(0.5));

    let byte = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::Byte),
        "255",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_fixed_scalar_pattern(&byte, FixedScalar::Byte, None, Some(255), None);
}

#[test]
fn profile_float_subject_patterns_materialise_whole_literals_directly() {
    let large = parse_numeric_pattern(
        builtin_type_ids::FLOAT,
        "18_446_744_073_709_551_615",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_eq!(large.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(large.kind, ExpressionKind::Float(value) if value == 18_446_744_073_709_551_615_f64),
        "Float match subjects should materialise whole literals as profile Float"
    );

    let negative_zero = parse_negative_numeric_pattern(
        builtin_type_ids::FLOAT,
        "0",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert!(
        matches!(negative_zero.kind, ExpressionKind::Float(value) if value.to_bits() == (-0.0_f64).to_bits()),
        "Float match subjects should preserve the sign of whole-number zero"
    );
}

#[test]
fn optional_fixed_scalar_literal_pattern_materializes_at_the_inner_type() {
    let pattern = parse_optional_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "200",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    let MatchPattern::OptionValue { value, .. } = pattern else {
        panic!("fixed literal on an optional scalar should match its present value");
    };
    assert_fixed_scalar_pattern(&value, FixedScalar::U8, None, Some(200), None);
}

#[test]
fn fixed_scalar_literal_patterns_preserve_range_sign_and_kind_diagnostics() {
    let out_of_range = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "300",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap_err();
    assert_invalid_number_literal_reason(
        out_of_range,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U8),
    );

    let negative_unsigned = parse_negative_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U16),
        "1",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap_err();
    assert_invalid_number_literal_reason(
        negative_unsigned,
        NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U16),
    );

    let decimal_integer = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        "1.5",
        NumericLiteralKind::DecimalPoint,
    )
    .unwrap_err();
    assert_match_pattern_type_mismatch(decimal_integer);
}

#[test]
fn fixed_integer_patterns_reject_integral_decimal_and_exponent_literals() {
    for (literal, kind) in [
        ("1.0", NumericLiteralKind::DecimalPoint),
        ("1e0", NumericLiteralKind::Exponent),
    ] {
        let error = parse_numeric_pattern(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            literal,
            kind,
        )
        .unwrap_err();
        assert_match_pattern_type_mismatch(error);
    }
}

#[test]
fn byte_literal_patterns_reject_negative_out_of_range_and_decimal_values() {
    let byte_type = builtin_type_ids::fixed_scalar(FixedScalar::Byte);

    let negative = parse_negative_numeric_pattern(byte_type, "1", NumericLiteralKind::WholeNumber)
        .unwrap_err();
    assert_invalid_number_literal_reason(
        negative,
        NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::Byte),
    );

    let out_of_range =
        parse_numeric_pattern(byte_type, "256", NumericLiteralKind::WholeNumber).unwrap_err();
    assert_invalid_number_literal_reason(
        out_of_range,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::Byte),
    );

    let decimal =
        parse_numeric_pattern(byte_type, "1.0", NumericLiteralKind::DecimalPoint).unwrap_err();
    assert_match_pattern_type_mismatch(decimal);
}

#[test]
fn relational_patterns_materialize_fixed_integer_float_and_byte_literals() {
    let u64_pattern = parse_relational_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U64),
        TokenTag::LESS_THAN,
        "10",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_relational_fixed_scalar_pattern(
        &u64_pattern,
        RelationalPatternOp::LessThan,
        FixedScalar::U64,
        None,
        Some(10),
        None,
    );

    let f32_pattern = parse_relational_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::F32),
        TokenTag::GREATER_THAN_OR_EQUAL,
        "0.5",
        NumericLiteralKind::DecimalPoint,
    )
    .unwrap();
    assert_relational_fixed_scalar_pattern(
        &f32_pattern,
        RelationalPatternOp::GreaterThanOrEqual,
        FixedScalar::F32,
        None,
        None,
        Some(0.5),
    );

    let byte_pattern = parse_relational_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::Byte),
        TokenTag::LESS_THAN,
        "255",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_relational_fixed_scalar_pattern(
        &byte_pattern,
        RelationalPatternOp::LessThan,
        FixedScalar::Byte,
        None,
        Some(255),
        None,
    );
}

#[test]
fn malformed_numeric_literal_payload_is_infrastructure_error() {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("valid test tokens should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("valid test tokens should build");
    let mut owner = builder
        .finish()
        .expect("valid test tokens should build a canonical source owner");
    std::sync::Arc::get_mut(&mut owner)
        .expect("the test owner should remain uniquely owned")
        .corrupt_payload_for_test(0, TokenTag::NUMERIC_LITERAL);
    let range = owner
        .full_range()
        .expect("the canonical test owner should expose a full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("the malformed canonical owner should still construct a cursor");
    let type_environment = TypeEnvironment::new();

    let error = parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        NumericProfile::STANDARD,
        &mut StringTable::new(),
        &type_environment,
    )
    .expect_err("a malformed trusted literal payload must not become syntax");

    assert!(matches!(error, ExpressionParseError::Infrastructure(_)));
}

fn assert_invalid_number_literal_reason(
    error: ExpressionParseError,
    expected_reason: NumberLiteralErrorReason,
) {
    let diagnostic = match error {
        ExpressionParseError::Diagnostic(diagnostic) => diagnostic,
        ExpressionParseError::Infrastructure(error) => {
            panic!("literal pattern infrastructure failure is not a source diagnostic: {error:?}")
        }
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral { reason, .. } if reason == expected_reason
    ));
}

fn assert_match_pattern_type_mismatch(error: ExpressionParseError) {
    let diagnostic = match error {
        ExpressionParseError::Diagnostic(diagnostic) => diagnostic,
        ExpressionParseError::Infrastructure(error) => {
            panic!("literal pattern mismatch must be a source diagnostic: {error:?}")
        }
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::MatchPattern,
            ..
        }
    ));
}

fn assert_fixed_scalar_pattern(
    expression: &Expression,
    expected_scalar: FixedScalar,
    signed_value: Option<i64>,
    unsigned_value: Option<u64>,
    float_value: Option<f64>,
) {
    assert_eq!(
        expression.type_id,
        builtin_type_ids::fixed_scalar(expected_scalar)
    );
    let ExpressionKind::FixedScalar(value) = &expression.kind else {
        panic!("expected a fixed scalar pattern constant");
    };

    assert_eq!(value.scalar(), expected_scalar);
    assert_eq!(value.as_i64(), signed_value);
    assert_eq!(value.as_u64(), unsigned_value);
    assert_eq!(value.as_f64(), float_value);
}

fn assert_relational_fixed_scalar_pattern(
    pattern: &MatchPattern,
    expected_operator: RelationalPatternOp,
    expected_scalar: FixedScalar,
    signed_value: Option<i64>,
    unsigned_value: Option<u64>,
    float_value: Option<f64>,
) {
    let MatchPattern::Relational { op, value, .. } = pattern else {
        panic!("relational parser should produce a relational pattern");
    };

    assert_eq!(*op, expected_operator);
    assert_fixed_scalar_pattern(
        value,
        expected_scalar,
        signed_value,
        unsigned_value,
        float_value,
    );
}

fn parse_whole_number_pattern(
    sign: NumericLiteralSign,
    normalized_text: &str,
) -> LiteralPatternTestResult<Expression> {
    parse_numeric_pattern_with_sign(
        builtin_type_ids::INT,
        NumericLiteralKind::WholeNumber,
        sign,
        normalized_text,
    )
}

fn parse_negative_number_pattern(normalized_text: &str) -> LiteralPatternTestResult<Expression> {
    parse_negative_numeric_pattern(
        builtin_type_ids::INT,
        normalized_text,
        NumericLiteralKind::WholeNumber,
    )
}

fn parse_numeric_pattern(
    subject_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<Expression> {
    parse_numeric_pattern_with_sign(
        subject_type_id,
        kind,
        NumericLiteralSign::Positive,
        normalized_text,
    )
}

fn parse_numeric_pattern_with_sign(
    subject_type_id: TypeId,
    kind: NumericLiteralKind,
    sign: NumericLiteralSign,
    normalized_text: &str,
) -> LiteralPatternTestResult<Expression> {
    let mut string_table = StringTable::new();
    let literal = numeric_literal_token(kind, sign, normalized_text, &mut string_table);
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        subject_type_id,
        NumericProfile::STANDARD,
        &mut string_table,
        &type_environment,
    )
}

fn parse_negative_numeric_pattern(
    subject_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<Expression> {
    let mut string_table = StringTable::new();
    let literal = numeric_literal_token(
        kind,
        NumericLiteralSign::Positive,
        normalized_text,
        &mut string_table,
    );
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_static(TokenTag::NEGATIVE, LocalSpan::source_start())
        .expect("negative fixture token should build");
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        subject_type_id,
        NumericProfile::STANDARD,
        &mut string_table,
        &type_environment,
    )
}

fn parse_relational_pattern(
    subject_type_id: TypeId,
    operator: TokenTag,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<MatchPattern> {
    let mut string_table = StringTable::new();
    let literal = numeric_literal_token(
        kind,
        NumericLiteralSign::Positive,
        normalized_text,
        &mut string_table,
    );
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_static(operator, LocalSpan::source_start())
        .expect("relational fixture token should build");
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_non_choice_pattern(
        &mut token_stream,
        subject_type_id,
        NumericProfile::STANDARD,
        &mut string_table,
        &type_environment,
    )
}

fn parse_optional_numeric_pattern(
    inner_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<MatchPattern> {
    let mut string_table = StringTable::new();
    let literal = numeric_literal_token(
        kind,
        NumericLiteralSign::Positive,
        normalized_text,
        &mut string_table,
    );
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("optional numeric pattern token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_option_pattern(
        &mut token_stream,
        inner_type_id,
        NumericProfile::STANDARD,
        &mut string_table,
        &type_environment,
    )
}

fn numeric_literal_token(
    kind: NumericLiteralKind,
    sign: NumericLiteralSign,
    normalized_text: &str,
    string_table: &mut StringTable,
) -> NumericLiteralToken {
    let source = match sign {
        NumericLiteralSign::Positive => normalized_text.to_owned(),
        NumericLiteralSign::Negative => format!("-{normalized_text}"),
    };
    let mut token = NumericLiteralToken::test_new(normalized_text, string_table);
    assert_eq!(token.kind, kind);
    token.sign = sign;
    token.source_text = string_table.intern(&source);
    token
}
