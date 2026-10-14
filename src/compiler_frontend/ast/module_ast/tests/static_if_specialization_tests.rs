//! Static `if` finalisation invariant tests.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::IfBranchMetadata;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOp;
use crate::compiler_frontend::ast::statements::value_production::types::ValueLexicalScope;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[test]
fn terminating_value_body_lift_uses_explicit_branch_scope() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let branch_scope = path_fork
        .try_intern_portable_path("branch", &mut string_table)
        .expect("test path fits");
    let nested_scope = path_fork
        .try_intern_portable_path("nested", &mut string_table)
        .expect("test path fits");
    let span: Option<SourceSpan> = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let returned = builder
        .fixed_scalar(
            &mut const_values,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("typed scalar")
        .id();
    let nested_terminal = AstNode {
        kind: NodeKind::LexicalScope {
            body: vec![AstNode {
                kind: NodeKind::Return(vec![returned]),
                span,
                scope: nested_scope,
            }],
        },
        span,
        scope: nested_scope,
    };
    let block = builder
        .append_value_block(
            ValueBlock::LexicalScope(ValueLexicalScope {
                body: vec![nested_terminal],
                scope: branch_scope,
                result_type_ids: vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            }),
            span,
        )
        .expect("valid value block");
    let value = builder
        .append_expression(
            ExpressionOp::ValueBlock(block),
            span,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ValueMode::ImmutableOwned,
        )
        .expect("typed value block")
        .id();
    let receiver = NodeKind::Return(vec![value]);

    let (_, lifted_scope) = take_terminal_receiver_body(&receiver, &builder, &const_values)
        .expect("valid expression owner")
        .expect("purely terminating selected body should replace its receiver");

    assert_eq!(lifted_scope, branch_scope);
    let ExpressionOperationView::ValueBlock(ValueBlock::LexicalScope(original)) =
        builder.view(value).expect("retained root").operation()
    else {
        panic!("lifting must preserve the shared expression operation");
    };
    assert_eq!(
        original.body.len(),
        1,
        "another candidate still owns its original body"
    );
}

#[test]
fn inactive_static_branch_drops_nested_provenance() {
    use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
    use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
    use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
    use crate::compiler_frontend::synthetic_interface_provenance::{
        SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let span: Option<SourceSpan> = None;
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let function_path = path_fork
        .try_intern_portable_path("selected", &mut string_table)
        .expect("test path fits");
    let then_scope = path_fork
        .try_intern_portable_path("then", &mut string_table)
        .expect("test path fits");
    let else_scope = path_fork
        .try_intern_portable_path("else", &mut string_table)
        .expect("test path fits");
    let nested_scope = path_fork
        .try_intern_portable_path("nested", &mut string_table)
        .expect("test path fits");
    let nested_condition = builder
        .bool(&mut const_values, true, span, ValueMode::ImmutableOwned)
        .expect("typed bool")
        .id();
    let nested_condition = builder
        .with_synthetic_interface_provenance(
            builder.own(nested_condition),
            SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
                SyntheticInterfaceClass::ProjectContext,
                "source-config",
                "enabled",
            )),
        )
        .expect("occurrence provenance")
        .id();
    let nested_if = AstNode {
        kind: NodeKind::If(
            nested_condition,
            Vec::new(),
            Some(Vec::new()),
            IfBranchMetadata::new(
                IfGenericRequestRanges::default(),
                nested_scope,
                Some(nested_scope),
            ),
        ),
        span,
        scope: nested_scope,
    };
    let outer_condition = builder
        .bool(&mut const_values, false, span, ValueMode::ImmutableOwned)
        .expect("typed bool")
        .id();
    let outer_if = AstNode {
        kind: NodeKind::If(
            outer_condition,
            vec![nested_if],
            Some(Vec::new()),
            IfBranchMetadata::new(
                IfGenericRequestRanges::default(),
                then_scope,
                Some(else_scope),
            ),
        ),
        span,
        scope: then_scope,
    };
    let mut ast = vec![AstNode {
        kind: NodeKind::Function(function_path, FunctionSignature::default(), vec![outer_if]),
        span,
        scope: else_scope,
    }];

    let specialization = StaticIfSpecialization::run(
        &mut ast,
        &mut const_values,
        &mut builder,
        Rc::new(RefCell::new(TemplateIrStore::new())),
        &mut string_table,
        &TypeEnvironment::new(),
    )
    .expect("static literal conditions should specialize");

    assert!(
        specialization.function_provenance().is_empty(),
        "provenance from a nested condition in an inactive outer branch must be discarded: {:?}",
        specialization.function_provenance()
    );
}

#[test]
fn selected_value_root_keeps_authored_sibling_immutable() {
    use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
    use crate::compiler_frontend::ast::statements::value_production::types::{
        ProducedValues, ValueIfBlock,
    };
    use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("selected", &mut strings)
        .expect("scope");
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let condition = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("condition")
        .id();
    let selected_value = builder
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("selected value")
        .id();
    let body = vec![AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions: vec![selected_value],
            span: None,
        }),
        span: None,
        scope,
    }];
    let block = builder
        .append_value_block(
            ValueBlock::If(ValueIfBlock {
                condition,
                then_body: body.clone(),
                else_body: body,
                then_scope: scope,
                else_scope: scope,
                span: None,
                generic_request_ranges: IfGenericRequestRanges::default(),
                result_type_ids: vec![builtin_type_ids::BOOL],
            }),
            None,
        )
        .expect("authored block");
    let authored = builder
        .append_expression(
            ExpressionOp::ValueBlock(block),
            None,
            builtin_type_ids::BOOL,
            ValueMode::ImmutableOwned,
        )
        .expect("authored root")
        .id();
    let mut ast = vec![AstNode {
        kind: NodeKind::Return(vec![authored]),
        span: None,
        scope,
    }];
    StaticIfSpecialization::run(
        &mut ast,
        &mut constants,
        &mut builder,
        Rc::new(RefCell::new(TemplateIrStore::new())),
        &mut strings,
        &TypeEnvironment::new(),
    )
    .expect("selection");
    let NodeKind::Return(values) = &ast[0].kind else {
        panic!("producing receiver retained");
    };
    assert_ne!(values[0], authored);
    assert!(matches!(
        builder.view(authored).expect("sibling root").operation(),
        ExpressionOperationView::ValueBlock(ValueBlock::If(_)),
    ));
    let ExpressionOperationView::ValueBlock(ValueBlock::LexicalScope(selected)) =
        builder.view(values[0]).expect("selected root").operation()
    else {
        panic!("selected scoped body");
    };
    let NodeKind::ThenValue(produced) = &selected.body[0].kind else {
        panic!("produced value");
    };
    assert_eq!(
        produced.expressions,
        vec![selected_value],
        "unchanged child IDs are reused"
    );
}
