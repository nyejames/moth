use super::*;

#[test]
fn reactive_head_unknown_source_retains_exact_multibyte_span() {
    let source = "[$(π)]";
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let diagnostic = expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut PathInternerFork::empty())
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
