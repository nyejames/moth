use super::*;
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralToken};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::sync::Arc;

fn canonical_tokens_for_all_tags() -> Arc<SourceTokens> {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let string_id = strings.intern("taxonomy fixture");
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_id = path_syntax
        .try_push_for_source(PathId::ROOT, source, LocalSpan::source_start())
        .expect("path fixture row should fit");
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    let mut numeric = Some(NumericLiteralToken::test_new("1.5", &mut strings));

    for tag in TokenTag::all() {
        let result = match tag.descriptor().payload() {
            TokenDescriptorPayload::Static => builder.push_static(*tag, LocalSpan::source_start()),
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => {
                builder.push_symbol(*tag, string_id, LocalSpan::source_start())
            }
            TokenDescriptorPayload::Path => {
                builder.push_path(*tag, path_id, LocalSpan::source_start())
            }
            TokenDescriptorPayload::NumericLiteral => builder.push_numeric(
                numeric
                    .take()
                    .expect("taxonomy schema has one numeric payload tag"),
                LocalSpan::source_start(),
            ),
            TokenDescriptorPayload::CharLiteral => {
                builder.push_char(*tag, 'x', LocalSpan::source_start())
            }
            TokenDescriptorPayload::BoolLiteral => {
                builder.push_bool(*tag, true, LocalSpan::source_start())
            }
        };
        result.expect("canonical taxonomy token fixture should build");
    }

    builder
        .finish()
        .expect("canonical taxonomy token fixture should publish")
}
fn canonical_token_for_tag(tokens: &SourceTokens, tag: TokenTag) -> TokenRef<'_> {
    let index = TokenTag::all()
        .iter()
        .position(|candidate| *candidate == tag)
        .expect("canonical taxonomy tag should have a fixture index");
    tokens
        .token(TokenIndex::try_from_index(index).expect("taxonomy token index should fit"))
        .expect("canonical taxonomy token should resolve")
}

#[test]
fn schema_has_all_explicit_tags_once() {
    let tags = TokenTag::all();
    assert_eq!(tags.len(), 94);
    let tokens = canonical_tokens_for_all_tags();

    for (index, tag) in tags.iter().enumerate() {
        assert_eq!(tag.raw(), (index + 1) as u16);
        let schema = tag.schema().expect("schema row");
        assert_eq!(schema.tag(), *tag);
        assert_eq!(schema.descriptor().text(), tag.descriptor().text());
        assert_eq!(schema.allowed_flags(), tag.allowed_flags());
        assert_eq!(TokenTag::from_raw(tag.raw()), Some(*tag));

        let token = tokens
            .token(TokenIndex::try_from_index(index).expect("taxonomy token index should fit"))
            .expect("canonical taxonomy token should resolve");
        assert_eq!(token.tag(), *tag);
        assert_eq!(token.span(), LocalSpan::source_start());
        token
            .validate_payload()
            .expect("canonical taxonomy payload should validate");
        match tag.descriptor().payload() {
            TokenDescriptorPayload::Static => {
                assert_eq!(token.shape().flags(), 0);
                assert_eq!(token.shape().data(), 0);
            }
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => {
                assert_eq!(token.shape().flags(), 0);
                assert_eq!(token.string_id(), Some(StringId::from_index(0)));
            }
            TokenDescriptorPayload::Path => {
                assert_eq!(token.shape().flags(), 0);
                assert!(token.path_syntax().unwrap().is_some());
            }
            TokenDescriptorPayload::NumericLiteral => {
                assert_eq!(token.shape().flags(), 1);
                assert!(token.numeric_literal().unwrap().is_some());
                assert_eq!(token.shape().numeric_literal_id().unwrap().raw(), 1);
            }
            TokenDescriptorPayload::CharLiteral => {
                assert_eq!(token.char_value(), Some('x'));
            }
            TokenDescriptorPayload::BoolLiteral => {
                assert_eq!(token.bool_value(), Some(true));
            }
        }
    }
    assert_eq!(
        tags.iter()
            .filter(|tag| **tag != TokenTag::NUMERIC_LITERAL)
            .filter(|tag| tag.allowed_flags() != 0)
            .count(),
        0
    );
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
    let tokens = canonical_tokens_for_all_tags();

    let symbol_shape = canonical_token_for_tag(&tokens, TokenTag::SYMBOL).shape();
    assert!(symbol_shape.string_id().is_some());
    assert_eq!(symbol_shape.path_syntax_id(), None);
    assert_eq!(symbol_shape.numeric_literal_id(), None);
    assert_eq!(symbol_shape.bool_value_checked(), None);
    assert_eq!(symbol_shape.char_value_checked(), None);

    let path_shape = canonical_token_for_tag(&tokens, TokenTag::PATH).shape();
    assert!(path_shape.path_syntax_id().is_some());
    assert_eq!(path_shape.string_id(), None);
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::PATH.raw(), 0, 0),
        None,
        "absent path marker must not decode"
    );
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::PATH.raw(), 1, path_shape.data()),
        None,
        "path shapes carry no flags"
    );

    let numeric_shape = canonical_token_for_tag(&tokens, TokenTag::NUMERIC_LITERAL).shape();
    let numeric_id = numeric_shape
        .numeric_literal_id()
        .expect("canonical numeric shape should carry a handle");
    assert_eq!(numeric_id.raw(), 1);
    assert_eq!(
        numeric_shape.numeric_kind(),
        Some(NumericLiteralKind::DecimalPoint)
    );
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

    let bool_shape = canonical_token_for_tag(&tokens, TokenTag::BOOL_LITERAL).shape();
    assert_eq!(bool_shape.bool_value_checked(), Some(true));
    assert_eq!(
        TokenShape::from_raw_parts(TokenTag::BOOL_LITERAL.raw(), 0, 2),
        None,
        "bool payloads are only 0 or 1"
    );

    let char_shape = canonical_token_for_tag(&tokens, TokenTag::CHAR_LITERAL).shape();
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
    for tag in [
        TokenTag::ASSIGN,
        TokenTag::ADD_ASSIGN,
        TokenTag::SUBTRACT_ASSIGN,
        TokenTag::MULTIPLY_ASSIGN,
        TokenTag::DIVIDE_ASSIGN,
        TokenTag::MODULUS_ASSIGN,
        TokenTag::EXPONENT_ASSIGN,
        TokenTag::INT_DIVIDE_ASSIGN,
    ] {
        assert!(tag.is_assignment_operator());
        assert!(tag.continues_expression());
    }
    assert!(!TokenTag::EXPONENT.continues_expression());
    // The remaining newline-tolerant tokens keep the exact prior `continues_expression`
    // set; four dropped flags here already regressed multi-line expressions once.
    for tag in [
        TokenTag::COLON,
        TokenTag::OPEN_PARENTHESIS,
        TokenTag::TYPE_PARAMETER_BRACKET,
        TokenTag::COMMA,
        TokenTag::END,
        TokenTag::ADD,
        TokenTag::SUBTRACT,
        TokenTag::MULTIPLY,
        TokenTag::DIVIDE,
        TokenTag::MODULUS,
        TokenTag::INT_DIVIDE,
        TokenTag::ARROW,
        TokenTag::IS,
        TokenTag::LESS_THAN,
        TokenTag::LESS_THAN_OR_EQUAL,
        TokenTag::GREATER_THAN,
        TokenTag::GREATER_THAN_OR_EQUAL,
    ] {
        assert!(tag.continues_expression());
    }
    assert!(!TokenTag::SYMBOL.continues_expression());
    for tag in [
        TokenTag::SYMBOL,
        TokenTag::THIS,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::STRING_SLICE_LITERAL,
        TokenTag::RAW_STRING_LITERAL,
        TokenTag::CHAR_LITERAL,
        TokenTag::BOOL_LITERAL,
        TokenTag::NONE_LITERAL,
        TokenTag::CLOSE_PARENTHESIS,
        TokenTag::CLOSE_CURLY,
        TokenTag::TEMPLATE_CLOSE,
        TokenTag::BANG,
        TokenTag::QUESTION_MARK,
    ] {
        assert!(tag.can_end_expression());
    }

    for tag in [
        TokenTag::PATH,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::STRING_SLICE_LITERAL,
        TokenTag::BOOL_LITERAL,
        TokenTag::CHAR_LITERAL,
        TokenTag::NONE_LITERAL,
        TokenTag::SYMBOL,
        TokenTag::THIS,
        TokenTag::MUTABLE,
        TokenTag::OPEN_PARENTHESIS,
        TokenTag::OPEN_CURLY,
        TokenTag::COPY,
    ] {
        assert!(tag.is_operand_start());
    }
    assert!(!TokenTag::RAW_STRING_LITERAL.is_operand_start());

    assert!(TokenTag::RETURN_BANG.is_keyword());
    assert!(TokenTag::CAST_BANG.is_keyword());
    assert!(TokenTag::NOT.is_word_operator());
    assert!(!TokenTag::NOT.is_keyword());
    assert!(TokenTag::NONE_LITERAL.is_literal());
    assert!(TokenTag::DATATYPE_INT.is_builtin_type());
    assert!(!TokenTag::DATATYPE_INT.is_literal());
    assert!(TokenTag::OPEN_PARENTHESIS.is_delimiter());
    assert!(TokenTag::FAT_ARROW.is_delimiter());
    assert!(!TokenTag::ADD.is_delimiter());

    assert_eq!(TokenTag::NEGATIVE.precedence(), Some(6));
    assert_eq!(TokenTag::NOT.precedence(), Some(6));
    assert_eq!(TokenTag::EXCLUSIVE_RANGE.precedence(), Some(6));
    assert_eq!(TokenTag::EXPONENT.precedence(), Some(5));
    assert_eq!(TokenTag::MULTIPLY.precedence(), Some(4));
    assert_eq!(TokenTag::ADD.precedence(), Some(3));
    assert_eq!(TokenTag::IS.precedence(), Some(2));
    assert_eq!(TokenTag::AND.precedence(), Some(1));
    assert_eq!(TokenTag::OR.precedence(), Some(0));
    assert_eq!(TokenTag::ASSIGN.precedence(), None);
}

#[test]
fn stats_classification_uses_schema_authority() {
    use crate::compiler_frontend::arena::TokenStats;

    // Every tag funnels through the schema-owned stats facts, so representative and
    // boundary tags share one decision with no hand-maintained operator table.
    for tag in TokenTag::all() {
        let mut from_tag = TokenStats::default();
        from_tag.accumulate_tag(*tag);
        assert_eq!(from_tag.total_tokens, 1);

        let mut from_shape = TokenStats::default();
        // `TokenShape::new` rejects packed payload handles, so only static tags round-trip
        // here; payload tags assert through their canonical token fixture above.
        if let Some(shape) = TokenShape::new(*tag, 0, 0) {
            from_shape.accumulate_shape(shape);
            assert_eq!(from_shape, from_tag);
        }
    }

    for tag in [
        TokenTag::ADD,
        TokenTag::SUBTRACT,
        TokenTag::MULTIPLY,
        TokenTag::DIVIDE,
        TokenTag::MODULUS,
        TokenTag::INT_DIVIDE,
        TokenTag::EXPONENT,
        TokenTag::NEGATIVE,
        TokenTag::ADD_ASSIGN,
        TokenTag::SUBTRACT_ASSIGN,
        TokenTag::MULTIPLY_ASSIGN,
        TokenTag::DIVIDE_ASSIGN,
        TokenTag::MODULUS_ASSIGN,
        TokenTag::EXPONENT_ASSIGN,
        TokenTag::INT_DIVIDE_ASSIGN,
        TokenTag::LESS_THAN,
        TokenTag::LESS_THAN_OR_EQUAL,
        TokenTag::GREATER_THAN,
        TokenTag::GREATER_THAN_OR_EQUAL,
        TokenTag::IS,
        TokenTag::AND,
        TokenTag::OR,
        TokenTag::NOT,
        TokenTag::BANG,
        TokenTag::QUESTION_MARK,
        TokenTag::COPY,
        TokenTag::CHANNEL_SEND,
        TokenTag::CHANNEL_RECEIVE,
        TokenTag::AMPERSAND,
        TokenTag::ARROW,
        TokenTag::FAT_ARROW,
    ] {
        assert!(tag.is_stats_operator(), "{tag:?} stays an operator");
        let mut stats = TokenStats::default();
        stats.accumulate_tag(tag);
        assert_eq!(stats.operators, 1, "{tag:?} fills the operator bucket");
    }

    for tag in [
        TokenTag::STRING_SLICE_LITERAL,
        TokenTag::RAW_STRING_LITERAL,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::CHAR_LITERAL,
        TokenTag::BOOL_LITERAL,
        TokenTag::NONE_LITERAL,
    ] {
        assert!(tag.is_stats_literal(), "{tag:?} stays a literal");
    }
    // Raw strings stay literal-adjacent in the expression taxonomy only by exclusion from
    // `is_operand_start`; the stats bucket keeps the literal count.
    assert!(!TokenTag::RAW_STRING_LITERAL.is_operand_start());
    let mut raw_stats = TokenStats::default();
    raw_stats.accumulate_tag(TokenTag::RAW_STRING_LITERAL);
    assert_eq!(raw_stats.literals, 1);
    // Delimiter-adjacent `FatArrow` counts as an operator (not a delimiter bucket), while the
    // collection delimiters keep their own bucket.
    assert!(TokenTag::FAT_ARROW.is_stats_operator());
    assert!(TokenTag::FAT_ARROW.is_delimiter());
    for tag in [TokenTag::OPEN_CURLY, TokenTag::CLOSE_CURLY, TokenTag::COMMA] {
        assert!(!tag.is_stats_operator());
        let mut stats = TokenStats::default();
        stats.accumulate_tag(tag);
        assert_eq!(stats.map_or_collection_delimiters, 1);
    }

    // Keyword-adjacent operators (`copy`) and bang spellings keep their legacy buckets.
    assert!(TokenTag::COPY.is_keyword());
    assert!(TokenTag::COPY.is_stats_operator());
    for tag in [TokenTag::RETURN, TokenTag::RETURN_BANG] {
        let mut stats = TokenStats::default();
        stats.accumulate_tag(tag);
        assert_eq!(stats.return_tokens, 1);
    }
    for tag in [TokenTag::CAST, TokenTag::CAST_BANG] {
        let mut stats = TokenStats::default();
        stats.accumulate_tag(tag);
        assert_eq!(stats.cast_tokens, 1);
    }
}
