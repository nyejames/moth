use super::*;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIr, TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase,
    TemplateTirReference, TemplateViewContext, format_tir_template,
};
use crate::compiler_frontend::ast::templates::top_level_templates::FoldedConstTemplateResult;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

fn start_function_node(
    entry_dir: &InternedPath,
    body: Vec<AstNode>,
    location: SourceLocation,
    string_table: &mut StringTable,
) -> AstNode {
    AstNode {
        kind: NodeKind::Function(
            entry_dir.join_str(IMPLICIT_START_FUNC_NAME, string_table),
            FunctionSignature {
                parameters: vec![],
                returns: vec![ReturnSlot::success(DataType::StringSlice)],
            },
            body,
        ),
        location,
        scope: entry_dir.to_owned(),
    }
}

fn push_start_runtime_fragment_node(
    template: Template,
    location: SourceLocation,
    scope: InternedPath,
) -> AstNode {
    AstNode {
        kind: NodeKind::PushStartRuntimeFragment(Expression::template(
            template,
            ValueMode::ImmutableOwned,
        )),
        location,
        scope,
    }
}

fn collect_and_strip_comment_templates_for_tests_with_store(
    ast_nodes: &mut [AstNode],
    string_table: &mut StringTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
) -> Result<Vec<AstDocFragment>, TemplateError> {
    collect_and_strip_comment_templates(
        ast_nodes,
        string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        template_ir_store,
    )
}

#[test]
fn standalone_insert_helper_value_is_rejected_after_composition() {
    let source = r#"
value = [$insert("style"): color: red;]
"#;

    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::HelperOutsideWrapperSlot,
        }
    ));
}

#[test]
fn finalized_module_constants_materialize_const_templates_before_hir() {
    let source = r#"
wrapper #= [:<div class="frame">[$slot]</div>]
content #= [wrapper: [:Hello]]
"#;

    let (ast, string_table) = parse_single_file_ast(source);

    let wrapper_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(&string_table) == Some("wrapper"))
        .expect("wrapper constant should exist")
        .id;
    let content_id = ast
        .const_values
        .iter_module_constant_views()
        .find(|row| row.path.name_str(&string_table) == Some("content"))
        .expect("content constant should exist")
        .id;

    let wrapper_value = ast
        .const_values
        .string_value(wrapper_id)
        .expect("wrapper template should already be materialized before HIR");
    let content_value = ast
        .const_values
        .string_value(content_id)
        .expect("const template application should already be materialized before HIR");

    assert_eq!(
        string_table.resolve(wrapper_value),
        "<div class=\"frame\"></div>"
    );
    assert_eq!(
        string_table.resolve(content_value),
        "<div class=\"frame\"> Hello</div>"
    );
}

#[test]
fn collects_and_strips_top_level_doc_comment_templates() {
    let (ast, string_table) = parse_single_file_ast("[$doc:doc]\n[:runtime]");

    assert_eq!(ast.doc_fragments.len(), 1);
    assert!(matches!(ast.doc_fragments[0].kind, AstDocFragmentKind::Doc));
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "<p>doc</p>"
    );

    let entry_start = ast
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Function(_, _, _)))
        .expect("entry start should exist");
    let NodeKind::Function(_, _, body) = &entry_start.kind else {
        panic!("entry start should remain a function");
    };
    assert_eq!(
        body.len(),
        1,
        "top-level doc template should be stripped from runtime start body"
    );
}

#[test]
fn collects_top_level_doc_fragments_in_source_order() {
    let (ast, string_table) = parse_single_file_ast("[$doc:first]\n[$doc:second]\n[$doc:third]");
    let doc_fragments = ast.doc_fragments;

    assert_eq!(doc_fragments.len(), 3);
    assert_eq!(string_table.resolve(doc_fragments[0].value), "<p>first</p>");
    assert_eq!(
        string_table.resolve(doc_fragments[1].value),
        "<p>second</p>"
    );
    assert_eq!(string_table.resolve(doc_fragments[2].value), "<p>third</p>");
}

/// Builds a `$doc` template whose authoritative output is a directly
/// constructed, same-store formatted TIR root.
///
/// WHAT: pushes a literal body text node into a TIR store with
///       `TemplateIrBuilder`, finishes a markdown-styled doc template, runs
///       the TIR formatter adapter, and installs the formatted root as the
///       template's TIR reference.
/// WHY: lets doc-fragment collection tests prove that folding reads the
///      formatted TIR root built directly from TIR, with no detached body
///      representation involved.
fn formatted_doc_template_with_direct_tir(
    text: &str,
    string_table: &mut StringTable,
) -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let location = test_source_location(2);
    let text_id = string_table.intern(text);
    let byte_len = text.len();

    let style = Style {
        formatter: Some(markdown_formatter()),
        ..Style::default()
    };

    // WHAT: record the body-text shape so the parsed TIR template carries
    //       honest summary facts for the formatter pass.
    let parsed_summary = TemplateIrSummary {
        text_node_count: 1,
        text_byte_count: text.len(),
        estimated_output_bytes: text.len(),
        ..TemplateIrSummary::default()
    };

    let mut store = TemplateIrStore::new();
    let parsed_template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text_id,
            byte_len,
            TemplateSegmentOrigin::Body,
            location.clone(),
        );

        builder.finish_template(
            text_node,
            style.clone(),
            TemplateType::Comment(CommentDirectiveKind::Doc),
            parsed_summary.clone(),
            location.clone(),
        )
    };

    let store_handle = Rc::new(RefCell::new(store));
    let context = TemplateViewContext::default();

    let formatter_result = format_tir_template(
        &mut store_handle.borrow_mut(),
        parsed_template_id,
        TemplateTirPhase::Parsed,
        context,
        &style,
        string_table,
    )
    .expect("TIR formatter should succeed");

    let formatted_template_id = store_handle.borrow_mut().push_template(TemplateIr::new(
        formatter_result.root,
        style.clone(),
        TemplateType::Comment(CommentDirectiveKind::Doc),
        parsed_summary,
        location.clone(),
    ));

    let template = Template {
        tir_reference: TemplateTirReference {
            root: formatted_template_id,
            phase: TemplateTirPhase::Formatted,
            context,
        },
        location,
    };

    (template, store_handle)
}

#[test]
fn doc_fragment_folding_reads_directly_constructed_formatted_tir_root() {
    let mut string_table = StringTable::new();
    let entry_dir = InternedPath::from_single_str("main.moth", &mut string_table);
    let entry_scope = entry_dir.to_owned();

    let (doc_template, store) =
        formatted_doc_template_with_direct_tir("doc body", &mut string_table);

    let mut ast_nodes = vec![start_function_node(
        &entry_dir,
        vec![push_start_runtime_fragment_node(
            doc_template,
            test_source_location(2),
            entry_scope,
        )],
        test_source_location(1),
        &mut string_table,
    )];

    let doc_fragments = collect_and_strip_comment_templates_for_tests_with_store(
        &mut ast_nodes,
        &mut string_table,
        store,
    )
    .expect("doc fragment collection should succeed");

    assert_eq!(doc_fragments.len(), 1);
    assert_eq!(
        string_table.resolve(doc_fragments[0].value),
        "<p>doc body</p>",
        "doc fragment folding must read the directly constructed formatted TIR root"
    );
}

#[test]
fn top_level_doc_comment_produces_formatted_doc_fragment() {
    let source = r#"
[$doc:
doc body
]
"#;

    let (ast, string_table) = parse_single_file_ast(source);

    assert_eq!(
        ast.doc_fragments.len(),
        1,
        "top-level $doc comment should produce exactly one doc fragment"
    );
    assert_eq!(
        string_table.resolve(ast.doc_fragments[0].value),
        "<p>doc body</p>",
        "doc fragment should be formatted Markdown from the authoritative TIR root"
    );
}

#[test]
fn collects_const_top_level_fragments_from_tir_result_record() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("main.moth", &mut string_table);
    let value = string_table.intern("const html");

    let mut results = FxHashMap::default();
    results.insert(path.clone(), FoldedConstTemplateResult::new(value));

    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 0,
        header_path: path,
        location: test_source_location(2),
    }];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].runtime_insertion_index, 0);
    assert_eq!(string_table.resolve(collected[0].value), "const html");
}

#[test]
fn collects_const_top_level_fragments_from_folded_value() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("main.moth", &mut string_table);
    let value = string_table.intern("folded html");

    let mut results = FxHashMap::default();
    results.insert(path.clone(), FoldedConstTemplateResult::new(value));

    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 2,
        header_path: path,
        location: test_source_location(4),
    }];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].runtime_insertion_index, 2);
    assert_eq!(string_table.resolve(collected[0].value), "folded html");
}

#[test]
fn collects_mixed_const_top_level_fragments_in_source_order() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first.moth", &mut string_table);
    let second_path = InternedPath::from_single_str("second.moth", &mut string_table);

    let first_value = string_table.intern("first");
    let second_value = string_table.intern("second");

    let mut results = FxHashMap::default();
    results.insert(
        first_path.clone(),
        FoldedConstTemplateResult::new(first_value),
    );
    results.insert(
        second_path.clone(),
        FoldedConstTemplateResult::new(second_value),
    );

    let fragments = vec![
        TopLevelConstFragment {
            runtime_insertion_index: 1,
            header_path: first_path,
            location: test_source_location(2),
        },
        TopLevelConstFragment {
            runtime_insertion_index: 3,
            header_path: second_path,
            location: test_source_location(5),
        },
    ];

    let collected =
        collect_const_top_level_fragments(&fragments, &results).expect("collection should succeed");

    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0].runtime_insertion_index, 1);
    assert_eq!(string_table.resolve(collected[0].value), "first");
    assert_eq!(collected[1].runtime_insertion_index, 3);
    assert_eq!(string_table.resolve(collected[1].value), "second");
}

#[test]
fn missing_const_top_level_fragment_result_returns_compiler_error() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("main.moth", &mut string_table);

    let results = FxHashMap::<InternedPath, FoldedConstTemplateResult>::default();
    let fragments = vec![TopLevelConstFragment {
        runtime_insertion_index: 0,
        header_path: path,
        location: test_source_location(2),
    }];

    let error = collect_const_top_level_fragments(&fragments, &results)
        .expect_err("missing result should fail");

    assert!(
        format!("{:?}", error).contains("no corresponding folded template value"),
        "error should identify missing folded template value: {:?}",
        error
    );
}
