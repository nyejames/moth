use super::*;
use crate::compiler_frontend::symbols::path_interner::PathId;
use moth_lexical::numeric::profile::NumericProfile;

fn runtime_template_handoff_from_expression(expression: Expression) -> OwnedRuntimeTemplateHandoff {
    let ExpressionKind::RuntimeTemplateHandoff(handoff) = expression.kind else {
        panic!("expected expression normalization to return an owned runtime-template handoff");
    };

    *handoff
}

#[test]
fn conditional_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let location = None;
    let body_text = string_table.intern("conditional body");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_text_node(
            body_text,
            "conditional body".len(),
            TemplateSegmentOrigin::Body,
            location,
        );
        let root = builder.push_conditional_node(
            TemplateBranchSelector::Bool(Expression::reference_with_type_id(
                path_fork
                    .try_intern_portable_path("show_conditional", &mut string_table)
                    .expect("test path fits"),
                DataType::Bool,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )),
            body,
            location,
        );
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
        numeric_profile: NumericProfile::STANDARD,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        module_resources: None,
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("conditional TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Conditional {
        selector,
        body,
        ..
    }) = handoff.body
    else {
        panic!("expected a runtime conditional handoff");
    };
    assert!(matches!(selector.as_ref(), TemplateBranchSelector::Bool(_)));
    assert!(matches!(
        body.as_ref(),
        OwnedRuntimeTemplateNode::Text { .. }
    ));
}

#[test]
fn loop_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
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
                path_fork
                    .try_intern_portable_path("keep_looping", &mut string_table)
                    .expect("test path fits"),
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
        numeric_profile: NumericProfile::STANDARD,
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

        OwnedRuntimeTemplateNode::Conditional { body, .. } => {
            collect_owned_node_string_slice_expressions(body, string_slices);
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
    let mut path_fork = PathInternerFork::empty();
    let byte_len = string_table.resolve(text).len();
    let reference_path = path_fork
        .try_intern_portable_path(reference_name, string_table)
        .expect("test path fits");
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
    let _path_fork = PathInternerFork::empty();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template =
        registered_runtime_template(text, "name", context, &template_ir_store, &mut string_table);

    let mut expression = Expression::template(template, ValueMode::ImmutableOwned);

    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        numeric_profile: NumericProfile::STANDARD,
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
    let _path_fork = PathInternerFork::empty();
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
            location,
        );
        let selected_node = builder.push_dynamic_expression_node(
            Expression::string_slice(
                selected_structural_text,
                location,
                ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            location,
        );
        let unselected_conditional = builder.push_conditional_node(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                location,
                ValueMode::ImmutableOwned,
            )),
            unselected_node,
            location,
        );
        let selected_conditional = builder.push_conditional_node(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                location,
                ValueMode::ImmutableOwned,
            )),
            selected_node,
            location,
        );
        let root = builder
            .push_sequence_node(vec![unselected_conditional, selected_conditional], location);
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
        NumericProfile::STANDARD,
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
        numeric_profile: NumericProfile::STANDARD,
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
    let _path_fork = PathInternerFork::empty();
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
        numeric_profile: NumericProfile::STANDARD,
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
}

#[test]
fn runtime_template_expression_handoff_uses_finalized_expression_overlay_view() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let overlay_text = string_table.intern("normalized overlay text");
    let runtime_path = path_fork
        .try_intern_portable_path("runtime_name", &mut string_table)
        .expect("test path fits");

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
        numeric_profile: NumericProfile::STANDARD,
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
}

/// Proves that a nested runtime template inside a TIR dynamic expression node
/// is normalized through the final effective view.
#[test]
fn nested_runtime_template_normalizes_through_final_view() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
        numeric_profile: NumericProfile::STANDARD,
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
        OwnedRuntimeTemplateNode::Conditional { body, .. } => {
            find_runtime_handoff_in_node(body, found);
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
    let _path_fork = PathInternerFork::empty();
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
                numeric_profile: NumericProfile::STANDARD,
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

/// Proves that a slot-insert helper artifact surviving composition is rejected
/// after final view traversal, not silently passed to HIR.
#[test]
fn helper_artifact_rejected_after_final_view_traversal() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
        numeric_profile: NumericProfile::STANDARD,
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
    let _path_fork = PathInternerFork::empty();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let text = string_table.intern("generic default text");
    let template = registered_text_template(text, context, &template_ir_store, &string_table);

    let parameter_default = Expression::template(template, ValueMode::ImmutableOwned);
    let mut signature = FunctionSignature {
        parameters: vec![Declaration {
            id: PathId::ROOT,
            value: parameter_default,
            binding_span: None,
            config_qualifier: None,
        }],
        returns: Vec::new(),
    };

    normalize_retained_signature_defaults(
        &mut signature,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        NumericProfile::STANDARD,
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
    let _path_fork = PathInternerFork::empty();
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
            message: Box::new(message),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let mut context = TemplateNormalizationContext {
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        numeric_profile: NumericProfile::STANDARD,
        string_table: &mut string_table,
        template_ir_store,
        module_resources: None,
    };

    normalize_ast_node_templates(&mut node, &mut context)
        .expect("static-true assertion messages should normalize before discard");
    // The production finalizer calls this cleanup immediately after its authoritative type/TIR
    // validation pass. This unit test exercises the same post-validation boundary directly.
    discard_inactive_assertion_messages(std::slice::from_mut(&mut node))
        .expect("normalized inactive assertion messages should discard successfully");

    let NodeKind::Assert { message, .. } = node.kind else {
        panic!("expected the test node to remain an assertion");
    };
    assert!(
        matches!(message.kind, ExpressionKind::OptionNone),
        "inactive assertion messages must be replaced with typed none, got {:?}",
        message.kind
    );
    assert_eq!(message.type_id, option_string_type_id);
    assert!(message.synthetic_interface_provenance.is_empty());
}

// ---------------------------------------------------------------------------
//  Real-finalizer receiver default synchronization
// ---------------------------------------------------------------------------
//
// End-to-end coverage for `synchronize_normalized_public_defaults` through the real
// single-file frontend pipeline: environment build, emission and finalization all run, so the
// resolved public root table is the post-finalization table the public-interface draft reads.
// Root receiver entries carry the emitted normalized signature copy, retained generic defaults
// normalize in place, private methods join by receiver ownership, and private-receiver methods
// stay out of the public table while their call sites still fill retained defaults. Defaulted
// parameters use authored foldable constant template joins so the joined value must read as one
// folded string. Aligned generic receivers cannot yet author an extra defaulted parameter (the
// `of` argument list consumes the following comma), so the retained generic default is covered
// through an exported generic free function sharing `normalize_retained_signature_defaults`.

use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;

fn path_display_name(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Option<String> {
    path_fork
        .component(path)
        .map(|name| string_table.resolve(name).to_owned())
}

fn emitted_nominal_path(
    nodes: &[AstNode],
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<PathId> {
    nodes.iter().find_map(|node| match &node.kind {
        NodeKind::StructDefinition(path, _)
            if path_display_name(*path, path_fork, string_table).as_deref() == Some(name) =>
        {
            Some(*path)
        }
        _ => None,
    })
}

fn emitted_function<'a>(
    nodes: &'a [AstNode],
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<(PathId, &'a FunctionSignature, &'a [AstNode])> {
    nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Function(path, signature, body)
            if path_display_name(*path, path_fork, string_table).as_deref() == Some(name) =>
        {
            Some((*path, signature, body.as_slice()))
        }
        _ => None,
    })
}

fn root_receiver_entry<'a>(
    entries: &'a [ReceiverMethodEntry],
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<&'a ReceiverMethodEntry> {
    entries.iter().find(|entry| {
        path_display_name(entry.function_path, path_fork, string_table).as_deref() == Some(name)
    })
}

fn signature_parameter<'a>(
    signature: &'a FunctionSignature,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> Option<&'a Declaration> {
    signature.parameters.iter().find(|parameter| {
        path_display_name(parameter.id, path_fork, string_table).as_deref() == Some(name)
    })
}

fn body_method_call_args<'a>(
    body: &'a [AstNode],
    method_path: &PathId,
) -> Option<&'a [CallArgument]> {
    body.iter().find_map(|node| {
        let NodeKind::ExpressionStatement(expression) = &node.kind else {
            return None;
        };
        let ExpressionKind::MethodCall {
            method_path: call_path,
            args,
            ..
        } = &expression.kind
        else {
            return None;
        };
        (call_path == method_path).then_some(args.as_slice())
    })
}

#[test]
fn finalization_synchronizes_ordinary_receiver_root_entry_with_emitted_signature() {
    // Receiver ownership publishes this private method through Counter's public surface.
    let source = "export:\n    Counter = | count Int = 0 |\n    run || -> Int:\n        counter = Counter()\n        counter.bump()\n        return counter.count\n    ;\n;\nbump |this Counter, label String = [\"joined \", \"default\"]| -> Int:\n    return this.count\n;\n";
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(source)
        .expect("an ordinary receiver method with a template default should finalize");
    let nodes = &build_result.ast.nodes;
    let receiver_entries = &build_result
        .public_interface_projection_input
        .root_table
        .receiver_methods;

    let struct_path = emitted_nominal_path(nodes, &path_fork, &string_table, "Counter")
        .expect("the public struct should emit a declaration node");

    // The free `run` function is a root, not a receiver entry: an extra vector row would
    // admit non-method callables into this vector.
    assert_eq!(
        receiver_entries.len(),
        1,
        "only the receiver method joins the root table receiver vector"
    );
    let entry = root_receiver_entry(receiver_entries, &path_fork, &string_table, "bump")
        .expect("the public receiver method must join the root table");
    assert_eq!(entry.receiver, ReceiverKey::Struct(struct_path));
    assert!(
        !entry.receiver_mutable,
        "an immutable this receiver must not mark the entry mutable"
    );

    // The sync contract, semantically: the root entry carries the emitted signature's
    // once-normalized parameter facts, not an independent interpretation.
    let (_, emitted_signature, _) = emitted_function(nodes, &path_fork, &string_table, "bump")
        .expect("the non-generic receiver method must emit an ordinary declaration node");
    assert_eq!(
        entry.signature.parameters.len(),
        emitted_signature.parameters.len(),
        "the root entry must join the emitted parameter list"
    );
    for (entry_parameter, emitted_parameter) in entry
        .signature
        .parameters
        .iter()
        .zip(emitted_signature.parameters.iter())
    {
        assert_eq!(
            path_display_name(entry_parameter.id, &path_fork, &string_table),
            path_display_name(emitted_parameter.id, &path_fork, &string_table),
            "the root entry must join the emitted parameters in order"
        );
        assert_eq!(
            entry_parameter.value.type_id, emitted_parameter.value.type_id,
            "the joined parameter must keep the emitted semantic type"
        );
        assert_eq!(
            entry_parameter.value.span, emitted_parameter.value.span,
            "the joined parameter must keep the authored default span"
        );
        assert_eq!(
            entry_parameter.value.synthetic_interface_provenance,
            emitted_parameter.value.synthetic_interface_provenance,
            "the joined parameter must keep the normalized default provenance"
        );
    }

    let entry_label = signature_parameter(&entry.signature, &path_fork, &string_table, "label")
        .expect("the receiver signature must retain its defaulted label parameter");
    let ExpressionKind::StringSlice(entry_label_text) = &entry_label.value.kind else {
        panic!(
            "the authored template default must reach the root table folded, got {:?}",
            entry_label.value.kind
        );
    };
    assert_eq!(
        string_table.resolve(*entry_label_text),
        "joined default",
        "the root default must carry the folded template text"
    );
    assert!(
        entry_label.value.span.is_some(),
        "the folded root default must keep an authored span"
    );
    assert!(
        entry_label.value.synthetic_interface_provenance.is_empty(),
        "a template join over portable literals must stay portable"
    );

    // The omitted defaulted argument at the call site is filled from the retained signature
    // and normalized with the caller's body into the same concrete folded value.
    let (_, _, run_body) = emitted_function(nodes, &path_fork, &string_table, "run")
        .expect("the exported free function must emit an ordinary declaration node");
    let call_args = body_method_call_args(run_body, &entry.function_path)
        .expect("the receiver call inside run must be a method call node");
    let [filled] = call_args else {
        panic!("the receiver call must fill exactly its defaulted parameter")
    };
    let ExpressionKind::StringSlice(filled_text) = &filled.value.kind else {
        panic!(
            "the omitted template default must fill as a folded string, got {:?}",
            filled.value.kind
        );
    };
    assert_eq!(
        string_table.resolve(*filled_text),
        "joined default",
        "the omitted default must fill with the declared folded value"
    );
}

#[test]
fn finalization_normalizes_generic_receiver_root_entry_and_retained_function_default() {
    // Generic declarations do not emit ordinary declaration nodes, so their root-table
    // signatures are the retained normalization owner. Aligned generic receivers cannot yet
    // author an extra defaulted parameter, so the retained template default rides the
    // supported exported generic free function sharing `normalize_retained_signature_defaults`.
    let source = "export:\n    Cell type T = |\n        value T,\n    |\n    pick type T |value T, tag String = [\"joined \", \"n/a\"]| -> T:\n        return value\n    ;\n;\ncell_value type T |this Cell of T| -> T:\n    return this.value\n;\nbox = Cell(\"v\")\n";
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(source)
        .expect("generic receiver and function roots with retained defaults should finalize");
    let nodes = &build_result.ast.nodes;
    let root_table = &build_result.public_interface_projection_input.root_table;

    let entry = root_receiver_entry(
        &root_table.receiver_methods,
        &path_fork,
        &string_table,
        "cell_value",
    )
    .expect("the generic receiver method must join the root table");
    assert!(
        !nodes.iter().any(
            |node| matches!(&node.kind, NodeKind::Function(path, ..) if *path == entry.function_path)
        ),
        "generic receiver methods must not emit an ordinary declaration node"
    );

    let struct_path = root_table
        .roots
        .iter()
        .filter(|root| {
            matches!(&root.kind, ResolvedPublicTypeRootKind::Struct { .. })
                && path_display_name(root.path, &path_fork, &string_table).as_deref()
                    == Some("Cell")
        })
        .map(|root| root.path)
        .next()
        .expect("the exported generic struct must retain a struct root");
    assert_eq!(
        entry.receiver,
        ReceiverKey::Struct(struct_path),
        "a generic receiver's key must resolve to its generic struct base path"
    );

    let function_signature = root_table
        .roots
        .iter()
        .find_map(|root| match &root.kind {
            ResolvedPublicTypeRootKind::Function {
                signature,
                generic_parameter_list_id,
            } if generic_parameter_list_id.is_some()
                && path_display_name(root.path, &path_fork, &string_table).as_deref()
                    == Some("pick") =>
            {
                Some(signature)
            }
            _ => None,
        })
        .expect("the exported generic function must retain a generic function root");
    let tag = signature_parameter(function_signature, &path_fork, &string_table, "tag")
        .expect("the generic function signature must retain its defaulted tag parameter");
    let ExpressionKind::StringSlice(tag_text) = &tag.value.kind else {
        panic!(
            "the retained generic template default must normalize to a folded string, got {:?}",
            tag.value.kind
        );
    };
    assert_eq!(
        string_table.resolve(*tag_text),
        "joined n/a",
        "the retained generic default must carry the folded template text"
    );
    assert!(
        tag.value.span.is_some(),
        "the normalized generic default must keep an authored span"
    );
    assert!(
        tag.value.synthetic_interface_provenance.is_empty(),
        "a template join over portable literals must stay portable"
    );
}

#[test]
fn finalization_keeps_private_receiver_methods_out_of_public_roots_and_fills_call_defaults() {
    let source = "Counter = | count Int = 0 |\nbump |this Counter, step Int = 7| -> Int:\n    return this.count + step\n;\nexport:\n    run || -> Int:\n        counter = Counter()\n        counter.bump()\n        return counter.count\n    ;\n;\n";
    let (build_result, path_fork, string_table) = parse_single_file_ast_build_result(source)
        .expect("a private receiver method on a private struct should finalize and stay callable");
    let nodes = &build_result.ast.nodes;
    let receiver_entries = &build_result
        .public_interface_projection_input
        .root_table
        .receiver_methods;

    // The private struct's receiver is not a public nominal, so the method never joins the
    // root table receiver vector.
    assert!(
        receiver_entries.is_empty(),
        "a method on a private receiver must stay out of the public root table"
    );

    // The private method still emits and its retained signature keeps the authored default.
    let (bump_path, bump_signature, _) = emitted_function(nodes, &path_fork, &string_table, "bump")
        .expect("the private receiver method must emit an ordinary declaration node");
    let step = signature_parameter(bump_signature, &path_fork, &string_table, "step")
        .expect("the private method signature must retain its step parameter");
    assert!(
        matches!(step.value.kind, ExpressionKind::Int(7)),
        "the private method's authored default must survive finalization, got {:?}",
        step.value.kind
    );

    // The equivalent lane for non-root receiver defaults lives at the real consumer: the call
    // site fills the defaulted argument from the retained signature with the declared value.
    let (_, _, run_body) = emitted_function(nodes, &path_fork, &string_table, "run")
        .expect("the exported free function must emit an ordinary declaration node");
    let call_args = body_method_call_args(run_body, &bump_path)
        .expect("the receiver call inside run must be a method call node");
    let [filled] = call_args else {
        panic!("the receiver call must fill exactly its defaulted parameter")
    };
    assert!(
        matches!(filled.value.kind, ExpressionKind::Int(7)),
        "the omitted default must fill with the declared value at the call site, got {:?}",
        filled.value.kind
    );
}
