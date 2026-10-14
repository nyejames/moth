//! Numeric proof invariants over real HIR blocks.
//!
//! Tests assemble HIR blocks with ordinary statements and native Invoke continuations, then
//! query proofs by operation id exactly as the scalar backends do.
//!
//! Fixture calls are always bound to locals before being combined: the helpers take `&mut
//! fixture`, so nested calls in one expression would fight over the borrow.

use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::hir_fixture_support::{expression, local};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One analysis fixture: a fresh type environment, id counters and the module under build.
struct ProofFixture {
    environment: TypeEnvironment,
    module: HirModule,
    path_builder: PathInternerBuilder,
    string_table: StringTable,
    next_statement_id: u32,
}

impl ProofFixture {
    fn new() -> Self {
        Self {
            environment: TypeEnvironment::new(),
            module: HirModule::new(),
            path_builder: PathInternerBuilder::new(),
            string_table: StringTable::new(),
            next_statement_id: 0,
        }
    }

    fn path(&mut self, spelling: &str) -> PathId {
        self.path_builder
            .try_intern_portable_path(spelling, &mut self.string_table)
            .expect("test path fits")
    }

    fn int(&self) -> TypeId {
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    }

    fn uint(&self) -> TypeId {
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    }

    fn uint_literal(&mut self, value: u64, ty: TypeId) -> HirValueId {
        let value = FixedScalarValue::unsigned(FixedScalar::U32, value).expect("literal fits U32");
        self.fixed_literal(value, ty)
    }

    fn float(&self) -> TypeId {
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    }

    fn fixed(&self, scalar: FixedScalar) -> TypeId {
        builtin_type_ids::fixed_scalar(scalar)
    }

    fn number(&mut self) -> TypeId {
        self.environment.intern_number(NumberScale::ZERO)
    }

    /// The builtin `Error` type id, registered through the same canonical-identity dance the
    /// HIR validation fixtures use, because `TypeEnvironment::new()` does not seed it.
    fn error_type(&mut self) -> TypeId {
        let error_identity = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error);
        if let Some(error_type_id) = self
            .environment
            .type_id_for_canonical_identity(&error_identity)
        {
            return error_type_id;
        }

        let (_, error_type_id) = self
            .environment
            .register_nominal_struct(StructTypeDefinition {
                id: NominalTypeId(0),
                path: PathId::ROOT,
                fields: Box::new([]),
                generic_parameters: None,
                const_record: false,
            });
        self.environment
            .register_canonical_identity(error_identity, error_type_id)
            .expect("test builtin Error identity should register");
        error_type_id
    }

    /// A real single-field record type: one interned struct path, one interned field path, one
    /// `I32` field, plus the paired HIR layout entry bound in `finish`.
    fn record_with_int_field(&mut self) -> TypeId {
        let record_path = self.path("record");
        let value_field_path = self.path("value");
        let int = self.int();
        let (_, record_type) = self
            .environment
            .register_nominal_struct(StructTypeDefinition {
                id: NominalTypeId(0),
                path: record_path,
                fields: vec![FieldDefinition {
                    name: value_field_path,
                    type_id: int,
                    span: None,
                }]
                .into_boxed_slice(),
                generic_parameters: None,
                const_record: false,
            });
        self.module.structs.push(HirStruct {
            id: StructId(0),
            frontend_type_id: record_type,
            fields: vec![HirField {
                id: FieldId(0),
                ty: int,
            }],
        });
        self.module
            .side_table
            .bind_field_name(FieldId(0), value_field_path);
        record_type
    }

    fn expression(
        &mut self,
        kind: HirExpressionKind,
        ty: TypeId,
        value_kind: ValueKind,
    ) -> HirValueId {
        expression(
            kind,
            ty,
            RegionId(0),
            value_kind,
            &mut self.module.expressions,
        )
    }

    fn int_literal(&mut self, value: i64, ty: TypeId) -> HirValueId {
        let value = FixedScalarValue::signed(FixedScalar::I32, value).expect("literal fits I32");
        self.fixed_literal(value, ty)
    }

    fn float_literal(&mut self, value: f64, ty: TypeId) -> HirValueId {
        let scalar = self
            .environment
            .fixed_scalar(ty)
            .expect("float literal type must be a fixed scalar");
        let value = FixedScalarValue::binary_float(scalar, value).expect("literal fits its scalar");
        self.fixed_literal(value, ty)
    }

    fn number_literal(&mut self, value: i128, ty: TypeId) -> HirValueId {
        self.expression(
            HirExpressionKind::Number(NumberValue::from_integer(value, NumberScale::ZERO)),
            ty,
            ValueKind::Const,
        )
    }

    fn unsigned_literal(&mut self, scalar: FixedScalar, value: u64) -> HirValueId {
        let value = FixedScalarValue::unsigned(scalar, value).expect("literal fits its scalar");
        let ty = self.fixed(scalar);
        self.fixed_literal(value, ty)
    }

    fn signed_literal(&mut self, scalar: FixedScalar, value: i64) -> HirValueId {
        let value = FixedScalarValue::signed(scalar, value).expect("literal fits its scalar");
        let ty = self.fixed(scalar);
        self.fixed_literal(value, ty)
    }

    fn fixed_literal(&mut self, value: FixedScalarValue, ty: TypeId) -> HirValueId {
        self.expression(HirExpressionKind::FixedScalar(value), ty, ValueKind::Const)
    }

    fn load(&mut self, local: LocalId, ty: TypeId) -> HirValueId {
        self.expression(
            HirExpressionKind::Load(HirPlace::local(local)),
            ty,
            ValueKind::Place,
        )
    }

    fn copy_of(&mut self, local: LocalId, ty: TypeId) -> HirValueId {
        self.expression(
            HirExpressionKind::Copy(HirPlace::local(local)),
            ty,
            ValueKind::Place,
        )
    }

    fn cast_expression(
        &mut self,
        source: HirValueId,
        policy: BuiltinCastPolicyId,
        ty: TypeId,
    ) -> HirValueId {
        self.expression(
            HirExpressionKind::Cast { source, policy },
            ty,
            ValueKind::RValue,
        )
    }

    fn update_place(&mut self, target: HirPlace, value: HirValueId) -> HirStatement {
        HirStatement {
            id: self.statement_id(),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(target),
                value,
            },
            span: None,
        }
    }

    fn define_local(&mut self, target: LocalId, value: HirValueId) -> HirStatement {
        HirStatement {
            id: self.statement_id(),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(target),
                value,
            },
            span: None,
        }
    }

    fn update_local(&mut self, target: LocalId, value: HirValueId) -> HirStatement {
        self.update_place(HirPlace::local(target), value)
    }

    fn numeric_op(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        result: HirLocalDestination,
        failure_mode: NumericFailureMode,
    ) -> HirStatement {
        HirStatement {
            id: self.statement_id(),
            kind: HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            },
            span: None,
        }
    }

    fn invoke(
        &mut self,
        operation: HirFallibleOperation,
        result: LocalId,
        error: LocalId,
        success: BlockId,
        failure: BlockId,
    ) -> (HirNodeId, HirTerminator) {
        let id = self.statement_id();
        (
            id,
            HirTerminator::Invoke {
                id,
                span: None,
                operation,
                success,
                success_results: vec![result],
                failure,
                error_result: error,
            },
        )
    }

    fn call(&mut self) -> HirStatement {
        HirStatement {
            id: self.statement_id(),
            kind: HirStatementKind::Call {
                target: CallTarget::Local(FunctionId(0)),
                args: HirValueRange::empty(),
                results: vec![],
            },
            span: None,
        }
    }

    fn return_unit(&mut self) -> HirTerminator {
        HirTerminator::Return(HirValueRange::empty())
    }

    fn statement_id(&mut self) -> HirNodeId {
        let id = HirNodeId(self.next_statement_id);
        self.next_statement_id += 1;
        id
    }

    /// Assembles the module from `(locals, statements, terminator)` block specs and pairs it
    /// with the environment. Block 0 is the entry of the single local function.
    fn finish(
        mut self,
        blocks: Vec<(Vec<HirLocal>, Vec<HirStatement>, HirTerminator)>,
    ) -> (HirModule, TypeEnvironment) {
        for (index, (locals, statements, terminator)) in blocks.into_iter().enumerate() {
            self.module.blocks.push(HirBlock {
                id: BlockId(index as u32),
                region: RegionId(0),
                locals,
                statements,
                terminator,
            });
        }

        self.module.functions.push(HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            returns: Box::new([]),
            error_return: None,
        });
        self.module
            .function_origins
            .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
        self.module.start_function = Some(FunctionId(0));
        self.module
            .function_provenance
            .insert(FunctionId(0), Default::default());
        self.module.regions = vec![HirRegion::lexical(RegionId(0), None)];
        self.module.expressions.freeze();

        (self.module, self.environment)
    }
}

fn declared(local_id: LocalId, ty: TypeId) -> HirLocal {
    local(local_id.0, ty, RegionId(0))
}

fn int_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Fixed(FixedScalar::I32),
    }
}

fn uint_op(operator: NumericOperator) -> HirNumericOp {
    HirNumericOp {
        operator,
        domain: NumericScalar::Fixed(FixedScalar::U32),
    }
}

fn fixed_op(operator: NumericOperator, domain: NumericScalar) -> HirNumericOp {
    HirNumericOp { operator, domain }
}

fn binary(left: HirValueId, right: HirValueId) -> HirNumericOperands {
    HirNumericOperands::Binary { left, right }
}

fn unary(operand: HirValueId) -> HirNumericOperands {
    HirNumericOperands::Unary { operand }
}

fn jump(target: BlockId) -> HirTerminator {
    HirTerminator::Jump {
        target,
        args: vec![],
    }
}

// ---------------------------------------------------------------------------
//  Proven operations and retained intervals
// ---------------------------------------------------------------------------

#[test]
fn proven_trap_results_retain_their_exact_intervals() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let (sum, result) = (LocalId(0), LocalId(1));
    // sum = 2 + 2 (exact [4, 4]); result = sum + sum (exact [8, 8] through the cache).
    let left = fixture.int_literal(2, int);
    let right = fixture.int_literal(2, int);
    let first = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let sum_load_left = fixture.load(sum, int);
    let sum_load_right = fixture.load(sum, int);
    let second = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(sum_load_left, sum_load_right),
        HirLocalDestination::Define(result),
        NumericFailureMode::Trap,
    );
    let first_id = first.id;
    let second_id = second.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(sum, int), declared(result, int)],
        vec![first, second],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(first_id));
    assert!(proofs.integer_operation_is_safe(second_id));
}

#[test]
fn operand_facts_are_derived_before_the_destination_write() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let (x, y) = (LocalId(0), LocalId(1));
    // x = 10; y = x - 4; — the left operand reads the cache entry written by x's own assign
    // before y's destination write happens.
    let literal = fixture.int_literal(10, int);
    let write = fixture.define_local(x, literal);
    let left = fixture.load(x, int);
    let right = fixture.int_literal(4, int);
    let operation = fixture.numeric_op(
        int_op(NumericOperator::Subtract),
        binary(left, right),
        HirLocalDestination::Define(y),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(x, int), declared(y, int)],
        vec![write, operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    // Canonical I32 bounds minus four still overflow, so this only proves through the
    // pre-invalidation operand fact.
    assert!(proofs.integer_operation_is_safe(operation_id));
}

// ---------------------------------------------------------------------------
//  Unsafe arithmetic keeps its checks
// ---------------------------------------------------------------------------

#[test]
fn canonical_bounds_cannot_prove_unchecked_parameter_addition() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let (x, y, sum) = (LocalId(0), LocalId(1), LocalId(2));
    // Parameters have no interval facts: complete I32 bounds can overflow.
    let left = fixture.load(x, int);
    let right = fixture.load(y, int);
    let operation = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(x, int), declared(y, int), declared(sum, int)],
        vec![operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn unsigned_subtraction_below_zero_is_never_proven() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (x, difference) = (LocalId(0), LocalId(1));
    // x = 2; difference = x - 3; — borrows below zero for U32.
    let literal = fixture.unsigned_literal(FixedScalar::U32, 2);
    let write = fixture.define_local(x, literal);
    let left = fixture.load(x, u32_type);
    let right = fixture.unsigned_literal(FixedScalar::U32, 3);
    let operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Subtract,
            NumericScalar::Fixed(FixedScalar::U32),
        ),
        binary(left, right),
        HirLocalDestination::Define(difference),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(x, u32_type), declared(difference, u32_type)],
        vec![write, operation],
        terminator,
    )]);
    validate_hir_module(&module, &environment, &ExternalPackageRegistry::new())
        .expect("ordinary subtraction fixture is valid HIR");

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn division_with_possible_zero_divisor_keeps_its_check() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (x, y, quotient) = (LocalId(0), LocalId(1), LocalId(2));
    // y has no interval fact: its full U32 divisor interval contains zero.
    let left = fixture.load(x, u32_type);
    let right = fixture.load(y, u32_type);
    let operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::IntegerDivide,
            NumericScalar::Fixed(FixedScalar::U32),
        ),
        binary(left, right),
        HirLocalDestination::Define(quotient),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (mut module, environment) = fixture.finish(vec![(
        vec![
            declared(x, u32_type),
            declared(y, u32_type),
            declared(quotient, u32_type),
        ],
        vec![operation],
        terminator,
    )]);
    // Both operands are unknown ABI inputs, not reads of undefined locals.
    module.functions[0].params = vec![x, y];
    validate_hir_module(&module, &environment, &ExternalPackageRegistry::new())
        .expect("ordinary division fixture is valid HIR");

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn division_with_nonzero_literal_divisor_is_proven() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (x, quotient) = (LocalId(0), LocalId(1));
    // x = 9; quotient = x / 2; — divisor excludes zero and every quotient fits U32.
    let literal = fixture.unsigned_literal(FixedScalar::U32, 9);
    let write = fixture.define_local(x, literal);
    let left = fixture.load(x, u32_type);
    let right = fixture.unsigned_literal(FixedScalar::U32, 2);
    let operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::IntegerDivide,
            NumericScalar::Fixed(FixedScalar::U32),
        ),
        binary(left, right),
        HirLocalDestination::Define(quotient),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(x, u32_type), declared(quotient, u32_type)],
        vec![write, operation],
        terminator,
    )]);
    validate_hir_module(&module, &environment, &ExternalPackageRegistry::new())
        .expect("ordinary literal-divisor fixture is valid HIR");

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn signed_minimum_divided_by_minus_one_keeps_its_check_while_remainder_stays_exact() {
    let mut fixture = ProofFixture::new();
    let i64_type = fixture.fixed(FixedScalar::I64);
    let (x, quotient, remainder) = (LocalId(0), LocalId(1), LocalId(2));
    // x = I64::MIN; quotient = x / -1 overflows; remainder = x % -1 is the specified zero.
    let minimum = fixture.signed_literal(FixedScalar::I64, i64::MIN);
    let write = fixture.define_local(x, minimum);
    let minus_one = fixture.signed_literal(FixedScalar::I64, -1);
    let x_load = fixture.load(x, i64_type);
    let division_right = minus_one;
    let division = fixture.numeric_op(
        fixed_op(
            NumericOperator::IntegerDivide,
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        binary(x_load, division_right),
        HirLocalDestination::Define(quotient),
        NumericFailureMode::Trap,
    );
    let x_reload = fixture.load(x, i64_type);
    let remainder_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Remainder,
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        binary(x_reload, minus_one),
        HirLocalDestination::Define(remainder),
        NumericFailureMode::Trap,
    );
    let division_id = division.id;
    let remainder_id = remainder_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(x, i64_type),
            declared(quotient, i64_type),
            declared(remainder, i64_type),
        ],
        vec![write, division, remainder_operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(division_id));
    assert!(proofs.integer_operation_is_safe(remainder_id));
}

#[test]
fn product_bounds_overflowing_the_domain_or_i128_never_prove() {
    let mut fixture = ProofFixture::new();
    let u64_type = fixture.fixed(FixedScalar::U64);
    let i64_type = fixture.fixed(FixedScalar::I64);
    let (wide, wide_product, signed_left, signed_right, signed_product) =
        (LocalId(0), LocalId(1), LocalId(2), LocalId(3), LocalId(4));
    // U64 max squared overflows even the i128 product bound; full-range I64 parameters fit
    // i128 but overflow the I64 domain, so both keep their checks.
    let maximum = fixture.unsigned_literal(FixedScalar::U64, u64::MAX);
    let write = fixture.define_local(wide, maximum);
    let wide_left = fixture.load(wide, u64_type);
    let wide_right = fixture.load(wide, u64_type);
    let wide_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Multiply,
            NumericScalar::Fixed(FixedScalar::U64),
        ),
        binary(wide_left, wide_right),
        HirLocalDestination::Define(wide_product),
        NumericFailureMode::Trap,
    );
    let left_load = fixture.load(signed_left, i64_type);
    let right_load = fixture.load(signed_right, i64_type);
    let signed_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Multiply,
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        binary(left_load, right_load),
        HirLocalDestination::Define(signed_product),
        NumericFailureMode::Trap,
    );
    let wide_id = wide_operation.id;
    let signed_id = signed_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(wide, u64_type),
            declared(wide_product, u64_type),
            declared(signed_left, i64_type),
            declared(signed_right, i64_type),
            declared(signed_product, i64_type),
        ],
        vec![write, wide_operation, signed_operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(wide_id));
    assert!(!proofs.integer_operation_is_safe(signed_id));
}

#[test]
fn u64_interval_arithmetic_is_exact_at_the_domain_edge() {
    let mut fixture = ProofFixture::new();
    let u64_type = fixture.fixed(FixedScalar::U64);
    let (x, exact, overflow) = (LocalId(0), LocalId(1), LocalId(2));
    // x = u64::MAX - 1: x + 1 stays exactly inside U64, x + 2 overflows it.
    let literal = fixture.unsigned_literal(FixedScalar::U64, u64::MAX - 1);
    let write = fixture.define_local(x, literal);
    let exact_left = fixture.load(x, u64_type);
    let exact_right = fixture.unsigned_literal(FixedScalar::U64, 1);
    let exact_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U64)),
        binary(exact_left, exact_right),
        HirLocalDestination::Define(exact),
        NumericFailureMode::Trap,
    );
    let overflow_left = fixture.load(x, u64_type);
    let overflow_right = fixture.unsigned_literal(FixedScalar::U64, 2);
    let overflow_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U64)),
        binary(overflow_left, overflow_right),
        HirLocalDestination::Define(overflow),
        NumericFailureMode::Trap,
    );
    let exact_id = exact_operation.id;
    let overflow_id = overflow_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(x, u64_type),
            declared(exact, u64_type),
            declared(overflow, u64_type),
        ],
        vec![write, exact_operation, overflow_operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(exact_id));
    assert!(!proofs.integer_operation_is_safe(overflow_id));
}

#[test]
fn negate_is_proven_only_for_signed_domains_inside_their_range() {
    let mut fixture = ProofFixture::new();
    let i64_type = fixture.fixed(FixedScalar::I64);
    let u8_type = fixture.fixed(FixedScalar::U8);
    let (signed_x, signed_negation, unsigned_x, unsigned_negation) =
        (LocalId(0), LocalId(1), LocalId(2), LocalId(3));
    // Signed minimum cannot negate inside I64; unsigned negation is not a valid source shape
    // and must never become a proven raw operation, even for a known zero operand.
    let minimum = fixture.signed_literal(FixedScalar::I64, i64::MIN);
    let signed_write = fixture.define_local(signed_x, minimum);
    let signed_operand = fixture.load(signed_x, i64_type);
    let signed_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Negate,
            NumericScalar::Fixed(FixedScalar::I64),
        ),
        unary(signed_operand),
        HirLocalDestination::Define(signed_negation),
        NumericFailureMode::Trap,
    );
    let zero = fixture.unsigned_literal(FixedScalar::U8, 0);
    let unsigned_write = fixture.define_local(unsigned_x, zero);
    let unsigned_operand = fixture.load(unsigned_x, u8_type);
    let unsigned_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Negate,
            NumericScalar::Fixed(FixedScalar::U8),
        ),
        unary(unsigned_operand),
        HirLocalDestination::Define(unsigned_negation),
        NumericFailureMode::Trap,
    );
    let signed_id = signed_operation.id;
    let unsigned_id = unsigned_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(signed_x, i64_type),
            declared(signed_negation, i64_type),
            declared(unsigned_x, u8_type),
            declared(unsigned_negation, i64_type),
        ],
        vec![
            signed_write,
            signed_operation,
            unsigned_write,
            unsigned_operation,
        ],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(signed_id));
    assert!(!proofs.integer_operation_is_safe(unsigned_id));
}

#[test]
fn power_float_and_number_domains_always_stay_checked() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let float = fixture.float();
    let f32_type = fixture.fixed(FixedScalar::F32);
    let number_type = fixture.number();
    let (base, exponent, power_result, float_result, f32_result, number_result) = (
        LocalId(0),
        LocalId(1),
        LocalId(2),
        LocalId(3),
        LocalId(4),
        LocalId(5),
    );
    let base_literal = fixture.int_literal(2, int);
    let base_write = fixture.define_local(base, base_literal);
    let exponent_literal = fixture.int_literal(3, int);
    let exponent_write = fixture.define_local(exponent, exponent_literal);
    let power_left = fixture.load(base, int);
    let power_right = fixture.load(exponent, int);
    let power_operation = fixture.numeric_op(
        int_op(NumericOperator::Power),
        binary(power_left, power_right),
        HirLocalDestination::Define(power_result),
        NumericFailureMode::Trap,
    );
    let float_left = fixture.float_literal(1.0, float);
    let float_right = fixture.float_literal(2.0, float);
    let float_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::F64)),
        binary(float_left, float_right),
        HirLocalDestination::Define(float_result),
        NumericFailureMode::Trap,
    );
    let f32_left = fixture.float_literal(1.0, f32_type);
    let f32_right = fixture.float_literal(2.0, f32_type);
    let f32_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::F32)),
        binary(f32_left, f32_right),
        HirLocalDestination::Define(f32_result),
        NumericFailureMode::Trap,
    );
    let number_left = fixture.number_literal(1, number_type);
    let number_right = fixture.number_literal(2, number_type);
    let number_operation = fixture.numeric_op(
        fixed_op(
            NumericOperator::Add,
            NumericScalar::Number(NumberScale::ZERO),
        ),
        binary(number_left, number_right),
        HirLocalDestination::Define(number_result),
        NumericFailureMode::Trap,
    );
    let power_id = power_operation.id;
    let float_id = float_operation.id;
    let f32_id = f32_operation.id;
    let number_id = number_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(base, int),
            declared(exponent, int),
            declared(power_result, int),
            declared(float_result, float),
            declared(f32_result, f32_type),
            declared(number_result, number_type),
        ],
        vec![
            base_write,
            exponent_write,
            power_operation,
            float_operation,
            f32_operation,
            number_operation,
        ],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(power_id));
    assert!(!proofs.integer_operation_is_safe(float_id));
    assert!(!proofs.integer_operation_is_safe(f32_id));
    assert!(!proofs.integer_operation_is_safe(number_id));
}

// ---------------------------------------------------------------------------
//  Narrowing proofs
// ---------------------------------------------------------------------------

#[test]
fn narrowing_is_proven_only_when_the_source_interval_fits_the_target() {
    assert_narrowing_proofs(FixedScalar::U8);
}

#[test]
fn narrowing_skips_non_integer_conversion_policies() {
    for source_scalar in [
        NumericScalar::Fixed(FixedScalar::F64),
        NumericScalar::Number(NumberScale::ZERO),
    ] {
        let mut fixture = ProofFixture::new();
        let source_type = match source_scalar {
            NumericScalar::Fixed(scalar) => fixture.fixed(scalar),
            NumericScalar::Number(_) => fixture.number(),
        };
        let int = fixture.int();
        let error_type = fixture.error_type();
        let source = fixture.load(LocalId(0), source_type);
        let (id, invoke) = fixture.invoke(
            HirFallibleOperation::CastOp {
                policy: BuiltinCastPolicyId::NumericConversion {
                    source: source_scalar,
                    target: NumericScalar::Fixed(FixedScalar::I32),
                },
                source,
            },
            LocalId(1),
            LocalId(2),
            BlockId(1),
            BlockId(2),
        );
        let (module, environment) = fixture.finish(vec![
            (
                vec![
                    declared(LocalId(0), source_type),
                    declared(LocalId(1), int),
                    declared(LocalId(2), error_type),
                ],
                vec![],
                invoke,
            ),
            (
                vec![],
                vec![],
                HirTerminator::Return(HirValueRange::empty()),
            ),
            (
                vec![],
                vec![],
                HirTerminator::Return(HirValueRange::empty()),
            ),
        ]);
        assert!(!analyse_numeric_proofs(&module, &environment).integer_narrowing_is_safe(id));
    }
}

#[test]
fn infallible_cast_expressions_preserve_operand_intervals() {
    let mut fixture = ProofFixture::new();
    let u8_type = fixture.fixed(FixedScalar::U8);
    let i32_type = fixture.fixed(FixedScalar::I32);
    let (source, narrowed, sum) = (LocalId(0), LocalId(1), LocalId(2));
    // source = 200u8; narrowed = source as I32 (infallible widening, exact [200, 200]);
    // sum = narrowed + narrowed is exact [400, 400] inside I32 only because the cast
    // preserved the source interval instead of falling back to full I32 bounds.
    let literal = fixture.unsigned_literal(FixedScalar::U8, 200);
    let write = fixture.define_local(source, literal);
    let cast_operand = fixture.load(source, u8_type);
    let cast_value = fixture.cast_expression(
        cast_operand,
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::U8),
            target: NumericScalar::Fixed(FixedScalar::I32),
        },
        i32_type,
    );
    let cast_write = fixture.define_local(narrowed, cast_value);
    let left = fixture.load(narrowed, i32_type);
    let right = fixture.load(narrowed, i32_type);
    let operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::I32)),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(source, u8_type),
            declared(narrowed, i32_type),
            declared(sum, i32_type),
        ],
        vec![write, cast_write, operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(operation_id));
}

// ---------------------------------------------------------------------------
//  Invalidation
// ---------------------------------------------------------------------------

#[test]
fn reassignment_replaces_the_cached_interval() {
    // Small-to-large: x = 3; x = U32::MAX; sum = x + 1; — only the newest [MAX, MAX] interval
    // survives, and it overflows U32, so the addition keeps its check. A retained [3, 3]
    // would wrongly prove it.
    {
        let mut fixture = ProofFixture::new();
        let u32_type = fixture.fixed(FixedScalar::U32);
        let (x, sum) = (LocalId(0), LocalId(1));
        let first = fixture.unsigned_literal(FixedScalar::U32, 3);
        let first_write = fixture.define_local(x, first);
        let second = fixture.unsigned_literal(FixedScalar::U32, u32::MAX.into());
        let second_write = fixture.update_local(x, second);
        let left = fixture.load(x, u32_type);
        let right = fixture.unsigned_literal(FixedScalar::U32, 1);
        let operation = fixture.numeric_op(
            fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
            binary(left, right),
            HirLocalDestination::Define(sum),
            NumericFailureMode::Trap,
        );
        let operation_id = operation.id;
        let terminator = fixture.return_unit();
        let (module, environment) = fixture.finish(vec![(
            vec![declared(x, u32_type), declared(sum, u32_type)],
            vec![first_write, second_write, operation],
            terminator,
        )]);

        let proofs = analyse_numeric_proofs(&module, &environment);

        assert!(!proofs.integer_operation_is_safe(operation_id));
    }

    // Large-to-small: x = U32::MAX; x = 3; sum = x + 1; — the newest [3, 3] interval proves
    // the exact [4, 4] result that a retained [MAX, MAX] would have refused.
    {
        let mut fixture = ProofFixture::new();
        let u32_type = fixture.fixed(FixedScalar::U32);
        let (x, sum) = (LocalId(0), LocalId(1));
        let first = fixture.unsigned_literal(FixedScalar::U32, u32::MAX.into());
        let first_write = fixture.define_local(x, first);
        let second = fixture.unsigned_literal(FixedScalar::U32, 3);
        let second_write = fixture.update_local(x, second);
        let left = fixture.load(x, u32_type);
        let right = fixture.unsigned_literal(FixedScalar::U32, 1);
        let operation = fixture.numeric_op(
            fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
            binary(left, right),
            HirLocalDestination::Define(sum),
            NumericFailureMode::Trap,
        );
        let operation_id = operation.id;
        let terminator = fixture.return_unit();
        let (module, environment) = fixture.finish(vec![(
            vec![declared(x, u32_type), declared(sum, u32_type)],
            vec![first_write, second_write, operation],
            terminator,
        )]);

        let proofs = analyse_numeric_proofs(&module, &environment);

        assert!(proofs.integer_operation_is_safe(operation_id));
    }
}

#[test]
fn an_intermediate_write_to_another_local_drops_the_single_entry_cache() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (x, y, sum) = (LocalId(0), LocalId(1), LocalId(2));
    // x = 3; y = 4; sum = x + x; — x's fact was displaced by y's write, so x reads canonical
    // bounds whose sum overflows U32.
    let x_literal = fixture.unsigned_literal(FixedScalar::U32, 3);
    let x_write = fixture.define_local(x, x_literal);
    let y_literal = fixture.unsigned_literal(FixedScalar::U32, 4);
    let y_write = fixture.define_local(y, y_literal);
    let left = fixture.load(x, u32_type);
    let right = fixture.load(x, u32_type);
    let operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(x, u32_type),
            declared(y, u32_type),
            declared(sum, u32_type),
        ],
        vec![x_write, y_write, operation],
        terminator,
    )]);
    validate_hir_module(&module, &environment, &ExternalPackageRegistry::new())
        .expect("ordinary cache-displacement fixture is valid HIR");

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn alias_copies_carry_intervals_until_the_next_call() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let (x, alias, after_sum) = (LocalId(0), LocalId(1), LocalId(2));
    // x = 3; alias = copy(x); alias = alias + alias; call(); after_sum = alias + alias;
    // The first operation is proven [6, 6] and writes back into the alias itself, so the
    // cache entry the call must evict is the only thing standing between the post-call
    // operands and canonical I32 bounds. Without call invalidation the retained [6, 6]
    // would prove the post-call operation too and flip the second assertion.
    let literal = fixture.int_literal(3, int);
    let write = fixture.define_local(x, literal);
    let aliased = fixture.copy_of(x, int);
    let alias_write = fixture.define_local(alias, aliased);
    let first_left = fixture.load(alias, int);
    let first_right = fixture.load(alias, int);
    let first = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(first_left, first_right),
        HirLocalDestination::Update(alias),
        NumericFailureMode::Trap,
    );
    let call = fixture.call();
    let after_left = fixture.load(alias, int);
    let after_right = fixture.load(alias, int);
    let after = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(after_left, after_right),
        HirLocalDestination::Define(after_sum),
        NumericFailureMode::Trap,
    );
    let first_id = first.id;
    let after_id = after.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(x, int),
            declared(alias, int),
            declared(after_sum, int),
        ],
        vec![write, alias_write, first, call, after],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(first_id));
    assert!(!proofs.integer_operation_is_safe(after_id));
}

#[test]
fn projection_writes_invalidate_the_cached_interval() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let record_type = fixture.record_with_int_field();
    let (x, record, sum) = (LocalId(0), LocalId(1), LocalId(2));
    // x = 3; record.value = 4; sum = x + x; — the projection write through the record drops
    // x's fact. `record` is typed at the registered single-field record, and FieldId(0) is
    // that struct's paired HIR layout field.
    let literal = fixture.int_literal(3, int);
    let write = fixture.define_local(x, literal);
    let field_value = fixture.int_literal(4, int);
    let projection_target = HirPlace::local(record)
        .with_field(FieldId(0), &mut fixture.module.expressions, None)
        .expect("record field projection should fit the HIR expression store");
    let projection_write = fixture.update_place(projection_target, field_value);
    let left = fixture.load(x, int);
    let right = fixture.load(x, int);
    let operation = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![
            declared(x, int),
            declared(record, record_type),
            declared(sum, int),
        ],
        vec![write, projection_write, operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn invoke_proofs_do_not_propagate_intervals_to_successors() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let error_type = fixture.error_type();
    let (source, result, error, sum) = (LocalId(0), LocalId(1), LocalId(2), LocalId(3));
    let literal = fixture.int_literal(3, int);
    let write = fixture.define_local(source, literal);
    let left = fixture.load(source, int);
    let right = fixture.int_literal(1, int);
    let (id, invoke) = fixture.invoke(
        HirFallibleOperation::NumericOp {
            op: int_op(NumericOperator::Add),
            operands: binary(left, right),
        },
        result,
        error,
        BlockId(1),
        BlockId(2),
    );
    let left = fixture.load(result, int);
    let right = fixture.load(result, int);
    let follow_up = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(left, right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let follow_up_id = follow_up.id;
    let (module, environment) = fixture.finish(vec![
        (
            vec![
                declared(source, int),
                declared(result, int),
                declared(error, error_type),
                declared(sum, int),
            ],
            vec![write],
            invoke,
        ),
        (
            vec![],
            vec![follow_up],
            HirTerminator::Return(HirValueRange::empty()),
        ),
        (
            vec![],
            vec![],
            HirTerminator::Return(HirValueRange::empty()),
        ),
    ]);
    let proofs = analyse_numeric_proofs(&module, &environment);
    assert!(proofs.integer_operation_is_safe(id));
    assert!(!proofs.integer_operation_is_safe(follow_up_id));
    assert!(matches!(
        &module.blocks[0].terminator,
        HirTerminator::Invoke {
            success: BlockId(1),
            failure: BlockId(2),
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
//  Block independence
// ---------------------------------------------------------------------------

#[test]
fn joined_blocks_reset_value_specific_state() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (x, sum) = (LocalId(0), LocalId(1));
    // Entry: x = 3; x = x + x (exact [6, 6] cached in the very local the join reads); jump join.
    // Join:  sum = x + x; — the join resets state, so x reads canonical U32 bounds.
    let literal = fixture.unsigned_literal(FixedScalar::U32, 3);
    let write = fixture.define_local(x, literal);
    let entry_left = fixture.load(x, u32_type);
    let entry_right = fixture.load(x, u32_type);
    let entry_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
        binary(entry_left, entry_right),
        HirLocalDestination::Update(x),
        NumericFailureMode::Trap,
    );
    let join_left = fixture.load(x, u32_type);
    let join_right = fixture.load(x, u32_type);
    let join_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
        binary(join_left, join_right),
        HirLocalDestination::Define(sum),
        NumericFailureMode::Trap,
    );
    let entry_id = entry_operation.id;
    let join_id = join_operation.id;
    let join_terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![
        (
            vec![declared(x, u32_type)],
            vec![write, entry_operation],
            jump(BlockId(1)),
        ),
        (
            vec![declared(sum, u32_type)],
            vec![join_operation],
            join_terminator,
        ),
    ]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(entry_id));
    assert!(!proofs.integer_operation_is_safe(join_id));
}

#[test]
fn loop_headers_reset_state_against_back_edge_writes() {
    let mut fixture = ProofFixture::new();
    let int = fixture.int();
    let (counter, doubled) = (LocalId(0), LocalId(1));
    // Entry: counter = 3; jump header.
    // Header: doubled = counter + counter; counter = I32::MAX; jump self. The header walk
    // resets state, so the doubling cannot lean on the entry's [3, 3] singleton even though
    // the back edge later writes the I32 maximum into the very same local.
    let literal = fixture.int_literal(3, int);
    let write = fixture.define_local(counter, literal);
    let left = fixture.load(counter, int);
    let right = fixture.load(counter, int);
    let header_operation = fixture.numeric_op(
        int_op(NumericOperator::Add),
        binary(left, right),
        HirLocalDestination::Define(doubled),
        NumericFailureMode::Trap,
    );
    let back_edge_write = fixture.int_literal(i64::from(i32::MAX), int);
    let counter_write = fixture.update_local(counter, back_edge_write);
    let header_id = header_operation.id;
    let (module, environment) = fixture.finish(vec![
        (vec![declared(counter, int)], vec![write], jump(BlockId(1))),
        (
            vec![declared(doubled, int)],
            vec![header_operation, counter_write],
            jump(BlockId(1)),
        ),
    ]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(header_id));
}

#[test]
fn loop_body_writes_prove_operations_within_the_same_block_walk() {
    let mut fixture = ProofFixture::new();
    let u32_type = fixture.fixed(FixedScalar::U32);
    let (index, doubled) = (LocalId(0), LocalId(1));
    // A self-jumping header that writes its counter first can still prove an operation on it
    // inside the same block walk: index = 2; doubled = index + index; jump self.
    let literal = fixture.unsigned_literal(FixedScalar::U32, 2);
    let write = fixture.define_local(index, literal);
    let left = fixture.load(index, u32_type);
    let right = fixture.load(index, u32_type);
    let operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U32)),
        binary(left, right),
        HirLocalDestination::Define(doubled),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let (module, environment) = fixture.finish(vec![(
        vec![declared(index, u32_type), declared(doubled, u32_type)],
        vec![write, operation],
        jump(BlockId(0)),
    )]);
    validate_hir_module(&module, &environment, &ExternalPackageRegistry::new())
        .expect("ordinary same-block loop fixture is valid HIR");

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(operation_id));
}

#[test]
fn uint_singletons_prove_exact_arithmetic_and_reject_overflow() {
    // U32 literals are exact singletons, so `1 + 2` proves while
    // `u32::MAX - 1 + 2` keeps its check; the U64 test below covers the
    // bounds above `i64::MAX`.
    let mut fixture = ProofFixture::new();
    let uint = fixture.uint();
    let u32_max = u64::from(u32::MAX);
    let (exact, overflow) = (LocalId(0), LocalId(1));
    let exact_left = fixture.uint_literal(1, uint);
    let exact_right = fixture.uint_literal(2, uint);
    let exact_operation = fixture.numeric_op(
        uint_op(NumericOperator::Add),
        binary(exact_left, exact_right),
        HirLocalDestination::Define(exact),
        NumericFailureMode::Trap,
    );
    let overflow_left = fixture.uint_literal(u32_max - 1, uint);
    let overflow_right = fixture.uint_literal(2, uint);
    let overflow_operation = fixture.numeric_op(
        uint_op(NumericOperator::Add),
        binary(overflow_left, overflow_right),
        HirLocalDestination::Define(overflow),
        NumericFailureMode::Trap,
    );
    let exact_id = exact_operation.id;
    let overflow_id = overflow_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(exact, uint), declared(overflow, uint)],
        vec![exact_operation, overflow_operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(exact_id));
    assert!(!proofs.integer_operation_is_safe(overflow_id));
}

#[test]
fn uint64_singletons_prove_exact_arithmetic_above_i64_max() {
    // U64 spans the whole u64 range, so `u64::MAX - 1 + 1` proves exactly while
    // `+ 2` overflows — the i128 interval path above `i64::MAX`.
    let mut fixture = ProofFixture::new();
    let uint = fixture.fixed(FixedScalar::U64);
    let (exact, overflow) = (LocalId(0), LocalId(1));
    let exact_left = fixture.unsigned_literal(FixedScalar::U64, u64::MAX - 1);
    let exact_right = fixture.unsigned_literal(FixedScalar::U64, 1);
    let exact_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U64)),
        binary(exact_left, exact_right),
        HirLocalDestination::Define(exact),
        NumericFailureMode::Trap,
    );
    let overflow_left = fixture.unsigned_literal(FixedScalar::U64, u64::MAX - 1);
    let overflow_right = fixture.unsigned_literal(FixedScalar::U64, 2);
    let overflow_operation = fixture.numeric_op(
        fixed_op(NumericOperator::Add, NumericScalar::Fixed(FixedScalar::U64)),
        binary(overflow_left, overflow_right),
        HirLocalDestination::Define(overflow),
        NumericFailureMode::Trap,
    );
    let exact_id = exact_operation.id;
    let overflow_id = overflow_operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(exact, uint), declared(overflow, uint)],
        vec![exact_operation, overflow_operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(proofs.integer_operation_is_safe(exact_id));
    assert!(!proofs.integer_operation_is_safe(overflow_id));
}

#[test]
fn uint_subtraction_below_zero_is_never_proven() {
    // `0 - 1` underflows U32 exactly like the unsigned fixed case.
    let mut fixture = ProofFixture::new();
    let uint = fixture.uint();
    let difference = LocalId(0);
    let left = fixture.uint_literal(0, uint);
    let right = fixture.uint_literal(1, uint);
    let operation = fixture.numeric_op(
        uint_op(NumericOperator::Subtract),
        binary(left, right),
        HirLocalDestination::Define(difference),
        NumericFailureMode::Trap,
    );
    let operation_id = operation.id;
    let terminator = fixture.return_unit();
    let (module, environment) = fixture.finish(vec![(
        vec![declared(difference, uint)],
        vec![operation],
        terminator,
    )]);

    let proofs = analyse_numeric_proofs(&module, &environment);

    assert!(!proofs.integer_operation_is_safe(operation_id));
}

fn assert_narrowing_proofs(target: FixedScalar) {
    let mut fixture = ProofFixture::new();
    let uint = fixture.uint();
    let target_type = fixture.fixed(target);
    let error_type = fixture.error_type();
    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::U32),
        target: NumericScalar::Fixed(target),
    };
    let literal = fixture.uint_literal(5, uint);
    let write = fixture.define_local(LocalId(0), literal);
    let source = fixture.load(LocalId(0), uint);
    let (proven_id, proven) = fixture.invoke(
        HirFallibleOperation::CastOp { policy, source },
        LocalId(1),
        LocalId(4),
        BlockId(1),
        BlockId(2),
    );
    let source = fixture.load(LocalId(2), uint);
    let (unproven_id, unproven) = fixture.invoke(
        HirFallibleOperation::CastOp { policy, source },
        LocalId(3),
        LocalId(5),
        BlockId(2),
        BlockId(2),
    );
    let (module, environment) = fixture.finish(vec![
        (
            vec![
                declared(LocalId(0), uint),
                declared(LocalId(1), target_type),
                declared(LocalId(2), uint),
                declared(LocalId(3), target_type),
                declared(LocalId(4), error_type),
                declared(LocalId(5), error_type),
            ],
            vec![write],
            proven,
        ),
        (vec![], vec![], unproven),
        (
            vec![],
            vec![],
            HirTerminator::Return(HirValueRange::empty()),
        ),
    ]);
    let proofs = analyse_numeric_proofs(&module, &environment);
    assert!(proofs.integer_narrowing_is_safe(proven_id));
    assert!(!proofs.integer_narrowing_is_safe(unproven_id));
}

#[test]
fn uint_narrowing_is_proven_only_when_the_source_interval_fits() {
    assert_narrowing_proofs(FixedScalar::I32);
}

#[test]
fn uint_invoke_proof_preserves_recoverable_delivery() {
    let mut fixture = ProofFixture::new();
    let uint = fixture.uint();
    let error_type = fixture.error_type();
    let left = fixture.uint_literal(1, uint);
    let right = fixture.uint_literal(2, uint);
    let (id, invoke) = fixture.invoke(
        HirFallibleOperation::NumericOp {
            op: uint_op(NumericOperator::Add),
            operands: binary(left, right),
        },
        LocalId(0),
        LocalId(1),
        BlockId(1),
        BlockId(2),
    );
    let (module, environment) = fixture.finish(vec![
        (
            vec![declared(LocalId(0), uint), declared(LocalId(1), error_type)],
            vec![],
            invoke,
        ),
        (
            vec![],
            vec![],
            HirTerminator::Return(HirValueRange::empty()),
        ),
        (
            vec![],
            vec![],
            HirTerminator::Return(HirValueRange::empty()),
        ),
    ]);
    assert!(analyse_numeric_proofs(&module, &environment).integer_operation_is_safe(id));
    assert!(matches!(
        &module.blocks[0].terminator,
        HirTerminator::Invoke {
            success: BlockId(1),
            failure: BlockId(2),
            ..
        }
    ));
}
