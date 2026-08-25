//! Unit tests for compile-time template folding.
//!
//! WHAT: exercises the borrow-first fold-binding resolver used by template
//! folding
//!       so the common no-substitution path returns a borrowed reference instead
//!       of cloning the whole expression tree.
//! WHY: these tests are intentionally narrow: they assert the resolver's
//!      allocation behaviour, not end-to-end fold output. End-to-end parity is
//!      protected by the existing template integration suite.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{Declaration, LoopBindings};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeIterationValue, TemplateFoldBinding, build_collection_iteration_bindings,
    build_range_iteration_bindings,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    FoldResolvedExpression, TirFoldContext, resolve_fold_bindings_in_expression,
    selected_option_capture_payload_with_provenance,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn const_loop_iteration_bindings_preserve_source_provenance() {
    let mut string_table = StringTable::new();
    let item_path = InternedPath::from_single_str("item", &mut string_table);
    let index_path = InternedPath::from_single_str("index", &mut string_table);
    let location = test_source_location(1);
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let provenance = SyntheticInterfaceProvenance::single(member.clone());
    let bindings = LoopBindings {
        item: Some(Declaration {
            id: item_path,
            value: Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
        }),
        index: Some(Declaration {
            id: index_path,
            value: Expression::int(0, location.clone(), ValueMode::ImmutableOwned),
        }),
    };

    let collection_bindings = build_collection_iteration_bindings(
        &bindings,
        &Expression::int(1, location.clone(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(provenance.clone()),
        0,
        &provenance,
    );
    assert!(collection_bindings.iter().all(|binding| {
        binding.value.synthetic_interface_provenance.members() == [member.clone()]
    }));

    let range_bindings =
        build_range_iteration_bindings(&bindings, ConstRangeIterationValue::Int(1), 0, &provenance);
    assert!(range_bindings.iter().all(|binding| {
        binding.value.synthetic_interface_provenance.members() == [member.clone()]
    }));
}

// -------------------------------------------------------
//  Borrow-first: no-substitution path returns Borrowed
// -------------------------------------------------------

#[test]
fn bool_condition_with_no_bindings_returns_borrowed() {
    let mut string_table = StringTable::new();
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    let condition = Expression::bool(true, test_source_location(1), ValueMode::ImmutableOwned);
    let resolved = resolve_fold_bindings_in_expression(&condition, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Borrowed(_)),
        "bool literal with no bindings should return Borrowed, not Owned"
    );
}

#[test]
fn string_slice_with_no_bindings_returns_borrowed() {
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("hello");
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    let text =
        Expression::string_slice(text_id, test_source_location(1), ValueMode::ImmutableOwned);
    let resolved = resolve_fold_bindings_in_expression(&text, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Borrowed(_)),
        "string slice with no bindings should return Borrowed"
    );
}

// -------------------------------------------------------
//  Borrow-first: binding substitution returns Owned
// -------------------------------------------------------

#[test]
fn bool_condition_binding_substitution_returns_owned() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("show", &mut string_table);

    let binding_value = Expression::bool(true, test_source_location(2), ValueMode::ImmutableOwned);
    let bindings = vec![TemplateFoldBinding {
        path: path.clone(),
        value: binding_value,
    }];

    let condition = Expression::reference(
        path,
        DataType::Bool,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );

    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings,
    };

    let resolved = resolve_fold_bindings_in_expression(&condition, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Owned(_)),
        "reference with a matching binding should return Owned"
    );

    let owned = resolved.into_owned();
    assert!(
        matches!(owned.kind, ExpressionKind::Bool(true)),
        "substituted expression should be the bound bool literal"
    );
}

// -------------------------------------------------------
//  Borrow-first: option-present capture substitution
// -------------------------------------------------------

#[test]
fn option_present_capture_substitution_returns_owned() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("maybe_name", &mut string_table);

    let inner_value = Expression::string_slice(
        string_table.intern("Alice"),
        test_source_location(2),
        ValueMode::ImmutableOwned,
    );
    let option_value = Expression::coerced(inner_value, builtin_type_ids::STRING);

    let bindings = vec![TemplateFoldBinding {
        path: path.clone(),
        value: option_value,
    }];

    let scrutinee = Expression::reference(
        path,
        DataType::StringSlice,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );

    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings,
    };

    let resolved = resolve_fold_bindings_in_expression(&scrutinee, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Owned(_)),
        "option reference with a matching binding should return Owned"
    );
}

#[test]
fn option_capture_classifies_same_store_payload_under_active_fold_borrow() {
    let mut string_table = StringTable::new();
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();

    let template_id = {
        let mut store_borrow = store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store_borrow);
        let slot = builder.push_slot_node(SlotKey::Default, test_source_location(1));
        let root = builder.push_sequence_node(vec![slot], test_source_location(1));

        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            test_source_location(1),
        )
    };

    let payload_template = store_qualified_template_with_tir_reference(TemplateTirReference {
        root: template_id,
        phase: TemplateTirPhase::Composed,
        context,
    });

    assert_store_backed_option_capture(&mut string_table, Rc::clone(&store), payload_template);
}

#[test]
fn option_capture_scalar_payload_uses_ordinary_const_rules() {
    let mut string_table = StringTable::new();
    let option_path = InternedPath::from_single_str("maybe_payload", &mut string_table);
    let option_value = Expression::coerced(
        Expression::string_slice(
            string_table.intern("payload"),
            test_source_location(1),
            ValueMode::ImmutableOwned,
        ),
        builtin_type_ids::STRING,
    );
    let scrutinee = Expression::reference(
        option_path.clone(),
        DataType::StringSlice,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );
    let capture_path = InternedPath::from_single_str("payload", &mut string_table);
    let pattern = MatchPattern::OptionPresentCapture {
        name: string_table.intern("payload"),
        binding_path: capture_path.clone(),
        inner_type_id: builtin_type_ids::STRING,
        location: test_source_location(1),
        binding_location: test_source_location(1),
    };

    let store = TemplateIrStore::new();
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![TemplateFoldBinding {
            path: option_path,
            value: option_value,
        }],
    };

    let capture = selected_option_capture_payload_with_provenance(
        &scrutinee,
        &pattern,
        &store,
        &mut fold_context,
    )
    .expect("a scalar const option payload should remain compile-time constant")
    .0
    .expect("the present option should produce a capture binding");

    assert_eq!(capture.path, capture_path);
    assert!(matches!(capture.value.kind, ExpressionKind::StringSlice(_)));
}

fn store_qualified_template_with_tir_reference(tir_reference: TemplateTirReference) -> Template {
    Template {
        tir_reference,
        location: SourceLocation::default(),
    }
}

fn assert_store_backed_option_capture(
    string_table: &mut StringTable,
    store: Rc<RefCell<TemplateIrStore>>,
    payload_template: Template,
) {
    let option_path = InternedPath::from_single_str("maybe_payload", string_table);
    let option_value = Expression::coerced(
        Expression::template(payload_template, ValueMode::ImmutableOwned),
        builtin_type_ids::STRING,
    );
    let scrutinee = Expression::reference(
        option_path.clone(),
        DataType::StringSlice,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );
    let capture_name = string_table.intern("payload");
    let capture_path = InternedPath::from_single_str("payload", string_table);
    let pattern = MatchPattern::OptionPresentCapture {
        name: capture_name,
        binding_path: capture_path.clone(),
        inner_type_id: builtin_type_ids::STRING,
        location: test_source_location(1),
        binding_location: test_source_location(1),
    };

    let mut fold_context = TirFoldContext {
        string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![TemplateFoldBinding {
            path: option_path,
            value: option_value,
        }],
    };

    // The TIR folder retains this borrow while option-capture resolution classifies
    // nested template payloads. Store classification must therefore remain read-only.
    let active_fold_borrow = store.borrow();
    let capture = selected_option_capture_payload_with_provenance(
        &scrutinee,
        &pattern,
        &active_fold_borrow,
        &mut fold_context,
    )
    .expect("the composed slot wrapper is a compile-time option payload")
    .0
    .expect("the present option should produce a capture binding");

    assert_eq!(capture.path, capture_path);
    assert!(matches!(capture.value.kind, ExpressionKind::Template(_)));
}

// -------------------------------------------------------
//  Borrow-first: coerced expression stays Borrowed when inner unchanged
// -------------------------------------------------------

#[test]
fn coerced_expression_with_no_bindings_returns_borrowed() {
    let mut string_table = StringTable::new();
    let inner = Expression::string_slice(
        string_table.intern("value"),
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );
    let coerced = Expression::coerced(inner, builtin_type_ids::STRING);

    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    let resolved = resolve_fold_bindings_in_expression(&coerced, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Borrowed(_)),
        "coerced expression with no bindings should return Borrowed"
    );
}

#[test]
fn coerced_template_with_no_bindings_returns_inner_template_borrow() {
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("nested");

    // Build a minimal module-local text template so the borrow path receives
    // the same authoritative identity as any other parsed template.
    let mut tir_store = TemplateIrStore::new();
    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut tir_store);
        let root = builder.push_text_node(
            text_id,
            6,
            TemplateSegmentOrigin::Body,
            test_source_location(1),
        );
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            test_source_location(1),
        )
    };

    let nested_template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext::default(),
        },
        location: SourceLocation::default(),
    };

    let coerced_template = Expression::coerced(
        Expression::template(nested_template, ValueMode::ImmutableOwned),
        builtin_type_ids::STRING,
    );

    // The no-substitution path does not semantically read the template, so it
    // must not depend on store classification.
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    let resolved = resolve_fold_bindings_in_expression(&coerced_template, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(
            resolved,
            FoldResolvedExpression::Borrowed(Expression {
                kind: ExpressionKind::Template(_),
                ..
            })
        ),
        "Coerced(Template) should borrow the inner template for string rendering"
    );
}

// -------------------------------------------------------
//  Borrow-first: RPN substitution inside const template loops
// -------------------------------------------------------

#[test]
fn rpn_with_no_substitutable_operands_returns_borrowed() {
    let mut string_table = StringTable::new();
    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings: vec![],
    };

    let rpn = ExpressionRpn {
        items: vec![
            ExpressionRpnItem::Operand(Expression::int(
                1,
                test_source_location(1),
                ValueMode::ImmutableOwned,
            )),
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                location: test_source_location(1),
            },
            ExpressionRpnItem::Operand(Expression::int(
                2,
                test_source_location(1),
                ValueMode::ImmutableOwned,
            )),
        ],
    };
    let runtime_expr = Expression::runtime(
        rpn,
        DataType::Int,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );

    let resolved = resolve_fold_bindings_in_expression(&runtime_expr, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Borrowed(_)),
        "RPN with only literal operands should return Borrowed"
    );
}

#[test]
fn rpn_with_bound_reference_operand_returns_owned() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("counter", &mut string_table);

    let binding_value = Expression::int(5, test_source_location(2), ValueMode::ImmutableOwned);
    let bindings = vec![TemplateFoldBinding {
        path: path.clone(),
        value: binding_value,
    }];

    let rpn = ExpressionRpn {
        items: vec![
            ExpressionRpnItem::Operand(Expression::reference(
                path,
                DataType::Int,
                test_source_location(1),
                ValueMode::ImmutableOwned,
            )),
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                location: test_source_location(1),
            },
            ExpressionRpnItem::Operand(Expression::int(
                1,
                test_source_location(1),
                ValueMode::ImmutableOwned,
            )),
        ],
    };
    let runtime_expr = Expression::runtime(
        rpn,
        DataType::Int,
        test_source_location(1),
        ValueMode::ImmutableOwned,
    );

    let mut fold_context = TirFoldContext {
        string_table: &mut string_table,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        bindings,
    };

    let resolved = resolve_fold_bindings_in_expression(&runtime_expr, &mut fold_context)
        .expect("resolution should succeed");

    assert!(
        matches!(resolved, FoldResolvedExpression::Owned(_)),
        "RPN with a bound reference operand should return Owned"
    );
}
