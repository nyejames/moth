//! Tests for the shared AST const value resolver.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::facts::{
    ConstBindingScope, ConstBindingSource, ConstFactValueKind,
};
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    SlotOccurrenceId, TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase,
    TemplateTirReference, TemplateViewContext, TirSlotResolution, TirSlotResolutionOverlay,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

fn empty_location() -> SourceLocation {
    SourceLocation::default()
}

fn make_resolver<'a>(
    string_table: &'a mut StringTable,
    store: &mut TemplateIrStore,
) -> ConstValueResolver<'a> {
    ConstValueResolver::new(string_table, Rc::new(RefCell::new(std::mem::take(store))))
}

fn make_environment_with(
    path: &str,
    expression: Expression,
    string_table: &mut StringTable,
) -> ConstValueEnvironment {
    let mut env = ConstValueEnvironment::default();
    let interned_path = InternedPath::from_single_str(path, string_table);
    env.insert(interned_path, expression);
    env
}

fn rvalue_item(expression: Expression) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

fn operator_item(operator: Operator) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator {
        operator,
        location: SourceLocation::default(),
    }
}

// ------------------------------
//  Literal expression resolves
// ------------------------------

#[test]
fn literal_int_resolves_as_const() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let expression = Expression::int(42, empty_location(), ValueMode::ImmutableOwned);
    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("literal should resolve");

    assert!(matches!(result.kind, ExpressionKind::Int(42)));
}

#[test]
fn literal_string_resolves_as_const() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let string_id = string_table.intern("hello");
    let expression =
        Expression::string_slice(string_id, empty_location(), ValueMode::ImmutableOwned);
    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("literal should resolve");

    assert!(matches!(result.kind, ExpressionKind::StringSlice(_)));
}

// ------------------------------
//  Folded arithmetic resolves
// ------------------------------

#[test]
fn folded_arithmetic_resolves_to_literal() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::int(
                1,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            rvalue_item(Expression::int(
                2,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::Int,
        builtin_type_ids::INT,
        empty_location(),
        ValueMode::ImmutableOwned,
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("folded arithmetic should resolve");

    assert!(matches!(result.kind, ExpressionKind::Int(3)));
}

#[test]
fn folded_arithmetic_with_reference_substitution_resolves() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::reference(
                InternedPath::from_single_str("x", &mut string_table),
                DataType::Int,
                empty_location(),
                ValueMode::ImmutableReference,
            )),
            rvalue_item(Expression::int(
                5,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Multiply),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::Int,
        builtin_type_ids::INT,
        empty_location(),
        ValueMode::ImmutableOwned,
    );

    let env = make_environment_with(
        "x",
        Expression::int(3, empty_location(), ValueMode::ImmutableOwned),
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("substituted arithmetic should resolve");

    assert!(matches!(result.kind, ExpressionKind::Int(15)));
}

#[test]
fn folded_arithmetic_with_coerced_reference_substitution_resolves() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let reference = Expression::reference(
        InternedPath::from_single_str("x", &mut string_table),
        DataType::Int,
        empty_location(),
        ValueMode::ImmutableReference,
    );
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::coerced(reference, builtin_type_ids::INT)),
            rvalue_item(Expression::int(
                2,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::Int,
        builtin_type_ids::INT,
        empty_location(),
        ValueMode::ImmutableOwned,
    );

    let env = make_environment_with(
        "x",
        Expression::int(40, empty_location(), ValueMode::ImmutableOwned),
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("coerced reference arithmetic should resolve");

    assert!(matches!(result.kind, ExpressionKind::Int(42)));
}

// ------------------------------
//  Reference to known const resolves
// ------------------------------

#[test]
fn reference_to_known_const_resolves() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let path = InternedPath::from_single_str("ratio", &mut string_table);
    let expression = Expression::reference_with_type_id(
        path.clone(),
        DataType::Float,
        builtin_type_ids::FLOAT,
        empty_location(),
        ValueMode::ImmutableReference,
        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
    );

    let env = make_environment_with(
        "ratio",
        Expression::float(2.71, empty_location(), ValueMode::ImmutableOwned),
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("reference should resolve");

    assert!(
        matches!(result.kind, ExpressionKind::Float(value) if (value - 2.71).abs() < f64::EPSILON)
    );
}

// ------------------------------
//  Forward reference fails
// ------------------------------

#[test]
fn unresolved_reference_fails() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let path = InternedPath::from_single_str("unknown", &mut string_table);
    let expression = Expression::reference_with_type_id(
        path,
        DataType::Int,
        builtin_type_ids::INT,
        empty_location(),
        ValueMode::ImmutableReference,
        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("unresolved reference should fail");

    assert_eq!(error, ConstResolutionError::UnresolvedReference);
}

// ------------------------------
//  Function call fails
// ------------------------------

#[test]
fn function_call_fails_const_resolution() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let expression = Expression::function_call(
        InternedPath::from_single_str("foo", &mut string_table),
        vec![],
        vec![builtin_type_ids::INT],
        empty_location(),
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("function call should fail");

    assert_eq!(error, ConstResolutionError::CallInConstContext);
}

// ------------------------------
//  Mutable declaration fails
// ------------------------------

#[test]
fn mutable_declaration_fails_private_const_resolution() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let declaration = Declaration {
        id: InternedPath::from_single_str("value", &mut string_table),
        value: Expression::int(1, empty_location(), ValueMode::MutableOwned),
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let error = resolver
        .resolve_private_top_level_declaration(&declaration, &env)
        .expect_err("mutable declaration should fail");

    assert_eq!(error, ConstResolutionError::MutableDeclaration);
}

#[test]
fn explicit_top_level_constant_ignores_value_mode() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let declaration = Declaration {
        id: InternedPath::from_single_str("value", &mut string_table),
        value: Expression::int(1, empty_location(), ValueMode::MutableOwned),
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    // Explicit constants are const by syntax; the resolver does not check mutability.
    let fact = resolver
        .resolve_explicit_top_level_constant(&declaration, &env)
        .expect("explicit constant should resolve");

    assert_eq!(fact.scope, ConstBindingScope::ExplicitTopLevel);
    assert_eq!(fact.source, ConstBindingSource::ExplicitHash);
    assert!(matches!(
        fact.resolved_expression.kind,
        ExpressionKind::Int(1)
    ));
}

// ------------------------------
//  Fact value kinds
// ------------------------------

#[test]
fn fact_value_kind_from_literal_is_literal() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::Literal),
        ConstFactValueKind::Literal
    );
}

#[test]
fn fact_value_kind_from_runtime_is_non_const() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::NonConst),
        ConstFactValueKind::NonConst
    );
}

// ------------------------------
//  Coerced expression resolution
// ------------------------------

#[test]
fn coerced_expression_resolves_inner_value() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let inner = Expression::int(7, empty_location(), ValueMode::ImmutableOwned);
    let coerced = Expression::coerced(inner, builtin_type_ids::FLOAT);

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let result = resolver
        .resolve_expression(&coerced, &env)
        .expect("coerced literal should resolve");

    // The fast path preserves the Coerced wrapper because it is_compile_time_constant.
    assert!(matches!(result.kind, ExpressionKind::Coerced { .. }));
}

// ------------------------------
//  Runtime RPN with non-const reference fails
// ------------------------------

#[test]
fn runtime_rpn_with_unresolved_reference_fails() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::reference(
                InternedPath::from_single_str("missing", &mut string_table),
                DataType::Int,
                empty_location(),
                ValueMode::ImmutableReference,
            )),
            rvalue_item(Expression::int(
                2,
                empty_location(),
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::Int,
        builtin_type_ids::INT,
        empty_location(),
        ValueMode::ImmutableOwned,
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("unresolved reference in RPN should fail");

    assert_eq!(error, ConstResolutionError::UnresolvedReference);
}

// ------------------------------
//  Body-local declaration resolution
// ------------------------------

#[test]
fn body_local_immutable_literal_resolves() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let declaration = Declaration {
        id: InternedPath::from_single_str("local", &mut string_table),
        value: Expression::int(99, empty_location(), ValueMode::ImmutableOwned),
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let fact = resolver
        .resolve_body_local_declaration(&declaration, &env)
        .expect("body-local immutable literal should resolve");

    assert_eq!(fact.scope, ConstBindingScope::BodyLocal);
    assert_eq!(fact.source, ConstBindingSource::InferredImmutable);
    assert!(matches!(
        fact.resolved_expression.kind,
        ExpressionKind::Int(99)
    ));
}

#[test]
fn body_local_mutable_declaration_fails() {
    let mut string_table = StringTable::new();
    let mut store = TemplateIrStore::new();
    let declaration = Declaration {
        id: InternedPath::from_single_str("local", &mut string_table),
        value: Expression::int(99, empty_location(), ValueMode::MutableOwned),
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &mut store);

    let error = resolver
        .resolve_body_local_declaration(&declaration, &env)
        .expect_err("body-local mutable declaration should fail");

    assert_eq!(error, ConstResolutionError::MutableDeclaration);
}

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
    let location = SourceLocation::default();
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));

    let (template_id, fill_template_id) = {
        let mut store = store_handle.borrow_mut();

        let mut fill_builder = TemplateIrBuilder::new(&mut store);
        let fill_root = fill_builder.push_sequence_node(Vec::new(), location.clone());
        let fill_template_id = fill_builder.finish_template(
            fill_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
        );

        let mut wrapper_builder = TemplateIrBuilder::new(&mut store);
        let slot_node = wrapper_builder.push_slot_node(SlotKey::Default, location.clone());
        let template_id = wrapper_builder.finish_template(
            slot_node,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            location.clone(),
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
        location,
    };

    (template, store_handle)
}

#[test]
fn slot_template_const_fact_uses_effective_tir_view() {
    let mut string_table = StringTable::new();

    let (template, registry) = build_resolved_slot_template_store();
    let declaration = Declaration {
        id: InternedPath::from_single_str("wrapper", &mut string_table),
        value: Expression::template(template, ValueMode::ImmutableOwned),
    };
    let environment = ConstValueEnvironment::default();

    let mut resolver = ConstValueResolver::new(&mut string_table, registry);
    let fact = resolver
        .resolve_explicit_top_level_constant(&declaration, &environment)
        .expect("slot template should resolve as a const fact");

    assert_eq!(
        fact.value_kind,
        ConstFactValueKind::TemplateWrapper,
        "resolved-slot const facts must classify through the effective TIR view"
    );
}
