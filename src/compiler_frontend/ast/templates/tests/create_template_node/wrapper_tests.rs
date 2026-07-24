use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn docs_style_data_wrapper_keeps_tir_node_count_bounded_for_many_rows() {
    let mut string_table = StringTable::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let declarations = docs_style_table_and_data_declarations(&mut string_table, &shared_store);

    let row_count = 48usize;
    let mut source =
        String::from("[table:\n    [header_row: [: Operator] [: Description] [: Precedence] ]\n");
    for index in 0..row_count {
        source.push_str(&format!(
            "    [data: [: op-{index}] [: desc-{index}] [: {index}] ]\n"
        ));
    }
    source.push(']');

    let mut token_stream = template_tokens_from_source(&source, &mut string_table);
    let context = constant_template_context(&token_stream.src_path, &declarations)
        .with_template_ir_store(shared_store);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("docs-style table with many rows should parse");

    let reference = &template.tir_reference;

    let store = context.template_ir_store();
    let store_borrow = store.borrow();

    assert!(
        reference.phase.is_at_least(TemplateTirPhase::Composed),
        "docs-style wrapper composition must reach at least the Composed phase"
    );

    let node_count = store_borrow.node_count();

    assert!(
        node_count > 0,
        "composed docs-style table should produce a nonzero TIR node count"
    );
    assert!(
        node_count <= 5000,
        "unexpectedly large shared-store TIR node count: {node_count} nodes for {row_count} rows"
    );
}

fn docs_style_table_and_data_declarations(
    string_table: &mut StringTable,
    shared_store: &Rc<RefCell<TemplateIrStore>>,
) -> Vec<Declaration> {
    let wrapper_scope = InternedPath::from_single_str("main.moth/#const_template0", string_table);

    let mut header_row_tokens = template_tokens_from_source(
        "[$children([:\n            <th style=\"border: 1px solid; padding: 0.5em; text-align: left;\">[$slot]</th>\n        ]):[$slot]]",
        string_table,
    );
    let header_row_context = new_constant_context(header_row_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(shared_store));
    let header_row = Template::new(
        &mut header_row_tokens,
        &header_row_context,
        vec![],
        string_table,
    )
    .expect("docs-style header row wrapper should parse");

    let mut table_tokens = template_tokens_from_source(
        "[:\n    <table style=\"[$slot(\"style\") ]\">\n        <tr style=\"background-color: hsla(107, 100%, 36%, 0.23);\">\n            [$slot(1)]\n        </tr>\n        [$children([:<tr style=\"border-bottom: 1px dotted grey;\">[$slot]</tr>]):\n            [$slot]\n        ]\n    </table>\n]",
        string_table,
    );
    let table_context = new_constant_context(table_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(shared_store));
    let table = Template::new(&mut table_tokens, &table_context, vec![], string_table)
        .expect("docs-style table wrapper should parse");

    let mut data_tokens = template_tokens_from_source(
        "[$children([: <td style=\"padding: 0.2em 0.5em;\">[$slot]</td>]):\n    [$slot]\n]",
        string_table,
    );
    let data_context = new_constant_context(data_tokens.src_path.to_owned())
        .with_template_ir_store(Rc::clone(shared_store));
    let data = Template::new(&mut data_tokens, &data_context, vec![], string_table)
        .expect("docs-style data wrapper should parse");

    vec![
        Declaration {
            id: wrapper_scope.append(string_table.intern("header_row")),
            value: Expression::template(header_row, ValueMode::ImmutableOwned),
        },
        Declaration {
            id: wrapper_scope.append(string_table.intern("table")),
            value: Expression::template(table, ValueMode::ImmutableOwned),
        },
        Declaration {
            id: wrapper_scope.append(string_table.intern("data")),
            value: Expression::template(data, ValueMode::ImmutableOwned),
        },
    ]
}

#[test]
fn child_wrapper_composition_marks_template_tir_reference_composed() {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source(
        "[$children([:<b>[$slot]</b>]): hello [:child] ]",
        &mut string_table,
    );
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("child-wrapper composition should parse");

    let reference = &template.tir_reference;

    assert!(
        reference.phase.is_at_least(TemplateTirPhase::Composed),
        "child-wrapper composition must advance the template's TIR reference to at least Composed"
    );
    assert!(
        reference.phase.is_at_least(TemplateTirPhase::Formatted),
        "wrapper-only composition with TIR-normalized wrappers should reach Formatted through the TIR formatter view"
    );

    assert!(
        reference.context.wrapper_context.is_some(),
        "direct-child wrappers must thread a wrapper-context overlay"
    );
    assert!(
        reference.context.slot_resolution.is_none(),
        "direct-child wrapper removal must not leave structural slot-resolution composition"
    );
}
