//! Native result-channel validation invariants, including edge-local definitions.

use super::hir_validation_tests::builtin_error_type_id;
use super::minimal_lowered_hir_module;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::hir_builder::validate_module_for_tests;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::hir_fixture_support::expression;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

struct InvokeFixture {
    strings: StringTable,
    module: HirModule,
    types: TypeEnvironment,
    entry: BlockId,
    success: BlockId,
    failure: BlockId,
    success_local: LocalId,
    error_local: LocalId,
    next_node_id: u32,
}

impl InvokeFixture {
    fn validate(&self) -> Result<(), CompilerError> {
        validate_module_for_tests(
            &self.module,
            &self.strings,
            &self.types,
            &ExternalPackageRegistry::new(),
        )
    }

    fn invoke(&mut self) -> &mut HirTerminator {
        &mut self.module.blocks[self.entry.0 as usize].terminator
    }

    fn read_local(&mut self, block: BlockId, local: LocalId, ty: TypeId) {
        let region = self.module.blocks[block.0 as usize].region;
        let value = expression(
            HirExpressionKind::Load(HirPlace::local(local)),
            ty,
            region,
            ValueKind::RValue,
            &mut self.module.expressions,
        );
        self.module.side_table.map_value(None, value);
        let id = HirNodeId(self.next_node_id);
        self.next_node_id += 1;
        self.module.blocks[block.0 as usize]
            .statements
            .push(HirStatement {
                id,
                kind: HirStatementKind::Expr(value),
                span: None,
            });
    }
}

fn format_float_invoke() -> InvokeFixture {
    let (strings, mut module, mut types) = minimal_lowered_hir_module();
    let function = module.start_function.expect("fixture has a start function");
    let entry = module.functions[function.0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let error_type = builtin_error_type_id(&mut types);
    let source = expression(
        HirExpressionKind::FixedScalar(
            FixedScalarValue::binary_float(FixedScalar::F64, 1.5)
                .expect("1.5 is exactly representable in F64"),
        ),
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        region,
        ValueKind::Const,
        &mut module.expressions,
    );
    module.side_table.map_value(None, source);
    let success_local = LocalId(9000);
    let error_local = LocalId(9001);
    let success = BlockId(module.blocks.len() as u32);
    let failure = BlockId(success.0 + 1);
    for (block, local, ty) in [
        (success, success_local, builtin_type_ids::STRING),
        (failure, error_local, error_type),
    ] {
        module.blocks.push(HirBlock {
            id: block,
            region,
            locals: vec![HirLocal {
                id: local,
                ty,
                mutable: false,
                region,
                span: None,
            }],
            statements: vec![],
            terminator: HirTerminator::Return(HirValueRange::empty()),
        });
    }
    module.blocks[entry.0 as usize].terminator = HirTerminator::Invoke {
        id: HirNodeId(9000),
        span: None,
        operation: HirFallibleOperation::FormatFloat { source },
        success,
        success_results: vec![success_local],
        failure,
        error_result: error_local,
    };
    InvokeFixture {
        strings,
        module,
        types,
        entry,
        success,
        failure,
        success_local,
        error_local,
        next_node_id: 9001,
    }
}

#[test]
fn validator_accepts_format_float_invoke_and_each_selected_edge_definition() {
    let mut fixture = format_float_invoke();
    fixture.read_local(
        fixture.success,
        fixture.success_local,
        builtin_type_ids::STRING,
    );
    let error_type = builtin_error_type_id(&mut fixture.types);
    fixture.read_local(fixture.failure, fixture.error_local, error_type);
    fixture
        .validate()
        .expect("native float formatting defines only its selected edge result");
}

#[test]
fn validator_rejects_duplicate_invoke_success_destinations() {
    let mut fixture = format_float_invoke();
    if let HirTerminator::Invoke {
        success_results, ..
    } = fixture.invoke()
    {
        success_results.push(success_results[0]);
    }
    let error = fixture
        .validate()
        .expect_err("success destinations must be unique");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Invoke success destinations must be unique")
    );
}

#[test]
fn validator_rejects_overlapping_invoke_success_and_error_destinations() {
    let mut fixture = format_float_invoke();
    if let HirTerminator::Invoke {
        success_results,
        error_result,
        ..
    } = fixture.invoke()
    {
        *error_result = success_results[0];
    }
    let error = fixture
        .validate()
        .expect_err("success and error definitions must be disjoint");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Invoke success/error destinations must be disjoint")
    );
}

#[test]
fn validator_rejects_invoke_success_arity_mismatch() {
    let mut fixture = format_float_invoke();
    if let HirTerminator::Invoke {
        success_results, ..
    } = fixture.invoke()
    {
        success_results.clear();
    }
    let error = fixture
        .validate()
        .expect_err("float formatting requires one success slot");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Invoke operation success slot arity is invalid")
    );
}

#[test]
fn validator_rejects_success_result_use_on_failure_edge() {
    let mut fixture = format_float_invoke();
    fixture.read_local(
        fixture.failure,
        fixture.success_local,
        builtin_type_ids::STRING,
    );
    let error = fixture
        .validate()
        .expect_err("failure does not define success locals");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("not definitely defined"));
}

#[test]
fn validator_rejects_error_result_use_on_success_edge() {
    let mut fixture = format_float_invoke();
    let error_type = builtin_error_type_id(&mut fixture.types);
    fixture.read_local(fixture.success, fixture.error_local, error_type);
    let error = fixture
        .validate()
        .expect_err("success does not define the error local");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("not definitely defined"));
}

fn module_with_ordered_returns() -> (
    StringTable,
    HirModule,
    TypeEnvironment,
    BlockId,
    Vec<HirValueId>,
) {
    let (strings, mut module, types) = minimal_lowered_hir_module();
    let function = module.start_function.expect("fixture has a start function");
    let entry = module.functions[function.0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    module.functions[function.0 as usize].returns =
        vec![integer_type, builtin_type_ids::BOOL].into_boxed_slice();
    let integer = expression(
        HirExpressionKind::FixedScalar(
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32"),
        ),
        integer_type,
        region,
        ValueKind::Const,
        &mut module.expressions,
    );
    let boolean = expression(
        HirExpressionKind::Bool(true),
        builtin_type_ids::BOOL,
        region,
        ValueKind::Const,
        &mut module.expressions,
    );
    for value in [integer, boolean] {
        module.side_table.map_value(None, value);
    }
    (strings, module, types, entry, vec![integer, boolean])
}

#[test]
fn validator_accepts_ordered_multiple_success_return_values() {
    let (strings, mut module, types, entry, values) = module_with_ordered_returns();
    let values = module
        .expressions
        .append_values(&values, None)
        .expect("native return values fit the expression store");
    module.blocks[entry.0 as usize].terminator = HirTerminator::Return(values);
    validate_module_for_tests(&module, &strings, &types, &ExternalPackageRegistry::new())
        .expect("ordered native values match both declared success slots");
}

#[test]
fn validator_rejects_multiple_success_return_arity_mismatch() {
    let (strings, mut module, types, entry, mut values) = module_with_ordered_returns();
    values.pop();
    let values = module
        .expressions
        .append_values(&values, None)
        .expect("native return values fit the expression store");
    module.blocks[entry.0 as usize].terminator = HirTerminator::Return(values);
    let error =
        validate_module_for_tests(&module, &strings, &types, &ExternalPackageRegistry::new())
            .expect_err("each declared success slot requires one value");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Return success slot arity does not match function signature")
    );
}

#[test]
fn validator_rejects_reordered_multiple_success_return_types() {
    let (strings, mut module, types, entry, mut values) = module_with_ordered_returns();
    values.reverse();
    let values = module
        .expressions
        .append_values(&values, None)
        .expect("native return values fit the expression store");
    module.blocks[entry.0 as usize].terminator = HirTerminator::Return(values);
    let error =
        validate_module_for_tests(&module, &strings, &types, &ExternalPackageRegistry::new())
            .expect_err("return types match corresponding slots, not an unordered type set");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Return success slot type does not match function signature")
    );
}

fn map_invoke(op: HirMapOp, key_type: TypeId, value_type: TypeId) -> InvokeFixture {
    let (strings, mut module, mut types) = minimal_lowered_hir_module();
    let function = module.start_function.expect("fixture has a start function");
    let entry = module.functions[function.0 as usize].entry;
    let region = module.blocks[entry.0 as usize].region;
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let map_type = types.intern_map(integer_type, builtin_type_ids::BOOL);
    let error_type = builtin_error_type_id(&mut types);
    let receiver_local = LocalId(9002);
    module.functions[function.0 as usize]
        .params
        .push(receiver_local);
    module.blocks[entry.0 as usize].locals.push(HirLocal {
        id: receiver_local,
        ty: map_type,
        mutable: true,
        region,
        span: None,
    });
    let receiver = expression(
        HirExpressionKind::Load(HirPlace::local(receiver_local)),
        map_type,
        region,
        ValueKind::Place,
        &mut module.expressions,
    );
    let key_local = LocalId(9003);
    let value_local = LocalId(9004);
    let mut args = Vec::new();
    for (local, ty) in [(key_local, key_type), (value_local, value_type)] {
        if local == value_local && op != HirMapOp::Set {
            continue;
        }
        module.functions[function.0 as usize].params.push(local);
        module.blocks[entry.0 as usize].locals.push(HirLocal {
            id: local,
            ty,
            mutable: false,
            region,
            span: None,
        });
        args.push(expression(
            HirExpressionKind::Load(HirPlace::local(local)),
            ty,
            region,
            ValueKind::RValue,
            &mut module.expressions,
        ));
    }
    let args = module
        .expressions
        .append_values(&args, None)
        .expect("map operands fit the expression store");
    let success_local = LocalId(9000);
    let error_local = LocalId(9001);
    let success = BlockId(module.blocks.len() as u32);
    let failure = BlockId(success.0 + 1);
    for (block, local, ty) in [
        (success, success_local, builtin_type_ids::BOOL),
        (failure, error_local, error_type),
    ] {
        module.blocks.push(HirBlock {
            id: block,
            region,
            locals: vec![HirLocal {
                id: local,
                ty,
                mutable: false,
                region,
                span: None,
            }],
            statements: vec![],
            terminator: HirTerminator::Return(HirValueRange::empty()),
        });
    }
    module.blocks[entry.0 as usize].terminator = HirTerminator::Invoke {
        id: HirNodeId(9000),
        span: None,
        operation: HirFallibleOperation::MapOp { op, receiver, args },
        success,
        success_results: if op == HirMapOp::Set {
            vec![]
        } else {
            vec![success_local]
        },
        failure,
        error_result: error_local,
    };
    InvokeFixture {
        strings,
        module,
        types,
        entry,
        success,
        failure,
        success_local,
        error_local,
        next_node_id: 9001,
    }
}

#[test]
fn validator_accepts_native_map_get_remove_and_zero_success_set() {
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    for op in [HirMapOp::Get, HirMapOp::Remove, HirMapOp::Set] {
        let mut fixture = map_invoke(op, integer_type, builtin_type_ids::BOOL);
        if op != HirMapOp::Set {
            fixture.read_local(
                fixture.success,
                fixture.success_local,
                builtin_type_ids::BOOL,
            );
        }
        fixture
            .validate()
            .expect("native map operation preserves its exact result signature");
    }
}

#[test]
fn validator_rejects_native_map_get_success_arity_and_type_mismatches() {
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut fixture = map_invoke(HirMapOp::Get, integer_type, builtin_type_ids::BOOL);
    if let HirTerminator::Invoke {
        success_results, ..
    } = fixture.invoke()
    {
        success_results.clear();
    }
    let error = fixture
        .validate()
        .expect_err("Get requires one success slot");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "Builtin operation argument/result slot arity does not match semantic signature"
    ));

    let mut fixture = map_invoke(HirMapOp::Get, integer_type, builtin_type_ids::BOOL);
    fixture.module.blocks[fixture.success.0 as usize].locals[0].ty = integer_type;
    let error = fixture
        .validate()
        .expect_err("Get returns the map value type");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Builtin operation result type does not match semantic signature")
    );
}

#[test]
fn validator_rejects_native_map_set_success_slot_and_wrong_error_type() {
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut fixture = map_invoke(HirMapOp::Set, integer_type, builtin_type_ids::BOOL);
    let success_local = fixture.success_local;
    if let HirTerminator::Invoke {
        success_results, ..
    } = fixture.invoke()
    {
        success_results.push(success_local);
    }
    let error = fixture.validate().expect_err("Set has no success values");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains(
        "Builtin operation argument/result slot arity does not match semantic signature"
    ));

    let mut fixture = map_invoke(HirMapOp::Set, integer_type, builtin_type_ids::BOOL);
    fixture.module.blocks[fixture.failure.0 as usize].locals[0].ty = builtin_type_ids::STRING;
    let error = fixture
        .validate()
        .expect_err("map failure returns builtin Error");
    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(
        error
            .msg
            .contains("Invoke builtin error destination has the wrong error type")
    );
}

#[test]
fn validator_rejects_native_map_key_and_set_value_type_mismatches() {
    let integer_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    for (op, key_type, value_type) in [
        (
            HirMapOp::Get,
            builtin_type_ids::BOOL,
            builtin_type_ids::BOOL,
        ),
        (HirMapOp::Set, integer_type, integer_type),
    ] {
        let fixture = map_invoke(op, key_type, value_type);
        let error = fixture
            .validate()
            .expect_err("map operands match exact key/value types");
        assert_eq!(error.error_type, ErrorType::HirTransformation);
        assert!(
            error
                .msg
                .contains("Builtin operation argument type does not match semantic signature")
        );
    }
}
