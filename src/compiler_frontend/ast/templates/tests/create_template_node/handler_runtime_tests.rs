use super::*;

#[test]
fn handler_directive_argument_preserves_infrastructure_failure() {
    assert_stale_template_directive_argument_is_infrastructure("[$code(stale_template): body]");
}

#[test]
fn children_directive_argument_preserves_infrastructure_failure() {
    assert_stale_template_directive_argument_is_infrastructure("[$children(stale_template): body]");
}

#[test]
fn truncated_template_head_stream_returns_missing_closing_delimiter() {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let context = new_constant_context(scope.to_owned());

    let mut token_stream = FileTokens::new(
        scope,
        SourceId::COMPILATION_ROOT,
        vec![
            token(TokenKind::TemplateHead, 1),
            numeric_token("3", 1, &mut string_table),
        ],
    );

    let result = Template::new(&mut token_stream, &context, vec![], &mut string_table);
    assert!(
        result.is_err(),
        "truncated template-head stream without closing delimiter should produce an error"
    );
}

#[test]
fn const_required_template_head_folds_const_record_instance_field() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[html_defaults.color]",
        &mut string_table,
        &mut span_builder,
    );
    let scope = token_stream.src_path.clone();

    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let struct_name = string_table.intern("HtmlDefaults");
    let struct_path = scope.append(struct_name);
    let field_name = string_table.intern("color");
    let field_path = struct_path.append(field_name);
    let (_, struct_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path.clone(),
        fields: vec![FieldDefinition {
            name: field_path.clone(),
            type_id: string_type_id,
            span: None,
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });

    let field_value = Expression::string_slice(
        string_table.intern("green"),
        None,
        ValueMode::ImmutableOwned,
    );
    let record_value = Expression::struct_instance(
        struct_path,
        vec![Declaration {
            id: field_path,
            value: field_value,
            binding_span: None,
            config_qualifier: None,
        }],
        None,
        ValueMode::ImmutableOwned,
        true,
        None,
        struct_type_id,
    );
    let record_name = string_table.intern("html_defaults");
    let declaration = Declaration {
        id: scope.append(record_name),
        value: record_value,
        binding_span: None,
        config_qualifier: None,
    };
    let context = constant_template_context(&scope, &[declaration]);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let template = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect("const-required template head should project const-record field values")
    .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "green");
}

#[test]
fn runtime_template_if_rejects_insert_leaking_from_branch() {
    let error = parse_control_flow_template_after_composition_error(
        "[if true:
            [$insert(\"style\"): color: red;]
        ]",
    );

    assert_invalid_template_structure(
        &error,
        InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
    );
}

#[test]
fn runtime_template_loop_rejects_insert_leaking_from_body() {
    let error = parse_control_flow_template_after_composition_error(
        "[loop 0 to 2 |i|:
            [$insert(\"row\"): [i]]
        ]",
    );

    assert_invalid_template_structure(
        &error,
        InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
    );
}

#[test]
fn runtime_template_loop_with_continue_preserves_loop_control_signal() {
    let (template, context, _string_table) = parse_runtime_template(
        "[loop 0 to 2 |i|:
            <li>before [i]</li>
            [continue]
            <li>after [i]</li>
        ]",
    );

    let loop_node = expect_loop_node(&template, &context);
    assert_eq!(
        body_node_loop_control_signal_count(loop_body_node(loop_node, &context), &context),
        1,
        "loop body should contain a loop control signal",
    );
}

#[test]
fn runtime_template_loop_with_continue_inside_parent_parses() {
    let (template, context, _string_table) = parse_runtime_template(
        "[:
            outer
            [loop 0 to 2 |i|:
                <li>before [i]</li>
                [continue]
                <li>after [i]</li>
            ]
        ]",
    );

    let store = context.template_ir_store.borrow();
    assert!(
        tir_root_has_control_flow_child(&template, &store),
        "outer template TIR root should contain the nested loop"
    );
}

#[test]
fn runtime_template_loop_with_continue_as_slot_fill_parses() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut shell_tokens =
        template_tokens_from_source("[:<ul>[$slot]</ul>]", &mut string_table, &mut span_builder);
    let shell_context = new_constant_context(shell_tokens.src_path.to_owned());
    let shell_template =
        Template::new(&mut shell_tokens, &shell_context, vec![], &mut string_table)
            .expect("slot shell should parse");

    let mut token_stream = template_tokens_from_source(
        "[:
            before
            [list_shell, loop keep_going:
                [break]
                <li>hidden</li>
            ]
            after
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let scope = token_stream.src_path.clone();
    let list_shell_name = string_table.intern("list_shell");
    let keep_going_name = string_table.intern("keep_going");
    let declaration = Declaration {
        id: scope.append(list_shell_name),
        value: Expression::template(shell_template, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let condition_declaration = Declaration {
        id: scope.append(keep_going_name),
        value: Expression::new(
            ExpressionKind::NoValue,
            None,
            builtin_type_ids::BOOL,
            DataType::Bool,
            ValueMode::ImmutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    };
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(vec![
                declaration,
                condition_declaration,
            ])),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        &scope,
        &frontend_test_style_directives(),
    )
    .with_template_ir_store(shell_context.template_ir_store.clone());

    Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("slot-fill loop should parse");
}

#[test]
fn runtime_template_if_allows_unresolved_slot_receiver() {
    let (template, context, _string_table) = parse_runtime_template_without_validation(
        "[if true:
            [$slot]
        ]",
    );

    let store = context.template_ir_store.borrow();
    validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect("receiver $slot markers may remain inside runtime control flow");
}

#[test]
fn runtime_template_if_rejects_unresolved_insert() {
    let diagnostic = parse_template_error(
        "[if true:
            [$insert(\"style\"): color: red;]
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
    );
}

#[test]
fn const_required_template_if_allows_unresolved_slot_wrapper() {
    let (template, context, _string_table) = parse_const_required_template(
        "[if true:
            [$slot]
        ]",
    );

    let branch_chain = expect_branch_chain_node(&template, &context);

    assert!(
        body_node_contains_unresolved_slots(
            first_branch_body_node(branch_chain, &context),
            &context
        ),
        "const-required helper templates may keep slot structure for later composition"
    );
}

#[test]
fn const_required_template_if_folds_selected_branch() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[if true:
            Visible
        [else]
            Hidden
        ]",
    );

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("Visible"));
    assert!(!rendered.contains("Hidden"));
}

#[test]
fn const_required_template_else_if_folds_first_selected_branch() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[if false:
            First
        [else if true]
            Second
        [else if true]
            Third
        [else]
            Fallback
        ]",
    );

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("Second"));
    assert!(!rendered.contains("First"));
    assert!(!rendered.contains("Third"));
    assert!(!rendered.contains("Fallback"));
}

#[test]
fn const_required_template_if_inlines_same_file_source_const_bool() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if show_banner:
            Visible
        [else]
            Hidden
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let show_banner = string_table.intern("show_banner");
    let declaration = Declaration {
        id: token_stream.src_path.append(show_banner),
        value: Expression::bool(true, None, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let context = constant_template_context(&token_stream.src_path, &[declaration]);

    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required template if should inline source const bool")
            .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("Visible"));
    assert!(!rendered.contains("Hidden"));
}

#[test]
fn const_required_template_if_inlines_imported_source_const_bool() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if show_banner:
            Visible
        [else]
            Hidden
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let show_banner = string_table.intern("show_banner");
    let flags_scope = InternedPath::from_single_str("flags.moth", &mut string_table);
    let imported_path = flags_scope.append(show_banner);
    let declaration = Declaration {
        id: imported_path.clone(),
        value: Expression::bool(true, None, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let context = imported_const_template_context(&token_stream.src_path, declaration, show_banner);

    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required template if should inline imported source const bool")
            .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("Visible"));
    assert!(!rendered.contains("Hidden"));
}

#[test]
fn const_required_template_if_false_without_else_skips_shared_head_output() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut card_tokens = template_tokens_from_source(
        "[:<card>[$slot]</card>]",
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
        "[card, if false:
            Visible
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = constant_template_context(&token_stream.src_path, &declarations);
    // Production scopes in one module share one module-local TIR store. Keep
    // the declaration fixture on that topology so the wrapper reference stays
    // resolvable without compatibility reconstruction.
    context.template_ir_store = card_context.template_ir_store.clone();
    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required template if should parse")
            .template;

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "");
}

#[test]
fn const_required_template_if_inspects_inactive_branch_control_flow() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[if true:
            Visible
        [else]
            [loop 0 to 1 |i|:
                Hidden
            ]
        ]",
    );

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("Visible"));
    assert!(!rendered.contains("Hidden"));
}

#[test]
fn const_required_template_range_loop_folds_iteration_bindings() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to & 3 |i, index|:
            [index]:[i];
        ]",
    );

    {
        assert_eq!(
            classify_template_from_effective_tir(&template, &context.template_ir_store)
                .expect("const classification should succeed"),
            TemplateConstValueKind::RenderableString
        );
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "0:0;1:1;2:2;3:3;");
}

#[test]
fn const_required_template_range_loop_folds_expressions_with_iteration_bindings() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to 2 |i|:
            [i + 1];
        ]",
    );

    {
        assert_eq!(
            classify_template_from_effective_tir(&template, &context.template_ir_store)
                .expect("const classification should succeed"),
            TemplateConstValueKind::RenderableString
        );
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "1;2;");
}

#[test]
fn const_required_template_loop_allows_nested_if_to_use_iteration_binding() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to 2 |i|:
            [if true:
                [i]
            ]
        ]",
    );

    {
        assert_eq!(
            classify_template_from_effective_tir(&template, &context.template_ir_store)
                .expect("const classification should succeed"),
            TemplateConstValueKind::RenderableString
        );
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "01");
}

#[test]
fn const_required_template_loop_allows_nested_if_condition_to_use_iteration_binding() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to 3 |i|:
            [if i is 1:
                [:T]
            [else]
                [i]
            ]
        ]",
    );

    {
        assert_eq!(
            classify_template_from_effective_tir(&template, &context.template_ir_store)
                .expect("const classification should succeed"),
            TemplateConstValueKind::RenderableString
        );
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "0T2");
}

#[test]
fn const_required_template_loop_body_if_can_use_source_const_condition() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[loop 0 to 2 |i|:
            [if show_item:
                [i]
            ]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let show_item = string_table.intern("show_item");
    let declaration = Declaration {
        id: token_stream.src_path.append(show_item),
        value: Expression::bool(true, None, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let context = constant_template_context(&token_stream.src_path, &[declaration]);

    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("nested const-required template if should inline source const bool")
            .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "01");
}

#[test]
fn const_required_template_collection_loop_folds_iteration_bindings() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop {\"Ada\", \"Bo\"} |name, index|:
            [index]-[name];
        ]",
    );

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "0-Ada;1-Bo;");
}

#[test]
fn const_required_template_zero_iteration_loop_skips_shared_head_output() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut card_tokens = template_tokens_from_source(
        "[:<card>[$slot]</card>]",
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
        "[card, loop 0 to 0 |i|:
            [i]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let context = constant_template_context(&token_stream.src_path, &declarations)
        .with_template_ir_store(card_context.template_ir_store.clone());
    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required zero loop should parse")
            .template;

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "");
}

#[test]
fn const_required_template_loop_wraps_aggregate_once() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let wrapper_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut card_tokens = template_tokens_from_source(
        "[:<card>[$slot]</card>]",
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
        "[card, loop 0 to 2 |i|:
            [i]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let context = constant_template_context(&token_stream.src_path, &declarations)
        .with_template_ir_store(card_context.template_ir_store.clone());
    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required loop should parse")
            .template;

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "<card>01</card>");
}

#[test]
fn const_required_template_conditional_loop_false_folds_to_no_output() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop false:
            Never
        ]",
    );

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "");
}

#[test]
fn const_required_template_conditional_loop_true_is_rejected() {
    let diagnostic = parse_const_required_template_error(
        "[loop true:
            Never
        ]",
    );

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
    );
}

#[test]
fn const_required_template_conditional_loop_reports_runtime_condition() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[loop keep_going:
            Never
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = new_constant_context(token_stream.src_path.clone());
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let keep_going = string_table.intern("keep_going");

    context.add_var(
        Declaration {
            id: token_stream.src_path.append(keep_going),
            value: Expression::new(
                ExpressionKind::NoValue,
                None,
                builtin_type_ids::BOOL,
                DataType::Bool,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
        None,
    );

    let diagnostic = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("const-required conditional loop should reject runtime conditions");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateLoopConditionNotConst,
    );
}

#[test]
fn const_required_template_loop_reports_non_const_collection_source() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[loop items |item|:
            [item]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = new_constant_context(token_stream.src_path.clone());
    let mut type_environment = TypeEnvironment::new();
    let collection_type_id =
        type_environment.intern_collection(type_environment.builtins().string, None);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let items = string_table.intern("items");

    context.add_var(
        Declaration {
            id: token_stream.src_path.append(items),
            value: Expression::new(
                ExpressionKind::NoValue,
                None,
                collection_type_id,
                DataType::collection(DataType::StringSlice),
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
        None,
    );

    let diagnostic = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("const-required loop should reject runtime collection source");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateLoopSourceNotConst,
    );
}

#[test]
fn const_required_template_loop_reports_non_const_body() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[loop 0 to 1 |i|:
            [value]
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = new_constant_context(token_stream.src_path.clone());
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let value = string_table.intern("value");

    context.add_var(
        Declaration {
            id: token_stream.src_path.append(value),
            value: Expression::new(
                ExpressionKind::NoValue,
                None,
                builtin_type_ids::STRING,
                DataType::StringSlice,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
        None,
    );

    let diagnostic = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("const-required loop should reject runtime body content");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateLoopBodyNotConst,
    );
}

#[test]
fn const_required_template_if_prepares_a_foldable_view_from_body_tir_roots() {
    // Construction owns the single const preparation, so the outcome it carries is the contract.
    // Asserting `Foldable` rather than "parsing returned Ok" proves the branch bodies were read
    // through their module-local TIR roots and classified, not merely accepted.
    let construction = const_required_construction(
        "[if true:
            Visible
        [else]
            Hidden
        ]",
    );

    assert_eq!(
        construction.preparation.outcome,
        TemplatePreparationOutcome::Foldable,
        "a const-required branch over module-local TIR body roots should prepare as foldable"
    );
}

#[test]
fn const_required_template_loop_prepares_a_foldable_view_from_its_body_tir_root() {
    let construction = const_required_construction(
        "[loop 0 to 1 |i|:
            [i]
        ]",
    );

    assert_eq!(
        construction.preparation.outcome,
        TemplatePreparationOutcome::Foldable,
        "a const-required loop over its module-local TIR body root should prepare as foldable"
    );
}

#[cfg(feature = "benchmark_counters")]
#[test]
fn const_required_construction_preparation_is_reused_by_folding() {
    use crate::compiler_frontend::instrumentation::{
        AstCounter, lock_counter_test, reset_ast_counters, test_read_ast_counter,
    };

    let _guard = lock_counter_test();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if true:
            Visible
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let context = new_constant_context(token_stream.src_path.clone());

    reset_ast_counters();
    let construction =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect("const-required template should parse");
    let template = construction.template;
    let prepared = construction.preparation;
    if !matches!(prepared.outcome, TemplatePreparationOutcome::Foldable) {
        panic!("const-required test template should be foldable");
    }

    let mut fold_context = context.new_tir_fold_context(&mut string_table);
    let store = context.template_ir_store.borrow();
    let reference = template.tir_reference;
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )
    .expect("const-required construction view should remain valid for folding");
    // This counter test asserts only the folded text shape.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&prepared, view, &mut fold_context)
            .expect("returned const-required preparation should fold");

    assert!(matches!(emission, TemplateEmission::Output(_)));
    assert_eq!(
        test_read_ast_counter(AstCounter::TirPreparationAttempts),
        1,
        "const-required construction and folding should prepare the view once"
    );
}

#[test]
fn const_required_template_loop_reports_expansion_limit() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to & 10000 |i|:
            [i]
        ]",
    );

    let mut fold_context = context.new_tir_fold_context(&mut string_table);
    let error = fold_template_with_fold_context(
        &template,
        &context.template_ir_store,
        &mut fold_context,
        crate::compiler_frontend::ast::templates::tir::TemplatePreparationMode::ConstRequired,
    )
    .expect_err("const loop should enforce expansion limit");
    let TemplateError::Diagnostic(error) = error else {
        panic!("const loop expansion limit should remain a source diagnostic");
    };

    assert_invalid_template_structure(
        &error,
        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded { limit: 10_000 },
    );
}

#[test]
fn const_required_template_loop_uses_configured_expansion_limit() {
    let (template, context, mut string_table) = parse_const_required_template(
        "[loop 0 to & 10000 |i|:
            [if false:
                hidden
            ]
        ]",
    );

    let mut fold_context = context.new_tir_fold_context(&mut string_table);
    fold_context.template_const_loop_iteration_limit = 10_001;

    let folded = fold_template_with_fold_context(
        &template,
        &context.template_ir_store,
        &mut fold_context,
        crate::compiler_frontend::ast::templates::tir::TemplatePreparationMode::ConstRequired,
    )
    .expect("configured const loop limit should allow the loop");
    drop(fold_context);

    assert_eq!(string_table.resolve(folded), "");
}

#[test]
fn const_required_template_option_capture_present_folds_then_branch() {
    let mut string_table = StringTable::new();
    let context_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let context = new_constant_context(context_scope.clone());

    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let option_string_type_id = type_environment.intern_option(string_type_id);
    let capture_name = string_table.intern("name");
    let capture_path = context_scope.append(capture_name);
    let present_value = Expression::string_slice(
        string_table.intern("Priya"),
        None,
        ValueMode::ImmutableOwned,
    );
    let scrutinee = Expression::coerced(present_value, option_string_type_id);

    let template = const_required_option_capture_template_with_direct_tir(
        scrutinee,
        capture_name,
        capture_path,
        string_type_id,
        &context,
        &mut string_table,
    );

    {
        let store = context.template_ir_store.borrow();
        prepare_const_required_view_directly(&template, &store)
            .expect("present const option capture should validate");
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "Hello Priya");
}

#[test]
fn const_required_template_option_capture_absent_folds_else_branch() {
    let mut string_table = StringTable::new();
    let context_scope =
        InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let context = new_constant_context(context_scope.clone());

    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let capture_name = string_table.intern("name");
    let capture_path = context_scope.append(capture_name);
    let scrutinee = Expression::option_none_with_type_id(
        string_type_id,
        DataType::StringSlice,
        &mut type_environment,
        None,
    );

    let template = const_required_option_capture_template_with_direct_tir(
        scrutinee,
        capture_name,
        capture_path,
        string_type_id,
        &context,
        &mut string_table,
    );

    {
        let store = context.template_ir_store.borrow();
        prepare_const_required_view_directly(&template, &store)
            .expect("absent const option capture should validate");
    }

    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "Guest");
}

#[test]
fn const_required_template_option_capture_inlines_present_source_const() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if maybe_name is |name|:Hello [name]]",
        &mut string_table,
        &mut span_builder,
    );
    let maybe_name = string_table.intern("maybe_name");

    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let option_string_type_id = type_environment.intern_option(string_type_id);
    let present_value = Expression::string_slice(
        string_table.intern("Priya"),
        None,
        ValueMode::ImmutableOwned,
    );
    let declaration = Declaration {
        id: token_stream.src_path.append(maybe_name),
        value: Expression::coerced(present_value, option_string_type_id),
        binding_span: None,
        config_qualifier: None,
    };
    let context = constant_template_context(&token_stream.src_path, &[declaration]);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let template = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect("const-required option capture should inline present source const")
    .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "Hello Priya");
}

#[test]
fn const_required_template_option_capture_inlines_absent_source_const() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if maybe_name is |name|:
            Hello [name]
        [else]
            Guest
        ]",
        &mut string_table,
        &mut span_builder,
    );
    let maybe_name = string_table.intern("maybe_name");

    let mut type_environment = TypeEnvironment::new();
    let string_type_id = type_environment.builtins().string;
    let absent_value = Expression::option_none_with_type_id(
        string_type_id,
        DataType::StringSlice,
        &mut type_environment,
        None,
    );
    let declaration = Declaration {
        id: token_stream.src_path.append(maybe_name),
        value: absent_value,
        binding_span: None,
        config_qualifier: None,
    };
    let context = constant_template_context(&token_stream.src_path, &[declaration]);
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let template = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect("const-required option capture should inline absent source const")
    .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "Guest");
}

#[test]
fn const_required_template_option_capture_reports_runtime_scrutinee_diagnostic() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if maybe_name is |name|: [name]]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = new_constant_context(token_stream.src_path.clone());

    let mut type_environment = TypeEnvironment::new();
    let maybe_name_type_id = type_environment.intern_option(type_environment.builtins().string);
    let maybe_name = string_table.intern("maybe_name");
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

    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let diagnostic = Template::new_const_required_with_type_interner(
        &mut token_stream,
        &context,
        &mut type_interner,
        vec![],
        &mut string_table,
    )
    .expect_err("const-required option capture should be deferred");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
    );
}

#[test]
fn const_required_template_if_rejects_runtime_local_condition() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source(
        "[if show_banner: Visible]",
        &mut string_table,
        &mut span_builder,
    );
    let mut context = new_constant_context(token_stream.src_path.clone());
    let show_banner = string_table.intern("show_banner");
    context.add_var(
        Declaration {
            id: token_stream.src_path.append(show_banner),
            value: Expression::new(
                ExpressionKind::NoValue,
                None,
                builtin_type_ids::BOOL,
                DataType::Bool,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
        None,
    );

    let diagnostic =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table)
            .expect_err("const-required template if should reject runtime local condition");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::TemplateIfConditionNotConst,
    );
}

#[test]
fn const_required_template_if_validates_branch_condition_through_tir_view_overlay() {
    let (mut template, context, mut string_table) = parse_const_required_template(
        "[if true:
            Visible
        ]",
    );

    let mut store = context.template_ir_store.borrow_mut();
    let site_id = find_first_branch_selector_site_id(&template, &store)
        .expect("parsed const-required branch should have a selector site");

    let override_span = synthetic_source_span(99, 103);
    let runtime_condition = Expression::reference_with_type_id(
        InternedPath::from_single_str("runtime_condition", &mut string_table),
        DataType::Bool,
        builtin_type_ids::BOOL,
        Some(override_span),
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    );

    install_expression_overlay_on_template(&mut template, &mut store, site_id, runtime_condition);

    drop(store);
    let store = context.template_ir_store.borrow();
    let error = prepare_const_required_view_directly(&template, &store)
        .expect_err("TirView overlay should make the branch condition non-const");
    let TemplateError::Diagnostic(error) = error else {
        panic!("non-const branch should remain a source diagnostic");
    };

    assert_invalid_template_structure(
        &error,
        InvalidTemplateStructureReason::TemplateIfConditionNotConst,
    );
    assert_eq!(error.primary_span, Some(override_span));
}

#[test]
fn const_required_template_loop_validates_header_through_tir_view_overlay() {
    let (mut template, context, _string_table) = parse_const_required_template(
        "[loop false:
            body
        ]",
    );

    let mut store = context.template_ir_store.borrow_mut();
    let site_id = find_first_loop_header_site_id(&template, &store)
        .expect("parsed const-required conditional loop should have a header site");

    let override_span = synthetic_source_span(99, 103);
    let const_true_condition =
        Expression::bool(true, Some(override_span), ValueMode::ImmutableOwned);

    install_expression_overlay_on_template(
        &mut template,
        &mut store,
        site_id,
        const_true_condition,
    );

    drop(store);
    let store = context.template_ir_store.borrow();
    let error = prepare_const_required_view_directly(&template, &store)
        .expect_err("TirView overlay should turn the conditional loop into const true");
    let TemplateError::Diagnostic(error) = error else {
        panic!("non-const loop should remain a source diagnostic");
    };

    assert_invalid_template_structure(
        &error,
        InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
    );
    assert_eq!(error.primary_span, Some(override_span));
}

#[test]
fn const_required_validation_reports_missing_effective_node_as_internal_error() {
    // A const-required template whose root node has been corrupted to reference
    // a non-existent node must propagate the missing-node error through the
    // internal-error lane instead of silently returning success.
    let (mut template, context, _string_table) = parse_const_required_template("[if true: body]");
    let source_template_id = template.tir_reference.root;

    let malformed_template_id = {
        let store_handle = context.template_ir_store();
        let mut store = store_handle.borrow_mut();
        let mut malformed_template = store
            .get_template(source_template_id)
            .cloned()
            .expect("parsed template should exist in its registered store");
        malformed_template.root = TemplateIrNodeId::new(store.node_count() + 1);
        store.push_template(malformed_template)
    };
    template.tir_reference.root = malformed_template_id;

    let store = context.template_ir_store.borrow();
    let error = prepare_const_required_view_directly(&template, &store)
        .expect_err("missing root node should be an internal error, not a silent success");

    let TemplateError::Infrastructure(error) = error else {
        panic!("missing effective node should remain an infrastructure error");
    };
    assert!(
        error.msg.contains("does not exist in the module store"),
        "error message should mention missing node, got: {}",
        error.msg
    );
}

#[test]
fn const_required_validation_ignores_referenced_child_expression_overlay() {
    // The recursive template first evaluates a structurally const branch, then
    // references itself with an expression overlay that would make the selector
    // runtime-only if imported. Structural transitions deliberately retain the
    // parent expression authority, so the referenced overlay is ignored and the
    // exact child view is a real cycle after the structural selector is checked.
    let (valid_template, context, mut string_table) =
        parse_const_required_template("[if true: body]");

    let valid_branch_root = {
        let store_handle = context.template_ir_store();
        let store = store_handle.borrow();
        store
            .get_template(valid_template.tir_reference.root)
            .expect("parsed const template should exist")
            .root
    };
    let const_context = valid_template.tir_reference.context;

    let non_const_context = {
        let mut store = context.template_ir_store.borrow_mut();
        let site_id = find_first_branch_selector_site_id(&valid_template, &store)
            .expect("child branch should have a selector site");

        let non_const_condition = Expression::reference_with_type_id(
            InternedPath::from_single_str("runtime_condition", &mut string_table),
            DataType::Bool,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        );

        let overlay_id = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(site_id, Box::new(non_const_condition))],
            })
            .expect("test overlay allocation");
        TemplateViewContext {
            expression_overlay: Some(overlay_id),
            slot_resolution: None,
            wrapper_context: None,
        }
    };

    let location = None;
    let recursive_template_id = {
        let store_handle = context.template_ir_store();
        let mut store = store_handle.borrow_mut();
        let recursive_template_id = TemplateIrId::new(store.template_count());
        let recursive_root = recursive_template_id;

        let recursive_child_reference = TemplateTirChildReference::new(
            recursive_root,
            TemplateTirPhase::Finalized,
            non_const_context,
        );

        let mut builder = TemplateIrBuilder::new(&mut store);
        let recursive_child =
            builder.push_child_template_node_with_reference(recursive_child_reference, location);
        let root = builder.push_sequence_node(vec![valid_branch_root, recursive_child], location);
        let built_template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        );
        assert_eq!(
            built_template_id, recursive_template_id,
            "recursive fixture must reference the template allocated next"
        );

        recursive_template_id
    };

    let recursive_template = Template {
        tir_reference: TemplateTirReference {
            root: recursive_template_id,
            phase: TemplateTirPhase::Finalized,
            context: const_context,
        },
        span: None,
    };

    let store = context.template_ir_store.borrow();
    let error = prepare_const_required_view_directly(&recursive_template, &store)
        .expect_err("exact-view child cycles must be CompilerError");
    let TemplateError::Infrastructure(error) = error else {
        panic!("exact-view child cycles must stay on the infrastructure lane, got {error:?}");
    };
    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::Compiler
    );
    assert!(
        error.msg.contains("re-entered while still active"),
        "cycle error should name exact-view re-entry, got: {}",
        error.msg
    );
    assert!(
        !error.msg.contains("runtime_condition"),
        "structural validation must not import the child's expression overlay"
    );
}

#[test]
fn runtime_template_if_allows_unresolved_slot_through_tir_view() {
    let (template, context, _string_table) =
        parse_runtime_template_without_validation("[if true:\n            [$slot]\n        ]");

    let store = context.template_ir_store.borrow();
    validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect("receiver $slot markers may remain inside runtime control flow");
}

#[test]
fn runtime_template_if_rejects_unresolved_insert_through_tir_view() {
    let (template, context, _string_table) = parse_runtime_template_without_validation(
        "[if true:\n            [$insert(\"style\"): color: red;]\n        ]",
    );

    let store = context.template_ir_store.borrow();
    let expected_span = find_first_branch_span(&template, &store)
        .expect("runtime branch should have a stable source span");

    let error = validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect_err("TirView path should report the escaped insert in the branch body");

    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("unresolved runtime insert should remain a source diagnostic");
    };
    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert,
    );
    assert_eq!(diagnostic.primary_span, Some(expected_span));
}

#[test]
fn runtime_template_if_allows_resolved_slot_through_tir_view_overlay() {
    let (mut template, context, _string_table) =
        parse_runtime_template_without_validation("[if true:\n            [$slot]\n        ]");

    let mut store = context.template_ir_store.borrow_mut();
    let (occurrence_id, key) = find_first_slot_occurrence_id(&template, &store)
        .expect("parsed runtime branch body should contain a slot occurrence");

    install_slot_resolution_overlay_on_template(
        &mut template,
        &mut store,
        occurrence_id,
        TirSlotResolution::missing(key.clone()),
    );

    drop(store);
    let store = context.template_ir_store.borrow();
    validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect("resolved slot overlay should suppress the unresolved-slot artifact");
}

#[test]
fn runtime_validation_uses_nested_child_overlay_identity() {
    let (mut child_template, context, _string_table) =
        parse_runtime_template_without_validation("[if true:\n            [$slot]\n        ]");

    {
        let mut store = context.template_ir_store.borrow_mut();
        let (occurrence_id, key) = find_first_slot_occurrence_id(&child_template, &store)
            .expect("nested runtime child should contain a slot occurrence");
        install_slot_resolution_overlay_on_template(
            &mut child_template,
            &mut store,
            occurrence_id,
            TirSlotResolution::missing(key),
        );
    }

    let location = None;
    let child_reference = TemplateTirChildReference::new(
        child_template.tir_reference.root,
        child_template.tir_reference.phase,
        child_template.tir_reference.context,
    );
    let parent_template_id = {
        let store_handle = context.template_ir_store();
        let mut store = store_handle.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_node = builder.push_child_template_node_with_reference(child_reference, location);
        let root = builder.push_sequence_node(vec![child_node], location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        )
    };
    let parent_context = TemplateViewContext::default();
    let parent_template = Template {
        tir_reference: TemplateTirReference {
            root: parent_template_id,
            phase: TemplateTirPhase::Finalized,
            context: parent_context,
        },
        span: None,
    };

    let store = context.template_ir_store.borrow();
    validate_runtime_template_control_flow_slot_artifacts(&parent_template, &store)
        .expect("nested child validation should use the child's resolved-slot overlay");
}

#[test]
fn runtime_validation_reports_missing_root_node_as_internal_error() {
    let (mut template, context, _string_table) =
        parse_runtime_template_without_validation("[if true: body]");
    let source_template_id = template.tir_reference.root;

    let malformed_template_id = {
        let store_handle = context.template_ir_store();
        let mut store = store_handle.borrow_mut();
        let mut malformed_template = store
            .get_template(source_template_id)
            .cloned()
            .expect("parsed template should exist in its registered store");
        malformed_template.root = TemplateIrNodeId::new(store.node_count() + 1);
        store.push_template(malformed_template)
    };
    template.tir_reference.root = malformed_template_id;

    let store = context.template_ir_store.borrow();
    let error = validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect_err("missing root node should be an internal error");

    assert_internal_template_error_contains(error, "does not exist in the store");
}

#[test]
fn runtime_validation_reports_missing_view_context_as_internal_error() {
    let (mut template, context, _string_table) =
        parse_runtime_template_without_validation("[if true: body]");
    template.tir_reference.context = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };

    let store = context.template_ir_store.borrow();
    let error = validate_runtime_template_control_flow_slot_artifacts(&template, &store)
        .expect_err("missing view context should be an internal error");

    assert_internal_template_error_contains(error, "expression overlay");
}
