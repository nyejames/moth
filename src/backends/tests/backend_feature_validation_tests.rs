//! Backend feature validation tests.
//!
//! WHAT: exercises backend-owned unsupported-feature diagnostics over synthetic HIR modules.
//! WHY: backend rejection policy should stay separate from frontend reachability tests while still
//! proving that unreachable HIR helper bodies do not block a backend build.

use crate::backends::backend_feature_validation::{
    BackendFeatureValidationError, BackendFeatureValidationInput,
    validate_hir_backend_feature_support,
};
use crate::backends::external_package_validation::BackendTarget;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, RuleDiagnosticKind, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::reachability::{
    ReachableFloatStatementKind, collect_module_function_link_facts,
    collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

#[test]
fn wasm_feature_validation_rejects_reachable_format_float() {
    let location = location_at(30, 2);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![float_statement(
                10,
                ReachableFloatStatementKind::FormatFloat,
                location,
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm validation should reject reachable FormatFloat",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FloatFormatting,
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_validate_float() {
    let location = location_at(30, 2);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![float_statement(
                10,
                ReachableFloatStatementKind::ValidateFloat,
                location,
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm validation should reject reachable ValidateFloat",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FloatBoundaryValidation,
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_checked_numeric_op() {
    let location = location_at(30, 2);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![numeric_op_statement(10, HirNumericOp::IntAdd, location)],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm validation should reject reachable checked numeric operations",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
    );
}

#[test]
fn wasm_feature_validation_ignores_unreachable_checked_numeric_ops() {
    let location = location_at(50, 4);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![numeric_op_statement(10, HirNumericOp::IntMul, location)],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore unreachable checked numeric operations"
    );
}

#[test]
fn wasm_feature_validation_ignores_unreachable_float_statements() {
    let location = location_at(50, 4);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![float_statement(
                    10,
                    ReachableFloatStatementKind::FormatFloat,
                    location,
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore unreachable float statements"
    );
}

fn wasm_feature_validation_diagnostic(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    expectation: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let reachability = test_reachability(module);
    let error = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(type_environment),
        },
        string_table,
    )
    .expect_err(expectation);

    match error {
        BackendFeatureValidationError::Diagnostic(diagnostic) => *diagnostic,
        BackendFeatureValidationError::Infrastructure(_) => {
            panic!("expected a user-facing Rule diagnostic, not an infrastructure error")
        }
    }
}

fn test_reachability(
    module: &HirModule,
) -> crate::compiler_frontend::hir::reachability::HirReachability {
    let facts = collect_module_function_link_facts(module)
        .expect("test HIR should produce function link facts");
    collect_reachability_from_function_link_facts(
        &facts,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("test HIR should produce entry reachability")
}

fn assert_unsupported_feature(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    string_table: &mut StringTable,
    expected_reason: UnsupportedBackendFeatureReason,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBackendFeature)
    );

    let DiagnosticPayload::UnsupportedBackendFeature {
        backend_name,
        reason,
    } = &diagnostic.payload
    else {
        panic!("expected UnsupportedBackendFeature payload");
    };

    assert_eq!(*backend_name, string_table.intern("Wasm"));
    assert_eq!(*reason, expected_reason);
}

fn hir_module(
    start_function: FunctionId,
    functions: Vec<HirFunction>,
    blocks: Vec<HirBlock>,
) -> HirModule {
    let mut module = HirModule::new();
    module.start_function = Some(start_function);
    module.functions = functions;
    module.blocks = blocks;
    module
}

fn function(id: FunctionId, entry: BlockId) -> HirFunction {
    HirFunction {
        id,
        entry,
        params: vec![],
        return_type: builtin_type_ids::NONE,
    }
}

fn block(id: BlockId, statements: Vec<HirStatement>, terminator: HirTerminator) -> HirBlock {
    HirBlock {
        id,
        region: RegionId(0),
        locals: vec![],
        statements,
        terminator,
    }
}

fn numeric_op_statement(id: u32, op: HirNumericOp, location: SourceLocation) -> HirStatement {
    let failure_mode = NumericFailureMode::Trap;
    let left = HirExpression {
        id: HirValueId(id + 100),
        kind: HirExpressionKind::Int(1),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    };
    let right = HirExpression {
        id: HirValueId(id + 101),
        kind: HirExpressionKind::Int(2),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    };
    let result = LocalId(9000);

    HirStatement {
        id: HirNodeId(id),
        kind: HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands: HirNumericOperands::Binary { left, right },
            result,
        },
        location,
    }
}

fn float_statement(
    id: u32,
    kind: ReachableFloatStatementKind,
    location: SourceLocation,
) -> HirStatement {
    let failure_mode = NumericFailureMode::Trap;
    let source = HirExpression {
        id: HirValueId(id + 100),
        kind: HirExpressionKind::Float(1.5),
        ty: builtin_type_ids::FLOAT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    };
    let result = LocalId(9000);

    HirStatement {
        id: HirNodeId(id),
        kind: match kind {
            ReachableFloatStatementKind::FormatFloat => HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            },
            ReachableFloatStatementKind::ValidateFloat => HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            },
        },
        location,
    }
}

fn unit_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::TupleConstruct { elements: vec![] },
        ty: builtin_type_ids::NONE,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
    }
}

fn location_at(line_number: i32, char_column: i32) -> SourceLocation {
    SourceLocation {
        start_pos: CharPosition {
            line_number,
            char_column,
        },
        end_pos: CharPosition {
            line_number,
            char_column: char_column + 1,
        },
        ..SourceLocation::default()
    }
}
