use super::*;

#[test]
fn template_option_capture_binding_is_not_visible_in_else_branch() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if maybe_name is |name|:
            [name]
        [else]
            [name]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = runtime_template_context(&token_stream.src_path.clone(), &mut string_table);

    let mut type_environment = TypeEnvironment::new();
    let maybe_name_type_id = type_environment.intern_option(type_environment.builtins().string);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let maybe_name = string_table.intern("maybe_name");
    let capture_name = string_table.intern("name");
    let declaration = Declaration {
        id: token_stream.src_path.append(maybe_name),
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
    context.add_var(declaration, None);

    let diagnostic = Template::new_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("option-present capture should not be visible in template else branch");
    let diagnostic = expect_template_diagnostic(diagnostic);

    // Unknown names in template heads now produce a structured UnknownName
    // diagnostic instead of a generic UnexpectedToken, which is the intended
    // improvement from routing symbol-led head items through create_expression.
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

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidTemplateStructure {
                reason: InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported
            }
        ),
        "unexpected payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn match_style_template_else_if_is_rejected() {
    let diagnostic =
        parse_template_error("[if false:\n    First\n[else if true is]\n    Second\n]");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidTemplateStructure {
                reason: InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported
            }
        ),
        "unexpected payload: {:?}",
        diagnostic.payload
    );
}

#[test]
fn else_in_template_head_is_rejected_until_body_sentinel_parsing() {
    let diagnostic = parse_template_error("[else]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::ElseInTemplateHead
        }
    ));
}

#[test]
fn nested_template_else_if_builds_independent_branch_chains() {
    let (template, context, string_table) = parse_control_flow_template_after_body_parse(
        "[if true:
            Outer first
            [if false:
                Inner first
            [else if true]
                Inner second
            [else]
                Inner fallback
            ]
        [else if false]
            Outer second
        [else]
            Outer fallback
        ]",
    );

    let outer_chain = expect_branch_chain_node(&template, &context);
    assert_eq!(
        branch_count(outer_chain, &context),
        2,
        "outer else-if should extend only the outer chain"
    );

    assert_body_node_static_contains(
        first_branch_body_node(outer_chain, &context),
        &context,
        &string_table,
        "Inner second",
    );
    assert_body_node_static_contains(
        first_branch_body_node(outer_chain, &context),
        &context,
        &string_table,
        "Inner fallback",
    );
    assert_body_node_static_contains(
        fallback_body_node(outer_chain, &context),
        &context,
        &string_table,
        "Outer fallback",
    );
}

#[test]
fn template_else_if_option_capture_binding_is_branch_local() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if false:
            hidden
        [else if maybe_name is |name|]
            [name]
        [else]
            [name]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = runtime_template_context(&token_stream.src_path.clone(), &mut string_table);

    let mut type_environment = TypeEnvironment::new();
    let maybe_name_type_id = type_environment.intern_option(type_environment.builtins().string);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let maybe_name = string_table.intern("maybe_name");
    let capture_name = string_table.intern("name");
    let declaration = Declaration {
        id: token_stream.src_path.append(maybe_name),
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
    context.add_var(declaration, None);

    let diagnostic = Template::new_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("else-if option capture should not be visible in the fallback branch");
    let diagnostic = expect_template_diagnostic(diagnostic);

    // Unknown names in template heads now produce a structured UnknownName
    // diagnostic instead of a generic UnexpectedToken.
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
fn orphan_template_else_in_normal_body_is_rejected_before_nested_template_parsing() {
    let diagnostic = parse_template_error("[: Before [else] After]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::OrphanTemplateElse
        }
    ));
}

#[test]
fn duplicate_template_else_is_rejected() {
    let diagnostic = parse_template_error(
        "[if true:
            Then
        [else]
            Else
        [else]
            Again
        ]",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::DuplicateTemplateElse
        }
    ));
}

#[test]
fn template_else_in_literal_body_template_if_is_rejected() {
    let diagnostic = parse_template_error(
        "[$doc, if true:
            literal then
        [else]
            literal else
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateElseInLiteralBody,
    );
}

#[test]
fn template_else_if_in_literal_body_template_if_is_rejected() {
    let diagnostic = parse_template_error(
        "[$doc, if true:
            literal then
        [else if false]
            literal else if
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateElseIfInLiteralBody,
    );
}

#[test]
fn malformed_template_else_forms_are_rejected() {
    for source in [
        "[if true:\nThen\n[else: nope]\n]",
        "[if true:\nThen\n[else, nope]\n]",
        "[if true:\nThen\n[else nope]\n]",
    ] {
        let diagnostic = parse_template_error(source);

        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidTemplateStructure {
                    reason: InvalidTemplateStructureReason::MalformedTemplateElse
                }
            ),
            "unexpected payload for {source:?}: {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn malformed_template_else_if_forms_are_rejected() {
    for source in [
        "[if true:\nThen\n[else if false:]\n]",
        "[if true:\nThen\n[else if false, nope]\n]",
    ] {
        let diagnostic = parse_template_error(source);

        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidTemplateStructure {
                    reason: InvalidTemplateStructureReason::MalformedTemplateElseIf
                }
            ),
            "unexpected payload for {source:?}: {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn template_else_if_requires_condition() {
    let diagnostic = parse_template_error(
        "[if true:
            Then
        [else if]
            Hidden
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateElseIfCondition,
    );
}

#[test]
fn orphan_template_else_if_is_rejected_before_nested_template_parsing() {
    let diagnostic = parse_template_error("[: Before [else if true] After]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateElseIf,
    );
}

#[test]
fn template_else_if_after_else_is_rejected() {
    let diagnostic = parse_template_error(
        "[if true:
            Then
        [else]
            Else
        [else if false]
            Too late
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateElseIfAfterElse,
    );
}

#[test]
fn inline_template_else_boundary_text_is_rejected() {
    for source in [
        "[if true: Visible [else]\nHidden]",
        "[if true:\nVisible\n[else] Hidden]",
        "[if true: [$slot][else]\nHidden]",
    ] {
        let diagnostic = parse_template_error(source);

        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidTemplateStructure {
                    reason: InvalidTemplateStructureReason::InlineTemplateElse
                }
            ),
            "unexpected payload for {source:?}: {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn inline_template_else_if_boundary_text_is_rejected() {
    for source in [
        "[if true: Visible [else if false]\nHidden]",
        "[if true:\nVisible\n[else if false] Hidden]",
    ] {
        let diagnostic = parse_template_error(source);

        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidTemplateStructure {
                    reason: InvalidTemplateStructureReason::InlineTemplateElseIf
                }
            ),
            "unexpected payload for {source:?}: {:?}",
            diagnostic.payload
        );
    }
}

#[test]
fn template_if_allows_slot_on_previous_line_before_else_sentinel() {
    let (template, context, _string_table) = parse_control_flow_template_after_body_parse(
        "[if true:
            [$slot]
        [else]
            Hidden
        ]",
    );

    let branch_chain = expect_branch_chain_node(&template, &context);

    assert!(
        body_node_contains_unresolved_slots(
            first_branch_body_node(branch_chain, &context),
            &context
        ),
        "slot placeholders before a next-line else sentinel should remain valid branch content"
    );
}

#[test]
fn direct_template_else_inside_loop_body_is_rejected() {
    let diagnostic = parse_template_error("[loop 0 to 3 |i|:\n[else]\n]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::TemplateElseInLoopBody
        }
    ));
}

#[test]
fn direct_template_else_if_inside_loop_body_is_rejected() {
    let diagnostic = parse_template_error("[loop 0 to 3 |i|:\n[else if true]\n]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateElseIfInLoopBody,
    );
}

#[test]
fn template_loop_break_is_structural_body_signal() {
    let (template, context, _unused_table) = parse_control_flow_template_after_body_parse(
        "[loop 0 to 2 |i|:
            [break]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);

    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1
    );
}

#[test]
fn template_loop_continue_is_structural_body_signal() {
    let (template, context, _unused_table) = parse_control_flow_template_after_body_parse(
        "[loop 0 to 2 |i|:
            [continue]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);

    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1
    );
}

#[test]
fn nested_template_if_break_inside_loop_is_structural_signal() {
    let (template, context, _unused_table) = parse_control_flow_template_after_body_parse(
        "[loop 0 to 2 |i|:
            [if true:
                [break]
            ]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);
    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1
    );
}

#[test]
fn nested_template_if_continue_inside_loop_is_structural_signal() {
    let (template, context, _unused_table) = parse_control_flow_template_after_body_parse(
        "[loop 0 to 2 |i|:
            [if true:
                [continue]
            ]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);
    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1
    );
}

#[test]
fn nested_template_with_loop_control_uses_enclosing_loop_without_owning_control_flow() {
    let (template, context, _unused_table) = parse_runtime_template(
        "[loop 0 to 2 |i|:
            [:
                [continue]
            ]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);
    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1,
        "the nested child should retain its loop-control node without being prepared as an owning loop",
    );
}

#[test]
fn nested_template_if_inside_loop_consumes_its_own_else_sentinel() {
    let (template, context, string_table) = parse_control_flow_template_after_body_parse(
        "[loop 0 to 3 |i|:
            [if true:
                Inner then
            [else]
                Inner else
            ]
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);

    assert_body_node_static_contains(
        loop_body_node(loop_node, &context),
        &context,
        &string_table,
        "Inner else",
    );
}

#[test]
fn template_if_composition_formats_each_branch_independently() {
    let (template, context, string_table) = parse_control_flow_template_after_composition(
        "[$md, if true:
            # Visible
        [else]
            # Hidden
        ]",
    );

    let branch_chain = expect_branch_chain_node(&template, &context);

    assert_body_node_static_contains(
        first_branch_body_node(branch_chain, &context),
        &context,
        &string_table,
        "<h1>Visible</h1>",
    );

    assert_body_node_static_contains(
        fallback_body_node(branch_chain, &context),
        &context,
        &string_table,
        "<h1>Hidden</h1>",
    );
    assert_body_node_static_excludes(
        first_branch_body_node(branch_chain, &context),
        &context,
        &string_table,
        "Hidden",
    );
    assert_body_node_static_excludes(
        fallback_body_node(branch_chain, &context),
        &context,
        &string_table,
        "Visible",
    );
}

#[test]
fn template_if_composition_applies_shared_head_prefix_to_each_branch() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut card_tokens = template_tokens_from_source(
        "[: <card>[$slot]</card>]",
        &mut string_table,
        &mut span_builder,
    );
    let card_context = new_constant_context(card_tokens.src_path.to_owned());
    let card_template = Template::new(&mut card_tokens, &card_context, vec![], &mut string_table)
        .expect("card wrapper should parse");

    let card_name = string_table.intern("card");
    let declarations = vec![Declaration {
        id: wrapper_scope.append(card_name),
        value: Expression::template(card_template, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    }];

    let mut token_stream = template_tokens_from_source(
        "[card, if true:
            Visible
        [else]
            Hidden
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let context = constant_template_context(&token_stream.src_path, &declarations)
        .with_template_ir_store(card_context.template_ir_store.clone());

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let template = Template::new_nested_template(
        &mut token_stream,
        &context,
        &mut type_interner,
        Vec::new(),
        &mut string_table,
        NestedTemplateParseOptions::runtime_capable(),
    )
    .expect("template if should parse through control-flow composition")
    .template;

    let branch_chain = expect_branch_chain_node(&template, &context);

    assert_body_node_static_contains(
        first_branch_body_node(branch_chain, &context),
        &context,
        &string_table,
        "<card>",
    );
    assert_body_node_static_contains(
        first_branch_body_node(branch_chain, &context),
        &context,
        &string_table,
        "Visible",
    );

    assert_body_node_static_contains(
        fallback_body_node(branch_chain, &context),
        &context,
        &string_table,
        "<card>",
    );
    assert_body_node_static_contains(
        fallback_body_node(branch_chain, &context),
        &context,
        &string_table,
        "Hidden",
    );
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
        "<h1>Item</h1>",
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

    // The control-flow child is TIR-owned: the parent's TIR root contains it as
    // a ChildTemplate node, and the $children wrapper is attached via wrapper-
    // context overlays rather than as an external content-mirror wrapper.
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
