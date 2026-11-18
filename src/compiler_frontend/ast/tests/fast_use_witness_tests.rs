//! Selected-fast-operation witnesses recorded during expression resolution.
//!
//! WHAT: proves the AST retains fast-use evidence after the selected operation folds away, and
//!       that ordinary arithmetic records none.
//! WHY: `$safe_math` verification runs after folding and dead-result elimination, so evidence on
//!      the expression node alone would disappear exactly where the contract still needs it.

use crate::compiler_frontend::ast::ast_nodes::{AstFunctionIdentity, AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolve_expression_result_type;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::{
    AstFastOperationSource, AstFastOperationWitness, ContextKind, ScopeContext,
    TopLevelDeclarationTable, normalize_fast_operation_witnesses,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::{SourceConstraintSet, SourceDirectiveKind};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::rc::Rc;
use std::sync::Arc;

/// Build a source span distinguishable from every other one in a test.
fn marked_span(start: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::from_index(7),
        LocalSpan::exact(start, 4, &mut span_builder).expect("test span should fit"),
    )
}

/// A function-body scope with a stable callable identity and no declarations.
fn witness_scope(paths: &PathInternerFork) -> ScopeContext {
    ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(vec![], paths)),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    )
    .with_constraint_function(AstFunctionIdentity::Authored(PathId::ROOT))
}

/// The single expression of a one-expression `return` statement.
fn returned_expression(body: &[AstNode]) -> &ExprId {
    let [statement] = body else {
        panic!("the fixture body must hold exactly one return statement");
    };
    let NodeKind::Return(expressions) = &statement.kind else {
        panic!("the fixture body must return a value");
    };
    let [expression] = expressions.as_slice() else {
        panic!("the fixture return must hold exactly one expression");
    };
    expression
}

#[test]
fn a_folded_fast_operation_still_records_a_witness() {
    // The wrapping addition folds to the U8 constant 0 during construction. The witness is the
    // only surviving evidence that the function selected fast arithmetic.
    let (ast, paths, strings) = parse_single_file_ast(
        "$fast_math\n\
         wrapped || -> U8:\n\
         \x20   return 200 + 56\n\
         ;\n",
    );

    let [witness] = ast.fast_operation_witnesses.as_slice() else {
        panic!(
            "one fast addition must record exactly one witness, found {:?}",
            ast.fast_operation_witnesses
        );
    };
    assert!(
        matches!(witness.function, AstFunctionIdentity::Authored(_)),
        "the witness must name the authored callable"
    );
    assert!(
        witness.span.is_some(),
        "an authored operator records its source span"
    );

    let body = function_body_by_name(&ast, &paths, &strings, "wrapped");
    let expression = returned_expression(body);
    let view = ast
        .expressions
        .view(*expression)
        .expect("the return expression is stored");
    assert!(
        matches!(view.operation(), ExpressionOperationView::Constant(_)),
        "the fast addition must fold before the runtime AST"
    );
}

#[test]
fn ordinary_arithmetic_records_no_fast_use_witness() {
    let (ast, paths, strings) = parse_single_file_ast(
        "plain || -> U8:\n\
         \x20   return 100 + 56\n\
         ;\n",
    );
    assert!(
        ast.fast_operation_witnesses.is_empty(),
        "ordinary arithmetic must record no fast use, found {:?}",
        ast.fast_operation_witnesses
    );

    let body = function_body_by_name(&ast, &paths, &strings, "plain");
    let expression = returned_expression(body);
    let view = ast
        .expressions
        .view(*expression)
        .expect("the return expression is stored");
    assert!(
        matches!(view.operation(), ExpressionOperationView::Constant(_)),
        "the ordinary addition still folds, so the absence is not a folding artefact"
    );
}

#[test]
fn recompleting_a_resolved_operation_records_one_witness() {
    // Deferred completion can resolve the same authored fragment again. Resolution reuses the
    // already-resolved operation without reporting a second selection, and the drain-time ledger
    // normalisation would collapse a repeated record anyway, so evidence stays one row.
    let paths = PathInternerFork::empty();
    let context = witness_scope(&paths);
    context
        .source_constraints
        .set(SourceConstraintSet::default().with_directive(SourceDirectiveKind::FastMath));
    let function = context
        .constraint_function
        .expect("the fixture scope installs a callable identity");

    let mut builder = AstExpressionBuilder::new().expect("test expression builder");
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let types = TypeEnvironment::new();
    let lhs = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits U8"),
            Some(marked_span(100)),
            ValueMode::ImmutableOwned,
        )
        .expect("left operand should construct")
        .id();
    let rhs = builder
        .fixed_scalar(
            &mut constants,
            FixedScalarValue::unsigned(FixedScalar::U8, 56).expect("56 fits U8"),
            Some(marked_span(140)),
            ValueMode::ImmutableOwned,
        )
        .expect("right operand should construct")
        .id();
    let operator_span = marked_span(230);
    let mut items = vec![
        ExpressionRpnItem::Operand(lhs),
        ExpressionRpnItem::Operand(rhs),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: Some(operator_span),
            preceded_by_operand: true,
        },
    ];

    let mut reported_selections = 0;
    for _completion in 0..2 {
        let resolved = resolve_expression_result_type(
            &mut items,
            None,
            None,
            &mut builder,
            &mut constants,
            &mut strings,
            &types,
            &paths,
            true,
        )
        .unwrap_or_else(|_| panic!("the fast addition should resolve on each completion"));
        reported_selections += resolved.fast_operation_spans.len();
        for span in resolved.fast_operation_spans {
            context.record_fast_operation_witness(span);
        }
    }
    assert_eq!(
        reported_selections, 1,
        "a resolved operation reused by a later completion must not report a second selection"
    );

    let scope = context.scope;
    let witnesses = context.fast_operation_witnesses.borrow();
    assert_eq!(
        witnesses.len(),
        1,
        "two completions of one authored operation must not duplicate evidence, found {witnesses:?}"
    );
    assert_eq!(
        witnesses[0],
        AstFastOperationWitness {
            function,
            scope,
            span: Some(operator_span),
            source: AstFastOperationSource::AuthoredArithmetic,
        }
    );
}

#[test]
fn the_witness_ledger_normalises_when_it_is_drained() {
    let paths = PathInternerFork::empty();
    let context = witness_scope(&paths);
    context
        .source_constraints
        .set(SourceConstraintSet::default().with_directive(SourceDirectiveKind::FastMath));
    assert!(
        context.source_constraints().fast_math_active(),
        "the fixture scope must select fast arithmetic"
    );
    let function = context
        .constraint_function
        .expect("the fixture scope installs a callable identity");
    let scope = context.scope;

    // Recording is one push per selection: a synthetic operation without an authored span and a
    // fragment that a deferred completion resolved again both reach the ledger as rows.
    context.record_fast_operation_witness(None);
    context.record_fast_operation_witness(None);
    context.record_fast_operation_witness(Some(marked_span(300)));
    context.record_fast_operation_witness(Some(marked_span(10)));
    context.record_fast_operation_witness(Some(marked_span(10)));

    // A second callable shares the module ledger, so its rows join the same drain.
    let entry_scope = witness_scope(&paths)
        .with_fast_operation_witness_sink(Rc::clone(&context.fast_operation_witnesses))
        .with_constraint_function(AstFunctionIdentity::Entry);
    let entry_scope_path = entry_scope.scope;
    entry_scope.record_fast_operation_witness(None);
    entry_scope.record_fast_operation_witness(Some(marked_span(20)));

    let mut witnesses = std::mem::take(&mut *context.fast_operation_witnesses.borrow_mut());
    assert_eq!(
        witnesses.len(),
        7,
        "recording must append rather than scan the ledger for duplicates"
    );

    normalize_fast_operation_witnesses(&mut witnesses);
    assert_eq!(
        witnesses,
        vec![
            AstFastOperationWitness {
                function: AstFunctionIdentity::Entry,
                scope: entry_scope_path,
                source: AstFastOperationSource::AuthoredArithmetic,
                span: None
            },
            AstFastOperationWitness {
                function: AstFunctionIdentity::Entry,
                scope: entry_scope_path,
                source: AstFastOperationSource::AuthoredArithmetic,
                span: Some(marked_span(20))
            },
            AstFastOperationWitness {
                function,
                scope,
                source: AstFastOperationSource::AuthoredArithmetic,
                span: None
            },
            AstFastOperationWitness {
                function,
                scope,
                source: AstFastOperationSource::AuthoredArithmetic,
                span: Some(marked_span(10))
            },
            AstFastOperationWitness {
                function,
                scope,
                source: AstFastOperationSource::AuthoredArithmetic,
                span: Some(marked_span(300))
            },
        ],
        "the drained ledger holds one row per operation in callable then span order"
    );

    // A scope without a callable identity has no verifiable boundary, so it records nothing.
    let without_function = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(vec![], &paths)),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    without_function.record_fast_operation_witness(Some(marked_span(10)));
    assert!(
        without_function
            .fast_operation_witnesses
            .borrow()
            .is_empty(),
        "a scope with no callable identity cannot own fast-use evidence"
    );
}
