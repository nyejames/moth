//! HIR validation regression tests.
//!
//! WHAT: exercises the post-lowering HIR validator against valid and intentionally broken modules.
//! WHY: validator coverage needs focused tests that isolate invariants from the rest of lowering.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericParameterId, NominalTypeId, TypeConstructor,
    TypeId, builtin_type_ids,
};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{
    HirTestChoiceDefinition, build_ast_with_choices, build_ast_with_registered_types, lower_ast,
    validate_module_for_tests,
};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::{
    ChoiceId, FieldId, HirNodeId, HirValueId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::{
    HirChoice, HirChoiceField, HirChoiceVariant, HirModule,
};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::param_with_type_id;
use crate::compiler_frontend::tests::type_id_fixture_support::no_value_expr;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;

use crate::compiler_frontend::value_mode::ValueMode;

fn node(kind: NodeKind, span: Option<SourceSpan>) -> AstNode {
    AstNode {
        kind,
        span,
        scope: crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
    }
}

fn make_test_variable(name: PathId, value: Expression) -> Declaration {
    Declaration {
        id: name,
        value,
        binding_span: None,
        config_qualifier: None,
    }
}

fn function_node(
    name: PathId,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    span: Option<SourceSpan>,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), span)
}

// Shared builders for the validation regressions below.
fn generic_parameter_type_id(
    string_table: &mut StringTable,
    type_environment: &mut TypeEnvironment,
) -> TypeId {
    let parameter_name = string_table.intern("T");
    type_environment.intern_generic_parameter(GenericParameterId(0), parameter_name)
}

fn minimal_lowered_hir_module() -> (StringTable, HirModule, TypeEnvironment) {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");

    (string_table, module, type_environment)
}

fn start_entry_block_index(module: &HirModule) -> usize {
    module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize
}

fn validation_error_for_injected_local_type(
    build_type: impl FnOnce(&mut StringTable, &mut TypeEnvironment) -> TypeId,
) -> CompilerError {
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let local_type_id = build_type(&mut string_table, &mut type_environment);

    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    entry_block.locals.push(HirLocal {
        id: LocalId(9000),
        ty: local_type_id,
        mutable: false,
        region: entry_block.region,
        span: None,
    });

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter inside TypeId")
}

fn inject_collection_expression_statement(
    module: &mut HirModule,
    collection_type_id: TypeId,
    span: Option<SourceSpan>,
) {
    let _path_fork = super::PathInternerFork::empty();
    let entry_block_index = start_entry_block_index(module);
    let entry_block = &mut module.blocks[entry_block_index];
    let value_id = HirValueId(9000);
    let statement_id = HirNodeId(9000);
    let expression = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Collection(vec![]),
        ty: collection_type_id,
        value_kind: ValueKind::RValue,
        region: entry_block.region,
        span: None,
    };

    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id, span);
    entry_block.statements.push(statement);
}

fn int_expression(
    id: HirValueId,
    value: i64,
    type_id: TypeId,
    region: RegionId,
    span: &Option<SourceSpan>,
    module: &mut HirModule,
) -> HirExpression {
    module.side_table.map_value(*span, id, *span);

    HirExpression {
        id,
        kind: HirExpressionKind::Int(value),
        ty: type_id,
        value_kind: ValueKind::RValue,
        region,
        span: None,
    }
}

fn float_expression(
    id: HirValueId,
    value: f64,
    type_id: TypeId,
    region: RegionId,
    span: &Option<SourceSpan>,
    module: &mut HirModule,
) -> HirExpression {
    module.side_table.map_value(*span, id, *span);

    HirExpression {
        id,
        kind: HirExpressionKind::Float(value),
        ty: type_id,
        value_kind: ValueKind::RValue,
        region,
        span: None,
    }
}

fn validate_numeric_op_for_test(
    op: HirNumericOp,
    operand_types: &[TypeId],
    result_type: TypeId,
) -> Result<(), CompilerError> {
    validate_numeric_op_for_test_with_types(op, |_| (operand_types.to_vec(), result_type))
}

fn validate_numeric_op_for_test_with_types(
    op: HirNumericOp,
    build_types: impl FnOnce(&mut TypeEnvironment) -> (Vec<TypeId>, TypeId),
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let (operand_types, result_type) = build_types(&mut type_environment);
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: result_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let mut operands = Vec::with_capacity(operand_types.len());
    for (index, operand_type) in operand_types.iter().copied().enumerate() {
        let local_id = LocalId(9001 + index as u32);
        module.blocks[entry_block_index].locals.push(HirLocal {
            id: local_id,
            ty: operand_type,
            mutable: false,
            region: entry_region,
            span: None,
        });

        let value_id = HirValueId(9000 + index as u32);
        module.side_table.map_value(span, value_id, span);
        operands.push(HirExpression {
            id: value_id,
            kind: HirExpressionKind::Load(HirPlace::Local(local_id)),
            ty: operand_type,
            value_kind: ValueKind::RValue,
            region: entry_region,
            span: None,
        });
    }

    let operands = match (op.operator.is_unary(), operands.as_slice()) {
        (true, [operand]) => HirNumericOperands::Unary {
            operand: operand.clone(),
        },
        (false, [left, right]) => HirNumericOperands::Binary {
            left: left.clone(),
            right: right.clone(),
        },
        _ => panic!("test NumericOp operand types must match operation arity"),
    };
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op,
            failure_mode: NumericFailureMode::Trap,
            operands,
            result: result_local,
        },
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
}

fn validate_number_numeric_op_for_test(
    operator: NumericOperator,
    scale: NumberScale,
    right_domain: Option<NumericScalar>,
) -> Result<(), CompilerError> {
    validate_numeric_op_for_test_with_types(
        HirNumericOp {
            operator,
            domain: NumericScalar::Number(scale),
        },
        |type_environment| {
            let number_type = type_environment.intern_number(scale);
            let right_type = right_domain.map_or(number_type, |domain| match domain {
                NumericScalar::Number(right_scale) => type_environment.intern_number(right_scale),
                _ => domain.type_id(type_environment),
            });
            let operand_types = if operator.is_unary() {
                vec![number_type]
            } else {
                vec![number_type, right_type]
            };
            (operand_types, number_type)
        },
    )
}

fn builtin_error_type_id(type_environment: &mut TypeEnvironment) -> TypeId {
    let error_identity = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
    if let Some(error_type_id) = type_environment.type_id_for_canonical_identity(&error_identity) {
        return error_type_id;
    }

    let (_, error_type_id) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Vec::new().into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });
    type_environment
        .register_canonical_identity(error_identity, error_type_id)
        .expect("test builtin Error identity should register");
    error_type_id
}

fn validate_cast_op_for_test(
    policy: BuiltinCastPolicyId,
    source_type: TypeId,
    result_success_type: TypeId,
    result_error_override: Option<TypeId>,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let source_local_id = LocalId(9001);
    let result_local_id = LocalId(9000);
    let source_value_id = HirValueId(9000);
    let builtin_error_type = builtin_error_type_id(&mut type_environment);
    let result_error_type = result_error_override.unwrap_or(builtin_error_type);
    let result_type =
        type_environment.intern_fallible_carrier(result_success_type, result_error_type);

    module.blocks[entry_block_index].locals.extend([
        HirLocal {
            id: result_local_id,
            ty: result_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
        HirLocal {
            id: source_local_id,
            ty: source_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
    ]);

    module.side_table.map_value(span, source_value_id, span);
    let source = HirExpression {
        id: source_value_id,
        kind: HirExpressionKind::Load(HirPlace::Local(source_local_id)),
        ty: source_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::CastOp {
            policy,
            source,
            result: Some(result_local_id),
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
}

fn numeric_scalar_type_for_test(
    type_environment: &mut TypeEnvironment,
    scalar: NumericScalar,
) -> TypeId {
    match scalar {
        NumericScalar::Number(scale) => type_environment.intern_number(scale),
        _ => scalar.type_id(type_environment),
    }
}

fn validate_number_cast_op_for_test(
    source_scalar: NumericScalar,
    target_scalar: NumericScalar,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let source_type = numeric_scalar_type_for_test(&mut type_environment, source_scalar);
    let target_type = numeric_scalar_type_for_test(&mut type_environment, target_scalar);
    let error_type = builtin_error_type_id(&mut type_environment);
    let result_type = type_environment.intern_fallible_carrier(target_type, error_type);
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let source_local = LocalId(9001);
    let result_local = LocalId(9000);
    let source_value = HirValueId(9000);
    module.blocks[entry_block_index].locals.extend([
        HirLocal {
            id: result_local,
            ty: result_type,
            mutable: false,
            region: entry_region,
            span,
        },
        HirLocal {
            id: source_local,
            ty: source_type,
            mutable: false,
            region: entry_region,
            span,
        },
    ]);
    module.side_table.map_value(span, source_value, span);
    let source = HirExpression {
        id: source_value,
        kind: HirExpressionKind::Load(HirPlace::Local(source_local)),
        ty: source_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span,
    };
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::CastOp {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: source_scalar,
                target: target_scalar,
            },
            source,
            result: Some(result_local),
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
}

fn fixed_type(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn validate_comparison_for_test(
    left_type: TypeId,
    right_type: TypeId,
    op: HirBinOp,
) -> Result<(), CompilerError> {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let left_local = LocalId(9000);
    let right_local = LocalId(9001);
    module.blocks[entry_block_index].locals.extend([
        HirLocal {
            id: left_local,
            ty: left_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
        HirLocal {
            id: right_local,
            ty: right_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
    ]);

    let left_id = HirValueId(9000);
    let right_id = HirValueId(9001);
    module.side_table.map_value(span, left_id, span);
    module.side_table.map_value(span, right_id, span);
    let left = HirExpression {
        id: left_id,
        kind: HirExpressionKind::Load(HirPlace::Local(left_local)),
        ty: left_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    let right = HirExpression {
        id: right_id,
        kind: HirExpressionKind::Load(HirPlace::Local(right_local)),
        ty: right_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    let expression = HirExpression {
        id: HirValueId(9002),
        kind: HirExpressionKind::BinOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: builtin_type_ids::BOOL,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    module.side_table.map_value(span, expression.id, span);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
}

fn validate_number_comparison_for_test(
    integer_domain: NumericScalar,
    explicitly_converted: bool,
) -> Result<(), CompilerError> {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let scale = NumberScale::new(2).expect("test Dec scale is valid");
    let number_type = type_environment.intern_number(scale);
    let integer_type = integer_domain.type_id(&type_environment);
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let number_local = LocalId(9000);
    let integer_local = LocalId(9001);
    module.blocks[entry_block_index].locals.extend([
        HirLocal {
            id: number_local,
            ty: number_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
        HirLocal {
            id: integer_local,
            ty: integer_type,
            mutable: false,
            region: entry_region,
            span: None,
        },
    ]);

    let number_value_id = HirValueId(9000);
    let integer_value_id = HirValueId(9001);
    module.side_table.map_value(span, number_value_id, span);
    module.side_table.map_value(span, integer_value_id, span);
    let number = HirExpression {
        id: number_value_id,
        kind: HirExpressionKind::Load(HirPlace::Local(number_local)),
        ty: number_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    let integer = HirExpression {
        id: integer_value_id,
        kind: HirExpressionKind::Load(HirPlace::Local(integer_local)),
        ty: integer_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    let right = if explicitly_converted {
        let converted_id = HirValueId(9002);
        module.side_table.map_value(span, converted_id, span);
        HirExpression {
            id: converted_id,
            kind: HirExpressionKind::Cast {
                source: Box::new(integer),
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: integer_domain,
                    target: NumericScalar::Number(scale),
                },
            },
            ty: number_type,
            value_kind: ValueKind::RValue,
            region: entry_region,
            span: None,
        }
    } else {
        integer
    };
    let expression = HirExpression {
        id: HirValueId(9003),
        kind: HirExpressionKind::BinOp {
            op: HirBinOp::Lt,
            left: Box::new(number),
            right: Box::new(right),
        },
        ty: builtin_type_ids::BOOL,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    module.side_table.map_value(span, expression.id, span);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
}

#[test]
fn validator_rejects_numeric_ops_with_invalid_fixed_domains() {
    let f32_type = fixed_type(FixedScalar::F32);
    let u32_type = fixed_type(FixedScalar::U32);
    let i8_type = fixed_type(FixedScalar::I8);
    let cases = [
        (
            HirNumericOp {
                operator: NumericOperator::IntegerDivide,
                domain: NumericScalar::Fixed(FixedScalar::F32),
            },
            vec![f32_type, f32_type],
            f32_type,
        ),
        (
            HirNumericOp {
                operator: NumericOperator::Divide,
                domain: NumericScalar::Fixed(FixedScalar::U32),
            },
            vec![u32_type, u32_type],
            u32_type,
        ),
        (
            HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Fixed(FixedScalar::U32),
            },
            vec![u32_type],
            u32_type,
        ),
        (
            HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Fixed(FixedScalar::I8),
            },
            vec![i8_type],
            i8_type,
        ),
    ];

    for (op, operand_types, result_type) in cases {
        let error = validate_numeric_op_for_test(op, &operand_types, result_type)
            .expect_err("validator should reject NumericOp domains unsupported by the operator");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }
}

#[test]
fn validator_rejects_fixed_numeric_op_operand_type_mismatch() {
    let u8_type = fixed_type(FixedScalar::U8);
    let u32_type = fixed_type(FixedScalar::U32);
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u8_type, u32_type],
        u32_type,
    )
    .expect_err("validator should reject operands not converted to the NumericOp domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_accepts_valid_fixed_numeric_ops() {
    let u32_type = fixed_type(FixedScalar::U32);
    let i32_type = fixed_type(FixedScalar::I32);

    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u32_type, u32_type],
        u32_type,
    )
    .expect("valid fixed unsigned addition should pass HIR validation");
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Fixed(FixedScalar::I32),
        },
        &[i32_type],
        i32_type,
    )
    .expect("valid fixed signed negation should pass HIR validation");
    let f64_type = fixed_type(FixedScalar::F64);
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Fixed(FixedScalar::F64),
        },
        &[f64_type, f64_type],
        f64_type,
    )
    .expect("valid fixed binary-float division should pass HIR validation");
    validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::IntegerDivide,
            domain: NumericScalar::Fixed(FixedScalar::U32),
        },
        &[u32_type, u32_type],
        u32_type,
    )
    .expect("valid fixed integer division should pass HIR validation");
}

#[test]
fn validator_accepts_uint_numeric_ops_and_rejects_mixed_or_negated_pairs() {
    // Uint arithmetic validates in its own domain; mixed Uint/Int
    // operands and Uint negation never reach HIR, so both are rejected here.
    let uint_type = builtin_type_ids::UINT;
    let int_type = builtin_type_ids::INT;

    for operator in [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::IntegerDivide,
        NumericOperator::Remainder,
        NumericOperator::Power,
    ] {
        validate_numeric_op_for_test(
            HirNumericOp {
                operator,
                domain: NumericScalar::Uint,
            },
            &[uint_type, uint_type],
            uint_type,
        )
        .expect("valid Uint arithmetic should pass HIR validation");
    }

    // Uint `/` computes in Float, so a `/` NumericOp in the Uint domain itself
    // is rejected: lowering must convert the operands before emitting it.
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Divide,
            domain: NumericScalar::Uint,
        },
        &[uint_type, uint_type],
        uint_type,
    )
    .expect_err("Uint division must leave the Uint domain before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Uint,
        },
        &[uint_type, int_type],
        uint_type,
    )
    .expect_err("mixed Uint/Int operands must be converted before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Negate,
            domain: NumericScalar::Uint,
        },
        &[uint_type],
        uint_type,
    )
    .expect_err("Uint negation has no operator domain");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_accepts_uint_cast_pairs_and_comparisons() {
    // Uint participates in fallible CastOp narrowing and in exact
    // Uint/Uint and Uint/Int comparisons without any conversion failure.
    validate_number_cast_op_for_test(NumericScalar::Uint, NumericScalar::Int)
        .expect("Uint-to-Int narrowing should use a fallible CastOp");
    validate_number_cast_op_for_test(NumericScalar::Int, NumericScalar::Uint)
        .expect("Int-to-Uint narrowing should use a fallible CastOp");

    validate_comparison_for_test(builtin_type_ids::UINT, builtin_type_ids::UINT, HirBinOp::Lt)
        .expect("exact Uint comparison should remain valid");
    validate_comparison_for_test(builtin_type_ids::UINT, builtin_type_ids::INT, HirBinOp::Gt)
        .expect("exact Uint/Int comparison should remain valid");
}

#[test]
fn validator_accepts_number_operator_scale_rules_and_int_power_exponent() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");

    for operator in [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::Divide,
        NumericOperator::Remainder,
    ] {
        validate_number_numeric_op_for_test(operator, scale_two, None)
            .expect("valid Dec2 arithmetic should satisfy HIR validation");
    }

    validate_number_numeric_op_for_test(NumericOperator::IntegerDivide, NumberScale::ZERO, None)
        .expect("Dec0 integer division should be valid");
    validate_number_numeric_op_for_test(NumericOperator::Negate, scale_two, None)
        .expect("Dec negation should be valid");
    validate_number_numeric_op_for_test(
        NumericOperator::Power,
        scale_two,
        Some(NumericScalar::Int),
    )
    .expect("Dec power with a profile Int exponent should be valid");
}

#[test]
fn validator_rejects_number_operator_scale_and_operand_mismatches() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");
    let scale_three = NumberScale::new(3).expect("test Dec scale is valid");

    for (operator, scale) in [
        (NumericOperator::Divide, NumberScale::ZERO),
        (NumericOperator::IntegerDivide, scale_two),
    ] {
        let error = validate_number_numeric_op_for_test(operator, scale, None)
            .expect_err("invalid Dec scale/operator pair should be rejected");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Power,
        scale_two,
        Some(NumericScalar::Number(scale_two)),
    )
    .expect_err("Dec power must not scale its exponent to Dec");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Add,
        scale_two,
        Some(NumericScalar::Int),
    )
    .expect_err("mixed integer operands must be explicitly converted before NumericOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    let error = validate_number_numeric_op_for_test(
        NumericOperator::Add,
        scale_two,
        Some(NumericScalar::Number(scale_three)),
    )
    .expect_err("NumericOp operands must share the result scale");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_unregistered_number_domain_without_panicking() {
    let scale = NumberScale::new(7).expect("test Dec scale is valid");
    let error = validate_numeric_op_for_test(
        HirNumericOp {
            operator: NumericOperator::Add,
            domain: NumericScalar::Number(scale),
        },
        &[builtin_type_ids::INT, builtin_type_ids::INT],
        builtin_type_ids::INT,
    )
    .expect_err("an unregistered Dec scale cannot form a valid HIR numeric domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_distinguishes_fallible_number_cast_pairs() {
    let scale_two = NumberScale::new(2).expect("test Dec scale is valid");
    let scale_four = NumberScale::new(4).expect("test Dec scale is valid");

    validate_number_cast_op_for_test(
        NumericScalar::Number(scale_four),
        NumericScalar::Number(scale_two),
    )
    .expect("Dec scale narrowing should use a fallible CastOp");
    validate_number_cast_op_for_test(NumericScalar::Number(scale_two), NumericScalar::Int)
        .expect("Dec-to-Int should use a fallible CastOp");

    let error = validate_number_cast_op_for_test(
        NumericScalar::Number(scale_two),
        NumericScalar::Number(scale_four),
    )
    .expect_err("an infallible Dec widening must not be represented by CastOp");
    assert_eq!(error.error_type, ErrorType::HirTransformation);

    for (source, target) in [
        (
            NumericScalar::Number(scale_two),
            NumericScalar::Number(scale_two),
        ),
        (NumericScalar::Number(scale_two), NumericScalar::Float),
        (
            NumericScalar::Number(scale_two),
            NumericScalar::Fixed(FixedScalar::Byte),
        ),
        (NumericScalar::Float, NumericScalar::Number(scale_two)),
    ] {
        let error = validate_number_cast_op_for_test(source, target)
            .expect_err("unsupported Dec cast pair must have no HIR policy row");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
    }
}

#[test]
fn validator_rejects_numeric_conversion_cast_source_policy_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Float,
            target: NumericScalar::Int,
        },
        builtin_type_ids::INT,
        builtin_type_ids::INT,
        None,
    )
    .expect_err("validator should reject a CastOp source that disagrees with its policy");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_numeric_conversion_cast_carrier_success_type_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Int,
            target: NumericScalar::Int,
        },
        builtin_type_ids::INT,
        fixed_type(FixedScalar::U8),
        None,
    )
    .expect_err("validator should reject a CastOp carrier with the wrong success type");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_numeric_conversion_cast_carrier_error_type_mismatch() {
    let error = validate_cast_op_for_test(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Int,
            target: NumericScalar::Int,
        },
        builtin_type_ids::INT,
        builtin_type_ids::INT,
        Some(builtin_type_ids::STRING),
    )
    .expect_err("validator should reject a CastOp carrier with a non-builtin error slot");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_uses_numeric_comparison_compatibility_and_keeps_same_type_other_values() {
    let i64_type = fixed_type(FixedScalar::I64);
    let u64_type = fixed_type(FixedScalar::U64);
    validate_comparison_for_test(i64_type, u64_type, HirBinOp::Lt)
        .expect("exact mixed signed/unsigned comparison should remain valid");

    let byte_type = fixed_type(FixedScalar::Byte);
    validate_comparison_for_test(byte_type, byte_type, HirBinOp::Eq)
        .expect("same-type Byte comparison should remain valid");

    let f64_type = fixed_type(FixedScalar::F64);
    let error = validate_comparison_for_test(builtin_type_ids::INT, f64_type, HirBinOp::Eq)
        .expect_err("incompatible numeric comparison should be rejected");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_requires_explicit_integer_to_number_comparison_conversion() {
    validate_number_comparison_for_test(NumericScalar::Int, true)
        .expect("explicit profile Int-to-Dec comparison conversion should pass");
    validate_number_comparison_for_test(NumericScalar::Fixed(FixedScalar::U64), true)
        .expect("explicit fixed-integer-to-Dec comparison conversion should pass");

    let error = validate_number_comparison_for_test(NumericScalar::Int, false)
        .expect_err("mixed Dec and Int comparison must be converted in HIR");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn valid_module_passes_explicit_validation() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept a valid lowered module");
}

#[test]
fn validator_rejects_assertion_message_evaluation_fact_mismatch() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    let message_span = None;
    let option_string = type_environment.intern_option(builtin_type_ids::STRING);
    let message_id = HirValueId(9000);
    let message = HirExpression {
        id: message_id,
        kind: HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 0,
            fields: vec![],
        },
        ty: option_string,
        value_kind: ValueKind::RValue,
        region: entry_block.region,
        span: None,
    };

    module
        .side_table
        .map_value(message_span, message_id, message_span);
    entry_block.terminator = HirTerminator::AssertFailure {
        message,
        message_evaluation: HirAssertionMessageEvaluation::Folded,
    };

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject a stale assertion message evaluation fact");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("evaluation fact"),
        "expected evaluation-fact mismatch, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_numeric_op_operand_shape_mismatch() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: int_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let left = int_expression(
        HirValueId(9000),
        1,
        int_type,
        entry_region,
        &span,
        &mut module,
    );
    let right = int_expression(
        HirValueId(9001),
        2,
        int_type,
        entry_region,
        &span,
        &mut module,
    );

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Int,
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: result_local,
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject mismatched NumericOp arity");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("operand shape does not match the operation arity")
    );
}

#[test]
fn validator_rejects_numeric_op_operand_domain_mismatch() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;
    let float_type = type_environment.builtins().float;
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: int_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    // Int-domain addition fed Float operands: lowering must convert explicitly, so a
    // mismatched operand type is a builder bug the validator has to catch.
    let left = float_expression(
        HirValueId(9000),
        1.0,
        float_type,
        entry_region,
        &span,
        &mut module,
    );
    let right = float_expression(
        HirValueId(9001),
        2.0,
        float_type,
        entry_region,
        &span,
        &mut module,
    );

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Int,
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: result_local,
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject NumericOp operands outside the domain type");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_rejects_integer_divide_on_float_domain() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let float_type = type_environment.builtins().float;
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: float_type,
        mutable: false,
        region: entry_region,
        span: None,
    });

    let left = float_expression(
        HirValueId(9000),
        1.0,
        float_type,
        entry_region,
        &span,
        &mut module,
    );
    let right = float_expression(
        HirValueId(9001),
        2.0,
        float_type,
        entry_region,
        &span,
        &mut module,
    );

    // Truncating `//` needs an integer domain; real `/` is the only division on Float.
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::IntegerDivide,
                domain: NumericScalar::Float,
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: result_local,
        },
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject IntegerDivide on a Float domain");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("operator does not match the domain"));
}

fn append_expression_for_validation(
    module: &mut HirModule,
    span: &Option<SourceSpan>,
    left_type: TypeId,
    result_type: TypeId,
    int_type: TypeId,
) -> HirExpression {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let left = HirExpression {
        id: HirValueId(9010),
        kind: HirExpressionKind::StringLiteral("prefix".to_owned()),
        ty: left_type,
        value_kind: ValueKind::Const,
        region: entry_region,
        span: None,
    };
    module.side_table.map_value(*span, left.id, *span);

    let right = int_expression(HirValueId(9011), 7, int_type, entry_region, span, module);
    let expression = HirExpression {
        id: HirValueId(9012),
        kind: HirExpressionKind::BinOp {
            op: HirBinOp::StringAppend,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: result_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    module.side_table.map_value(*span, expression.id, *span);
    expression
}

#[test]
fn validator_accepts_internal_string_append_with_scalar_chunk() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        string_type,
        string_type,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9013),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("valid StringAppend should pass HIR validation");
}

#[test]
fn validator_rejects_string_append_with_non_string_result() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        string_type,
        type_environment.builtins().int,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9014),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("StringAppend with a non-String result should fail validation");
    assert!(
        error
            .msg
            .contains("StringAppend must produce a String from a String accumulator")
    );
}

#[test]
fn validator_rejects_string_append_with_non_string_accumulator() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let expression = append_expression_for_validation(
        &mut module,
        &span,
        type_environment.builtins().int,
        type_environment.builtins().string,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9015),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("StringAppend with a non-String accumulator should fail validation");
    assert!(
        error
            .msg
            .contains("StringAppend must produce a String from a String accumulator")
    );
}

#[test]
fn validator_rejects_plain_numeric_unary_op() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;

    let operand = int_expression(
        HirValueId(9000),
        1,
        int_type,
        entry_region,
        &span,
        &mut module,
    );

    let expression = HirExpression {
        id: HirValueId(9001),
        kind: HirExpressionKind::UnaryOp {
            op: HirUnaryOp::Neg,
            operand: Box::new(operand),
        },
        ty: int_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
        span: None,
    };
    module.side_table.map_value(span, expression.id, span);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject plain numeric UnaryOp");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Plain HirUnaryOp::Neg must be lowered through HirStatementKind::NumericOp")
    );
}

fn inject_float_statement(
    module: &mut HirModule,
    type_environment: &TypeEnvironment,
    span: &Option<SourceSpan>,
    kind: HirStatementKind,
    result_type: TypeId,
) {
    let _path_fork = super::PathInternerFork::empty();
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);

    let source = float_expression(
        HirValueId(9000),
        1.5,
        type_environment.builtins().float,
        entry_region,
        span,
        module,
    );

    {
        let entry_block = &mut module.blocks[entry_block_index];
        entry_block.locals.push(HirLocal {
            id: result_local,
            ty: result_type,
            mutable: false,
            region: entry_region,
            span: None,
        });
    }

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: match kind {
            HirStatementKind::FormatFloat { failure_mode, .. } => HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result: result_local,
            },
            HirStatementKind::ValidateFloat { failure_mode, .. } => {
                HirStatementKind::ValidateFloat {
                    source,
                    failure_mode,
                    result: result_local,
                }
            }
            _ => panic!("inject_float_statement only supports FormatFloat and ValidateFloat"),
        },
        span: *span,
    };

    module.side_table.map_statement(*span, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

#[test]
fn validator_accepts_format_float_trap() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::Trap,
            result: LocalId(0),
        },
        string_type,
    );

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept FormatFloat with Trap and String result local");
}

#[test]
fn validator_accepts_validate_float_trap() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::Trap,
            result: LocalId(0),
        },
        float_type,
    );

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept ValidateFloat with Trap and Float result local");
}

#[test]
fn validator_rejects_format_float_trap_with_non_string_result() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::Trap,
            result: LocalId(0),
        },
        float_type,
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject FormatFloat Trap with non-String result local");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("FormatFloat Trap result local has the wrong success type")
    );
}

#[test]
fn validator_accepts_format_float_return_error_with_carrier() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;
    let error_type = builtin_error_type_id(&mut type_environment);
    let carrier_type = type_environment.intern_fallible_carrier(string_type, error_type);

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::ReturnError,
            result: LocalId(0),
        },
        carrier_type,
    );

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept FormatFloat with ReturnError and carrier result local");
}

#[test]
fn validator_rejects_format_float_return_error_without_carrier() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let string_type = type_environment.builtins().string;

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::ReturnError,
            result: LocalId(0),
        },
        string_type,
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment).expect_err(
        "validator should reject FormatFloat with ReturnError and non-carrier result local",
    );

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "FormatFloat ReturnError result local must have an internal fallible carrier type"
    ));
}

#[test]
fn validator_rejects_validate_float_return_error_without_carrier() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &span,
        HirStatementKind::ValidateFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            failure_mode: NumericFailureMode::ReturnError,
            result: LocalId(0),
        },
        float_type,
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment).expect_err(
        "validator should reject ValidateFloat with ReturnError and non-carrier result local",
    );

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "ValidateFloat ReturnError result local must have an internal fallible carrier type"
    ));
}

#[test]
fn validator_rejects_discharged_mode_on_float_and_range_step_statements() {
    let _path_fork = super::PathInternerFork::empty();
    let dummy_source = || HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::Float(0.0),
        ty: TypeId(u32::MAX),
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span: None,
    };
    // (statement kind under test, expected rejection): the `inject_float_statement`
    // fixture reads the failure mode off the passed kind, so each case reuses it;
    // `RangeStepFailure` needs its own injection below.
    let float_cases = [
        (
            HirStatementKind::FormatFloat {
                source: dummy_source(),
                failure_mode: NumericFailureMode::Infallible,
                result: LocalId(0),
            },
            "FormatFloat cannot use the discharged numeric mode",
        ),
        (
            HirStatementKind::ValidateFloat {
                source: dummy_source(),
                failure_mode: NumericFailureMode::Infallible,
                result: LocalId(0),
            },
            "ValidateFloat cannot use the discharged numeric mode",
        ),
    ];

    for (kind, expected_message) in float_cases {
        let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
        let span = None;
        let success_type = match &kind {
            HirStatementKind::FormatFloat { .. } => type_environment.builtins().string,
            _ => type_environment.builtins().float,
        };
        inject_float_statement(&mut module, &type_environment, &span, kind, success_type);

        let error = validate_module_for_tests(&module, &string_table, &type_environment)
            .expect_err("validator should reject Infallible on float statements");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
        assert!(
            error.msg.contains(expected_message),
            "unexpected rejection message: {}",
            error.msg
        );
    }

    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let span = None;
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: type_environment.builtins().bool,
        mutable: false,
        region: entry_region,
        span: None,
    });
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::RangeStepFailure {
            cause: RangeStepFailureCause::ZeroStep,
            failure_mode: NumericFailureMode::Infallible,
            result: result_local,
        },
        span,
    };
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject Infallible on RangeStepFailure");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("RangeStepFailure cannot use the discharged numeric mode"),
        "unexpected rejection message: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_invalid_jump_target() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry_block = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    module.blocks[entry_block.0 as usize].terminator = HirTerminator::Jump {
        target: crate::compiler_frontend::hir::ids::BlockId(999),
        args: vec![],
    };

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject invalid jump target");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unknown HIR block id"));
}

#[test]
fn validator_rejects_non_literal_match_pattern() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(x, builtin_type_ids::INT, false, None)],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &mut module.blocks[start.entry.0 as usize];
    let local_id = start.params[0];
    let local_ty = entry_block.locals[0].ty;
    let region = entry_block.region;
    let scrutinee_id = HirValueId(9000);
    let pattern_id = HirValueId(9001);

    let value_span = None;
    module
        .side_table
        .map_value(value_span, scrutinee_id, value_span);
    module
        .side_table
        .map_value(value_span, pattern_id, value_span);

    entry_block.terminator = HirTerminator::Match {
        scrutinee: HirExpression {
            id: scrutinee_id,
            kind: HirExpressionKind::Int(1),
            ty: local_ty,
            value_kind: ValueKind::Const,
            region,
            span: None,
        },
        arms: vec![HirMatchArm {
            pattern: HirPattern::Literal(HirExpression {
                id: pattern_id,
                kind: HirExpressionKind::Load(HirPlace::Local(local_id)),
                ty: local_ty,
                value_kind: ValueKind::Place,
                region,
                span: None,
            }),
            guard: None,
            body: start.entry,
        }],
    };

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject non-literal match pattern");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Match literal pattern"));
}

fn inject_fixed_scalar_match_pattern(
    module: &mut HirModule,
    scrutinee_type_id: TypeId,
    pattern_type_id: TypeId,
    pattern_value: FixedScalarValue,
    build_pattern: impl FnOnce(HirExpression) -> HirPattern,
) {
    let start_function = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block_id = start_function.entry;
    let entry_block_index = entry_block_id.0 as usize;
    let region = module.blocks[entry_block_index].region;
    let local_id = LocalId(9000);
    module.blocks[entry_block_index].locals.push(HirLocal {
        id: local_id,
        ty: scrutinee_type_id,
        mutable: false,
        region,
        span: None,
    });

    let scrutinee_id = HirValueId(9000);
    let pattern_id = HirValueId(9001);
    module.side_table.map_value(None, scrutinee_id, None);
    module.side_table.map_value(None, pattern_id, None);

    let scrutinee = HirExpression {
        id: scrutinee_id,
        kind: HirExpressionKind::Load(HirPlace::Local(local_id)),
        ty: scrutinee_type_id,
        value_kind: ValueKind::Place,
        region,
        span: None,
    };
    let pattern_value = HirExpression {
        id: pattern_id,
        kind: HirExpressionKind::FixedScalar(pattern_value),
        ty: pattern_type_id,
        value_kind: ValueKind::Const,
        region,
        span: None,
    };

    module.blocks[entry_block_index].terminator = HirTerminator::Match {
        scrutinee,
        arms: vec![HirMatchArm {
            pattern: build_pattern(pattern_value),
            guard: None,
            body: entry_block_id,
        }],
    };
}

#[test]
fn validator_rejects_fixed_match_literal_type_different_from_scrutinee() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    inject_fixed_scalar_match_pattern(
        &mut module,
        fixed_type(FixedScalar::U8),
        fixed_type(FixedScalar::U16),
        FixedScalarValue::unsigned(FixedScalar::U16, 200).expect("200 fits in U16"),
        HirPattern::Literal,
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject a pattern with a different type from its scrutinee");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_fixed_match_relational_type_different_from_scrutinee() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    inject_fixed_scalar_match_pattern(
        &mut module,
        fixed_type(FixedScalar::U8),
        fixed_type(FixedScalar::U16),
        FixedScalarValue::unsigned(FixedScalar::U16, 10).expect("10 fits in U16"),
        |value| HirPattern::Relational {
            op: HirRelationalPatternOp::LessThan,
            value,
        },
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject a relational pattern with the wrong scalar type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_option_labelled_fixed_scalar_match_pattern() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let u8_type = fixed_type(FixedScalar::U8);
    let option_u8_type = type_environment.intern_option(u8_type);
    inject_fixed_scalar_match_pattern(
        &mut module,
        option_u8_type,
        option_u8_type,
        FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits in U8"),
        |value| HirPattern::OptionValue { value },
    );

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject a fixed payload labelled with the option type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("direct scrutinee type"));
}

#[test]
fn validator_rejects_missing_side_table_mappings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let x = super::symbol("x", &mut path_fork, &mut string_table);
    let authored_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::source_start(),
    ));

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    Expression::int(1, authored_span, ValueMode::ImmutableOwned),
                )),
                authored_span,
            ),
            node(NodeKind::Return(vec![]), authored_span),
        ],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    module.side_table.clear();

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject missing side-table mappings");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("side-table mapping"));
}

#[test]
fn validator_rejects_unresolved_generic_parameter_types() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let parameter_name = string_table.intern("T");
    let generic_type_id =
        type_environment.intern_generic_parameter(GenericParameterId(0), parameter_name);

    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    entry_block.locals.push(HirLocal {
        id: LocalId(9000),
        ty: generic_type_id,
        mutable: false,
        region: entry_block.region,
        span: None,
    });

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter TypeIds");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_collection_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        type_environment.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
                fixed_capacity: None,
            }),
            Box::new([generic_type_id]),
        )
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_option_and_result_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let option_error =
        validation_error_for_injected_local_type(|string_table, type_environment| {
            let generic_type_id = generic_parameter_type_id(string_table, type_environment);
            type_environment.intern_constructed(
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
                Box::new([generic_type_id]),
            )
        });
    assert_eq!(option_error.error_type, ErrorType::HirTransformation);
    assert!(option_error.msg.contains("Unresolved generic parameter"));

    let result_error =
        validation_error_for_injected_local_type(|string_table, type_environment| {
            let generic_type_id = generic_parameter_type_id(string_table, type_environment);
            type_environment.intern_constructed(
                TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
                Box::new([type_environment.builtins().int, generic_type_id]),
            )
        });
    assert_eq!(result_error.error_type, ErrorType::HirTransformation);
    assert!(result_error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_generic_nominal_instance_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        let box_path = PathId::ROOT;
        let (nominal_id, _) = type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: box_path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });

        type_environment.intern_generic_instance(nominal_id, Box::new([generic_type_id]))
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_type_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        type_environment.intern_function(FunctionTypeKey {
            parameters: Box::new([generic_type_id]),
            returns: Box::new([type_environment.builtins().int]),
            error_return: None,
        })
    });

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_struct_field_type_containing_generic_parameter() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);
    let collection_type_id = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([generic_type_id]),
    );

    module.structs.push(HirStruct {
        id: StructId(9000),
        frontend_type_id: type_environment.builtins().int,
        fields: vec![HirField {
            id: FieldId(9000),
            ty: collection_type_id,
        }],
    });

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in HIR field types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_return_type_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let start_index = module
        .start_function
        .expect("normal test module should have start")
        .0 as usize;
    module.functions[start_index].return_type = generic_type_id;

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in return types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_function_parameter_type_containing_generic_parameter() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let value_name = super::symbol("value", &mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                value_name,
                builtin_type_ids::INT,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    entry_block.locals[0].ty = generic_type_id;

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in parameter locals");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_choice_payload_type_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);
    let field_name = string_table.intern("value");

    module.choices.push(HirChoice {
        id: ChoiceId(9000),
        frontend_type_id: type_environment.builtins().int,
        variants: vec![HirChoiceVariant {
            name: string_table.intern("Some"),
            fields: vec![HirChoiceField {
                name: field_name,
                ty: generic_type_id,
            }],
        }],
    });

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in choice payloads");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_expression_type_containing_generic_parameter() {
    let _path_fork = super::PathInternerFork::empty();
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    let value_id = HirValueId(9000);
    let statement_id = HirNodeId(9000);
    let span = None;
    let expression = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Int(1),
        ty: generic_type_id,
        value_kind: ValueKind::Const,
        region: entry_block.region,
        span: None,
    };
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in expression types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_local() {
    let _path_fork = super::PathInternerFork::empty();
    let error = validation_error_for_injected_local_type(|_, type_environment| {
        type_environment.anonymous_const_record_type()
    });
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_expression() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let marker = type_environment.anonymous_const_record_type();
    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    let value_id = HirValueId(9000);
    let statement_id = HirNodeId(9000);
    let span = None;
    let expression = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Int(1),
        ty: marker,
        value_kind: ValueKind::Const,
        region: entry_block.region,
        span: None,
    };
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, value_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject the compile-time marker on an expression");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn validator_rejects_anonymous_const_record_marker_on_function_return() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let marker = type_environment.anonymous_const_record_type();
    let start_function = module
        .start_function
        .expect("normal test module should have start");
    module.functions[start_function.0 as usize].return_type = marker;

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject the compile-time marker on a function return");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn classify_hir_type_rejects_anonymous_const_record_marker() {
    let _path_fork = super::PathInternerFork::empty();
    let type_environment = TypeEnvironment::new();
    let error = classify_hir_type(
        type_environment.anonymous_const_record_type(),
        &type_environment,
    )
    .expect_err("the marker must not classify as a runtime ABI type");
    assert!(
        error
            .msg
            .contains("compile-time-only anonymous const-record marker")
    );
}

#[test]
fn validator_rejects_placeholder_terminator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    module.blocks[entry.0 as usize].terminator = HirTerminator::Uninitialized;

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject placeholder terminators");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("placeholder terminator"));
}

#[test]
fn validator_rejects_region_cycle() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(region_id));

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject cyclic region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("cycle"));
}

#[test]
fn validator_rejects_missing_region_parent() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(RegionId(9999)));

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject missing region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("missing parent"));
}

#[test]
fn validator_rejects_cross_function_cfg_edges() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let helper_name = super::symbol("helper", &mut path_fork, &mut string_table);

    let helper = function_node(
        helper_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![helper, start], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let start_entry = module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry;
    let helper_entry = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .map(|function| function.entry)
        .expect("helper function should exist");

    module.blocks[start_entry.0 as usize].terminator = HirTerminator::Jump {
        target: helper_entry,
        args: vec![],
    };

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject cross-function CFG edges");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("multiple functions") || error.msg.contains("crosses function boundary")
    );
}

#[test]
fn lowering_errors_preserve_string_table_context() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let missing_function = super::symbol("missing_fn", &mut path_fork, &mut string_table);

    let call_span = None;

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::ExpressionStatement(Expression::function_call(
                    missing_function,
                    Vec::new(),
                    Vec::new(),
                    call_span,
                )),
                call_span,
            ),
            node(NodeKind::Return(vec![]), None),
        ],
        None,
    );

    let messages = lower_ast(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("unknown function call should fail HIR lowering");

    let _error = messages
        .infrastructure_error()
        .expect("expected HIR lowering error");
}
// ---------------------------------------------------------------------------
// VariantConstruct validation
// ---------------------------------------------------------------------------

#[test]
fn hir_variant_construct_option_invalid_index_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    let region = entry_block.region;

    let mut type_env = type_environment.clone();
    let int_ty = builtin_type_ids::INT;
    let option_ty = type_env.intern_constructed(
        crate::compiler_frontend::datatypes::ids::TypeConstructor::Builtin(
            crate::compiler_frontend::datatypes::ids::BuiltinTypeConstructor::Option,
        ),
        Box::new([int_ty]),
    );

    let expr_id = HirValueId(9000);
    let stmt_id = HirNodeId(9000);
    let span = None;

    let expression = HirExpression {
        id: expr_id,
        kind: HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Option,
            variant_index: 99,
            fields: vec![],
        },
        ty: option_ty,
        value_kind: ValueKind::Const,
        region,
        span: None,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_env)
        .expect_err("validator should reject out-of-range Option variant index");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("out of range"),
        "expected 'out of range' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_variant_construct_result_invalid_index_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    let region = entry_block.region;

    let mut type_env = type_environment.clone();
    let int_ty = builtin_type_ids::INT;
    let result_ty = type_env.intern_constructed(
        crate::compiler_frontend::datatypes::ids::TypeConstructor::Builtin(
            crate::compiler_frontend::datatypes::ids::BuiltinTypeConstructor::FallibleCarrier,
        ),
        Box::new([int_ty, int_ty]),
    );

    let expr_id = HirValueId(9000);
    let stmt_id = HirNodeId(9000);
    let span = None;

    let expression = HirExpression {
        id: expr_id,
        kind: HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Fallible,
            variant_index: 99,
            fields: vec![],
        },
        ty: result_ty,
        value_kind: ValueKind::Const,
        region,
        span: None,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_env)
        .expect_err("validator should reject out-of-range Result variant index");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("out of range"),
        "expected 'out of range' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_variant_construct_choice_wrong_field_name_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let response_param = super::symbol("response", &mut path_fork, &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let wrong_name = string_table.intern("content");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: path_fork
                        .try_intern_portable_path("message", &mut string_table)
                        .expect("test path fits"),
                    value: no_value_expr(builtin_type_ids::STRING, None, ValueMode::ImmutableOwned),
                    binding_span: None,
                    config_qualifier: None,
                }],
            },
            span: None,
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                response_param,
                builtin_type_ids::NONE,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: path_fork
                .try_intern_portable_path("Response", &mut string_table)
                .expect("test path fits"),
            variants: choice_variants,
        }],
    );
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    let region = entry_block.region;

    let string_ty = builtin_type_ids::STRING;

    let expr_id = HirValueId(9000);
    let stmt_id = HirNodeId(9000);
    let span = None;

    let expression = HirExpression {
        id: expr_id,
        kind: HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice {
                choice_id: ChoiceId(0),
            },
            variant_index: 0,
            fields: vec![HirVariantField {
                name: Some(wrong_name),
                value: HirExpression {
                    id: HirValueId(9001),
                    kind: HirExpressionKind::StringLiteral("hello".to_owned()),
                    ty: string_ty,
                    value_kind: ValueKind::Const,
                    region,
                    span: None,
                },
            }],
        },
        ty: string_ty,
        value_kind: ValueKind::Const,
        region,
        span: None,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject wrong field name in choice VariantConstruct");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("field name"),
        "expected 'field name' in error, got: {}",
        error.msg
    );
}

#[test]
fn hir_variant_construct_choice_wrong_field_type_rejected() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let response_param = super::symbol("response", &mut path_fork, &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let message_name = string_table.intern("message");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: path_fork
                        .try_intern_portable_path("message", &mut string_table)
                        .expect("test path fits"),
                    value: no_value_expr(builtin_type_ids::STRING, None, ValueMode::ImmutableOwned),
                    binding_span: None,
                    config_qualifier: None,
                }],
            },
            span: None,
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            span: None,
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                response_param,
                builtin_type_ids::NONE,
                false,
                None,
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: path_fork
                .try_intern_portable_path("Response", &mut string_table)
                .expect("test path fits"),
            variants: choice_variants,
        }],
    );
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("lowering should succeed");
    let entry_block = &mut module.blocks[module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize];
    let region = entry_block.region;

    let string_ty = builtin_type_ids::STRING;
    let bool_ty = builtin_type_ids::BOOL;

    let expr_id = HirValueId(9000);
    let stmt_id = HirNodeId(9000);
    let span = None;

    let expression = HirExpression {
        id: expr_id,
        kind: HirExpressionKind::VariantConstruct {
            carrier: HirVariantCarrier::Choice {
                choice_id: ChoiceId(0),
            },
            variant_index: 0,
            fields: vec![HirVariantField {
                name: Some(message_name),
                value: HirExpression {
                    id: HirValueId(9001),
                    kind: HirExpressionKind::Bool(true),
                    ty: bool_ty,
                    value_kind: ValueKind::Const,
                    region,
                    span: None,
                },
            }],
        },
        ty: string_ty,
        value_kind: ValueKind::Const,
        region,
        span: None,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_statement(span, &statement);
    module.side_table.map_value(span, expr_id, span);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject wrong field type in choice VariantConstruct");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("field type mismatch"),
        "expected 'field type mismatch' in error, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_collection_expression_with_non_collection_type() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    inject_collection_expression_statement(&mut module, int_type, None);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject Collection expression with non-collection type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("not a collection type"),
        "expected 'not a collection type' in error, got: {}",
        error.msg
    );
}

#[test]
fn validator_accepts_collection_expression_with_growable_collection_type() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    let growable_collection = type_environment.intern_collection(int_type, None);
    inject_collection_expression_statement(&mut module, growable_collection, None);

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept Collection expression with growable collection type");
}

#[test]
fn validator_accepts_collection_expression_with_fixed_collection_type() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    let fixed_collection = type_environment.intern_collection(int_type, Some(64));
    inject_collection_expression_statement(&mut module, fixed_collection, None);

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept Collection expression with fixed collection type");
}

// ---------------------------------------------------------------------------
// Non-finite Float literal invariant
// ---------------------------------------------------------------------------

/// WHAT: injects a `HirExpressionKind::Float(value)` expression into the start entry block and
///       runs HIR validation.
/// WHY: Moth Float stays finite at its profile-selected precision. Non-finite f64 literal
///      payloads breach that invariant and must be rejected before backend lowering.
fn inject_nonfinite_float_expression(
    module: &mut HirModule,
    float_type: TypeId,
    value: f64,
    span: &Option<SourceSpan>,
) {
    let _path_fork = super::PathInternerFork::empty();
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let expression = float_expression(
        HirValueId(9000),
        value,
        float_type,
        entry_region,
        span,
        module,
    );

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span: *span,
    };

    module.side_table.map_statement(*span, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

#[test]
fn validator_rejects_nonfinite_float_literal_infinity() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let float_type = type_environment.builtins().float;
    let span = None;

    inject_nonfinite_float_expression(&mut module, float_type, f64::INFINITY, &span);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject HIR Float literal with INFINITY");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("must be finite"),
        "expected 'must be finite' in error, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_nonfinite_float_literal_nan() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let float_type = type_environment.builtins().float;
    let span = None;

    inject_nonfinite_float_expression(&mut module, float_type, f64::NAN, &span);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject HIR Float literal with NaN");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("must be finite"),
        "expected 'must be finite' in error, got: {}",
        error.msg
    );
}

#[test]
fn validator_rejects_number_literal_scale_that_mismatches_hir_type() {
    let _path_fork = super::PathInternerFork::empty();
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let value_scale = NumberScale::new(2).expect("test scale is valid");
    let value = NumberValue::from_normalized("1.25", NumericLiteralSign::Positive, value_scale)
        .expect("Dec value is exactly representable at its declared scale");
    let number_type = type_environment.intern_number(value_scale);
    let mismatched_type = type_environment.intern_number(NumberScale::ZERO);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let span = None;
    let value_id = HirValueId(9000);
    let expression = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Number(value),
        ty: number_type,
        value_kind: ValueKind::Const,
        region: entry_region,
        span,
    };
    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        span,
    };

    module.side_table.map_value(span, value_id, span);
    module.side_table.map_statement(span, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("matching Dec value and TypeId scales should pass validation");

    let Some(HirStatement {
        kind: HirStatementKind::Expr(expression),
        ..
    }) = module.blocks[entry_block_index].statements.last_mut()
    else {
        panic!("injected Dec expression statement should remain in the HIR block");
    };
    expression.ty = mismatched_type;

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject Dec value/type scale disagreement");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
}

#[test]
fn validator_owns_pending_catch_handler_edges_and_rejects_dangling_or_cross_function_records() {
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

    let source = "double |value Int| -> Int:\nreturn value + 2147483647\n;\n\
                  recover |value Int| -> Int:\nreturn double(value) catch then 0\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect("the direct inferred-failure catch must lower");
    assert_eq!(module.catch_protected_calls.len(), 1);
    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("the pending handler edge must belong to its caller before installation");
    let record = module.catch_protected_calls[0];

    module.catch_protected_calls[0].statement = HirNodeId(u32::MAX);
    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("a dangling pending edge cannot validate");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    module.catch_protected_calls[0] = record;

    let foreign_function = module
        .functions
        .iter()
        .find(|function| function.id != record.owner)
        .expect("the fixture must have a distinct callee");
    module.catch_protected_calls[0].handler.block = foreign_function.entry;
    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("a pending handler cannot enter another function's CFG");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
}
