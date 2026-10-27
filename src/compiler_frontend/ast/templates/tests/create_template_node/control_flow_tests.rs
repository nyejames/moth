use super::*;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::UnknownValueNameView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateStructureCode, InvalidTemplateStructureView,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

#[test]
fn template_option_capture_binding_is_scoped_to_selected_body() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
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
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let mut context = runtime_template_context(
        &source_path.clone(),
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );

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
        type_id: maybe_name_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };
    context.add_var(declaration, None, &path_fork);

    let diagnostic = Template::new_with_type_interner(
        &mut token_stream,
        source_path,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("option-present capture should not escape its selected body");
    let diagnostic = expect_template_diagnostic(diagnostic);

    let diagnostic = diagnostic
        .first()
        .expect("unknown-name failure should compact to one record");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0034");
    let facts = diagnostic
        .family_view::<UnknownValueNameView>()
        .expect("unknown-value-name failure should retain typed facts");
    assert_eq!(facts.name, capture_name);
}

#[test]
fn match_style_template_if_is_rejected() {
    let diagnostic = parse_template_error("[if true is: Visible]");

    let facts = diagnostic
        .first()
        .expect("invalid structure failure should compact to one record")
        .family_view::<InvalidTemplateStructureView>()
        .expect("invalid structure should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidTemplateStructureCode::TemplateMatchStyleControlFlowUnsupported
    );
}

#[test]
fn template_loop_composition_formats_body_without_repeating_shared_head_prefix() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let (template, context, string_table) = parse_control_flow_template_after_composition(
        "[\"prefix\", $md, loop 0 to 3 |i|:
            # Item
        ]",
        &mut expression_builder,
        &mut const_values,
    );

    let loop_node = expect_loop_node(&template, &context);

    assert_body_node_static_contains(
        loop_aggregate_wrapper_node(loop_node, &context),
        &context,
        &string_table,
        &expression_builder,
        &const_values,
        "prefix",
    );
    assert_body_node_static_contains(
        loop_body_node(loop_node, &context),
        &context,
        &string_table,
        &expression_builder,
        &const_values,
        "<h1 id=\"item\">Item</h1>",
    );
    assert_body_node_static_excludes(
        loop_body_node(loop_node, &context),
        &context,
        &string_table,
        &expression_builder,
        &const_values,
        "prefix",
    );
}
