//! Tests for AST reactive-template metadata propagation.
//!
//! WHAT: exercises the one-store reactive metadata pass: subscription
//! propagation, call-argument parameter rebasing, prior-declaration references,
//! non-inheritance through runtime string operations, branch/fallback/loop body
//! overlay composition, selector/body overlay precedence, option-capture
//! bodies, sink operands, and runtime slot-site render pieces.
//! WHY: reactive metadata is the value-level contract between finalized AST
//!      templates and the reactive backend. One `TemplateIrStore` owns every
//!      TIR root and overlay payload used here.

use super::propagate_reactive_template_metadata_in_ast;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotSite, OwnedRuntimeSlotSiteRenderPiece,
    OwnedRuntimeSlotSiteRenderPlan, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId;
use crate::compiler_frontend::ast::templates::tir::TirExpressionOverlayId;
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIr, TemplateIrBranch, TemplateIrId, TemplateIrNode, TemplateIrNodeId,
    TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary, TemplateLoopHeaderExpressionSites,
    TemplateTirPhase, TemplateTirReference, TemplateViewContext, TirExpressionOverlay,
    refs::TemplateTirChildReference,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, node, symbol, test_location,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

// -------------------------
//  Shared construction helpers
// -------------------------

/// Builds a `Template` expression backed by a one-store Composed TIR root.
fn template_with_reference(reference: TemplateTirReference, location: SourceLocation) -> Template {
    Template {
        tir_reference: reference,
        location,
    }
}

/// Builds a module-local Composed template reference for `template_id`.
fn composed_reference(template_id: TemplateIrId) -> TemplateTirReference {
    TemplateTirReference {
        root: template_id,
        phase: TemplateTirPhase::Composed,
        context: TemplateViewContext::default(),
    }
}

fn string_return_slot() -> ReturnSlot {
    ReturnSlot {
        value: DataType::StringSlice,
        type_id: Some(builtin_type_ids::STRING),
        reactive_template: None,
        channel: ReturnChannel::Success,
    }
}

fn no_value_declaration(
    path: InternedPath,
    data_type: DataType,
    type_id: crate::compiler_frontend::datatypes::TypeId,
) -> Declaration {
    Declaration {
        id: path,
        value: Expression::no_value_with_type_id(
            test_location(1),
            data_type,
            type_id,
            ValueMode::ImmutableReference,
        ),
    }
}

fn reference_expression(
    path: InternedPath,
    data_type: DataType,
    type_id: crate::compiler_frontend::datatypes::TypeId,
) -> Expression {
    Expression::reference_with_type_id(
        path,
        data_type,
        type_id,
        test_location(1),
        ValueMode::ImmutableReference,
        ConstRecordState::RuntimeValue,
    )
}

fn reactive_source(path: InternedPath, kind: ReactiveSourceKind) -> ReactiveSource {
    ReactiveSource { path, kind }
}

/// Builds a template expression carrying one reactive subscription on its
/// single dynamic-expression body root.
fn template_with_subscription(store: &mut TemplateIrStore, source: ReactiveSource) -> Expression {
    template_expression_from_tir(
        store,
        reference_expression(source.path.clone(), DataType::Int, builtin_type_ids::INT)
            .with_reactive_source(source.clone()),
        Some(ReactiveSubscription {
            source,
            type_id: builtin_type_ids::INT,
            location: test_location(2),
        }),
    )
}

/// Builds a Composed template expression from a body expression and an
/// optional reactive subscription attached to the dynamic-expression node.
fn template_expression_from_tir(
    store: &mut TemplateIrStore,
    expression: Expression,
    reactive_subscription: Option<ReactiveSubscription>,
) -> Expression {
    let location = test_location(2);
    let site_id = store.next_expression_site_id();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription,
            site_id,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));
    Expression::template(
        template_with_reference(composed_reference(template_id), location.clone()),
        ValueMode::ImmutableOwned,
    )
}

/// Builds a single dynamic-expression body root and returns its node id and the
/// allocated expression-site id.
fn single_expression_body_root(
    store: &mut TemplateIrStore,
    expression: Expression,
    location: &SourceLocation,
) -> (TemplateIrNodeId, ExpressionSiteId) {
    let site_id = store.next_expression_site_id();
    let expr_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(expression),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        location.clone(),
    ));
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: vec![expr_node],
        },
        location.clone(),
    ));
    (root, site_id)
}

/// Reads the annotated expression for `site_id` from the template reference's
/// composed root expression overlay, returning its reactive metadata.
fn root_overlay_expression_metadata<'a>(
    store: &'a TemplateIrStore,
    template: &Template,
    site_id: ExpressionSiteId,
) -> Option<&'a ReactiveTemplateMetadata> {
    let expression_overlay_id = template.tir_reference.context.expression_overlay?;
    let expression_overlay = store.expression_overlay(expression_overlay_id)?;
    let expression = expression_overlay.expression_for_site(site_id)?;
    expression.reactive_template.as_ref()
}

fn call_expression(function_path: InternedPath, arguments: Vec<CallArgument>) -> Expression {
    let mut type_environment = TypeEnvironment::new();
    Expression::function_call_with_typed_arguments(
        function_path,
        arguments,
        vec![builtin_type_ids::STRING],
        &mut type_environment,
        test_location(3),
    )
}

fn first_expression_statement_metadata(ast: &[AstNode]) -> &ReactiveTemplateMetadata {
    let NodeKind::ExpressionStatement(expression) = &ast[1].kind else {
        panic!("expected expression statement node");
    };
    expression
        .reactive_template
        .as_ref()
        .expect("expected reactive template metadata")
}

fn template_from_expression_statement(ast: &[AstNode]) -> &Template {
    let NodeKind::ExpressionStatement(expression) = &ast[0].kind else {
        panic!("expected template expression statement");
    };
    let ExpressionKind::Template(template) = &expression.kind else {
        panic!("expected template expression");
    };
    template
}

// -------------------------
//  Module-local subscription propagation
// -------------------------

#[test]
fn propagates_metadata_from_a_template() {
    let mut strings = StringTable::new();
    let source_path = symbol("count", &mut strings);
    let mut store = TemplateIrStore::new();
    let expression = template_with_subscription(
        &mut store,
        reactive_source(source_path.clone(), ReactiveSourceKind::Declaration),
    );
    let mut ast = vec![node(
        NodeKind::ExpressionStatement(expression),
        test_location(1),
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive metadata propagation should succeed");

    let NodeKind::ExpressionStatement(expression) = &ast[0].kind else {
        panic!("expected expression statement");
    };
    let metadata = expression
        .reactive_template
        .as_ref()
        .expect("template should receive reactive metadata");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, source_path);
}

#[test]
fn annotates_linear_tir_root_metadata_through_overlay() {
    let mut strings = StringTable::new();
    let count_path = symbol("count", &mut strings);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let source_expression = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let (root, site_id) = single_expression_body_root(&mut store, source_expression, &location);
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(
            template_with_reference(composed_reference(template_id), location.clone()),
            ValueMode::ImmutableOwned,
        )),
        location,
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let template = template_from_expression_statement(&ast);
    let metadata = root_overlay_expression_metadata(&store, template, site_id)
        .expect("expected TIR expression overlay metadata");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

#[test]
fn runtime_string_operations_do_not_inherit_nested_template_metadata() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let template = template_with_subscription(
        &mut store,
        reactive_source(
            symbol("count", &mut string_table),
            ReactiveSourceKind::Declaration,
        ),
    );
    assert!(
        template.reactive_template.is_none(),
        "template metadata should be deferred until store-aware finalization"
    );

    let runtime_expression = Expression::runtime_with_type_id(
        ExpressionRpn {
            items: vec![ExpressionRpnItem::Operand(template)],
        },
        DataType::StringSlice,
        builtin_type_ids::STRING,
        test_location(1),
        ValueMode::ImmutableOwned,
    );

    let mut ast = vec![node(
        NodeKind::ExpressionStatement(runtime_expression),
        test_location(1),
    )];
    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(expression) = &ast[0].kind else {
        panic!("expected expression statement node");
    };
    assert!(expression.reactive_template.is_none());
}

#[test]
fn plain_string_expression_does_not_gain_template_metadata() {
    let mut store = TemplateIrStore::new();
    let mut strings = StringTable::new();
    let expression = Expression::string_slice(
        strings.intern("plain"),
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    );
    let mut ast = vec![node(
        NodeKind::ExpressionStatement(expression),
        test_location(1),
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("plain expression propagation should succeed");

    let NodeKind::ExpressionStatement(expression) = &ast[0].kind else {
        panic!("expected expression statement");
    };
    assert!(expression.reactive_template.is_none());
}

// -------------------------
//  Call-argument parameter rebasing and substitution
// -------------------------

#[test]
fn rebases_reactive_parameter_subscription_to_call_argument_source() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let function_path = symbol("render_count", &mut string_table);
    let parameter_path = symbol("source", &mut string_table);
    let count_path = symbol("count", &mut string_table);

    let mut parameter =
        no_value_declaration(parameter_path.clone(), DataType::Int, builtin_type_ids::INT);
    parameter.value.reactive_source = Some(reactive_source(
        parameter_path.clone(),
        ReactiveSourceKind::Parameter,
    ));

    let signature = FunctionSignature {
        parameters: vec![parameter],
        returns: vec![string_return_slot()],
    };
    let body = vec![node(
        NodeKind::Return(vec![template_with_subscription(
            &mut store,
            reactive_source(parameter_path, ReactiveSourceKind::Parameter),
        )]),
        test_location(2),
    )];

    let mut argument =
        reference_expression(count_path.clone(), DataType::Int, builtin_type_ids::INT)
            .with_reactive_source(reactive_source(
                count_path.clone(),
                ReactiveSourceKind::Declaration,
            ));
    argument.reactive_template = None;

    let mut ast = vec![
        function_node(function_path.clone(), signature, body, test_location(1)),
        node(
            NodeKind::ExpressionStatement(call_expression(
                function_path,
                vec![CallArgument::positional(
                    argument,
                    CallAccessMode::Shared,
                    test_location(3),
                )],
            )),
            test_location(3),
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let metadata = first_expression_statement_metadata(&ast);
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
    assert_eq!(
        metadata.subscriptions[0].source.kind,
        ReactiveSourceKind::Declaration
    );
}

#[test]
fn substitutes_string_parameter_template_value_from_call_argument() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let function_path = symbol("wrap", &mut string_table);
    let parameter_path = symbol("content", &mut string_table);
    let count_path = symbol("count", &mut string_table);

    let mut parameter = no_value_declaration(
        parameter_path.clone(),
        DataType::StringSlice,
        builtin_type_ids::STRING,
    );
    parameter.value.reactive_template =
        Some(ReactiveTemplateMetadata::from_template_value_parameter(
            parameter_path.clone(),
            test_location(1),
        ));

    let mut inserted_parameter = reference_expression(
        parameter_path.clone(),
        DataType::StringSlice,
        builtin_type_ids::STRING,
    );
    inserted_parameter.reactive_template = parameter.value.reactive_template.clone();

    let wrapper_template = template_expression_from_tir(&mut store, inserted_parameter, None);

    let signature = FunctionSignature {
        parameters: vec![parameter],
        returns: vec![string_return_slot()],
    };
    let body = vec![node(
        NodeKind::Return(vec![wrapper_template]),
        test_location(2),
    )];

    let argument_template = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );

    let mut ast = vec![
        function_node(function_path.clone(), signature, body, test_location(1)),
        node(
            NodeKind::ExpressionStatement(call_expression(
                function_path,
                vec![CallArgument::positional(
                    argument_template,
                    CallAccessMode::Shared,
                    test_location(3),
                )],
            )),
            test_location(3),
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let metadata = first_expression_statement_metadata(&ast);
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
    assert!(metadata.template_value_parameters.is_empty());
}

#[test]
fn references_use_metadata_computed_for_prior_declarations() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let function_path = symbol("render_count", &mut string_table);
    let parameter_path = symbol("source", &mut string_table);
    let count_path = symbol("count", &mut string_table);
    let view_path = symbol("view", &mut string_table);

    let mut parameter =
        no_value_declaration(parameter_path.clone(), DataType::Int, builtin_type_ids::INT);
    parameter.value.reactive_source = Some(reactive_source(
        parameter_path.clone(),
        ReactiveSourceKind::Parameter,
    ));

    let signature = FunctionSignature {
        parameters: vec![parameter],
        returns: vec![string_return_slot()],
    };
    let body = vec![node(
        NodeKind::Return(vec![template_with_subscription(
            &mut store,
            reactive_source(parameter_path, ReactiveSourceKind::Parameter),
        )]),
        test_location(2),
    )];

    let argument = reference_expression(count_path.clone(), DataType::Int, builtin_type_ids::INT)
        .with_reactive_source(reactive_source(
            count_path.clone(),
            ReactiveSourceKind::Declaration,
        ));
    let declaration = Declaration {
        id: view_path.clone(),
        value: call_expression(
            function_path.clone(),
            vec![CallArgument::positional(
                argument,
                CallAccessMode::Shared,
                test_location(3),
            )],
        ),
    };

    let mut ast = vec![
        function_node(function_path, signature, body, test_location(1)),
        node(NodeKind::VariableDeclaration(declaration), test_location(3)),
        node(
            NodeKind::ExpressionStatement(reference_expression(
                view_path,
                DataType::StringSlice,
                builtin_type_ids::STRING,
            )),
            test_location(4),
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(expression) = &ast[2].kind else {
        panic!("expected expression statement reference");
    };
    let metadata = expression
        .reactive_template
        .as_ref()
        .expect("expected reference to use declaration metadata");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

// -------------------------
//  Control-flow body overlay composition
// -------------------------

#[test]
fn annotates_branch_body_tir_root_metadata() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let body_expression = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let (body_root, body_site_id) =
        single_expression_body_root(&mut store, body_expression, &location);

    let selector_site_id = store.next_expression_site_id();
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            true,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        body_root,
        location.clone(),
    )
    .with_selector_site_id(selector_site_id);

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(
            template_with_reference(composed_reference(template_id), location.clone()),
            ValueMode::ImmutableOwned,
        )),
        location,
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let template = template_from_expression_statement(&ast);
    let metadata = root_overlay_expression_metadata(&store, template, body_site_id)
        .expect("expected TIR root expression overlay metadata for branch body");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

#[test]
fn annotates_fallback_body_tir_root_metadata() {
    let mut string_table = StringTable::new();
    let fallback_path = symbol("fallback_count", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let branch_body_node = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Text {
            text: string_table.intern("branch"),
            byte_len: 6,
            origin: TemplateSegmentOrigin::Body,
        },
        location.clone(),
    ));

    let fallback_expression = template_with_subscription(
        &mut store,
        reactive_source(fallback_path.clone(), ReactiveSourceKind::Declaration),
    );
    let (fallback_body_root, fallback_site_id) =
        single_expression_body_root(&mut store, fallback_expression, &location);

    let selector_site_id = store.next_expression_site_id();
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(Expression::bool(
            true,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        branch_body_node,
        location.clone(),
    )
    .with_selector_site_id(selector_site_id);

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: Some(fallback_body_root),
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(
            template_with_reference(composed_reference(template_id), location.clone()),
            ValueMode::ImmutableOwned,
        )),
        location,
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let template = template_from_expression_statement(&ast);
    let metadata = root_overlay_expression_metadata(&store, template, fallback_site_id)
        .expect("expected TIR root expression overlay metadata for fallback body");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, fallback_path);
}

#[test]
fn annotates_loop_body_tir_root_metadata() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let body_expression = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let (body_root, body_site_id) =
        single_expression_body_root(&mut store, body_expression, &location);

    let condition_site_id = store.next_expression_site_id();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Conditional {
                condition: Box::new(Expression::bool(
                    true,
                    location.clone(),
                    ValueMode::ImmutableOwned,
                )),
            },
            header_sites: TemplateLoopHeaderExpressionSites::Conditional {
                condition: condition_site_id,
            },
            body: body_root,
            aggregate_wrapper: None,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(
            template_with_reference(composed_reference(template_id), location.clone()),
            ValueMode::ImmutableOwned,
        )),
        location,
    )];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let template = template_from_expression_statement(&ast);
    let metadata = root_overlay_expression_metadata(&store, template, body_site_id)
        .expect("expected TIR root expression overlay metadata for loop body");
    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

#[test]
fn annotates_branch_selector_and_body_through_one_root_overlay() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let show_path = symbol("show", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let show_template = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let show_declaration = Declaration {
        id: show_path.clone(),
        value: show_template,
    };

    let body_expression = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let (body_root, body_site_id) =
        single_expression_body_root(&mut store, body_expression, &location);

    let selector_expression = reference_expression(
        show_path.clone(),
        DataType::StringSlice,
        builtin_type_ids::STRING,
    );
    let selector_site_id = store.next_expression_site_id();
    let branch = TemplateIrBranch::new(
        TemplateBranchSelector::Bool(selector_expression),
        body_root,
        location.clone(),
    )
    .with_selector_site_id(selector_site_id);

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![
        node(
            NodeKind::VariableDeclaration(show_declaration),
            test_location(1),
        ),
        node(
            NodeKind::ExpressionStatement(Expression::template(
                template_with_reference(composed_reference(template_id), location.clone()),
                ValueMode::ImmutableOwned,
            )),
            location,
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(Expression {
        kind: ExpressionKind::Template(template),
        ..
    }) = &ast[1].kind
    else {
        panic!("expected template expression statement at ast[1]");
    };

    let selector_metadata = root_overlay_expression_metadata(&store, template, selector_site_id)
        .expect("expected TIR root expression overlay metadata for branch selector");
    assert_eq!(
        selector_metadata.subscriptions.len(),
        1,
        "branch selector reference should inherit reactive metadata from the prior declaration through the root overlay"
    );
    assert_eq!(selector_metadata.subscriptions[0].source.path, count_path);

    let body_metadata = root_overlay_expression_metadata(&store, template, body_site_id)
        .expect("expected TIR root expression overlay metadata for branch body");
    assert_eq!(body_metadata.subscriptions.len(), 1);
    assert_eq!(body_metadata.subscriptions[0].source.path, count_path);
}

#[test]
fn option_capture_body_uses_scrutinee_reactive_metadata() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let optional_path = symbol("optional", &mut string_table);
    let capture_path = symbol("captured", &mut string_table);
    let capture_name = string_table.intern("captured");
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let optional_declaration = Declaration {
        id: optional_path.clone(),
        value: template_with_subscription(
            &mut store,
            reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
        ),
    };

    let body_site_id = store.next_expression_site_id();
    let body_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(reference_expression(
                capture_path.clone(),
                DataType::StringSlice,
                builtin_type_ids::STRING,
            )),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: body_site_id,
        },
        location.clone(),
    ));

    let selector_site_id = store.next_expression_site_id();
    let selector = TemplateBranchSelector::OptionPresentCapture {
        scrutinee: reference_expression(
            optional_path,
            DataType::StringSlice,
            builtin_type_ids::STRING,
        ),
        pattern: Box::new(MatchPattern::OptionPresentCapture {
            name: capture_name,
            binding_path: capture_path,
            inner_type_id: builtin_type_ids::STRING,
            location: location.clone(),
            binding_location: location.clone(),
        }),
    };
    let branch = TemplateIrBranch::new(selector, body_root, location.clone())
        .with_selector_site_id(selector_site_id);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::BranchChain {
            branches: vec![branch],
            fallback: None,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let mut ast = vec![
        node(
            NodeKind::VariableDeclaration(optional_declaration),
            test_location(1),
        ),
        node(
            NodeKind::ExpressionStatement(Expression::template(
                template_with_reference(composed_reference(template_id), location.clone()),
                ValueMode::ImmutableOwned,
            )),
            location,
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(Expression {
        kind: ExpressionKind::Template(template),
        ..
    }) = &ast[1].kind
    else {
        panic!("expected template expression statement at ast[1]");
    };
    let metadata = root_overlay_expression_metadata(&store, template, body_site_id)
        .expect("option-capture body reference should inherit scrutinee metadata");

    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

// -------------------------
//  Effective expression overrides
// -------------------------

#[test]
fn annotates_existing_effective_expression_override_instead_of_structural_payload() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let show_path = symbol("show", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let show_declaration = Declaration {
        id: show_path.clone(),
        value: template_with_subscription(
            &mut store,
            reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
        ),
    };

    let site_id = store.next_expression_site_id();
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(Expression::bool(
                false,
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id,
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let expression_overlay_id = store.allocate_expression_overlay(TirExpressionOverlay {
        overrides: vec![(
            site_id,
            Box::new(reference_expression(
                show_path,
                DataType::StringSlice,
                builtin_type_ids::STRING,
            )),
        )],
    });
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        location.clone(),
    );

    let mut ast = vec![
        node(
            NodeKind::VariableDeclaration(show_declaration),
            test_location(1),
        ),
        node(
            NodeKind::ExpressionStatement(Expression::template(
                template,
                ValueMode::ImmutableOwned,
            )),
            location,
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(Expression {
        kind: ExpressionKind::Template(template),
        ..
    }) = &ast[1].kind
    else {
        panic!("expected template expression statement at ast[1]");
    };
    let metadata = root_overlay_expression_metadata(&store, template, site_id)
        .expect("existing effective expression override should be annotated");

    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

#[test]
fn annotates_existing_child_expression_override() {
    let mut string_table = StringTable::new();
    let count_path = symbol("count", &mut string_table);
    let show_path = symbol("show", &mut string_table);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let show_declaration = Declaration {
        id: show_path.clone(),
        value: template_with_subscription(
            &mut store,
            reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
        ),
    };

    let child_site_id = store.next_expression_site_id();
    let child_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::DynamicExpression {
            expression: Box::new(Expression::bool(
                false,
                location.clone(),
                ValueMode::ImmutableOwned,
            )),
            origin: TemplateSegmentOrigin::Body,
            reactive_subscription: None,
            site_id: child_site_id,
        },
        location.clone(),
    ));
    let child_template_id = store.push_template(TemplateIr::new(
        child_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let child_expression_overlay_id = store.allocate_expression_overlay(TirExpressionOverlay {
        overrides: vec![(
            child_site_id,
            Box::new(reference_expression(
                show_path,
                DataType::StringSlice,
                builtin_type_ids::STRING,
            )),
        )],
    });
    let child_context = TemplateViewContext {
        expression_overlay: Some(child_expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    let root_context = TemplateViewContext::default();

    let child_reference = TemplateTirChildReference::new(
        child_template_id,
        TemplateTirPhase::Composed,
        child_context,
    );
    let child_occurrence_id = store.next_child_template_occurrence_id();
    let parent_root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::ChildTemplate {
            reference: child_reference,
            occurrence_id: child_occurrence_id,
        },
        location.clone(),
    ));
    let parent_template_id = store.push_template(TemplateIr::new(
        parent_root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));

    let template = template_with_reference(
        TemplateTirReference {
            root: parent_template_id,
            phase: TemplateTirPhase::Composed,
            context: root_context,
        },
        location.clone(),
    );
    let mut ast = vec![
        node(
            NodeKind::VariableDeclaration(show_declaration),
            test_location(1),
        ),
        node(
            NodeKind::ExpressionStatement(Expression::template(
                template,
                ValueMode::ImmutableOwned,
            )),
            location,
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::ExpressionStatement(Expression {
        kind: ExpressionKind::Template(template),
        ..
    }) = &ast[1].kind
    else {
        panic!("expected template expression statement at ast[1]");
    };
    let metadata = root_overlay_expression_metadata(&store, template, child_site_id)
        .expect("child effective override should be annotated on the root overlay");

    assert_eq!(metadata.subscriptions.len(), 1);
    assert_eq!(metadata.subscriptions[0].source.path, count_path);
}

// -------------------------
//  Sink operands and runtime slot-site render pieces
// -------------------------

#[test]
fn sink_operand_expressions_keep_reactive_template_metadata() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let count_path = symbol("count", &mut string_table);
    let fragment_template = template_with_subscription(
        &mut store,
        reactive_source(count_path.clone(), ReactiveSourceKind::Declaration),
    );
    let host_call_template = template_with_subscription(
        &mut store,
        reactive_source(count_path, ReactiveSourceKind::Declaration),
    );

    let mut ast = vec![
        node(
            NodeKind::PushStartRuntimeFragment(fragment_template),
            test_location(1),
        ),
        node(
            NodeKind::ExpressionStatement({
                let mut type_environment = TypeEnvironment::new();
                Expression::host_function_call_with_typed_arguments(
                    ExternalFunctionId::IoLine,
                    vec![CallArgument::positional(
                        host_call_template,
                        CallAccessMode::Shared,
                        test_location(2),
                    )],
                    Vec::new(),
                    &mut type_environment,
                    test_location(2),
                )
            }),
            test_location(2),
        ),
    ];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("reactive template metadata propagation should succeed");

    let NodeKind::PushStartRuntimeFragment(fragment) = &ast[0].kind else {
        panic!("expected runtime fragment push");
    };
    assert!(fragment.reactive_template.is_some());

    let NodeKind::ExpressionStatement(Expression {
        kind: ExpressionKind::HostFunctionCall { args, .. },
        ..
    }) = &ast[1].kind
    else {
        panic!("expected host function call");
    };
    assert!(args[0].value.reactive_template.is_some());
}

#[test]
fn propagates_metadata_through_runtime_slot_site_render_piece() {
    let mut strings = StringTable::new();
    let source_path = symbol("count", &mut strings);
    let location = test_location(2);
    let mut store = TemplateIrStore::new();

    let inner_expression = template_with_subscription(
        &mut store,
        reactive_source(source_path.clone(), ReactiveSourceKind::Declaration),
    );

    let render_piece_node = OwnedRuntimeTemplateNode::DynamicExpression {
        expression: Box::new(inner_expression),
        reactive_subscription: None,
    };

    let slot_site = OwnedRuntimeSlotSite {
        site: RuntimeSlotSiteId(0),
        render_plan: OwnedRuntimeSlotSiteRenderPlan {
            pieces: vec![OwnedRuntimeSlotSiteRenderPiece::Render(render_piece_node)],
        },
        location: location.clone(),
    };

    let handoff = OwnedRuntimeSlotApplicationHandoff {
        wrapper: OwnedRuntimeTemplateNode::RuntimeSlotSite {
            site: RuntimeSlotSiteId(0),
        },
        contribution_sources: vec![],
        slot_sites: vec![slot_site],
        location: location.clone(),
    };

    let expression =
        Expression::runtime_slot_application_handoff(handoff, ValueMode::ImmutableOwned);
    let mut ast = vec![node(NodeKind::ExpressionStatement(expression), location)];

    propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect("runtime slot metadata propagation should succeed");

    let NodeKind::ExpressionStatement(expression) = &ast[0].kind else {
        panic!("expected expression statement");
    };
    let outer_metadata = expression
        .reactive_template
        .as_ref()
        .expect("handoff should receive reactive metadata");
    assert_eq!(outer_metadata.subscriptions.len(), 1);
    assert_eq!(outer_metadata.subscriptions[0].source.path, source_path);

    let ExpressionKind::RuntimeSlotApplicationHandoff(handoff) = &expression.kind else {
        panic!("expected runtime slot application handoff expression");
    };
    let site = handoff.slot_sites.first().expect("expected one slot site");
    let OwnedRuntimeSlotSiteRenderPiece::Render(node) = &site.render_plan.pieces[0] else {
        panic!("expected render piece");
    };
    let OwnedRuntimeTemplateNode::DynamicExpression {
        expression: inner, ..
    } = node
    else {
        panic!("expected dynamic expression node");
    };
    let inner_metadata = inner
        .reactive_template
        .as_ref()
        .expect("expected inner dynamic expression reactive template metadata");
    assert_eq!(
        inner_metadata.subscriptions.len(),
        1,
        "inner dynamic expression in slot-site render piece should keep its subscription"
    );
    assert_eq!(inner_metadata.subscriptions[0].source.path, source_path);
}

// -------------------------
//  Required authority rejection
// -------------------------

#[test]
fn reactive_annotation_rejects_missing_root_template() {
    let mut store = TemplateIrStore::new();
    let location = test_location(2);
    let template = template_with_reference(
        TemplateTirReference {
            root: TemplateIrId::new(99),
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        location.clone(),
    );
    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(template, ValueMode::ImmutableOwned)),
        location,
    )];

    let error = propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect_err("missing root template authority must propagate");

    assert!(format!("{error:?}").contains("root"));
}

#[test]
fn reactive_annotation_rejects_missing_root_view_context() {
    let mut store = TemplateIrStore::new();
    let location = test_location(2);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: Vec::new(),
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));
    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext {
                expression_overlay: Some(TirExpressionOverlayId::new(99)),
                ..TemplateViewContext::default()
            },
        },
        location.clone(),
    );
    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(template, ValueMode::ImmutableOwned)),
        location,
    )];

    let error = propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect_err("missing root overlay authority must propagate");

    assert!(format!("{error:?}").contains("expression overlay"));
}

#[test]
fn reactive_annotation_rejects_missing_expression_overlay() {
    let mut store = TemplateIrStore::new();
    let location = test_location(2);
    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence {
            children: Vec::new(),
        },
        location.clone(),
    ));
    let template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::empty(),
        location.clone(),
    ));
    // Allocate a real expression overlay and reference it from the view context,
    // then drop the expression-overlay arena so the retained ID dangles. This
    // keeps the malformed state inside the test-owned store without adding a
    // production constructor.
    let expression_overlay_id = store.allocate_expression_overlay(TirExpressionOverlay::default());
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    store.expression_overlays.clear();

    let template = template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        location.clone(),
    );
    let mut ast = vec![node(
        NodeKind::ExpressionStatement(Expression::template(template, ValueMode::ImmutableOwned)),
        location,
    )];

    let error = propagate_reactive_template_metadata_in_ast(&mut ast, &mut store)
        .expect_err("missing expression overlay authority must propagate");

    assert!(
        format!("{error:?}").contains("expression overlay"),
        "expected a missing expression-overlay authority error, got: {:?}",
        error
    );
}
