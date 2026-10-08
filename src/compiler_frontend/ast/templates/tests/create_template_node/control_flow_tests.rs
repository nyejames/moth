use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

#[test]
fn template_option_capture_binding_is_scoped_to_selected_body() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[:
    [if maybe_name is |name|:
        [name]
    ]
    [name]
]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
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
    let mut context =
        runtime_template_context(&source_path.clone(), &mut string_table, &mut path_fork);

    let mut type_environment = TypeEnvironment::new();
    let maybe_name_type_id = type_environment.intern_option(type_environment.builtins().string);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let maybe_name = string_table.intern("maybe_name");
    let capture_name = string_table.intern("name");
    let declaration = Declaration {
        id: path_fork
            .try_intern_child(source_path, maybe_name)
            .expect("test path fits"),
        value: Expression::new(
            ExpressionKind::NoValue,
            None,
            maybe_name_type_id,
            DataType::Option(Box::new(DataType::StringSlice)),
            ValueMode::ImmutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    };
    context.add_var(declaration, None, &path_fork);

    let diagnostic = Template::new_with_type_interner(
        &mut token_stream,
        source_path,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("option-present capture should not escape its selected body");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            } if name == capture_name
        ),
        "unexpected payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn match_style_template_if_is_rejected() {
    let diagnostic = parse_template_error("[if true is: Visible]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported
        }
    ));
}

#[test]
fn template_loop_composition_formats_body_without_repeating_shared_head_prefix() {
    let (template, context, string_table) = parse_control_flow_template_after_composition(
        "[\"prefix\", $md, loop 0 to 3 |i|:
            # Item
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);

    assert_body_node_static_contains(
        loop_aggregate_wrapper_node(loop_node, &context),
        &context,
        &string_table,
        "prefix",
    );
    assert_body_node_static_contains(
        loop_body_node(loop_node, &context),
        &context,
        &string_table,
        "<h1 id=\"item\">Item</h1>",
    );
    assert_body_node_static_excludes(
        loop_body_node(loop_node, &context),
        &context,
        &string_table,
        "prefix",
    );
}

#[test]
fn parent_children_wrappers_attach_conditionally_to_control_flow_child() {
    let (template, context, _unused_table) = parse_control_flow_template_after_composition(
        "[$children([:<li>[$slot]</li>]):
            [if true:
                item
            ]
        ]",
    );

    let store = context.template_ir_store.borrow();
    assert!(
        tir_root_has_control_flow_child(&template, &store),
        "TIR root should contain the control-flow child template"
    );
}

#[test]
fn fresh_control_flow_child_skips_parent_children_wrapper() {
    let (template, context, _unused_table) = parse_control_flow_template_after_composition(
        "[$children([:<li>[$slot]</li>]):
            [$fresh, if true:
                item
            ]
        ]",
    );

    let store = context.template_ir_store.borrow();
    assert!(
        tir_root_has_control_flow_child(&template, &store),
        "TIR root should contain the $fresh control-flow child template"
    );
}
