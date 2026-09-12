//! Static `if` finalisation invariant tests.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::IfBranchMetadata;
use crate::compiler_frontend::ast::statements::value_production::types::ValueLexicalScope;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn terminating_value_body_lift_uses_explicit_branch_scope() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let branch_scope = path_fork.try_intern_portable_path("branch", &mut string_table).expect("test path fits");
    let nested_scope = path_fork.try_intern_portable_path("nested", &mut string_table).expect("test path fits");
    let span: Option<SourceSpan> = None;
    let nested_terminal = AstNode {
        kind: NodeKind::LexicalScope {
            body: vec![AstNode {
                kind: NodeKind::Return(vec![Expression::int(1, span, ValueMode::ImmutableOwned)]),
                span,
                scope: nested_scope.clone(),
            }],
        },
        span,
        scope: nested_scope,
    };
    let value = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::LexicalScope(ValueLexicalScope {
                body: vec![nested_terminal],
                scope: branch_scope.clone(),
                result_type_ids: vec![builtin_type_ids::INT],
            })),
        },
        span,
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );
    let mut receiver = NodeKind::Return(vec![value]);

    let (_, lifted_scope) = take_terminal_receiver_body(&mut receiver)
        .expect("purely terminating selected body should replace its receiver");

    assert_eq!(lifted_scope, branch_scope);
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
    let function_path = path_fork.try_intern_portable_path("selected", &mut string_table).expect("test path fits");
    let then_scope = path_fork.try_intern_portable_path("then", &mut string_table).expect("test path fits");
    let else_scope = path_fork.try_intern_portable_path("else", &mut string_table).expect("test path fits");
    let nested_scope = path_fork.try_intern_portable_path("nested", &mut string_table).expect("test path fits");
    let nested_condition = Expression::bool(true, span, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            SyntheticInterfaceMemberIdentity::new(
                SyntheticInterfaceClass::ProjectContext,
                "source-config",
                "enabled",
            ),
        ));
    let nested_if = AstNode {
        kind: NodeKind::If(
            nested_condition,
            Vec::new(),
            Some(Vec::new()),
            IfBranchMetadata::new(
                IfGenericRequestRanges::default(),
                nested_scope.clone(),
                Some(nested_scope.clone()),
            ),
        ),
        span,
        scope: nested_scope,
    };
    let outer_if = AstNode {
        kind: NodeKind::If(
            Expression::bool(false, span, ValueMode::ImmutableOwned),
            vec![nested_if],
            Some(Vec::new()),
            IfBranchMetadata::new(
                IfGenericRequestRanges::default(),
                then_scope.clone(),
                Some(else_scope.clone()),
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
        &ConstValueStore::default(),
        Rc::new(RefCell::new(TemplateIrStore::new())),
        &mut string_table,
    )
    .expect("static literal conditions should specialize");

    assert!(
        specialization.function_provenance().is_empty(),
        "provenance from a nested condition in an inactive outer branch must be discarded: {:?}",
        specialization.function_provenance()
    );
}
