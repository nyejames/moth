//! Lexical numeric-mode selection and fast HIR delivery tests.
//!
//! WHAT: exercises `$fast_math` selection through real source annotations, plus the HIR delivery
//!       that fast arithmetic and generated range guards must use.
//! WHY: resolution, folding and lowering share one selected semantics fact; only source-level
//!       tests can prove the annotation scope, the callee reset, the cast boundary and the fatal
//!       delivery that catch handlers and `Error!` contracts must not intercept.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ResolvedOperationKind;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::datatypes::numeric_operators::{NumericOperator, NumericSemantics};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::failure_facts::HirBuiltinFailureSource;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    FastStoreConversionClass, HirFastStoreConversion, HirNumericOp, LoopStepFailureCause,
    NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureCause,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Collects the selected numeric semantics of one expression in post-order.
fn collected_semantics(view: ExpressionView<'_>, out: &mut Vec<NumericSemantics>) {
    view.visit_children(|child| collected_semantics(child, out));
    let (ExpressionOperationView::NumericBinary { facts, .. }
    | ExpressionOperationView::NumericUnary { facts, .. }) = view.operation()
    else {
        return;
    };
    let ResolvedOperationKind::Numeric(numeric) = &facts.kind else {
        return;
    };
    out.push(numeric.semantics);
}

fn expression_semantics(view: ExpressionView<'_>) -> Vec<NumericSemantics> {
    let mut semantics = Vec::new();
    collected_semantics(view, &mut semantics);
    semantics
}

fn returned_expression<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function_name: &str,
) -> ExpressionView<'a> {
    let body = function_body_by_name(ast, path_fork, string_table, function_name);
    let value = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Return(values) => values.first(),
            _ => None,
        })
        .expect("fixture function should return a value");
    ast.expressions
        .view(*value)
        .expect("returned expression belongs to the parsed AST")
}

fn declaration_expression<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function_name: &str,
) -> ExpressionView<'a> {
    let body = function_body_by_name(ast, path_fork, string_table, function_name);
    let expression = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::VariableDeclaration(declaration) => match &declaration.value {
                DeclarationValue::Initializer(expression) => Some(*expression),
                _ => None,
            },
            _ => None,
        })
        .expect("fixture function should declare one initialized binding");
    ast.expressions
        .view(expression)
        .expect("declaration initializer belongs to the parsed AST")
}

fn lower_source(source: &str) -> HirModule {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    lower_hir(ast, &mut string_table, &mut path_fork, &registry)
}

fn numeric_op_statements(module: &HirModule) -> Vec<(HirNumericOp, NumericFailureMode)> {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op, failure_mode, ..
            } => Some((*op, *failure_mode)),
            _ => None,
        })
        .collect()
}

fn numeric_op_invokes(module: &HirModule) -> Vec<HirNumericOp> {
    module
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                operation: HirFallibleOperation::NumericOp { op, .. },
                ..
            } => Some(*op),
            _ => None,
        })
        .collect()
}

#[test]
fn annotations_select_fast_per_function_and_per_statement_and_reset_at_callees() {
    let source = "\
$fast_math
fast |a I32, b I32| -> I32:
    return plain(a, b) + a
;

plain |a I32, b I32| -> I32:
    return a + b
;

statement |a I32, b I32| -> I32:
    $fast_math
    first = a + b
    return first + a
;
";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);

    assert_eq!(
        expression_semantics(returned_expression(&ast, &path_fork, &string_table, "fast")),
        vec![NumericSemantics::Fast],
        "a function annotation applies to its own body"
    );
    assert_eq!(
        expression_semantics(returned_expression(
            &ast,
            &path_fork,
            &string_table,
            "plain"
        )),
        vec![NumericSemantics::Ordinary],
        "a fast caller does not change its ordinary callee"
    );
    assert_eq!(
        expression_semantics(declaration_expression(
            &ast,
            &path_fork,
            &string_table,
            "statement"
        )),
        vec![NumericSemantics::Fast],
        "a statement annotation applies to the annotated statement"
    );
    assert_eq!(
        expression_semantics(returned_expression(
            &ast,
            &path_fork,
            &string_table,
            "statement"
        )),
        vec![NumericSemantics::Ordinary],
        "the next sibling statement sees the enclosing mode again"
    );
}

#[test]
fn explicit_cast_keeps_ordinary_handling_while_its_operand_is_fast() {
    let source = "\
$fast_math
narrow |a I32, b I32| -> I16:
    return cast (a + b)
;
";
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let returned = returned_expression(&ast, &path_fork, &string_table, "narrow");
    let ExpressionOperationView::Cast { facts, .. } = returned.operation() else {
        panic!("the fixture returns an explicit cast");
    };
    assert_eq!(
        facts.handling,
        CastHandling::Implicit,
        "a plain source cast keeps the ordinary failure path under fast mode"
    );
    assert_eq!(
        expression_semantics(returned),
        vec![NumericSemantics::Fast],
        "the arithmetic operand inside an explicit cast still selects fast mode"
    );
}

#[test]
fn fast_division_inside_catch_fails_fatally_and_fast_addition_is_infallible() {
    let source = "\
$fast_math
zero || -> I32:
    return 0
;

$fast_math
division = 10 // zero() catch:
    then -1
;

$fast_math
addition = 10 + zero()
";
    let hir = lower_source(source);

    let statements = numeric_op_statements(&hir);
    let (division, division_mode) = statements
        .iter()
        .find(|(op, _)| op.operator == NumericOperator::IntegerDivide)
        .expect("fast truncating division should emit a NumericOp statement");
    assert_eq!(division.semantics, NumericSemantics::Fast);
    assert_eq!(
        *division_mode,
        NumericFailureMode::Fatal,
        "fast division by a runtime zero must fail fatally outside the catch edge"
    );

    let (addition, addition_mode) = statements
        .iter()
        .find(|(op, _)| op.operator == NumericOperator::Add)
        .expect("fast addition should emit a NumericOp statement");
    assert_eq!(addition.semantics, NumericSemantics::Fast);
    assert_eq!(
        *addition_mode,
        NumericFailureMode::Infallible,
        "total fast integer addition has no failure edge"
    );

    assert!(
        numeric_op_invokes(&hir).is_empty(),
        "fast arithmetic must never be delivered through a recoverable Invoke"
    );
}

#[test]
fn fast_mode_keeps_dec_division_ordinary_and_recoverable() {
    let source = "\
$fast_math
dec_divide |value Dec2, divisor I32| -> Dec2, Error!:
    return value / divisor
;
";
    let hir = lower_source(source);

    let invokes = numeric_op_invokes(&hir);
    let divide = invokes
        .iter()
        .find(|op| op.operator == NumericOperator::Divide)
        .expect("Dec division in an Error! function should use the recoverable edge");
    assert_eq!(
        divide.semantics,
        NumericSemantics::Ordinary,
        "Dec has no fast mode"
    );
    assert_eq!(
        divide.domain,
        NumericScalar::Number(NumberScale::new(2).expect("two is a valid Dec scale"))
    );
    assert!(
        numeric_op_statements(&hir)
            .iter()
            .all(|(op, _)| op.operator != NumericOperator::Divide),
        "ordinary Dec division must not also emit a trapping statement"
    );
}

#[test]
fn fast_range_loop_guards_and_ordinal_overflow_fail_fatally() {
    let source = "\
$fast_math
zero_step || -> I32:
    return 0
;

$fast_math
loop 0 to 3 by zero_step() |item, ordinal|:
    marker = item
;
";
    let hir = lower_source(source);

    let guard = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::LoopStepFailure {
                cause, semantics, ..
            } if *cause == LoopStepFailureCause::ZeroStep => {
                Some((*semantics, NumericFailureMode::range_guard(*semantics)))
            }
            _ => None,
        })
        .expect("the dynamic step should emit the zero-step guard");
    assert_eq!(guard.0, NumericSemantics::Fast);
    assert_eq!(guard.1, NumericFailureMode::Fatal);

    let ordinal = numeric_op_statements(&hir)
        .into_iter()
        .find(|(op, _)| op.operator == NumericOperator::Add)
        .expect("the range loop should emit its ordinal increment");
    assert_eq!(
        ordinal.0.semantics,
        NumericSemantics::Ordinary,
        "the generated ordinal is checked bookkeeping, not fast arithmetic"
    );
    assert_eq!(
        ordinal.1,
        NumericFailureMode::Fatal,
        "a fast range loop must fail fatally when its ordinal overflows"
    );
    assert!(
        numeric_op_invokes(&hir).is_empty(),
        "the fast ordinal overflow must not become a recoverable Invoke"
    );
}

#[test]
fn fast_float_range_guards_fail_fatally() {
    // A binary-float range loop has both a zero-step guard and a no-progress guard. Under
    // `$fast_math` each fails fatally, so neither may become a recoverable `Error` edge that a
    // catch handler could intercept.
    let source = "\
$fast_math
float_loop |start F64, end F64, step F64| -> F64:
    loop start to end by step |item|:
    ;
    return start
;
";
    let hir = lower_source(source);
    let guards: Vec<(LoopStepFailureCause, NumericSemantics, NumericFailureMode)> = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::LoopStepFailure {
                cause, semantics, ..
            } => Some((
                *cause,
                *semantics,
                NumericFailureMode::range_guard(*semantics),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        guards.len(),
        2,
        "a float range loop must emit its zero-step and no-progress guards"
    );
    for cause in [
        LoopStepFailureCause::ZeroStep,
        LoopStepFailureCause::NoProgress,
    ] {
        let guard = guards
            .iter()
            .find(|(guard_cause, _, _)| *guard_cause == cause)
            .unwrap_or_else(|| panic!("the float range loop must emit the {cause:?} guard"));
        assert_eq!(
            (guard.1, guard.2),
            (NumericSemantics::Fast, NumericFailureMode::Fatal),
            "the fast {cause:?} guard must fail fatally"
        );
    }
    assert!(
        !hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::LoopStepFailure { .. },
                ..
            }
        )),
        "fast float guards must not become recoverable Invoke edges"
    );
}

// -------------------------
//  Fast compound write-back
// -------------------------

fn fast_store_conversions(module: &HirModule) -> Vec<(HirFastStoreConversion, NumericFailureMode)> {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::FastStoreConversion { conversion, .. } => Some((
                *conversion,
                conversion
                    .required_failure_mode()
                    .expect("a lowered fast write-back has a permitted conversion class"),
            )),
            _ => None,
        })
        .collect()
}

fn ordinary_store_conversion_invokes(module: &HirModule) -> usize {
    module
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::CastOp { .. },
                    ..
                }
            )
        })
        .count()
}

/// Counts the ordinary checked write-back trap continuations.
///
/// WHY: the ordinary store conversion owns a trap edge; a fast write-back must never gain one.
fn store_conversion_traps(module: &HirModule) -> usize {
    module
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::RuntimeFailure {
                    cause: Some(RuntimeFailureCause::StoreConversion { .. }),
                    ..
                }
            )
        })
        .count()
}

/// Whether any function projects the recoverable compound write-back contributor.
///
/// WHY: the fast write-back is a total wrap or a fatal trap, so only the ordinary checked
///      conversion may contribute a recoverable failure fact.
fn has_compound_write_back_fact(module: &HirModule) -> bool {
    module.function_failure_facts.values().any(|facts| {
        facts.contributors.iter().any(|contributor| {
            matches!(
                contributor.source,
                HirBuiltinFailureSource::CompoundWriteBack { .. }
            )
        })
    })
}

/// Returns the conversion, its result local and the value stored immediately after it.
///
/// WHY: the store must consume the conversion result, so the write cannot precede or bypass the
///      statement that produces it.
fn fast_conversion_and_store(
    module: &HirModule,
) -> (HirFastStoreConversion, LocalId, HirPlace, HirValueId) {
    for block in &module.blocks {
        for (index, statement) in block.statements.iter().enumerate() {
            let HirStatementKind::FastStoreConversion {
                conversion,
                result: HirLocalDestination::Define(conversion_local),
                ..
            } = &statement.kind
            else {
                continue;
            };
            let (target, stored) = block.statements[index + 1..]
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Write {
                        target: HirWriteTarget::AssignPlace(place),
                        value,
                    } => Some((*place, *value)),
                    _ => None,
                })
                .expect("a compound write-back must store into its target after the conversion");
            return (*conversion, *conversion_local, target, stored);
        }
    }
    panic!("the fixture must emit one fast store conversion");
}

#[test]
fn fast_integer_compound_write_back_wraps_infallibly_before_storing() {
    let hir = lower_source(
        "$fast_math\n\
         wrap_i8 |target ~I8, increment I16|:\n\
         \x20   ~target += increment\n\
         ;\n",
    );

    assert_eq!(
        fast_store_conversions(&hir),
        vec![(
            HirFastStoreConversion {
                source: NumericScalar::Fixed(FixedScalar::I16),
                target: NumericScalar::Fixed(FixedScalar::I8),
            },
            NumericFailureMode::Infallible,
        )],
        "an integer-to-integer fast write-back is a total wrap at the destination width"
    );
    assert_eq!(
        fast_store_conversions(&hir)[0].0.classify(),
        Some(FastStoreConversionClass::IntegerWrap),
        "a mixed-width integer write-back classifies as an integer wrap"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&hir),
        0,
        "a fast integer wrap must not also use the checked store-conversion cast"
    );
    let (conversion, conversion_local, target, stored) = fast_conversion_and_store(&hir);
    assert_eq!(conversion.target, NumericScalar::Fixed(FixedScalar::I8));
    assert!(
        target.projections.is_empty(),
        "the local fixture must store into a bare local"
    );
    assert!(
        matches!(
            &hir.expressions.expression(stored).kind,
            HirExpressionKind::Load(place) if *place == HirPlace::local(conversion_local)
        ),
        "the target may only be written from the conversion result"
    );
}

#[test]
fn fast_float_compound_write_backs_trap_fatally() {
    for (source, expected, expected_class) in [
        (
            "$fast_math\n\
             halve |target ~I8, divisor I8|:\n\
             \x20   ~target /= divisor\n\
             ;\n",
            HirFastStoreConversion {
                source: NumericScalar::Fixed(FixedScalar::F64),
                target: NumericScalar::Fixed(FixedScalar::I8),
            },
            FastStoreConversionClass::FloatToInteger,
        ),
        (
            "$fast_math\n\
             shrink |target ~F32, divisor F64|:\n\
             \x20   ~target /= divisor\n\
             ;\n",
            HirFastStoreConversion {
                source: NumericScalar::Fixed(FixedScalar::F64),
                target: NumericScalar::Fixed(FixedScalar::F32),
            },
            FastStoreConversionClass::FloatToFloat,
        ),
    ] {
        let hir = lower_source(source);
        assert_eq!(
            fast_store_conversions(&hir),
            vec![(expected, NumericFailureMode::Fatal)],
            "a fast float write-back traps out of range instead of recovering"
        );
        assert_eq!(
            expected.classify(),
            Some(expected_class),
            "the fixture must exercise the named conversion class"
        );
        assert_eq!(
            ordinary_store_conversion_invokes(&hir),
            0,
            "a fast float write-back must not become a recoverable checked cast edge"
        );
        assert!(
            numeric_op_invokes(&hir).is_empty(),
            "fast arithmetic itself must never be delivered through an Invoke"
        );
    }
}

#[test]
fn fast_mode_keeps_dec_compound_conversions_ordinary() {
    // The arithmetic domain is `Dec2`, so the I8 write-back stays the ordinary checked
    // conversion even though the enclosing scope selected fast math.
    let dec_source = lower_source(
        "$fast_math\n\
         dec_source |target ~I8, increment Dec2|:\n\
         \x20   ~target += increment\n\
         ;\n",
    );
    assert!(
        fast_store_conversions(&dec_source).is_empty(),
        "a Dec arithmetic source must never select the fast write-back"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&dec_source),
        1,
        "a Dec source keeps the checked store-conversion edge"
    );
    assert_eq!(
        store_conversion_traps(&dec_source),
        1,
        "the ordinary Dec write-back keeps its trap delivery"
    );
    assert!(
        has_compound_write_back_fact(&dec_source),
        "the ordinary Dec write-back still contributes its recoverable failure fact"
    );

    // A Dec destination has no fast write-back at all: the promoted Dec result already has the
    // destination type.
    let dec_target = lower_source(
        "$fast_math\n\
         dec_target |target ~Dec2, increment I32|:\n\
         \x20   ~target += increment\n\
         ;\n",
    );
    assert!(
        fast_store_conversions(&dec_target).is_empty(),
        "a Dec destination must never receive a fast write-back"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&dec_target),
        0,
        "a same-type Dec update needs no conversion"
    );
}

#[test]
fn ordinary_compound_store_conversion_stays_checked() {
    let hir = lower_source(
        "wrap_i8 |target ~I8, increment I16|:\n\
         \x20   ~target += increment\n\
         ;\n",
    );

    assert!(
        fast_store_conversions(&hir).is_empty(),
        "ordinary mode keeps the checked store conversion"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&hir),
        1,
        "the ordinary write-back routes through the checked cast edge"
    );
    assert_eq!(
        store_conversion_traps(&hir),
        1,
        "the ordinary write-back traps instead of wrapping"
    );
    assert!(
        has_compound_write_back_fact(&hir),
        "the ordinary write-back keeps its recoverable failure fact"
    );
}

#[test]
fn explicit_cast_in_a_fast_scope_keeps_ordinary_checked_lowering() {
    let hir = lower_source(
        "$fast_math\n\
         narrow |a I32, b I32| -> I16:\n\
         \x20   return cast (a + b)\n\
         ;\n",
    );

    assert!(
        fast_store_conversions(&hir).is_empty(),
        "an explicit cast is not a compound write-back"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&hir),
        1,
        "an explicit cast keeps the ordinary checked cast edge"
    );
    let addition = numeric_op_statements(&hir)
        .into_iter()
        .find(|(op, _)| op.operator == NumericOperator::Add)
        .expect("the cast operand should still emit its fast addition");
    assert_eq!(
        (addition.0.semantics, addition.1),
        (NumericSemantics::Fast, NumericFailureMode::Infallible),
        "the arithmetic operand inside the explicit cast still selects fast mode"
    );
}

#[test]
fn fast_compound_write_back_contributes_no_recoverable_failure_fact() {
    let fast = lower_source(
        "$fast_math\n\
         wrap_i8 |target ~I8, increment I16|:\n\
         \x20   ~target += increment\n\
         ;\n",
    );
    assert!(
        !fast.function_failure_facts.is_empty(),
        "the fixture must project function failure facts"
    );
    assert!(
        !has_compound_write_back_fact(&fast),
        "a total fast wrap has no recoverable write-back failure to contribute"
    );

    let trapping_fast = lower_source(
        "$fast_math\n\
         halve |target ~I8, divisor I8|:\n\
         \x20   ~target /= divisor\n\
         ;\n",
    );
    assert!(
        !has_compound_write_back_fact(&trapping_fast),
        "a fatal fast write-back trap is not a recoverable failure fact"
    );
}

#[test]
fn fast_projected_place_write_back_stores_the_conversion_result() {
    let hir = lower_source(
        "Box = |\n\
         \x20   value I8,\n\
         |\n\
         \n\
         $fast_math\n\
         bump |box ~Box, increment I16|:\n\
         \x20   ~box.value += increment\n\
         ;\n",
    );

    assert_eq!(
        fast_store_conversions(&hir),
        vec![(
            HirFastStoreConversion {
                source: NumericScalar::Fixed(FixedScalar::I16),
                target: NumericScalar::Fixed(FixedScalar::I8),
            },
            NumericFailureMode::Infallible,
        )],
        "a projected place takes the same fast wrap as a local"
    );
    let (_, conversion_local, target, stored) = fast_conversion_and_store(&hir);
    assert!(
        !target.projections.is_empty(),
        "the store must keep its member projection"
    );
    assert!(
        matches!(
            &hir.expressions.expression(stored).kind,
            HirExpressionKind::Load(place) if *place == HirPlace::local(conversion_local)
        ),
        "a projected store may only be written from the conversion result"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&hir),
        0,
        "a fast projected write-back must not become a checked cast edge"
    );
}

#[test]
fn fast_statement_annotation_reaches_a_nested_loop_body_write_back() {
    let hir = lower_source(
        "count_up || -> I32:\n\
         \x20   total ~I32 = 2147483646\n\
         \x20   one I64 = 1\n\
         \x20   loop true:\n\
         \x20       $fast_math\n\
         \x20       ~total += one\n\
         \x20       if total is -2147483647:\n\
         \x20           break\n\
         \x20       ;\n\
         \x20   ;\n\
         \x20   return total\n\
         ;\n",
    );

    assert_eq!(
        fast_store_conversions(&hir),
        vec![(
            HirFastStoreConversion {
                source: NumericScalar::Fixed(FixedScalar::I64),
                target: NumericScalar::Fixed(FixedScalar::I32),
            },
            NumericFailureMode::Infallible,
        )],
        "the statement annotation must apply inside the loop body"
    );
    assert_eq!(
        ordinary_store_conversion_invokes(&hir),
        0,
        "the loop-body write-back must not fall back to a checked cast edge"
    );
}
