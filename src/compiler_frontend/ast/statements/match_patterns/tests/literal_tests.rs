//! Literal match-pattern parsing regression tests.
//!
//! WHAT: validates destination-aware fixed scalar literal patterns, exact I32 subject boundaries,
//!       and relational-pattern literal typing.
//! WHY: pattern literals have an independent parser path whose receiver-aware and signed numeric
//!      behavior must stay aligned with the shared materialization contract.

use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::numeric_construction::DeferredArmPattern;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, MatchSubject, PatternValue, PatternValueKind, RelationalPatternOp,
    literal::{parse_literal_pattern, parse_non_choice_pattern},
    option::parse_option_pattern,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, TypeMismatchContext};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::grammar::{NumericLiteralKind, NumericLiteralSign};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

type LiteralPatternTestResult<T> = Result<T, ExpressionParseError>;

#[derive(Debug)]
struct TestValue {
    value: PatternValue,
    constants: ConstValueStore,
}

#[derive(Debug)]
struct TestPattern {
    pattern: MatchPattern,
    constants: ConstValueStore,
}

#[test]
fn folded_pattern_retains_occurrence_metadata_without_an_expression_edge() {
    let fixture = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "200",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_eq!(
        fixture.value.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert!(fixture.value.span.is_some());
    let pattern = MatchPattern::Literal(fixture.value);
    let mut edges = Vec::new();
    pattern.visit_expression_ids(&mut |id| edges.push(id));
    assert!(edges.is_empty());
}

#[test]
fn runtime_pattern_value_keeps_a_remappable_expression_edge() {
    use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
    use crate::compiler_frontend::symbols::path_interner::PathId;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut builder = AstExpressionBuilder::new().unwrap();
    let type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let original = builder
        .reference_with_type_id(
            PathId::ROOT,
            type_id,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .unwrap();
    let replacement = builder
        .reference_with_type_id(
            PathId::ROOT,
            type_id,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .unwrap();
    let value = PatternValue::from_expression(builder.view_owned(original).unwrap()).unwrap();
    assert!(matches!(value.kind, PatternValueKind::Runtime(id) if id == original.id()));
    let mut pattern = MatchPattern::Literal(value);
    pattern.visit_expression_ids_mut(&mut |id| *id = replacement.id());
    let mut edges = Vec::new();
    pattern.visit_expression_ids(&mut |id| edges.push(id));
    assert_eq!(edges, vec![replacement.id()]);
}

#[test]
fn parse_literal_pattern_accepts_i32_boundary_values() {
    let max = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483647").unwrap();
    assert_fixed_scalar_pattern(&max, FixedScalar::I32, Some(2147483647), None, None);

    let min = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483648").unwrap();
    assert_fixed_scalar_pattern(&min, FixedScalar::I32, Some(-2147483648), None, None);
}

#[test]
fn parse_literal_pattern_rejects_i32_out_of_range() {
    let error = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483648").unwrap_err();
    assert_invalid_number_literal_reason(
        error,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
    );

    let error = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483649").unwrap_err();
    assert_invalid_number_literal_reason(
        error,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
    );
}

#[test]
fn parse_literal_pattern_negative_lead_token_allows_i32_min() {
    let min = parse_negative_number_pattern("2147483648").unwrap();
    assert_fixed_scalar_pattern(&min, FixedScalar::I32, Some(-2147483648), None, None);
}

#[test]
fn parse_literal_pattern_negative_lead_token_rejects_i32_underflow() {
    let error = parse_negative_number_pattern("2147483649").unwrap_err();
    assert_invalid_number_literal_reason(
        error,
        NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
    );
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
fn f64_subject_patterns_materialise_whole_literals_directly() {
    let large = parse_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        "18_446_744_073_709_551_615",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_fixed_scalar_pattern(
        &large,
        FixedScalar::F64,
        None,
        None,
        Some(18_446_744_073_709_551_615_f64),
    );

    let negative_zero = parse_negative_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        "0",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    assert_float_pattern_bits(&negative_zero, FixedScalar::F64, (-0.0_f64).to_bits());
}

#[test]
fn f64_subject_patterns_apply_attached_numeric_sign_once() {
    let negative_decimal = parse_numeric_pattern_with_sign(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        NumericLiteralKind::DecimalPoint,
        NumericLiteralSign::Negative,
        "1.5",
    )
    .unwrap();
    assert_fixed_scalar_pattern(&negative_decimal, FixedScalar::F64, None, None, Some(-1.5));

    let negative_exponent = parse_numeric_pattern_with_sign(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        NumericLiteralKind::Exponent,
        NumericLiteralSign::Negative,
        "15e-1",
    )
    .unwrap();
    assert_fixed_scalar_pattern(&negative_exponent, FixedScalar::F64, None, None, Some(-1.5));

    let negative_zero = parse_numeric_pattern_with_sign(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        NumericLiteralKind::DecimalPoint,
        NumericLiteralSign::Negative,
        "0.0",
    )
    .unwrap();
    assert_float_pattern_bits(&negative_zero, FixedScalar::F64, (-0.0_f64).to_bits());
}

#[test]
fn optional_fixed_scalar_literal_pattern_materializes_at_the_inner_type() {
    let pattern = parse_optional_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::U8),
        "200",
        NumericLiteralKind::WholeNumber,
    )
    .unwrap();
    let MatchPattern::OptionValue { value, .. } = pattern.pattern else {
        panic!("fixed literal on an optional scalar should match its present value");
    };
    assert_fixed_scalar_pattern(
        &TestValue {
            value,
            constants: pattern.constants,
        },
        FixedScalar::U8,
        None,
        Some(200),
        None,
    );
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
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        &mut StringTable::new(),
        &type_environment,
        &mut AstExpressionBuilder::new().unwrap(),
        &mut ConstValueStore::default(),
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
        ExpressionParseError::NumericDeferral(deferral) => {
            panic!("literal pattern numeric deferral is not a source diagnostic: {deferral:?}")
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
        ExpressionParseError::NumericDeferral(deferral) => {
            panic!("literal pattern mismatch must be a source diagnostic: {deferral:?}")
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
    expression: &TestValue,
    expected_scalar: FixedScalar,
    signed_value: Option<i64>,
    unsigned_value: Option<u64>,
    float_value: Option<f64>,
) {
    assert_eq!(
        expression.value.type_id,
        builtin_type_ids::fixed_scalar(expected_scalar)
    );
    let PatternValueKind::Constant(id) = expression.value.kind else {
        panic!("expected a constant pattern value");
    };
    let Some(ConstValuePayload::FixedScalar(value)) = expression.constants.payload(id) else {
        panic!("expected a fixed scalar pattern constant");
    };

    assert_eq!(value.scalar(), expected_scalar);
    assert_eq!(value.as_i64(), signed_value);
    assert_eq!(value.as_u64(), unsigned_value);
    assert_eq!(value.as_f64(), float_value);
}

/// Assert a fixed binary-float pattern at its exact payload bits.
///
/// Signed zero compares equal to positive zero through `f64` equality, so only the bit pattern
/// can pin a preserved sign.
fn assert_float_pattern_bits(
    expression: &TestValue,
    expected_scalar: FixedScalar,
    expected_bits: u64,
) {
    let PatternValueKind::Constant(id) = expression.value.kind else {
        panic!("expected a constant pattern value");
    };
    let Some(ConstValuePayload::FixedScalar(value)) = expression.constants.payload(id) else {
        panic!("expected a fixed scalar pattern constant");
    };

    assert_eq!(value.scalar(), expected_scalar);
    assert_eq!(value.as_f64().map(f64::to_bits), Some(expected_bits));
}

fn assert_relational_fixed_scalar_pattern(
    pattern: &TestPattern,
    expected_operator: RelationalPatternOp,
    expected_scalar: FixedScalar,
    signed_value: Option<i64>,
    unsigned_value: Option<u64>,
    float_value: Option<f64>,
) {
    let MatchPattern::Relational { op, value, .. } = &pattern.pattern else {
        panic!("relational parser should produce a relational pattern");
    };

    assert_eq!(*op, expected_operator);
    let PatternValueKind::Constant(id) = value.kind else {
        panic!("expected a constant relational pattern value");
    };
    assert_eq!(
        value.type_id,
        builtin_type_ids::fixed_scalar(expected_scalar)
    );
    let Some(ConstValuePayload::FixedScalar(value)) = pattern.constants.payload(id) else {
        panic!("expected a fixed scalar relational pattern constant");
    };
    assert_eq!(value.scalar(), expected_scalar);
    assert_eq!(value.as_i64(), signed_value);
    assert_eq!(value.as_u64(), unsigned_value);
    assert_eq!(value.as_f64(), float_value);
}

fn parse_whole_number_pattern(
    sign: NumericLiteralSign,
    normalized_text: &str,
) -> LiteralPatternTestResult<TestValue> {
    parse_numeric_pattern_with_sign(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        NumericLiteralKind::WholeNumber,
        sign,
        normalized_text,
    )
}

fn parse_negative_number_pattern(normalized_text: &str) -> LiteralPatternTestResult<TestValue> {
    parse_negative_numeric_pattern(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        normalized_text,
        NumericLiteralKind::WholeNumber,
    )
}

fn parse_numeric_pattern(
    subject_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<TestValue> {
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
) -> LiteralPatternTestResult<TestValue> {
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

    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let id = parse_literal_pattern(
        &mut token_stream,
        subject_type_id,
        &mut string_table,
        &type_environment,
        &mut expression_builder,
        &mut constants,
    )?;
    Ok(TestValue {
        value: PatternValue::from_expression(expression_builder.view(id)?)?,
        constants,
    })
}

fn parse_negative_numeric_pattern(
    subject_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<TestValue> {
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

    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let id = parse_literal_pattern(
        &mut token_stream,
        subject_type_id,
        &mut string_table,
        &type_environment,
        &mut expression_builder,
        &mut constants,
    )?;
    Ok(TestValue {
        value: PatternValue::from_expression(expression_builder.view(id)?)?,
        constants,
    })
}

fn parse_relational_pattern(
    subject_type_id: TypeId,
    operator: TokenTag,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<TestPattern> {
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

    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let draft = parse_non_choice_pattern(
        &mut token_stream,
        MatchSubject::Known(subject_type_id),
        &mut string_table,
        &type_environment,
        &mut expression_builder,
        &mut constants,
    )?;

    // A known subject always parses to a ready pattern; a retained draft would be a parser
    // regression, so the test fails loudly instead of hiding it.
    match draft {
        DeferredArmPattern::Ready(pattern) => Ok(TestPattern { pattern, constants }),
        DeferredArmPattern::Retained(_)
        | DeferredArmPattern::PendingChoice(_)
        | DeferredArmPattern::PendingOption(_) => {
            panic!("known subject {subject_type_id:?} produced a retained pattern draft")
        }
    }
}

fn parse_optional_numeric_pattern(
    inner_type_id: TypeId,
    normalized_text: &str,
    kind: NumericLiteralKind,
) -> LiteralPatternTestResult<TestPattern> {
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

    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let pattern = parse_option_pattern(
        &mut token_stream,
        inner_type_id,
        &mut string_table,
        &type_environment,
        &mut expression_builder,
        &mut constants,
    )?;
    Ok(TestPattern { pattern, constants })
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
