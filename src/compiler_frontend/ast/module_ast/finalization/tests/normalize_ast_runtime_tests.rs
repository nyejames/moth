use super::*;

fn runtime_template_handoff_from_expression(expression: Expression) -> OwnedRuntimeTemplateHandoff {
    let ExpressionKind::RuntimeTemplateHandoff(handoff) = expression.kind else {
        panic!("expected expression normalization to return an owned runtime-template handoff");
    };

    *handoff
}

#[test]
fn branch_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let location = None;
    let branch_text = string_table.intern("branch body");
    let fallback_text = string_table.intern("fallback body");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let branch_body = builder.push_text_node(
            branch_text,
            "branch body".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let fallback_body = builder.push_text_node(
            fallback_text,
            "fallback body".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::reference_with_type_id(
                InternedPath::from_single_str("show_branch", &mut string_table),
                DataType::Bool,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )),
            branch_body,
            location,
            builder.store.next_expression_site_id(),
        );
        let root =
            builder.push_branch_chain_node(vec![branch], Some(fallback_body), None, location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            location,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("branch TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::BranchChain {
        branches,
        fallback,
        ..
    }) = handoff.body
    else {
        panic!("expected a branch-chain runtime handoff");
    };
    assert_eq!(branches.len(), 1);
    assert!(
        fallback.is_some(),
        "the fallback must remain owned by the handoff"
    );
}

#[test]
fn loop_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let location = None;
    let loop_text = string_table.intern("loop body");
    let open_text = string_table.intern("[");
    let close_text = string_table.intern("]");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let aggregate_output = store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::AggregateOutput,
            location,
        ));
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_text_node(
            loop_text,
            "loop body".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let open = builder.push_text_node(open_text, 1, TemplateSegmentOrigin::Body, location);
        let close = builder.push_text_node(close_text, 1, TemplateSegmentOrigin::Body, location);
        let aggregate_wrapper =
            builder.push_sequence_node(vec![open, aggregate_output, close], location);
        let header = TemplateLoopHeader::Conditional {
            condition: Box::new(Expression::reference_with_type_id(
                InternedPath::from_single_str("keep_looping", &mut string_table),
                DataType::Bool,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )),
        };
        let root = builder.push_loop_node(header, body, Some(aggregate_wrapper), location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            location,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("loop TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop { .. }) = handoff.body
    else {
        panic!("expected a loop runtime handoff");
    };
}

fn collect_owned_handoff_string_slice_expressions(
    handoff: &OwnedRuntimeTemplateHandoff,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match &handoff.body {
        OwnedRuntimeTemplateBody::Render(root) => {
            collect_owned_node_string_slice_expressions(root, string_slices);
        }

        OwnedRuntimeTemplateBody::RuntimeSlotApplication(slot_handoff) => {
            collect_owned_node_string_slice_expressions(&slot_handoff.wrapper, string_slices);
            for source in &slot_handoff.contribution_sources {
                collect_owned_node_string_slice_expressions(&source.render_root, string_slices);
            }
            for site in &slot_handoff.slot_sites {
                collect_owned_node_string_slice_expressions(&site.render_root, string_slices);
            }
        }
    }
}

fn collect_owned_node_string_slice_expressions(
    node: &OwnedRuntimeTemplateNode,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if let ExpressionKind::StringSlice(text) = &expression.kind {
                string_slices.push(*text);
            }
        }

        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                collect_owned_node_string_slice_expressions(child, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                collect_owned_node_string_slice_expressions(&branch.body, string_slices);
            }
            if let Some(fallback) = fallback {
                collect_owned_node_string_slice_expressions(fallback, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_owned_node_string_slice_expressions(body, string_slices);
            if let Some(wrapper) = aggregate_wrapper {
                collect_owned_node_string_slice_expressions(wrapper, string_slices);
            }
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            collect_owned_handoff_string_slice_expressions(template, string_slices);
        }

        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            collect_owned_node_string_slice_expressions(child, string_slices);
            collect_owned_node_string_slice_expressions(wrapper, string_slices);
        }

        OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }
}

/// Builds a `Template` with a registered TIR root containing a text segment and
/// a runtime reference expression, matching the production shape for ordinary
/// runtime templates that are not const-foldable.
///
/// WHAT: the resulting template is not const-foldable because the reference is
///       a runtime value, so it must go through the runtime-template handoff path.
/// WHY: gives the new store-focused test a simple, representative input shape.
fn registered_runtime_template(
    text: crate::compiler_frontend::symbols::string_interning::StringId,
    reference_name: &str,
    context: TemplateViewContext,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Template {
    let byte_len = string_table.resolve(text).len();
    let reference_path = InternedPath::from_single_str(reference_name, string_table);
    let reference_expression = Expression::reference_with_type_id(
        reference_path,
        DataType::StringSlice,
        builtin_type_ids::STRING,
        None,
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    );
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(text, byte_len, TemplateSegmentOrigin::Body, None);
        let dynamic_node = builder.push_dynamic_expression_node(
            reference_expression,
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        let root = builder.push_sequence_node(vec![text_node, dynamic_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };
    template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    )
}

#[test]
fn ordinary_runtime_template_handoff_uses_module_tir_store() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template =
        registered_runtime_template(text, "name", context, &template_ir_store, &mut string_table);

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("ordinary runtime template expression normalization should succeed");

    let handoff = runtime_template_handoff_from_expression(expression);
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must materialize a render body handoff"
    );
}

#[test]
fn folded_template_preserves_selected_effective_dynamic_provenance() {
    let mut string_table = StringTable::new();
    let unselected_text = string_table.intern("unselected");
    let selected_structural_text = string_table.intern("selected structural");
    let selected_effective_text = string_table.intern("selected effective");
    let location = None;
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let unselected_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "unselected",
    );
    let selected_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "selected",
    );

    let (template_id, selected_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let unselected_node = builder.push_dynamic_expression_node(
            Expression::string_slice(unselected_text, location, ValueMode::ImmutableOwned)
                .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
                    unselected_member,
                )),
            TemplateSegmentOrigin::Body,
            None,
            location,
        );
        let selected_node = builder.push_dynamic_expression_node(
            Expression::string_slice(
                selected_structural_text,
                location,
                ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            location,
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                location,
                ValueMode::ImmutableOwned,
            )),
            unselected_node,
            location,
            builder.store.next_expression_site_id(),
        );
        let root =
            builder.push_branch_chain_node(vec![branch], Some(selected_node), None, location);
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );
        let selected_site_id = match store
            .get_node(selected_node)
            .expect("selected dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => site_id,
            _ => panic!("expected selected dynamic expression node"),
        };
        (template_id, selected_site_id)
    };

    let overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                selected_site_id,
                Box::new(
                    Expression::string_slice(
                        selected_effective_text,
                        None,
                        ValueMode::ImmutableOwned,
                    )
                    .with_synthetic_interface_provenance(
                        SyntheticInterfaceProvenance::single(selected_member.clone()),
                    ),
                ),
            )],
        })
        .expect("test overlay allocation");
    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext {
                expression_overlay: Some(overlay_id),
                ..TemplateViewContext::default()
            },
        },
        None,
    );
    let constant_expression = Expression::template(template.clone(), ValueMode::ImmutableOwned);
    assert!(
        constant_expression
            .synthetic_interface_provenance
            .is_empty(),
        "the outer template must start without injected provenance"
    );

    let ExpressionKind::Template(constant_template) = &constant_expression.kind else {
        panic!("module constant regression must start from a template expression");
    };
    let projected_constant = project_const_template_value(
        constant_template,
        &template_ir_store.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("selected exact TIR fold should project module constants");
    assert_eq!(
        projected_constant.provenance.members(),
        std::slice::from_ref(&selected_member),
        "module constant projection must retain selected folded provenance"
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("selected exact TIR fold should normalize");

    assert!(matches!(
        expression.kind,
        ExpressionKind::StringSlice(value) if value == selected_effective_text
    ));
    assert_eq!(
        expression.synthetic_interface_provenance.members(),
        &[selected_member],
        "only the selected effective dynamic payload may reach the folded value"
    );
}

#[test]
fn runtime_template_expression_normalization_replaces_template_with_owned_handoff() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template =
        registered_runtime_template(text, "name", context, &template_ir_store, &mut string_table);

    // This test covers preservation of metadata already carried by the outer runtime template
    // expression. Nested effective-fold provenance is covered by the folded-template test above.
    let provenance_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            provenance_member.clone(),
        ));

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template expression normalization should succeed");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must keep using the render handoff body"
    );
    assert_eq!(expression.diagnostic_type, DataType::Template);
    assert_eq!(expression.value_mode, ValueMode::ImmutableOwned);
    assert_eq!(
        expression.synthetic_interface_provenance.members(),
        &[provenance_member]
    );
    assert!(
        expression
            .reactive_template
            .as_ref()
            .is_some_and(|metadata| metadata.template_backed),
        "runtime handoff expressions must preserve template-backed metadata"
    );
}

#[test]
fn runtime_template_expression_handoff_uses_finalized_expression_overlay_view() {
    let mut string_table = StringTable::new();
    let overlay_text = string_table.intern("normalized overlay text");
    let runtime_path = InternedPath::from_single_str("runtime_name", &mut string_table);

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();
    let nested_template_expression = Expression::template(
        registered_text_template(
            overlay_text,
            empty_context,
            &template_ir_store,
            &string_table,
        ),
        ValueMode::ImmutableOwned,
    );

    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let normalized_dynamic_node = builder.push_dynamic_expression_node(
            nested_template_expression,
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        let runtime_dynamic_node = builder.push_dynamic_expression_node(
            Expression::reference_with_type_id(
                runtime_path,
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
        let root =
            builder.push_sequence_node(vec![normalized_dynamic_node, runtime_dynamic_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: empty_context,
        },
        None,
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template normalization should use the finalized view handoff");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };

    let mut string_slices = Vec::new();
    collect_owned_handoff_string_slice_expressions(handoff, &mut string_slices);
    assert!(
        string_slices.contains(&overlay_text),
        "runtime handoff must materialize normalized dynamic expressions from the final effective TirView"
    );
    assert!(
        expression.reactive_template.is_some(),
        "runtime handoff replacement should preserve template metadata"
    );
}

/// Proves that a nested runtime template inside a TIR dynamic expression node
/// is normalized through the final effective view.
#[test]
fn nested_runtime_template_normalizes_through_final_view() {
    let mut string_table = StringTable::new();
    let nested_text = string_table.intern("nested runtime text");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR whose sole dynamic expression holds a nested runtime
    // template (text plus a runtime reference, so it is not const-foldable).
    let nested_template = registered_runtime_template(
        nested_text,
        "runtime_ref",
        context,
        &template_ir_store,
        &mut string_table,
    );

    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::template(nested_template, ValueMode::ImmutableOwned),
            TemplateSegmentOrigin::Body,
            None,
            None,
        );
        let root = builder.push_sequence_node(vec![dynamic_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("nested runtime template normalization should succeed through the final TIR view");

    let ExpressionKind::RuntimeTemplateHandoff(handoff) = &expression.kind else {
        panic!("outer template expression should be replaced with an owned handoff");
    };

    // The handoff must contain the nested runtime template handoff inside a
    // DynamicExpression node, proving the overlay path normalized it.
    let mut found_nested_handoff = false;
    if let OwnedRuntimeTemplateBody::Render(root) = &handoff.body {
        find_runtime_handoff_in_node(root, &mut found_nested_handoff);
    }
    assert!(
        found_nested_handoff,
        "handoff must contain the nested runtime template handoff materialized from the final TIR view"
    );
}

/// Recursively checks whether any DynamicExpression node in the owned handoff
/// tree carries a RuntimeTemplateHandoff expression kind.
fn find_runtime_handoff_in_node(node: &OwnedRuntimeTemplateNode, found: &mut bool) {
    if *found {
        return;
    }
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if matches!(expression.kind, ExpressionKind::RuntimeTemplateHandoff(_)) {
                *found = true;
            }
        }
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                find_runtime_handoff_in_node(child, found);
            }
        }
        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                find_runtime_handoff_in_node(&branch.body, found);
            }
            if let Some(fallback) = fallback {
                find_runtime_handoff_in_node(fallback, found);
            }
        }
        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            find_runtime_handoff_in_node(body, found);
            if let Some(wrapper) = aggregate_wrapper {
                find_runtime_handoff_in_node(wrapper, found);
            }
        }
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            find_runtime_handoff_in_node(child, found);
            find_runtime_handoff_in_node(wrapper, found);
        }
        _ => {}
    }
}

/// Proves that a const child template referenced from the outer TIR view folds
/// correctly through the final view.
#[test]
fn nested_const_template_folds_through_final_view() {
    let mut string_table = StringTable::new();
    let child_text_str = "child folded text";
    let child_text = string_table.intern(child_text_str);
    let child_byte_len = child_text_str.len();

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a child template (const text) and an outer template whose TIR
    // root is a sequence containing a child-template ref to it.
    let outer_template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let child_root = builder.push_text_node(
            child_text,
            child_byte_len,
            TemplateSegmentOrigin::Body,
            None,
        );
        let child_id = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        );

        let child_ref_node = builder.push_child_template_node(child_id, None);
        let outer_root = builder.push_sequence_node(vec![child_ref_node], None);
        builder.finish_template(
            outer_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: outer_template_id,
            phase: TemplateTirPhase::Composed,
            context,
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
        .expect("fold through final view should succeed"),
    );

    assert_eq!(
        folded, child_text,
        "fold must produce the child template's text from the final TIR view"
    );
}

/// Proves that reactive subscriptions stored on TIR dynamic expression nodes
/// are collected into the expression's reactive metadata through the finalized
/// effective view.
#[test]
fn reactive_metadata_derived_from_nested_final_view() {
    let mut string_table = StringTable::new();
    let reactive_path = InternedPath::from_single_str("reactive_source", &mut string_table);

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR with a dynamic expression carrying a reactive subscription.
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let subscription = ReactiveSubscription {
            source: ReactiveSource {
                path: reactive_path.clone(),
                kind: ReactiveSourceKind::Declaration,
            },
            type_id: builtin_type_ids::STRING,
            span: None,
        };

        let dynamic_node = builder.push_dynamic_expression_node(
            Expression::reference_with_type_id(
                reactive_path.clone(),
                DataType::StringSlice,
                builtin_type_ids::STRING,
                None,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            ),
            TemplateSegmentOrigin::Body,
            Some(subscription),
            None,
        );
        let root = builder.push_sequence_node(vec![dynamic_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );
    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("reactive template normalization should succeed");

    let metadata = expression
        .reactive_template
        .as_ref()
        .expect("runtime handoff replacement should preserve reactive template metadata");

    assert!(
        metadata.template_backed,
        "reactive metadata should be template-backed"
    );
    assert!(
        metadata.subscriptions.iter().any(|sub| {
            sub.source.path == reactive_path
                && matches!(sub.source.kind, ReactiveSourceKind::Declaration)
        }),
        "reactive metadata must contain the subscription from the final TIR view"
    );
}

/// Pins the finalizer seam before durable reactive handoff annotation can repair it.
#[test]
fn selected_static_candidate_carries_annotated_context_into_runtime_handoff() {
    use super::super::super::reactive_templates::propagate_reactive_template_metadata_in_ast;
    use super::super::super::static_if_specialization::StaticIfCandidate;

    let mut string_table = StringTable::new();
    let function_path = InternedPath::from_single_str("render_count", &mut string_table);
    let parameter_path = InternedPath::from_single_str("source", &mut string_table);
    let active_source_path = InternedPath::from_single_str("count", &mut string_table);
    let inactive_source_path = InternedPath::from_single_str("inactive", &mut string_table);
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));

    let template_expression =
        |store: &mut TemplateIrStore,
         expression: Expression,
         subscription: Option<ReactiveSubscription>| {
            let location = None;
            let mut builder = TemplateIrBuilder::new(store);
            let dynamic = builder.push_dynamic_expression_node(
                expression,
                TemplateSegmentOrigin::Body,
                subscription,
                location,
            );
            let root = builder.push_sequence_node(vec![dynamic], location);
            let template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::empty(),
                location,
            );
            Expression::template(
                template_with_reference(
                    TemplateTirReference {
                        root: template_id,
                        phase: TemplateTirPhase::Composed,
                        context: TemplateViewContext::default(),
                    },
                    location,
                ),
                ValueMode::ImmutableOwned,
            )
        };

    let mut parameter = Declaration {
        id: parameter_path.clone(),
        value: Expression::no_value_with_type_id(
            None,
            DataType::Int,
            builtin_type_ids::INT,
            ValueMode::ImmutableReference,
        ),
        binding_span: None,
        config_qualifier: None,
    };
    parameter.value.reactive_source = Some(ReactiveSource {
        path: parameter_path.clone(),
        kind: ReactiveSourceKind::Parameter,
    });

    let parameter_reference = Expression::reference_with_type_id(
        parameter_path.clone(),
        DataType::Int,
        builtin_type_ids::INT,
        None,
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    )
    .with_reactive_source(ReactiveSource {
        path: parameter_path.clone(),
        kind: ReactiveSourceKind::Parameter,
    });
    let function_template = template_expression(
        &mut template_ir_store.borrow_mut(),
        parameter_reference,
        Some(ReactiveSubscription {
            source: ReactiveSource {
                path: parameter_path,
                kind: ReactiveSourceKind::Parameter,
            },
            type_id: builtin_type_ids::INT,
            span: None,
        }),
    );
    let function = fixture_function_node(
        function_path.clone(),
        FunctionSignature {
            parameters: vec![parameter],
            returns: vec![ReturnSlot {
                value: DataType::StringSlice,
                type_id: Some(builtin_type_ids::STRING),
                reactive_template: None,
                channel: ReturnChannel::Success,
            }],
        },
        vec![node(NodeKind::Return(vec![function_template]), None)],
        None,
    );

    let active_argument = Expression::reference_with_type_id(
        active_source_path.clone(),
        DataType::Int,
        builtin_type_ids::INT,
        None,
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    )
    .with_reactive_source(ReactiveSource {
        path: active_source_path.clone(),
        kind: ReactiveSourceKind::Declaration,
    });
    let mut type_environment = TypeEnvironment::new();
    let active_call = Expression::function_call_with_typed_arguments(
        function_path,
        vec![CallArgument::positional(
            active_argument,
            CallAccessMode::Shared,
            None,
        )],
        vec![builtin_type_ids::STRING],
        &mut type_environment,
        None,
    );
    let active_template =
        template_expression(&mut template_ir_store.borrow_mut(), active_call, None);

    let inactive_reference = Expression::reference_with_type_id(
        inactive_source_path.clone(),
        DataType::Int,
        builtin_type_ids::INT,
        None,
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    );
    let inactive_template = template_expression(
        &mut template_ir_store.borrow_mut(),
        inactive_reference,
        Some(ReactiveSubscription {
            source: ReactiveSource {
                path: inactive_source_path.clone(),
                kind: ReactiveSourceKind::Declaration,
            },
            type_id: builtin_type_ids::INT,
            span: None,
        }),
    );

    let mut ast = vec![
        function,
        node(
            NodeKind::If(
                Expression::bool(true, None, ValueMode::ImmutableOwned),
                vec![node(NodeKind::ExpressionStatement(active_template), None)],
                Some(vec![node(
                    NodeKind::ExpressionStatement(inactive_template),
                    None,
                )]),
                test_if_branch_metadata(true),
            ),
            None,
        ),
    ];

    let mut candidate = StaticIfCandidate::prepare(
        &ast,
        &ConstValueStore::default(),
        Rc::clone(&template_ir_store),
        &mut string_table,
    )
    .expect("static candidate specialization should succeed");
    assert!(candidate.has_selections());

    propagate_reactive_template_metadata_in_ast(
        candidate.ast_mut(),
        &mut template_ir_store.borrow_mut(),
    )
    .expect("candidate reactive propagation should succeed");

    let mut normalization_context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store,
        module_resources: None,
    };
    for ast_node in candidate.ast_mut() {
        normalize_ast_node_templates(ast_node, &mut normalization_context)
            .expect("candidate normalization should succeed");
    }

    candidate.publish(&mut ast);

    let NodeKind::LexicalScope { body } = &ast[1].kind else {
        panic!("known Bool should publish one active lexical scope");
    };
    let NodeKind::ExpressionStatement(expression) = &body[0].kind else {
        panic!("selected body should retain its template expression");
    };
    assert!(matches!(
        expression.kind,
        ExpressionKind::RuntimeTemplateHandoff(_)
    ));
    let metadata = expression
        .reactive_template
        .as_ref()
        .expect("handoff must retain candidate metadata before durable publication");
    assert!(metadata.subscriptions.iter().any(|subscription| {
        subscription.source.path == active_source_path
            && subscription.source.kind == ReactiveSourceKind::Declaration
    }));
    assert!(
        metadata
            .subscriptions
            .iter()
            .all(|subscription| subscription.source.path != inactive_source_path)
    );
}

/// Proves that a slot-insert helper artifact surviving composition is rejected
/// after final view traversal, not silently passed to HIR.
#[test]
fn helper_artifact_rejected_after_final_view_traversal() {
    let mut string_table = StringTable::new();
    let text = string_table.intern("slot insert content");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    // Build a TIR root with simple text. The template kind is SlotInsert,
    // which finalization must reject as a helper artifact.
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(
            text,
            "slot insert content".len(),
            TemplateSegmentOrigin::Body,
            None,
        );
        let root = builder.push_sequence_node(vec![text_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::SlotInsert(SlotKey::Default),
            TemplateIrSummary::default(),
            None,
        )
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    );

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    let result = normalize_expression_templates(&mut expression, &mut context);
    assert!(
        result.is_err(),
        "slot-insert helper artifact must be rejected after final view traversal"
    );

    let TemplateNormalizationError::Diagnostic(diagnostic) =
        result.expect_err("error was asserted above")
    else {
        panic!(
            "helper artifact rejection should produce a diagnostic, not an infrastructure error"
        );
    };
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidTemplateStructure {
                reason: InvalidTemplateStructureReason::HelperOutsideWrapperSlot
            }
        ),
        "diagnostic must be HelperOutsideWrapperSlot"
    );
}

#[test]
fn retained_signature_default_normalizes_template_to_string_slice() {
    // Proves the retained-only generic default normalization path exercised by generic
    // declarations without an emitted node: a `FunctionSignature` parameter whose default is a
    // live TIR template normalizes to a TIR-free `StringSlice` through the real
    // `normalize_retained_signature_defaults` helper used by
    // `synchronize_normalized_public_defaults`, not through a direct
    // `normalize_expression_templates` call labelled generic.
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("generic default text");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);

    let parameter_default = Expression::template(template, ValueMode::ImmutableOwned);
    let mut signature = FunctionSignature {
        parameters: vec![Declaration {
            id: InternedPath::new(),
            value: parameter_default,
            binding_span: None,
            config_qualifier: None,
        }],
        returns: Vec::new(),
    };

    normalize_retained_signature_defaults(
        &mut signature,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        &template_ir_store,
        &mut string_table,
    )
    .expect("a const text template default should normalize to a folded string");

    assert!(
        matches!(
            &signature.parameters[0].value.kind,
            ExpressionKind::StringSlice(normalized) if *normalized == text,
        ),
        "a retained template default must normalize to a TIR-free StringSlice, got {:?}",
        signature.parameters[0].value.kind
    );
}

#[test]
fn static_true_assertion_discards_normalized_runtime_template_message_after_validation() {
    let mut string_table = StringTable::new();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template = registered_runtime_template(
        string_table.intern("inactive: "),
        "name",
        TemplateViewContext::default(),
        &template_ir_store,
        &mut string_table,
    );
    let template_expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut type_environment = TypeEnvironment::new();
    let option_string_type_id = type_environment.intern_option(builtin_type_ids::STRING);
    let message = Expression::new(
        ExpressionKind::Coerced {
            value: Box::new(template_expression),
            to_type: option_string_type_id,
        },
        None,
        option_string_type_id,
        DataType::Option(Box::new(DataType::StringSlice)),
        ValueMode::ImmutableOwned,
    );
    let mut node = AstNode {
        kind: NodeKind::Assert {
            condition: Expression::bool(true, None, ValueMode::ImmutableOwned),
            message,
        },
        span: None,
        scope: InternedPath::new(),
    };
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store,
        module_resources: None,
    };

    normalize_ast_node_templates(&mut node, &mut context)
        .expect("static-true assertion messages should normalize before discard");
    // The production finalizer calls this cleanup immediately after its authoritative type/TIR
    // validation pass. This unit test exercises the same post-validation boundary directly.
    discard_inactive_assertion_messages(std::slice::from_mut(&mut node));

    let NodeKind::Assert { message, .. } = node.kind else {
        panic!("expected the test node to remain an assertion");
    };
    assert!(
        matches!(message.kind, ExpressionKind::OptionNone),
        "inactive assertion messages must be replaced with typed none, got {:?}",
        message.kind
    );
    assert_eq!(message.type_id, option_string_type_id);
    assert!(message.reactive_template.is_none());
    assert!(message.synthetic_interface_provenance.is_empty());
}
