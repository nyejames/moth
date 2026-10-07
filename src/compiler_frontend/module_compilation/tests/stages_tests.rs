//! Stage-boundary invariants for HIR failure conversion and warning ownership.
//!
//! WHAT: exercises the premerge conversion with a forced compact-store capacity failure.
//! WHY: the lowering stage owns accumulated warnings and must attach each once while preserving
//!      the failure's source, frozen-identity, and type-rendering context.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, HirCapacityResource,
    PremergeFailure, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::hir::expression_store::HirExpressionStoreTestLimits;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, LocalSpan, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{function_node, node, symbol};
use crate::compiler_frontend::tests::type_id_fixture_support::build_ast_with_registered_types;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

fn test_span(start: u32) -> SourceSpan {
    let mut extended_spans = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, 1, &mut extended_spans).expect("test span fits");
    SourceSpan::new(SourceId::from_index(1), local)
}

#[test]
fn lower_hir_capacity_failure_keeps_each_warning_and_render_context_once() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let entry_path = symbol("@page.moth", &mut path_fork, &mut string_table);
    let start_name = path_fork
        .try_intern_child(entry_path, string_table.intern(IMPLICIT_START_FUNC_NAME))
        .expect("test start path fits");
    let authored_span = test_span(10);
    let ast_warning_span = test_span(20);
    let preparation_warning_span = test_span(30);
    let ast_warning = CompilerDiagnostic::unreachable_match_arm(Some(ast_warning_span));
    let preparation_warning =
        CompilerDiagnostic::unreachable_match_arm(Some(preparation_warning_span));
    let mut ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(
                NodeKind::ExpressionStatement(Expression::int(
                    42,
                    Some(authored_span),
                    ValueMode::ImmutableOwned,
                )),
                Some(authored_span),
            )],
            Some(authored_span),
        )],
        entry_path,
    );
    ast.warnings.push(ast_warning.clone());
    let warnings = [preparation_warning.clone(), ast_warning.clone()];

    let type_environment = ast.type_environment.clone();
    let mut builder = HirBuilder::new(
        &mut string_table,
        &mut path_fork,
        type_environment,
        HirFunctionOriginLookup::default(),
        Default::default(),
    );
    builder.set_expression_store_test_limits(HirExpressionStoreTestLimits {
        rows: 0,
        ..Default::default()
    });
    let hir_messages = match builder.build_hir_module(ast) {
        Ok(_) => panic!("the first authored HIR expression should exceed the forced row limit"),
        Err(messages) => messages,
    };

    let frozen_identity = FrozenIdentityHandle::new();
    let failure = super::premerge_hir_failure(hir_messages, &warnings, Some(&frozen_identity));
    assert!(matches!(failure, PremergeFailure::Diagnosed(_)));
    let messages = failure.into_messages(&string_table);

    assert_eq!(
        messages
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
            .count(),
        2,
        "the preparation warning and AST warning should each appear once"
    );
    assert_eq!(
        messages
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.primary_span == Some(preparation_warning_span))
            .count(),
        1
    );
    assert_eq!(
        messages
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.primary_span == Some(ast_warning_span))
            .count(),
        1
    );

    let (capacity_index, capacity_diagnostic) = messages
        .diagnostics
        .iter()
        .enumerate()
        .find(|(_, diagnostic)| {
            diagnostic.kind
                == DiagnosticKind::Syntax(SyntaxDiagnosticKind::CompilerCapacityExceeded)
        })
        .expect("the HIR row limit produces a source-capacity diagnostic");
    assert_eq!(capacity_diagnostic.primary_span, Some(authored_span));
    assert_eq!(
        capacity_diagnostic.payload,
        DiagnosticPayload::CompilerCapacityExceeded {
            resource: HirCapacityResource::ExpressionRows,
        }
    );
    assert_eq!(
        capacity_diagnostic.primary_frozen_identity_handle.as_ref(),
        Some(&frozen_identity),
        "the exact frozen identity owner should remain paired with the authored span"
    );
    assert!(messages.render_type_contexts.iter().any(|context| {
        context.diagnostic_range.contains(&capacity_index)
            && context
                .type_environment
                .get(context.type_environment.builtins().int)
                .is_some()
    }));
}
