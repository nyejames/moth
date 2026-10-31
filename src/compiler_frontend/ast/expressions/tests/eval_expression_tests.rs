//! Expression evaluation and runtime-RPN parsing regression tests.
//!
//! WHAT: validates operator precedence, runtime expression node construction, and template
//!       expression parsing.
//! WHY: expression parsing is dense and easy to break during refactors; targeted tests catch
//!      semantic drift before it reaches HIR lowering.

use super::*;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueMetadata, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, ResolvedOperation,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::CompileTimeEvaluationErrorView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    InvalidNumberLiteralView, MissingOperatorOperandView, NumberLiteralCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TypeMismatchView, UnsupportedOperatorTypesView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag, DiagnosticOperator, OperatorOperandPosition,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_primary_span, ast_first_error_rendered_message,
    ast_first_error_type_display, parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use std::rc::Rc;
use std::sync::Arc;

/// A parsed start-function declaration initializer. The owning AST keeps its expression graph alive.
struct ParsedExpression {
    ast: Ast,
    root: ExprId,
}

impl ParsedExpression {
    fn view(&self) -> ExpressionView<'_> {
        self.ast
            .expressions
            .view(self.root)
            .expect("parsed root expression should be stored")
    }

    fn value_type_id(&self) -> TypeId {
        self.view()
            .value_type_id()
            .expect("parsed expression should have one value type")
    }

    fn payload(&self) -> Option<&ConstValuePayload> {
        constant_payload(&self.ast.const_values, self.view())
    }
}

fn constant_payload<'a>(
    const_values: &'a ConstValueStore,
    view: ExpressionView<'a>,
) -> Option<&'a ConstValuePayload> {
    match view.operation() {
        ExpressionOperationView::Constant(id) => const_values.payload(id),
        _ => None,
    }
}

fn assert_folded_fixed_scalar(expression: &ParsedExpression, expected: FixedScalarValue) {
    assert!(
        matches!(
            expression.payload(),
            Some(ConstValuePayload::FixedScalar(scalar)) if *scalar == expected
        ),
        "expected folded {expected:?}, got {:?}",
        expression.payload()
    );
}

fn resolved_operation_of<'a>(view: ExpressionView<'a>) -> Option<&'a ResolvedOperation> {
    match view.operation() {
        ExpressionOperationView::NumericUnary { facts, .. }
        | ExpressionOperationView::NumericBinary { facts, .. }
        | ExpressionOperationView::Compare { facts, .. }
        | ExpressionOperationView::Range { facts, .. } => Some(facts),
        _ => None,
    }
}

/// Pre-order views of `root` and every descendant expression.
fn subtree_views(root: ExpressionView<'_>) -> Vec<ExpressionView<'_>> {
    let mut views = Vec::new();
    collect_subtree_views(root, &mut views);
    views
}

fn collect_subtree_views<'a>(view: ExpressionView<'a>, views: &mut Vec<ExpressionView<'a>>) {
    views.push(view);
    view.visit_children(|child| collect_subtree_views(child, views));
}

fn first_start_declaration_expression(source: &str) -> ParsedExpression {
    nth_start_declaration_expression(source, 0)
}

fn nth_start_declaration_expression(source: &str, index: usize) -> ParsedExpression {
    let (ast, _path_fork, _string_table) = parse_single_file_ast(source);
    let root = {
        let start_function = ast
            .nodes
            .iter()
            .find(|node| matches!(node.kind, NodeKind::Function(_, _, _)))
            .expect("start function should exist");

        let NodeKind::Function(_, _, body) = &start_function.kind else {
            panic!("expected start function body");
        };
        let NodeKind::VariableDeclaration(declaration) = &body[index].kind else {
            panic!("expected start statement {index} to be a variable declaration");
        };

        declaration
            .value
            .initializer()
            .expect("parsed declaration initializer")
    };
    ParsedExpression { ast, root }
}

fn assert_unsupported_operator(source: &str, expected_operator: DiagnosticOperator) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_unsupported_operator_fact(&failure, expected_operator);
}

fn assert_unsupported_operator_fact(
    failure: &crate::compiler_frontend::compiler_errors::CompilationResult<()>,
    expected_operator: DiagnosticOperator,
) {
    let diagnostic = assert_ast_diagnostic(failure, "MOTH-TYPE-0003", None);
    let facts = diagnostic
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("unsupported-operator diagnostic should retain typed facts");
    assert_eq!(facts.operator, expected_operator);
}

fn assert_byte_has_no_arithmetic(source: &str, expected_operator: DiagnosticOperator) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_unsupported_operator_fact(&failure, expected_operator);
}

fn assert_type_mismatch_context(
    failure: &crate::compiler_frontend::compiler_errors::CompilationResult<()>,
    expected_context: TypeMismatchContext,
) {
    let diagnostic = assert_ast_diagnostic(failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("type-mismatch diagnostic should retain typed facts");
    assert_eq!(facts.context, expected_context);
}

fn assert_outside_fixed_scalar_range(
    failure: &crate::compiler_frontend::compiler_errors::CompilationResult<()>,
    expected_scalar: FixedScalar,
) {
    let diagnostic = assert_ast_diagnostic(
        failure,
        "MOTH-SYNTAX-0008",
        Some("invalid_number_literal.outside_fixed_scalar_range"),
    );
    let facts = diagnostic
        .family_view::<InvalidNumberLiteralView>()
        .expect("number-literal diagnostic should retain typed facts");
    let expected_reason = NumberLiteralErrorReason::OutsideFixedScalarRange(expected_scalar);
    let (expected_code, expected_scalar_fact, expected_scale_fact) =
        NumberLiteralCode::split(&expected_reason);
    assert_eq!(facts.code, expected_code);
    assert_eq!(facts.scalar, expected_scalar_fact);
    assert_eq!(facts.scale, expected_scale_fact);
}

/// Builds a `String` operand holding a `SiteRoot` structural piece. The piece is not final text,
/// so the operand must stay unfolded in every runtime position.
fn structural_site_root_operand(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
) -> ExprId {
    let constant = constants
        .insert_value(
            ConstValueMetadata::resolved(builtin_type_ids::STRING, None, ValueMode::ImmutableOwned),
            ConstValuePayload::String(ConstStringValue::Pieces(vec![ConstStringPiece::SiteRoot])),
        )
        .expect("structural string constant should insert");
    expressions
        .constant(
            constant,
            builtin_type_ids::STRING,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("structural string operand should build")
        .id()
}

#[test]
fn ordinary_expression_rejects_path_string_concatenation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_scope = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let context = ScopeContext::new_for_tests(
        ContextKind::Template,
        source_scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    )
    .with_source_file_scope(source_scope);

    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let structural_path = structural_site_root_operand(&mut expressions, &mut constants);
    let suffix = expressions
        .string_slice(
            &mut constants,
            string_table.get_or_intern(String::from("?v=1")),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("string slice operand should build");

    let nodes = vec![
        ExpressionRpnItem::Operand(structural_path),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: None,
        },
        ExpressionRpnItem::Operand(suffix.id()),
    ];

    let mut current_type = ExpectedType::Infer;
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let error = evaluate_expression(
        &context,
        nodes,
        &mut type_interner,
        &mut expressions,
        &mut constants,
        &mut current_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
        &path_fork,
    )
    .expect_err("ordinary expressions should stay strict");

    let crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError::Diagnostic(diagnostic) = error else {
        panic!("expected an expression type diagnostic");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    assert_eq!(diagnostic.descriptor().code, "MOTH-TYPE-0003");
    let facts = diagnostic
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("the diagnostic is an unsupported-operator failure");
    assert_eq!(facts.operator, DiagnosticOperator::Add);
}
#[test]
fn structural_string_equality_is_refused_only_in_a_constant_context() {
    // WHY: the refusal belongs only to const-required positions; rejecting runtime
    // positions would remove legal expressive power.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_scope = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let context = |kind| {
        ScopeContext::new_for_tests(
            kind,
            source_scope,
            Rc::new(TopLevelDeclarationTable::new(
                vec![],
                &PathInternerFork::empty(),
            )),
            Arc::new(ExternalPackageRegistry::new()),
            0,
        )
    };
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let structural = structural_site_root_operand(&mut expressions, &mut constants);
    let plain = expressions
        .string_slice(
            &mut constants,
            string_table.intern("plain"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("plain string operand should build");
    let nodes = || {
        vec![
            ExpressionRpnItem::Operand(structural),
            ExpressionRpnItem::Operand(plain.id()),
            ExpressionRpnItem::Operator {
                operator: Operator::Equality,
                span: None,
            },
        ]
    };

    let mut constant_type_environment = TypeEnvironment::new();
    let mut constant_compatibility_cache = TypeCompatibilityCache::new();
    let mut constant_type_interner = AstTypeInterner::new(
        &mut constant_type_environment,
        &mut constant_compatibility_cache,
    );
    let mut constant_expected_type = ExpectedType::Infer;
    let constant_error = evaluate_expression(
        &context(ContextKind::Constant),
        nodes(),
        &mut constant_type_interner,
        &mut expressions,
        &mut constants,
        &mut constant_expected_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
        &path_fork,
    )
    .expect_err("constant equality must require final structural-string text");
    let crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError::Diagnostic(
        diagnostic,
    ) = constant_error
    else {
        panic!("expected a typed structural-string diagnostic");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    let facts = diagnostic
        .family_view::<CompileTimeEvaluationErrorView>()
        .expect("the diagnostic is a compile-time evaluation failure");
    assert_eq!(
        facts.reason,
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText
    );

    let mut runtime_type_environment = TypeEnvironment::new();
    let mut runtime_compatibility_cache = TypeCompatibilityCache::new();
    let mut runtime_type_interner = AstTypeInterner::new(
        &mut runtime_type_environment,
        &mut runtime_compatibility_cache,
    );
    let mut runtime_expected_type = ExpectedType::Infer;
    let runtime_root = match evaluate_expression(
        &context(ContextKind::Function),
        nodes(),
        &mut runtime_type_interner,
        &mut expressions,
        &mut constants,
        &mut runtime_expected_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
        &path_fork,
    ) {
        Ok(root) => root,
        Err(_) => {
            panic!("runtime equality must preserve the structural operation without a diagnostic")
        }
    };

    let runtime_view = expressions
        .view_owned(runtime_root)
        .expect("runtime equality root should be stored");
    assert!(
        matches!(
            runtime_view.operation(),
            ExpressionOperationView::Compare { facts, .. }
                if facts.operator == Operator::Equality
        ),
        "runtime equality should remain a resolved Compare expression"
    );
    assert_eq!(
        subtree_views(runtime_view).len(),
        3,
        "runtime context must preserve both operands and the equality operator"
    );
}

#[test]
fn unary_not_requires_boolean_operand() {
    assert_unsupported_operator("value = not 1\n", DiagnosticOperator::Not);
}

#[test]
fn logical_and_requires_bool_operands() {
    assert_unsupported_operator("value = true and 1\n", DiagnosticOperator::And);
}

#[test]
fn logical_and_reports_found_types_in_operand_order() {
    assert_unsupported_operator("value = 1 and true\n", DiagnosticOperator::And);
}

#[test]
fn logical_or_rejects_string_operands() {
    assert_unsupported_operator("value = \"a\" or \"b\"\n", DiagnosticOperator::Or);
}

#[test]
fn logical_mix_rejects_non_bool_rhs_after_comparison() {
    assert_unsupported_operator("value = 1 < 2 and 3\n", DiagnosticOperator::And);
}

#[test]
fn scalar_type_called_as_function_is_rejected() {
    // Bool, String, Char and explicit-width numeric tags share one builtin-scalar-type
    // dispatch; a call form must stay a type-as-call diagnostic, not an unknown-symbol call.
    for source in [
        "value = Bool(true)\n",
        "value = String(1)\n",
        "value = Char(\"a\")\n",
        "value = U8(200)\n",
        "value = I32(\"42\")\n",
        "value = I64(200)\n",
        "value = Byte(200)\n",
        "value = F64(\"3.14\")\n",
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0046",
            Some("invalid_builtin_call.scalar_type_called_as_function"),
        );
    }
}

#[test]
fn generic_choice_application_in_value_position_uses_variant_guidance() {
    let source = r#"
Maybe type T ::
    Some | value T |,
    Empty,
;

value = Maybe of I32
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0037",
        Some("type_used_as_value.generic_choice_application"),
    );
    let message = ast_first_error_rendered_message(&failure);
    assert!(
        message.contains("`Maybe of ...` is a type annotation, not a value expression. Construct one of the `Maybe` variants instead"),
        "{message}"
    );
}

#[test]
fn non_generic_type_followed_by_of_keeps_ordinary_type_as_value_wording() {
    let source = r#"
Point = |
    x I32,
|

value = Point of I32
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0037",
        Some("type_used_as_value.bare_type_name"),
    );
    let message = ast_first_error_rendered_message(&failure);
    assert!(
        message.contains("'Point' is a type and cannot be used as a value"),
        "{message}"
    );
}

#[test]
fn logical_operator_rejects_option_operands_with_precise_found_type() {
    assert_unsupported_operator(
        "maybe String? = none\nvalue = maybe or true\n",
        DiagnosticOperator::Or,
    );
}

#[test]
fn comparison_operator_accepts_option_to_scalar_comparison() {
    let value =
        nth_start_declaration_expression("maybe String? = \"x\"\nvalue = maybe is \"x\"\n", 1);

    assert_eq!(value.value_type_id(), builtin_type_ids::BOOL);
}

#[test]
fn comparison_operator_rejects_none_without_option_context() {
    assert_unsupported_operator("value = none is none\n", DiagnosticOperator::Equality);
}

#[test]
fn mixed_whole_and_decimal_literal_arithmetic_resolves_to_f64() {
    let value = first_start_declaration_expression("value = 1 + 2.5\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn real_division_of_whole_literals_resolves_to_f64() {
    let value = first_start_declaration_expression("value = 5 / 2\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn grouped_integer_subexpression_does_not_override_division_result_type() {
    let (ast, _path_fork, _string_table) =
        parse_single_file_ast("value #= ((10 * 10) + (20 * 20)) / 10\n\ntyped F64 = value\n");
    let value_id = ast
        .const_values
        .iter_module_constant_views()
        .next()
        .expect("the inferred constant should be retained in module constants")
        .id;
    let value = ast
        .const_values
        .value(value_id)
        .expect("the inferred constant value should be retained");

    assert_eq!(
        value.metadata.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        matches!(&value.payload, ConstValuePayload::FixedScalar(scalar) if scalar.as_f64() == Some(50.0))
    );
}

#[test]
fn integer_division_resolves_to_i32() {
    let value = first_start_declaration_expression("value = 5 // 2\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn integer_division_rejects_int_float_operands() {
    assert_unsupported_operator("value = 5 // 2.0\n", DiagnosticOperator::IntDivide);
}

#[test]
fn integer_division_rejects_float_int_operands() {
    assert_unsupported_operator("value = 5.0 // 2\n", DiagnosticOperator::IntDivide);
}

#[test]
fn multiline_expression_with_operator_on_next_line_resolves_correctly() {
    let value = first_start_declaration_expression("value = 1\n + 2\n + 3\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::signed(FixedScalar::I32, 6).expect("6 is an I32 value"),
    );
}

#[test]
fn multiline_expression_with_operator_at_end_of_line_resolves_correctly() {
    let value = first_start_declaration_expression("value = 1 +\n 2 +\n 3\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::signed(FixedScalar::I32, 6).expect("6 is an I32 value"),
    );
}

#[test]
fn multiline_comparison_expression_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 1\n is\n 1\n");

    assert_eq!(value.value_type_id(), builtin_type_ids::BOOL);
    assert!(
        matches!(value.payload(), Some(ConstValuePayload::Bool(true))),
        "expected folded Bool(true), got {:?}",
        value.payload()
    );
}

#[test]
fn bool_relational_comparison_is_rejected() {
    assert_unsupported_operator("value = true < false\n", DiagnosticOperator::LessThan);
}

#[test]
fn string_equality_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = \"a\" is \"b\"\n");

    assert_eq!(value.value_type_id(), builtin_type_ids::BOOL);
}

#[test]
fn string_equality_accepts_quoted_and_runtime_template_values() {
    let value = nth_start_declaration_expression(
        "suffix = \"x\"\ntemplate_value = [:same[suffix]]\nvalue = \"samex\" is template_value\n",
        2,
    );

    assert_eq!(value.value_type_id(), builtin_type_ids::BOOL);
}

#[test]
fn string_ordering_comparison_is_rejected() {
    assert_unsupported_operator("value = \"a\" < \"b\"\n", DiagnosticOperator::LessThan);
}

#[test]
fn char_relational_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 'a' < 'b'\n");

    assert_eq!(value.value_type_id(), builtin_type_ids::BOOL);
}

#[test]
fn fully_constant_boolean_and_comparison_expressions_fold() {
    let (ast, _path_fork, _string_table) =
        parse_single_file_ast("flag = not (1 < 2) or (3 < 4 and false)\n");
    let start_function = ast
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Function(_, _, _)))
        .expect("start function should exist");

    let NodeKind::Function(_, _, body) = &start_function.kind else {
        panic!("expected start function body");
    };
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected folded declaration");
    };

    let root = declaration
        .value
        .initializer()
        .expect("fixture initializer");
    let view = ast
        .expressions
        .view(root)
        .expect("folded initializer should be stored");
    assert!(
        matches!(
            constant_payload(&ast.const_values, view),
            Some(ConstValuePayload::Bool(false))
        ),
        "expected fully-folded boolean/comparison expression to collapse to Bool(false)"
    );
    assert_eq!(
        view.value_type_id()
            .expect("folded boolean should have a type"),
        builtin_type_ids::BOOL
    );
}

#[test]
fn string_slice_concatenation_with_variable_is_rejected() {
    assert_unsupported_operator(
        "str1 = \"Hello\"\nvalue = str1 + \" World\"\n",
        DiagnosticOperator::Add,
    );
}

#[test]
fn folded_template_preserves_template_type_id() {
    let value = first_start_declaration_expression("value = [:template body]\n");

    assert_eq!(value.value_type_id(), builtin_type_ids::STRING);
    assert!(
        matches!(value.payload(), Some(ConstValuePayload::String(_))),
        "expected folded template to collapse to a string constant"
    );
}

#[test]
fn copied_template_string_preserves_copy_expression_kind() {
    let value = nth_start_declaration_expression(
        "source String = [:template body]\nvalue = copy source\n",
        1,
    );

    assert_eq!(value.value_type_id(), builtin_type_ids::STRING);
    assert!(
        matches!(value.view().operation(), ExpressionOperationView::Copy(_)),
        "expected explicit copy to remain a copy expression"
    );
}

#[test]
fn template_like_string_operands_reject_add() {
    let failure = parse_single_file_ast_diagnostic("value = \"left\" + \"right\"\n");
    assert_unsupported_operator_fact(&failure, DiagnosticOperator::Add);
}

#[test]
fn function_result_string_concatenation_is_rejected() {
    assert_unsupported_operator(
        "f || -> String:\n    return \"a\"\n;\nvalue = f() + \"b\"\n",
        DiagnosticOperator::Add,
    );
}
#[test]
fn direct_fixed_literal_materialises_signed_minimum_from_token_sign() {
    let value = first_start_declaration_expression("small I8 = -128\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I8)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::signed(FixedScalar::I8, -128).expect("I8 minimum should materialise"),
    );
}

#[test]
fn direct_fixed_literal_rejects_parser_negation_as_operator_result() {
    // A parser-level subtraction is an operator, so both operands must be representable at the
    // declared `I8` destination: a positive `128` is not. Folding `0 - 128` into the direct
    // `-128` spelling would wrongly accept it.
    let failure = parse_single_file_ast_diagnostic("small I8 = 0 - 128\n");
    assert_outside_fixed_scalar_range(&failure, FixedScalar::I8);
}

#[test]
fn direct_fixed_literal_materialises_u64_max() {
    let value = first_start_declaration_expression("large U64 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX)
            .expect("U64 maximum should materialise"),
    );
}

#[test]
fn direct_fixed_literal_materialises_large_u64() {
    let value = first_start_declaration_expression("large U64 = 18_000_000_000\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U64, 18_000_000_000)
            .expect("U64 literal should materialise"),
    );
}

#[test]
fn direct_fixed_literal_materialises_byte_endpoints() {
    let low = first_start_declaration_expression("raw Byte = 0\n");
    assert_folded_fixed_scalar(
        &low,
        FixedScalarValue::unsigned(FixedScalar::Byte, 0).expect("Byte zero should materialise"),
    );

    let high = first_start_declaration_expression("raw Byte = 255\n");
    assert_folded_fixed_scalar(
        &high,
        FixedScalarValue::unsigned(FixedScalar::Byte, 255)
            .expect("Byte maximum should materialise"),
    );
}

#[test]
fn direct_fixed_literal_materialises_f16_and_f32_bits() {
    let half = first_start_declaration_expression("half F16 = 0.1\n");
    assert_eq!(
        half.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F16)
    );
    assert!(
        matches!(
            half.payload(),
            Some(ConstValuePayload::FixedScalar(scalar))
                if scalar.as_f64().is_some_and(|value| value.to_bits() == moth_lexical::numeric::binary16::round_f64_to_f16(0.1).to_bits())
        ),
        "expected F16(0.1) bits, got {:?}",
        half.payload()
    );

    let single = first_start_declaration_expression("single F32 = 0.1\n");
    assert_eq!(
        single.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert!(
        matches!(
            single.payload(),
            Some(ConstValuePayload::FixedScalar(scalar))
                if scalar.as_f64().is_some_and(|value| value == f64::from(0.1f32))
        ),
        "expected F32(0.1) bits, got {:?}",
        single.payload()
    );
}

#[test]
fn direct_fixed_literal_materialises_whole_literal_into_f64() {
    let value = first_start_declaration_expression("wide F64 = 2\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        matches!(
            value.payload(),
            Some(ConstValuePayload::FixedScalar(scalar)) if scalar.as_f64() == Some(2.0)
        ),
        "expected F64(2.0), got {:?}",
        value.payload()
    );
}

#[test]
fn f64_whole_literals_materialise_directly_with_exact_identity_and_signed_zero() {
    let large = first_start_declaration_expression("large F64 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        large.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        matches!(
            large.payload(),
            Some(ConstValuePayload::FixedScalar(scalar))
                if scalar.as_f64() == Some(18_446_744_073_709_551_615_f64)
        ),
        "expected an F64 value beyond I32 range, got {:?}",
        large.payload()
    );

    let negative_zero = first_start_declaration_expression("negative F64 = -0\n");
    assert_eq!(
        negative_zero.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        matches!(
            negative_zero.payload(),
            Some(ConstValuePayload::FixedScalar(scalar))
                if scalar.as_f64().is_some_and(|value| value.to_bits() == (-0.0_f64).to_bits())
        ),
        "expected a sign-preserving F64 zero, got {:?}",
        negative_zero.payload()
    );

    let optional = first_start_declaration_expression("maybe F64? = 18_446_744_073_709_551_615\n");
    let inner = coerced_inner(&optional);
    assert_eq!(
        inner
            .value_type_id()
            .expect("optional inner should have one value type"),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(matches!(
        constant_payload(&optional.ast.const_values, inner),
        Some(ConstValuePayload::FixedScalar(scalar))
            if scalar.as_f64() == Some(18_446_744_073_709_551_615_f64)
    ));

    let f32 = first_start_declaration_expression("single F32 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        f32.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert!(matches!(
        f32.payload(),
        Some(ConstValuePayload::FixedScalar(_))
    ));

    let f64 = first_start_declaration_expression("double F64 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        f64.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(matches!(
        f64.payload(),
        Some(ConstValuePayload::FixedScalar(_))
    ));
}

#[test]
fn f64_peer_materialises_whole_literals_without_i32_range_narrowing() {
    let sum = nth_start_declaration_expression(
        "peer F64 = 1.0\nsum = peer + 18_446_744_073_709_551_615\n",
        1,
    );

    assert_eq!(
        sum.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        sum.payload().is_none(),
        "a variable-plus-literal expression should remain runtime"
    );
    assert!(
        constant_leaves(&sum.ast.const_values, sum.view())
            .iter()
            .any(|(_, payload)| matches!(
                payload,
                ConstValuePayload::FixedScalar(scalar)
                    if scalar.as_f64() == Some(18_446_744_073_709_551_615_f64)
            )),
        "the immediate F64 peer should materialise the whole literal as F64"
    );
}

#[test]
fn compound_f64_subtree_materialises_lone_peer_as_f64() {
    // A compound decimal subtree resolves as `F64` before the lone whole literal reads its
    // floating-family peer, so no I32 range narrowing applies. The comparison shape stays Bool
    // while its floating side folds the same way.
    for source in [
        "value = 0.5 + 0.5 + 3000000000\n",
        "value = (0.5 + 0.5) + 3000000000\n",
        "value = 1.5 * 2.0 * 3000000000\n",
        "value = 3000000000 * (1.5 + 0.5)\n",
        "value F64 = -(0.5) + 3000000000\n",
    ] {
        let expression = first_start_declaration_expression(source);
        assert_eq!(
            expression.value_type_id(),
            builtin_type_ids::fixed_scalar(FixedScalar::F64),
            "compound F64 peer should keep F64 for {source:?}"
        );
    }
    let comparison = first_start_declaration_expression("value = 0.5 + 0.5 < 3000000000\n");
    assert_eq!(comparison.value_type_id(), builtin_type_ids::BOOL);
}

#[test]
fn i32_and_fixed_receiving_context_keeps_integer_rules_and_does_not_retag_operator_results() {
    // The exact `I32` receiver owns the range check, so the literal reports the fixed-scalar
    // reason for its destination.
    let int_overflow = parse_single_file_ast_diagnostic("large I32 = 18_446_744_073_709_551_615\n");
    assert_outside_fixed_scalar_range(&int_overflow, FixedScalar::I32);

    let decimal_int = parse_single_file_ast_diagnostic("value I32 = 1.0\n");
    assert_type_mismatch_context(&decimal_int, TypeMismatchContext::Declaration);

    let unconstrained = first_start_declaration_expression("value = 1 + 1\n");
    assert_eq!(
        unconstrained.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_folded_fixed_scalar(
        &unconstrained,
        FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 is an I32 value"),
    );

    // Raw literal arithmetic is typed by the receiving context before operator selection, so a
    // `F32` receiver materialises both whole literals and folds the `F32` sum.
    let literal_arithmetic = first_start_declaration_expression("value F32 = 1 + 1\n");
    assert_eq!(
        literal_arithmetic.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );

    // Concrete named operands are never retagged by the receiver: two `I32` operands produce
    // `I32`, and the exact `F32` annotation rejects that completed result.
    let concrete_operands =
        parse_single_file_ast_diagnostic("left I32 = 1\nright I32 = 2\nvalue F32 = left + right\n");
    assert_type_mismatch_context(&concrete_operands, TypeMismatchContext::Declaration);
}

#[test]
fn direct_fixed_literal_materialises_option_of_fixed() {
    let value = nth_start_declaration_expression("maybe U8? = 200\n", 0);

    let inner = coerced_inner(&value);
    assert!(
        matches!(
            constant_payload(&value.ast.const_values, inner),
            Some(ConstValuePayload::FixedScalar(scalar))
                if *scalar == FixedScalarValue::unsigned(FixedScalar::U8, 200)
                    .expect("U8 literal should materialise before wrapping")
        ),
        "expected U8(200) inside the option wrap, got {:?}",
        constant_payload(&value.ast.const_values, inner)
    );
}

#[test]
fn literal_arithmetic_into_fixed_uses_the_receiving_context() {
    // An exact receiving context types raw literal operands before operator selection, so
    // equal `U8` operands keep `U8` and the checked constant folds instead of being promoted to
    // a wider domain and rejected at the exact receiver.
    let value = first_start_declaration_expression("small U8 = 1 + 1\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U8, 2).expect("2 is a U8 value"),
    );
}

#[test]
fn operator_result_into_u64_uses_the_receiving_context() {
    // The explicit `U64` receiver types the whole-number leaves before operator selection, so
    // the complete literal keeps its exact value without an I32 intermediate. The defaulted
    // no-receiver boundary is covered by `unannotated_overflow_keeps_default_int_range_error`.
    let value = first_start_declaration_expression("large U64 = 18_000_000_000 + 1\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U64, 18_000_000_001)
            .expect("18000000001 is a U64 value"),
    );
}

#[test]
fn decimal_spelling_into_integer_fixed_reports_declaration_mismatch() {
    for source in ["small U8 = 1.0\n", "raw Byte = 1.0\n"] {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_type_mismatch_context(&failure, TypeMismatchContext::Declaration);
    }
}

#[test]
fn grouped_literal_into_fixed_materialises_destination() {
    let value = first_start_declaration_expression("small U8 = (5)\n");

    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U8, 5)
            .expect("grouped U8 literal should materialise"),
    );
}

#[test]
fn grouped_literal_arithmetic_into_fixed_materialises_destination() {
    let value = first_start_declaration_expression("small U8 = (5) + 1\n");

    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_folded_fixed_scalar(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U8, 6).expect("6 is a U8 value"),
    );
}

#[test]
fn parser_negation_over_out_of_range_operand_reports_fixed_range_error() {
    // `-(128)` is an operator negation, so the operand literal must itself be representable at
    // the I8 destination: a positive `128` is not, unlike the direct `-128` spelling whose sign
    // is parser-owned (covered by
    // `direct_fixed_literal_materialises_signed_minimum_from_token_sign`).
    let failure = parse_single_file_ast_diagnostic("small I8 = -(128)\n");
    assert_outside_fixed_scalar_range(&failure, FixedScalar::I8);
}

#[test]
fn curly_literal_into_fixed_reports_baseline_mismatch() {
    let failure = parse_single_file_ast_diagnostic("small U8 = {1}\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("type-mismatch diagnostic should retain typed facts");
    assert_eq!(facts.context, TypeMismatchContext::Declaration);
    assert_eq!(ast_first_error_type_display(&failure, facts.expected), "U8");
    assert_ne!(
        ast_first_error_type_display(&failure, facts.found),
        "String"
    );
}

#[test]
fn unannotated_overflow_keeps_default_int_range_error() {
    // An unconstrained whole literal defaults to the exact `I32` domain, so an above-range
    // value reports the fixed-scalar range reason for that destination.
    let failure = parse_single_file_ast_diagnostic("value = 3_000_000_000\n");
    assert_outside_fixed_scalar_range(&failure, FixedScalar::I32);
}

#[test]
fn fixed_width_arithmetic_uses_exact_result_domains() {
    // Equal operand types keep their own type, same-signedness mixes take the wider
    // width, mixed signedness takes the smallest containing signed type, and real division
    // always produces F64. No 32-bit integer or minimum-F32 floor applies.
    let cases = [
        ("U8", "+", "U8", FixedScalar::U8),
        ("I32", "+", "U32", FixedScalar::I64),
        ("I8", "*", "I16", FixedScalar::I16),
        ("U32", "-", "U64", FixedScalar::U64),
        ("U8", "/", "U8", FixedScalar::F64),
        ("U8", "//", "U8", FixedScalar::U8),
        ("I64", "%", "I8", FixedScalar::I64),
        ("F16", "*", "F16", FixedScalar::F16),
        ("F16", "+", "F64", FixedScalar::F64),
    ];

    for (left, operator, right, result) in cases {
        assert_binary_result_type(
            left,
            operator,
            right,
            builtin_type_ids::fixed_scalar(result),
        );
    }

    assert_binary_result_type(
        "I32",
        "+",
        "I32",
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    assert_binary_result_type(
        "F64",
        "+",
        "F64",
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
    );
    assert_binary_result_type(
        "I32",
        "/",
        "I32",
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
    );
}

#[test]
fn unsupported_fixed_width_arithmetic_keeps_operator_diagnostics() {
    let cases = [
        ("I64", "+", "U64", DiagnosticOperator::Add),
        ("I32", "+", "U64", DiagnosticOperator::Add),
        ("I32", "+", "F64", DiagnosticOperator::Add),
        ("F64", "+", "I32", DiagnosticOperator::Add),
        ("U8", "+", "F32", DiagnosticOperator::Add),
        ("Byte", "+", "Byte", DiagnosticOperator::Add),
        ("F32", "//", "F32", DiagnosticOperator::IntDivide),
    ];

    for (left, operator, right, diagnostic_operator) in cases {
        assert_binary_rejected(left, operator, right, diagnostic_operator);
    }
}

#[test]
fn fixed_width_comparisons_use_compatibility_without_arithmetic_promotion() {
    for (left, operator, right) in [
        ("Byte", "is", "Byte"),
        ("Byte", "<", "Byte"),
        ("I64", "<", "U64"),
        ("F16", "is", "F64"),
    ] {
        assert_binary_result_type(left, operator, right, builtin_type_ids::BOOL);
    }

    assert_binary_rejected("Byte", "is", "U8", DiagnosticOperator::Equality);
    assert_binary_rejected("U8", "is", "F32", DiagnosticOperator::Equality);
}

#[test]
fn unary_minus_uses_the_numeric_negation_domain() {
    // Unary minus preserves a signed integer or binary float operand type, so `-I8` is
    // `I8` and `-F16` is `F16`; every unsigned or `Byte` operand stays invalid.
    for (operand_type, result_scalar) in [
        ("I8", FixedScalar::I8),
        ("F16", FixedScalar::F16),
        ("I64", FixedScalar::I64),
    ] {
        let source = format!("operand {operand_type} = 1\nvalue = -operand\n");
        let value = nth_start_declaration_expression(&source, 1);
        assert_eq!(
            value.value_type_id(),
            builtin_type_ids::fixed_scalar(result_scalar),
            "unexpected negation result for {operand_type}"
        );
    }

    assert_unsupported_operator("zero U8 = 0\nvalue = -zero\n", DiagnosticOperator::Subtract);
    assert_unsupported_operator(
        "zero #U8 = 0\nvalue = -zero\n",
        DiagnosticOperator::Subtract,
    );
    assert_byte_has_no_arithmetic(
        "octet Byte = 0\nvalue = -octet\n",
        DiagnosticOperator::Subtract,
    );
}

#[test]
fn pending_numeric_literals_materialise_from_immediate_fixed_peers() {
    // Both operands resolve to `U8`, so the checked result keeps `U8`; the raw literal
    // still materialises directly at its peer's exact domain.
    let right_peer = nth_start_declaration_expression("x U8 = 5\nvalue = x + 1\n", 1);
    assert_eq!(
        right_peer.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_runtime_contains_fixed_literal(
        &right_peer,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );

    let left_peer = nth_start_declaration_expression("x U8 = 5\nvalue = 1 + x\n", 1);
    assert_eq!(
        left_peer.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_runtime_contains_fixed_literal(
        &left_peer,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );

    let float_peer = nth_start_declaration_expression("f F32 = 1\nvalue = f * 0.5\n", 1);
    assert_eq!(
        float_peer.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert_runtime_contains_fixed_literal(
        &float_peer,
        FixedScalarValue::binary_float(FixedScalar::F32, 0.5)
            .expect("0.5 is exactly representable as F32"),
    );

    let byte_comparison =
        nth_start_declaration_expression("raw Byte = 255\nvalue = raw is 255\n", 1);
    assert_eq!(byte_comparison.value_type_id(), builtin_type_ids::BOOL);
    assert_runtime_contains_fixed_literal(
        &byte_comparison,
        FixedScalarValue::unsigned(FixedScalar::Byte, 255)
            .expect("Byte maximum should materialise"),
    );

    let chained = nth_start_declaration_expression("x U8 = 5\nvalue = x + 1 + 1\n", 1);
    assert_eq!(
        chained.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );

    let grouped_literal = nth_start_declaration_expression("x U8 = 5\nvalue = x + (1)\n", 1);
    assert_eq!(
        grouped_literal.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
    assert_runtime_contains_fixed_literal(
        &grouped_literal,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );
}

#[test]
fn peer_literal_materialisation_preserves_natural_and_grouped_operator_types() {
    let out_of_range = parse_single_file_ast_diagnostic("x U8 = 5\nvalue = x + 300\n");
    assert_outside_fixed_scalar_range(&out_of_range, FixedScalar::U8);

    // A decimal leaf beside an exact integer peer keeps its own family: the integer
    // destination is dropped for a decimal token, the leaf defaults to F64, and the family mix
    // is rejected instead of inserting an implicit conversion.
    assert_unsupported_operator("x U8 = 5\nvalue = x + 1.5\n", DiagnosticOperator::Add);

    // A compound literal-only subtree beside an exact fixed peer takes that peer's domain in
    // either operand order, so grouped whole-literal arithmetic materialises as U8.
    for source in [
        "x U8 = 5\nvalue = x + (1 + 2)\n",
        "x U8 = 5\nvalue = 1 + 1 + x\n",
    ] {
        let expression = nth_start_declaration_expression(source, 1);
        assert_eq!(
            expression.value_type_id(),
            builtin_type_ids::fixed_scalar(FixedScalar::U8),
            "grouped literal arithmetic beside a U8 peer should keep U8 for {source:?}"
        );
    }

    // A range header combines its endpoints through the shared Add table, so a defaulted I32
    // end beside a U8 start selects the I32 counter domain instead of failing.
    let range_value = nth_start_declaration_expression("x U8 = 5\nvalue = x to 10\n", 1);
    assert_eq!(range_value.value_type_id(), builtin_type_ids::RANGE);
}

#[test]
fn optional_fixed_scalar_does_not_materialise_pending_peers_as_inner_scalars() {
    let failure = parse_single_file_ast_diagnostic(
        "optional_value U8? = none\nresult = optional_value + 1\n",
    );
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0003", None);
    let facts = diagnostic
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("unsupported-operator diagnostic should retain typed facts");
    assert_eq!(facts.operator, DiagnosticOperator::Add);
    let rhs = facts
        .rhs
        .expect("pending whole literal should remain the reported right operand");
    assert_eq!(ast_first_error_type_display(&failure, rhs), "I32");
}

#[test]
fn number_operator_domains_keep_scale_and_reject_invalid_families() {
    let add = binary_expression("Dec2", "+", "I32");
    assert_number_scale(&add, 2);

    let reversed_subtract = binary_expression("U64", "-", "Dec2");
    assert_number_scale(&reversed_subtract, 2);

    let mixed_comparison = binary_expression("Dec2", "<", "I64");
    assert_eq!(mixed_comparison.value_type_id(), builtin_type_ids::BOOL);

    let int_power = binary_expression("Dec2", "^", "I32");
    assert_number_scale(&int_power, 2);

    let decimal_division = binary_expression("Dec2", "/", "Dec2");
    assert_number_scale(&decimal_division, 2);

    let integer_division = binary_expression("Dec0", "//", "Dec0");
    assert_number_scale(&integer_division, 0);

    let remainder = binary_expression("Dec2", "%", "Dec2");
    assert_number_scale(&remainder, 2);

    for (left, operator, right, diagnostic_operator) in [
        ("Dec", "/", "Dec", DiagnosticOperator::Divide),
        ("Dec2", "//", "Dec2", DiagnosticOperator::IntDivide),
        ("Dec2", "+", "Dec3", DiagnosticOperator::Add),
        ("Dec2", "+", "F64", DiagnosticOperator::Add),
        ("Dec2", "+", "Byte", DiagnosticOperator::Add),
        ("Dec2", "^", "Dec2", DiagnosticOperator::Exponent),
        ("Dec2", "^", "I64", DiagnosticOperator::Exponent),
    ] {
        assert_binary_rejected(left, operator, right, diagnostic_operator);
    }
}
#[test]
fn pending_literal_subtrees_use_the_local_dec_peer_in_both_orders() {
    let source = concat!(
        "base ~Dec2 = 1.25\n",
        "right = base + 1\n",
        "left = 1 + base\n",
        "group_right = base + (0.1 + 0.2)\n",
        "group_left = (0.1 + 0.2) + base\n",
        "flat_left = 0.1 + 0.2 + base\n",
        "product_right = base + 0.1 * 2\n",
        "product_left = 0.1 * 2 + base\n",
        "same = base is (0.1 + 0.2)\n",
        "negated = -base\n",
    );
    let base = first_start_declaration_expression(source);
    assert_number_literal(&base, base.value_type_id(), "125", 2);

    for (index, coefficient) in [
        (1, "100"),
        (2, "100"),
        (3, "30"),
        (4, "30"),
        (5, "30"),
        (6, "20"),
        (7, "20"),
    ] {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(expression.value_type_id(), base.value_type_id());
        assert_runtime_contains_number_literal(&expression, base.value_type_id(), coefficient, 2);
    }
    let same = nth_start_declaration_expression(source, 8);
    assert_eq!(same.value_type_id(), builtin_type_ids::BOOL);
    assert_runtime_contains_number_literal(&same, base.value_type_id(), "30", 2);
    let negated = nth_start_declaration_expression(source, 9);
    assert_eq!(negated.value_type_id(), base.value_type_id());
    assert_number_scale(&negated, 2);
}

#[test]
fn u32_receiver_types_literal_arithmetic_before_evaluation() {
    // The U32 receiver reaches whole-number leaves through arithmetic and groups; leaves
    // materialise as U32 directly, never via a signed-integer intermediate. Literal-only U32 arithmetic
    // folds to its checked value.
    let source = concat!(
        "count U32 = 1 + 2\n",
        "large U32 = 3_000_000_000 + 1\n",
        "grouped U32 = (1 + 2) * 3\n",
    );
    for (index, expected) in [3_u64, 3_000_000_001, 9].iter().enumerate() {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(
            expression.value_type_id(),
            builtin_type_ids::fixed_scalar(FixedScalar::U32)
        );
        assert_folded_fixed_scalar(
            &expression,
            FixedScalarValue::unsigned(FixedScalar::U32, *expected)
                .expect("fixture value is a U32 value"),
        );
        assert_completed_expression_tree(&expression);
    }

    // The folded-I32 analogue still folds.
    let natural = first_start_declaration_expression("value = 1 + 2\n");
    assert_eq!(
        natural.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_folded_fixed_scalar(
        &natural,
        FixedScalarValue::signed(FixedScalar::I32, 3).expect("3 is an I32 value"),
    );
}

#[test]
fn u32_peer_types_literal_only_subtree_from_either_side() {
    // A mutable U32 base keeps the peer sum runtime, mirroring the Dec peer test: the
    // literal-only side reads U32 context from the peer, and folds that side to its checked
    // U32 value inside the runtime sum.
    let source = concat!(
        "peer ~U32 = 10\n",
        "from_right = (1 + 2) + peer\n",
        "from_left = peer + (1 + 2)\n",
    );
    let peer = first_start_declaration_expression(source);
    assert_eq!(
        peer.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_folded_fixed_scalar(
        &peer,
        FixedScalarValue::unsigned(FixedScalar::U32, 10).expect("10 is a U32 value"),
    );

    for index in 1..=2 {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(
            expression.value_type_id(),
            peer.value_type_id(),
            "a U32 peer must type the opposite literal-only subtree"
        );
        assert_completed_expression_tree(&expression);
        assert_runtime_contains_fixed_literal(
            &expression,
            FixedScalarValue::unsigned(FixedScalar::U32, 3).expect("3 is a U32 value"),
        );
    }

    // A subtree holding a typed I32 operand is not literal-only: the receiver still refuses
    // to convert the named operand, so the mixed-width result is rejected at the declaration
    // instead of being retagged to the receiver.
    let typed = parse_single_file_ast_diagnostic("signed I32 = 1\ninvalid U32 = signed + 2\n");
    let typed_diagnostic = assert_ast_diagnostic(&typed, "MOTH-TYPE-0001", None);
    let typed_facts = typed_diagnostic
        .family_view::<TypeMismatchView>()
        .expect("typed I32 mismatch should retain type facts");
    assert_eq!(typed_facts.context, TypeMismatchContext::Declaration);
    assert_eq!(
        ast_first_error_type_display(&typed, typed_facts.expected),
        "U32"
    );
    // Named I32 constants stay typed under a U32 receiver for the same reason.
    let named = parse_single_file_ast_diagnostic("left #I32 = 1\nresult U32 = left + 2\n");
    let named_diagnostic = assert_ast_diagnostic(&named, "MOTH-TYPE-0001", None);
    let named_facts = named_diagnostic
        .family_view::<TypeMismatchView>()
        .expect("named I32 mismatch should retain type facts");
    assert_eq!(named_facts.context, TypeMismatchContext::Declaration);
    assert_eq!(
        ast_first_error_type_display(&named, named_facts.expected),
        "U32"
    );
}

#[test]
fn u32_power_exponent_shares_the_u32_domain() {
    // U32 power uses ordinary same-family arithmetic: the raw exponent receives U32 context,
    // unlike the Dec exponent which keeps I32. A mutable base keeps the peer power shape
    // runtime while folding the literal-only exponent to its checked U32 value inside it.
    let source = concat!("base ~U32 = 2\n", "value = base ^ (1 + 2)\n");
    let value = nth_start_declaration_expression(source, 1);
    assert_eq!(
        value.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_completed_expression_tree(&value);
    assert_runtime_contains_fixed_literal(
        &value,
        FixedScalarValue::unsigned(FixedScalar::U32, 3).expect("3 is a U32 value"),
    );

    // A U32 receiver types the whole power shape, base and grouped exponent alike, and folds
    // it to the checked value itself.
    let received = first_start_declaration_expression("value U32 = 2 ^ (1 + 2)\n");
    assert_eq!(
        received.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_completed_expression_tree(&received);
    assert_folded_fixed_scalar(
        &received,
        FixedScalarValue::unsigned(FixedScalar::U32, 8).expect("8 is a U32 value"),
    );
    // Dec power still keeps its exponent in I32.
    let dec = nth_start_declaration_expression("base ~Dec2 = 1.5\npower = base ^ (1 + 2)\n", 1);
    assert_runtime_number_power_keeps_i32_exponent(&dec);
}

#[test]
fn dec_receiver_keeps_u32_base_power_exponent_in_u32() {
    // Under a Dec receiver the exponent edge follows the base type: a U32 base routes its
    // grouped exponent through U32 context, so `3 ^ 2` stays a U32 input to the exact Dec
    // mixed rule.
    let source = concat!("base Dec2 = 1.5\n", "scaled Dec2 = base * 3 ^ 2\n",);
    let scaled = nth_start_declaration_expression(source, 1);
    assert_number_scale(&scaled, 2);
    assert_completed_expression_tree(&scaled);

    // A Dec base under the same receiver keeps the exponent in I32.
    let dec_base =
        nth_start_declaration_expression("base Dec2 = 1.5\nscaled Dec2 = base * base ^ 2\n", 1);
    assert_runtime_number_power_keeps_i32_exponent(&dec_base);

    // A typed U32 exponent is still not an implicit Dec exponent.
    assert_unsupported_operator(
        "base Dec2 = 1.5\nfactor U32 = 3\nscaled Dec2 = base ^ factor\n",
        DiagnosticOperator::Exponent,
    );
}

#[test]
fn dec_receiver_left_nested_powers_resolve_each_level_once() {
    // `((base ^ 1) ^ 1) ^ 1` with a typed Dec base under a Dec receiver:
    // every exponent edge follows the Dec base, so each `1` stays I32.
    // The receiver schedules each base once; resolving the exponent
    // must not revisit an already-resolved base subtree.
    let source = concat!("base Dec2 = 1.5\n", "cubed Dec2 = ((base ^ 1) ^ 1) ^ 1\n",);
    let base = first_start_declaration_expression(source);
    let cubed = nth_start_declaration_expression(source, 1);
    assert_eq!(cubed.value_type_id(), base.value_type_id());
    assert_completed_expression_tree(&cubed);
    assert!(
        cubed.payload().is_none(),
        "nested Dec power keeps a runtime base, got {:?}",
        cubed.payload()
    );
    let views = subtree_views(cubed.view());
    assert_eq!(
        views
            .iter()
            .filter(|view| resolved_operation_of(**view)
                .is_some_and(|operation| operation.operator == Operator::Exponent))
            .count(),
        3,
        "every nesting level keeps its power operator"
    );
    let leaves = constant_leaves(&cubed.ast.const_values, cubed.view());
    assert_eq!(
        leaves
            .iter()
            .filter(|(type_id, payload)| {
                *type_id == builtin_type_ids::fixed_scalar(FixedScalar::I32)
                    && matches!(
                        payload,
                        ConstValuePayload::FixedScalar(value)
                            if *value == FixedScalarValue::signed(FixedScalar::I32, 1)
                                .expect("1 is an I32 value")
                    )
            })
            .count(),
        3,
        "every nested exponent stays an exact I32(1)"
    );
    assert!(
        !leaves
            .iter()
            .any(|(_, payload)| matches!(payload, ConstValuePayload::Number(_))),
        "no nested exponent may materialise as Number"
    );
}

#[test]
fn u32_decimal_mix_rejects_and_completed_f64_rejects_into_u32() {
    // Integer-to-float mixing has no convenience path: a decimal leaf beside a
    // U32 operand does not select a float result without an explicit conversion.
    assert_unsupported_operator(
        "peer U32 = 10\nvalue = peer + 1.5\n",
        DiagnosticOperator::Add,
    );
    assert_unsupported_operator(
        "peer U32 = 10\nresult U32 = peer + 1.5\n",
        DiagnosticOperator::Add,
    );

    // The completed F64 result cannot coerce back into a U32 receiver.
    let completed = parse_single_file_ast_diagnostic("result U32 = 1.5 + 2.5\n");
    assert_type_mismatch_context(&completed, TypeMismatchContext::Declaration);

    // Unary minus on a typed U32 is a compile error, including known zero.
    assert_unsupported_operator(
        "peer U32 = 10\nvalue = -peer\n",
        DiagnosticOperator::Subtract,
    );
    assert_unsupported_operator(
        "peer U32 = 0\nvalue = -peer\n",
        DiagnosticOperator::Subtract,
    );
}

#[test]
fn u32_grouped_compound_exponent_keeps_u32_rhs() {
    // Grouped `^=` on a U32 target keeps ordinary same-family arithmetic: the
    // grouped right side receives U32 context, while Dec keeps the I32 edge.
    let source = concat!("powered ~U32 = 2\n", "~powered ^= 1 + 2\n");
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::Assignment { target, value } = &body[1].kind else {
        panic!("expected grouped U32 exponent compound assignment");
    };
    let target_type = ast
        .expressions
        .place(*target)
        .expect("compound target place should be stored")
        .metadata()
        .type_id;
    let root = *value;
    let expression = ParsedExpression { ast, root };
    assert_eq!(expression.value_type_id(), target_type);
    assert_eq!(
        expression.value_type_id(),
        builtin_type_ids::fixed_scalar(FixedScalar::U32),
        "grouped U32 ^= must keep a U32 right side"
    );
    assert_completed_expression_tree(&expression);
}

#[test]
fn dec_peer_and_receiver_differ_on_partly_typed_inner_sums() {
    // Without a receiver the inner `integer + 1` stays checked I32 arithmetic: the peer is
    // not a Dec and the subtree is not literal-only, so `1` keeps a fixed I32 identity.
    let peer_source = "value Dec4 = 1\ninteger I32 = 7\nresult = value + (integer + 1)\n";
    let peer_inner = nth_start_declaration_expression(peer_source, 2);
    assert!(
        peer_inner.payload().is_none(),
        "a partly typed peer sum should stay runtime, got {:?}",
        peer_inner.payload()
    );
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut int_literals = constant_leaves(&peer_inner.ast.const_values, peer_inner.view())
        .into_iter()
        .filter(|(type_id, payload)| {
            *type_id == i32_type && matches!(payload, ConstValuePayload::FixedScalar(_))
        });
    let (_, one) = int_literals
        .next()
        .expect("the inner I32 sum should keep its `1` as a fixed I32 literal");
    assert!(matches!(
        one,
        ConstValuePayload::FixedScalar(value)
            if *value == FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 is an I32 value")
    ));
    assert!(int_literals.next().is_none());

    // Under an explicit Dec4 receiver the context reaches `1`, so the inner operation is
    // mixed Dec4 and `1` materialises as a Dec4 `Number`.
    let received_source = "value Dec4 = 1\ninteger I32 = 7\nresult Dec4 = value + (integer + 1)\n";
    let received = nth_start_declaration_expression(received_source, 2);
    assert_eq!(
        received.value_type_id(),
        nth_start_declaration_expression(received_source, 0).value_type_id()
    );
    assert_runtime_contains_number_literal(
        &received,
        nth_start_declaration_expression(received_source, 0).value_type_id(),
        "10000",
        4,
    );

    // `(i + j) + 1` under a Dec4 receiver keeps the inner sum checked I32 while the lone
    // `1` reads the mixed outer peer as Dec4; `i + (j + 1)` lets the receiver reach `1`
    // so both sums produce Dec4.
    let grouped_source = "left I32 = 3\nright I32 = 4\nouter Dec4 = (left + right) + 1\n";
    let outer = nth_start_declaration_expression(grouped_source, 2);
    assert!(
        outer.payload().is_none(),
        "a grouped inner-I32 sum should stay runtime, got {:?}",
        outer.payload()
    );
    // The inner `left + right` keeps checked I32 typing while the outer peer stays runtime.
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let i32_operands = subtree_views(outer.view())
        .into_iter()
        .filter(|view| {
            resolved_operation_of(*view).is_none()
                && view
                    .value_type_id()
                    .expect("operand should have one value type")
                    == i32_type
        })
        .count();
    assert!(
        i32_operands >= 2,
        "the inner I32 sum should keep both I32 operands"
    );
    assert_runtime_contains_number_literal(
        &outer,
        nth_start_declaration_expression("anchor Dec4 = 0\n", 0).value_type_id(),
        "10000",
        4,
    );
    let nested_source = "left I32 = 3\nright I32 = 4\nouter Dec4 = left + (right + 1)\n";
    let nested = nth_start_declaration_expression(nested_source, 2);
    assert_eq!(
        nested.value_type_id(),
        nth_start_declaration_expression("anchor Dec4 = 0\n", 0).value_type_id()
    );
}

#[test]
fn dec_contextual_arithmetic_preserves_literal_type_and_coefficient() {
    let source = concat!(
        "anchor Dec4 = 0\n",
        "sum Dec4 = 0.1 + 0.2\n",
        "grouped Dec4 = ((0.1 + 0.2))\n",
        "product Dec4 = (1 + 2) * 3\n",
        "negated Dec4 = -(0.1 + 0.2)\n",
        "power Dec4 = 2 ^ (1 + 2)\n",
        "right_power Dec4 = 2 ^ 3 ^ 2\n",
        "left_power Dec4 = (2 ^ 3) ^ 2\n",
        "base ~Dec4 = 2\n",
        "runtime_power Dec4 = base ^ (1 + 2)\n",
        "exponent ~I32 = 3\n",
        "runtime_base Dec4 = 2 ^ exponent\n",
    );
    let anchor = first_start_declaration_expression(source);
    for (index, coefficient) in [
        (1, "3000"),
        (2, "3000"),
        (3, "90000"),
        (4, "-3000"),
        (5, "80000"),
        (6, "5120000"),
        (7, "640000"),
    ] {
        let expression = nth_start_declaration_expression(source, index);
        assert_number_literal(&expression, anchor.value_type_id(), coefficient, 4);
    }

    for index in [9, 11] {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(expression.value_type_id(), anchor.value_type_id());
        let ExpressionOperationView::NumericBinary {
            left: base,
            right: exponent,
            facts,
            ..
        } = expression.view().operation()
        else {
            panic!("a mutable power operand should keep the expression runtime");
        };
        assert_eq!(facts.operator, Operator::Exponent);
        assert_eq!(
            base.value_type_id()
                .expect("power base should have one value type"),
            anchor.value_type_id()
        );
        assert_eq!(
            exponent
                .value_type_id()
                .expect("power exponent should have one value type"),
            builtin_type_ids::fixed_scalar(FixedScalar::I32)
        );
        if index == 9 {
            assert!(matches!(
                constant_payload(&expression.ast.const_values, exponent),
                Some(ConstValuePayload::FixedScalar(value))
                    if *value == FixedScalarValue::signed(FixedScalar::I32, 3)
                        .expect("3 is an I32 value")
            ));
        } else {
            assert_number_view(
                &expression.ast.const_values,
                base,
                anchor.value_type_id(),
                "20000",
                4,
            );
        }
    }
}

#[test]
fn malformed_groups_keep_group_operand_diagnostics() {
    // `x = 2 * (1 + )` must report the dangling `+`, not the outer `*`: eager group
    // evaluation keeps the diagnostic at the group's own operator span.
    let plus_source = "value = 2 * (1 + )\n";
    let plus = parse_single_file_ast_diagnostic(plus_source);
    let plus_diagnostic = assert_ast_diagnostic(&plus, "MOTH-SYNTAX-0024", None);
    let plus_facts = plus_diagnostic
        .family_view::<MissingOperatorOperandView>()
        .expect("missing-operand diagnostic should retain typed position");
    assert_eq!(plus_facts.position, OperatorOperandPosition::BinaryLeft);
    let mut span_builder = ExtendedSpanBuilder::new();
    let plus_span = LocalSpan::exact(
        plus_source
            .find(" + ")
            .expect("dangling plus should be present") as u32
            + 1,
        1,
        &mut span_builder,
    )
    .expect("the dangling operator span should fit inline");
    assert_eq!(
        ast_error_primary_span(plus_diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, plus_span))
    );

    let star_source = "value = 5 + (* 2)\n";
    let star = parse_single_file_ast_diagnostic(star_source);
    let star_diagnostic = assert_ast_diagnostic(&star, "MOTH-SYNTAX-0024", None);
    let star_facts = star_diagnostic
        .family_view::<MissingOperatorOperandView>()
        .expect("missing-operand diagnostic should retain typed position");
    assert_eq!(star_facts.position, OperatorOperandPosition::BinaryLeft);
    let mut span_builder = ExtendedSpanBuilder::new();
    let star_span = LocalSpan::exact(
        star_source
            .find("(* ")
            .expect("dangling operator should be present") as u32
            + 1,
        1,
        &mut span_builder,
    )
    .expect("the dangling operator span should fit inline");
    assert_eq!(
        ast_error_primary_span(star_diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, star_span))
    );
}
#[test]
fn completed_evaluator_results_contain_no_pending_syntax() {
    let source = concat!(
        "peer ~Dec4 = 1\n",
        "received Dec4 = ((0.1 + 0.2))\n",
        "right = peer + ((0.1 + 0.2) * 2)\n",
        "left = ((0.1 + 0.2) * 2) + peer\n",
        "power Dec4 = peer ^ ((1 + 2))\n",
        "nested = peer + (peer + 1) * 2\n",
    );
    for index in 0..=5 {
        let expression = nth_start_declaration_expression(source, index);
        let visited = assert_completed_expression_tree(&expression);
        if index == 5 {
            // Both `peer` leaves keep `nested` runtime, so the walker must visit the
            // outer operands rather than only folded constants.
            assert!(
                visited >= 3,
                "the walker must visit outer runtime operands, visited {visited}"
            );
        }
    }
}

#[test]
fn bounded_flat_and_nested_dec_arithmetic_have_identical_exact_results() {
    const TERM_COUNT: usize = 128;
    let flat = vec!["0.1"; TERM_COUNT].join(" + ");
    // Group terms pairwise so parser nesting stays shallow while the expression length grows;
    // ordering and typing own the long-expression invariant.
    let mut groups = vec![String::from("0.1"); TERM_COUNT];
    while groups.len() > 1 {
        let mut next_groups = Vec::with_capacity(groups.len().div_ceil(2));
        let mut pairs = groups.into_iter();
        while let Some(left) = pairs.next() {
            next_groups.push(match pairs.next() {
                Some(right) => format!("({left} + {right})"),
                None => left,
            });
        }
        groups = next_groups;
    }
    let nested = &groups[0];
    let source = format!("flat Dec4 = {flat}\nnested Dec4 = {nested}\n");
    let flat = first_start_declaration_expression(&source);
    let nested = nth_start_declaration_expression(&source, 1);

    assert_number_literal(&flat, flat.value_type_id(), "128000", 4);
    assert_number_literal(&nested, flat.value_type_id(), "128000", 4);
    let (
        Some(ConstValuePayload::Number(flat_value)),
        Some(ConstValuePayload::Number(nested_value)),
    ) = (flat.payload(), nested.payload())
    else {
        panic!("both bounded expressions should fold to exact Dec values");
    };
    assert_eq!(flat_value, nested_value);
}

#[test]
fn number_power_keeps_pending_and_typed_exponents_in_i32() {
    let source = concat!(
        "base ~Dec2 = 1.5\n",
        "plain = base ^ 2\n",
        "negative = base ^ -2\n",
        "parenthesized = base ^ (2)\n",
        "negative_grouped = base ^ -(2)\n",
        "typed_result Dec2 = base ^ 2\n",
    );
    let base = first_start_declaration_expression(source);
    for index in 1..=5 {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(expression.value_type_id(), base.value_type_id());
        assert_number_scale(&expression, 2);
        assert_runtime_number_power_keeps_i32_exponent(&expression);
    }

    let parameter_source = concat!(
        "power |base Dec2, exponent I32| -> Dec2:\n",
        "    return base ^ exponent\n",
        ";\n",
    );
    let (ast, path_fork, string_table) = parse_single_file_ast(parameter_source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "power");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("the typed-parameter function should return the power expression");
    };
    let expression = ast
        .expressions
        .view(values[0])
        .expect("the returned power expression should be stored");
    assert_eq!(
        ast.type_environment.number_scale(
            expression
                .value_type_id()
                .expect("returned power should have one value type")
        ),
        Some(NumberScale::new(2).expect("test Dec scale is valid"))
    );
    let ExpressionOperationView::NumericBinary {
        right: exponent,
        facts,
        ..
    } = expression.operation()
    else {
        panic!("a typed I32 parameter keeps the power expression runtime-dependent");
    };
    assert_eq!(facts.operator, Operator::Exponent);
    assert_eq!(
        exponent
            .value_type_id()
            .expect("typed exponent should have one value type"),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

fn assert_binary_result_type(
    left_type: &str,
    operator: &str,
    right_type: &str,
    expected_type: TypeId,
) {
    let expression = binary_expression(left_type, operator, right_type);
    assert_eq!(
        expression.value_type_id(),
        expected_type,
        "{left_type} {operator} {right_type} resolved to a different type"
    );
}

fn assert_binary_rejected(
    left_type: &str,
    operator: &str,
    right_type: &str,
    diagnostic_operator: DiagnosticOperator,
) {
    let source =
        format!("left {left_type} = 1\nright {right_type} = 2\nvalue = left {operator} right\n");
    assert_unsupported_operator(&source, diagnostic_operator);
}

fn binary_expression(left_type: &str, operator: &str, right_type: &str) -> ParsedExpression {
    let source =
        format!("left {left_type} = 1\nright {right_type} = 2\nvalue = left {operator} right\n");
    nth_start_declaration_expression(&source, 2)
}

fn assert_runtime_contains_fixed_literal(
    expression: &ParsedExpression,
    expected: FixedScalarValue,
) {
    assert!(
        expression.payload().is_none(),
        "expected a runtime expression, got folded {:?}",
        expression.payload()
    );
    assert!(
        constant_leaves(&expression.ast.const_values, expression.view())
            .iter()
            .any(|(_, payload)| matches!(
                payload,
                ConstValuePayload::FixedScalar(value) if *value == expected
            )),
        "expected runtime expression to contain {expected:?}"
    );
}

fn assert_runtime_contains_number_literal(
    expression: &ParsedExpression,
    expected_type_id: TypeId,
    expected_coefficient: &str,
    expected_scale: u16,
) {
    assert!(
        expression.payload().is_none(),
        "expected a runtime expression, got folded {:?}",
        expression.payload()
    );
    assert!(
        constant_leaves(&expression.ast.const_values, expression.view())
            .iter()
            .any(|(type_id, payload)| {
                *type_id == expected_type_id
                    && matches!(
                        payload,
                        ConstValuePayload::Number(value)
                            if value.coefficient().to_string() == expected_coefficient
                                && value.scale().get() == expected_scale
                    )
            }),
        "expected runtime expression to contain Number({expected_coefficient}, scale {expected_scale})"
    );
}

fn assert_runtime_number_power_keeps_i32_exponent(expression: &ParsedExpression) {
    assert!(
        expression.payload().is_none(),
        "expected a runtime power expression, got folded {:?}",
        expression.payload()
    );
    let leaves = constant_leaves(&expression.ast.const_values, expression.view());
    assert!(
        leaves.iter().any(|(type_id, payload)| {
            *type_id == builtin_type_ids::fixed_scalar(FixedScalar::I32)
                && matches!(payload, ConstValuePayload::FixedScalar(_))
        }),
        "expected an I32 exponent operand"
    );
    assert!(
        !leaves
            .iter()
            .any(|(_, payload)| matches!(payload, ConstValuePayload::Number(_))),
        "pending power exponent must not be materialised as Number"
    );
}

fn assert_number_literal(
    expression: &ParsedExpression,
    expected_type_id: TypeId,
    expected_coefficient: &str,
    expected_scale: u16,
) {
    assert_eq!(expression.value_type_id(), expected_type_id);
    let Some(ConstValuePayload::Number(value)) = expression.payload() else {
        panic!(
            "expected an exact Dec value, got {:?}",
            expression.payload()
        );
    };
    assert_eq!(value.coefficient().to_string(), expected_coefficient);
    assert_eq!(value.scale().get(), expected_scale);
    assert_number_scale(expression, expected_scale);
}

fn assert_number_view(
    const_values: &ConstValueStore,
    view: ExpressionView<'_>,
    expected_type_id: TypeId,
    expected_coefficient: &str,
    expected_scale: u16,
) {
    assert_eq!(
        view.value_type_id()
            .expect("folded operand should have one value type"),
        expected_type_id
    );
    let Some(ConstValuePayload::Number(value)) = constant_payload(const_values, view) else {
        panic!("expected a folded exact Dec operand");
    };
    assert_eq!(value.coefficient().to_string(), expected_coefficient);
    assert_eq!(value.scale().get(), expected_scale);
}

fn number_scale_of(expression: &ParsedExpression) -> Option<NumberScale> {
    expression
        .ast
        .type_environment
        .number_scale(expression.value_type_id())
}

fn assert_number_scale(expression: &ParsedExpression, expected_scale: u16) {
    assert_eq!(
        number_scale_of(expression).map(|scale| scale.get()),
        Some(expected_scale),
        "expected a Dec{expected_scale} result"
    );
}

/// Folded leaves reachable from `root`, paired with each leaf's value type.
fn constant_leaves<'a>(
    const_values: &'a ConstValueStore,
    root: ExpressionView<'a>,
) -> Vec<(TypeId, &'a ConstValuePayload)> {
    subtree_views(root)
        .into_iter()
        .filter_map(|view| {
            let payload = constant_payload(const_values, view)?;
            let type_id = view
                .value_type_id()
                .expect("folded leaf should have one value type");
            Some((type_id, payload))
        })
        .collect()
}

/// The value wrapped by an option coercion (`maybe T? = value`).
fn coerced_inner(expression: &ParsedExpression) -> ExpressionView<'_> {
    match expression.view().operation() {
        ExpressionOperationView::Coerce { value, .. } => value,
        _ => panic!("expected the expression to be an option coercion"),
    }
}

/// Walks the completed expression graph and returns the number of nodes visited. Every node
/// must report one value type. Pending parse syntax has no `ExpressionOperationView` form, so
/// the completed-only guarantee is enforced by the operation type rather than by this walk.
fn assert_completed_expression_tree(expression: &ParsedExpression) -> usize {
    let views = subtree_views(expression.view());
    for view in &views {
        view.value_type_id()
            .expect("completed expression node should have one value type");
    }
    views.len()
}
