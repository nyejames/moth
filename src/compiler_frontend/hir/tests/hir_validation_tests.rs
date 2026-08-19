//! HIR validation regression tests.
//!
//! WHAT: exercises the post-lowering HIR validator against valid and intentionally broken modules.
//! WHY: validator coverage needs focused tests that isolate invariants from the rest of lowering.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind, SourceLocation};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::{AstDocFragment, AstDocFragmentKind};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericParameterId, NominalTypeId, TypeConstructor,
    TypeId, builtin_type_ids,
};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{
    HirTestChoiceDefinition, build_ast, build_ast_with_choices, lower_ast, lower_ast_with_metadata,
    validate_module_for_tests,
};
use crate::compiler_frontend::hir::ids::{
    ChoiceId, FieldId, HirNodeId, HirValueId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::{
    HirChoice, HirChoiceField, HirChoiceVariant, HirModule,
};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tests::type_id_fixture_support::no_value_expr;

use crate::compiler_frontend::value_mode::ValueMode;

fn node(kind: NodeKind, location: SourceLocation) -> AstNode {
    AstNode {
        kind,
        location,
        scope: InternedPath::new(),
    }
}

fn make_test_variable(name: InternedPath, value: Expression) -> Declaration {
    Declaration { id: name, value }
}

fn param(
    name: InternedPath,
    type_id: TypeId,
    mutable: bool,
    location: SourceLocation,
) -> Declaration {
    crate::compiler_frontend::tests::type_id_fixture_support::param_declaration(
        name, type_id, mutable, location,
    )
}

fn function_node(
    name: InternedPath,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    location: SourceLocation,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), location)
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");

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
        source_info: Some(test_source_location(20)),
    });

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter inside TypeId")
}

fn inject_collection_expression_statement(
    module: &mut HirModule,
    collection_type_id: TypeId,
    location: SourceLocation,
) {
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
    };

    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, value_id, &location);
    entry_block.statements.push(statement);
}

fn int_expression(
    id: HirValueId,
    value: i32,
    type_id: TypeId,
    region: RegionId,
    location: &SourceLocation,
    module: &mut HirModule,
) -> HirExpression {
    module.side_table.map_value(location, id, location);

    HirExpression {
        id,
        kind: HirExpressionKind::Int(value),
        ty: type_id,
        value_kind: ValueKind::RValue,
        region,
    }
}

fn float_expression(
    id: HirValueId,
    value: f64,
    type_id: TypeId,
    region: RegionId,
    location: &SourceLocation,
    module: &mut HirModule,
) -> HirExpression {
    module.side_table.map_value(location, id, location);

    HirExpression {
        id,
        kind: HirExpressionKind::Float(value),
        ty: type_id,
        value_kind: ValueKind::RValue,
        region,
    }
}

#[test]
fn valid_module_passes_explicit_validation() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept a valid lowered module");
}

#[test]
fn validator_rejects_numeric_op_operand_shape_mismatch() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(50);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;
    let result_local = LocalId(9000);

    module.blocks[entry_block_index].locals.push(HirLocal {
        id: result_local,
        ty: int_type,
        mutable: false,
        region: entry_region,
        source_info: Some(location.clone()),
    });

    let left = int_expression(
        HirValueId(9000),
        1,
        int_type,
        entry_region,
        &location,
        &mut module,
    );
    let right = int_expression(
        HirValueId(9001),
        2,
        int_type,
        entry_region,
        &location,
        &mut module,
    );

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp::IntNeg,
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary { left, right },
            result: result_local,
        },
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
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
fn validator_rejects_plain_numeric_binop() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(51);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;

    let left = int_expression(
        HirValueId(9000),
        1,
        int_type,
        entry_region,
        &location,
        &mut module,
    );
    let right = int_expression(
        HirValueId(9001),
        2,
        int_type,
        entry_region,
        &location,
        &mut module,
    );

    let expression = HirExpression {
        id: HirValueId(9002),
        kind: HirExpressionKind::BinOp {
            op: HirBinOp::Add,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: int_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
    };
    module
        .side_table
        .map_value(&location, expression.id, &location);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject plain numeric BinOp");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "Plain HirBinOp::Add arithmetic must be lowered through HirStatementKind::NumericOp"
    ));
}

fn append_expression_for_validation(
    module: &mut HirModule,
    location: &SourceLocation,
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
    };
    module.side_table.map_value(location, left.id, location);

    let right = int_expression(
        HirValueId(9011),
        7,
        int_type,
        entry_region,
        location,
        module,
    );
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
    };
    module
        .side_table
        .map_value(location, expression.id, location);
    expression
}

#[test]
fn validator_accepts_internal_string_append_with_scalar_chunk() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(53);
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &location,
        string_type,
        string_type,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9013),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
    let entry_block_index = start_entry_block_index(&module);
    module.blocks[entry_block_index].statements.push(statement);

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("valid StringAppend should pass HIR validation");
}

#[test]
fn validator_rejects_string_append_with_non_string_result() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(54);
    let string_type = type_environment.builtins().string;
    let expression = append_expression_for_validation(
        &mut module,
        &location,
        string_type,
        type_environment.builtins().int,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9014),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(55);
    let expression = append_expression_for_validation(
        &mut module,
        &location,
        type_environment.builtins().int,
        type_environment.builtins().string,
        type_environment.builtins().int,
    );
    let statement = HirStatement {
        id: HirNodeId(9015),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(52);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let int_type = type_environment.builtins().int;

    let operand = int_expression(
        HirValueId(9000),
        1,
        int_type,
        entry_region,
        &location,
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
    };
    module
        .side_table
        .map_value(&location, expression.id, &location);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
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

#[test]
fn validator_rejects_plain_string_concatenation_binop() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(53);
    let entry_block_index = start_entry_block_index(&module);
    let entry_region = module.blocks[entry_block_index].region;
    let string_type = type_environment.builtins().string;

    let left = HirExpression {
        id: HirValueId(9000),
        kind: HirExpressionKind::StringLiteral("a".to_owned()),
        ty: string_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
    };
    module.side_table.map_value(&location, left.id, &location);
    let right = HirExpression {
        id: HirValueId(9001),
        kind: HirExpressionKind::StringLiteral("b".to_owned()),
        ty: string_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
    };
    module.side_table.map_value(&location, right.id, &location);

    let expression = HirExpression {
        id: HirValueId(9002),
        kind: HirExpressionKind::BinOp {
            op: HirBinOp::Add,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty: string_type,
        value_kind: ValueKind::RValue,
        region: entry_region,
    };
    module
        .side_table
        .map_value(&location, expression.id, &location);

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };
    module.side_table.map_statement(&location, &statement);
    module.blocks[entry_block_index].statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject plain string BinOp");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "Plain HirBinOp::Add arithmetic must be lowered through HirStatementKind::NumericOp"
    ));
}

fn inject_float_statement(
    module: &mut HirModule,
    type_environment: &TypeEnvironment,
    location: &SourceLocation,
    kind: HirStatementKind,
    result_type: TypeId,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let result_local = LocalId(9000);

    let source = float_expression(
        HirValueId(9000),
        1.5,
        type_environment.builtins().float,
        entry_region,
        location,
        module,
    );

    {
        let entry_block = &mut module.blocks[entry_block_index];
        entry_block.locals.push(HirLocal {
            id: result_local,
            ty: result_type,
            mutable: false,
            region: entry_region,
            source_info: Some(location.clone()),
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
        location: location.clone(),
    };

    module.side_table.map_statement(location, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

#[test]
fn validator_accepts_format_float_trap() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(54);
    let string_type = type_environment.builtins().string;

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(55);
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::ValidateFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(56);
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(57);
    let string_type = type_environment.builtins().string;
    let int_type = type_environment.builtins().int;
    let carrier_type = type_environment.intern_fallible_carrier(string_type, int_type);

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(58);
    let string_type = type_environment.builtins().string;

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::FormatFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let location = test_source_location(59);
    let float_type = type_environment.builtins().float;

    inject_float_statement(
        &mut module,
        &type_environment,
        &location,
        HirStatementKind::ValidateFloat {
            source: HirExpression {
                id: HirValueId(0),
                kind: HirExpressionKind::Float(0.0),
                ty: type_environment.builtins().float,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
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
fn validator_rejects_invalid_jump_target() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param(
                x.clone(),
                builtin_type_ids::INT,
                false,
                test_source_location(2),
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(3))],
        test_source_location(2),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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

    let value_location = test_source_location(20);
    module
        .side_table
        .map_value(&value_location, scrutinee_id, &value_location);
    module
        .side_table
        .map_value(&value_location, pattern_id, &value_location);

    entry_block.terminator = HirTerminator::Match {
        scrutinee: HirExpression {
            id: scrutinee_id,
            kind: HirExpressionKind::Int(1),
            ty: local_ty,
            value_kind: ValueKind::Const,
            region,
        },
        arms: vec![HirMatchArm {
            pattern: HirPattern::Literal(HirExpression {
                id: pattern_id,
                kind: HirExpressionKind::Load(HirPlace::Local(local_id)),
                ty: local_ty,
                value_kind: ValueKind::Place,
                region,
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

#[test]
fn validator_rejects_missing_side_table_mappings() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let x = super::symbol("x", &mut string_table);

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
                    Expression::int(1, test_source_location(4), ValueMode::ImmutableOwned),
                )),
                test_source_location(4),
            ),
            node(NodeKind::Return(vec![]), test_source_location(5)),
        ],
        test_source_location(3),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
    module.side_table.clear();

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject missing side-table mappings");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("side-table mapping"));
}

#[test]
fn validator_rejects_unresolved_generic_parameter_types() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
        source_info: Some(test_source_location(20)),
    });

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter TypeIds");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn validator_rejects_collection_containing_generic_parameter() {
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
    let error = validation_error_for_injected_local_type(|string_table, type_environment| {
        let generic_type_id = generic_parameter_type_id(string_table, type_environment);
        let box_path = InternedPath::from_single_str("Box", string_table);
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let value_name = super::symbol("value", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param(
                value_name,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(2))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, mut type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let (mut string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let generic_type_id = generic_parameter_type_id(&mut string_table, &mut type_environment);

    let entry_block_index = start_entry_block_index(&module);
    let entry_block = &mut module.blocks[entry_block_index];
    let value_id = HirValueId(9000);
    let statement_id = HirNodeId(9000);
    let location = test_source_location(20);
    let expression = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Int(1),
        ty: generic_type_id,
        value_kind: ValueKind::Const,
        region: entry_block.region,
    };
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, value_id, &location);
    entry_block.statements.push(statement);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject unresolved generic parameter in expression types");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unresolved generic parameter"));
}

#[test]
fn module_metadata_validation_rejects_invalid_doc_fragment_location() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let mut ast = build_ast(vec![start_fn], entry_path);
    let mut invalid_location = test_source_location(10);
    invalid_location.end_pos.line_number = 9;
    ast.doc_fragments.push(AstDocFragment {
        kind: AstDocFragmentKind::Doc,
        value: string_table.intern("broken"),
        location: invalid_location,
    });

    let lowering =
        lower_ast_with_metadata(ast, &mut string_table).expect("HIR lowering should succeed");
    let error = lowering
        .metadata
        .validate()
        .expect_err("metadata validation should reject invalid doc fragment locations");
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("Doc fragment"));
}

#[test]
fn validator_rejects_placeholder_terminator() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(region_id));

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject cyclic region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("cycle"));
}

#[test]
fn validator_rejects_missing_region_parent() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
    let region_id = module.regions[0].id();
    module.regions[0] = HirRegion::lexical(region_id, Some(RegionId(9999)));

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject missing region parents");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("missing parent"));
}

#[test]
fn validator_rejects_cross_function_cfg_edges() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let helper_name = super::symbol("helper", &mut string_table);

    let helper = function_node(
        helper_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(2))],
        test_source_location(2),
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![helper, start], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let missing_function = super::symbol("missing_fn", &mut string_table);

    let mut call_location = test_source_location(2);
    call_location.scope = entry_path.clone();

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
                    call_location.clone(),
                )),
                call_location.clone(),
            ),
            node(NodeKind::Return(vec![]), test_source_location(3)),
        ],
        test_source_location(1),
    );

    let messages = lower_ast(build_ast(vec![start_fn], entry_path), &mut string_table)
        .expect_err("unknown function call should fail HIR lowering");

    let resolved_scope = messages
        .first_error()
        .expect("expected HIR lowering error")
        .primary_location
        .scope
        .to_portable_string(&messages.string_table);
    assert!(
        resolved_scope.ends_with("main.moth"),
        "HIR lowering errors should preserve the source path in the returned StringTable, got '{resolved_scope}'",
    );
}

// ---------------------------------------------------------------------------
// VariantConstruct validation
// ---------------------------------------------------------------------------

#[test]
fn hir_variant_construct_option_invalid_index_rejected() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let location = test_source_location(10);

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
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, expr_id, &location);
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(1))],
        test_source_location(1),
    );

    let ast = build_ast(vec![start_fn], entry_path);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let location = test_source_location(10);

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
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, expr_id, &location);
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let response_param = super::symbol("response", &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let wrong_name = string_table.intern("content");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: InternedPath::from_single_str("message", &mut string_table),
                    value: no_value_expr(
                        builtin_type_ids::STRING,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                }],
            },
            location: test_source_location(2),
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_source_location(2),
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param(
                response_param,
                builtin_type_ids::NONE,
                false,
                test_source_location(2),
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(3))],
        test_source_location(1),
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: InternedPath::from_single_str("Response", &mut string_table),
            variants: choice_variants,
        }],
    );
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let location = test_source_location(10);

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
                },
            }],
        },
        ty: string_ty,
        value_kind: ValueKind::Const,
        region,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, expr_id, &location);
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
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let response_param = super::symbol("response", &mut string_table);
    let ok_name = string_table.intern("Ok");
    let err_name = string_table.intern("Err");
    let message_name = string_table.intern("message");

    let choice_variants = vec![
        ChoiceVariant {
            id: ok_name,
            payload: ChoiceVariantPayload::Record {
                fields: vec![Declaration {
                    id: InternedPath::from_single_str("message", &mut string_table),
                    value: no_value_expr(
                        builtin_type_ids::STRING,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                }],
            },
            location: test_source_location(2),
        },
        ChoiceVariant {
            id: err_name,
            payload: ChoiceVariantPayload::Unit,
            location: test_source_location(2),
        },
    ];

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![param(
                response_param,
                builtin_type_ids::NONE,
                false,
                test_source_location(2),
            )],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(3))],
        test_source_location(1),
    );

    let ast = build_ast_with_choices(
        vec![start_fn],
        entry_path,
        vec![HirTestChoiceDefinition {
            nominal_path: InternedPath::from_single_str("Response", &mut string_table),
            variants: choice_variants,
        }],
    );
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("lowering should succeed");
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
    let location = test_source_location(10);

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
                },
            }],
        },
        ty: string_ty,
        value_kind: ValueKind::Const,
        region,
    };

    let statement = HirStatement {
        id: stmt_id,
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(&location, &statement);
    module.side_table.map_value(&location, expr_id, &location);
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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    inject_collection_expression_statement(&mut module, int_type, test_source_location(20));

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
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    let growable_collection = type_environment.intern_collection(int_type, None);
    inject_collection_expression_statement(
        &mut module,
        growable_collection,
        test_source_location(20),
    );

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept Collection expression with growable collection type");
}

#[test]
fn validator_accepts_collection_expression_with_fixed_collection_type() {
    let (string_table, mut module, mut type_environment) = minimal_lowered_hir_module();
    let int_type = type_environment.builtins().int;
    let fixed_collection = type_environment.intern_collection(int_type, Some(64));
    inject_collection_expression_statement(&mut module, fixed_collection, test_source_location(20));

    validate_module_for_tests(&module, &string_table, &type_environment)
        .expect("validator should accept Collection expression with fixed collection type");
}

// ---------------------------------------------------------------------------
// Non-finite Float literal invariant
// ---------------------------------------------------------------------------

/// WHAT: injects a `HirExpressionKind::Float(value)` expression into the start entry block and
///       runs HIR validation.
/// WHY: non-finite HIR Float literals violate the `Float = finite f64` language contract. The
///      validator must reject them as internal invariant breaches before any backend lowers them.
fn inject_nonfinite_float_expression(
    module: &mut HirModule,
    float_type: TypeId,
    value: f64,
    location: &SourceLocation,
) {
    let entry_block_index = start_entry_block_index(module);
    let entry_region = module.blocks[entry_block_index].region;
    let expression = float_expression(
        HirValueId(9000),
        value,
        float_type,
        entry_region,
        location,
        module,
    );

    let statement = HirStatement {
        id: HirNodeId(9000),
        kind: HirStatementKind::Expr(expression),
        location: location.clone(),
    };

    module.side_table.map_statement(location, &statement);
    module.blocks[entry_block_index].statements.push(statement);
}

#[test]
fn validator_rejects_nonfinite_float_literal_infinity() {
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let float_type = type_environment.builtins().float;
    let location = test_source_location(60);

    inject_nonfinite_float_expression(&mut module, float_type, f64::INFINITY, &location);

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
    let (string_table, mut module, type_environment) = minimal_lowered_hir_module();
    let float_type = type_environment.builtins().float;
    let location = test_source_location(61);

    inject_nonfinite_float_expression(&mut module, float_type, f64::NAN, &location);

    let error = validate_module_for_tests(&module, &string_table, &type_environment)
        .expect_err("validator should reject HIR Float literal with NaN");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error.msg.contains("must be finite"),
        "expected 'must be finite' in error, got: {}",
        error.msg
    );
}
