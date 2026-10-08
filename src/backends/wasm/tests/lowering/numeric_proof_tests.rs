//! Numeric proof gating for Wasm integer lowering.
//!
//! WHAT: lowers one genuine fixture HIR twice — with the retained (empty) proof table and with
//!       the analysed table computed on the same HIR — and asserts the explicit per-statement
//!       `IntegerCheckMode` decisions: proven aliasing add/multiply/divide lower without alias
//!       temps, copies or check-only widened scratch, while dynamic-operand add/subtract/negate/
//!       divide statements and Power stay Runtime-checked in both tables.
//! WHY: proof-mode lowering must keep observable semantics identical while removing only the
//!       redundant predicates and scratch the analysis authorized. The Node-execution companion
//!       (`proven_integer_operations_execute_with_aliased_destinations` in tests/emit/emit_tests.rs)
//!       runs the actually emitted, validated bytes of both tables of this same HIR.

use super::test_support::{
    build_module, build_type_environment, default_borrow_facts, default_numeric_proofs, expression,
    load_local, local,
};
use crate::backends::wasm::backend::lower_hir_to_wasm_lir;
use crate::backends::wasm::lir::function::WasmLirFunctionOrigin;
use crate::backends::wasm::lir::instructions::{
    IntegerCheckMode, WasmCheckedIntegerOperation, WasmLirStmt, WasmNumericOperationOperands,
};
use crate::backends::wasm::request::WasmBackendRequest;
use crate::compiler_frontend::analysis::numeric_proofs::{NumericProofs, analyse_numeric_proofs};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::NumericProfile;

/// Fixture statement-local identifiers the two lanes share, keying the analysis queries.
///
/// WHAT: one record per provable dynamic shape, plus the explicit unbounded controls.
#[derive(Clone, Copy)]
pub(crate) struct ProofFixtureStatementIds {
    /// `x = x + x` after `x = 3`: intervals [3,3]+[3,3] = [6,6] are exact and in-domain.
    pub(crate) proven_alias_add: HirNodeId,
    /// `x = x * x` after the proven add: [6,6]*[6,6] = [36,36] is in-domain, so the widened
    /// product exists only as a check-only scratch.
    pub(crate) proven_alias_multiply: HirNodeId,
    /// `q = x / d` over a dynamic divisor: zero and INT_MIN/-1 remain possible failures.
    pub(crate) dynamic_divide: HirNodeId,
    /// `c = a - d` over dynamic parameters: the subtraction can underflow.
    pub(crate) dynamic_subtract: HirNodeId,
    /// `b = -b` over a dynamic parameter: negating INT_MIN remains possible.
    pub(crate) dynamic_negate: HirNodeId,
    /// `w = w + w` after `w = 3` at I64 width: exact [6,6] within the I64 domain.
    pub(crate) proven_i64_alias_add: HirNodeId,
    /// `u = u / u` after `u = 6` at U64 width: the divisor interval excludes zero,
    /// unlike the dynamic divisor in `q = x / d`.
    pub(crate) proven_u64_alias_divide: HirNodeId,
    /// `p = 2 ^ 3`: Power always retains checks regardless of operand intervals.
    pub(crate) power: HirNodeId,
    /// `r = x + q`: earlier writes cleared x's interval, so the checksum add stays Runtime.
    pub(crate) checksum_add: HirNodeId,
    /// `r = r + c` consuming the dynamic subtraction into the checksum (stays Runtime).
    pub(crate) consumed_checksum_subtract: HirNodeId,
    /// `r = r + p` consuming the Power result into the checksum (stays Runtime).
    pub(crate) consumed_checksum_power_add: HirNodeId,
    /// Isolated-width function `run_i64`: `w = w + w` over the I64 alias chain.
    pub(crate) isolated_i64_alias_add: HirNodeId,
    /// Isolated-width function `run_u64`: `u = u * u` over the U64 alias chain.
    pub(crate) isolated_u64_alias_multiply: HirNodeId,
}

/// The genuine fixture HIR shared by both proof tables (and by the Node emission companion).
///
/// WHAT: one entry function `run(a, b, d) -> r` with the exact statement sequence
///       1 `x = 3`, 2 `x = x + x`, 3 `x = x * x`, 4 `c = a - d`, 5 `b = -b`, 6 `q = x / d`,
///       7 `w = 3` (I64), 8 `w = w + w`, 9 `u = 6` (U64), 10 `u = u / u`, 11 `p = 2 ^ 3`,
///       and the checksum consumption chain 12 `r = x + q`, 13 `r = r + c`, 14 `r = r + p`
///       (return value 44 for the safe case (0, 7, 6): 42 - 6 + 8 = 44, so the exact safe
///       x chain stays observable; the dynamic divisor retains its check and the later
///       Power drops the one-entry interval cache, leaving the checksum add Runtime).
///       Two additional exports surface the width chains on their own: `run_i64()` returns the
///       proven I64 alias result (6) and `run_u64()` the proven U64 alias multiply result (36).
/// WHY: every provable statement is bounded by exact literal-derived intervals; every control
///       statement consumes a raw function parameter, so either mode must keep its runtime
///       checks. Alias writes exercise the destination/operand alias policy on both lanes, and
///       the exported checksum/width values make the proven results consumer-visible.
pub(crate) struct ProofFixture {
    pub(crate) module: HirModule,
    pub(crate) ids: ProofFixtureStatementIds,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) path_table: PathTable,
    pub(crate) string_table: StringTable,
}

/// Local-id layout of the fixture (also the stable LIR local ids; temps start afterwards).
mod fixture_locals {
    use crate::compiler_frontend::hir::ids::LocalId;
    pub(crate) const A: LocalId = LocalId(0);
    pub(crate) const B: LocalId = LocalId(1);
    pub(crate) const D: LocalId = LocalId(2);
    pub(crate) const X: LocalId = LocalId(3);
    pub(crate) const Q: LocalId = LocalId(4);
    pub(crate) const C: LocalId = LocalId(5);
    pub(crate) const W: LocalId = LocalId(6);
    pub(crate) const U: LocalId = LocalId(7);
    pub(crate) const P: LocalId = LocalId(8);
    pub(crate) const R: LocalId = LocalId(9);
    pub(crate) const WIDE_I64: LocalId = LocalId(0);
    pub(crate) const WIDE_U64: LocalId = LocalId(0);
}

/// Literal statement ids of the isolated-width functions (`i64=6`, `u64=36` exports).
const WIDE_I64_LITERAL_ID: HirNodeId = HirNodeId(8000);
const WIDE_U64_LITERAL_ID: HirNodeId = HirNodeId(9000);

/// Builds the genuine HIR both proof tables analyse.
pub(crate) fn build_proof_fixture_hir() -> ProofFixture {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);
    let mut expressions = HirExpressionStore::default();
    let int_type: TypeId = types.int;
    let i64_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
    let start_path = intern_path(&mut path_fork, &mut string_table, "run");

    let ids = ProofFixtureStatementIds {
        proven_alias_add: HirNodeId(7000),
        proven_alias_multiply: HirNodeId(7001),
        dynamic_subtract: HirNodeId(7002),
        dynamic_negate: HirNodeId(7003),
        dynamic_divide: HirNodeId(7004),
        proven_i64_alias_add: HirNodeId(7005),
        proven_u64_alias_divide: HirNodeId(7006),
        power: HirNodeId(7007),
        checksum_add: HirNodeId(7008),
        consumed_checksum_subtract: HirNodeId(7009),
        consumed_checksum_power_add: HirNodeId(7010),
        isolated_i64_alias_add: HirNodeId(7100),
        isolated_u64_alias_multiply: HirNodeId(7200),
    };

    // Builds one `Int`/fixed-scalar literal Assign.
    let literal = |expressions: &mut HirExpressionStore,
                   statement_id: HirNodeId,
                   target: LocalId,
                   value: i64,
                   ty: TypeId| HirStatement {
        id: statement_id,
        kind: HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(target),
            value: expression(
                HirExpressionKind::Int(value),
                ty,
                region,
                ValueKind::Const,
                expressions,
            ),
        },
        span: None,
    };
    // Builds one binary NumericOp with two module-store operands.
    let numeric_binary = |expressions: &mut HirExpressionStore,
                          statement_id: HirNodeId,
                          operator: NumericOperator,
                          domain: NumericScalar,
                          dest: HirLocalDestination,
                          left: LocalId,
                          right: LocalId,
                          operand_type: TypeId| {
        HirStatement {
            id: statement_id,
            kind: HirStatementKind::NumericOp {
                op: HirNumericOp { operator, domain },
                failure_mode: NumericFailureMode::Trap,
                operands: HirNumericOperands::Binary {
                    left: load_local(expressions, left, operand_type, region),
                    right: load_local(expressions, right, operand_type, region),
                },
                result: dest,
            },
            span: None,
        }
    };
    // Builds one unary Negate over a dynamic parameter.
    let numeric_negate = |expressions: &mut HirExpressionStore,
                          statement_id: HirNodeId,
                          operand: LocalId,
                          operand_type: TypeId| HirStatement {
        id: statement_id,
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Negate,
                domain: NumericScalar::Int,
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Unary {
                operand: load_local(expressions, operand, operand_type, region),
            },
            result: HirLocalDestination::Update(operand),
        },
        span: None,
    };
    // Builds one Power over literal Int operands.
    let numeric_literal_power = |expressions: &mut HirExpressionStore,
                                 statement_id: HirNodeId,
                                 base: i64,
                                 exponent: i64| HirStatement {
        id: statement_id,
        kind: HirStatementKind::NumericOp {
            op: HirNumericOp {
                operator: NumericOperator::Power,
                domain: NumericScalar::Int,
            },
            failure_mode: NumericFailureMode::Trap,
            operands: HirNumericOperands::Binary {
                left: expression(
                    HirExpressionKind::Int(base),
                    int_type,
                    region,
                    ValueKind::Const,
                    expressions,
                ),
                right: expression(
                    HirExpressionKind::Int(exponent),
                    int_type,
                    region,
                    ValueKind::Const,
                    expressions,
                ),
            },
            result: HirLocalDestination::Define(fixture_locals::P),
        },
        span: None,
    };

    // Expression IDs follow append order; statement IDs continue to identify proof facts.
    let statements = vec![
        literal(
            &mut expressions,
            HirNodeId(1000),
            fixture_locals::X,
            3,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.proven_alias_add,
            NumericOperator::Add,
            NumericScalar::Int,
            HirLocalDestination::Update(fixture_locals::X),
            fixture_locals::X,
            fixture_locals::X,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.proven_alias_multiply,
            NumericOperator::Multiply,
            NumericScalar::Int,
            HirLocalDestination::Update(fixture_locals::X),
            fixture_locals::X,
            fixture_locals::X,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.dynamic_subtract,
            NumericOperator::Subtract,
            NumericScalar::Int,
            HirLocalDestination::Define(fixture_locals::C),
            fixture_locals::A,
            fixture_locals::D,
            int_type,
        ),
        numeric_negate(
            &mut expressions,
            ids.dynamic_negate,
            fixture_locals::B,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.dynamic_divide,
            NumericOperator::IntegerDivide,
            NumericScalar::Int,
            HirLocalDestination::Define(fixture_locals::Q),
            fixture_locals::X,
            fixture_locals::D,
            int_type,
        ),
        literal(
            &mut expressions,
            HirNodeId(2000),
            fixture_locals::W,
            3,
            i64_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.proven_i64_alias_add,
            NumericOperator::Add,
            NumericScalar::Fixed(FixedScalar::I64),
            HirLocalDestination::Update(fixture_locals::W),
            fixture_locals::W,
            fixture_locals::W,
            i64_type,
        ),
        literal(
            &mut expressions,
            HirNodeId(3000),
            fixture_locals::U,
            6,
            u64_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.proven_u64_alias_divide,
            NumericOperator::IntegerDivide,
            NumericScalar::Fixed(FixedScalar::U64),
            HirLocalDestination::Update(fixture_locals::U),
            fixture_locals::U,
            fixture_locals::U,
            u64_type,
        ),
        numeric_literal_power(&mut expressions, ids.power, 2, 3),
        // The exported checksum consumes the safe chain and every dynamic result:
        // r = ((x + q) + (a - d)) + p = 44 for the safe case. Earlier writes and Power
        // clear the cached x interval, so the checksum add keeps its runtime check.
        numeric_binary(
            &mut expressions,
            ids.checksum_add,
            NumericOperator::Add,
            NumericScalar::Int,
            HirLocalDestination::Define(fixture_locals::R),
            fixture_locals::X,
            fixture_locals::Q,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.consumed_checksum_subtract,
            NumericOperator::Add,
            NumericScalar::Int,
            HirLocalDestination::Update(fixture_locals::R),
            fixture_locals::R,
            fixture_locals::C,
            int_type,
        ),
        numeric_binary(
            &mut expressions,
            ids.consumed_checksum_power_add,
            NumericOperator::Add,
            NumericScalar::Int,
            HirLocalDestination::Update(fixture_locals::R),
            fixture_locals::R,
            fixture_locals::P,
            int_type,
        ),
    ];

    let start_block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            local(0, int_type, region),
            local(1, int_type, region),
            local(2, int_type, region),
            local(3, int_type, region),
            local(4, int_type, region),
            local(5, int_type, region),
            local(6, i64_type, region),
            local(7, u64_type, region),
            local(8, int_type, region),
            local(9, int_type, region),
        ],
        statements,
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            fixture_locals::R,
            int_type,
            region,
        )),
    };
    let start_function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![fixture_locals::A, fixture_locals::B, fixture_locals::D],
        return_type: int_type,
    };

    // Isolated-width exports: each width's proven alias chain must be consumer-visible exactly,
    // so the emission companion observes the I64 and U64 results through real exports instead
    // of relying on the mixed-domain checksum inside run().
    let isolated_i64_block = HirBlock {
        id: BlockId(1),
        region,
        locals: vec![local(0, i64_type, region)],
        statements: vec![
            literal(
                &mut expressions,
                WIDE_I64_LITERAL_ID,
                fixture_locals::WIDE_I64,
                3,
                i64_type,
            ),
            numeric_binary(
                &mut expressions,
                ids.isolated_i64_alias_add,
                NumericOperator::Add,
                NumericScalar::Fixed(FixedScalar::I64),
                HirLocalDestination::Update(fixture_locals::WIDE_I64),
                fixture_locals::WIDE_I64,
                fixture_locals::WIDE_I64,
                i64_type,
            ),
        ],
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            fixture_locals::WIDE_I64,
            i64_type,
            region,
        )),
    };
    let isolated_i64_function = HirFunction {
        id: FunctionId(1),
        entry: BlockId(1),
        params: vec![],
        return_type: i64_type,
    };
    let isolated_u64_block = HirBlock {
        id: BlockId(2),
        region,
        locals: vec![local(0, u64_type, region)],
        statements: vec![
            literal(
                &mut expressions,
                WIDE_U64_LITERAL_ID,
                fixture_locals::WIDE_U64,
                6,
                u64_type,
            ),
            numeric_binary(
                &mut expressions,
                ids.isolated_u64_alias_multiply,
                NumericOperator::Multiply,
                NumericScalar::Fixed(FixedScalar::U64),
                HirLocalDestination::Update(fixture_locals::WIDE_U64),
                fixture_locals::WIDE_U64,
                fixture_locals::WIDE_U64,
                u64_type,
            ),
        ],
        terminator: HirTerminator::Return(load_local(
            &mut expressions,
            fixture_locals::WIDE_U64,
            u64_type,
            region,
        )),
    };
    let isolated_u64_function = HirFunction {
        id: FunctionId(2),
        entry: BlockId(2),
        params: vec![],
        return_type: u64_type,
    };

    let isolated_i64_path = intern_path(&mut path_fork, &mut string_table, "run_i64");
    let isolated_u64_path = intern_path(&mut path_fork, &mut string_table, "run_u64");
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        expressions,
        vec![
            (start_function, start_path, HirFunctionOrigin::EntryStart),
            (
                isolated_i64_function,
                isolated_i64_path,
                HirFunctionOrigin::Normal,
            ),
            (
                isolated_u64_function,
                isolated_u64_path,
                HirFunctionOrigin::Normal,
            ),
        ],
        vec![start_block, isolated_i64_block, isolated_u64_block],
        FunctionId(0),
    );

    ProofFixture {
        module,
        ids,
        type_environment,
        path_table: path_fork.snapshot_table(),
        string_table,
    }
}

fn intern_path(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    name: &str,
) -> PathId {
    path_fork
        .try_intern_portable_path(name, string_table)
        .expect("test path fits")
}

/// Lowers the fixture body with one proof table and returns its checked operations.
pub(crate) fn lower_fixture(
    fixture: &ProofFixture,
    proofs: &NumericProofs,
    profile: NumericProfile,
) -> Vec<WasmCheckedIntegerOperation> {
    let request = WasmBackendRequest {
        numeric_profile: profile,
        ..WasmBackendRequest::default()
    };
    let result = lower_hir_to_wasm_lir(
        &fixture.module,
        &default_borrow_facts(),
        proofs,
        &request,
        &fixture.string_table,
        &fixture.type_environment,
        &fixture.path_table,
    )
    .expect("proof-contrast lowering should succeed");

    let start = result
        .lir_module
        .functions
        .iter()
        .find(|function| function.origin == WasmLirFunctionOrigin::EntryStart)
        .expect("entry function lowers");
    start
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter_map(|statement| match statement {
            WasmLirStmt::CheckedIntegerOp { operation } => Some(*operation),
            _ => None,
        })
        .collect()
}

#[test]
fn analysed_table_proves_literal_derived_aliasing_and_retains_dynamic_controls() {
    let fixture = build_proof_fixture_hir();
    let proofs = analyse_numeric_proofs(
        &fixture.module,
        &fixture.type_environment,
        NumericProfile::STANDARD,
    );
    let proven_facts = [
        (fixture.ids.proven_alias_add, "literal-derived [3,3]+[3,3]"),
        (
            fixture.ids.proven_alias_multiply,
            "literal-derived [6,6]*[6,6]",
        ),
        (
            fixture.ids.proven_i64_alias_add,
            "literal-derived I64 [3,3]+[3,3]",
        ),
        (
            fixture.ids.proven_u64_alias_divide,
            "literal-derived U64 [6,6]/[6,6] with a non-zero divisor",
        ),
        (
            fixture.ids.isolated_i64_alias_add,
            "isolated-width I64 alias add",
        ),
        (
            fixture.ids.isolated_u64_alias_multiply,
            "isolated-width U64 alias multiply",
        ),
    ];
    for (id, label) in proven_facts {
        assert!(
            proofs.integer_operation_is_safe(id, NumericProfile::STANDARD),
            "{label} must prove"
        );
    }

    let retained_facts = [
        (
            fixture.ids.dynamic_subtract,
            "a raw parameter subtraction can underflow",
        ),
        (
            fixture.ids.dynamic_negate,
            "a raw parameter negation can hit INT_MIN",
        ),
        (
            fixture.ids.dynamic_divide,
            "a dynamic divisor can be zero or -1",
        ),
        (
            fixture.ids.consumed_checksum_subtract,
            "the checksum subtraction consumes the unbounded c",
        ),
        (
            fixture.ids.consumed_checksum_power_add,
            "the checksum consumes the Power result",
        ),
        (
            fixture.ids.power,
            "Power must never be proven by the analysis table",
        ),
        (
            fixture.ids.checksum_add,
            "the checksum add is Runtime: q divide never proves and Power drops the cache",
        ),
    ];
    for (id, label) in retained_facts {
        assert!(
            !proofs.integer_operation_is_safe(id, NumericProfile::STANDARD),
            "{label}: the statement stays runtime-checked"
        );
    }

    // Lowering honors the table per statement; identical ops and count in both tables.
    let retained = lower_fixture(
        &fixture,
        &default_numeric_proofs(),
        NumericProfile::STANDARD,
    );
    let analysed = lower_fixture(&fixture, &proofs, NumericProfile::STANDARD);
    assert_eq!(
        retained.len(),
        analysed.len(),
        "proof mode must not change the checked statement set"
    );
    for (statement_index, (left, right)) in retained.iter().zip(analysed.iter()).enumerate() {
        assert!(
            left.operator == right.operator && left.kind == right.kind,
            "checked statement {statement_index} stays identical in both editions"
        );
    }

    // Ordered CheckedIntegerOp sequence of the entry function:
    // 0..1 x chain, 2..4 dynamic controls, 5..6 widths, 7 power, 8..10 checksum chain.
    assert_eq!(analysed.len(), 11, "fixture lowers the complete chain");

    let prove = |label: &str, operation: &WasmCheckedIntegerOperation| {
        assert_eq!(
            operation.check_mode,
            IntegerCheckMode::ProvenSafe,
            "{label} must be proven-safe in the analysed table"
        );
        assert!(
            operation.scratch.product.is_none(),
            "proven {label} must keep no check-only product scratch"
        );
        assert!(
            operation.scratch.power.is_none(),
            "proven {label} must keep no power scratch"
        );
    };
    let checked = |label: &str, operation: &WasmCheckedIntegerOperation, table: &str| {
        assert_eq!(
            operation.check_mode,
            IntegerCheckMode::Runtime,
            "{label} in the {table} table stays runtime-checked"
        );
    };

    prove("alias add", &analysed[0]);
    prove("alias multiply", &analysed[1]);
    prove("I64 alias add", &analysed[5]);
    prove("U64 alias divide", &analysed[6]);
    checked("checksum add", &analysed[8], "analysed");

    checked("dynamic subtract", &analysed[2], "analysed");
    checked("dynamic negate", &analysed[3], "analysed");
    checked("dynamic divide", &analysed[4], "analysed");
    checked("checksum consumption via c", &analysed[9], "analysed");
    checked("checksum consumption via power", &analysed[10], "analysed");
    for (position, label) in [
        (2, "dynamic subtract"),
        (3, "dynamic negate"),
        (4, "dynamic divide"),
        (8, "checksum add"),
        (9, "checksum consumption via c"),
        (10, "checksum consumption via power"),
    ] {
        checked(label, &retained[position], "retained");
    }

    // Power: runtime-checked with BOTH scratch locals in either table.
    for (position, table, label) in [(7, &retained, "retained"), (7, &analysed, "analysed")] {
        let power_operation = &table[position];
        assert_eq!(
            power_operation.check_mode,
            IntegerCheckMode::Runtime,
            "{label} power stays runtime-checked"
        );
        assert!(
            power_operation.scratch.product.is_some(),
            "{label} 32-bit power keeps its widened product scratch"
        );
        assert!(
            power_operation.scratch.power.is_some(),
            "{label} power keeps its loop scratch"
        );
    }

    // Proven alias add/multiply write their alias destination directly: their operands read
    // the same local the result overwrites, so no intermediate temp is lowered.
    for (position, label) in [(0, "add"), (1, "multiply")] {
        let operand = match analysed[position].operands {
            WasmNumericOperationOperands::Binary { left, .. } => left,
            _ => panic!("{label} lowers binary operands"),
        };
        assert_eq!(
            analysed[position].dst, operand,
            "proven {label} writes its alias destination directly"
        );
    }
    // Dynamic alias negation keeps the retained temp+commit shape.
    let negate_operand = match retained[3].operands {
        WasmNumericOperationOperands::Unary { operand } => operand,
        _ => panic!("negate lowers unary operands"),
    };
    assert_ne!(
        retained[3].dst, negate_operand,
        "retained negation writes a separate temp before committing its alias dst"
    );
}
