//! Static `if` finalisation invariant tests.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::IfBranchMetadata;
use crate::compiler_frontend::ast::statements::value_production::types::ValueLexicalScope;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn terminating_value_body_lift_uses_explicit_branch_scope() {
    let mut string_table = StringTable::new();
    let branch_scope = InternedPath::from_single_str("branch", &mut string_table);
    let nested_scope = InternedPath::from_single_str("nested", &mut string_table);
    let location = SourceLocation::default();
    let nested_terminal = AstNode {
        kind: NodeKind::LexicalScope {
            body: vec![AstNode {
                kind: NodeKind::Return(vec![Expression::int(
                    1,
                    location.clone(),
                    ValueMode::ImmutableOwned,
                )]),
                location: location.clone(),
                scope: nested_scope.clone(),
            }],
        },
        location: location.clone(),
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
        location,
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
    let location = SourceLocation::default();
    let function_path = InternedPath::from_single_str("selected", &mut string_table);
    let then_scope = InternedPath::from_single_str("then", &mut string_table);
    let else_scope = InternedPath::from_single_str("else", &mut string_table);
    let nested_scope = InternedPath::from_single_str("nested", &mut string_table);
    let nested_condition = Expression::bool(true, location.clone(), ValueMode::ImmutableOwned)
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
        location: location.clone(),
        scope: nested_scope,
    };
    let outer_if = AstNode {
        kind: NodeKind::If(
            Expression::bool(false, location.clone(), ValueMode::ImmutableOwned),
            vec![nested_if],
            Some(Vec::new()),
            IfBranchMetadata::new(
                IfGenericRequestRanges::default(),
                then_scope.clone(),
                Some(else_scope.clone()),
            ),
        ),
        location: location.clone(),
        scope: then_scope,
    };
    let mut ast = vec![AstNode {
        kind: NodeKind::Function(function_path, FunctionSignature::default(), vec![outer_if]),
        location,
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
