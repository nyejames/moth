use super::*;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

#[test]
fn entry_runtime_fragment_count_is_zero_with_no_templates() {
    let headers = parse_single_file_headers("x = 1\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "no top-level templates should yield runtime fragment count of 0"
    );
}

#[test]
fn entry_runtime_fragment_count_is_zero_for_const_only_templates() {
    // #[...] is a const (exported) template — it does not contribute to the runtime count.
    let headers = parse_single_file_headers("#[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "const templates should not increment the runtime fragment count"
    );
    assert_eq!(headers.const_fragment_count, 1);
}

#[test]
fn entry_runtime_fragment_count_reflects_runtime_template_count() {
    // [3] is a runtime template (no # prefix); one at top level should yield count 1.
    let headers = parse_single_file_headers("[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 1,
        "one runtime top-level template should yield runtime fragment count of 1"
    );
    assert!(headers.has_non_trivial_root_body);
}

#[test]
fn entry_runtime_fragment_count_accumulates_across_multiple_runtime_templates() {
    let headers = parse_single_file_headers("[1]\n[2]\n[3]\n");
    assert_eq!(
        headers.entry_runtime_fragment_count, 3,
        "three runtime top-level templates should yield runtime fragment count of 3"
    );
}

#[test]
fn entry_runtime_fragment_count_ignores_assigned_templates() {
    let headers = parse_single_file_headers(
        "buffer ~= [:initial content]\nbuffer = [:updated content]\nvalues = {\n[:nested collection]\n}\ngrouped = (\n[:nested group]\n)\n[:fragment]\n",
    );

    assert_eq!(
        headers.entry_runtime_fragment_count, 1,
        "only the direct top-level template should count as an entry runtime fragment"
    );
}

#[test]
fn entry_runtime_fragment_count_is_zero_when_parsed_as_non_entry_file() {
    // An imported root with only declarations reports runtime fragment count 0.
    // WHY: only the active module root contributes runtime fragments.
    let headers = parse_single_file_headers_with_entry(
        "f || -> I32:\n    1\n;\n",
        "src/lib.moth",
        "src/@page.moth",
    )
    .expect("headers should parse");
    assert_eq!(
        headers.entry_runtime_fragment_count, 0,
        "runtime_fragment_count must be 0 when the file is not the active root"
    );
}

#[test]
fn start_function_retains_segmented_source_runs_in_order_with_eof() {
    let source_text = "before = {\n\
                           (\n\
                               Status::Ready(1)\n\
                           ),\n\
                       }\n\
                       answer #= 2\n\
                       after = {\n\
                           \"ready\" = Status::Ready(\n\
                               3\n\
                           ),\n\
                       }\n\
                       icon = (\n\
                           @assets/icon.svg\n\
                       )\n\
                       Status :: Ready | value I32 |,;\n";
    let (headers, string_table) = parse_single_file_headers_with_table(source_text);
    assert_eq!(headers.headers.len(), 3);
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Constant { .. })),
        "the constant after the collection must remain a declaration"
    );
    assert!(
        headers
            .headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Choice { .. })),
        "the choice after the grouped file expression must remain a declaration"
    );
    let start_header = start_function_header(&headers);
    assert!(
        start_header.tokens.is_empty(),
        "start syntax should be retained by its sequence handle, not a contiguous body range"
    );
    let sequence = start_header
        .token_sequence
        .expect("active start function should retain a source sequence");
    let source = headers
        .source_token_owners
        .get(&start_header.tokens.source())
        .expect("start sequence should retain its canonical source owner");
    let source = source.tokens_ref();
    let view = source
        .token_sequence(sequence)
        .expect("start sequence handle should resolve");
    let ranges = view.ranges().collect::<Vec<_>>();
    assert_eq!(
        ranges.len(),
        3,
        "the constant and choice headers should split the start-body runs"
    );
    assert!(
        ranges
            .windows(2)
            .all(|pair| pair[0].end() < pair[1].start())
    );

    let mut body = header_body_tokens(&headers, start_header);
    let mut symbols = Vec::new();
    let mut contains_answer = false;
    let mut last_tag = None;
    let mut delimiter_tags = Vec::new();
    let mut path_count = 0;
    while let Some(token) = body.advance() {
        let tag = token.tag();
        last_tag = Some(tag);
        if matches!(
            tag,
            TokenTag::OPEN_PARENTHESIS
                | TokenTag::CLOSE_PARENTHESIS
                | TokenTag::OPEN_CURLY
                | TokenTag::CLOSE_CURLY
        ) {
            delimiter_tags.push(tag);
        }
        path_count += usize::from(tag == TokenTag::PATH);
        if tag == TokenTag::SYMBOL
            && let Some(symbol) = token.string_id()
        {
            let symbol = string_table.resolve(symbol);
            contains_answer |= symbol == "answer";
            symbols.push(symbol.to_owned());
        }
        if token.is_eof() {
            break;
        }
    }
    assert_eq!(
        symbols,
        [
            "before", "Status", "Ready", "after", "Status", "Ready", "icon"
        ]
    );
    assert_eq!(
        delimiter_tags,
        [
            TokenTag::OPEN_CURLY,
            TokenTag::OPEN_PARENTHESIS,
            TokenTag::OPEN_PARENTHESIS,
            TokenTag::CLOSE_PARENTHESIS,
            TokenTag::CLOSE_PARENTHESIS,
            TokenTag::CLOSE_CURLY,
            TokenTag::OPEN_CURLY,
            TokenTag::OPEN_PARENTHESIS,
            TokenTag::CLOSE_PARENTHESIS,
            TokenTag::CLOSE_CURLY,
            TokenTag::OPEN_PARENTHESIS,
            TokenTag::CLOSE_PARENTHESIS,
        ],
        "nested expression delimiters must remain intact in authored order"
    );
    assert_eq!(path_count, 1, "the file value must remain a runtime token");
    assert!(
        !contains_answer,
        "the constant declaration must not leak into the start body"
    );
    assert_eq!(
        last_tag,
        Some(TokenTag::EOF),
        "segmented start syntax must retain the source EOF sentinel"
    );

    let duplicate_source = format!("{source_text}Status :: Other,;\n");
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(&duplicate_source, "src/@page.moth", "src/@page.moth"),
        "a real duplicate after the closed expressions must still be rejected",
    );
    let duplicate_span = source_span_for(&duplicate_source, "Status", 3);
    assert!(errors.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.payload,
            DiagnosticPayload::DuplicateDeclaration { .. }
        ) && diagnostic.primary_span == Some(duplicate_span)
    }));
}
