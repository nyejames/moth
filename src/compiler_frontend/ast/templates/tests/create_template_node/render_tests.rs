use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn markdown_formatter_output_text_uses_authored_tir_spans() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source(
        "[$md:\n# Hello\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
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

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("markdown template should parse");
    let spans = collect_formatted_body_text_spans_from_tir(&template, &context);

    assert!(
        !spans.is_empty(),
        "expected body text pieces in formatted TIR"
    );
    assert!(
        spans.iter().all(Option::is_some),
        "formatter-emitted TIR text should keep authored source spans"
    );
}

fn collect_formatted_body_text_spans_from_tir(
    template: &Template,
    context: &ScopeContext,
) -> Vec<Option<SourceSpan>> {
    let reference = &template.tir_reference;
    let store = context.template_ir_store();
    let store = store.borrow();
    let tir = store
        .get_template(reference.root)
        .expect("referenced TIR template should exist");

    let mut spans = Vec::new();
    collect_text_node_spans(tir.root, &store, &mut spans);
    spans
}

fn collect_text_node_spans(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    output: &mut Vec<Option<SourceSpan>>,
) {
    let node = store.get_node(node_id).expect("TIR node should exist");
    match &node.kind {
        TemplateIrNodeKind::Text { .. } => {
            output.push(node.span);
        }
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                collect_text_node_spans(*child, store, output);
            }
        }
        _ => {}
    }
}

/// Collects the body text nodes from the template's authoritative formatted TIR root.
///
/// WHAT: walks the same-store TIR tree referenced by `template.tir_reference` and
///       gathers every interned string carried by a `Text` node.
/// WHY: simple formatted templates are TIR-authoritative; render-plan
///      assertions no longer observe formatter output for this shape, so the
///      formatted body text is gathered from the TIR tree instead.
fn collect_formatted_body_text_from_tir(
    template: &Template,
    context: &ScopeContext,
    string_table: &StringTable,
) -> Vec<String> {
    let reference = &template.tir_reference;
    let store = context.template_ir_store();
    let store = store.borrow();
    let tir = store
        .get_template(reference.root)
        .expect("referenced TIR template should exist");

    let mut texts = Vec::new();
    collect_text_nodes(tir.root, &store, string_table, &mut texts);
    texts
}

fn collect_text_nodes(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
    output: &mut Vec<String>,
) {
    let node = store.get_node(node_id).expect("TIR node should exist");
    match &node.kind {
        TemplateIrNodeKind::Text { text, .. } => {
            output.push(string_table.resolve(*text).to_owned());
        }
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                collect_text_nodes(*child, store, string_table, output);
            }
        }
        // Simple formatted bodies contain only text and opaque anchors. Other
        // node kinds are ignored because this helper is only asserting body text.
        _ => {}
    }
}

#[test]
fn markdown_formatter_produces_formatted_tir_output() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source(
        "[$md:\n# Hello\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
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

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("markdown template should parse");

    let formatted_body = collect_formatted_body_text_from_tir(&template, &context, &string_table);

    assert!(
        formatted_body
            .iter()
            .any(|text| text.contains("<h1>Hello</h1>")),
        "formatted TIR root should carry formatted markdown output"
    );
}
