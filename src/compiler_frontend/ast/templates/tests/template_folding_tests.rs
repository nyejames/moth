//! Template fold-binding identity, provenance and exact nested-template invariants.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue, LoopBindings};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateFoldBinding, build_collection_iteration_bindings, build_range_iteration_bindings,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TirFoldContext, resolve_fold_bindings_in_expression,
    selected_option_capture_payload_with_provenance,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::symbols::{
    path_interner::{PathId, PathInternerFork},
    string_interning::StringTable,
};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn context<'a>(
    strings: &'a mut StringTable,
    expressions: &'a mut AstExpressionBuilder,
    constants: &'a mut ConstValueStore,
    bindings: Vec<TemplateFoldBinding>,
) -> TirFoldContext<'a> {
    TirFoldContext {
        string_table: strings,
        expression_builder: expressions,
        const_values: constants,
        template_const_loop_iteration_limit: DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS,
        source_scope: None,
        external_package_registry: None,
        bindings,
    }
}

fn path(strings: &mut StringTable, text: &str) -> PathId {
    PathInternerFork::empty()
        .try_intern_portable_path(text, strings)
        .expect("test path fits")
}

fn reference(
    expressions: &mut AstExpressionBuilder,
    path: PathId,
    type_id: crate::compiler_frontend::datatypes::ids::TypeId,
) -> ExprId {
    expressions
        .reference_with_type_id(
            path,
            type_id,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("valid reference")
        .id()
}

#[test]
fn bool_condition_with_no_bindings_reuses_root() {
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let root = expressions
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let mut context = context(&mut strings, &mut expressions, &mut constants, vec![]);
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        root
    );
}

#[test]
fn string_slice_with_no_bindings_reuses_root() {
    let mut strings = StringTable::new();
    let text = strings.intern("hello");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let root = expressions
        .string_slice(&mut constants, text, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let mut context = context(&mut strings, &mut expressions, &mut constants, vec![]);
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        root
    );
}

#[test]
fn bool_condition_binding_substitution_reuses_binding() {
    let mut strings = StringTable::new();
    let binding_path = path(&mut strings, "show");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let value = expressions
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let root = reference(&mut expressions, binding_path, builtin_type_ids::BOOL);
    let mut context = context(
        &mut strings,
        &mut expressions,
        &mut constants,
        vec![TemplateFoldBinding {
            path: binding_path,
            value,
        }],
    );
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        value
    );
    assert!(matches!(
        context
            .expression_builder
            .view(root)
            .expect("original")
            .operation(),
        ExpressionOperationView::Reference(_)
    ));
}

#[test]
fn option_present_capture_substitution_preserves_wrapper() {
    let mut strings = StringTable::new();
    let binding_path = path(&mut strings, "maybe_name");
    let text = strings.intern("Alice");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = expressions
        .string_slice(&mut constants, text, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let value = expressions
        .coerced(expressions.own(inner), builtin_type_ids::STRING)
        .expect("wrapper")
        .id();
    let root = reference(&mut expressions, binding_path, builtin_type_ids::STRING);
    let mut context = context(
        &mut strings,
        &mut expressions,
        &mut constants,
        vec![TemplateFoldBinding {
            path: binding_path,
            value,
        }],
    );
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        value
    );
}

fn capture_pattern(strings: &mut StringTable, capture_path: PathId) -> MatchPattern {
    MatchPattern::OptionPresentCapture {
        name: strings.intern("payload"),
        binding_path: capture_path,
        inner_type_id: builtin_type_ids::STRING,
        span: None,
        binding_span: None,
    }
}

#[test]
fn option_capture_scalar_payload_uses_ordinary_const_rules() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let binding_path = paths
        .try_intern_portable_path("maybe_payload", &mut strings)
        .expect("path");
    let capture_path = paths
        .try_intern_portable_path("payload", &mut strings)
        .expect("path");
    let pattern = capture_pattern(&mut strings, capture_path);
    let text = strings.intern("payload");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = expressions
        .string_slice(&mut constants, text, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let value = expressions
        .coerced(expressions.own(inner), builtin_type_ids::STRING)
        .expect("wrapper")
        .id();
    let root = reference(&mut expressions, binding_path, builtin_type_ids::STRING);
    let mut context = context(
        &mut strings,
        &mut expressions,
        &mut constants,
        vec![TemplateFoldBinding {
            path: binding_path,
            value,
        }],
    );
    let capture = selected_option_capture_payload_with_provenance(
        root,
        &pattern,
        &TemplateIrStore::new(),
        &mut context,
    )
    .expect("capture")
    .0
    .expect("present");
    assert_eq!(capture.path, capture_path);
    assert_eq!(capture.value, inner);
}

#[test]
fn option_capture_classifies_same_store_payload_under_active_fold_borrow() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let binding_path = paths
        .try_intern_portable_path("maybe_payload", &mut strings)
        .expect("path");
    let capture_path = paths
        .try_intern_portable_path("payload", &mut strings)
        .expect("path");
    let pattern = capture_pattern(&mut strings, capture_path);
    let store = std::cell::RefCell::new(TemplateIrStore::new());
    let template_root = {
        let mut owner = store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut owner);
        let slot = builder.push_slot_node(SlotKey::Default, None);
        let root = builder.push_sequence_node(vec![slot], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_root,
            phase: TemplateTirPhase::Composed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template")
        .id();
    let value = expressions
        .coerced(expressions.own(inner), builtin_type_ids::STRING)
        .expect("wrapper")
        .id();
    let root = reference(&mut expressions, binding_path, builtin_type_ids::STRING);
    let mut context = context(
        &mut strings,
        &mut expressions,
        &mut constants,
        vec![TemplateFoldBinding {
            path: binding_path,
            value,
        }],
    );
    let active_fold_borrow = store.borrow();
    let capture = selected_option_capture_payload_with_provenance(
        root,
        &pattern,
        &active_fold_borrow,
        &mut context,
    )
    .expect("const template capture")
    .0
    .expect("present");
    assert_eq!(capture.path, capture_path);
    assert_eq!(capture.value, inner);
}

#[test]
fn coerced_expression_with_no_bindings_reuses_root() {
    let mut strings = StringTable::new();
    let text = strings.intern("value");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = expressions
        .string_slice(&mut constants, text, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let root = expressions
        .coerced(expressions.own(inner), builtin_type_ids::STRING)
        .expect("wrapper")
        .id();
    let mut context = context(&mut strings, &mut expressions, &mut constants, vec![]);
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        root
    );
}

#[test]
fn coerced_template_with_no_bindings_returns_exact_inner_template() {
    let mut strings = StringTable::new();
    let text = strings.intern("nested");
    let mut store = TemplateIrStore::new();
    let mut builder = TemplateIrBuilder::new(&mut store);
    let root = builder.push_text_node(text, 6, TemplateSegmentOrigin::Body, None);
    let template_root = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        None,
    );
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_root,
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template")
        .id();
    let root = expressions
        .coerced(expressions.own(inner), builtin_type_ids::STRING)
        .expect("wrapper")
        .id();
    let mut context = context(&mut strings, &mut expressions, &mut constants, vec![]);
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        inner
    );
    let ExpressionOperationView::Template(template) = context
        .expression_builder
        .view(inner)
        .expect("template")
        .operation()
    else {
        panic!("expected template")
    };
    assert_eq!(template.tir_reference.root, template_root);
    assert_eq!(template.tir_reference.phase, TemplateTirPhase::Parsed);
}

fn numeric_root(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    left: ExprId,
) -> ExprId {
    use crate::compiler_frontend::ast::expressions::{
        expression_kind::Operator, expression_rpn::ExpressionRpnItem,
    };
    let right = expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal")
        .id();
    crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr(
        expressions,
        constants,
        vec![
            ExpressionRpnItem::Operand(left),
            ExpressionRpnItem::Operand(right),
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                span: None,
            },
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
        &crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
    )
}

#[test]
fn numeric_operation_with_no_substitutions_reuses_root() {
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let left = expressions
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 2).expect("I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal")
        .id();
    let root = numeric_root(&mut expressions, &mut constants, left);
    let mut context = context(&mut strings, &mut expressions, &mut constants, vec![]);
    assert_eq!(
        resolve_fold_bindings_in_expression(root, &mut context)
            .expect("resolution")
            .expression,
        root
    );
}

#[test]
fn numeric_operation_with_bound_reference_folds_replacement() {
    let mut strings = StringTable::new();
    let binding_path = path(&mut strings, "counter");
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let value = expressions
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 5).expect("I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal")
        .id();
    let left = reference(
        &mut expressions,
        binding_path,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let root = numeric_root(&mut expressions, &mut constants, left);
    let mut context = context(
        &mut strings,
        &mut expressions,
        &mut constants,
        vec![TemplateFoldBinding {
            path: binding_path,
            value,
        }],
    );
    let folded = resolve_fold_bindings_in_expression(root, &mut context).expect("resolution");
    let folded = folded.expression;
    assert_ne!(folded, root);
    let ExpressionOperationView::Constant(value) = context
        .expression_builder
        .view(folded)
        .expect("result")
        .operation()
    else {
        panic!("expected constant")
    };
    assert!(
        matches!(context.const_values.payload(value), Some(ConstValuePayload::FixedScalar(value)) if value.as_i64() == Some(6))
    );
}

#[test]
fn const_loop_iteration_bindings_preserve_source_provenance() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let item_path = paths
        .try_intern_portable_path("item", &mut strings)
        .expect("path");
    let index_path = paths
        .try_intern_portable_path("index", &mut strings)
        .expect("path");
    let declaration = |id| Declaration {
        id,
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };
    let bindings = LoopBindings {
        item: Some(declaration(item_path)),
        index: Some(declaration(index_path)),
    };
    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let provenance = SyntheticInterfaceProvenance::single(member.clone());
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let counter = FixedScalarValue::signed(FixedScalar::I32, 1).expect("I32");
    let item = expressions
        .fixed_scalar(&mut constants, counter, None, ValueMode::ImmutableOwned)
        .expect("literal")
        .id();
    let collection = build_collection_iteration_bindings(
        &bindings,
        item,
        0,
        &provenance,
        &mut expressions,
        &mut constants,
    )
    .expect("bindings");
    let range = build_range_iteration_bindings(
        &bindings,
        counter,
        0,
        &provenance,
        &mut expressions,
        &mut constants,
    )
    .expect("bindings");
    for binding in collection.iter().chain(&range) {
        assert_eq!(
            expressions
                .view(binding.value)
                .expect("binding")
                .occurrence_facts()
                .expect("provenance")
                .provenance
                .members(),
            std::slice::from_ref(&member)
        );
    }
}
