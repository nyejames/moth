use super::*;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::symbols::string_interning::StringId;

fn all_token_kinds() -> Vec<TokenKind> {
    vec![
        TokenKind::ModuleStart,
        TokenKind::Eof,
        TokenKind::Export,
        TokenKind::Hash,
        TokenKind::Reactive,
        TokenKind::Arrow,
        TokenKind::Symbol(StringId::from_index(0)),
        TokenKind::StyleDirective(StringId::from_index(0)),
        TokenKind::StringSliceLiteral(StringId::from_index(0)),
        TokenKind::Path(PathSyntaxId::NONE),
        TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut Default::default())),
        TokenKind::CharLiteral('x'),
        TokenKind::RawStringLiteral(StringId::from_index(0)),
        TokenKind::BoolLiteral(true),
        TokenKind::OpenCurly,
        TokenKind::CloseCurly,
        TokenKind::TypeParameterBracket,
        TokenKind::Newline,
        TokenKind::End,
        TokenKind::StartTemplateBody,
        TokenKind::Comma,
        TokenKind::Dot,
        TokenKind::Colon,
        TokenKind::DoubleColon,
        TokenKind::Assign,
        TokenKind::This,
        TokenKind::Must,
        TokenKind::TraitThis,
        TokenKind::OpenParenthesis,
        TokenKind::CloseParenthesis,
        TokenKind::As,
        TokenKind::Type,
        TokenKind::Of,
        TokenKind::Variadic,
        TokenKind::Mutable,
        TokenKind::DatatypeNone,
        TokenKind::NoneLiteral,
        TokenKind::DatatypeInt,
        TokenKind::DatatypeFloat,
        TokenKind::DatatypeBool,
        TokenKind::DatatypeTrue,
        TokenKind::DatatypeFalse,
        TokenKind::DatatypeString,
        TokenKind::DatatypeChar,
        TokenKind::Bang,
        TokenKind::QuestionMark,
        TokenKind::Negative,
        TokenKind::Exponent,
        TokenKind::Multiply,
        TokenKind::Divide,
        TokenKind::Modulus,
        TokenKind::IntDivide,
        TokenKind::ExponentAssign,
        TokenKind::MultiplyAssign,
        TokenKind::DivideAssign,
        TokenKind::ModulusAssign,
        TokenKind::IntDivideAssign,
        TokenKind::Add,
        TokenKind::Subtract,
        TokenKind::AddAssign,
        TokenKind::SubtractAssign,
        TokenKind::Not,
        TokenKind::Is,
        TokenKind::LessThan,
        TokenKind::LessThanOrEqual,
        TokenKind::GreaterThan,
        TokenKind::GreaterThanOrEqual,
        TokenKind::And,
        TokenKind::Or,
        TokenKind::If,
        TokenKind::Else,
        TokenKind::Return,
        TokenKind::ReturnBang,
        TokenKind::Catch,
        TokenKind::Then,
        TokenKind::Checked,
        TokenKind::Async,
        TokenKind::Cast,
        TokenKind::CastBang,
        TokenKind::Assert,
        TokenKind::Loop,
        TokenKind::By,
        TokenKind::Break,
        TokenKind::Continue,
        TokenKind::ExclusiveRange,
        TokenKind::Ampersand,
        TokenKind::FatArrow,
        TokenKind::Wildcard,
        TokenKind::Copy,
        TokenKind::TemplateClose,
        TokenKind::TemplateHead,
        TokenKind::ChannelSend,
        TokenKind::ChannelReceive,
        TokenKind::Yield,
    ]
}

#[test]
fn schema_has_all_explicit_tags_once() {
    let tags = TokenTag::all();
    assert_eq!(tags.len(), 94);
    for (index, tag) in tags.iter().enumerate() {
        assert_eq!(tag.raw(), (index + 1) as u16);
        let schema = tag.schema().expect("schema row");
        assert_eq!(schema.tag(), *tag);
        assert_eq!(schema.descriptor().text(), tag.descriptor().text());
        assert_eq!(schema.allowed_flags(), tag.allowed_flags());
        assert_eq!(TokenTag::from_raw(tag.raw()), Some(*tag));
    }
    assert_eq!(
        tags.iter()
            .filter(|tag| **tag != TokenTag::NUMERIC_LITERAL)
            .filter(|tag| tag.allowed_flags() != 0)
            .count(),
        0
    );
    let kinds = all_token_kinds();
    assert_eq!(kinds.len(), tags.len());
    for (index, kind) in kinds.iter().enumerate() {
        assert_eq!(kind.token_tag().raw(), (index + 1) as u16);
    }
    assert_eq!(
        TokenTag::from_raw_unchecked(95).descriptor().text(),
        "token"
    );
}

#[test]
fn unknown_tags_and_reserved_flags_are_rejected() {
    assert_eq!(TokenTag::from_raw(0), None);
    assert_eq!(TokenTag::from_raw(95), None);
    assert_eq!(TokenShape::from_raw_parts(95, 0, 0), None);

    assert_eq!(TokenTag::NUMERIC_LITERAL.allowed_flags(), 0b11);
    assert!(TokenTag::NUMERIC_LITERAL.flags_are_valid(0b11));
    assert!(!TokenTag::NUMERIC_LITERAL.flags_are_valid(0b100));
    assert_eq!(TokenTag::SYMBOL.allowed_flags(), 0);
    assert!(!TokenTag::SYMBOL.flags_are_valid(1));

    let shape = TokenShape::new(TokenTag::NUMERIC_LITERAL, 0b11, 7).expect("valid shape");
    assert_eq!(shape.tag(), TokenTag::NUMERIC_LITERAL);
    assert_eq!(shape.flags(), 0b11);
    assert_eq!(shape.data(), 7);
    assert!(TokenShape::new(TokenTag::NUMERIC_LITERAL, 0b100, 7).is_none());
    assert!(TokenShape::new(TokenTag::SYMBOL, 1, 7).is_none());
}

#[test]
fn schema_classifications_match_frontend_semantics() {
    for kind in [
        TokenKind::Assign,
        TokenKind::AddAssign,
        TokenKind::SubtractAssign,
        TokenKind::MultiplyAssign,
        TokenKind::DivideAssign,
        TokenKind::ModulusAssign,
        TokenKind::ExponentAssign,
        TokenKind::IntDivideAssign,
    ] {
        assert!(kind.is_assignment_operator());
        assert!(kind.continues_expression());
    }
    assert!(!TokenKind::Exponent.continues_expression());
    // The remaining newline-tolerant tokens keep the exact prior `continues_expression`
    // set; four dropped flags here already regressed multi-line expressions once.
    for kind in [
        TokenKind::Colon,
        TokenKind::OpenParenthesis,
        TokenKind::TypeParameterBracket,
        TokenKind::Comma,
        TokenKind::End,
        TokenKind::Add,
        TokenKind::Subtract,
        TokenKind::Multiply,
        TokenKind::Divide,
        TokenKind::Modulus,
        TokenKind::IntDivide,
        TokenKind::Arrow,
        TokenKind::Is,
        TokenKind::LessThan,
        TokenKind::LessThanOrEqual,
        TokenKind::GreaterThan,
        TokenKind::GreaterThanOrEqual,
    ] {
        assert!(kind.continues_expression());
    }
    assert!(!TokenKind::Symbol(StringId::from_index(0)).continues_expression());
    for kind in [
        TokenKind::Symbol(StringId::from_index(0)),
        TokenKind::This,
        TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut Default::default())),
        TokenKind::StringSliceLiteral(StringId::from_index(0)),
        TokenKind::RawStringLiteral(StringId::from_index(0)),
        TokenKind::CharLiteral('x'),
        TokenKind::BoolLiteral(true),
        TokenKind::NoneLiteral,
        TokenKind::CloseParenthesis,
        TokenKind::CloseCurly,
        TokenKind::TemplateClose,
        TokenKind::Bang,
        TokenKind::QuestionMark,
    ] {
        assert!(kind.can_end_expression());
    }

    for kind in [
        TokenKind::Path(PathSyntaxId::NONE),
        TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut Default::default())),
        TokenKind::StringSliceLiteral(StringId::from_index(0)),
        TokenKind::BoolLiteral(true),
        TokenKind::CharLiteral('x'),
        TokenKind::NoneLiteral,
        TokenKind::Symbol(StringId::from_index(0)),
        TokenKind::This,
        TokenKind::Mutable,
        TokenKind::OpenParenthesis,
        TokenKind::OpenCurly,
        TokenKind::Copy,
    ] {
        assert!(kind.is_operand_start());
    }
    assert!(!TokenKind::RawStringLiteral(StringId::from_index(0)).is_operand_start());

    assert!(TokenKind::ReturnBang.is_keyword());
    assert!(TokenKind::CastBang.is_keyword());
    assert!(TokenKind::Not.is_word_operator());
    assert!(!TokenKind::Not.is_keyword());
    assert!(TokenKind::NoneLiteral.is_literal());
    assert!(TokenKind::DatatypeInt.is_builtin_type());
    assert!(!TokenKind::DatatypeInt.is_literal());
    assert!(TokenKind::OpenParenthesis.is_delimiter());
    assert!(TokenKind::FatArrow.is_delimiter());
    assert!(!TokenKind::Add.is_delimiter());

    assert_eq!(TokenKind::Negative.precedence(), Some(6));
    assert_eq!(TokenKind::Not.precedence(), Some(6));
    assert_eq!(TokenKind::ExclusiveRange.precedence(), Some(6));
    assert_eq!(TokenKind::Exponent.precedence(), Some(5));
    assert_eq!(TokenKind::Multiply.precedence(), Some(4));
    assert_eq!(TokenKind::Add.precedence(), Some(3));
    assert_eq!(TokenKind::Is.precedence(), Some(2));
    assert_eq!(TokenKind::And.precedence(), Some(1));
    assert_eq!(TokenKind::Or.precedence(), Some(0));
    assert_eq!(TokenKind::Assign.precedence(), None);
}
