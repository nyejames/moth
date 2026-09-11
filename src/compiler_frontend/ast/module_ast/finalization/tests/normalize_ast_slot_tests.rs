use super::*;

// ------------------------------
//  Slot-bearing template classification uses effective view
// ------------------------------

/// Builds a finalized slot template whose effective overlay resolves one fill.
///
/// WHAT: gives the resolver a module-local root plus the slot-resolution
///       overlay that makes the template an effective wrapper value.
/// WHY: const-fact classification must preserve the overlay-backed wrapper
///      category rather than reading only the structural root.
fn build_resolved_slot_template_store() -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let location = None;
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));

    let (template_id, fill_template_id) = {
        let mut store = store_handle.borrow_mut();

        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_root = fill_builder.push_sequence_node(Vec::new(), location);
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let slot_node = wrapper_builder.push_slot_node(SlotKey::Default, location);
        let template_id = wrapper_builder.finish_template(
            slot_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        );

        (template_id, fill_template_id)
    };

    let slot_overlay_id = store_handle
        .borrow_mut()
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(
                SlotOccurrenceId::new(0),
                TirSlotResolution::resolved(SlotKey::Default, vec![fill_template_id]),
            )],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(slot_overlay_id),
        wrapper_context: None,
    };

    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        span: None,
    };

    (template, store_handle)
}

#[test]
fn slot_bearing_module_constant_classifies_through_effective_tir_view() {
    let mut string_table = StringTable::new();

    let (template, registry) = build_resolved_slot_template_store();
    let projected = project_const_template_value(
        &template,
        &registry.borrow(),
        &mut string_table,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        None,
    )
    .expect("slot template should project as a const template value");

    assert_eq!(
        projected.kind,
        TemplateConstValueKind::WrapperTemplate,
        "resolved-slot module constants must classify through the effective TIR view"
    );

    let value = const_template_value_from_projection(projected, &template)
        .expect("an effective wrapper is a supported module-constant store value");
    assert!(
        matches!(
            value,
            ConstTemplateValue::Public {
                kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
                folded: Some(_),
                ..
            }
        ),
        "a wrapper module constant keeps its public projection and its folded string"
    );
}

#[test]
fn const_template_projection_round_trips_structural_resource_and_site_root() {
    let location = None;
    let mut producer_strings = StringTable::new();
    let mut producer_resources = ModuleResourceTable::new();
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("const-template-test"),
        "pages".to_owned(),
        ModuleRootRole::Normal,
    );
    let resource_origin = StableResourceOriginId::module_owned(
        module_origin,
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("test resource path should be portable"),
    );
    let resource_id = producer_resources.intern_origin(resource_origin.clone(), location);
    let before = producer_strings.intern("before");
    let after = producer_strings.intern("after");
    let structural_expression = Expression::new(
        ExpressionKind::StructuralString {
            pieces: vec![
                ConstStringPiece::Text(before),
                ConstStringPiece::Resource(resource_id),
                ConstStringPiece::SiteRoot,
                ConstStringPiece::Text(after),
            ],
        },
        location,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );

    let mut producer_store = TemplateIrStore::new();
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut producer_store);
        let dynamic_node = builder.push_dynamic_expression_node(
            structural_expression,
            TemplateSegmentOrigin::Body,
            None,
            location,
        );
        let root = builder.push_sequence_node(vec![dynamic_node], location);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location,
        )
    };
    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location,
    );

    let projected = project_const_template_value(
        &template,
        &producer_store,
        &mut producer_strings,
        DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        Some(&producer_resources),
    )
    .expect("structural const-template projection should succeed");
    let public = projected
        .public
        .expect("foldable structural template should have a public projection");
    assert_eq!(
        public.pieces,
        vec![PublicConstTemplatePiece::Text(OwnedFoldedString::Pieces(
            vec![
                OwnedFoldedStringPiece::Text("before".to_owned()),
                OwnedFoldedStringPiece::Resource(resource_origin.clone()),
                OwnedFoldedStringPiece::SiteRoot,
                OwnedFoldedStringPiece::Text("after".to_owned()),
            ]
        ),)],
        "projection must keep structural anchors inside one owned text run",
    );

    let mut consumer = ConstTemplateProjectionMaterializer::new();
    let mut consumer_strings = StringTable::new();
    let consumer_store_handle = Rc::clone(&consumer.template_ir_store);
    let imported = materialize_public_const_template(
        &mut consumer,
        &public,
        &consumer_store_handle,
        &mut consumer_strings,
        location,
    )
    .expect("consumer const-template materialization should succeed");
    let consumer_store = consumer.template_ir_store.borrow();
    let root = consumer_store
        .get_template(imported.tir_reference.root)
        .expect("materialized template should have a TIR entry")
        .root;
    let TemplateIrNodeKind::Sequence { children } = &consumer_store
        .get_node(root)
        .expect("materialized template root should exist")
        .kind
    else {
        panic!("materialized const-template root should be a sequence");
    };
    assert_eq!(children.len(), 1);
    let TemplateIrNodeKind::DynamicExpression { expression, .. } = &consumer_store
        .get_node(children[0])
        .expect("materialized structural text node should exist")
        .kind
    else {
        panic!("structural owned text should materialize as a dynamic expression");
    };
    let ExpressionKind::StructuralString { pieces } = &expression.kind else {
        panic!("structural owned text should remain a structural string expression");
    };
    let [
        ConstStringPiece::Text(consumer_before),
        ConstStringPiece::Resource(consumer_resource),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::Text(consumer_after),
    ] = pieces.as_slice()
    else {
        panic!("consumer materialization changed structural piece order");
    };
    assert_eq!(consumer_strings.resolve(*consumer_before), "before");
    assert_eq!(consumer_strings.resolve(*consumer_after), "after");
    assert_eq!(
        consumer
            .module_resources
            .try_origin(*consumer_resource)
            .expect("consumer resource handle should resolve")
            .origin,
        resource_origin,
    );
}
