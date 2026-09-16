use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn docs_style_data_wrapper_keeps_tir_node_count_bounded_for_many_rows() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut span_builder = ExtendedSpanBuilder::new();
    let declarations = docs_style_table_and_data_declarations(
        &mut string_table,
        &shared_store,
        &mut span_builder,
        &mut path_fork,
    );

    let row_count = 48usize;
    let mut source =
        String::from("[table:\n    [header_row: [: Operator] [: Description] [: Precedence] ]\n");
    for index in 0..row_count {
        source.push_str(&format!(
            "    [data: [: op-{index}] [: desc-{index}] [: {index}] ]\n"
        ));
    }
    source.push(']');

    let mut file_tokens = template_tokens_from_source(
        &source,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let context = constant_template_context(&source_path, &declarations, &path_fork)
        .with_template_ir_store(shared_store);
    let mut token_stream = AstCursor::from_file_tokens(&mut file_tokens)
        .expect("test token stream must expose an AST cursor");

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
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
    span_builder: &mut ExtendedSpanBuilder,
    path_fork: &mut PathInternerFork,
) -> Vec<Declaration> {
    let wrapper_scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", string_table)
        .expect("test path fits");

    let mut header_row_file_tokens = template_tokens_from_source(
        "[$children([:\n            <th style=\"border: 1px solid; padding: 0.5em; text-align: left;\">[$slot]</th>\n        ]):[$slot]]",
        string_table,
        span_builder,
        path_fork,
    );
    let header_row_path = header_row_file_tokens.src_path;
    let header_row_context = new_constant_context(header_row_path.to_owned(), path_fork)
        .with_template_ir_store(Rc::clone(shared_store));
    let mut header_row_stream = AstCursor::from_file_tokens(&mut header_row_file_tokens)
        .expect("test token stream must expose an AST cursor");
    let header_row = Template::new(
        &mut header_row_stream,
        header_row_path,
        &header_row_context,
        vec![],
        string_table,
        path_fork,
    )
    .expect("docs-style header row wrapper should parse");

    let mut table_file_tokens = template_tokens_from_source(
        "[:\n    <table style=\"[$slot(\"style\") ]\">\n        <tr style=\"background-color: hsla(107, 100%, 36%, 0.23);\">\n            [$slot(1)]\n        </tr>\n        [$children([:<tr style=\"border-bottom: 1px dotted grey;\">[$slot]</tr>]):\n            [$slot]\n        ]\n    </table>\n]",
        string_table,
        span_builder,
        path_fork,
    );
    let table_path = table_file_tokens.src_path;
    let table_context = new_constant_context(table_path.to_owned(), path_fork)
        .with_template_ir_store(Rc::clone(shared_store));
    let mut table_stream = AstCursor::from_file_tokens(&mut table_file_tokens)
        .expect("test token stream must expose an AST cursor");
    let table = Template::new(
        &mut table_stream,
        table_path,
        &table_context,
        vec![],
        string_table,
        path_fork,
    )
    .expect("docs-style table wrapper should parse");

    let mut data_file_tokens = template_tokens_from_source(
        "[$children([: <td style=\"padding: 0.2em 0.5em;\">[$slot]</td>]):\n    [$slot]\n]",
        string_table,
        span_builder,
        path_fork,
    );
    let data_path = data_file_tokens.src_path;
    let data_context = new_constant_context(data_path.to_owned(), path_fork)
        .with_template_ir_store(Rc::clone(shared_store));
    let mut data_stream = AstCursor::from_file_tokens(&mut data_file_tokens)
        .expect("test token stream must expose an AST cursor");
    let data = Template::new(
        &mut data_stream,
        data_path,
        &data_context,
        vec![],
        string_table,
        path_fork,
    )
    .expect("docs-style data wrapper should parse");

    vec![
        Declaration {
            id: path_fork
                .try_intern_child(wrapper_scope, string_table.intern("header_row"))
                .expect("test path fits"),
            value: Expression::template(header_row, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        },
        Declaration {
            id: path_fork
                .try_intern_child(wrapper_scope, string_table.intern("table"))
                .expect("test path fits"),
            value: Expression::template(table, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        },
        Declaration {
            id: path_fork
                .try_intern_child(wrapper_scope, string_table.intern("data"))
                .expect("test path fits"),
            value: Expression::template(data, ValueMode::ImmutableOwned),
            binding_span: None,
            config_qualifier: None,
        },
    ]
}

#[test]
fn child_wrapper_composition_marks_template_tir_reference_composed() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut file_tokens = template_tokens_from_source(
        "[$children([:<b>[$slot]</b>]): hello [:child] ]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let context = new_constant_context(source_path.to_owned(), &path_fork);
    let mut token_stream = AstCursor::from_file_tokens(&mut file_tokens)
        .expect("test token stream must expose an AST cursor");

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
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
