use super::*;

#[test]
fn finalization_fold_composed_tir_root_folds_view_text() {
    let mut string_table = StringTable::new();
    let view_text = string_table.intern("store-backed view");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template = registered_text_template(view_text, context, &template_ir_store, &string_table);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("composed TIR root fold should succeed"),
    );

    assert_eq!(
        folded, view_text,
        "finalization should fold the composed TIR view text"
    );
}

#[test]
fn finalization_normalizes_dynamic_expression_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized dynamic payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let dynamic_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let (template_id, dynamic_node_id, site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (template_id, dynamic_node_id) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let dynamic_node_id = builder.push_dynamic_expression_node(
                dynamic_expression,
                TemplateSegmentOrigin::Body,
                None,
                None,
            );
            let template_id = builder.finish_template(
                dynamic_node_id,
                Style::default(),
                TemplateType::StringFunction,
                TemplateIrSummary::default(),
                None,
            );
            (template_id, dynamic_node_id)
        };

        let site_id = match &store
            .get_node(dynamic_node_id)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected dynamic expression node, got {other:?}"),
        };

        (template_id, dynamic_node_id, site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the dynamic expression overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(site_id)
        .expect("site lookup should be valid")
        .expect("normalized dynamic expression should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );

    let expression_by_node = view
        .effective_expression_for_node(dynamic_node_id)
        .expect("node lookup should be valid")
        .expect("normalized dynamic expression should be visible by node");
    assert!(
        matches!(expression_by_node.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );

    let structural_expression_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(dynamic_node_id)
            .expect("dynamic node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::DynamicExpression { expression, .. }
                if matches!(expression.kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_expression_is_unchanged,
        "Phase 10 dynamic-expression normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_merges_expression_overrides_without_duplicate_sites() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::int(1, None, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        builder.finish_template(
            dynamic_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };
    let site_id = {
        let store = template_ir_store.borrow();
        match &store
            .get_node(store.get_template(template_id).expect("template").root)
            .expect("dynamic node")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected dynamic expression node, got {other:?}"),
        }
    };
    let existing_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                site_id,
                Box::new(Expression::int(2, None, ValueMode::ImmutableOwned)),
            )],
        })
        .expect("test overlay allocation");
    let initial_context = TemplateViewContext {
        expression_overlay: Some(existing_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: initial_context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("normalization should canonicalize the root expression overlay");

    let store = template_ir_store.borrow();
    let overlay_id = template
        .tir_reference
        .context
        .expression_overlay
        .expect("normalization should retain an expression overlay");
    let overlay = store
        .expression_overlay(overlay_id)
        .expect("normalized expression overlay should exist");
    assert_eq!(overlay.overrides.len(), 1);
    assert_eq!(overlay.overrides[0].0, site_id);
    assert!(matches!(
        overlay.overrides[0].1.kind,
        ExpressionKind::Int(2)
    ));
}

#[test]
fn finalization_does_not_mark_parsed_expression_overlay_reference_finalized() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized parsed dynamic payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let dynamic_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node_id = builder.push_dynamic_expression_node(
            dynamic_expression,
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        builder.finish_template(
            dynamic_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should preserve parsed reference identity");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "parsed references may receive expression overlays without becoming finalized views"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Parsed,
        "parsed references are not stable finalization views and must keep their parsed phase"
    );
}

#[test]
fn finalization_uses_durable_phase_for_pre_finalized_descendant_overlay_collection() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let (root_template_id, root_context, child_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let child_dynamic_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_dynamic_expression_node(
                Expression::int(1, None, ValueMode::ImmutableOwned),
                TemplateSegmentOrigin::Body,
                None,
                None,
            )
        };
        let child_site_id = match &store
            .get_node(child_dynamic_node)
            .expect("child dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected child dynamic expression, got {other:?}"),
        };
        let child_template_id = store.push_template(TemplateIr::new(
            child_dynamic_node,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        ));
        let child_expression_overlay = store
            .allocate_expression_overlay(TirExpressionOverlay {
                overrides: vec![(
                    child_site_id,
                    Box::new(Expression::int(2, None, ValueMode::ImmutableOwned)),
                )],
            })
            .expect("child expression overlay should allocate");

        let child_node = {
            let occurrence_id = store.next_child_template_occurrence_id();
            store.push_node(TemplateIrNode::new(
                TemplateIrNodeKind::ChildTemplate {
                    reference: TemplateTirChildReference::new(
                        child_template_id,
                        TemplateTirPhase::Composed,
                        TemplateViewContext {
                            expression_overlay: Some(child_expression_overlay),
                            slot_resolution: None,
                            wrapper_context: None,
                        },
                    ),
                    occurrence_id,
                },
                None,
            ))
        };
        let root = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence {
                children: vec![child_node],
            },
            None,
        ));
        let root_template_id = store.push_template(TemplateIr::new(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        ));

        (
            root_template_id,
            TemplateViewContext::default(),
            child_site_id,
        )
    };
    let mut template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Parsed,
            context: root_context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("pre-finalized descendant overlays should normalize");

    assert_eq!(template.tir_reference.phase, TemplateTirPhase::Parsed);
    let store = template_ir_store.borrow();
    let view = TirView::new(
        &store,
        template.tir_reference.root,
        template.tir_reference.phase,
        template.tir_reference.context,
    )
    .expect("the parsed reference should retain its exact durable identity");
    let effective_expression = view
        .effective_expression_for_site(child_site_id)
        .expect("child expression site should remain valid")
        .expect("normalization should publish the descendant overlay");
    assert!(matches!(effective_expression.kind, ExpressionKind::Int(2)));
}

#[test]
fn finalization_normalizes_branch_selector_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized branch selector payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let selector_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let (template_id, branch_chain_node_id, selector_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let branch_body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            None,
        ));
        let selector_site_id = store.next_expression_site_id();
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(selector_expression),
            branch_body,
            None,
            selector_site_id,
        );
        let branch_chain_node_id = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::BranchChain {
                branches: vec![branch],
                fallback: None,
                else_marker: None,
            },
            None,
        ));
        let template_id = store.push_template(TemplateIr::new(
            branch_chain_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        ));

        (template_id, branch_chain_node_id, selector_site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the branch selector overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(selector_site_id)
        .expect("site lookup should be valid")
        .expect("normalized branch selector should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );

    let structural_selector_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(branch_chain_node_id)
            .expect("branch chain node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::BranchChain { branches, .. }
                if matches!(branches[0].condition_expression().kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_selector_is_unchanged,
        "Phase 10 branch-selector normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_normalizes_loop_header_payloads_into_expression_overlay() {
    let mut string_table = StringTable::new();
    let normalized_text = string_table.intern("normalized loop header payload");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let header_expression = Expression::template(
        registered_text_template(normalized_text, context, &template_ir_store, &string_table),
        ValueMode::ImmutableOwned,
    );
    let (template_id, loop_node_id, condition_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let loop_body = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children: vec![] },
            None,
        ));
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(header_expression),
        };
        let header_sites = store.allocate_loop_header_expression_sites(&header);
        let condition_site_id = match header_sites {
            TemplateLoopHeaderExpressionSites::Conditional { condition } => condition,
            _ => panic!("expected conditional loop header sites"),
        };
        let loop_node_id = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Loop {
                header,
                header_sites,
                body: loop_body,
                aggregate_wrapper: None,
            },
            None,
        ));
        let template_id = store.push_template(TemplateIr::new(
            loop_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        ));

        (template_id, loop_node_id, condition_site_id)
    };

    let mut template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_template_for_hir(&mut template, &mut context)
        .expect("template normalization should install the loop header overlay");

    let reference = &template.tir_reference;
    assert_ne!(
        reference.context,
        TemplateViewContext::default(),
        "normalization should update the template reference with an expression overlay"
    );
    assert_eq!(
        reference.phase,
        TemplateTirPhase::Finalized,
        "normalization should advance the effective reference to the finalized phase"
    );

    let store = template_ir_store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Finalized,
        reference.context,
    )
    .expect("updated template reference should build a finalized TirView");

    let expression_by_site = view
        .effective_expression_for_site(condition_site_id)
        .expect("site lookup should be valid")
        .expect("normalized loop header expression should be visible by site");
    assert!(
        matches!(expression_by_site.kind, ExpressionKind::StringSlice(text) if text == normalized_text)
    );

    let structural_header_is_unchanged = {
        let store = template_ir_store.borrow();
        let node = store
            .get_node(loop_node_id)
            .expect("loop node should remain in the structural store");
        matches!(
            &node.kind,
            TemplateIrNodeKind::Loop {
                header: TemplateLoopHeader::Conditional { condition },
                ..
            } if matches!(condition.kind, ExpressionKind::Template(_))
        )
    };
    assert!(
        structural_header_is_unchanged,
        "Phase 10 loop-header normalization should layer the normalized payload through an overlay"
    );
}

#[test]
fn finalization_fold_uses_finalized_expression_overlay_view() {
    #[cfg(feature = "benchmark_counters")]
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let mut string_table = StringTable::new();
    let structural_text = string_table.intern("structural dynamic payload");
    let overlay_text = string_table.intern("finalized expression overlay");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (template_id, dynamic_node) = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::string_slice(structural_text, None, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        let root = builder.push_sequence_node(vec![dynamic_node], None);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );

        (template_id, dynamic_node)
    };

    let site_id = {
        let store = template_ir_store.borrow();
        match &store
            .get_node(dynamic_node)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected dynamic expression node"),
        }
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                site_id,
                Box::new(Expression::string_slice(
                    overlay_text,
                    None,
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let expression_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let context = empty_context.merge(expression_context);

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        None,
    );

    #[cfg(feature = "benchmark_counters")]
    reset_ast_counters();

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("expression-overlay view fold should succeed"),
    );

    assert_eq!(
        folded, overlay_text,
        "finalized expression overlays must fold from the same effective TirView instead of the structural payload"
    );
}

#[test]
fn finalization_classifies_root_expression_overlay_through_nested_children() {
    let mut string_table = StringTable::new();
    let dynamic_text = string_table.intern("root-overlay-dynamic");
    let branch_text = string_table.intern("root-overlay-branch");
    let loop_text = string_table.intern("root-overlay-loop");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (root_template_id, dynamic_site_id, selector_site_id, loop_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (leaf_template_id, dynamic_node, branch_node, loop_node) = {
            let mut builder = TemplateIrBuilder::new(&mut store);

            let dynamic_node = builder.push_dynamic_expression_node(
                Expression::reference_with_type_id(
                    InternedPath::from_single_str("nested_dynamic", &mut string_table),
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    None,
                    ValueMode::ImmutableReference,
                    ConstRecordState::RuntimeValue,
                ),
                TemplateSegmentOrigin::Body,
                None,
                None,
            );
            let branch_text_node = builder.push_text_node(
                branch_text,
                "root-overlay-branch".len(),
                TemplateSegmentOrigin::Body,
                None,
            );
            let nested_selector_site = builder.store.next_expression_site_id();
            let branch_node = builder.push_branch_chain_node(
                vec![TemplateIrBranch::new(
                    TemplateBranchSelector::Bool(Expression::reference_with_type_id(
                        InternedPath::from_single_str("nested_selector", &mut string_table),
                        DataType::Bool,
                        builtin_type_ids::BOOL,
                        None,
                        ValueMode::ImmutableReference,
                        ConstRecordState::RuntimeValue,
                    )),
                    branch_text_node,
                    None,
                    nested_selector_site,
                )],
                None,
                None,
                None,
            );
            let loop_text_node = builder.push_text_node(
                loop_text,
                "root-overlay-loop".len(),
                TemplateSegmentOrigin::Body,
                None,
            );
            let loop_node = builder.push_loop_node(
                TemplateLoopHeader::Conditional {
                    condition: Box::new(Expression::reference_with_type_id(
                        InternedPath::from_single_str("nested_loop", &mut string_table),
                        DataType::Bool,
                        builtin_type_ids::BOOL,
                        None,
                        ValueMode::ImmutableReference,
                        ConstRecordState::RuntimeValue,
                    )),
                },
                loop_text_node,
                None,
                None,
            );
            let leaf_root =
                builder.push_sequence_node(vec![dynamic_node, branch_node, loop_node], None);
            let leaf_template_id = builder.finish_template(
                leaf_root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            );
            (leaf_template_id, dynamic_node, branch_node, loop_node)
        };

        let dynamic_site_id = match &store
            .get_node(dynamic_node)
            .expect("dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected a dynamic-expression node"),
        };
        let (selector_site_id, loop_site_id) = match &store
            .get_node(branch_node)
            .expect("branch node should exist")
            .kind
        {
            TemplateIrNodeKind::BranchChain { branches, .. } => {
                let selector_site_id = branches[0].selector_site_id;
                let loop_site_id = match &store
                    .get_node(loop_node)
                    .expect("loop node should exist")
                    .kind
                {
                    TemplateIrNodeKind::Loop {
                        header_sites: TemplateLoopHeaderExpressionSites::Conditional { condition },
                        ..
                    } => *condition,
                    _ => panic!("expected a conditional loop node"),
                };
                (selector_site_id, loop_site_id)
            }
            _ => panic!("expected a branch-chain node"),
        };

        let mut builder = TemplateIrBuilder::new(&mut store);
        let mut descendant_template_id = leaf_template_id;
        for _ in 0..3 {
            let child_reference = TemplateTirChildReference::new(
                descendant_template_id,
                TemplateTirPhase::Composed,
                empty_context,
            );
            let child_node = builder.push_child_template_node_with_reference(child_reference, None);
            let root = builder.push_sequence_node(vec![child_node], None);
            descendant_template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            );
        }

        (
            descendant_template_id,
            dynamic_site_id,
            selector_site_id,
            loop_site_id,
        )
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![
                (
                    dynamic_site_id,
                    Box::new(Expression::string_slice(
                        dynamic_text,
                        None,
                        ValueMode::ImmutableOwned,
                    )),
                ),
                (
                    selector_site_id,
                    Box::new(Expression::bool(true, None, ValueMode::ImmutableOwned)),
                ),
                (
                    loop_site_id,
                    Box::new(Expression::bool(false, None, ValueMode::ImmutableOwned)),
                ),
            ],
        })
        .expect("test overlay allocation");
    let root_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Finalized,
            context: root_context,
        },
        None,
    );

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("root overlay should classify and fold through nested descendants"),
    );

    assert_eq!(
        string_table.resolve(folded),
        "root-overlay-dynamicroot-overlay-branch",
        "dynamic, branch-selector, and loop-header overlays must all reach the nested leaf"
    );
}

#[test]
fn finalization_ignores_parsed_child_overlay_before_later_composed_descendant() {
    let mut string_table = StringTable::new();
    let structural_text = string_table.intern("structural");
    let override_text = string_table.intern("root-override");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();
    let missing_context = TemplateViewContext {
        expression_overlay: Some(TirExpressionOverlayId::new(999)),
        ..TemplateViewContext::default()
    };

    let (root_template_id, descendant_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let (descendant_template_id, descendant_site_id) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let dynamic_node = builder.push_dynamic_expression_node(
                Expression::string_slice(structural_text, None, ValueMode::ImmutableOwned),
                TemplateSegmentOrigin::Body,
                None,
                None,
            );
            let root = builder.push_sequence_node(vec![dynamic_node], None);
            let template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            );
            let site_id = match &store
                .get_node(dynamic_node)
                .expect("descendant dynamic node should exist")
                .kind
            {
                TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
                _ => panic!("expected descendant dynamic-expression node"),
            };
            (template_id, site_id)
        };

        let parsed_child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    descendant_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                None,
            );
            let root = builder.push_sequence_node(vec![child_node], None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            )
        };

        let mut builder = TemplateIrBuilder::new(&mut store);
        let child_node = builder.push_child_template_node_with_reference(
            TemplateTirChildReference::new(
                parsed_child_template_id,
                TemplateTirPhase::Parsed,
                missing_context,
            ),
            None,
        );
        let root = builder.push_sequence_node(vec![child_node], None);
        let root_template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );

        (root_template_id, descendant_site_id)
    };

    let expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                descendant_site_id,
                Box::new(Expression::string_slice(
                    override_text,
                    None,
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let root_context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let template = template_with_reference(
        TemplateTirReference {
            root: root_template_id,
            phase: TemplateTirPhase::Finalized,
            context: root_context,
        },
        None,
    );

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("a Parsed child must not consume its missing overlay during finalization"),
    );

    assert_eq!(
        folded, override_text,
        "the finalized root expression overlay must reach the later composed descendant"
    );
}

#[test]
fn finalization_rejects_nested_runtime_wrapper_in_exact_wrapper_overlay() {
    let mut string_table = StringTable::new();
    let (template, template_ir_store) =
        nested_wrapper_finalization_fixture(&mut string_table, true);

    let result = finalize_template_value(
        &template,
        TemplateValueFinalizationInputs {
            string_table: &mut string_table,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            template_ir_store: &template_ir_store,
        },
        TemplatePreparationMode::Value,
    )
    .expect("runtime nested wrapper should be a valid non-foldable shape");

    assert!(
        matches!(result, FinalizedTemplateValue::Runtime(_)),
        "the production safety gate must not fold through a runtime nested wrapper hidden in the exact wrapper overlay"
    );
}

#[test]
fn finalization_keeps_valid_runtime_slot_plan_out_of_folded_string() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime root");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            span: None,
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let result = finalize_template_value(
        &template,
        TemplateValueFinalizationInputs {
            string_table: &mut string_table,
            template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
            template_ir_store: &template_ir_store,
        },
        TemplatePreparationMode::Value,
    )
    .expect("valid runtime slot plan should use the handoff path");

    assert!(
        matches!(result, FinalizedTemplateValue::Runtime(_)),
        "a valid runtime slot plan must not become a folded empty string"
    );
    assert!(
        template_ir_store
            .borrow()
            .get_template(template_id)
            .expect("template should remain in the store")
            .runtime_slot_plan
            .is_some(),
        "the runtime slot plan must remain available for owned handoff"
    );
}

#[test]
fn finalization_replaces_renderable_runtime_slot_plan_with_owned_handoff() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime handoff");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            span: None,
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("renderable runtime slot plans should use the owned handoff path");

    let ExpressionKind::RuntimeSlotApplicationHandoff(handoff) = expression.kind else {
        panic!("expected renderable runtime slot plan to become an owned slot handoff");
    };
    assert!(
        handoff.slot_sites.is_empty(),
        "the owned handoff must retain the valid empty slot plan"
    );
    assert!(
        template_ir_store
            .borrow()
            .get_template(template_id)
            .expect("template should remain in the store")
            .runtime_slot_plan
            .is_some(),
        "normalization must retain the source runtime slot plan"
    );
}

#[test]
fn runtime_handoff_shape_uses_root_slot_plan_not_preparation_reason() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("runtime slot root");
    let mut template = registered_text_template(text, context, &template_ir_store, &string_table);
    template.tir_reference.phase = TemplateTirPhase::Finalized;
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            span: None,
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };
    let store = template_ir_store.borrow();
    let view = TirView::new(
        &store,
        template_id,
        TemplateTirPhase::Finalized,
        TemplateViewContext::default(),
    )
    .expect("finalized runtime-slot view should construct");
    let prepared = prepare_tir_view(&view, TemplatePreparationMode::Value)
        .expect("runtime-slot preparation should succeed");
    assert!(matches!(
        prepared.outcome,
        TemplatePreparationOutcome::Runtime(_)
    ));

    let normalized = super::materialize_runtime_template_handoff_for_hir(
        &template,
        &mut context,
        &prepared,
        None,
    )
    .expect("prepared runtime handoff should materialize")
    .expect("runtime template should produce a normalized handoff");

    assert!(
        matches!(
            normalized,
            NormalizedTemplateExpression::RuntimeSlotApplication(..)
        ),
        "the actual root slot plan must select the specialized runtime-slot handoff shape"
    );
}

#[test]
fn module_constant_normalization_rejects_runtime_slot_plan_with_structured_diagnostic() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("module constant runtime plan");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);
    let template_id = template.tir_reference.root;

    {
        let mut store = template_ir_store.borrow_mut();
        let slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
            span: None,
        });
        store
            .attach_runtime_slot_plan(template_id, slot_plan_id)
            .expect("template should accept the committed slot plan");
    }

    let expression = Expression::template(template, ValueMode::ImmutableOwned);
    let ExpressionKind::Template(template) = &expression.kind else {
        panic!("module constant regression must start from a template expression");
    };
    let projected = project_const_template_value(
        template,
        &template_ir_store.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("a runtime slot plan must classify rather than fail preparation");
    let Err(result) = const_template_value_from_projection(projected, template) else {
        panic!("runtime-plan module constants must be rejected structurally");
    };

    let ConstValueStoreError::Diagnostic(diagnostic) = result else {
        panic!(
            "runtime-plan module constants must not report the old internal fold transformation error"
        );
    };
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::NonFoldableConstTemplate,
        }
    ));
}

#[test]
fn finalization_accepts_supported_nested_wrapper_exact_view() {
    let mut string_table = StringTable::new();
    let (template, template_ir_store) =
        nested_wrapper_finalization_fixture(&mut string_table, false);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("supported nested wrapper should fold through the exact views"),
    );

    assert_eq!(
        string_table.resolve(folded),
        "outer-structuralinner-beforenestedinner-afterparentouter-after",
        "supported exact-view wrapper traversal must preserve structural expression authority and wrapper order"
    );
}

#[test]
fn finalization_fold_uses_resolved_slot_view_context() {
    let mut string_table = StringTable::new();
    let before_text = string_table.intern("before");
    let after_text = string_table.intern("after");
    let fill_text = string_table.intern("filled");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let reference = {
        let mut store = template_ir_store.borrow_mut();
        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_node = fill_builder.push_text_node(
            fill_text,
            "filled".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let fill_root = fill_builder.push_sequence_node(vec![fill_node], None);
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let before_node = wrapper_builder.push_text_node(
            before_text,
            "before".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let slot_node = wrapper_builder.push_slot_node(SlotKey::Default, None);
        let after_node = wrapper_builder.push_text_node(
            after_text,
            "after".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let wrapper_root =
            wrapper_builder.push_sequence_node(vec![before_node, slot_node, after_node], None);
        let wrapper_template_id = wrapper_builder.finish_template(
            wrapper_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );

        let slot_occurrence_id = match &store
            .get_node(slot_node)
            .expect("slot node should exist")
            .kind
        {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
            _ => panic!("expected slot node"),
        };

        let slot_overlay_id = store
            .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
                resolutions: vec![(
                    slot_occurrence_id,
                    TirSlotResolution::resolved(SlotKey::Default, vec![fill_template_id]),
                )],
            })
            .expect("test overlay allocation");
        let context = TemplateViewContext {
            expression_overlay: None,
            slot_resolution: Some(slot_overlay_id),
            wrapper_context: None,
        };
        assert!(
            context.slot_resolution.is_some(),
            "test must exercise a real slot-resolution overlay"
        );

        TemplateTirReference {
            root: wrapper_template_id,
            phase: TemplateTirPhase::Composed,
            context,
        }
    };

    let template = template_with_reference(reference, None);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("resolved slot-overlay fold should succeed"),
    );

    let expected = string_table.intern("beforefilledafter");
    assert_eq!(
        folded, expected,
        "composed slot overlays must fold from the effective TirView"
    );
}

#[test]
fn finalization_fold_composed_root_with_unfilled_slot_emits_no_slot_output() {
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("text before unfilled slot");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // An unfilled slot contributes no output. Finalization folds that rule
    // directly from the composed TIR root.
    let reference = {
        let location = None;
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text_id,
            "text before unfilled slot".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location);
        let root = builder.push_sequence_node(vec![text_node, slot_node], location);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        );

        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        }
    };

    let template = template_with_reference(reference, None);

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("composed slot-root fold should succeed"),
    );

    assert_eq!(
        folded, text_id,
        "the unfilled slot must contribute no output to the composed TIR root"
    );
}

#[test]
fn finalization_fold_formatted_root_with_unfilled_slot_emits_no_slot_output() {
    #[cfg(feature = "benchmark_counters")]
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let mut string_table = StringTable::new();
    let text_id = string_table.intern("formatted text before unfilled slot");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let reference = {
        let location = None;
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text_id,
            "formatted text before unfilled slot".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let slot_node = builder.push_slot_node(SlotKey::Default, location);
        let root = builder.push_sequence_node(vec![text_node, slot_node], location);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary {
                slot_count: 1,
                ..TemplateIrSummary::default()
            },
            location,
        );

        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Formatted,
            context,
        }
    };

    let template = template_with_reference(reference, None);

    #[cfg(feature = "benchmark_counters")]
    reset_ast_counters();

    let folded = finalized_folded(
        finalize_template_value(
            &template,
            TemplateValueFinalizationInputs {
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
            },
            TemplatePreparationMode::Value,
        )
        .expect("formatted slot-root fold should succeed"),
    );

    assert_eq!(
        folded, text_id,
        "the unfilled slot must contribute no output to the formatted TIR root"
    );

    #[cfg(feature = "benchmark_counters")]
    {
        assert_eq!(
            test_read_ast_counter(AstCounter::TirFinalizationFoldAttempts),
            1,
            "slot-bearing formatted roots are now real store fold attempts"
        );
        assert_eq!(
            test_read_ast_counter(AstCounter::TirFinalizationFoldSuccesses),
            1,
            "the store-backed fold completes directly"
        );
    }
}
