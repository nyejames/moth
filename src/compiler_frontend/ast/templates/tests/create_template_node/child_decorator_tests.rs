use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeKind, collect_tir_slot_layout, wrapper_reference_for_template,
};
use crate::compiler_frontend::compiler_messages::InvalidTemplateSlotReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView, InvalidTemplateSlotView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn parse_decorator_template(
    source: &str,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> (Template, ScopeContext) {
    let file_tokens = template_tokens_from_source(source, string_table, span_builder, path_fork);
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
    let context = new_constant_context(source_path, path_fork);
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        expression_builder,
        const_values,
        vec![],
        string_table,
        path_fork,
    )
    .expect("decorator template should parse");
    (template, context)
}

fn wrapper_contexts(
    template: &Template,
    context: &ScopeContext,
) -> Vec<crate::compiler_frontend::ast::templates::tir::TemplateWrapperReference> {
    let store = context.template_ir_store.borrow();
    let overlay_id = template
        .tir_reference
        .context
        .wrapper_context
        .expect("decorators should attach a wrapper-context overlay");
    let overlay = store
        .wrapper_context_overlay(overlay_id)
        .expect("wrapper-context overlay should exist");
    assert_eq!(
        overlay.contexts.len(),
        1,
        "one child occurrence should carry inherited wrapper context"
    );
    let wrapper_set_ref = overlay.contexts[0]
        .1
        .inherited_wrapper_set
        .expect("child occurrence should carry an inherited wrapper set");
    store
        .get_wrapper_set(wrapper_set_ref)
        .expect("wrapper set should exist in the store")
        .wrappers
        .to_vec()
}

#[test]
fn fresh_marks_template_to_skip_parent_child_wrappers() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let wrapper_file_tokens = template_tokens_from_source(
        "[: inherited]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let wrapper_source_path = wrapper_file_tokens.source_path;
    let canonical_owner = wrapper_file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut wrapper_tokens = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    wrapper_tokens
        .set_position(wrapper_file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let context = new_constant_context(wrapper_source_path, &path_fork);
    let wrapper = Template::new(
        &mut wrapper_tokens,
        wrapper_source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("inherited wrapper should parse");
    let inherited_wrapper = {
        let reference = &wrapper.tir_reference;
        TemplateWrapperReference::new(reference.root, reference.phase, reference.context)
    };

    let file_tokens = template_tokens_from_source(
        "[$fresh, $md:\n# Hello\n]",
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
    let context = new_constant_context(source_path, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![inherited_wrapper],
        &mut string_table,
        &mut path_fork,
    )
    .expect("template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert!(effective_style.formatter.is_some());
    assert!(effective_style.skip_parent_child_wrappers);

    // A template without decorators must not attach a wrapper-context overlay.
    assert!(
        template.tir_reference.context.wrapper_context.is_none(),
        "template without $before/$after should not attach wrapper-context overlays"
    );
}

#[test]
fn before_and_after_compose_one_direct_child_wrapper() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let (template, context) = parse_decorator_template(
        "[$before [:prefix], $after [:suffix]: [: child]]",
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
        &mut span_builder,
    );

    let wrappers = wrapper_contexts(&template, &context);
    assert_eq!(
        wrappers.len(),
        1,
        "both decorator edges must compose into one direct-child wrapper"
    );

    let store = context.template_ir_store.borrow();
    let layout = collect_tir_slot_layout(&store, wrappers[0].root)
        .expect("composite decorator layout should resolve");
    assert!(
        layout.schema.has_default_slot,
        "the composite wrapper must expose one implicit default child slot"
    );
    assert_eq!(
        layout.placeholders.len(),
        1,
        "only the compiler-created default slot may exist without named slots"
    );

    let composite = store
        .get_template(wrappers[0].root)
        .expect("composite wrapper template should exist");
    let TemplateIrNodeKind::Sequence { children } = &store
        .get_node(composite.root)
        .expect("composite root should exist")
        .kind
    else {
        panic!("composite decorator wrapper should use a sequence root");
    };
    assert_eq!(
        children.len(),
        3,
        "prefix, implicit default slot and suffix must stay in source order"
    );
    assert!(matches!(
        store.get_node(children[0]).expect("prefix node").kind,
        TemplateIrNodeKind::ChildTemplate { .. }
    ));
    assert!(matches!(
        store.get_node(children[1]).expect("slot node").kind,
        TemplateIrNodeKind::Slot { .. }
    ));
    assert!(matches!(
        store.get_node(children[2]).expect("suffix node").kind,
        TemplateIrNodeKind::ChildTemplate { .. }
    ));
}

#[test]
fn before_directive_accepts_const_string_reference() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");
    let prefix_name = string_table.intern("prefix");
    let prefix_value = expression_builder
        .string_slice(
            &mut const_values,
            string_table.intern("prefix: "),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("prefix string expression should construct")
        .id();
    let declarations = vec![make_test_variable(
        path_fork
            .try_intern_child(scope, prefix_name)
            .expect("prefix declaration path should intern"),
        prefix_value,
        &expression_builder,
    )];

    let file_tokens = template_tokens_from_source(
        "[$before prefix:[:child]]",
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
    let context = constant_template_context(&source_path, &declarations, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("$before should accept const-folded references");

    let wrappers = wrapper_contexts(&template, &context);
    assert_eq!(wrappers.len(), 1);
    {
        let store = context.template_ir_store.borrow();
        let composite = store
            .get_template(wrappers[0].root)
            .expect("composite wrapper TIR should exist in the module store");
        let TemplateIrNodeKind::Sequence { children } = &store
            .get_node(composite.root)
            .expect("composite root should exist")
            .kind
        else {
            panic!("composite decorator wrapper should have a sequence root");
        };
        assert_eq!(
            children.len(),
            2,
            "a prefix-only composite holds the prefix reference and the implicit slot"
        );
        assert!(
            matches!(
                store
                    .get_node(children[0])
                    .expect("prefix child exists")
                    .kind,
                TemplateIrNodeKind::ChildTemplate { .. }
            ),
            "first composite child should reference the prefix wrapper"
        );
        assert!(
            matches!(
                store.get_node(children[1]).expect("slot child exists").kind,
                TemplateIrNodeKind::Slot { .. }
            ),
            "second composite child should be the implicit default slot"
        );
    }

    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    assert_eq!(
        string_table.resolve(folded),
        "prefix: child",
        "the const String reference must supply the composite prefix at folding"
    );
}

#[test]
fn before_directive_resolves_stored_constant_wrapper_in_runtime_template() {
    use crate::compiler_frontend::ast::module_ast::environment::ResolvedConstantSet;

    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let shared_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let style_directives = frontend_test_style_directives();

    // Stored wrapper constant with one named slot. Its TIR reference is the identity the
    // decorator must retain when it resolves the runtime `prefix` reference.
    let prefix_scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");
    let prefix_file_tokens = template_tokens_from_source(
        "[:<b style=\"[$slot tone]\">]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let prefix_source_path = prefix_file_tokens.source_path;
    let prefix_context = new_constant_context(prefix_source_path, &path_fork)
        .with_template_ir_store(Rc::clone(&shared_store));
    let canonical_owner = prefix_file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut prefix_tokens = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    prefix_tokens
        .set_position(prefix_file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let prefix = Template::new(
        &mut prefix_tokens,
        prefix_source_path,
        &prefix_context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("stored prefix wrapper should parse");
    let prefix_reference = prefix.tir_reference;

    let prefix_name = string_table.intern("prefix");
    let prefix_path = path_fork
        .try_intern_child(prefix_scope, prefix_name)
        .expect("prefix declaration path should intern");
    let prefix_expression = expression_builder
        .template(prefix, ValueMode::ImmutableOwned)
        .expect("prefix template expression should construct")
        .id();
    let declaration = make_test_variable(prefix_path, prefix_expression, &expression_builder);

    let declarations = Rc::new(TopLevelDeclarationTable::new(vec![declaration], &path_fork));
    let declaration_id = declarations
        .declaration_id_by_path(&prefix_path)
        .expect("fixture declaration should be table-resolved");
    let mut resolved_constants = ResolvedConstantSet::default();
    resolved_constants.insert(declaration_id);

    // The consumer is an ordinary runtime-capable template, not a constant context.
    let consumer_scope = path_fork
        .try_intern_portable_path("main.moth/#runtime_template0", &mut string_table)
        .expect("test path fits");
    let consumer_context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            consumer_scope,
            declarations,
            Arc::new(ExternalPackageRegistry::default()),
            0,
        )
        .with_template_ir_store(Rc::clone(&shared_store))
        .with_resolved_module_constants(Rc::new(resolved_constants)),
        &consumer_scope,
        &style_directives,
    );

    let file_tokens = template_tokens_from_source(
        "[$before prefix: [: child]]",
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

    let template = Template::new(
        &mut token_stream,
        source_path,
        &consumer_context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("$before should resolve a stored constant wrapper in a runtime template");

    let wrappers = wrapper_contexts(&template, &consumer_context);
    assert_eq!(wrappers.len(), 1);
    {
        let store = consumer_context.template_ir_store.borrow();
        let composite = store
            .get_template(wrappers[0].root)
            .expect("composite wrapper TIR should exist in the module store");
        let TemplateIrNodeKind::Sequence { children } = &store
            .get_node(composite.root)
            .expect("composite root should exist")
            .kind
        else {
            panic!("composite decorator wrapper should have a sequence root");
        };
        assert_eq!(
            children.len(),
            2,
            "a prefix-only composite holds the referenced prefix and the implicit slot"
        );
        let TemplateIrNodeKind::ChildTemplate { reference, .. } = &store
            .get_node(children[0])
            .expect("prefix child exists")
            .kind
        else {
            panic!("first composite child should reference the stored prefix wrapper");
        };
        assert_eq!(
            reference.root, prefix_reference.root,
            "the stored reference must keep its exact TIR root"
        );
        assert_eq!(
            reference.phase, prefix_reference.phase,
            "the stored reference must keep its exact TIR phase"
        );
        assert_eq!(
            reference.context, prefix_reference.context,
            "the stored reference must keep its exact TIR view context"
        );

        let layout = collect_tir_slot_layout(&store, wrappers[0].root)
            .expect("composite decorator layout should resolve");
        assert!(
            layout.schema.has_default_slot,
            "the composite must expose one implicit default child slot"
        );
        let tone = string_table.intern("tone");
        assert!(
            layout.schema.named_slots.contains(&tone),
            "the stored wrapper's named slot must stay addressable through the composite"
        );
    }
}

#[test]
fn before_directive_rejects_runtime_values() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[$before value: [: child]]",
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
    let context = runtime_template_context(
        &source_path,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );

    let error = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("$before should reject runtime values");
    let error = expect_template_diagnostic(error);

    let facts = error
        .first()
        .expect("decorator error should compact to one record")
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("decorator error should retain typed directive facts");
    assert_eq!(
        facts.reason,
        InvalidTemplateDirectiveCode::InvalidChildDecoratorValue
    );
}

#[test]
fn before_directive_rejects_non_string_values() {
    let error = template_parse_error("[$before 42: [: child]]");
    assert!(
        error.contains(
            "`$before` requires one bare compile-time `String` or constant template wrapper."
        ),
        "unexpected non-string value diagnostic: {error}"
    );
}

#[test]
fn before_directive_requires_operand() {
    let error = template_parse_error("[$before: body]");
    assert!(
        error.contains(
            "`$before` requires one bare compile-time `String` or constant template wrapper."
        ),
        "unexpected missing-operand diagnostic: {error}"
    );
}

#[test]
fn decorator_value_rejects_default_slot_at_its_own_span() {
    let source = "[$before [:<b>[$slot]</b>]: [: child]]";
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
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
    let context = new_constant_context(source_path, &path_fork);

    let error = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("the default slot inside a decorator value must be rejected");
    let error = expect_template_diagnostic(error);
    let first = error
        .first()
        .expect("default-slot diagnostic should compact to one record");
    let facts = first
        .family_view::<InvalidTemplateSlotView>()
        .expect("default-slot rejection should retain slot facts");
    assert_eq!(
        facts.reason,
        InvalidTemplateSlotReason::DefaultSlotInChildDecorator
    );
    let SiteRef::Local(primary_span) = first.primary_site() else {
        panic!("default-slot rejection should keep its local authored span");
    };
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    let range_start = range.start() as usize;
    let range_end = range.end() as usize;
    let slot_start = source
        .find("[$slot]")
        .expect("fixture authors one default slot occurrence");
    let slot_end = slot_start + "[$slot]".len();
    // Template construction owns the exact extent of a slot occurrence span; the
    // diagnostic contract is that it anchors at that authored occurrence rather
    // than at the decorator value or the enclosing template.
    assert_eq!(
        range_start, slot_start,
        "the diagnostic must anchor at the offending slot occurrence"
    );
    assert!(
        range_end > range_start && range_end <= slot_end,
        "the diagnostic span {range_start}..{range_end} must stay inside the offending slot occurrence {slot_start}..{slot_end}"
    );
}

#[test]
fn decorator_value_rejects_positional_slot() {
    let error = template_parse_error("[$after [: [$slot 1] ]: [: child]]");
    assert!(
        error.contains("A `$before` or `$after` value may only contain named slots."),
        "unexpected positional-slot diagnostic: {error}"
    );
}

#[test]
fn duplicate_before_directive_is_rejected_in_one_head() {
    let error = template_parse_error("[$before [: a], $before [: b]: body]");
    assert!(
        error.contains("`$before` cannot appear more than once in a template head."),
        "unexpected duplicate-before reason: {error}"
    );
}

#[test]
fn duplicate_after_directive_is_rejected_in_one_head() {
    let error = template_parse_error("[$after [: a], $after [: b]: body]");
    assert!(
        error.contains("`$after` cannot appear more than once in a template head."),
        "unexpected duplicate-after reason: {error}"
    );
}

#[test]
fn both_decorators_wrap_each_direct_child() {
    let rendered = folded_template_output("[$before [:<b>], $after [:</b>]:[:first][:second]]");

    assert_eq!(
        rendered, "<b>first</b><b>second</b>",
        "each direct child must receive its own prefix and suffix"
    );
}

#[test]
fn string_before_wrapper_prepends_to_child() {
    let rendered = folded_template_output("[$before \"prefix: \":[:child]]");
    assert_eq!(rendered, "prefix: child");
}

#[test]
fn decorators_do_not_apply_to_grandchildren() {
    let rendered = folded_template_output("[$before [:<wrap>]:[:[:body]]]");

    assert_eq!(rendered, "<wrap>body");
    assert_eq!(rendered.matches("<wrap>").count(), 1);
}

#[test]
fn nested_inline_templates_inside_table_cells_do_not_become_extra_cells() {
    let rendered = folded_template_output(
        "[
            $before [:<tr>], $after [:</tr>]:
            [$before [:<td>], $after [:</td>]:
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
fn decorator_wrapper_named_slot_stays_addressable() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let (template, context) = parse_decorator_template(
        "[$before [:<tr style=\"[$slot row_style]\">], $after [:</tr>]: [$slot]]",
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
        &mut span_builder,
    );

    let store = context.template_ir_store.borrow();
    let template_root = template.tir_reference.root;
    let layout =
        collect_tir_slot_layout(&store, template_root).expect("receiver layout should resolve");
    let row_style = string_table.intern("row_style");
    assert!(
        layout.schema.named_slots.contains(&row_style),
        "a named slot inside a decorator value must stay addressable from the receiver"
    );
    assert!(
        layout.schema.has_default_slot,
        "the receiver's own default slot must remain addressable"
    );

    let wrapper_reference = wrapper_reference_for_template(&template, &store)
        .expect("receiver template should normalize as a wrapper");
    assert_eq!(wrapper_reference.root, template_root);
}
