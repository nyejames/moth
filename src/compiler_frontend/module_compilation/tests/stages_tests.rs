//! Stage-boundary invariants for prepared HIR failures and warning ownership.
//!
//! WHAT: exercises owning-site capture with a forced compact-store capacity failure.
//! WHY: lowering must attach accumulated warnings once while preserving source and frozen identity.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::unreachable_match_arm;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CompilerCapacityExceededView;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticBag, DiagnosticSeverity, HirCapacityResource,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
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
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn test_span(start: u32) -> SourceSpan {
    let mut extended_spans = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, 1, &mut extended_spans).expect("test span fits");
    SourceSpan::new(SourceId::from_index(1), local)
}

#[test]
fn lower_hir_capacity_failure_keeps_each_warning_and_captured_site_once() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let entry_path = symbol("@page.moth", &mut path_fork, &mut string_table);
    let start_name = path_fork
        .try_intern_child(entry_path, string_table.intern(IMPLICIT_START_FUNC_NAME))
        .expect("test start path fits");
    let authored_span = test_span(10);
    let ast_warning_span = test_span(20);
    let preparation_warning_span = test_span(30);
    let ast_warning = unreachable_match_arm(Some(ast_warning_span));
    let preparation_warning = unreachable_match_arm(Some(preparation_warning_span));
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let literal = expressions
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::signed(FixedScalar::I32, 42).expect("I32 literal in range"),
            Some(authored_span),
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal fits store");
    let mut ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(
                NodeKind::ExpressionStatement(literal.id()),
                Some(authored_span),
            )],
            Some(authored_span),
        )],
        entry_path,
        expressions,
        constants,
        Default::default(),
    );
    ast.warnings.push(ast_warning);
    let mut warnings = DiagnosticBag::from_diagnostics(vec![
        preparation_warning,
        unreachable_match_arm(Some(ast_warning_span)),
    ]);

    let type_environment = std::mem::take(&mut ast.type_environment);
    let frozen_identity = FrozenIdentityHandle::for_domain(
        crate::compiler_frontend::semantic_identity::StablePackageIdentity::project_local(
            "hir-stage-test",
        ),
    );
    let mut builder = HirBuilder::new(
        &mut string_table,
        &mut path_fork,
        type_environment,
        HirFunctionOriginLookup::default(),
        Default::default(),
    );
    builder.set_frozen_identity_handle(Some(frozen_identity.clone()));
    builder.set_expression_store_test_limits(HirExpressionStoreTestLimits {
        rows: 0,
        ..Default::default()
    });
    let mut failure = match builder.build_hir_module(ast, &ExternalPackageRegistry::new()) {
        Ok(_) => panic!("the first authored HIR expression should exceed the forced row limit"),
        Err(failure) => failure,
    };
    failure
        .prepend_diagnostics_with_required_paths(
            warnings
                .take_diagnostics()
                .into_iter()
                .map(|mut diagnostic| {
                    diagnostic.attach_frozen_identity_handle_if_missing(frozen_identity.clone());
                    diagnostic
                }),
            || None,
        )
        .expect("test warnings have no rendered paths");
    let failure =
        crate::compiler_frontend::tests::parse_support::freeze_ast_diagnostic_failure(failure);

    let warning_views = failure
        .reports
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(_, diagnostic)| diagnostic)
        .collect::<Vec<_>>();
    assert_eq!(
        warning_views.len(),
        2,
        "the preparation warning and AST warning should each appear once"
    );
    let warning_spans = warning_views
        .iter()
        .copied()
        .map(crate::compiler_frontend::tests::parse_support::ast_error_primary_span)
        .collect::<Vec<_>>();
    assert_eq!(
        warning_spans,
        vec![Some(preparation_warning_span), Some(ast_warning_span)]
    );

    let mut error_views = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .map(|(_, diagnostic)| diagnostic);
    let capacity_view = error_views
        .next()
        .expect("the HIR row limit produces a source-capacity diagnostic");
    assert!(
        error_views.next().is_none(),
        "the HIR capacity failure should be the only error"
    );
    assert_eq!(
        crate::compiler_frontend::tests::parse_support::ast_error_primary_span(capacity_view),
        Some(authored_span)
    );
    let capacity = capacity_view
        .family_view::<CompilerCapacityExceededView>()
        .expect("the source-capacity diagnostic should retain typed resource facts");
    assert_eq!(
        capacity.resource,
        CapacityResource::Hir(HirCapacityResource::ExpressionRows)
    );
    assert!(matches!(
        capacity_view.primary_site(),
        crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
            context,
            ..
        } if context == &frozen_identity
    ));
}
