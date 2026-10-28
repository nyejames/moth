//! Postfix entry rejects foreign receiver handles before their owner token is erased.

use super::{ParsedPostfixExpression, PostfixChainAccess, parse_postfix_chain_expression};
use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn storage_state(builder: &AstExpressionBuilder) -> [usize; 8] {
    let statistics = builder.store().storage_statistics();
    [
        statistics.rows,
        statistics.row_capacity,
        statistics.row_bytes,
        statistics.row_capacity_bytes,
        statistics.side_entries,
        statistics.side_capacity,
        statistics.side_bytes,
        statistics.side_capacity_bytes,
    ]
}

#[test]
fn foreign_postfix_receiver_at_eof_is_rejected_before_token_erasure() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = paths
        .try_intern_portable_path("postfix_owner.moth", &mut strings)
        .expect("test path fits");
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![], &paths)),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let mut token_builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    token_builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let tokens = token_builder
        .finish()
        .expect("canonical fixture tokens should build");
    let range = tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut cursor = AstCursor::from_source_tokens(&tokens, range)
        .expect("test token stream must expose an AST cursor");
    assert_eq!(cursor.current_tag(), TokenTag::EOF);

    let mut constants = ConstValueStore::default();
    let mut first = AstExpressionBuilder::new().expect("first expression owner should build");
    let mut second = AstExpressionBuilder::new().expect("second expression owner should build");
    let foreign_root = first
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("foreign Bool should build");
    let local_root = second
        .bool(&mut constants, false, None, ValueMode::ImmutableOwned)
        .expect("local Bool should build");
    assert_eq!(foreign_root.id().index(), 0);
    assert_eq!(local_root.id(), foreign_root.id());
    assert!(matches!(
        first
            .view_owned(foreign_root)
            .expect("foreign root belongs to its issuing owner")
            .operation(),
        ExpressionOperationView::Constant(value)
            if matches!(constants.payload(value), Some(ConstValuePayload::Bool(true)))
    ));
    let before = storage_state(&second);

    // The shared EOF path returns its receiver unchanged, exposing index-only reattachment.
    let result = parse_postfix_chain_expression(
        &mut cursor,
        foreign_root,
        None,
        PostfixChainAccess::shared(),
        false,
        &context,
        &mut type_interner,
        &mut second,
        &mut constants,
        &mut strings,
        &mut paths,
    );
    let Err(ExpressionParseError::Infrastructure(error)) = result else {
        panic!("a foreign postfix receiver must return an invariant error");
    };
    assert_eq!(error.primary.kind, InfrastructureFailureKind::Internal);
    assert_eq!(storage_state(&second), before);
    assert_eq!(cursor.current_tag(), TokenTag::EOF);

    let local_result = parse_postfix_chain_expression(
        &mut cursor,
        local_root,
        None,
        PostfixChainAccess::shared(),
        false,
        &context,
        &mut type_interner,
        &mut second,
        &mut constants,
        &mut strings,
        &mut paths,
    )
    .expect("the receiving owner's local receiver remains valid at EOF");
    let ParsedPostfixExpression::Value(local_result) = local_result else {
        panic!("the shared EOF path must return its local receiver");
    };
    assert_eq!(local_result.id(), local_root.id());
    assert!(matches!(
        second
            .view_owned(local_result)
            .expect("local result belongs to the receiving owner")
            .operation(),
        ExpressionOperationView::Constant(value)
            if matches!(constants.payload(value), Some(ConstValuePayload::Bool(false)))
    ));
    assert_eq!(storage_state(&second), before);
}
