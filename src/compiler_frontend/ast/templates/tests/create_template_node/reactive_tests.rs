use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;

#[test]
fn reactive_head_unknown_source_retains_exact_multibyte_span() {
    let source = "[$(π)]";
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path.to_owned(), &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let diagnostic = expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("an unknown reactive source should fail"),
    );
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));

    let primary_span = diagnostic
        .primary_span
        .expect("unknown reactive source should retain its exact source span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (3, 5));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "π");
}
