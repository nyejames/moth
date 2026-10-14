//! Regression tests for constant-expression folding helpers.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::eval_expression::result_type::resolve_expression_result_type;
use crate::compiler_frontend::ast::expressions::eval_expression::{
    ExpressionTypingError, expression_from_resolved_items,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, ResolvedOperationKind,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    CastFacts, ExprId, ExpressionOperationView, LogicalOperator, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ConstRecordState, FallibleExpressionHandling, FallibleHandling,
    ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::value_production::ProducedValues;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

#[test]
fn folding_handled_expressions_preserves_zero_and_multiple_success_shapes() {
    for slots in [
        vec![],
        vec![builtin_type_ids::BOOL, builtin_type_ids::STRING],
    ] {
        let mut f = FoldFixture::new();
        let producer = f
            .builder
            .function_call_with_typed_arguments(PathId::ROOT, vec![], slots.clone(), None)
            .expect("test call should construct")
            .id();
        let handled = f
            .builder
            .handled_result(
                f.builder.own(producer),
                FallibleExpressionHandling::Propagate,
                None,
            )
            .expect("test handled result should construct")
            .id();
        let folded = fold_compile_time_expression(
            handled,
            &mut f.builder,
            &mut f.constants,
            &mut f.strings,
            false,
        )
        .expect("ordinary handling folds without scalarizing its result");

        let view = f
            .builder
            .view(folded)
            .expect("folded handling should be stored");
        let success = view.success();
        if slots.is_empty() {
            assert_eq!(success, SuccessShape::Zero);
        } else {
            assert!(matches!(success, SuccessShape::Multiple(_)));
        }
        let view = f
            .builder
            .view(folded)
            .expect("folded handling should be stored");
        assert!(matches!(
            view.operation(),
            ExpressionOperationView::HandleFallible { .. }
        ));
        let view = f
            .builder
            .view(folded)
            .expect("folded handling should be stored");
        assert_eq!(view.error_type(), None);
    }
}

fn assert_compile_time_error(
    error: &ConstantFoldError,
    expected_reason: CompileTimeEvaluationErrorReason,
    expected_operation: Option<&str>,
    string_table: &StringTable,
) {
    let diagnostic = match error {
        ConstantFoldError::Diagnostic(diagnostic) => diagnostic,
        ConstantFoldError::Infrastructure(error) => {
            panic!("expected compile-time diagnostic, found infrastructure error: {error:?}")
        }
    };

    match &diagnostic.payload {
        DiagnosticPayload::CompileTimeEvaluationError {
            reason, operation, ..
        } => {
            assert_eq!(*reason, expected_reason);

            let operation_text = operation.map(|operation| string_table.resolve(operation));
            assert_eq!(operation_text, expected_operation);
        }
        payload => panic!("expected compile-time evaluation payload, found {payload:?}"),
    }
}

fn assert_invalid_cast_error(error: &ConstantFoldError, expected_reason: InvalidCastReason) {
    let diagnostic = match error {
        ConstantFoldError::Diagnostic(diagnostic) => diagnostic,
        ConstantFoldError::Infrastructure(error) => {
            panic!("expected invalid-cast diagnostic, found infrastructure error: {error:?}")
        }
    };

    match &diagnostic.payload {
        DiagnosticPayload::InvalidCast { reason, .. } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid-cast payload, found {payload:?}"),
    }
}

/// Builds typed expression fixtures for fold tests. Fields are accessed directly inside the
/// methods so the builder, constant store, and string table can be borrowed disjointly.
struct FoldFixture {
    builder: AstExpressionBuilder,
    constants: ConstValueStore,
    strings: StringTable,
    types: TypeEnvironment,
    path_fork: PathInternerFork,
}

impl FoldFixture {
    fn new() -> Self {
        Self {
            builder: AstExpressionBuilder::new().unwrap(),
            constants: ConstValueStore::default(),
            strings: StringTable::new(),
            types: TypeEnvironment::new(),
            path_fork: PathInternerFork::empty(),
        }
    }

    fn fixed_integer(&mut self, scalar: FixedScalar, value: i128) -> ExprId {
        self.fixed_integer_at(scalar, value, None)
    }

    fn fixed_integer_at(
        &mut self,
        scalar: FixedScalar,
        value: i128,
        span: Option<SourceSpan>,
    ) -> ExprId {
        let value = match scalar.class() {
            FixedScalarClass::SignedInteger => FixedScalarValue::signed(
                scalar,
                i64::try_from(value).expect("test signed fixed integer fits its carrier"),
            )
            .expect("test signed fixed integer fits its domain"),
            FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(
                scalar,
                u64::try_from(value).expect("test unsigned fixed integer fits its carrier"),
            )
            .expect("test unsigned fixed integer fits its domain"),
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => {
                panic!("fixed integer test helper requires an integer scalar")
            }
        };
        self.fixed_scalar_expr(value, span, ValueMode::ImmutableOwned)
    }

    fn fixed_float(&mut self, scalar: FixedScalar, value: f64) -> ExprId {
        self.fixed_scalar_expr(
            fixed_float_value(scalar, value),
            None,
            ValueMode::ImmutableOwned,
        )
    }

    fn fixed_scalar_expr(
        &mut self,
        value: FixedScalarValue,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> ExprId {
        self.builder
            .fixed_scalar(&mut self.constants, value, span, mode)
            .expect("test fixed scalar should construct")
            .id()
    }

    fn number(&mut self, normalized: &str, sign: NumericLiteralSign, scale: u16) -> ExprId {
        let scale = NumberScale::new(scale).expect("test Dec scale is within the supported range");
        let value = NumberValue::from_normalized(normalized, sign, scale)
            .expect("test Dec value is exact at its scale");
        let type_id = self.types.intern_number(scale);
        self.builder
            .number(
                &mut self.constants,
                value,
                type_id,
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("test Dec should construct")
            .id()
    }

    fn bool_literal(&mut self, value: bool, span: Option<SourceSpan>) -> ExprId {
        self.builder
            .bool(&mut self.constants, value, span, ValueMode::ImmutableOwned)
            .expect("test bool should construct")
            .id()
    }

    fn string_literal(&mut self, text: &str) -> ExprId {
        let text = self.strings.intern(text);
        self.builder
            .string_slice(&mut self.constants, text, None, ValueMode::ImmutableOwned)
            .expect("test string should construct")
            .id()
    }

    fn reference(
        &mut self,
        name: &str,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> ExprId {
        let path = self
            .path_fork
            .try_intern_portable_path(name, &mut self.strings)
            .expect("test path fits");
        self.reference_path(path, type_id, span, mode)
    }

    fn reference_path(
        &mut self,
        path: PathId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        mode: ValueMode,
    ) -> ExprId {
        self.builder
            .reference_with_type_id(path, type_id, span, mode, ConstRecordState::RuntimeValue)
            .expect("test reference should construct")
            .id()
    }

    /// A structural string whose pieces are only known at the call site, such as a site-root
    /// placeholder. Its final text is unavailable to constant folding.
    fn structural_string(&mut self, pieces: Vec<ConstStringPiece>) -> ExprId {
        let value = self
            .constants
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
                None,
            )
            .expect("test structural string should store");
        self.builder
            .constant(
                value,
                builtin_type_ids::STRING,
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("test structural string should construct")
            .id()
    }

    /// Resolves operator result types in place, as production expression parsing does.
    fn resolve_operator_types(&mut self, items: &mut [ExpressionRpnItem]) {
        resolve_expression_result_type(
            items,
            None,
            None,
            &mut self.builder,
            &mut self.constants,
            &mut self.strings,
            &self.types,
            &self.path_fork,
        )
        .unwrap_or_else(|_| panic!("test operator sequence should resolve"));
    }

    /// Resolves an RPN sequence into a typed root. Constant operands fold as the tree is built,
    /// matching production construction.
    fn resolve_rpn(&mut self, mut items: Vec<ExpressionRpnItem>) -> ExprId {
        self.resolve_operator_types(&mut items);
        expression_from_resolved_items(
            items,
            &mut self.builder,
            &mut self.constants,
            &mut self.strings,
            None,
            ValueMode::ImmutableOwned,
        )
        .unwrap_or_else(|_| panic!("test operator sequence should construct"))
        .id()
    }

    /// Resolves an RPN sequence into an unfolded typed root, so `fold` owns constant folding.
    /// Operator-level fixtures use this path to exercise `constant_fold` directly.
    fn resolve_rpn_unfolded(&mut self, mut items: Vec<ExpressionRpnItem>) -> ExprId {
        self.resolve_operator_types(&mut items);
        let mut stack = Vec::with_capacity(items.len());
        for item in items {
            match item {
                ExpressionRpnItem::Operand(id) => stack.push(self.builder.own(id)),
                ExpressionRpnItem::ResolvedOperator(facts) => {
                    let right = if facts.operator.required_values() == 2 {
                        Some(
                            stack
                                .pop()
                                .expect("test operator sequence should have a right operand"),
                        )
                    } else {
                        None
                    };
                    let left = stack
                        .pop()
                        .expect("test operator sequence should have a left operand");
                    let node = self
                        .builder
                        .resolved_operation(left, right, facts, ValueMode::ImmutableOwned)
                        .unwrap_or_else(|_| panic!("test operator sequence should construct"));
                    stack.push(node);
                }
                _ => panic!("test operator sequence should be resolved"),
            }
        }
        let [root] = stack.as_slice() else {
            panic!("test operator sequence should produce one root");
        };
        root.id()
    }

    fn binary(
        &mut self,
        lhs: ExprId,
        rhs: ExprId,
        operator: Operator,
        span: Option<SourceSpan>,
    ) -> ExprId {
        self.resolve_rpn_unfolded(vec![
            ExpressionRpnItem::Operand(lhs),
            ExpressionRpnItem::Operand(rhs),
            ExpressionRpnItem::Operator { operator, span },
        ])
    }

    fn unary(&mut self, operand: ExprId, operator: Operator, span: Option<SourceSpan>) -> ExprId {
        self.resolve_rpn_unfolded(vec![
            ExpressionRpnItem::Operand(operand),
            ExpressionRpnItem::Operator { operator, span },
        ])
    }

    fn fold(&mut self, root: ExprId) -> Result<ConstantFoldOutcome, ConstantFoldError> {
        constant_fold(
            root,
            &mut self.builder,
            &mut self.constants,
            &mut self.strings,
            None,
        )
    }

    fn fold_binary(
        &mut self,
        operator: Operator,
        span: Option<SourceSpan>,
        lhs: ExprId,
        rhs: ExprId,
    ) -> Result<ConstantFoldOutcome, ConstantFoldError> {
        let root = self.binary(lhs, rhs, operator, span);
        self.fold(root)
    }

    fn folded_binary(
        &mut self,
        operator: Operator,
        span: Option<SourceSpan>,
        lhs: ExprId,
        rhs: ExprId,
    ) -> ExprId {
        let outcome = self
            .fold_binary(operator, span, lhs, rhs)
            .expect("operator evaluation should succeed");
        expect_folded(outcome)
    }

    fn constant_payload(&self, id: ExprId) -> &ConstValuePayload {
        let view = self
            .builder
            .view(id)
            .expect("test expression should be stored");
        let ExpressionOperationView::Constant(constant) = view.operation() else {
            panic!("expected a folded constant expression");
        };
        self.constants
            .payload(constant)
            .expect("folded constant payload should be stored")
    }

    fn fixed_scalar_value(&self, id: ExprId) -> FixedScalarValue {
        match self.constant_payload(id) {
            ConstValuePayload::FixedScalar(value) => *value,
            _ => panic!("expected a folded fixed scalar"),
        }
    }

    fn folded_boolean(&self, id: ExprId) -> bool {
        match self.constant_payload(id) {
            ConstValuePayload::Bool(value) => *value,
            _ => panic!("expected a folded boolean"),
        }
    }

    fn type_of(&self, id: ExprId) -> TypeId {
        self.builder
            .view(id)
            .expect("test expression should be stored")
            .value_type_id()
            .expect("test expression should have a value type")
    }

    fn contains_regular_division(&self, id: ExprId) -> bool {
        self.builder
            .view(id)
            .expect("test expression should be stored")
            .classification()
            .contains_regular_division
    }

    fn cast_expr(
        &mut self,
        source: ExprId,
        evidence: ResolvedCastEvidence,
        target_type_id: TypeId,
        fallibility: BuiltinCastFallibility,
        handling: CastHandling,
        requires_optional_wrap_after_cast: bool,
    ) -> ExprId {
        let source_type_id = self.type_of(source);
        let result_type_id = if requires_optional_wrap_after_cast {
            self.types.intern_option(target_type_id)
        } else {
            target_type_id
        };
        self.builder
            .cast(
                self.builder.own(source),
                CastFacts {
                    source_type_id,
                    target_type_id,
                    requires_optional_wrap_after_cast,
                    fallibility,
                    evidence,
                    handling,
                    span: None,
                },
                result_type_id,
            )
            .expect("test cast should construct")
            .id()
    }

    fn builtin_cast(
        &mut self,
        source: ExprId,
        policy: BuiltinCastPolicyId,
        target_type_id: TypeId,
    ) -> ExprId {
        self.cast_expr(
            source,
            ResolvedCastEvidence::Builtin { policy },
            target_type_id,
            BuiltinCastFallibility::Fallible,
            CastHandling::Implicit,
            false,
        )
    }

    fn fold_compile_time(&mut self, root: ExprId) -> Result<ExprId, ConstantFoldError> {
        fold_compile_time_expression(
            root,
            &mut self.builder,
            &mut self.constants,
            &mut self.strings,
            true,
        )
    }
}

fn expect_folded(outcome: ConstantFoldOutcome) -> ExprId {
    match outcome {
        ConstantFoldOutcome::Folded(id) => id,
        ConstantFoldOutcome::NotConstant(_) => {
            panic!("expected a folded constant, found a runtime expression")
        }
        ConstantFoldOutcome::TextUnavailable { .. } => {
            panic!("expected a folded constant, found unavailable text")
        }
    }
}

fn expect_not_constant(outcome: ConstantFoldOutcome) -> ExprId {
    match outcome {
        ConstantFoldOutcome::NotConstant(root) => root,
        ConstantFoldOutcome::Folded(_) => {
            panic!("expected a runtime expression, found a folded constant")
        }
        ConstantFoldOutcome::TextUnavailable { .. } => {
            panic!("expected a runtime expression, found unavailable text")
        }
    }
}

fn fixed_float_value(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    let precision = match scalar {
        FixedScalar::F16 => BinaryFloatPrecision::Binary16,
        FixedScalar::F32 => BinaryFloatPrecision::Binary32,
        FixedScalar::F64 => BinaryFloatPrecision::Binary64,
        _ => panic!("fixed float test helper requires a binary-float scalar"),
    };
    FixedScalarValue::binary_float(scalar, precision.round(value))
        .expect("test float rounds to a finite fixed scalar")
}

/// Build a source span that is distinguishable from every other one in a test.
fn marked_span(start: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::from_index(7),
        LocalSpan::exact(start, 4, &mut span_builder).expect("provenance span should fit"),
    )
}

#[test]
fn resolved_mixed_integer_operation_retains_canonical_domain_and_discharge() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I8, -1);
    let rhs = f.fixed_integer(FixedScalar::U8, 255);
    let root = f.binary(lhs, rhs, Operator::Add, None);

    let lhs_type = f.type_of(lhs);
    let rhs_type = f.type_of(rhs);
    let ExpressionOperationView::NumericBinary { facts, .. } = f
        .builder
        .view(root)
        .expect("resolved operation should be stored")
        .operation()
    else {
        panic!("mixed addition must retain numeric facts");
    };
    assert_eq!(
        facts.result_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I16)
    );
    assert_eq!(facts.left_type, lhs_type);
    assert_eq!(facts.right_type, Some(rhs_type));
    let ResolvedOperationKind::Numeric(numeric) = &facts.kind else {
        panic!("mixed addition must retain numeric facts");
    };
    let domain = NumericScalar::Fixed(FixedScalar::I16);
    assert_eq!(numeric.domain, domain);
    assert_eq!(numeric.left_conversion, Some(domain));
    assert_eq!(numeric.right_conversion, Some(domain));
    assert!(numeric.discharged);
    assert!(facts.failure.is_none());
    let result_type = facts.result_type;

    let folded = expect_folded(f.fold(root).expect("operator evaluation should succeed"));
    assert_eq!(f.type_of(folded), result_type);
    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(254));
}

#[test]
fn resolved_extreme_integer_comparison_retains_exact_unconverted_operands() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I64, i128::from(i64::MAX));
    let rhs = f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX));
    let root = f.binary(lhs, rhs, Operator::LessThan, None);

    let ExpressionOperationView::Compare { facts, .. } = f
        .builder
        .view(root)
        .expect("resolved comparison should be stored")
        .operation()
    else {
        panic!("comparison must retain its selected domain");
    };
    assert_eq!(facts.result_type, builtin_type_ids::BOOL);
    let ResolvedOperationKind::Comparison(comparison) = &facts.kind else {
        panic!("comparison must retain its selected domain");
    };
    assert_eq!(comparison.domain, NumericComparisonDomain::ExactInteger);
    assert_eq!(
        comparison.left_domain,
        NumericScalar::Fixed(FixedScalar::I64)
    );
    assert_eq!(
        comparison.right_domain,
        NumericScalar::Fixed(FixedScalar::U64)
    );
    assert_eq!(comparison.left_conversion, None);
    assert_eq!(comparison.right_conversion, None);

    let folded = expect_folded(f.fold(root).expect("operator evaluation should succeed"));
    assert_eq!(f.type_of(folded), builtin_type_ids::BOOL);
    assert!(f.folded_boolean(folded));
}

#[test]
fn resolved_decimal_power_keeps_i32_exponent_and_canonical_result() {
    let mut f = FoldFixture::new();
    let scale = NumberScale::new(2).expect("valid fixture scale");
    let type_id = f.types.intern_number(scale);
    let lhs = f.number("1.07", NumericLiteralSign::Positive, 2);
    let rhs = f.fixed_integer(FixedScalar::I32, 3);
    let root = f.binary(lhs, rhs, Operator::Exponent, None);

    let ExpressionOperationView::NumericBinary { facts, .. } = f
        .builder
        .view(root)
        .expect("resolved operation should be stored")
        .operation()
    else {
        panic!("Dec power must retain numeric facts");
    };
    assert_eq!(facts.result_type, type_id);
    assert_eq!(
        facts.right_type,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32))
    );
    let ResolvedOperationKind::Numeric(numeric) = &facts.kind else {
        panic!("Dec power must retain numeric facts");
    };
    assert_eq!(numeric.domain, NumericScalar::Number(scale));
    assert_eq!(
        numeric.right_domain,
        Some(NumericScalar::Fixed(FixedScalar::I32))
    );
    assert_eq!(numeric.right_conversion, None);
    assert!(!numeric.discharged);

    let folded = expect_folded(f.fold(root).expect("operator evaluation should succeed"));
    assert_eq!(f.type_of(folded), type_id);
    assert!(matches!(
        f.constant_payload(folded),
        ConstValuePayload::Number(value) if value.coefficient().to_string() == "123"
    ));
}

#[test]
fn numeric_unary_folding_preserves_resolved_runtime_references() {
    let mut f = FoldFixture::new();
    let decimal_type = f.types.intern_number(NumberScale::ZERO);
    for type_id in [
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        decimal_type,
    ] {
        let reference_span = marked_span(30);
        let operator_span = marked_span(40);
        let reference = f.reference_path(
            PathId::ROOT,
            type_id,
            Some(reference_span),
            ValueMode::ImmutableReference,
        );
        let root = f.unary(reference, Operator::Negate, Some(operator_span));

        let not_constant = expect_not_constant(
            f.fold(root)
                .expect("numeric reference negation should fold without failing"),
        );
        assert_eq!(not_constant, root);

        let ExpressionOperationView::NumericUnary { operand, facts } = f
            .builder
            .view(root)
            .expect("runtime negation should be stored")
            .operation()
        else {
            panic!("runtime reference and resolved negation must remain intact");
        };
        assert_eq!(operand.id(), reference);
        assert_eq!(facts.operator, Operator::Negate);
        assert_eq!(facts.result_type, type_id);
        assert_eq!(facts.left_type, type_id);
        assert_eq!(facts.span, Some(operator_span));

        let operand_view = f
            .builder
            .view(reference)
            .expect("runtime reference should be stored");
        assert!(matches!(
            operand_view.operation(),
            ExpressionOperationView::Reference(PathId::ROOT)
        ));
        assert_eq!(
            operand_view
                .value_type_id()
                .expect("reference should have a value type"),
            type_id
        );
        assert_eq!(operand_view.span(), Some(reference_span));
        assert!(matches!(
            operand_view.value_mode(),
            ValueMode::ImmutableReference
        ));
    }
}

#[test]
fn structural_string_requirement_has_stable_rule_identity() {
    let requirements = [
        (
            ConstStringRequirement::EqualityComparison,
            "string equality comparison",
        ),
        (ConstStringRequirement::CastOrParse, "string cast or parse"),
        (
            ConstStringRequirement::CompileTimeMapKey,
            "compile-time map key",
        ),
        (
            ConstStringRequirement::DuplicateKeyValidation,
            "duplicate map-key validation",
        ),
    ];

    for (requirement, operation_name) in requirements {
        let mut f = FoldFixture::new();
        let value = f.structural_string(vec![ConstStringPiece::SiteRoot]);
        let view = f
            .builder
            .view(value)
            .expect("structural string should be stored");
        let diagnostic = require_concrete_text(view, &f.constants, requirement, &mut f.strings)
            .expect_err("structural strings should require a final-text diagnostic");

        let identity = diagnostic.identity();
        assert_eq!(identity.code, "MOTH-RULE-0053");
        assert_eq!(
            identity.reason_key,
            Some("compile_time_evaluation_error.structural_string_requires_final_text")
        );
        match &diagnostic.payload {
            DiagnosticPayload::CompileTimeEvaluationError { operation, .. } => {
                assert_eq!(
                    operation.map(|id| f.strings.resolve(id)),
                    Some(operation_name)
                );
            }
            payload => panic!("expected compile-time evaluation payload, found {payload:?}"),
        }
    }
}

#[test]
fn all_text_structural_string_requirement_concatenates_in_order() {
    // Test-only construction stands in for item 3's first structural text-piece producer.
    let mut f = FoldFixture::new();
    let first = f.strings.intern("first/");
    let second = f.strings.intern("second");
    let value = f.structural_string(vec![
        ConstStringPiece::Text(first),
        ConstStringPiece::Text(second),
    ]);

    let view = f
        .builder
        .view(value)
        .expect("structural string should be stored");
    let text = require_concrete_text(
        view,
        &f.constants,
        ConstStringRequirement::EqualityComparison,
        &mut f.strings,
    )
    .expect("all-text structural values have known final text")
    .expect("string values should return text");

    assert_eq!(f.strings.resolve(text), "first/second");
}

#[test]
fn structural_string_equality_reports_text_unavailable_outcome() {
    let mut f = FoldFixture::new();
    let lhs = f.structural_string(vec![ConstStringPiece::SiteRoot]);
    let rhs = f.string_literal("plain");
    let root = f.binary(lhs, rhs, Operator::Equality, None);

    let outcome = f
        .fold(root)
        .expect("structural equality should be a typed fold refusal");

    let ConstantFoldOutcome::TextUnavailable { diagnostic, .. } = outcome else {
        panic!("expected structural equality to report unavailable text");
    };
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("compile_time_evaluation_error.structural_string_requires_final_text")
    );
    let render_context = DiagnosticRenderContext::new(&f.strings);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("string equality comparison")),
        "the refusal must name the operation that needed final text: {guidance:?}"
    );
}

#[test]
fn constant_fold_propagates_structural_string_text_unavailable_outcome() {
    let mut f = FoldFixture::new();
    let lhs = f.structural_string(vec![ConstStringPiece::SiteRoot]);
    let rhs = f.string_literal("plain");
    let root = f.resolve_rpn(vec![
        ExpressionRpnItem::Operand(lhs),
        ExpressionRpnItem::Operand(rhs),
        ExpressionRpnItem::Operator {
            operator: Operator::Equality,
            span: None,
        },
    ]);

    let outcome = f
        .fold(root)
        .expect("structural equality should return a typed fold outcome");
    let ConstantFoldOutcome::TextUnavailable { diagnostic, .. } = outcome else {
        panic!("expected constant folding to preserve text-unavailable outcome");
    };
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("compile_time_evaluation_error.structural_string_requires_final_text")
    );
    let render_context = DiagnosticRenderContext::new(&f.strings);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("string equality comparison")),
        "the operation name must survive the operator-to-stack hop: {guidance:?}"
    );
}

#[test]
fn text_unavailable_refusal_keeps_the_items_that_follow_it() {
    // Returning early on the refusal would drop `flag and` and silently miscompile
    // `(@/ == "plain") and flag` into a comparison alone.
    let mut f = FoldFixture::new();
    let structural = f.structural_string(vec![ConstStringPiece::SiteRoot]);
    let plain = f.string_literal("plain");
    let flag = f.reference(
        "flag",
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableReference,
    );
    let root = f.resolve_rpn(vec![
        ExpressionRpnItem::Operand(structural),
        ExpressionRpnItem::Operand(plain),
        ExpressionRpnItem::Operator {
            operator: Operator::Equality,
            span: None,
        },
        ExpressionRpnItem::Operand(flag),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            span: None,
        },
    ]);

    let ConstantFoldOutcome::TextUnavailable {
        root: preserved, ..
    } = f.fold(root).expect("text refusal is a typed fold outcome")
    else {
        panic!("structural equality should refuse with unavailable text");
    };
    assert_eq!(preserved, root);

    let ExpressionOperationView::LogicalBinary {
        left,
        right,
        operation,
    } = f
        .builder
        .view(root)
        .expect("refused root should remain stored")
        .operation()
    else {
        panic!("the logical root must survive the text refusal");
    };
    assert!(matches!(operation, LogicalOperator::And));
    assert!(matches!(
        left.operation(),
        ExpressionOperationView::Compare { .. }
    ));
    assert!(matches!(
        right.operation(),
        ExpressionOperationView::Reference(_)
    ));
}

#[test]
fn evaluate_operator_rejects_negative_integer_exponent() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, 2);
    let rhs = f.fixed_integer(FixedScalar::I32, -1);
    let error = f
        .fold_binary(Operator::Exponent, None, lhs, rhs)
        .expect_err("negative integer exponent should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_divides_ints_to_float() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, 5);
    let rhs = f.fixed_integer(FixedScalar::I32, 2);
    let folded = f.folded_binary(Operator::Divide, None, lhs, rhs);

    assert_eq!(f.fixed_scalar_value(folded).as_f64(), Some(2.5));
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        f.contains_regular_division(folded),
        "folded regular division should preserve provenance"
    );
}

#[test]
fn evaluate_operator_integer_division_truncates_toward_zero() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, -5);
    let rhs = f.fixed_integer(FixedScalar::I32, 2);
    let folded = f.folded_binary(Operator::IntDivide, None, lhs, rhs);

    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(-2));
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn evaluate_operator_rejects_divide_by_zero_for_both_division_operators() {
    for operator in [Operator::Divide, Operator::IntDivide] {
        let mut f = FoldFixture::new();
        let lhs = f.fixed_integer(FixedScalar::I32, 5);
        let rhs = f.fixed_integer(FixedScalar::I32, 0);
        let error = f
            .fold_binary(operator, None, lhs, rhs)
            .expect_err("division by zero should fail");
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::DivideByZero,
            None,
            &f.strings,
        );
    }
}

#[test]
fn evaluate_operator_rejects_integer_add_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, i128::from(i32::MAX));
    let rhs = f.fixed_integer(FixedScalar::I32, 1);
    let error = f
        .fold_binary(Operator::Add, None, lhs, rhs)
        .expect_err("integer addition overflow should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_folds_i64_add_beyond_i32_range() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I64, i128::from(i32::MAX));
    let rhs = f.fixed_integer(FixedScalar::I64, 1);
    let folded = f.folded_binary(Operator::Add, None, lhs, rhs);

    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(2_147_483_648));
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::I64)
    );
}

#[test]
fn evaluate_operator_folds_deep_constant_chain_on_small_stack() {
    // The fold walks an explicit work stack, so a 4096-deep left-leaning chain must not
    // need one Rust frame per operator. The thread stack is 1 MiB, the budget the audit names.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut f = FoldFixture::new();
            let one = f.fixed_integer(FixedScalar::I64, 1);
            let mut root = one;
            for _ in 0..4096 {
                root = f.binary(root, one, Operator::Add, None);
            }
            let folded = expect_folded(f.fold(root).expect("deep constant chain should fold"));

            assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(4097));
            assert_eq!(
                f.type_of(folded),
                builtin_type_ids::fixed_scalar(FixedScalar::I64)
            );
        })
        .expect("test thread should spawn")
        .join()
        .expect("deep constant chain fold should not overflow or panic");
}

#[test]
fn evaluate_operator_keeps_deep_runtime_leaf_chain_on_small_stack() {
    // A runtime leaf makes every ancestor unfoldable, so the walk leaves the root unchanged.
    // It must do so without one Rust frame per operator, on a 1 MiB thread.
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut f = FoldFixture::new();
            let i64_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
            let x = f.reference("x", i64_type, None, ValueMode::ImmutableReference);
            let one = f.fixed_integer(FixedScalar::I64, 1);
            let mut root = x;
            for _ in 0..4096 {
                root = f.binary(root, one, Operator::Add, None);
            }

            let outcome = f.fold(root).expect("runtime leaf chain should not fail");
            assert_eq!(expect_not_constant(outcome), root);
        })
        .expect("test thread should spawn")
        .join()
        .expect("deep runtime leaf chain fold should not overflow or panic");
}

#[test]
fn evaluate_operator_rejects_int64_add_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I64, i128::from(i64::MAX));
    let rhs = f.fixed_integer(FixedScalar::I64, 1);
    let error = f
        .fold_binary(Operator::Add, None, lhs, rhs)
        .expect_err("int64 addition overflow should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_rejects_integer_subtract_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, i128::from(i32::MIN));
    let rhs = f.fixed_integer(FixedScalar::I32, 1);
    let error = f
        .fold_binary(Operator::Subtract, None, lhs, rhs)
        .expect_err("integer subtraction overflow should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_rejects_integer_multiply_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, i128::from(i32::MAX));
    let rhs = f.fixed_integer(FixedScalar::I32, 2);
    let error = f
        .fold_binary(Operator::Multiply, None, lhs, rhs)
        .expect_err("integer multiplication overflow should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("*"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_rejects_integer_exponent_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, 2);
    let rhs = f.fixed_integer(FixedScalar::I32, 31);
    let error = f
        .fold_binary(Operator::Exponent, None, lhs, rhs)
        .expect_err("integer exponentiation overflow should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_folds_trivial_base_exponent_despite_oversized_exponent() {
    // Only I64 can materialise an exponent that does not narrow to u32. Trivial bases still
    // fold to their exact result; |base| >= 2 keeps reporting IntegerOverflow.
    let oversized = i64::from(u32::MAX) + 1;
    for (base, exponent, expected) in [
        (0, oversized, 0),
        (1, oversized, 1),
        (-1, oversized, 1),
        (-1, oversized + 1, -1),
    ] {
        let mut f = FoldFixture::new();
        let lhs = f.fixed_integer(FixedScalar::I64, i128::from(base));
        let rhs = f.fixed_integer(FixedScalar::I64, i128::from(exponent));
        let folded = f.folded_binary(Operator::Exponent, None, lhs, rhs);
        assert_eq!(
            f.fixed_scalar_value(folded).as_i64(),
            Some(expected),
            "{base}^{exponent} should fold to {expected}"
        );
    }

    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I64, 2);
    let rhs = f.fixed_integer(FixedScalar::I64, i128::from(oversized));
    let error = f
        .fold_binary(Operator::Exponent, None, lhs, rhs)
        .expect_err("a non-trivial base with an oversized exponent should overflow");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_folds_truncated_int_remainder_at_both_widths() {
    // Remainder keeps the dividend's sign, and `% -1` is zero even for the width's signed
    // minimum, whose paired quotient is the only overflowing case.
    for (scalar, minimum) in [
        (FixedScalar::I32, i128::from(i32::MIN)),
        (FixedScalar::I64, i128::from(i64::MIN)),
    ] {
        for (lhs, rhs, expected) in [(7, 3, 1), (-7, 3, -1), (7, -3, 1), (minimum, -1, 0)] {
            let mut f = FoldFixture::new();
            let lhs_id = f.fixed_integer(scalar, lhs);
            let rhs_id = f.fixed_integer(scalar, rhs);
            let folded = f.folded_binary(Operator::Modulus, None, lhs_id, rhs_id);
            assert_eq!(
                f.fixed_scalar_value(folded).as_i64().map(i128::from),
                Some(expected),
                "{lhs} % {rhs} should fold to {expected}"
            );
        }
    }
}

#[test]
fn evaluate_operator_rejects_integer_division_overflow() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, i128::from(i32::MIN));
    let rhs = f.fixed_integer(FixedScalar::I32, -1);
    let error = f
        .fold_binary(Operator::IntDivide, None, lhs, rhs)
        .expect_err("signed minimum integer division by -1 should overflow");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("//"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_folds_signed_minimum_remainder_by_negative_one_to_zero() {
    // The quotient overflows, but the remainder is zero and must not inherit that check.
    for (scalar, minimum) in [
        (FixedScalar::I32, i128::from(i32::MIN)),
        (FixedScalar::I64, i128::from(i64::MIN)),
    ] {
        let mut f = FoldFixture::new();
        let lhs = f.fixed_integer(scalar, minimum);
        let rhs = f.fixed_integer(scalar, -1);
        let folded = f.folded_binary(Operator::Modulus, None, lhs, rhs);
        assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(0));
    }
}

#[test]
fn evaluate_operator_rejects_non_finite_float_exponent_result() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_float(FixedScalar::F64, 1e308);
    let rhs = f.fixed_float(FixedScalar::F64, 2.0);
    let error = f
        .fold_binary(Operator::Exponent, None, lhs, rhs)
        .expect_err("a non-finite float power should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_rejects_non_finite_float_multiply_result() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_float(FixedScalar::F64, 1e308);
    let rhs = f.fixed_float(FixedScalar::F64, 1e308);
    let error = f
        .fold_binary(Operator::Multiply, None, lhs, rhs)
        .expect_err("a non-finite float product should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some("*"),
        &f.strings,
    );
}

#[test]
fn constant_fold_rejects_integer_unary_negation_overflow() {
    let mut f = FoldFixture::new();
    let operand = f.fixed_integer(FixedScalar::I32, i128::from(i32::MIN));
    let root = f.unary(operand, Operator::Negate, None);
    let error = f
        .fold(root)
        .expect_err("negating the signed minimum should overflow");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_rejects_float_modulo_by_zero() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_float(FixedScalar::F64, 1.0);
    let rhs = f.fixed_float(FixedScalar::F64, 0.0);
    let error = f
        .fold_binary(Operator::Modulus, None, lhs, rhs)
        .expect_err("float modulo by zero should fail");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &f.strings,
    );
}

#[test]
fn evaluate_operator_folds_fixed_integer_results_in_the_operand_domain() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "equal U8 addition keeps U8",
            f.fixed_integer(FixedScalar::U8, 100),
            f.fixed_integer(FixedScalar::U8, 100),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits U8"),
        ),
        (
            "explicit U32 addition retains the wide sum",
            f.fixed_integer(FixedScalar::U32, 255),
            f.fixed_integer(FixedScalar::U32, 255),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U32, 510).expect("510 fits U32"),
        ),
        (
            "mixed signedness selects I16",
            f.fixed_integer(FixedScalar::I8, -1),
            f.fixed_integer(FixedScalar::U8, 255),
            Operator::Add,
            FixedScalarValue::signed(FixedScalar::I16, 254).expect("254 fits I16"),
        ),
        (
            "U64 maximum survives a no-op addition",
            f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX)),
            f.fixed_integer(FixedScalar::U64, 0),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 maximum fits"),
        ),
        (
            "equal narrow integer division keeps I8",
            f.fixed_integer(FixedScalar::I8, -128),
            f.fixed_integer(FixedScalar::I8, 2),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I8, -64).expect("-64 fits I8"),
        ),
        (
            "explicit I32 integer division retains the former promoted quotient",
            f.fixed_integer(FixedScalar::I32, -128),
            f.fixed_integer(FixedScalar::I32, -1),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I32, 128).expect("128 fits I32"),
        ),
        (
            "signed minimum remainder by negative one is zero",
            f.fixed_integer(FixedScalar::I64, i128::from(i64::MIN)),
            f.fixed_integer(FixedScalar::I64, -1),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I64, 0).expect("zero fits I64"),
        ),
        (
            "negative integer division truncates toward zero",
            f.fixed_integer(FixedScalar::I32, -7),
            f.fixed_integer(FixedScalar::I32, 2),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I32, -3).expect("-3 fits I32"),
        ),
        (
            "remainder follows the dividend sign",
            f.fixed_integer(FixedScalar::I32, -7),
            f.fixed_integer(FixedScalar::I32, 2),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I32, -1).expect("-1 fits I32"),
        ),
        (
            "negative divisor keeps positive dividend remainder",
            f.fixed_integer(FixedScalar::I32, 7),
            f.fixed_integer(FixedScalar::I32, -2),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
        ),
        (
            "U8 exponentiation keeps U8",
            f.fixed_integer(FixedScalar::U8, 2),
            f.fixed_integer(FixedScalar::U8, 7),
            Operator::Exponent,
            FixedScalarValue::unsigned(FixedScalar::U8, 128).expect("2^7 fits U8"),
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        let folded = f.folded_binary(operator, None, lhs, rhs);
        assert_eq!(
            f.fixed_scalar_value(folded),
            expected,
            "{case} must fold in the operand-derived domain"
        );
    }

    // These exact inputs folded through a wider carrier before D15 (`U8 + U8` into U32,
    // `I8::MIN // -1` into I32). They must now fail at the operand width, so a carrier-width
    // overflow check cannot accept them. The wider 510/128 calculations are retained above
    // with explicit U32/I32 operands.
    let overflow_cases = [
        (
            "U8 255 + 255 overflows U8",
            f.fixed_integer(FixedScalar::U8, 255),
            f.fixed_integer(FixedScalar::U8, 255),
            Operator::Add,
            "+",
        ),
        (
            "I8 minimum divided by -1 overflows I8",
            f.fixed_integer(FixedScalar::I8, -128),
            f.fixed_integer(FixedScalar::I8, -1),
            Operator::IntDivide,
            "//",
        ),
    ];
    for (case, lhs, rhs, operator, operation) in overflow_cases {
        let error = f
            .fold_binary(operator, None, lhs, rhs)
            .expect_err(&format!("{case} should fail during folding"));
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
            Some(operation),
            &f.strings,
        );
    }
}

#[test]
fn evaluate_operator_folds_fixed_integer_division_to_f64() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "U8 division",
            f.fixed_integer(FixedScalar::U8, 7),
            f.fixed_integer(FixedScalar::U8, 2),
            fixed_float_value(FixedScalar::F64, 3.5),
        ),
        (
            "I64 maximum division rounds each operand to binary64",
            f.fixed_integer(FixedScalar::I64, i128::from(i64::MAX)),
            f.fixed_integer(FixedScalar::I64, 1),
            fixed_float_value(FixedScalar::F64, 9_223_372_036_854_775_808.0),
        ),
    ];

    for (case, lhs, rhs, expected) in cases {
        let folded = f.folded_binary(Operator::Divide, None, lhs, rhs);
        assert_eq!(f.fixed_scalar_value(folded), expected, "{case}");
    }
}

#[test]
fn evaluate_operator_handles_huge_fixed_integer_exponents_without_host_overflow() {
    let mut f = FoldFixture::new();
    let successes = [
        (
            "every integer raised to zero is one",
            f.fixed_integer(FixedScalar::U64, 2),
            f.fixed_integer(FixedScalar::U64, 0),
            FixedScalarValue::unsigned(FixedScalar::U64, 1).expect("1 fits U64"),
        ),
        (
            "one raised to a huge exponent stays one",
            f.fixed_integer(FixedScalar::U64, 1),
            f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX)),
            FixedScalarValue::unsigned(FixedScalar::U64, 1).expect("1 fits U64"),
        ),
    ];

    for (case, lhs, rhs, expected) in successes {
        let folded = f.folded_binary(Operator::Exponent, None, lhs, rhs);
        assert_eq!(f.fixed_scalar_value(folded), expected, "{case}");
    }

    let lhs = f.fixed_integer(FixedScalar::U64, 2);
    let rhs = f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX));
    let error = f
        .fold_binary(Operator::Exponent, None, lhs, rhs)
        .expect_err("a huge non-trivial power should overflow without a host-sized loop");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn evaluate_operator_reports_fixed_integer_overflow_and_invalid_exponents() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "U32 multiplication overflow",
            f.fixed_integer(FixedScalar::U32, i128::from(u32::MAX)),
            f.fixed_integer(FixedScalar::U32, 2),
            Operator::Multiply,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "I64 minimum quotient overflow",
            f.fixed_integer(FixedScalar::I64, i128::from(i64::MIN)),
            f.fixed_integer(FixedScalar::I64, -1),
            Operator::IntDivide,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "I32 minimum quotient overflows its own domain",
            f.fixed_integer(FixedScalar::I32, i128::from(i32::MIN)),
            f.fixed_integer(FixedScalar::I32, -1),
            Operator::IntDivide,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "U32 exponent overflow",
            f.fixed_integer(FixedScalar::U8, 2),
            f.fixed_integer(FixedScalar::U8, 32),
            Operator::Exponent,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "negative fixed integer exponent",
            f.fixed_integer(FixedScalar::I32, 2),
            f.fixed_integer(FixedScalar::I32, -1),
            Operator::Exponent,
            CompileTimeEvaluationErrorReason::InvalidExponent,
        ),
    ];

    for (case, lhs, rhs, operator, reason) in cases {
        let symbol = operator.to_str();
        let error = f
            .fold_binary(operator, None, lhs, rhs)
            .expect_err(&format!("{case} should fail during folding"));
        assert_compile_time_error(&error, reason, Some(symbol), &f.strings);
    }
}

#[test]
fn evaluate_operator_reports_fixed_integer_and_float_zero_divisors() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "fixed integer division",
            f.fixed_integer(FixedScalar::I32, 5),
            f.fixed_integer(FixedScalar::I32, 0),
            Operator::IntDivide,
        ),
        (
            "fixed integer remainder",
            f.fixed_integer(FixedScalar::U8, 5),
            f.fixed_integer(FixedScalar::U8, 0),
            Operator::Modulus,
        ),
        (
            "fixed integer regular division",
            f.fixed_integer(FixedScalar::U8, 5),
            f.fixed_integer(FixedScalar::U8, 0),
            Operator::Divide,
        ),
        (
            "fixed float regular division",
            f.fixed_float(FixedScalar::F32, 5.0),
            f.fixed_float(FixedScalar::F32, 0.0),
            Operator::Divide,
        ),
        (
            "fixed float remainder",
            f.fixed_float(FixedScalar::F16, 5.0),
            f.fixed_float(FixedScalar::F16, -0.0),
            Operator::Modulus,
        ),
    ];

    for (case, lhs, rhs, operator) in cases {
        let error = f
            .fold_binary(operator, None, lhs, rhs)
            .expect_err(&format!("{case} should fail during folding"));
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::DivideByZero,
            None,
            &f.strings,
        );
    }
}

#[test]
fn evaluate_operator_rounds_fixed_float_results_once_in_the_operand_domain() {
    let mut f = FoldFixture::new();
    let half = fixed_float_value(FixedScalar::F16, 0.1);
    let half_product = fixed_float_value(
        FixedScalar::F16,
        half.as_f64().expect("F16 has a float value")
            * half.as_f64().expect("F16 has a float value"),
    );
    let f32_value = fixed_float_value(FixedScalar::F32, 0.1);
    let f64_value = fixed_float_value(FixedScalar::F64, 0.2);
    let cases = [
        (
            "F16 product rounds once to F16",
            f.fixed_scalar_expr(half, None, ValueMode::ImmutableOwned),
            f.fixed_scalar_expr(half, None, ValueMode::ImmutableOwned),
            Operator::Multiply,
            half_product,
        ),
        (
            "F32 and F64 promote to F64",
            f.fixed_scalar_expr(f32_value, None, ValueMode::ImmutableOwned),
            f.fixed_scalar_expr(f64_value, None, ValueMode::ImmutableOwned),
            Operator::Add,
            fixed_float_value(
                FixedScalar::F64,
                f32_value.as_f64().expect("F32 has a float value")
                    + f64_value.as_f64().expect("F64 has a float value"),
            ),
        ),
        (
            "F16 and F16 keep F16",
            f.fixed_float(FixedScalar::F16, 0.5),
            f.fixed_float(FixedScalar::F16, 0.5),
            Operator::Add,
            fixed_float_value(FixedScalar::F16, 1.0),
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        let folded = f.folded_binary(operator, None, lhs, rhs);
        assert_eq!(
            f.fixed_scalar_value(folded),
            expected,
            "{case} must retain the semantic precision"
        );
    }

    // B20: F16 completes every operation before the next one starts. The ulp at 2048 is 2, so
    // `2048 + 1` is a binary16 midpoint whose ties-to-even result is 2048, and the following
    // subtraction then cancels the chain to zero. A carrier that postponed the F16 rounding
    // until after the chain would keep 2049 and end at 1 instead.
    let two_forty_eight = f.fixed_float(FixedScalar::F16, 2048.0);
    let one = f.fixed_float(FixedScalar::F16, 1.0);
    let intermediate = f.folded_binary(Operator::Add, None, two_forty_eight, one);
    let intermediate_value = f.fixed_scalar_value(intermediate);
    assert_eq!(intermediate_value.scalar(), FixedScalar::F16);
    assert_eq!(
        intermediate_value
            .as_f64()
            .expect("F16 has a float value")
            .to_bits(),
        2048.0f64.to_bits(),
        "F16 `2048 + 1` must round to 2048 before the next operation"
    );

    let chain_start = f.fixed_float(FixedScalar::F16, 2048.0);
    let chain_one = f.fixed_float(FixedScalar::F16, 1.0);
    let chain_subtrahend = f.fixed_float(FixedScalar::F16, 2048.0);
    let chain_root = f.resolve_rpn(vec![
        ExpressionRpnItem::Operand(chain_start),
        ExpressionRpnItem::Operand(chain_one),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: None,
        },
        ExpressionRpnItem::Operand(chain_subtrahend),
        ExpressionRpnItem::Operator {
            operator: Operator::Subtract,
            span: None,
        },
    ]);
    let chained = expect_folded(f.fold(chain_root).expect("chained F16 fold should succeed"));
    let chained_value = f.fixed_scalar_value(chained);
    assert_eq!(chained_value.scalar(), FixedScalar::F16);
    assert_eq!(
        chained_value
            .as_f64()
            .expect("F16 has a float value")
            .to_bits(),
        0.0f64.to_bits(),
        "chained F16 `(2048 + 1) - 2048` must cancel to zero"
    );
}

#[test]
fn evaluate_operator_rounds_fixed_f32_arithmetic_to_f32_bits() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_float(FixedScalar::F32, 0.1);
    let rhs = f.fixed_float(FixedScalar::F32, 0.2);
    let expected_bits = f64::from(0.1f32 + 0.2f32).to_bits();

    let folded = f.folded_binary(Operator::Add, None, lhs, rhs);
    let actual = f.fixed_scalar_value(folded);
    assert_eq!(actual.scalar(), FixedScalar::F32);
    assert_eq!(
        actual
            .as_f64()
            .expect("F32 result has a float value")
            .to_bits(),
        expected_bits,
        "F32 addition must match independently computed F32 rounding"
    );
}

#[test]
fn evaluate_operator_preserves_signed_zero_result_bits() {
    let mut f = FoldFixture::new();
    let lhs = f.fixed_float(FixedScalar::F32, -0.0);
    let rhs = f.fixed_float(FixedScalar::F32, 1.0);

    let folded = f.folded_binary(Operator::Multiply, None, lhs, rhs);
    let actual = f.fixed_scalar_value(folded);
    assert_eq!(actual.scalar(), FixedScalar::F32);
    assert_eq!(
        actual
            .as_f64()
            .expect("F32 result has a float value")
            .to_bits(),
        f64::from(-0.0f32).to_bits(),
        "negative zero must retain its sign bit through fixed-float folding"
    );
}

#[test]
fn evaluate_operator_rejects_fixed_float_overflow() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "F32 multiplication overflow",
            f.fixed_float(FixedScalar::F32, 3.0e38),
            f.fixed_float(FixedScalar::F32, 10.0),
            Operator::Multiply,
            "*",
        ),
        (
            "F16 maximum plus itself overflows finite F16",
            f.fixed_float(FixedScalar::F16, 65504.0),
            f.fixed_float(FixedScalar::F16, 65504.0),
            Operator::Add,
            "+",
        ),
    ];

    for (case, lhs, rhs, operator, operation) in cases {
        let error = f
            .fold_binary(operator, None, lhs, rhs)
            .expect_err(&format!("{case} should fail during folding"));
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::FloatOverflow,
            Some(operation),
            &f.strings,
        );
    }
}

#[test]
fn constant_fold_uses_fixed_integer_and_float_negation_domains() {
    let mut f = FoldFixture::new();
    let successes = [
        (
            "I8 negates in I8",
            f.fixed_integer(FixedScalar::I8, 127),
            FixedScalarValue::signed(FixedScalar::I8, -127).expect("-127 fits I8"),
        ),
        (
            "F16 negates in F16",
            f.fixed_float(FixedScalar::F16, 0.5),
            fixed_float_value(FixedScalar::F16, -0.5),
        ),
    ];

    for (case, operand, expected) in successes {
        let root = f.unary(operand, Operator::Negate, None);
        let folded = expect_folded(f.fold(root).expect("negation should fold"));
        assert_eq!(f.fixed_scalar_value(folded), expected, "{case}");
    }

    // Negating a signed minimum fails in the operand's own type, including the narrow I8.
    let failures = [
        (
            "negating I64 minimum should overflow I64",
            f.fixed_integer(FixedScalar::I64, i128::from(i64::MIN)),
        ),
        (
            "negating I8 minimum should overflow I8",
            f.fixed_integer(FixedScalar::I8, -128),
        ),
    ];
    for (case, operand) in failures {
        let root = f.unary(operand, Operator::Negate, None);
        let error = f.fold(root).expect_err(case);
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
            Some("-"),
            &f.strings,
        );
    }
}

#[test]
fn evaluate_operator_compares_fixed_scalars_without_losing_value_identity() {
    let mut f = FoldFixture::new();
    let cases = [
        (
            "negative signed value orders below U64 maximum",
            f.fixed_integer(FixedScalar::I64, -1),
            f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX)),
            Operator::LessThan,
            true,
        ),
        (
            "U64 maximum is not I64 maximum",
            f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX)),
            f.fixed_integer(FixedScalar::I64, i128::from(i64::MAX)),
            Operator::Equality,
            false,
        ),
        (
            "adjacent integers above f64 exact precision remain ordered",
            f.fixed_integer(FixedScalar::U64, 9_007_199_254_740_993),
            f.fixed_integer(FixedScalar::I64, 9_007_199_254_740_992),
            Operator::GreaterThan,
            true,
        ),
        (
            "positive and negative float zero compare equal",
            f.fixed_float(FixedScalar::F16, 0.0),
            f.fixed_float(FixedScalar::F64, -0.0),
            Operator::Equality,
            true,
        ),
        (
            "Byte ordering is unsigned",
            f.fixed_scalar_expr(
                FixedScalarValue::unsigned(FixedScalar::Byte, 200).expect("200 fits Byte"),
                None,
                ValueMode::ImmutableOwned,
            ),
            f.fixed_scalar_expr(
                FixedScalarValue::unsigned(FixedScalar::Byte, 100).expect("100 fits Byte"),
                None,
                ValueMode::ImmutableOwned,
            ),
            Operator::GreaterThan,
            true,
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        let folded = f.folded_binary(operator, None, lhs, rhs);
        assert_eq!(f.folded_boolean(folded), expected, "{case}");
    }
}

#[test]
fn evaluate_operator_folds_number_arithmetic_once_and_retains_canonical_type() {
    let mut f = FoldFixture::new();
    let scale = NumberScale::new(1).expect("test scale is valid");
    let number_type_id = f.types.intern_number(scale);

    let lhs = f.number("1.5", NumericLiteralSign::Positive, 1);
    let rhs = f.number("0.1", NumericLiteralSign::Positive, 1);
    let product = f.folded_binary(Operator::Multiply, None, lhs, rhs);
    assert_eq!(f.type_of(product), number_type_id);
    assert!(matches!(
        f.constant_payload(product),
        ConstValuePayload::Number(value) if value.coefficient().to_string() == "2"
    ));

    let whole = f.fixed_integer(FixedScalar::U64, 2);
    let fraction = f.number("0.5", NumericLiteralSign::Positive, 1);
    let mixed_sum = f.folded_binary(Operator::Add, None, whole, fraction);
    assert_eq!(f.type_of(mixed_sum), number_type_id);
    assert!(matches!(
        f.constant_payload(mixed_sum),
        ConstValuePayload::Number(value) if value.coefficient().to_string() == "25"
    ));

    let base = f.number("1.5", NumericLiteralSign::Positive, 1);
    let exponent = f.fixed_integer(FixedScalar::I32, 3);
    let power = f.folded_binary(Operator::Exponent, None, base, exponent);
    assert_eq!(f.type_of(power), number_type_id);
    assert!(matches!(
        f.constant_payload(power),
        ConstValuePayload::Number(value) if value.coefficient().to_string() == "34"
    ));

    let operand = f.number("1.2", NumericLiteralSign::Positive, 1);
    let root = f.unary(operand, Operator::Negate, None);
    let negated = expect_folded(f.fold(root).expect("Dec negation should fold"));
    assert_eq!(f.type_of(negated), number_type_id);
    assert!(matches!(
        f.constant_payload(negated),
        ConstValuePayload::Number(value) if value.coefficient().to_string() == "-12"
    ));
}

#[test]
fn number_folding_reports_checked_division_and_exponent_failures() {
    let mut f = FoldFixture::new();
    let lhs = f.number("1.0", NumericLiteralSign::Positive, 1);
    let zero = f.number("0.0", NumericLiteralSign::Positive, 1);
    let error = f
        .fold_binary(Operator::Divide, None, lhs, zero)
        .expect_err("a known Dec zero divisor must be a compile-time failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &f.strings,
    );

    let error = f
        .fold_binary(Operator::Modulus, None, lhs, zero)
        .expect_err("a known Dec remainder-by-zero must be a compile-time failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &f.strings,
    );

    let base = f.number("1.5", NumericLiteralSign::Positive, 1);
    let negative_exponent = f.fixed_integer(FixedScalar::I32, -1);
    let error = f
        .fold_binary(Operator::Exponent, None, base, negative_exponent)
        .expect_err("a negative Dec exponent must use the checked exponent failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some("^"),
        &f.strings,
    );
}

#[test]
fn mixed_number_integer_comparisons_preserve_values_beyond_float_precision() {
    let mut f = FoldFixture::new();

    let number = f.number("9007199254740992", NumericLiteralSign::Positive, 0);
    let integer = f.fixed_integer(FixedScalar::I64, 9_007_199_254_740_993);
    let folded = f.folded_binary(Operator::LessThan, None, number, integer);
    assert!(f.folded_boolean(folded));

    let number = f.number("9007199254740992", NumericLiteralSign::Positive, 0);
    let integer = f.fixed_integer(FixedScalar::I64, 9_007_199_254_740_993);
    let folded = f.folded_binary(Operator::GreaterThan, None, integer, number);
    assert!(f.folded_boolean(folded));

    let number = f.number("18446744073709551616", NumericLiteralSign::Positive, 0);
    let maximum_u64 = f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX));
    let folded = f.folded_binary(Operator::GreaterThan, None, number, maximum_u64);
    assert!(f.folded_boolean(folded));
}

#[test]
fn evaluate_operator_rounds_float_add_at_the_operand_domain_precision() {
    let mut f = FoldFixture::new();

    // F32 operands fold at F32 precision: the sum of the widened carriers rounds once.
    let f32_lhs = f.fixed_float(FixedScalar::F32, f64::from(0.1f32));
    let f32_rhs = f.fixed_float(FixedScalar::F32, f64::from(0.2f32));
    let folded = f.folded_binary(Operator::Add, None, f32_lhs, f32_rhs);
    assert_eq!(
        f.fixed_scalar_value(folded).as_f64(),
        Some(f64::from(0.1f32 + 0.2f32))
    );
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );

    // The same carrier values under the F64 domain keep the exact f64 sum.
    let f64_lhs = f.fixed_float(FixedScalar::F64, f64::from(0.1f32));
    let f64_rhs = f.fixed_float(FixedScalar::F64, f64::from(0.2f32));
    let folded = f.folded_binary(Operator::Add, None, f64_lhs, f64_rhs);
    assert_eq!(
        f.fixed_scalar_value(folded).as_f64(),
        Some(f64::from(0.1f32) + f64::from(0.2f32))
    );

    // Digits materialised at F64 fold to the plain f64 sum.
    let f64_lhs = f.fixed_float(FixedScalar::F64, 0.1_f64);
    let f64_rhs = f.fixed_float(FixedScalar::F64, 0.2_f64);
    let folded = f.folded_binary(Operator::Add, None, f64_lhs, f64_rhs);
    assert_eq!(f.fixed_scalar_value(folded).as_f64(), Some(0.1 + 0.2));
}

#[test]
fn evaluate_operator_folds_int_division_quotient_in_f64() {
    // Integer real division computes in F64: both operands convert exactly, and the
    // quotient rounds once at binary64 precision.
    let mut f = FoldFixture::new();
    let lhs = f.fixed_integer(FixedScalar::I32, 16_777_217);
    let rhs = f.fixed_integer(FixedScalar::I32, 10);
    let folded = f.folded_binary(Operator::Divide, None, lhs, rhs);
    let value = f.fixed_scalar_value(folded);
    assert_eq!(
        value.as_f64(),
        Some(1_677_721.7),
        "I32 / I32 must fold to the exact f64 quotient, got {value:?}"
    );
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn constant_fold_partially_folds_runtime_expression() {
    // `runtime_var * (2 + 3)`: the runtime root keeps its node, while the constant `2 + 3`
    // operand folds to `5` when its operator node is built.
    let mut f = FoldFixture::new();
    let runtime_var = f.reference(
        "runtime_var",
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );
    let two = f.fixed_integer(FixedScalar::I32, 2);
    let three = f.fixed_integer(FixedScalar::I32, 3);
    let root = f.resolve_rpn(vec![
        ExpressionRpnItem::Operand(runtime_var),
        ExpressionRpnItem::Operand(two),
        ExpressionRpnItem::Operand(three),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: None,
        },
        ExpressionRpnItem::Operator {
            operator: Operator::Multiply,
            span: None,
        },
    ]);

    let outcome = f
        .fold(root)
        .expect("runtime-rooted expression should not be a diagnostic");
    assert_eq!(
        expect_not_constant(outcome),
        root,
        "a runtime-rooted expression keeps its root node"
    );

    let view = f.builder.view(root).expect("runtime root should be stored");
    let ExpressionOperationView::NumericBinary { left, right, facts } = view.operation() else {
        panic!("the runtime multiply should stay a numeric binary node");
    };
    assert!(matches!(facts.operator, Operator::Multiply));
    assert!(matches!(
        left.operation(),
        ExpressionOperationView::Reference(_)
    ));
    assert!(
        matches!(right.operation(), ExpressionOperationView::Constant(_)),
        "the constant `2 + 3` operand should fold to a constant as the operator tree is built"
    );
    assert_eq!(f.fixed_scalar_value(right.id()).as_i64(), Some(5));
}

#[test]
fn constant_fold_reports_static_failure_at_authored_operator_inside_runtime_expression() {
    // `runtime_var + (1 / 0)`: the constant `1 / 0` fails as the operator tree is built, and the
    // diagnostic must point at the authored `/` rather than the enclosing runtime operator.
    let mut f = FoldFixture::new();
    let runtime_var = f.reference(
        "runtime_var",
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        None,
        ValueMode::ImmutableReference,
    );
    let one = f.fixed_integer_at(FixedScalar::I32, 1, Some(marked_span(100)));
    let zero = f.fixed_integer_at(FixedScalar::I32, 0, Some(marked_span(140)));
    let operator_span = marked_span(230);
    let mut items = vec![
        ExpressionRpnItem::Operand(runtime_var),
        ExpressionRpnItem::Operand(one),
        ExpressionRpnItem::Operand(zero),
        ExpressionRpnItem::Operator {
            operator: Operator::Divide,
            span: Some(operator_span),
        },
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: None,
        },
    ];
    f.resolve_operator_types(&mut items);

    let error = expression_from_resolved_items(
        items,
        &mut f.builder,
        &mut f.constants,
        &mut f.strings,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect_err("divide by zero inside a runtime expression should still be diagnosed");
    let ExpressionTypingError::Diagnostic(diagnostic) = error else {
        panic!("divide by zero should remain a source diagnostic");
    };
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::CompileTimeEvaluationError {
            reason: CompileTimeEvaluationErrorReason::DivideByZero,
            ..
        }
    ));
    assert_eq!(diagnostic.primary_span, Some(operator_span));
}

#[test]
fn fold_int_cast_rejects_out_of_range_float_with_dedicated_code() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, 9_223_372_036_854_775_808.0)
            .expect("2^63 is a finite F64"),
    );
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::I64),
        },
        &source,
    )
    .expect_err("out-of-range float to I64 cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn fold_int_cast_truncates_toward_zero() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };

    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::F64),
        target: NumericScalar::Fixed(FixedScalar::I32),
    };
    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, 1.9).expect("1.9 is a finite F64"),
    );
    let result = apply_builtin_cast_policy(policy, &source).expect("float to int cast should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32")
        )
    );

    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, -1.9).expect("-1.9 is a finite F64"),
    );
    let result =
        apply_builtin_cast_policy(policy, &source).expect("negative float to int cast should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I32, -1).expect("-1 fits I32")
        )
    );
}

#[test]
fn fold_float_cast_rejects_non_finite_string_value() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

    let huge = format!("{}.0", "9".repeat(400));
    let source = BuiltinCastLiteral::String(huge);
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::F64)),
        &source,
    )
    .expect_err("non-finite float string cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseOutOfRange);
}

#[test]
fn fold_string_to_int_cast_uses_string_policy_row() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("42");
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.builtin_cast(source, policy, target_type_id);

    let folded = f
        .fold_compile_time(cast)
        .expect("valid string to int cast should fold");

    assert_eq!(f.type_of(folded), target_type_id);
    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(42));
}

#[test]
fn fold_string_to_float_cast_uses_string_policy_row() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("3.5e2");
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::F64);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::F64));
    let cast = f.builtin_cast(source, policy, target_type_id);

    let folded = f
        .fold_compile_time(cast)
        .expect("valid string to float cast should fold");

    assert_eq!(f.type_of(folded), target_type_id);
    assert_eq!(f.fixed_scalar_value(folded).as_f64(), Some(350.0));
}

#[test]
fn constant_fold_folds_comparison_then_boolean_chain() {
    let mut f = FoldFixture::new();
    let one = f.fixed_integer(FixedScalar::I32, 1);
    let two = f.fixed_integer(FixedScalar::I32, 2);
    let comparison = f.binary(one, two, Operator::LessThan, None);
    let flag = f.bool_literal(true, None);
    let root = f.binary(comparison, flag, Operator::And, None);

    let folded = expect_folded(f.fold(root).expect("comparison chain should fold"));

    assert!(f.folded_boolean(folded));
}

#[test]
fn constant_fold_keeps_unary_not_when_operand_is_not_bool_literal() {
    let mut f = FoldFixture::new();
    let operand = f.reference_path(
        PathId::ROOT,
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableReference,
    );
    let root = f.unary(operand, Operator::Not, None);

    let outcome = f.fold(root).expect("runtime not should not fail");

    assert_eq!(expect_not_constant(outcome), root);
    let view = f.builder.view(root).expect("unary root should be stored");
    assert!(matches!(
        view.operation(),
        ExpressionOperationView::LogicalNot { .. }
    ));
}

#[test]
fn constant_fold_preserves_runtime_operands_in_partial_fold() {
    // `flag and true` stays runtime. The reference and the literal keep their own nodes, and
    // the `and` operation is preserved rather than rebuilt or folded away.
    let mut f = FoldFixture::new();
    let flag = f.reference(
        "flag",
        builtin_type_ids::BOOL,
        None,
        ValueMode::MutableReference,
    );
    let literal = f.bool_literal(true, None);
    let root = f.binary(flag, literal, Operator::And, None);

    let outcome = f.fold(root).expect("partial boolean fold should not fail");
    assert_eq!(expect_not_constant(outcome), root);

    let view = f.builder.view(root).expect("runtime root should be stored");
    let ExpressionOperationView::LogicalBinary {
        left,
        right,
        operation,
    } = view.operation()
    else {
        panic!("the runtime `and` should stay a logical binary node");
    };
    assert!(matches!(operation, LogicalOperator::And));
    assert!(matches!(
        left.operation(),
        ExpressionOperationView::Reference(_)
    ));
    assert!(f.folded_boolean(right.id()));
}

#[test]
fn partial_fold_moves_non_foldable_operands_back_without_rebuilding_them() {
    // Distinct spans and value modes on every input make a rebuilt operand visible: the
    // runtime operand, the literal and the `and` operator keep their authored values.
    let mut f = FoldFixture::new();
    let flag_span = marked_span(70);
    let literal_span = marked_span(110);
    let operator_span = marked_span(230);
    let flag = f.reference(
        "flag",
        builtin_type_ids::BOOL,
        Some(flag_span),
        ValueMode::MutableReference,
    );
    let literal = f.bool_literal(true, Some(literal_span));
    let root = f.binary(flag, literal, Operator::And, Some(operator_span));

    let outcome = f.fold(root).expect("partial boolean fold should not fail");
    assert_eq!(expect_not_constant(outcome), root);

    let view = f.builder.view(root).expect("runtime root should be stored");
    assert_eq!(view.span(), Some(operator_span));
    let ExpressionOperationView::LogicalBinary { left, right, .. } = view.operation() else {
        panic!("the unfoldable operator should be preserved");
    };
    assert_eq!(left.span(), Some(flag_span));
    assert_eq!(left.value_mode(), ValueMode::MutableReference);
    assert_eq!(right.span(), Some(literal_span));
    assert_eq!(right.value_mode(), ValueMode::ImmutableOwned);
}

#[test]
fn partial_fold_keeps_the_folded_half_and_the_moved_half_distinct() {
    // The untouched runtime operand keeps its authored span. The constant `2 + 3` operand
    // folds to 5 under the runtime multiply and is anchored at its authored operator span.
    let mut f = FoldFixture::new();
    let counter_span = marked_span(30);
    let add_span = marked_span(50);
    let counter = f.reference(
        "counter",
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        Some(counter_span),
        ValueMode::ImmutableReference,
    );
    let two = f.fixed_integer(FixedScalar::I32, 2);
    let three = f.fixed_integer(FixedScalar::I32, 3);
    let root = f.resolve_rpn(vec![
        ExpressionRpnItem::Operand(counter),
        ExpressionRpnItem::Operand(two),
        ExpressionRpnItem::Operand(three),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: Some(add_span),
        },
        ExpressionRpnItem::Operator {
            operator: Operator::Multiply,
            span: None,
        },
    ]);

    let outcome = f.fold(root).expect("partial fold should not fail");
    assert_eq!(expect_not_constant(outcome), root);

    let view = f.builder.view(root).expect("runtime root should be stored");
    let ExpressionOperationView::NumericBinary { left, right, .. } = view.operation() else {
        panic!("the runtime multiply should stay a numeric binary node");
    };
    assert_eq!(left.span(), Some(counter_span));
    assert!(
        matches!(right.operation(), ExpressionOperationView::Constant(_)),
        "the constant half should fold to one operand"
    );
    assert_eq!(f.fixed_scalar_value(right.id()).as_i64(), Some(5));
    assert_eq!(right.span(), Some(add_span));
}

#[test]
fn full_fold_returns_the_folded_operand_with_its_source_anchor() {
    // A fully folded root is rebuilt as a constant that keeps the root's authored span rather
    // than a default produced by the rebuild.
    let mut f = FoldFixture::new();
    let root_span = marked_span(130);
    let lhs = f.fixed_integer_at(FixedScalar::I32, 20, Some(marked_span(120)));
    let rhs = f.fixed_integer_at(FixedScalar::I32, 22, Some(marked_span(140)));
    let root = f.binary(lhs, rhs, Operator::Add, Some(root_span));

    let folded = expect_folded(f.fold(root).expect("full integer fold should succeed"));

    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(42));
    assert_eq!(
        f.builder
            .view(folded)
            .expect("folded expression should be stored")
            .span(),
        Some(root_span)
    );
}

fn catch_handler_body(value: ExprId) -> Vec<AstNode> {
    vec![AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions: vec![value],
            span: None,
        }),
        span: None,
        scope: PathId::ROOT,
    }]
}

/// Builds `source as <target>` under `catch`, so the handler supplies the fallback when the cast fails.
fn catch_cast(
    f: &mut FoldFixture,
    source: ExprId,
    policy: BuiltinCastPolicyId,
    target_type_id: TypeId,
    handler_value: ExprId,
) -> ExprId {
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::Builtin { policy },
        target_type_id,
        BuiltinCastFallibility::Fallible,
        CastHandling::Recover,
        false,
    );
    let cast = f.builder.own(cast);
    wrap_catch_expression(
        &mut f.builder,
        cast,
        FallibleHandling::Handler {
            error: None,
            body: catch_handler_body(handler_value),
        },
        vec![target_type_id],
    )
    .expect("test catch should construct")
    .id()
}

#[test]
fn fold_cast_infallible_int_to_string_folds_to_string_literal() {
    let mut f = FoldFixture::new();
    let source = f.fixed_integer(FixedScalar::I32, 42);
    let target_type_id = f.types.builtins().string;
    let policy = BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::Builtin { policy },
        target_type_id,
        BuiltinCastFallibility::Infallible,
        CastHandling::Implicit,
        false,
    );

    let folded = f
        .fold_compile_time(cast)
        .expect("infallible builtin cast should fold");

    assert_eq!(f.type_of(folded), target_type_id);
    let ConstValuePayload::String(ConstStringValue::Text(interned)) = f.constant_payload(folded)
    else {
        panic!("expected folded I32 -> String cast to produce a text string");
    };
    assert_eq!(f.strings.resolve(*interned), "42");
}

#[test]
fn fold_structural_string_cast_reports_text_unavailable_rule() {
    let mut f = FoldFixture::new();
    let source = f.structural_string(vec![ConstStringPiece::SiteRoot]);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::Builtin { policy },
        target_type_id,
        BuiltinCastFallibility::Fallible,
        CastHandling::Implicit,
        false,
    );

    let error = f
        .fold_compile_time(cast)
        .expect_err("structural string cast should require final text");

    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        Some("string cast or parse"),
        &f.strings,
    );
}

#[test]
fn fold_cast_optional_wrap_coerces_value_to_optional() {
    let mut f = FoldFixture::new();
    let source = f.fixed_integer(FixedScalar::I32, 7);
    let target_type_id = f.types.builtins().string;
    let policy = BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::Builtin { policy },
        target_type_id,
        BuiltinCastFallibility::Infallible,
        CastHandling::Implicit,
        true,
    );

    let folded = f
        .fold_compile_time(cast)
        .expect("optional-wrapped infallible cast should fold");

    let option_type_id = f.types.intern_option(target_type_id);
    assert_eq!(f.type_of(folded), option_type_id);

    let ConstValuePayload::OptionSome(inner_id) = f.constant_payload(folded) else {
        panic!("expected optional-wrapped cast to produce an OptionSome payload");
    };
    let inner_payload = f
        .constants
        .payload(*inner_id)
        .expect("optional inner value should be stored");
    let ConstValuePayload::String(ConstStringValue::Text(interned)) = inner_payload else {
        panic!("expected optional inner value to be a text string");
    };
    assert_eq!(f.strings.resolve(*interned), "7");
}

#[test]
fn fold_cast_fallible_string_to_int_success_folds_to_int() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("123");
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.builtin_cast(source, policy, target_type_id);

    let folded = f
        .fold_compile_time(cast)
        .expect("successful fallible builtin cast should fold");

    assert_eq!(f.type_of(folded), target_type_id);
    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(123));
}

#[test]
fn fold_cast_fallible_string_to_int_failure_reports_builtin_cast_failed_in_const() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("not a number");
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = f.builtin_cast(source, policy, target_type_id);

    let error = f
        .fold_compile_time(cast)
        .expect_err("failed fallible builtin cast should report a const diagnostic");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn fold_cast_user_defined_evidence_rejected_in_const_context() {
    let mut f = FoldFixture::new();
    let source = f.fixed_integer(FixedScalar::I32, 42);
    let target_type_id = f.types.builtins().string;
    let method_path = f
        .path_fork
        .try_intern_portable_path("to_string", &mut f.strings)
        .expect("test path fits");
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::UserDefined {
            evidence_id: TraitEvidenceId(0),
            method_path,
        },
        target_type_id,
        BuiltinCastFallibility::Infallible,
        CastHandling::Implicit,
        false,
    );

    let error = f
        .fold_compile_time(cast)
        .expect_err("user-defined evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
    );
}

#[test]
fn fold_cast_generic_bound_evidence_rejected_in_const_context() {
    let mut f = FoldFixture::new();
    let source = f.fixed_integer(FixedScalar::I32, 42);
    let target_type_id = f.types.builtins().string;
    let cast = f.cast_expr(
        source,
        ResolvedCastEvidence::GenericBound {
            trait_id: TraitId(0),
            parameter_id: GenericParameterId(0),
        },
        target_type_id,
        BuiltinCastFallibility::Infallible,
        CastHandling::Implicit,
        false,
    );

    let error = f
        .fold_compile_time(cast)
        .expect_err("generic-bound evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::GenericBoundEvidenceNotConstFoldable,
    );
}

#[test]
fn known_invalid_cast_under_catch_is_diagnostic_in_constant_context() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("nope");
    let handler_value = f.fixed_integer(FixedScalar::I32, 0);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let root = catch_cast(&mut f, source, policy, target_type_id, handler_value);

    let error = f
        .fold_compile_time(root)
        .expect_err("a known invalid cast must not use its catch fallback");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn known_invalid_cast_under_catch_is_diagnostic_in_runtime_context() {
    let mut f = FoldFixture::new();
    let source = f.string_literal("nope");
    let handler_value = f.fixed_integer(FixedScalar::I32, 0);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let root = catch_cast(&mut f, source, policy, target_type_id, handler_value);

    let error = f
        .fold(root)
        .expect_err("a known invalid cast must not fall back to runtime catch delivery");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn runtime_parameter_cast_under_catch_remains_runtime() {
    let mut f = FoldFixture::new();
    let string_type = f.types.builtins().string;
    let source = f.reference("source", string_type, None, ValueMode::ImmutableReference);
    let handler_value = f.fixed_integer(FixedScalar::I32, 0);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let root = catch_cast(&mut f, source, policy, target_type_id, handler_value);

    let outcome = f
        .fold(root)
        .expect("an unknown parameter value must remain a runtime cast");
    let ConstantFoldOutcome::NotConstant(id) = outcome else {
        panic!("expected a runtime-rooted catch to stay non-constant, found {outcome:?}");
    };
    assert_eq!(id, root);
    assert!(matches!(
        f.builder
            .view(root)
            .expect("catch expression should be stored")
            .operation(),
        ExpressionOperationView::ValueBlock(_)
    ));
}

#[test]
fn fold_cast_fallible_builtin_success_with_catch_ignores_handler() {
    let mut f = FoldFixture::new();
    let source_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "source",
    );
    let handler_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::Builder,
        "render",
        "fallback",
    );
    let source = f.string_literal("123");
    let source = f
        .builder
        .with_synthetic_interface_provenance(
            f.builder.own(source),
            SyntheticInterfaceProvenance::single(source_member.clone()),
        )
        .expect("source should accept provenance")
        .id();
    let handler_value = f.fixed_integer(FixedScalar::I32, 999);
    let handler_value = f
        .builder
        .with_synthetic_interface_provenance(
            f.builder.own(handler_value),
            SyntheticInterfaceProvenance::single(handler_member),
        )
        .expect("handler value should accept provenance")
        .id();
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let root = catch_cast(&mut f, source, policy, target_type_id, handler_value);

    let folded = f
        .fold_compile_time(root)
        .expect("successful builtin cast should fold to success value even with catch handler");

    assert_eq!(f.type_of(folded), target_type_id);
    assert_eq!(f.fixed_scalar_value(folded).as_i64(), Some(123));
    let view = f
        .builder
        .view(folded)
        .expect("folded value should be stored");
    let provenance_members = view
        .occurrence_facts()
        .expect("folded value should keep its source occurrence")
        .provenance
        .members();
    assert_eq!(provenance_members, &[source_member]);
}

#[test]
fn evaluate_operator_folds_u32_arithmetic_and_rejects_overflow() {
    // U32 leaves fold in their own domain; overflow, underflow and
    // zero divisors report the stable integer failure codes at compile time.
    let mut f = FoldFixture::new();

    let lhs = f.fixed_integer(FixedScalar::U32, 1);
    let rhs = f.fixed_integer(FixedScalar::U32, 2);
    let folded = f.folded_binary(Operator::Add, None, lhs, rhs);
    assert_eq!(f.fixed_scalar_value(folded).as_u64(), Some(3));
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );

    let max = f.fixed_integer(FixedScalar::U32, i128::from(u32::MAX));
    let one = f.fixed_integer(FixedScalar::U32, 1);
    let error = f
        .fold_binary(Operator::Add, None, max, one)
        .expect_err("u32::MAX + 1 exceeds the U32 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &f.strings,
    );

    // `0 ^ 0` folds to one and overflowing powers report the boundary.
    let zero = f.fixed_integer(FixedScalar::U32, 0);
    let zero_again = f.fixed_integer(FixedScalar::U32, 0);
    let folded = f.folded_binary(Operator::Exponent, None, zero, zero_again);
    assert_eq!(f.fixed_scalar_value(folded).as_u64(), Some(1));

    let base = f.fixed_integer(FixedScalar::U32, 2);
    let exponent = f.fixed_integer(FixedScalar::U32, 32);
    let error = f
        .fold_binary(Operator::Exponent, None, base, exponent)
        .expect_err("2 ^ 32 exceeds the U32 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &f.strings,
    );

    let zero = f.fixed_integer(FixedScalar::U32, 0);
    let one = f.fixed_integer(FixedScalar::U32, 1);
    let error = f
        .fold_binary(Operator::Subtract, None, zero, one)
        .expect_err("0 - 1 underflows U32");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &f.strings,
    );

    let seven = f.fixed_integer(FixedScalar::U32, 7);
    let zero = f.fixed_integer(FixedScalar::U32, 0);
    let error = f
        .fold_binary(Operator::IntDivide, None, seven, zero)
        .expect_err("U32 // 0 must fail at compile time");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &f.strings,
    );

    // U32 `/` folds in F64 directly, never through an integer carrier.
    let seven = f.fixed_integer(FixedScalar::U32, 7);
    let two = f.fixed_integer(FixedScalar::U32, 2);
    let folded = f.folded_binary(Operator::Divide, None, seven, two);
    assert_eq!(f.fixed_scalar_value(folded).scalar(), FixedScalar::F64);
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn evaluate_operator_folds_u32_comparisons_and_dec_mixed_pairs() {
    // U32 compares exactly with U32 and I32, and Dec mixes with
    // U32 through the shared exact rule used by `k2 #Dec2 = cu + 0.5`.
    let mut f = FoldFixture::new();

    let three = f.fixed_integer(FixedScalar::U32, 3);
    let five = f.fixed_integer(FixedScalar::U32, 5);
    let less = f.folded_binary(Operator::LessThan, None, three, five);
    assert!(f.folded_boolean(less));

    let three = f.fixed_integer(FixedScalar::U32, 3);
    let minus_one = f.fixed_integer(FixedScalar::I32, -1);
    let less = f.folded_binary(Operator::LessThan, None, three, minus_one);
    assert!(!f.folded_boolean(less));

    let zero = f.fixed_integer(FixedScalar::U32, 0);
    let minus_one = f.fixed_integer(FixedScalar::I32, -1);
    let greater = f.folded_binary(Operator::GreaterThan, None, zero, minus_one);
    assert!(f.folded_boolean(greater));

    let three = f.fixed_integer(FixedScalar::U32, 3);
    let dec = f.number("5.00", NumericLiteralSign::Positive, 2);
    let less = f.folded_binary(Operator::LessThan, None, three, dec);
    assert!(f.folded_boolean(less));
}

#[test]
fn evaluate_operator_folds_u64_at_full_bounds_and_rejects_overflow() {
    // The U64 domain spans the whole u64 range, so values above i64::MAX fold exactly
    // and u64::MAX + 1, 2 ^ 64 and overflowing products report the stable integer failures.
    let mut f = FoldFixture::new();

    let max = f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX));
    let zero = f.fixed_integer(FixedScalar::U64, 0);
    let folded = f.folded_binary(Operator::Subtract, None, max, zero);
    assert_eq!(f.fixed_scalar_value(folded).as_u64(), Some(u64::MAX));
    assert_eq!(
        f.type_of(folded),
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );

    let max = f.fixed_integer(FixedScalar::U64, i128::from(u64::MAX));
    let one = f.fixed_integer(FixedScalar::U64, 1);
    let error = f
        .fold_binary(Operator::Add, None, max, one)
        .expect_err("u64::MAX + 1 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &f.strings,
    );

    let base = f.fixed_integer(FixedScalar::U64, 2);
    let exponent = f.fixed_integer(FixedScalar::U64, 64);
    let error = f
        .fold_binary(Operator::Exponent, None, base, exponent)
        .expect_err("2 ^ 64 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &f.strings,
    );

    let big = f.fixed_integer(FixedScalar::U64, 9_223_372_036_854_775_808);
    let two = f.fixed_integer(FixedScalar::U64, 2);
    let error = f
        .fold_binary(Operator::Multiply, None, big, two)
        .expect_err("2^63 * 2 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("*"),
        &f.strings,
    );
}
