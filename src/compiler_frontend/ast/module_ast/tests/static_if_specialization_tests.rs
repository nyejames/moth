//! Static `if` finalisation invariant tests.

use super::*;
use crate::compiler_frontend::ast::statements::value_production::types::ValueScopedBlock;
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
        kind: NodeKind::ScopedBlock {
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
            block: Box::new(ValueBlock::Scoped(ValueScopedBlock {
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
