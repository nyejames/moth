use super::*;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn markdown_formatter_output_text_uses_non_default_tir_locations() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source("[$md:\n# Hello\n]", &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("markdown template should parse");
    let locations = collect_formatted_body_text_locations_from_tir(&template, &context);

    assert!(
        !locations.is_empty(),
        "expected body text pieces in formatted TIR"
    );
    assert!(
        locations
            .iter()
            .all(|location| !is_default_text_location(location)),
        "formatter-emitted TIR text should keep coarse source provenance"
    );
}

fn collect_formatted_body_text_locations_from_tir(
    template: &Template,
    context: &ScopeContext,
) -> Vec<SourceLocation> {
    let reference = &template.tir_reference;
    let store = context.template_ir_store();
    let store = store.borrow();
    let tir = store
        .get_template(reference.root)
        .expect("referenced TIR template should exist");

    let mut locations = Vec::new();
    collect_text_node_locations(tir.root, &store, &mut locations);
    locations
}

fn collect_text_node_locations(
    node_id: TemplateIrNodeId,
    store: &TemplateIrStore,
    output: &mut Vec<SourceLocation>,
) {
    let node = store.get_node(node_id).expect("TIR node should exist");
    match &node.kind {
        TemplateIrNodeKind::Text { .. } => {
            output.push(node.location.clone());
        }
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                collect_text_node_locations(*child, store, output);
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
    let mut token_stream =
        template_tokens_from_source("[$md:\n# Hello\n]", &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("markdown template should parse");

    let formatted_body = collect_formatted_body_text_from_tir(&template, &context, &string_table);

    assert!(
        formatted_body
            .iter()
            .any(|text| text.contains("<h1>Hello</h1>")),
        "formatted TIR root should carry formatted markdown output"
    );
}
