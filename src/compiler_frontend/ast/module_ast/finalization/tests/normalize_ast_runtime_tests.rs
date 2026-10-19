use super::*;
use crate::compiler_frontend::ast::const_eval::FoldServices;

fn runtime_template_handoff_from_expression(
    expressions: &AstExpressionBuilder,
    expression: ExprId,
) -> OwnedRuntimeTemplateHandoff {
    let ExpressionOperationView::RuntimeTemplate(handoff) = expressions
        .view(expression)
        .expect("validated runtime handoff")
        .operation()
    else {
        panic!("expected expression normalization to return an owned runtime-template handoff");
    };
    handoff.clone()
}

#[test]
fn conditional_tir_root_normalizes_into_owned_runtime_handoff() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut path_fork = PathInternerFork::empty();
    let location = None;
    let body_text = string_table.intern("conditional body");
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let body = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_text_node(
                body_text,
                "conditional body".len(),
                TemplateSegmentOrigin::Body,
                location,
            )
        };
        let condition_path = path_fork
            .try_intern_portable_path("show_conditional", &mut string_table)
            .expect("test path fits");
        drop(store);
        let condition_expression = expressions
            .reference_with_type_id(
                condition_path,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )
            .expect("reference expression")
            .id();
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_conditional_node(
            TemplateBranchSelector::Bool(condition_expression),
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

    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("conditional TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(&expressions, expression);
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
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
        let (body, aggregate_wrapper) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let body = builder.push_text_node(
                loop_text,
                "loop body".len(),
                TemplateSegmentOrigin::Body,
                location,
            );
            let open = builder.push_text_node(open_text, 1, TemplateSegmentOrigin::Body, location);
            let close =
                builder.push_text_node(close_text, 1, TemplateSegmentOrigin::Body, location);
            let aggregate_wrapper =
                builder.push_sequence_node(vec![open, aggregate_output, close], location);
            (body, aggregate_wrapper)
        };
        drop(store);
        let loop_path = path_fork
            .try_intern_portable_path("keep_looping", &mut string_table)
            .expect("test path fits");
        let condition_expression = expressions
            .reference_with_type_id(
                loop_path,
                builtin_type_ids::BOOL,
                location,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )
            .expect("reference expression")
            .id();
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let header = TemplateLoopHeader::Conditional {
            condition: condition_expression,
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

    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("loop TIR root should normalize through the finalized effective view");

    let handoff = runtime_template_handoff_from_expression(&expressions, expression);
    let OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Loop { .. }) = handoff.body
    else {
        panic!("expected a loop runtime handoff");
    };
}

fn collect_owned_handoff_string_slice_expressions_with_owners(
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    handoff: &OwnedRuntimeTemplateHandoff,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match &handoff.body {
        OwnedRuntimeTemplateBody::Render(root) => {
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                root,
                string_slices,
            );
        }
        OwnedRuntimeTemplateBody::RuntimeSlotApplication(slot_handoff) => {
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                &slot_handoff.wrapper,
                string_slices,
            );
            for source in &slot_handoff.contribution_sources {
                collect_owned_node_string_slice_expressions(
                    expressions,
                    constants,
                    &source.render_root,
                    string_slices,
                );
            }
            for site in &slot_handoff.slot_sites {
                collect_owned_node_string_slice_expressions(
                    expressions,
                    constants,
                    &site.render_root,
                    string_slices,
                );
            }
        }
    }
}

fn collect_owned_node_string_slice_expressions(
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    node: &OwnedRuntimeTemplateNode,
    string_slices: &mut Vec<crate::compiler_frontend::symbols::string_interning::StringId>,
) {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if let ExpressionOperationView::Constant(value) = expressions
                .view(*expression)
                .expect("validated handoff leaf")
                .operation()
                && let Some(
                    crate::compiler_frontend::ast::const_values::store::ConstValuePayload::String(
                        crate::compiler_frontend::ast::const_values::store::ConstStringValue::Text(
                            text,
                        ),
                    ),
                ) = constants.payload(value)
            {
                string_slices.push(*text);
            }
        }
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                collect_owned_node_string_slice_expressions(
                    expressions,
                    constants,
                    child,
                    string_slices,
                );
            }
        }
        OwnedRuntimeTemplateNode::Conditional { body, .. } => {
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                body,
                string_slices,
            );
        }
        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                body,
                string_slices,
            );
            if let Some(wrapper) = aggregate_wrapper {
                collect_owned_node_string_slice_expressions(
                    expressions,
                    constants,
                    wrapper,
                    string_slices,
                );
            }
        }
        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            collect_owned_handoff_string_slice_expressions_with_owners(
                expressions,
                constants,
                template,
                string_slices,
            );
        }
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                child,
                string_slices,
            );
            collect_owned_node_string_slice_expressions(
                expressions,
                constants,
                wrapper,
                string_slices,
            );
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
    expressions: &mut AstExpressionBuilder,
) -> Template {
    let mut path_fork = PathInternerFork::empty();
    let byte_len = string_table.resolve(text).len();
    let reference_path = path_fork
        .try_intern_portable_path(reference_name, string_table)
        .expect("test path fits");
    let reference_expression = expressions
        .reference_with_type_id(
            reference_path,
            builtin_type_ids::STRING,
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("reference expression")
        .id();
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let _path_fork = PathInternerFork::empty();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template = registered_runtime_template(
        text,
        "name",
        context,
        &template_ir_store,
        &mut string_table,
        &mut expressions,
    );

    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();

    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("ordinary runtime template expression normalization should succeed");

    let handoff = runtime_template_handoff_from_expression(&expressions, expression);
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must materialize a render body handoff"
    );
}

#[test]
fn folded_template_preserves_selected_effective_dynamic_provenance() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
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
    let unselected_expression = {
        let value = expressions
            .string_slice(
                &mut constants,
                unselected_text,
                location,
                ValueMode::ImmutableOwned,
            )
            .expect("string expression")
            .id();
        expressions
            .with_synthetic_interface_provenance(
                expressions.own(value),
                SyntheticInterfaceProvenance::single(unselected_member),
            )
            .expect("expression provenance")
            .id()
    };
    let selected_expression = expressions
        .string_slice(
            &mut constants,
            selected_structural_text,
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("string expression")
        .id();
    let unselected_condition = expressions
        .bool(&mut constants, false, location, ValueMode::ImmutableOwned)
        .expect("bool expression")
        .id();
    let selected_condition = expressions
        .bool(&mut constants, true, location, ValueMode::ImmutableOwned)
        .expect("bool expression")
        .id();
    let overlay_expression = expressions
        .string_slice(
            &mut constants,
            selected_effective_text,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("string expression")
        .id();
    let overlay_expression = expressions
        .with_synthetic_interface_provenance(
            expressions.own(overlay_expression),
            SyntheticInterfaceProvenance::single(selected_member.clone()),
        )
        .expect("expression provenance")
        .id();

    let (template_id, selected_site_id) = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let unselected_node = builder.push_dynamic_expression_node(
            unselected_expression,
            TemplateSegmentOrigin::Body,
            location,
        );
        let selected_node = builder.push_dynamic_expression_node(
            selected_expression,
            TemplateSegmentOrigin::Body,
            location,
        );
        let unselected_conditional = builder.push_conditional_node(
            TemplateBranchSelector::Bool(unselected_condition),
            unselected_node,
            location,
        );
        let selected_conditional = builder.push_conditional_node(
            TemplateBranchSelector::Bool(selected_condition),
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
            overrides: vec![(selected_site_id, overlay_expression)],
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
    let constant_expression = expressions
        .template(template.clone(), ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let constant_view = expressions
        .view(constant_expression)
        .expect("validated template root");
    assert!(
        constant_view
            .occurrence_facts()
            .is_none_or(|facts| facts.provenance.is_empty()),
        "the outer template must start without injected provenance"
    );

    let ExpressionOperationView::Template(constant_template) = constant_view.operation() else {
        panic!("module constant regression must start from a template expression");
    };
    let constant_template = constant_template.clone();
    let projected_constant = project_const_template_value(
        &constant_template,
        &template_ir_store.borrow(),
        &mut expressions,
        &mut constants,
        FoldServices::new(&mut string_table, None, None),
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("selected exact TIR fold should project module constants");
    assert_eq!(
        projected_constant.provenance.members(),
        std::slice::from_ref(&selected_member),
        "module constant projection must retain selected folded provenance"
    );

    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("selected exact TIR fold should normalize");

    let normalized = expressions.view(expression).expect("validated folded root");
    assert!(matches!(
        normalized.operation(),
        ExpressionOperationView::Constant(_)
    ));
    assert_eq!(
        normalized
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        vec![selected_member],
        "only the selected effective dynamic payload may reach the folded value"
    );
}

#[test]
fn runtime_template_expression_normalization_replaces_template_with_owned_handoff() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let _path_fork = PathInternerFork::empty();
    let text = string_table.intern("hello ");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template = registered_runtime_template(
        text,
        "name",
        context,
        &template_ir_store,
        &mut string_table,
        &mut expressions,
    );

    // This test covers preservation of metadata already carried by the outer runtime template
    // expression. Nested effective-fold provenance is covered by the folded-template test above.
    let provenance_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let template_expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let mut expression = expressions
        .with_synthetic_interface_provenance(
            expressions.own(template_expression),
            SyntheticInterfaceProvenance::single(provenance_member.clone()),
        )
        .expect("expression provenance")
        .id();
    let expression_span = expressions
        .view(expression)
        .expect("expression provenance view")
        .span();

    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template expression normalization should succeed");

    let ExpressionOperationView::RuntimeTemplate(handoff) = expressions
        .view(expression)
        .expect("validated runtime handoff")
        .operation()
    else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };
    assert!(
        matches!(handoff.body, OwnedRuntimeTemplateBody::Render(_)),
        "ordinary runtime templates must keep using the render handoff body"
    );
    assert_eq!(
        expressions
            .view(expression)
            .expect("validated runtime handoff")
            .span(),
        expression_span,
    );
    assert_eq!(
        expressions
            .view(expression)
            .expect("validated runtime handoff")
            .value_mode(),
        ValueMode::ImmutableOwned,
    );
    assert_eq!(
        expressions
            .view(expression)
            .expect("validated runtime handoff")
            .occurrence_facts()
            .map(|facts| facts.provenance.members().to_vec())
            .unwrap_or_default(),
        vec![provenance_member],
    );
}

#[test]
fn runtime_template_expression_handoff_uses_finalized_expression_overlay_view() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let _path_fork = PathInternerFork::empty();
    let overlay_text = string_table.intern("normalized overlay text");

    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();
    let nested_template_expression = expressions
        .template(
            registered_text_template(
                overlay_text,
                empty_context,
                &template_ir_store,
                &string_table,
            ),
            ValueMode::ImmutableOwned,
        )
        .expect("template expression")
        .id();

    let template_id = {
        let mut local_paths = PathInternerFork::empty();
        let runtime_path = local_paths
            .try_intern_portable_path("runtime_name", &mut string_table)
            .expect("test path fits");
        let runtime_expression = expressions
            .reference_with_type_id(
                runtime_path,
                builtin_type_ids::STRING,
                None,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )
            .expect("reference expression")
            .id();
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let normalized_dynamic_node = builder.push_dynamic_expression_node(
            nested_template_expression,
            TemplateSegmentOrigin::Body,
            None,
        );
        let runtime_dynamic_node = builder.push_dynamic_expression_node(
            runtime_expression,
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
    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();

    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("runtime template normalization should use the finalized view handoff");

    let ExpressionOperationView::RuntimeTemplate(handoff) = expressions
        .view(expression)
        .expect("validated runtime handoff")
        .operation()
    else {
        panic!("runtime template expression should be replaced with an owned handoff");
    };

    let mut string_slices = Vec::new();
    collect_owned_handoff_string_slice_expressions_with_owners(
        &expressions,
        &constants,
        handoff,
        &mut string_slices,
    );
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
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
        &mut expressions,
    );
    let nested_template_expression = expressions
        .template(nested_template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();

    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let dynamic_node = builder.push_dynamic_expression_node(
            nested_template_expression,
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
    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();

    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_expression_templates(&mut expression, &mut context)
        .expect("nested runtime template normalization should succeed through the final TIR view");

    let ExpressionOperationView::RuntimeTemplate(handoff) = expressions
        .view(expression)
        .expect("validated runtime handoff")
        .operation()
    else {
        panic!("outer template expression should be replaced with an owned handoff");
    };

    // The handoff must contain the nested runtime template handoff inside a
    // DynamicExpression node, proving the overlay path normalized it.
    let mut found_nested_handoff = false;
    if let OwnedRuntimeTemplateBody::Render(root) = &handoff.body {
        find_runtime_handoff_in_node(&expressions, root, &mut found_nested_handoff);
    }
    assert!(
        found_nested_handoff,
        "handoff must contain the nested runtime template handoff materialized from the final TIR view"
    );
}

/// Recursively checks whether any DynamicExpression node in the owned handoff
/// tree carries a RuntimeTemplateHandoff expression kind.
fn find_runtime_handoff_in_node(
    expressions: &AstExpressionBuilder,
    node: &OwnedRuntimeTemplateNode,
    found: &mut bool,
) {
    if *found {
        return;
    }
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            if matches!(
                expressions
                    .view(*expression)
                    .expect("validated handoff leaf")
                    .operation(),
                ExpressionOperationView::RuntimeTemplate(_)
            ) {
                *found = true;
            }
        }
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                find_runtime_handoff_in_node(expressions, child, found);
            }
        }
        OwnedRuntimeTemplateNode::Conditional { body, .. } => {
            find_runtime_handoff_in_node(expressions, body, found);
        }
        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            find_runtime_handoff_in_node(expressions, body, found);
            if let Some(wrapper) = aggregate_wrapper {
                find_runtime_handoff_in_node(expressions, wrapper, found);
            }
        }
        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            find_runtime_handoff_in_node(expressions, child, found);
            find_runtime_handoff_in_node(expressions, wrapper, found);
        }
        _ => {}
    }
}

/// Proves that a const child template referenced from the outer TIR view folds
/// correctly through the final view.
#[test]
fn nested_const_template_folds_through_final_view() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
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
                expression_builder: &mut expressions,
                const_values: &mut constants,
                string_table: &mut string_table,
                template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
                template_ir_store: &template_ir_store,
                external_package_registry: None,
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
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
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

    let mut expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();

    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store: Rc::clone(&template_ir_store),
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
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
fn static_true_assertion_discards_normalized_runtime_template_message_after_validation() {
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let option_string_type_id = type_environment.intern_option(builtin_type_ids::STRING);
    let _path_fork = PathInternerFork::empty();
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let template = registered_runtime_template(
        string_table.intern("inactive: "),
        "name",
        TemplateViewContext::default(),
        &template_ir_store,
        &mut string_table,
        &mut expressions,
    );
    let template_expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression")
        .id();
    let template_expression = expressions.own(template_expression);
    let message = expressions
        .coerced(template_expression, option_string_type_id)
        .expect("message coercion");
    let mut node = AstNode {
        kind: NodeKind::Assert {
            condition: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("bool expression")
                .id(),
            message: message.id(),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let mut context = TemplateNormalizationContext {
        expression_builder: &mut expressions,
        const_values: &mut constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        string_table: &mut string_table,
        template_ir_store,
        external_package_registry: None,
        module_resources: None,
        expression_pending: Vec::new(),
        expression_results: Vec::new(),
    };

    normalize_ast_node_templates(&mut node, &mut context)
        .expect("static-true assertion messages should normalize before discard");
    // The production finalizer calls this cleanup immediately after its authoritative type/TIR
    // validation pass. This unit test exercises the same post-validation boundary directly.
    discard_inactive_assertion_messages(
        std::slice::from_mut(&mut node),
        &mut expressions,
        &mut constants,
    )
    .expect("normalized inactive assertion messages should discard successfully");

    let NodeKind::Assert { message, .. } = node.kind else {
        panic!("expected the test node to remain an assertion");
    };
    let normalized = expressions.view(message).expect("validated inert message");
    assert!(matches!(
        normalized.operation(),
        ExpressionOperationView::Constant(_)
    ));
    assert!(
        normalized
            .value_type_id()
            .expect("typed none keeps its result")
            == option_string_type_id
    );
    assert!(normalized.occurrence_facts().is_none());
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
// through an exported generic free function sharing `synchronize_normalized_public_defaults`.

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
) -> Option<&'a crate::compiler_frontend::ast::statements::functions::FunctionParameter> {
    signature.parameters.iter().find(|parameter| {
        path_display_name(parameter.id, path_fork, string_table).as_deref() == Some(name)
    })
}

#[test]
fn finalization_synchronizes_ordinary_receiver_root_entry_with_emitted_signature() {
    // Receiver ownership publishes this private method through Counter's public surface.
    let source = "export:\n    Counter = | count I32 = 0 |\n    run || -> I32:\n        counter = Counter()\n        counter.bump()\n        return counter.count\n    ;\n;\nbump |this Counter, label String = [\"joined \", \"default\"]| -> I32:\n    return this.count\n;\n";
    let (build_result, path_fork, mut string_table) = parse_single_file_ast_build_result(source)
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
            entry_parameter.type_id, emitted_parameter.type_id,
            "the joined parameter must keep the emitted semantic type"
        );
        assert_eq!(
            entry_parameter.default, emitted_parameter.default,
            "the joined parameter must keep the canonical folded default"
        );
    }

    let entry_label = signature_parameter(&entry.signature, &path_fork, &string_table, "label")
        .expect("the receiver signature must retain its defaulted label parameter");
    let entry_default = entry_label.default.expect("canonical folded default");
    assert_eq!(
        build_result.ast.const_values.string_value(entry_default),
        Some(string_table.intern("joined default")),
        "the root default must carry the folded template text"
    );
    let entry_metadata = build_result
        .ast
        .const_values
        .metadata(entry_default)
        .expect("canonical default metadata");
    assert!(
        entry_metadata.span.is_some(),
        "the folded root default must keep an authored span"
    );
    assert!(
        entry_metadata.synthetic_interface_provenance.is_empty(),
        "a template join over portable literals must stay portable"
    );

    // The omitted defaulted argument at the call site is filled from the retained signature
    // and normalized with the caller's body into the same concrete folded value.
    let (_, _, run_body) = emitted_function(nodes, &path_fork, &string_table, "run")
        .expect("the exported free function must emit an ordinary declaration node");
    let call_expression = run_body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::ExpressionStatement(expression) => Some(*expression),
            _ => None,
        })
        .expect("the receiver call inside run must be an expression statement");
    let ExpressionOperationView::MethodCall { receiver: _, call } = build_result
        .ast
        .expressions
        .view(call_expression)
        .expect("bound receiver call")
        .operation()
    else {
        panic!("the receiver call inside run must be a method call node");
    };
    assert!(
        call.arguments().len() == 1,
        "the receiver call must fill exactly its defaulted parameter"
    );
    let filled = call
        .arguments()
        .iter()
        .next()
        .expect("filled defaulted argument");
    let ExpressionOperationView::Constant(filled_value) = filled.value().operation() else {
        panic!("the omitted template default must fill as a folded string");
    };
    assert_eq!(
        build_result.ast.const_values.string_value(filled_value),
        Some(string_table.intern("joined default")),
        "the omitted default must fill with the declared folded value"
    );
}

#[test]
fn finalization_normalizes_generic_receiver_root_entry_and_retained_function_default() {
    // Generic declarations do not emit ordinary declaration nodes, so their root-table
    // signatures are the retained normalization owner. Aligned generic receivers cannot yet
    // author an extra defaulted parameter, so the retained template default rides the
    // supported exported generic free function sharing `synchronize_normalized_public_defaults`.
    let source = "export:\n    Cell type T = |\n        value T,\n    |\n    pick type T |value T, tag String = [\"joined \", \"n/a\"]| -> T:\n        return value\n    ;\n;\ncell_value type T |this Cell of T| -> T:\n    return this.value\n;\nbox = Cell(\"v\")\n";
    let (build_result, path_fork, mut string_table) = parse_single_file_ast_build_result(source)
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
    let tag_default = tag.default.expect("canonical folded default");
    assert_eq!(
        build_result.ast.const_values.string_value(tag_default),
        Some(string_table.intern("joined n/a")),
        "the retained generic default must carry the folded template text"
    );
    let tag_metadata = build_result
        .ast
        .const_values
        .metadata(tag_default)
        .expect("canonical default metadata");
    assert!(
        tag_metadata.span.is_some(),
        "the normalized generic default must keep an authored span"
    );
    assert!(
        tag_metadata.synthetic_interface_provenance.is_empty(),
        "a template join over portable literals must stay portable"
    );
}

#[test]
fn finalization_keeps_private_receiver_methods_out_of_public_roots_and_fills_call_defaults() {
    let source = "Counter = | count I32 = 0 |\nbump |this Counter, step I32 = 7| -> I32:\n    return this.count + step\n;\nexport:\n    run || -> I32:\n        counter = Counter()\n        counter.bump()\n        return counter.count\n    ;\n;\n";
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
    let (_, bump_signature, _) = emitted_function(nodes, &path_fork, &string_table, "bump")
        .expect("the private receiver method must emit an ordinary declaration node");
    let step = signature_parameter(bump_signature, &path_fork, &string_table, "step")
        .expect("the private method signature must retain its step parameter");
    let step_default = step.default.expect("canonical folded default");
    assert!(
        matches!(
            build_result.ast.const_values.payload(step_default),
            Some(crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(value))
                if value.scalar() == moth_lexical::numeric::fixed_scalar::FixedScalar::I32
                    && value.as_i64() == Some(7)
        ),
        "the private method's authored default must survive finalization"
    );

    // The equivalent lane for non-root receiver defaults lives at the real consumer: the call
    // site fills the defaulted argument from the retained signature with the declared value.
    let (_, _, run_body) = emitted_function(nodes, &path_fork, &string_table, "run")
        .expect("the exported free function must emit an ordinary declaration node");
    let call_expression = run_body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::ExpressionStatement(expression) => Some(*expression),
            _ => None,
        })
        .expect("the receiver call inside run must be an expression statement");
    let ExpressionOperationView::MethodCall { receiver: _, call } = build_result
        .ast
        .expressions
        .view(call_expression)
        .expect("bound receiver call")
        .operation()
    else {
        panic!("the receiver call inside run must be a method call node");
    };
    assert!(call.arguments().len() == 1);
    let filled = call
        .arguments()
        .iter()
        .next()
        .expect("filled defaulted argument");
    let ExpressionOperationView::Constant(filled_value) = filled.value().operation() else {
        panic!("the omitted default must fill with the declared value at the call site");
    };
    assert!(
        matches!(
            build_result.ast.const_values.payload(filled_value),
            Some(crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(value))
                if value.scalar() == moth_lexical::numeric::fixed_scalar::FixedScalar::I32
                    && value.as_i64() == Some(7)
        ),
        "the omitted default must fill with the declared value at the call site"
    );
}
