//! Statement diagnostics retain exact source-owned byte spans.
//!
//! WHY: the plain diagnostic lane must preserve the token's authored span,
//! including multibyte source, rather than reconstructing line/column data.

use super::{UnexpectedScopeCloseContext, unexpected_scope_close, unexpected_statement_token};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidStatementPositionReason,
};
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageRegistry, ExternalSymbolId,
};
use crate::compiler_frontend::source::test_support::TestSourceContext;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result_with_registry;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};

fn tokenize_source(source: &str) -> (LexedSource, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("statement diagnostic fixture should tokenize");

    (lexed, string_table)
}

fn token_position(tokens: &LexedSource, tag: TokenTag) -> usize {
    (0..tokens.tokens.len())
        .find_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            let token = tokens.tokens.token(index).ok()?;
            (token.tag() == tag).then_some(index.index())
        })
        .expect("statement diagnostic fixture should contain the requested token")
}

#[test]
fn unexpected_statement_token_retains_exact_multibyte_span() {
    let (tokens, mut string_table) = tokenize_source("value = \"é\"\n,\n");
    let token_position = token_position(&tokens, TokenTag::COMMA);
    let owner = &tokens.tokens;
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut cursor = AstCursor::from_source_tokens(owner, range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(token_position)
        .expect("the canonical cursor must seek to the requested token");
    assert_eq!(cursor.current_tag(), TokenTag::COMMA);
    let token_span = cursor.current_span();
    let diagnostic = unexpected_statement_token(&cursor, &mut string_table)
        .expect("unexpected statement token projection should stay on the diagnostic lane");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedComma
        }
    ));
    assert_eq!(diagnostic.primary_span, Some(token_span));
}
#[test]
fn unexpected_scope_close_retains_exact_multibyte_span() {
    let (tokens, _string_table) = tokenize_source("value = \"é\";\n");
    let token_position = token_position(&tokens, TokenTag::END);
    let owner = &tokens.tokens;
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut cursor = AstCursor::from_source_tokens(owner, range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(token_position)
        .expect("the canonical cursor must seek to the requested token");
    assert_eq!(cursor.current_tag(), TokenTag::END);
    let token_span = cursor.current_span();
    let diagnostic = unexpected_scope_close(UnexpectedScopeCloseContext::Expression, &cursor);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedScopeCloseInExpression
        }
    ));
    assert_eq!(diagnostic.primary_span, Some(token_span));
}

fn assert_external_binding_collision(source: &str, name: &str) {
    let mut registry = ExternalPackageRegistry::new();
    registry
        .register_prelude_symbol(
            "prelude_print",
            ExternalSymbolId::Function(ExternalFunctionId::IoPrint),
        )
        .unwrap();
    let diagnostic = parse_single_file_ast_build_result_with_registry(source, registry)
        .err()
        .unwrap();

    assert_eq!(diagnostic.kind.code(), "MOTH-RULE-0038", "{source}");
    let DiagnosticPayload::ShadowedName {
        name: authored_name,
    } = diagnostic.payload
    else {
        panic!("expected the authored shadowed name: {source}");
    };
    let mut authored = TestSourceContext::new("@page.moth");
    authored
        .tokenize(
            source,
            &mut PathInternerFork::empty(),
            &StyleDirectiveRegistry::built_ins(),
            TokenizerEntryMode::SourceFile,
        )
        .unwrap();
    assert_eq!(authored.preparation_parts().0.resolve(authored_name), name);

    let start = source.rfind(name).unwrap() as u32;
    let span = LocalSpan::exact(start, name.len() as u32, &mut ExtendedSpanBuilder::new()).unwrap();
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
        "{source}",
    );
    assert!(diagnostic.labels.is_empty(), "{source}");
}

#[test]
fn loop_external_binding_collisions_retain_name_spans() {
    for (source, name) in [
        ("items {I32} = {1}\nloop items |io|:\n;\n", "io"),
        (
            "items = {1}\nloop items |prelude_print|:\n;\n",
            "prelude_print",
        ),
        ("loop 0 to 2 |io|:\n;\n", "io"),
        (
            "start I32 = 0\nend I32 = 2\nloop start to end |io|:\n;\n",
            "io",
        ),
        ("loop 0 to 2 |item, prelude_print|:\n;\n", "prelude_print"),
    ] {
        assert_external_binding_collision(source, name);
    }
}

#[test]
fn option_external_capture_collisions_retain_name_spans() {
    for (source, name) in [
        ("value I32? = 1\nif value is |io|:\n;\n", "io"),
        (
            "Slot type T = |value T?|\nslot = Slot(1)\nif slot.value is:\n    |prelude_print| =>\n    none =>\n;\n",
            "prelude_print",
        ),
    ] {
        assert_external_binding_collision(source, name);
    }
}

#[test]
fn choice_external_capture_collisions_retain_alias_spans() {
    assert_external_binding_collision(
        "Response :: Ready |amount I32|;\nvalue Response = Response::Ready(1)\nif value is:\n    Ready(amount as prelude_print) =>\n;\n",
        "prelude_print",
    );
    assert_external_binding_collision(
        "Outcome type T :: Some |value T|, Empty;\norigin = Outcome::Some(1)\nif (origin) is:\n    Some(value as io) =>\n    Empty =>\n;\n",
        "io",
    );
}

#[test]
fn catch_external_binding_collisions_retain_name_spans() {
    assert_external_binding_collision("value = 1 + 2 catch |io| then 0\n", "io");
    assert_external_binding_collision(
        "fail_value type T, E |fallback T, err E| -> T, E!:\n    return! err\n;\nvalue = fail_value(7, 1) catch |prelude_print| then 0\n",
        "prelude_print",
    );
}
