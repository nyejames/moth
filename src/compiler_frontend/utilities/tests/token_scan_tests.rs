use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, Token, TokenCursor, TokenIndex, TokenKind, TokenRange, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, OpenConstruct, TokenFactView, TokenScanFailure,
    collect_declaration_initializer_range, collect_scanned_symbol_references,
    consume_balanced_template_region_from_source, has_top_level_comma_before_statement_end,
    innermost_open_construct,
};
use std::sync::Arc;

fn token(kind: TokenKind) -> Token {
    Token::new(kind, LocalSpan::source_start())
}

fn source_tokens_from_kinds(source: SourceId, kinds: Vec<TokenKind>) -> SourceTokens {
    let mut numeric_literals = NumericLiteralStore::with_source(source);
    for kind in &kinds {
        if let TokenKind::NumericLiteral(literal) = kind {
            numeric_literals.push(literal.clone());
        }
    }

    SourceTokens::try_from_tokens(
        source,
        kinds.into_iter().map(token).collect(),
        numeric_literals,
        PathSyntaxTable::with_source(source),
    )
    .expect("canonical token fixture should satisfy source-token invariants")
}

fn full_cursor(owner: &SourceTokens) -> TokenCursor<'_> {
    owner
        .cursor(owner.full_range().expect("test owner range should fit"))
        .expect("test cursor range should fit")
}

fn scan_initializer(
    owner: &SourceTokens,
    string_table: &mut StringTable,
) -> Result<(TokenRange, Vec<InitializerReference>), TokenScanFailure> {
    let mut cursor = full_cursor(owner);
    collect_declaration_initializer_range(&mut cursor, string_table)
}

fn tags_in_range(owner: &SourceTokens, range: TokenRange) -> Vec<TokenTag> {
    let mut cursor = owner.cursor(range).expect("checked range should fit");
    (0..range.len() as usize)
        .filter_map(|_| cursor.advance())
        .map(|token| token.tag())
        .collect()
}

fn boundary_tag(owner: &SourceTokens, range: TokenRange) -> Option<TokenTag> {
    owner.token(range.end()).ok().map(|token| token.tag())
}

fn symbol_ids_in_range(owner: &SourceTokens, range: TokenRange) -> Vec<crate::compiler_frontend::symbols::string_interning::StringId> {
    let mut cursor = owner.cursor(range).expect("checked range should fit");
    (0..range.len() as usize)
        .filter_map(|_| cursor.advance())
        .filter(|token| token.tag() == TokenTag::SYMBOL)
        .filter_map(|token| token.string_id())
        .collect()
}

fn ast_cursor(owner: &Arc<SourceTokens>) -> AstCursor<'_> {
    AstCursor::from_source_tokens(
        owner,
        None,
        owner.full_range().expect("test owner range should fit"),
    )
    .expect("canonical AST cursor should construct")
}

#[test]
fn canonical_range_view_rejects_foreign_source() {
    let owner = source_tokens_from_kinds(SourceId::COMPILATION_ROOT, vec![TokenKind::Eof]);
    let canonical = &owner;
    let foreign = TokenRange::new(
        SourceId::from_index(1),
        TokenIndex::try_from_raw(0).expect("zero index is representable"),
        TokenIndex::try_from_raw(1).expect("one index is representable"),
    )
    .expect("ordered foreign range should construct");

    assert!(matches!(
        TokenFactView::from_source_range(canonical, foreign),
        Err(crate::compiler_frontend::tokenizer::tokens::TokenRangeError::ForeignSource {
            expected,
            actual,
        }) if expected == canonical.source() && actual == foreign.source()
    ));
}

#[test]
fn canonical_range_view_rejects_out_of_owner_range() {
    let owner = source_tokens_from_kinds(SourceId::COMPILATION_ROOT, vec![TokenKind::Eof]);
    let out_of_owner = TokenRange::new(
        owner.source(),
        TokenIndex::try_from_raw(0).expect("zero index is representable"),
        TokenIndex::try_from_raw(2).expect("two index is representable"),
    )
    .expect("ordered range should construct");

    assert!(matches!(
        TokenFactView::from_source_range(&owner, out_of_owner),
        Err(crate::compiler_frontend::tokenizer::tokens::TokenRangeError::OutOfBounds { .. })
    ));
}

#[test]
fn top_level_comma_detection_ignores_nested_commas() {
    let source = SourceId::COMPILATION_ROOT;
    let mut string_table = StringTable::new();

    let nested_only = Arc::new(source_tokens_from_kinds(
        source,
        vec![
            TokenKind::OpenParenthesis,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut string_table)),
            TokenKind::Comma,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("2", &mut string_table)),
            TokenKind::CloseParenthesis,
            TokenKind::Newline,
            TokenKind::Eof,
        ],
    ));
    let nested_cursor = ast_cursor(&nested_only);
    assert!(!has_top_level_comma_before_statement_end(&nested_cursor));

    let top_level = Arc::new(source_tokens_from_kinds(
        source,
        vec![
            TokenKind::Symbol(string_table.intern("a")),
            TokenKind::Comma,
            TokenKind::Symbol(string_table.intern("b")),
            TokenKind::Assign,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut string_table)),
            TokenKind::Newline,
            TokenKind::Eof,
        ],
    ));
    let top_level_cursor = ast_cursor(&top_level);
    assert!(has_top_level_comma_before_statement_end(&top_level_cursor));

    let multiline = Arc::new(source_tokens_from_kinds(
        source,
        vec![
            TokenKind::Symbol(string_table.intern("a")),
            TokenKind::Comma,
            TokenKind::Newline,
            TokenKind::Symbol(string_table.intern("b")),
            TokenKind::Assign,
            TokenKind::Symbol(string_table.intern("pair")),
            TokenKind::Newline,
            TokenKind::Eof,
        ],
    ));
    let multiline_cursor = ast_cursor(&multiline);
    assert!(has_top_level_comma_before_statement_end(&multiline_cursor));
}

#[test]
fn canonical_comma_gate_respects_nested_call_depth() {
    let mut string_table = StringTable::new();
    let owner = Arc::new(source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(string_table.intern("call")),
            TokenKind::OpenParenthesis,
            TokenKind::Symbol(string_table.intern("a")),
            TokenKind::Comma,
            TokenKind::Symbol(string_table.intern("b")),
            TokenKind::CloseParenthesis,
            TokenKind::Comma,
            TokenKind::Symbol(string_table.intern("tail")),
            TokenKind::Eof,
        ],
    ));
    let cursor = ast_cursor(&owner);
    assert!(
        has_top_level_comma_before_statement_end(&cursor),
        "the gate must ignore the nested comma and see the top-level comma"
    );
}

#[test]
fn declaration_initializer_range_includes_terminal_catch_then_block() {
    let mut string_table = StringTable::new();
    let load_name = string_table.intern("load");
    let next_statement_name = string_table.intern("next_statement");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(load_name),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
            TokenKind::Catch,
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::Then,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("0", &mut string_table)),
            TokenKind::Newline,
            TokenKind::End,
            TokenKind::Newline,
            TokenKind::Symbol(next_statement_name),
            TokenKind::Eof,
        ],
    );

    let (range, _) = scan_initializer(&owner, &mut string_table)
        .expect("catch initializer should scan successfully");
    assert_eq!(
        tags_in_range(&owner, range),
        vec![
            TokenTag::SYMBOL,
            TokenTag::OPEN_PARENTHESIS,
            TokenTag::CLOSE_PARENTHESIS,
            TokenTag::CATCH,
            TokenTag::COLON,
            TokenTag::NEWLINE,
            TokenTag::THEN,
            TokenTag::NUMERIC_LITERAL,
            TokenTag::NEWLINE,
            TokenTag::END,
        ]
    );
    assert_eq!(boundary_tag(&owner, range), Some(TokenTag::NEWLINE));
}

#[test]
fn declaration_initializer_range_balances_nested_blocks_inside_catch() {
    let mut string_table = StringTable::new();
    let load_name = string_table.intern("load");
    let flag_name = string_table.intern("flag");
    let io_name = string_table.intern("io");
    let next_statement_name = string_table.intern("next_statement");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(load_name),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
            TokenKind::Catch,
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::If,
            TokenKind::Symbol(flag_name),
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::Symbol(io_name),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
            TokenKind::Newline,
            TokenKind::End,
            TokenKind::Newline,
            TokenKind::Then,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("0", &mut string_table)),
            TokenKind::Newline,
            TokenKind::End,
            TokenKind::Newline,
            TokenKind::Symbol(next_statement_name),
            TokenKind::Eof,
        ],
    );

    let (range, _) = scan_initializer(&owner, &mut string_table)
        .expect("nested catch initializer should scan successfully");
    let tags = tags_in_range(&owner, range);
    assert!(tags.contains(&TokenTag::THEN));
    assert_eq!(
        tags.iter().filter(|tag| **tag == TokenTag::END).count(),
        2,
        "both nested and outer terminators should be collected"
    );
    assert_eq!(boundary_tag(&owner, range), Some(TokenTag::NEWLINE));
}

#[test]
fn balanced_template_region_consumes_nested_templates_from_source() {
    let mut string_table = StringTable::new();
    let text_outer = string_table.intern("outer");
    let text_inner = string_table.intern("inner");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::TemplateHead,
            TokenKind::StringSliceLiteral(text_outer),
            TokenKind::TemplateHead,
            TokenKind::StringSliceLiteral(text_inner),
            TokenKind::TemplateClose,
            TokenKind::TemplateClose,
            TokenKind::Eof,
        ],
    );

    let mut cursor = full_cursor(&owner);
    let opening = cursor.advance().expect("opening token should be present");
    let mut consumed = Vec::new();
    let end = consume_balanced_template_region_from_source(
        &mut cursor,
        opening,
        |token| consumed.push(token.tag()),
        |_location| String::from("unexpected eof"),
        |error| format!("infrastructure failure: {error:?}"),
    )
    .expect("balanced template scan should succeed");

    assert_eq!(
        consumed,
        vec![
            TokenTag::STRING_SLICE_LITERAL,
            TokenTag::TEMPLATE_HEAD,
            TokenTag::STRING_SLICE_LITERAL,
            TokenTag::TEMPLATE_CLOSE,
            TokenTag::TEMPLATE_CLOSE,
        ]
    );
    assert_eq!(end, TokenIndex::try_from_index(6).expect("index should fit"));
    assert_eq!(cursor.peek().map(|token| token.tag()), Some(TokenTag::EOF));
}

#[test]
fn balanced_template_region_errors_on_eof_before_close_from_source() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::TemplateHead,
            TokenKind::StringSliceLiteral(string_table.intern("x")),
            TokenKind::Eof,
        ],
    );

    let mut cursor = full_cursor(&owner);
    let opening = cursor.advance().expect("opening token should be present");
    let error = consume_balanced_template_region_from_source(
        &mut cursor,
        opening,
        |_token| {},
        |_location| String::from("missing template close"),
        |error| format!("infrastructure failure: {error:?}"),
    )
    .expect_err("unterminated template should fail");

    assert_eq!(error, "missing template close");
}

#[test]
fn collect_scanned_symbol_references_matches_initializer_behavior_for_bare_symbol() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("value");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![TokenKind::Symbol(name), TokenKind::Newline, TokenKind::Eof],
    );
    let references = collect_scanned_symbol_references(
        TokenFactView::from_source(&owner),
        SourceId::COMPILATION_ROOT,
    );

    assert_eq!(references.len(), 1);
    assert_eq!(string_table.resolve(references[0].name), "value");
    assert!(references[0].dot_member.is_none());
    assert!(!references[0].followed_by_call);
    assert!(!references[0].followed_by_choice_namespace);
}

#[test]
fn collect_scanned_symbol_references_matches_initializer_behavior_for_dot_member() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("config");
    let member = string_table.intern("setting");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(name),
            TokenKind::Dot,
            TokenKind::Symbol(member),
            TokenKind::Newline,
        ],
    );
    let references = collect_scanned_symbol_references(
        TokenFactView::from_source(&owner),
        SourceId::COMPILATION_ROOT,
    );

    assert_eq!(references.len(), 1);
    assert_eq!(string_table.resolve(references[0].name), "config");
    assert_eq!(
        references[0].dot_member.map(|member| string_table.resolve(member)),
        Some("setting")
    );
    assert!(!references[0].followed_by_call);
}

#[test]
fn collect_scanned_symbol_references_matches_initializer_behavior_for_call() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("helper");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(name),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
        ],
    );
    let references = collect_scanned_symbol_references(
        TokenFactView::from_source(&owner),
        SourceId::COMPILATION_ROOT,
    );

    assert_eq!(references.len(), 1);
    assert_eq!(string_table.resolve(references[0].name), "helper");
    assert!(references[0].followed_by_call);
    assert!(!references[0].followed_by_choice_namespace);
}

#[test]
fn collect_scanned_symbol_references_matches_initializer_behavior_for_choice_namespace() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("Status");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(name),
            TokenKind::DoubleColon,
            TokenKind::Symbol(string_table.intern("Ready")),
        ],
    );
    let references = collect_scanned_symbol_references(
        TokenFactView::from_source(&owner),
        SourceId::COMPILATION_ROOT,
    );

    assert_eq!(references.len(), 1);
    assert_eq!(string_table.resolve(references[0].name), "Status");
    assert!(!references[0].followed_by_call);
    assert!(references[0].followed_by_choice_namespace);
}

// ---------------------------------------------------------------------------
// Declaration-initializer EOF delimiter reporting
// ---------------------------------------------------------------------------

fn eof_expected_delimiter(
    failure: &TokenScanFailure,
    string_table: &StringTable,
) -> Option<String> {
    let diagnostic = match failure {
        TokenScanFailure::Diagnostic(diagnostic) => diagnostic,
        TokenScanFailure::Infrastructure(error) => {
            panic!("token scanner infrastructure failure: {error:?}")
        }
    };
    let DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter } = &diagnostic.payload else {
        panic!(
            "expected UnexpectedEndOfFile diagnostic, got {:?}",
            diagnostic.payload
        );
    };
    expected_delimiter.map(|id| string_table.resolve(id).to_owned())
}

#[test]
fn eof_in_template_reports_closing_bracket() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(string_table.intern("value")),
            TokenKind::TemplateHead,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated template should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some("]".to_owned()),
        "open template expects ]"
    );
}

#[test]
fn eof_in_parenthesis_reports_closing_paren() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(string_table.intern("call")),
            TokenKind::OpenParenthesis,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated parenthesis should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some(")".to_owned()),
        "open parenthesis expects )"
    );
}

#[test]
fn eof_in_collection_reports_closing_brace() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(string_table.intern("data")),
            TokenKind::OpenCurly,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated collection should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some("}".to_owned()),
        "open collection/map expects }}",
    );
}

#[test]
fn eof_in_catch_block_reports_semicolon() {
    let mut string_table = StringTable::new();
    let load_name = string_table.intern("load");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(load_name),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
            TokenKind::Catch,
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated catch block should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some(";".to_owned()),
        "open catch block expects ;"
    );
}

#[test]
fn eof_in_value_producing_if_reports_semicolon() {
    let mut string_table = StringTable::new();
    let condition_name = string_table.intern("ready");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::If,
            TokenKind::Symbol(condition_name),
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated value-producing if should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some(";".to_owned()),
        "open value-producing if expects ;"
    );
}

#[test]
fn eof_in_nested_parenthesis_inside_catch_reports_innermost_paren() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::Symbol(string_table.intern("load")),
            TokenKind::OpenParenthesis,
            TokenKind::CloseParenthesis,
            TokenKind::Catch,
            TokenKind::Colon,
            TokenKind::Newline,
            TokenKind::Symbol(string_table.intern("arg")),
            TokenKind::OpenParenthesis,
            TokenKind::Eof,
        ],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated nested parenthesis should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some(")".to_owned()),
        "innermost open parenthesis inside catch expects ) not ;"
    );
}

#[test]
fn eof_in_parenthesis_inside_template_reports_innermost_paren() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![TokenKind::TemplateHead, TokenKind::OpenParenthesis, TokenKind::Eof],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated parenthesis inside template should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some(")".to_owned())
    );
}

#[test]
fn eof_in_template_inside_parenthesis_reports_innermost_template() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![TokenKind::OpenParenthesis, TokenKind::TemplateHead, TokenKind::Eof],
    );

    let error = scan_initializer(&owner, &mut string_table)
        .expect_err("unterminated template inside parenthesis should report EOF");
    assert_eq!(
        eof_expected_delimiter(&error, &string_table),
        Some("]".to_owned())
    );
}

#[test]
fn innermost_open_construct_is_none_when_nothing_is_open() {
    assert_eq!(innermost_open_construct(&[]), None);
}

#[test]
fn innermost_open_construct_prioritizes_depth_over_statement_blocks() {
    let open_constructs = [
        OpenConstruct::CatchBlock,
        OpenConstruct::ValueIfBlock,
        OpenConstruct::Parenthesis,
    ];
    assert_eq!(
        innermost_open_construct(&open_constructs),
        Some(OpenConstruct::Parenthesis)
    );
}

#[test]
fn declaration_initializer_range_keeps_a_multiline_pipe_list_together() {
    let mut string_table = StringTable::new();
    let field_a = string_table.intern("a");
    let field_b = string_table.intern("b");
    let next_statement = string_table.intern("next");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::TypeParameterBracket,
            TokenKind::Newline,
            TokenKind::Symbol(field_a),
            TokenKind::Assign,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut string_table)),
            TokenKind::Newline,
            TokenKind::Symbol(field_b),
            TokenKind::Assign,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("2", &mut string_table)),
            TokenKind::Newline,
            TokenKind::TypeParameterBracket,
            TokenKind::Newline,
            TokenKind::Symbol(next_statement),
            TokenKind::Eof,
        ],
    );

    let (range, _) = scan_initializer(&owner, &mut string_table)
        .expect("multiline pipe list should scan as one initializer");
    assert!(
        symbol_ids_in_range(&owner, range).contains(&field_b),
        "the scanner must not stop at the newline between record fields"
    );
    assert_eq!(boundary_tag(&owner, range), Some(TokenTag::NEWLINE));
}

#[test]
fn declaration_initializer_range_keeps_a_malformed_multiline_pipe_list_together() {
    let mut string_table = StringTable::new();
    let field_b = string_table.intern("b");
    let next_statement = string_table.intern("next");
    let owner = source_tokens_from_kinds(
        SourceId::COMPILATION_ROOT,
        vec![
            TokenKind::TypeParameterBracket,
            TokenKind::Newline,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("1", &mut string_table)),
            TokenKind::Comma,
            TokenKind::Newline,
            TokenKind::Symbol(field_b),
            TokenKind::Assign,
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("2", &mut string_table)),
            TokenKind::Newline,
            TokenKind::TypeParameterBracket,
            TokenKind::Newline,
            TokenKind::Symbol(next_statement),
            TokenKind::Eof,
        ],
    );

    let (range, _) = scan_initializer(&owner, &mut string_table)
        .expect("malformed multiline pipe list should remain one initializer");
    let symbol_ids = symbol_ids_in_range(&owner, range);
    assert!(
        symbol_ids.contains(&field_b),
        "the scanner must not stop at the comma after a malformed first pipe-list member"
    );
    assert!(
        !symbol_ids.contains(&next_statement),
        "the closing pipe must end the initializer before the next statement"
    );
    assert_eq!(range.end().index(), 10);
    assert_eq!(boundary_tag(&owner, range), Some(TokenTag::NEWLINE));
}

#[test]
fn initializer_references_carry_the_scanned_token_span() {
    let source_id = SourceId::from_index(1);
    let mut string_table = StringTable::new();
    let mut builder = ExtendedSpanBuilder::new();
    let name = string_table.intern("other_const");
    let span = LocalSpan::exact(4, 11, &mut builder).expect("reference bounds should encode");
    let owner = SourceTokens::try_from_tokens(
        source_id,
        vec![
            Token::new(TokenKind::Symbol(name), span),
            Token::new(TokenKind::Newline, LocalSpan::source_start()),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
        NumericLiteralStore::with_source(source_id),
        PathSyntaxTable::with_source(source_id),
    )
    .expect("canonical span fixture should satisfy source-token invariants");

    let references =
        collect_scanned_symbol_references(TokenFactView::from_source(&owner), source_id);

    assert_eq!(references.len(), 1);
    assert_eq!(references[0].name, name);
    assert_eq!(
        references[0].span,
        Some(SourceSpan::new(source_id, span)),
        "reference must keep exact token span and source identity"
    );
}
