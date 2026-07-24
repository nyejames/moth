use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::TemplateSegmentOrigin;
use crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

#[test]
fn fresh_marks_template_to_skip_parent_child_wrappers() {
    let mut string_table = StringTable::new();
    let mut wrapper_tokens = template_tokens_from_source("[: inherited]", &mut string_table);
    let context = new_constant_context(wrapper_tokens.src_path.to_owned());
    let wrapper = Template::new(&mut wrapper_tokens, &context, vec![], &mut string_table)
        .expect("inherited wrapper should parse");
    let inherited_wrapper = {
        let reference = &wrapper.tir_reference;
        TemplateWrapperReference::new(reference.root, reference.phase, reference.context)
    };

    let mut token_stream =
        template_tokens_from_source("[$fresh, $md:\n# Hello\n]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(
        &mut token_stream,
        &context,
        vec![inherited_wrapper],
        &mut string_table,
    )
    .expect("template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert!(effective_style.formatter.is_some());
    assert!(effective_style.skip_parent_child_wrappers);

    // No $children directive means no wrapper-context overlay is attached.
    assert!(
        template.tir_reference.context.wrapper_context.is_none(),
        "template without $children should not attach wrapper-context overlays"
    );
}

#[test]
fn children_directive_attaches_wrapper_context_to_direct_child() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[$children([:prefix]): [: child]]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("template should parse");

    // The $children directive attaches a wrapper-context overlay carrying
    // one inherited wrapper set for the direct child occurrence.
    let store = context.template_ir_store.borrow();
    let wrapper_overlay_id = template
        .tir_reference
        .context
        .wrapper_context
        .expect("$children should attach a wrapper-context overlay");
    let wrapper_overlay = store
        .wrapper_context_overlay(wrapper_overlay_id)
        .expect("wrapper-context overlay should exist");
    assert_eq!(
        wrapper_overlay.contexts.len(),
        1,
        "one child occurrence should carry inherited wrapper context"
    );
}

#[test]
fn children_directive_accepts_const_string_reference() {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let prefix_name = string_table.intern("prefix");
    let declarations = vec![Declaration {
        id: scope.append(prefix_name),
        value: Expression::string_slice(
            string_table.intern("prefix: "),
            SourceLocation {
                scope: InternedPath::new(),
                start_pos: CharPosition {
                    line_number: 1,
                    char_column: 0,
                },
                end_pos: CharPosition {
                    line_number: 1,
                    char_column: 120,
                },
            },
            ValueMode::ImmutableOwned,
        ),
    }];

    let mut token_stream =
        template_tokens_from_source("[$children(prefix): [: child]]", &mut string_table);
    let context = constant_template_context(&token_stream.src_path, &declarations);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("children directive should accept const-folded references");

    // Resolve the wrapper reference through the TIR overlay system.
    let store = context.template_ir_store.borrow();
    let wrapper_overlay_id = template
        .tir_reference
        .context
        .wrapper_context
        .expect("$children should attach a wrapper-context overlay");
    let wrapper_overlay = store
        .wrapper_context_overlay(wrapper_overlay_id)
        .expect("wrapper-context overlay should exist");
    let wrapper_context = &wrapper_overlay.contexts[0].1;
    let wrapper_set_ref = wrapper_context
        .inherited_wrapper_set
        .expect("child occurrence should carry an inherited wrapper set");

    let store = context.template_ir_store.borrow();
    let wrapper_set = store
        .get_wrapper_set(wrapper_set_ref)
        .expect("wrapper set should exist in the store");
    let wrapper_reference = wrapper_set
        .wrappers
        .first()
        .expect("children directive should record one wrapper");

    let wrapper_id = wrapper_reference.root;
    let wrapper_tir = store
        .get_template(wrapper_id)
        .expect("normalized string wrapper TIR should exist in the module store");
    let root = store
        .get_node(wrapper_tir.root)
        .expect("normalized string wrapper root should exist");
    let TemplateIrNodeKind::Sequence { children } = &root.kind else {
        panic!("normalized string wrapper should have a sequence root");
    };
    let [literal_id] = children.as_slice() else {
        panic!("normalized string wrapper should contain one literal node");
    };
    let literal = store
        .get_node(*literal_id)
        .expect("normalized string wrapper literal should exist");
    let TemplateIrNodeKind::Text { text, origin, .. } = &literal.kind else {
        panic!("normalized string wrapper should contain literal TIR text");
    };

    assert_eq!(string_table.resolve(*text), "prefix: ");
    assert_eq!(*origin, TemplateSegmentOrigin::Body);
}

#[test]
fn children_directive_rejects_runtime_values() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[$children(value): [: child]]", &mut string_table);
    let context = runtime_template_context(&token_stream.src_path, &mut string_table);

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("children directive should reject runtime values");

    match &error.payload {
        DiagnosticPayload::InvalidTemplateDirective {
            reason: InvalidTemplateDirectiveReason::InvalidChildrenArgument,
            ..
        } => {}
        payload => panic!("expected invalid template directive payload, found {payload:?}"),
    }
}

#[test]
fn children_wrappers_are_applied_to_direct_children() {
    let rendered = folded_template_output("[$children([: pref[$slot]suf]): [: body]]");
    assert!(rendered.contains("pref"));
    assert!(rendered.contains("body"));
    assert!(rendered.contains("suf"));
}

#[test]
fn children_string_wrappers_preserve_prepend_semantics() {
    let rendered = folded_template_output("[$children(\"prefix: \"): [: child]]");

    let prefix_index = rendered.find("prefix: ").expect("wrapper should render");
    let child_index = rendered.find("child").expect("child should render");
    assert!(prefix_index < child_index);
}

#[test]
fn children_wrappers_do_not_apply_to_grandchildren() {
    let rendered = folded_template_output("[$children([:<wrap>[$slot]</wrap>]): [:[ : body]]]");

    assert!(rendered.contains("body"));
    assert_eq!(rendered.matches("<wrap>").count(), 1);
}

#[test]
fn markdown_is_not_inherited_by_grandchildren() {
    let rendered = folded_template_output("[$md:\n[: [: <b>grandchild-body</b> ]]\n]");

    assert!(rendered.contains("<b>grandchild-body</b>"));
    assert!(!rendered.contains("&lt;b&gt;grandchild-body&lt;/b&gt;"));
}

#[test]
fn markdown_must_be_redeclared_at_each_nested_template_level() {
    let rendered = folded_template_output("[$md:\n[$md:\n[$md: <b>grandchild-body</b>]\n]\n]");

    assert!(rendered.contains("&lt;b&gt;grandchild-body&lt;/b&gt;"));
    assert!(!rendered.contains("<b>grandchild-body</b>"));
}

#[test]
fn nested_inline_templates_inside_table_cells_do_not_become_extra_cells() {
    let rendered = folded_template_output(
        "[
            $children([:<tr>[$slot]</tr>]):
            [$children([:<td>[$slot]</td>]):
                [: Cell with [:inline] content]
                [: Plain cell]
            ]
        ]",
    );
    assert_eq!(rendered.matches("<tr>").count(), 1, "expected one row");

    assert_eq!(
        rendered.matches("<td>").count(),
        2,
        "expected two cells; nested inline templates must not become extra cells"
    );

    assert!(
        rendered.contains("Cell with"),
        "expected text before nested inline template"
    );
    assert!(
        rendered.contains("inline"),
        "expected nested inline template content"
    );
    assert!(
        rendered.contains("content"),
        "expected text after nested inline template"
    );
}

#[test]
fn children_directive_argument_ending_at_template_boundary_uses_children_reason() {
    // The `]` closes the outer template before any argument expression is
    // authored. This stays on the directive owner (`InvalidChildrenArgument`),
    // not true file EOF, which header balancing owns as `MOTH-SYNTAX-0017`.
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source("[$children(]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("empty $children argument at a template boundary should fail to parse");

    match &error.payload {
        DiagnosticPayload::InvalidTemplateDirective {
            reason: InvalidTemplateDirectiveReason::InvalidChildrenArgument,
            ..
        } => {}
        payload => panic!(
            "expected InvalidChildrenArgument for $children argument ending at a template boundary, found {payload:?}"
        ),
    }
    assert!(
        !is_default_error_location(&error.primary_location),
        "$children argument ending at a template boundary should carry a meaningful source location, got {:?}",
        error.primary_location
    );
}
