use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenCursor, TokenIndex, TokenRange, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, OpenConstruct, TokenFactView, TokenScanFailure,
    collect_declaration_initializer_range, collect_scanned_symbol_references,
    consume_balanced_template_region_from_source, has_top_level_comma_before_statement_end,
    innermost_open_construct,
};
use std::sync::Arc;

enum FixturePart {
    Static(TokenTag),
    Symbol(crate::compiler_frontend::symbols::string_interning::StringId),
    Numeric(NumericLiteralToken),
    StringSlice(crate::compiler_frontend::symbols::string_interning::StringId),
}

fn source_tokens_from_parts(source: SourceId, parts: Vec<FixturePart>) -> Arc<SourceTokens> {
    let mut builder = TestSourceTokensBuilder::new(source);
    for part in parts {
        match part {
            FixturePart::Static(tag) => builder
                .push_static(tag, LocalSpan::source_start())
                .expect("static fixture token should build"),
            FixturePart::Symbol(value) => builder
                .push_symbol(TokenTag::SYMBOL, value, LocalSpan::source_start())
                .expect("symbol fixture token should build"),
            FixturePart::Numeric(literal) => builder
                .push_numeric(literal, LocalSpan::source_start())
                .expect("numeric fixture token should build"),
            FixturePart::StringSlice(value) => builder
                .push_symbol(
                    TokenTag::STRING_SLICE_LITERAL,
                    value,
                    LocalSpan::source_start(),
                )
                .expect("string fixture token should build"),
        }
    }
    builder.finish().expect("canonical fixture tokens should build")
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
    let owner = source_tokens_from_parts(SourceId::COMPILATION_ROOT, vec![FixturePart::Static(TokenTag::EOF)]);
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
    let owner = source_tokens_from_parts(SourceId::COMPILATION_ROOT, vec![FixturePart::Static(TokenTag::EOF)]);
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

    let nested_only = source_tokens_from_parts(
        source,
        vec![
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Numeric(NumericLiteralToken::test_new("1", &mut string_table)),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Numeric(NumericLiteralToken::test_new("2", &mut string_table)),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::EOF),
        ],
    );
    let nested_cursor = ast_cursor(&nested_only);
    assert!(!has_top_level_comma_before_statement_end(&nested_cursor));

    let top_level = source_tokens_from_parts(
        source,
        vec![
            FixturePart::Symbol(string_table.intern("a")),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Symbol(string_table.intern("b")),
            FixturePart::Static(TokenTag::ASSIGN),
            FixturePart::Numeric(NumericLiteralToken::test_new("1", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::EOF),
        ],
    );
    let top_level_cursor = ast_cursor(&top_level);
    assert!(has_top_level_comma_before_statement_end(&top_level_cursor));

    let multiline = source_tokens_from_parts(
        source,
        vec![
            FixturePart::Symbol(string_table.intern("a")),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(string_table.intern("b")),
            FixturePart::Static(TokenTag::ASSIGN),
            FixturePart::Symbol(string_table.intern("pair")),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::EOF),
        ],
    );
    let multiline_cursor = ast_cursor(&multiline);
    assert!(has_top_level_comma_before_statement_end(&multiline_cursor));
}

#[test]
fn canonical_comma_gate_respects_nested_call_depth() {
    let mut string_table = StringTable::new();
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(string_table.intern("call")),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Symbol(string_table.intern("a")),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Symbol(string_table.intern("b")),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Symbol(string_table.intern("tail")),
            FixturePart::Static(TokenTag::EOF),
        ],
    );
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(load_name),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::CATCH),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::THEN),
            FixturePart::Numeric(NumericLiteralToken::test_new("0", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::END),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(next_statement_name),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(load_name),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::CATCH),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::IF),
            FixturePart::Symbol(flag_name),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(io_name),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::END),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::THEN),
            FixturePart::Numeric(NumericLiteralToken::test_new("0", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::END),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(next_statement_name),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Static(TokenTag::TEMPLATE_HEAD),
            FixturePart::StringSlice(text_outer),
            FixturePart::Static(TokenTag::TEMPLATE_HEAD),
            FixturePart::StringSlice(text_inner),
            FixturePart::Static(TokenTag::TEMPLATE_CLOSE),
            FixturePart::Static(TokenTag::TEMPLATE_CLOSE),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Static(TokenTag::TEMPLATE_HEAD),
            FixturePart::StringSlice(string_table.intern("x")),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![FixturePart::Symbol(name), FixturePart::Static(TokenTag::NEWLINE), FixturePart::Static(TokenTag::EOF)],
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(name),
            FixturePart::Static(TokenTag::DOT),
            FixturePart::Symbol(member),
            FixturePart::Static(TokenTag::NEWLINE),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(name),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(name),
            FixturePart::Static(TokenTag::DOUBLE_COLON),
            FixturePart::Symbol(string_table.intern("Ready")),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(string_table.intern("value")),
            FixturePart::Static(TokenTag::TEMPLATE_HEAD),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(string_table.intern("call")),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(string_table.intern("data")),
            FixturePart::Static(TokenTag::OPEN_CURLY),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(load_name),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::CATCH),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Static(TokenTag::IF),
            FixturePart::Symbol(condition_name),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Symbol(string_table.intern("load")),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::CLOSE_PARENTHESIS),
            FixturePart::Static(TokenTag::CATCH),
            FixturePart::Static(TokenTag::COLON),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(string_table.intern("arg")),
            FixturePart::Static(TokenTag::OPEN_PARENTHESIS),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![FixturePart::Static(TokenTag::TEMPLATE_HEAD), FixturePart::Static(TokenTag::OPEN_PARENTHESIS), FixturePart::Static(TokenTag::EOF)],
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![FixturePart::Static(TokenTag::OPEN_PARENTHESIS), FixturePart::Static(TokenTag::TEMPLATE_HEAD), FixturePart::Static(TokenTag::EOF)],
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Static(TokenTag::TYPE_PARAMETER_BRACKET),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(field_a),
            FixturePart::Static(TokenTag::ASSIGN),
            FixturePart::Numeric(NumericLiteralToken::test_new("1", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(field_b),
            FixturePart::Static(TokenTag::ASSIGN),
            FixturePart::Numeric(NumericLiteralToken::test_new("2", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::TYPE_PARAMETER_BRACKET),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(next_statement),
            FixturePart::Static(TokenTag::EOF),
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
    let owner = source_tokens_from_parts(
        SourceId::COMPILATION_ROOT,
        vec![
            FixturePart::Static(TokenTag::TYPE_PARAMETER_BRACKET),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Numeric(NumericLiteralToken::test_new("1", &mut string_table)),
            FixturePart::Static(TokenTag::COMMA),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(field_b),
            FixturePart::Static(TokenTag::ASSIGN),
            FixturePart::Numeric(NumericLiteralToken::test_new("2", &mut string_table)),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Static(TokenTag::TYPE_PARAMETER_BRACKET),
            FixturePart::Static(TokenTag::NEWLINE),
            FixturePart::Symbol(next_statement),
            FixturePart::Static(TokenTag::EOF),
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
    let mut span_builder = ExtendedSpanBuilder::new();
    let name = string_table.intern("other_const");
    let span = LocalSpan::exact(4, 11, &mut span_builder).expect("reference bounds should encode");
    let mut token_builder = TestSourceTokensBuilder::new(source_id);
    token_builder
        .push_symbol(TokenTag::SYMBOL, name, span)
        .expect("symbol fixture token should build");
    token_builder
        .push_static(TokenTag::NEWLINE, LocalSpan::source_start())
        .expect("newline fixture token should build");
    token_builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = token_builder
        .finish()
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
