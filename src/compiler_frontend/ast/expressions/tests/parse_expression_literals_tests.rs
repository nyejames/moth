//! Literal expression parsing regression tests.
//!
//! WHAT: validates parsing of int, float, string, char, bool, and template literals plus
//!       malformed-literal diagnostics.
//! WHY: literals are the simplest expressions but span many token kinds; targeted coverage
//!      prevents silent changes to literal type inference.

use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::grammar::{NumericExponentSign, NumericLiteralKind, NumericLiteralSign};
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
use std::rc::Rc;
use std::sync::Arc;

#[test]
fn parse_literal_expression_defers_signed_i32_min_token() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Negative,
        "2147483648",
        10,
        false,
        NumericProfile::STANDARD,
    )
    .expect("signed i32 minimum boundary should parse");

    assert!(!outcome.next_number_negative);
    assert_eq!(outcome.expression.len(), 1);

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Negative);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_defers_positive_i32_overflow() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Positive,
        "2147483648",
        10,
        false,
        NumericProfile::STANDARD,
    )
    .expect("over-range literal should defer range checks to evaluation");

    assert_eq!(outcome.expression.len(), 1);

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Positive);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_folds_effective_negative_sign_into_pending_token() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Positive,
        "2147483648",
        10,
        true,
        NumericProfile::STANDARD,
    )
    .expect("parser-owned negation should fold into the pending sign");

    assert!(!outcome.next_number_negative);
    assert_eq!(outcome.expression.len(), 1);

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Negative);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_defers_effective_negative_underflow() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Positive,
        "2147483649",
        10,
        true,
        NumericProfile::STANDARD,
    )
    .expect("under-range literal should defer range checks to evaluation");

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Negative);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_defers_int32_overflow_at_either_width() {
    for profile in [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ] {
        let outcome = parse_whole_number_token(
            NumericLiteralSign::Positive,
            "3000000000",
            10,
            false,
            profile,
        )
        .expect("range checks belong to evaluation, not the literal parser");

        assert_eq!(outcome.expression.len(), 1);

        let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
            panic!("literal should defer to a pending item");
        };
        assert_eq!(token.sign, NumericLiteralSign::Positive);
        assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
    }
}
#[test]
fn parse_literal_expression_folds_effective_negative_sign_for_i64_min() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Positive,
        "9223372036854775808",
        19,
        true,
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    )
    .expect("parser-owned negation should fold into the pending sign");

    assert!(!outcome.next_number_negative);

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Negative);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_defers_positive_i64_overflow() {
    let outcome = parse_whole_number_token(
        NumericLiteralSign::Positive,
        "9223372036854775808",
        19,
        false,
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    )
    .expect("positive 2^63 should defer range checks to evaluation");

    let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
        panic!("literal should defer to a pending item");
    };
    assert_eq!(token.sign, NumericLiteralSign::Positive);
    assert_eq!(token.kind, NumericLiteralKind::WholeNumber);
}

#[test]
fn parse_literal_expression_defers_float_pending_item() {
    for precision in [FloatPrecision::Bits32, FloatPrecision::Bits64] {
        let outcome = parse_float_token(
            "0.1",
            2,
            1,
            NumericProfile {
                int_width: IntWidth::Bits32,
                float_precision: precision,
            },
        )
        .expect("float precision applies at evaluation, not the literal parser");

        let ExpressionRpnItem::PendingNumericLiteral { token, .. } = &outcome.expression[0] else {
            panic!("literal should defer to a pending item");
        };
        assert_eq!(token.sign, NumericLiteralSign::Positive);
        assert_eq!(token.kind, NumericLiteralKind::DecimalPoint);
    }
}

#[derive(Debug)]
struct LiteralParseOutcome {
    expression: Vec<ExpressionRpnItem>,
    next_number_negative: bool,
}

fn parse_whole_number_token(
    sign: NumericLiteralSign,
    normalized_text: &str,
    digit_count: u32,
    next_number_negative: bool,
    profile: NumericProfile,
) -> Result<LiteralParseOutcome, ExpressionParseError> {
    parse_numeric_token(
        sign,
        NumericLiteralKind::WholeNumber,
        normalized_text,
        digit_count,
        0,
        next_number_negative,
        profile,
    )
}

fn parse_float_token(
    normalized_text: &str,
    digit_count: u32,
    fractional_digit_count: u32,
    profile: NumericProfile,
) -> Result<LiteralParseOutcome, ExpressionParseError> {
    parse_numeric_token(
        NumericLiteralSign::Positive,
        NumericLiteralKind::DecimalPoint,
        normalized_text,
        digit_count,
        fractional_digit_count,
        false,
        profile,
    )
}

/// Parse one numeric token fixture while the profile under test reaches the parser
/// only through the scope context.
fn parse_numeric_token(
    sign: NumericLiteralSign,
    kind: NumericLiteralKind,
    normalized_text: &str,
    digit_count: u32,
    fractional_digit_count: u32,
    next_number_negative: bool,
    profile: NumericProfile,
) -> Result<LiteralParseOutcome, ExpressionParseError> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_numeric_profile(profile);

    let min_text = string_table.intern(normalized_text);
    // For signed tokens the source_text includes the sign prefix.
    let source = match sign {
        NumericLiteralSign::Positive => normalized_text.to_owned(),
        NumericLiteralSign::Negative => format!("-{normalized_text}"),
    };
    let source_text = string_table.intern(&source);
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_numeric(
            NumericLiteralToken::new(
                sign,
                source_text,
                min_text,
                kind,
                digit_count,
                fractional_digit_count,
                0,
                NumericExponentSign::None,
            ),
            LocalSpan::source_start(),
        )
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("canonical fixture tokens should build");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut expression = Vec::new();
    let mut next_number_negative = next_number_negative;
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let expected_type = ExpectedType::Infer;
    let value_mode = ValueMode::ImmutableOwned;

    {
        let mut literal_state = LiteralParseState {
            expected_type: &expected_type,
            value_mode: &value_mode,
            expression: &mut expression,
            next_number_negative: &mut next_number_negative,
            allow_boundary_catch: true,
        };

        parse_literal_expression(
            &mut token_stream,
            &context,
            &mut type_interner,
            &mut literal_state,
            &mut string_table,
            &mut path_fork,
        )?;
    }

    Ok(LiteralParseOutcome {
        expression,
        next_number_negative,
    })
}
