use super::*;
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralToken};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

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
fn shape_payload_decoding_validates_packed_handles_and_scalars() {
    let mut strings = StringTable::new();
    let symbol = strings.intern("alpha");
    let path_id = PathSyntaxId::try_from_raw(3).expect("checked path handle");
    let numeric_id = crate::compiler_frontend::numeric_text::store::NumericLiteralId::try_from_raw(2)
        .expect("checked numeric handle");
    let numeric_token = NumericLiteralToken::test_new("1.5", &mut strings);

    let symbol_shape = TokenShape::from_token_kind(&TokenKind::Symbol(symbol))
        .expect("symbol shape should pack");
    assert_eq!(symbol_shape.string_id(), Some(symbol));
    assert_eq!(symbol_shape.path_syntax_id(), None);
    assert_eq!(symbol_shape.numeric_literal_id(), None);
    assert_eq!(symbol_shape.bool_value_checked(), None);
    assert_eq!(symbol_shape.char_value_checked(), None);

    let path_shape = TokenShape::from_token_kind(&TokenKind::Path(path_id))
        .expect("path shape should pack");
    assert_eq!(path_shape.path_syntax_id(), Some(path_id));
    assert_eq!(path_shape.string_id(), None);
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::PATH.raw(), 0, 0),
        None,
        "absent path marker must not decode"
    );
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::PATH.raw(), 1, path_id.raw()),
        None,
        "path shapes carry no flags"
    );
    let numeric_shape =
        TokenShape::from_token_kind_with_numeric_id(&TokenKind::NumericLiteral(numeric_token.clone()), numeric_id)
            .expect("numeric shape should pack");
    assert_eq!(numeric_shape.numeric_literal_id(), Some(numeric_id));
    assert_eq!(numeric_shape.numeric_kind(), Some(NumericLiteralKind::DecimalPoint));
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::NUMERIC_LITERAL.raw(), 0, 0),
        None,
        "absent numeric marker must not decode"
    );
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::NUMERIC_LITERAL.raw(), 3, numeric_id.raw()),
        None,
        "numeric kind flags above two must not decode"
    );
    assert_eq!(
        TokenShape::from_token_kind_with_numeric_id(
            &TokenKind::NumericLiteral(numeric_token.clone()),
            crate::compiler_frontend::numeric_text::store::NumericLiteralId::NONE,
        ),
        None,
        "absent numeric handle must not pack"
    );
    assert_eq!(
        TokenShape::from_token_kind(&TokenKind::Path(PathSyntaxId::NONE)),
        None,
        "absent path handle must not pack"
    );
    let bool_shape = TokenShape::from_token_kind(&TokenKind::BoolLiteral(true))
        .expect("bool shape should pack");
    assert_eq!(bool_shape.bool_value_checked(), Some(true));
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::BOOL_LITERAL.raw(), 0, 2),
        None,
        "bool payloads are only 0 or 1"
    );

    let char_shape = TokenShape::from_token_kind(&TokenKind::CharLiteral('x'))
        .expect("char shape should pack");
    assert_eq!(char_shape.char_value_checked(), Some('x'));
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::CHAR_LITERAL.raw(), 0, 0xD800),
        None,
        "surrogate scalars must not decode"
    );
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::MODULE_START.raw(), 0, 1),
        None,
        "static shapes carry no payload"
    );
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

#[test]
fn stats_classification_uses_schema_authority_with_legacy_parity() {
    use crate::compiler_frontend::arena::TokenStats;

    // Every tag funnels through the schema-owned stats facts, so representative and
    // boundary tags share one decision with no hand-maintained operator table.
    for tag in TokenTag::all() {
        let mut from_tag = TokenStats::default();
        from_tag.accumulate_tag(*tag);
        assert_eq!(from_tag.total_tokens, 1);

        let mut from_shape = TokenStats::default();
        // `TokenShape::new` rejects packed payload handles, so only static tags round-trip
        // here; payload tags assert through their tag path above plus the kind parity below.
        if let Some(shape) = TokenShape::new(*tag, 0, 0) {
            from_shape.accumulate_shape(shape);
            assert_eq!(from_shape, from_tag);
        }
    }

    // Legacy operator/literal sets keep their exact bucket semantics through tags.
    for kind in [
        TokenKind::Add,
        TokenKind::Subtract,
        TokenKind::Multiply,
        TokenKind::Divide,
        TokenKind::Modulus,
        TokenKind::IntDivide,
        TokenKind::Exponent,
        TokenKind::Negative,
        TokenKind::AddAssign,
        TokenKind::SubtractAssign,
        TokenKind::MultiplyAssign,
        TokenKind::DivideAssign,
        TokenKind::ModulusAssign,
        TokenKind::ExponentAssign,
        TokenKind::IntDivideAssign,
        TokenKind::LessThan,
        TokenKind::LessThanOrEqual,
        TokenKind::GreaterThan,
        TokenKind::GreaterThanOrEqual,
        TokenKind::Is,
        TokenKind::And,
        TokenKind::Or,
        TokenKind::Not,
        TokenKind::Bang,
        TokenKind::QuestionMark,
        TokenKind::Copy,
        TokenKind::ChannelSend,
        TokenKind::ChannelReceive,
        TokenKind::Ampersand,
        TokenKind::Arrow,
        TokenKind::FatArrow,
    ] {
        assert!(kind.token_tag().is_stats_operator(), "{kind:?} stays an operator");
        let mut stats = TokenStats::default();
        stats.accumulate_tag(kind.token_tag());
        assert_eq!(stats.operators, 1, "{kind:?} fills the operator bucket");
    }

    for kind in [
        TokenKind::StringSliceLiteral(StringId::from_index(0)),
        TokenKind::RawStringLiteral(StringId::from_index(0)),
        TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut Default::default())),
        TokenKind::CharLiteral('x'),
        TokenKind::BoolLiteral(true),
        TokenKind::NoneLiteral,
    ] {
        assert!(kind.token_tag().is_stats_literal(), "{kind:?} stays a literal");
    }
    // Raw strings stay literal-adjacent in the expression taxonomy only by exclusion from
    // `is_operand_start`; the stats bucket keeps the legacy literal count.
    assert!(!TokenKind::RawStringLiteral(StringId::from_index(0)).is_operand_start());
    let mut raw_stats = TokenStats::default();
    raw_stats.accumulate_tag(TokenKind::RawStringLiteral(StringId::from_index(0)).token_tag());
    assert_eq!(raw_stats.literals, 1);
    // Delimiter-adjacent `FatArrow` counts as an operator (not a delimiter bucket), while the
    // collection delimiters keep their own bucket.
    assert!(TokenKind::FatArrow.token_tag().is_stats_operator());
    assert!(TokenKind::FatArrow.token_tag().is_delimiter());
    for kind in [TokenKind::OpenCurly, TokenKind::CloseCurly, TokenKind::Comma] {
        assert!(!kind.token_tag().is_stats_operator());
        let mut stats = TokenStats::default();
        stats.accumulate_tag(kind.token_tag());
        assert_eq!(stats.map_or_collection_delimiters, 1);
    }

    // Keyword-adjacent operators (`copy`) and bang spellings keep their legacy buckets.
    assert!(TokenKind::Copy.token_tag().is_keyword());
    assert!(TokenKind::Copy.token_tag().is_stats_operator());
    for kind in [TokenKind::Return, TokenKind::ReturnBang] {
        let mut stats = TokenStats::default();
        stats.accumulate_tag(kind.token_tag());
        assert_eq!(stats.return_tokens, 1);
    }
    for kind in [TokenKind::Cast, TokenKind::CastBang] {
        let mut stats = TokenStats::default();
        stats.accumulate_tag(kind.token_tag());
        assert_eq!(stats.cast_tokens, 1);
    }
}
